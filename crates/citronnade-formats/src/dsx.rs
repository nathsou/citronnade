//! Converting an ELF executable into a 3DSX.
//!
//! A 3DSX holds three segments (code, read-only data, data with `.bss` at
//! its end) and per-segment relocation tables, so a loader can place the
//! segments anywhere. The ELF must:
//!
//! - have its `PT_LOAD` segments in that order, contiguous, each starting on
//!   a page boundary after the previous one's page-rounded end, with sizes
//!   that are multiples of 4;
//! - have its entry point at the start of the code segment;
//! - keep its relocations (`--emit-relocs`), which become the tables.
//!
//! Absolute relocations (`R_ARM_ABS32`, `R_ARM_TARGET1`) are stored as
//! offsets from the base address, which the loader adds back. Relative ones
//! (`R_ARM_REL32`, `R_ARM_TARGET2`, `R_ARM_PREL31`) need fixing only when they
//! cross segments; they are stored as absolute offsets (with bit 28 marking a
//! 31-bit `PREL31` entry) and the loader rewrites them relative to their
//! position.
//!
//! The file layout is: header, optional extended header (SMDH and RomFS
//! offsets), three relocation headers, the segments (without `.bss`), the
//! relocation entries, then the SMDH and the RomFS.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use crate::elf::{self, Elf, ElfError};

pub const MAGIC: u32 = 0x5853_4433; // "3DSX"
const HEADER_SIZE: u16 = 8 * 4;
const EXT_HEADER_SIZE: u16 = 3 * 4;
const RELOC_HEADER_SIZE: u16 = 2 * 4;
const PAGE: u32 = 0x1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DsxError {
    Elf(ElfError),
    NoSegments,
    NonContiguousSegments { expected: u32, found: u32 },
    UnalignedSegment(u32),
    BssOutsideData,
    UnexpectedSegment { flags: u32 },
    DuplicateSegment(&'static str),
    SegmentOrder,
    EntryNotAtStart { entry: u32, base: u32 },
    TooLarge,
    NoSymbolTable,
    RelaUnsupported,
    UnalignedRelocation(u32),
    RelocationToInvalidAddress { at: u32, target: u32 },
}

impl From<ElfError> for DsxError {
    fn from(e: ElfError) -> Self {
        DsxError::Elf(e)
    }
}

impl fmt::Display for DsxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DsxError::Elf(e) => write!(f, "{e}"),
            DsxError::NoSegments => f.write_str("the ELF has no loadable segments"),
            DsxError::NonContiguousSegments { expected, found } => write!(
                f,
                "segments are not contiguous: expected one at 0x{expected:08X}, found 0x{found:08X} \
                 (each segment must start on the page after the previous one)"
            ),
            DsxError::UnalignedSegment(a) => {
                write!(f, "the segment at 0x{a:08X} has a size that is not a multiple of 4")
            }
            DsxError::BssOutsideData => {
                f.write_str("only the read/write data segment may have uninitialised (.bss) space")
            }
            DsxError::UnexpectedSegment { flags } => write!(
                f,
                "unexpected segment permissions {flags:#x} (expected R+X code, R rodata, R+W data)"
            ),
            DsxError::DuplicateSegment(k) => write!(f, "more than one {k} segment"),
            DsxError::SegmentOrder => {
                f.write_str("segments must be in the order code, rodata, data")
            }
            DsxError::EntryNotAtStart { entry, base } => write!(
                f,
                "the entry point 0x{entry:08X} is not the start of the code segment 0x{base:08X}"
            ),
            DsxError::TooLarge => f.write_str("the program is larger than 256 MiB"),
            DsxError::NoSymbolTable => f.write_str("the ELF has no symbol table"),
            DsxError::RelaUnsupported => f.write_str("RELA relocation sections are not supported"),
            DsxError::UnalignedRelocation(a) => write!(f, "unaligned relocation at 0x{a:08X}"),
            DsxError::RelocationToInvalidAddress { at, target } => write!(
                f,
                "the relocation at 0x{at:08X} points outside the program (0x{target:08X})"
            ),
        }
    }
}

/// A run of words: skip `skip`, then patch `patch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reloc {
    pub skip: u16,
    pub patch: u16,
}

/// What a 3DSX holds besides its segments.
#[derive(Default)]
pub struct Extras<'a> {
    pub smdh: Option<&'a [u8]>,
    pub romfs: Option<&'a [u8]>,
}

/// Sizes of a converted program, for reporting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Layout {
    pub base: u32,
    pub code: u32,
    pub rodata: u32,
    pub data: u32,
    pub bss: u32,
    pub absolute_relocations: u32,
    pub relative_relocations: u32,
}

struct Segments {
    base: u32,
    top: u32,
    code: u32,
    rodata: u32,
    data: u32,
    bss: u32,
    /// Memory image from `base` to `top` (file-backed bytes, zeros elsewhere).
    image: Vec<u8>,
}

fn align_page(x: u32) -> u32 {
    (x + PAGE - 1) & !(PAGE - 1)
}

fn segments(elf: &Elf<'_>) -> Result<Segments, DsxError> {
    let mut base = None;
    let mut top = 0u32;
    let (mut code, mut rodata, mut data, mut bss) = (None, None, None, 0u32);
    let mut loads = Vec::new();
    for ph in &elf.programs {
        if ph.kind != elf::PT_LOAD || ph.mem_size == 0 {
            continue;
        }
        match base {
            None => base = Some(ph.vaddr),
            Some(_) if ph.vaddr != top => {
                return Err(DsxError::NonContiguousSegments { expected: top, found: ph.vaddr });
            }
            _ => {}
        }
        if ph.mem_size % 4 != 0 || ph.file_size % 4 != 0 {
            return Err(DsxError::UnalignedSegment(ph.vaddr));
        }
        if ph.flags != 6 && ph.mem_size != ph.file_size {
            return Err(DsxError::BssOutsideData);
        }
        match ph.flags & 7 {
            5 => {
                if code.is_some() {
                    return Err(DsxError::DuplicateSegment("code"));
                }
                if rodata.is_some() || data.is_some() {
                    return Err(DsxError::SegmentOrder);
                }
                code = Some(ph.mem_size);
            }
            4 => {
                if rodata.is_some() {
                    return Err(DsxError::DuplicateSegment("rodata"));
                }
                if data.is_some() {
                    return Err(DsxError::SegmentOrder);
                }
                rodata = Some(ph.mem_size);
            }
            6 => {
                if data.is_some() {
                    return Err(DsxError::DuplicateSegment("data"));
                }
                data = Some(ph.mem_size);
                bss = ph.mem_size - ph.file_size;
            }
            flags => return Err(DsxError::UnexpectedSegment { flags }),
        }
        top = ph.vaddr + align_page(ph.mem_size);
        loads.push(*ph);
    }
    let base = base.ok_or(DsxError::NoSegments)?;
    if top - base >= 0x1000_0000 {
        return Err(DsxError::TooLarge);
    }
    if elf.entry != base {
        return Err(DsxError::EntryNotAtStart { entry: elf.entry, base });
    }
    let mut image = vec![0u8; (top - base) as usize];
    for ph in loads {
        let src = elf
            .data
            .get(ph.offset as usize..(ph.offset + ph.file_size) as usize)
            .ok_or(ElfError::Truncated)?;
        let at = (ph.vaddr - base) as usize;
        image[at..at + src.len()].copy_from_slice(src);
    }
    Ok(Segments {
        base,
        top,
        code: code.unwrap_or(0),
        rodata: rodata.unwrap_or(0),
        data: data.unwrap_or(0),
        bss,
        image,
    })
}

/// Converts an ELF executable into a 3DSX.
pub fn convert(elf_data: &[u8], extras: &Extras<'_>) -> Result<(Vec<u8>, Layout), DsxError> {
    let elf = Elf::parse(elf_data)?;
    let mut seg = segments(&elf)?;
    let base = seg.base;
    let top = seg.top;
    let rodata_start = base + align_page(seg.code);
    let data_start = rodata_start + align_page(seg.rodata);

    let words = ((top - base) / 4) as usize;
    let mut absolute = vec![false; words];
    let mut relative = vec![false; words];

    if !elf.sections.iter().any(|s| s.kind == elf::SHT_SYMTAB) {
        return Err(DsxError::NoSymbolTable);
    }

    let segment_of = |addr: u32| -> u8 {
        if addr < rodata_start {
            0
        } else if addr < data_start {
            1
        } else {
            2
        }
    };

    for rel_section in &elf.sections {
        if rel_section.kind == elf::SHT_RELA {
            return Err(DsxError::RelaUnsupported);
        }
        if rel_section.kind != elf::SHT_REL {
            continue;
        }
        let Some(target) = elf.sections.get(rel_section.info as usize) else { continue };
        if target.flags & elf::SHF_ALLOC == 0 {
            continue;
        }
        let Some(symtab) = elf.sections.get(rel_section.link as usize) else { continue };
        let symbols = elf.symbols(symtab)?;
        let (vstart, vend) = (target.addr, target.addr + target.size);
        let rels = elf.section_data(rel_section)?;
        for entry in rels.chunks_exact(8) {
            let at = elf::u32_at(entry, 0)?;
            let info = elf::u32_at(entry, 4)?;
            let kind = info & 0xFF;
            let sym = symbols.get((info >> 8) as usize).copied();
            if at & 3 != 0 {
                return Err(DsxError::UnalignedRelocation(at));
            }
            if at < vstart || at >= vend || at < base || at >= top {
                continue;
            }
            let index = ((at - base) / 4) as usize;
            if absolute[index] || relative[index] {
                continue;
            }
            let off = (at - base) as usize;
            let word = u32::from_le_bytes(seg.image[off..off + 4].try_into().unwrap());
            match kind {
                elf::R_ARM_ABS32 | elf::R_ARM_TARGET1 => {
                    // Unresolved weak symbols stay 0.
                    if let Some(s) = sym {
                        if s.binding() == elf::STB_WEAK && s.value == 0 {
                            continue;
                        }
                    }
                    if word < base {
                        return Err(DsxError::RelocationToInvalidAddress { at, target: word });
                    }
                    seg.image[off..off + 4].copy_from_slice(&(word - base).to_le_bytes());
                    absolute[index] = true;
                }
                elf::R_ARM_REL32 | elf::R_ARM_TARGET2 | elf::R_ARM_PREL31 | elf::R_ARM_TLS_IE32 => {
                    let mut offset = word as i32;
                    if kind == elf::R_ARM_PREL31 {
                        // An inline unwinding entry, not an offset.
                        if word & (1 << 31) != 0 {
                            continue;
                        }
                        if word & (1 << 30) != 0 {
                            offset = (word | (1 << 31)) as i32;
                        }
                    }
                    let target = at.wrapping_add(offset as u32);
                    if target < base || target > top {
                        return Err(DsxError::RelocationToInvalidAddress { at, target });
                    }
                    if segment_of(target) != segment_of(at) {
                        let mut stored = target - base;
                        if kind == elf::R_ARM_PREL31 {
                            stored |= 1 << 28;
                        }
                        seg.image[off..off + 4].copy_from_slice(&stored.to_le_bytes());
                        relative[index] = true;
                    }
                }
                _ => {}
            }
        }
    }

    let mut relocs = Vec::new();
    let mut headers = [[0u32; 2]; 3];
    let bounds = [(base, rodata_start), (rodata_start, data_start), (data_start, top)];
    for (i, &(start, end)) in bounds.iter().enumerate() {
        let (s, e) = (((start - base) / 4) as usize, ((end - base) / 4) as usize);
        headers[i][0] = build_relocs(&absolute[s..e], &mut relocs);
        headers[i][1] = build_relocs(&relative[s..e], &mut relocs);
    }

    let has_ext = extras.smdh.is_some() || extras.romfs.is_some();
    let mut out = Vec::with_capacity(seg.image.len() + 1024);
    push32(&mut out, MAGIC);
    push16(&mut out, HEADER_SIZE + if has_ext { EXT_HEADER_SIZE } else { 0 });
    push16(&mut out, RELOC_HEADER_SIZE);
    push32(&mut out, 0); // version
    push32(&mut out, 0); // flags
    push32(&mut out, seg.code);
    push32(&mut out, seg.rodata);
    push32(&mut out, seg.data);
    push32(&mut out, seg.bss);
    let ext_at = out.len();
    if has_ext {
        out.extend_from_slice(&[0; 12]);
    }
    for h in headers {
        push32(&mut out, h[0]);
        push32(&mut out, h[1]);
    }
    let code_at = 0usize;
    let rodata_at = (rodata_start - base) as usize;
    let data_at = (data_start - base) as usize;
    out.extend_from_slice(&seg.image[code_at..code_at + seg.code as usize]);
    out.extend_from_slice(&seg.image[rodata_at..rodata_at + seg.rodata as usize]);
    out.extend_from_slice(&seg.image[data_at..data_at + (seg.data - seg.bss) as usize]);
    for r in &relocs {
        push16(&mut out, r.skip);
        push16(&mut out, r.patch);
    }
    if has_ext {
        let smdh = extras.smdh.unwrap_or(&[]);
        let smdh_at = out.len() as u32;
        out[ext_at..ext_at + 4].copy_from_slice(&smdh_at.to_le_bytes());
        out[ext_at + 4..ext_at + 8].copy_from_slice(&(smdh.len() as u32).to_le_bytes());
        out.extend_from_slice(smdh);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        if let Some(romfs) = extras.romfs {
            let romfs_at = out.len() as u32;
            out[ext_at + 8..ext_at + 12].copy_from_slice(&romfs_at.to_le_bytes());
            out.extend_from_slice(romfs);
        }
    }

    let layout = Layout {
        base,
        code: seg.code,
        rodata: seg.rodata,
        data: seg.data - seg.bss,
        bss: seg.bss,
        absolute_relocations: absolute.iter().filter(|&&b| b).count() as u32,
        relative_relocations: relative.iter().filter(|&&b| b).count() as u32,
    };
    Ok((out, layout))
}

/// Encodes a word map as skip/patch runs, dropping a trailing skip.
/// Returns the number of entries added.
pub fn build_relocs(map: &[bool], out: &mut Vec<Reloc>) -> u32 {
    let before = out.len();
    let mut i = 0;
    while i < map.len() {
        let (mut skip, mut patch) = (0u32, 0u32);
        while i < map.len() && !map[i] {
            i += 1;
            skip += 1;
        }
        while i < map.len() && map[i] {
            i += 1;
            patch += 1;
        }
        if patch == 0 {
            break; // only skips were left
        }
        while skip > 0xFFFF {
            out.push(Reloc { skip: 0xFFFF, patch: 0 });
            skip -= 0xFFFF;
        }
        while patch > 0xFFFF {
            out.push(Reloc { skip: skip as u16, patch: 0xFFFF });
            skip = 0;
            patch -= 0xFFFF;
        }
        out.push(Reloc { skip: skip as u16, patch: patch as u16 });
    }
    (out.len() - before) as u32
}

/// The parts of a 3DSX, for inspection and tests.
#[derive(Debug)]
pub struct Parsed {
    pub code: u32,
    pub rodata: u32,
    pub data: u32,
    pub bss: u32,
    pub relocation_headers: [[u32; 2]; 3],
    pub smdh: Option<(u32, u32)>,
    pub romfs: Option<u32>,
}

/// Reads a 3DSX header.
pub fn parse_header(data: &[u8]) -> Result<Parsed, String> {
    let w = |o: usize| elf::u32_at(data, o).map_err(|_| String::from("truncated 3DSX"));
    if w(0)? != MAGIC {
        return Err("not a 3DSX file".into());
    }
    let header_size = elf::u16_at(data, 4).map_err(|_| String::from("truncated 3DSX"))? as usize;
    let reloc_at = header_size;
    let mut relocation_headers = [[0; 2]; 3];
    for (i, h) in relocation_headers.iter_mut().enumerate() {
        h[0] = w(reloc_at + i * 8)?;
        h[1] = w(reloc_at + i * 8 + 4)?;
    }
    let (smdh, romfs) = if header_size >= 44 {
        let (o, s, r) = (w(32)?, w(36)?, w(40)?);
        ((s != 0).then_some((o, s)), (r != 0).then_some(r))
    } else {
        (None, None)
    };
    Ok(Parsed {
        code: w(16)?,
        rodata: w(20)?,
        data: w(24)?,
        bss: w(28)?,
        relocation_headers,
        smdh,
        romfs,
    })
}

fn push16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn push32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(bits: &[u8]) -> Vec<Reloc> {
        let map: Vec<bool> = bits.iter().map(|&b| b == 1).collect();
        let mut out = Vec::new();
        build_relocs(&map, &mut out);
        out
    }

    #[test]
    fn encodes_runs() {
        assert_eq!(runs(&[]), vec![]);
        assert_eq!(runs(&[0, 0, 0]), vec![], "trailing skips are dropped");
        assert_eq!(runs(&[1, 1, 0, 1]), vec![Reloc { skip: 0, patch: 2 }, Reloc { skip: 1, patch: 1 }]);
        assert_eq!(runs(&[0, 0, 1, 0, 0]), vec![Reloc { skip: 2, patch: 1 }]);
    }

    #[test]
    fn splits_long_runs() {
        let mut map = vec![false; 0x1_0005];
        map.push(true);
        let mut out = Vec::new();
        build_relocs(&map, &mut out);
        assert_eq!(out, vec![Reloc { skip: 0xFFFF, patch: 0 }, Reloc { skip: 6, patch: 1 }]);

        let map = vec![true; 0x1_0002];
        let mut out = Vec::new();
        build_relocs(&map, &mut out);
        assert_eq!(out, vec![Reloc { skip: 0, patch: 0xFFFF }, Reloc { skip: 0, patch: 3 }]);
    }

    /// A tiny hand-built ELF: a code word that points into rodata (absolute
    /// relocation) and a rodata word holding a PC-relative offset to code
    /// (a cross-segment relative relocation).
    fn tiny_elf() -> Vec<u8> {
        let base = 0x0010_0000u32;
        let code = [0xEAFF_FFFEu32, base + 0x1000]; // b . ; .word rodata
        let rodata = [0u32.wrapping_sub(0x1000)]; // REL32 to `base` from base+0x1000
        let mut f = vec![0u8; 0x200];
        let code_off = 0x100u32;
        let ro_off = 0x180u32;
        for (i, w) in code.iter().enumerate() {
            f[(code_off as usize + i * 4)..][..4].copy_from_slice(&w.to_le_bytes());
        }
        f[ro_off as usize..][..4].copy_from_slice(&rodata[0].to_le_bytes());
        // sections: null, .text, .rodata, .symtab, .strtab, .rel.text, .rel.rodata, .shstrtab
        let shstr = b"\0.text\0.rodata\0.symtab\0.strtab\0.rel.text\0.rel.rodata\0.shstrtab\0";
        let strtab = b"\0_start\0";
        let shstr_off = 0x1C0usize;
        f.resize(0x300, 0);
        f[shstr_off..shstr_off + shstr.len()].copy_from_slice(shstr);
        let strtab_off = 0x220usize;
        f[strtab_off..strtab_off + strtab.len()].copy_from_slice(strtab);
        // symtab: null symbol + _start (value base) at 0x240
        let symtab_off = 0x240usize;
        f[symtab_off + 16..symtab_off + 20].copy_from_slice(&1u32.to_le_bytes());
        f[symtab_off + 20..symtab_off + 24].copy_from_slice(&base.to_le_bytes());
        f[symtab_off + 28] = 0x12; // global func
        f[symtab_off + 30..symtab_off + 32].copy_from_slice(&1u16.to_le_bytes());
        // rel.text at 0x260: offset base+4, ABS32 against sym 1
        let rel_text = 0x260usize;
        f[rel_text..rel_text + 4].copy_from_slice(&(base + 4).to_le_bytes());
        f[rel_text + 4..rel_text + 8].copy_from_slice(&((1u32 << 8) | elf::R_ARM_ABS32).to_le_bytes());
        // rel.rodata at 0x268: offset base+0x1000, REL32 against sym 1
        let rel_ro = 0x268usize;
        f[rel_ro..rel_ro + 4].copy_from_slice(&(base + 0x1000).to_le_bytes());
        f[rel_ro + 4..rel_ro + 8].copy_from_slice(&((1u32 << 8) | elf::R_ARM_REL32).to_le_bytes());

        let mut elf = vec![0u8; 52];
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf[4] = 1;
        elf[5] = 1;
        elf[6] = 1;
        elf[16..18].copy_from_slice(&2u16.to_le_bytes());
        elf[18..20].copy_from_slice(&40u16.to_le_bytes());
        elf[24..28].copy_from_slice(&base.to_le_bytes());
        let phoff = 0x300u32;
        let shoff = 0x340u32;
        elf[28..32].copy_from_slice(&phoff.to_le_bytes());
        elf[32..36].copy_from_slice(&shoff.to_le_bytes());
        elf[42..44].copy_from_slice(&32u16.to_le_bytes());
        elf[44..46].copy_from_slice(&2u16.to_le_bytes());
        elf[46..48].copy_from_slice(&40u16.to_le_bytes());
        elf[48..50].copy_from_slice(&8u16.to_le_bytes());
        elf[50..52].copy_from_slice(&7u16.to_le_bytes());
        f[..52].copy_from_slice(&elf);
        // program headers
        let ph = |f: &mut Vec<u8>, at: usize, off: u32, vaddr: u32, size: u32, flags: u32| {
            let vals = [1, off, vaddr, vaddr, size, size, flags, 0x1000];
            for (i, v) in vals.iter().enumerate() {
                f[at + i * 4..at + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
        };
        f.resize(0x340 + 8 * 40, 0);
        ph(&mut f, 0x300, code_off, base, 8, 5);
        ph(&mut f, 0x320, ro_off, base + 0x1000, 4, 4);
        // section headers
        let sh = |f: &mut Vec<u8>, i: usize, vals: [u32; 10]| {
            let at = 0x340 + i * 40;
            for (j, v) in vals.iter().enumerate() {
                f[at + j * 4..at + j * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
        };
        let name = |n: &str| -> u32 {
            let s = core::str::from_utf8(shstr).unwrap();
            s.find(n).unwrap() as u32
        };
        sh(&mut f, 1, [name(".text"), 1, 6, base, code_off, 8, 0, 0, 4, 0]);
        sh(&mut f, 2, [name(".rodata"), 1, 2, base + 0x1000, ro_off, 4, 0, 0, 4, 0]);
        sh(&mut f, 3, [name(".symtab"), 2, 0, 0, symtab_off as u32, 32, 4, 1, 4, 16]);
        sh(&mut f, 4, [name(".strtab"), 3, 0, 0, strtab_off as u32, strtab.len() as u32, 0, 0, 1, 0]);
        sh(&mut f, 5, [name(".rel.text"), 9, 0, 0, rel_text as u32, 8, 3, 1, 4, 8]);
        sh(&mut f, 6, [name(".rel.rodata"), 9, 0, 0, rel_ro as u32, 8, 3, 2, 4, 8]);
        sh(&mut f, 7, [name(".shstrtab"), 3, 0, 0, shstr_off as u32, shstr.len() as u32, 0, 0, 1, 0]);
        f
    }

    #[test]
    fn converts_a_tiny_elf() {
        let elf = tiny_elf();
        let smdh = [7u8; 6];
        let (out, layout) = convert(&elf, &Extras { smdh: Some(&smdh), romfs: Some(b"ROMFS") }).unwrap();
        assert_eq!(layout.code, 8);
        assert_eq!(layout.rodata, 4);
        assert_eq!(layout.absolute_relocations, 1);
        assert_eq!(layout.relative_relocations, 1);
        let p = parse_header(&out).unwrap();
        assert_eq!((p.code, p.rodata, p.data, p.bss), (8, 4, 0, 0));
        // code: the absolute word is at index 1; rodata: the relative one at 0
        assert_eq!(p.relocation_headers, [[1, 0], [0, 1], [0, 0]]);
        let code_at = 44 + 24;
        let word = |o: usize| u32::from_le_bytes(out[o..o + 4].try_into().unwrap());
        assert_eq!(word(code_at + 4), 0x1000, "absolute word stored as an offset from the base");
        assert_eq!(word(code_at + 8), 0, "relative word stored as the absolute offset of its target");
        let relocs_at = code_at + 12;
        assert_eq!(&out[relocs_at..relocs_at + 4], &[1, 0, 1, 0], "skip 1, patch 1");
        assert_eq!(&out[relocs_at + 4..relocs_at + 8], &[0, 0, 1, 0], "skip 0, patch 1");
        let (smdh_at, smdh_len) = p.smdh.unwrap();
        assert_eq!(smdh_len, 6);
        assert_eq!(&out[smdh_at as usize..smdh_at as usize + 6], &smdh);
        let romfs_at = p.romfs.unwrap() as usize;
        assert_eq!(romfs_at % 4, 0);
        assert_eq!(&out[romfs_at..], b"ROMFS");
    }

    #[test]
    fn rejects_bad_entry() {
        let mut elf = tiny_elf();
        elf[24..28].copy_from_slice(&0x0010_0004u32.to_le_bytes());
        assert!(matches!(convert(&elf, &Extras::default()), Err(DsxError::EntryNotAtStart { .. })));
    }
}
