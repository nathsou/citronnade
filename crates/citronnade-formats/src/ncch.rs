//! NCCH executables (CXI): what the system installs and starts as a title,
//! the content of a CIA.
//!
//! An NCCH is a 0x200-byte header, the extended header (what the program
//! is: its code segments, the system modules it needs, and what it may do,
//! twice: as asked and as allowed, the access descriptor), the logo (the
//! splash the HOME Menu shows while starting the title), the ExeFS (the
//! code, the SMDH icon and optionally a banner) and optionally a RomFS
//! wrapped in an IVFC hash tree. Offsets and sizes count 0x200-byte media
//! units; the header holds SHA-256 hashes of the extended header and of the
//! ExeFS and RomFS headers.
//!
//! Nothing is encrypted or signed: the signatures are placeholders, as
//! makerom writes them without keys, so consoles run these titles with
//! signature checks patched (as custom firmware does). The access control
//! is the usual homebrew template's (every system call, the common
//! services, SD card access).
//!
//! The layout follows makerom (Project_CTR, MIT licence) and 3dbrew.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use crate::elf::{self, Elf, ElfError};
use crate::sha256::sha256;

const MEDIA_UNIT: usize = 0x200;
const PAGE: u32 = 0x1000;
/// The RomFS's IVFC block size.
const IVFC_BLOCK: usize = 0x1000;
/// The size of the IVFC header.
const IVFC_HEADER: usize = 0x5C;

/// The splash the HOME Menu shows while it starts the title: the homebrew
/// logo (an LZ11-compressed darc of images), as makerom's `Logo: Homebrew`
/// (the usual homebrew template's) includes it.
const LOGO: &[u8; 0x2000] = include_bytes!("ncch_logo.bin");

/// The kernel's CoreVersion the access control targets.
pub const CORE_VERSION: u32 = 2;

/// The system modules an application depends on (the homebrew template's).
pub const DEPENDENCIES: [u64; 30] = [
    0x0004013000002402, // ac
    0x0004013000001502, // am
    0x0004013000003402, // boss
    0x0004013000001602, // camera
    0x0004013000002602, // cecd
    0x0004013000001702, // cfg
    0x0004013000001802, // codec
    0x0004013000002702, // csnd
    0x0004013000002802, // dlp
    0x0004013000001a02, // dsp
    0x0004013000003202, // friends
    0x0004013000001b02, // gpio
    0x0004013000001c02, // gsp
    0x0004013000001d02, // hid
    0x0004013000002902, // http
    0x0004013000001e02, // i2c
    0x0004013000003302, // ir
    0x0004013000001f02, // mcu
    0x0004013000002002, // mic
    0x0004013000002b02, // ndm
    0x0004013000003502, // news
    0x0004013000002c02, // nim
    0x0004013000002d02, // nwm
    0x0004013000002102, // pdn
    0x0004013000003102, // ps
    0x0004013000002202, // ptm
    0x0004013000003702, // ro
    0x0004013000002e02, // socket
    0x0004013000002302, // spi
    0x0004013000002f02, // ssl
];

/// The services the program may use.
const SERVICES: [&str; 31] = [
    "APT:U", "ac:u", "am:net", "boss:U", "cam:u", "cecd:u", "cfg:nor", "cfg:u", "csnd:SND", "dsp::DSP", "frd:u", "fs:USER", "gsp::Gpu",
    "gsp::Lcd", "hid:USER", "http:C", "ir:rst", "ir:u", "ir:USER", "mic:u", "mcu::HWC", "ndm:u", "news:s", "nwm::EXT", "nwm::UDS",
    "ptm:sysm", "ptm:u", "pxi:dev", "soc:U", "ssl:C", "y2r:u",
];

/// How much memory the title asks for on a New 3DS (the Old 3DS always
/// gives an application 64 MiB).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum New3dsMemory {
    /// As on the Old 3DS.
    #[default]
    Legacy,
    /// 124 MiB.
    Mb124,
    /// 178 MiB.
    Mb178,
}

/// What an NCCH executable is made of.
pub struct Program<'a> {
    /// `0x00040000_XXXXXX00` for an application (`program_id`).
    pub title_id: u64,
    /// Up to 16 ASCII characters, for example `CTR-P-ABCD`.
    pub product_code: &'a str,
    /// The process name, up to 8 ASCII characters.
    pub name: &'a str,
    /// The title version (major << 10 | minor << 4 | micro).
    pub version: u16,
    /// The program, linked at 0x00100000 with its code, read-only data and
    /// data in three segments.
    pub elf: &'a [u8],
    /// The main thread's stack.
    pub stack_size: u32,
    /// The SMDH (icon and titles).
    pub icon: &'a [u8],
    /// The HOME Menu banner (a CBMD, `.bnr`), if any.
    pub banner: Option<&'a [u8]>,
    /// The RomFS image (its "level 3", as [`crate::romfs::build`] makes it).
    pub romfs: Option<&'a [u8]>,
    /// On a New 3DS: the 804 MHz clock, the L2 cache and more memory from
    /// the start.
    pub new3ds_speedup: bool,
    pub new3ds_memory: New3dsMemory,
}

#[derive(Debug)]
pub enum NcchError {
    Elf(ElfError),
    /// The ELF lacks a code segment, or has one twice.
    Segments(&'static str),
    /// A string does not fit its field or is not ASCII.
    Field(&'static str),
}

impl From<ElfError> for NcchError {
    fn from(e: ElfError) -> Self {
        NcchError::Elf(e)
    }
}

impl fmt::Display for NcchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NcchError::Elf(e) => write!(f, "{e}"),
            NcchError::Segments(s) => f.write_str(s),
            NcchError::Field(s) => write!(f, "the {s} is too long or not ASCII"),
        }
    }
}

fn align(x: usize, a: usize) -> usize {
    x.div_ceil(a) * a
}

fn put32(out: &mut [u8], at: usize, v: u32) {
    out[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(out: &mut [u8], at: usize, v: u64) {
    out[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn put_str(out: &mut [u8], at: usize, len: usize, s: &str, what: &'static str) -> Result<(), NcchError> {
    if s.len() > len || !s.is_ascii() {
        return Err(NcchError::Field(what));
    }
    out[at..at + s.len()].copy_from_slice(s.as_bytes());
    Ok(())
}

/// A loadable segment: its address, its bytes, and its size in memory.
struct Segment<'a> {
    address: u32,
    bytes: &'a [u8],
    memory: u32,
}

impl Segment<'_> {
    fn pages(&self) -> u32 {
        (self.bytes.len() as u32).div_ceil(PAGE)
    }
}

/// The code, read-only data and data segments (by their permissions).
fn segments<'a>(elf: &Elf<'a>) -> Result<[Option<Segment<'a>>; 3], NcchError> {
    let mut found: [Option<Segment<'a>>; 3] = [None, None, None];
    for ph in &elf.programs {
        if ph.kind != elf::PT_LOAD || ph.mem_size == 0 {
            continue;
        }
        let slot = match ph.flags & 7 {
            5 => 0,
            4 => 1,
            6 => 2,
            _ => return Err(NcchError::Segments("a segment is neither code (R+X), read-only data (R) nor data (R+W)")),
        };
        if found[slot].is_some() {
            return Err(NcchError::Segments("the ELF has two segments of one kind"));
        }
        let bytes = elf.data.get(ph.offset as usize..(ph.offset + ph.file_size) as usize).ok_or(ElfError::Truncated)?;
        found[slot] = Some(Segment { address: ph.vaddr, bytes, memory: ph.mem_size });
    }
    if found[0].is_none() {
        return Err(NcchError::Segments("the ELF has no code segment"));
    }
    Ok(found)
}

/// The kernel capability descriptors: every system call (the template's
/// list), the DSP's memory as I/O, VRAM read-only, the other capabilities,
/// 0x200 handles and kernel 2.33.
fn kernel_descriptors() -> Vec<u32> {
    let mut syscalls = [0u32; 8];
    let allowed = (0x01..=0x3E).chain(0x47..=0x58).chain(0x60..=0x6D).chain(0x70..=0x73).chain(0x75..=0x7D);
    for svc in allowed {
        syscalls[svc / 24] |= 1 << (svc % 24);
    }
    let mut out: Vec<u32> = syscalls
        .iter()
        .enumerate()
        .filter(|(_, mask)| **mask != 0)
        .map(|(i, mask)| 0xF000_0000 | (i as u32) << 24 | mask)
        .collect();
    // mappings: an 11-bit prefix 0b11111111100 (0x7FC); bit 20 marks read-only
    let mapping = |address: u32, read_only: bool| 0xFF80_0000 | (address >> 12) & 0xFFFFF | if read_only { 0x10_0000 } else { 0 };
    out.push(mapping(0x1FF0_0000, false)); // DSP memory (I/O), 1ff00000-1ff7ffff
    out.push(mapping(0x1FF8_0000, false));
    out.push(mapping(0x1F00_0000, true)); // VRAM, read-only, 1f000000-1f5fffff
    out.push(mapping(0x1F60_0000, true));
    // other capabilities: debugging, non-alphanumeric names, writing the
    // shared page, main's arguments, sharing device memory, the special
    // memory arrangement and core 2; application memory
    let other = (1 << 0) | (1 << 2) | (1 << 3) | (1 << 5) | (1 << 6) | (1 << 12) | (1 << 13) | (1 << 8);
    out.push(0xFF00_0000 | other);
    out.push(0xFE00_0000 | 0x200);
    out.push(0xFC00_0000 | 2 << 8 | 33);
    out
}

/// The access control information (0x200 bytes): the ARM11 local
/// capabilities, the kernel's and the ARM9's. `descriptor` writes it as the
/// access descriptor does (the ideal processor as a mask, the priority
/// halved).
fn access_control(p: &Program<'_>, has_romfs: bool, descriptor: bool) -> Vec<u8> {
    let mut a = vec![0u8; 0x200];
    put64(&mut a, 0x00, p.title_id);
    put32(&mut a, 0x08, CORE_VERSION);
    // flags: New 3DS clock and cache, New 3DS memory, then ideal processor
    // 0, affinity mask 1, 64 MiB, then the priority (16, + 32 for apps)
    a[0x0C] = if p.new3ds_speedup { 0b11 } else { 0 };
    a[0x0D] = match p.new3ds_memory {
        New3dsMemory::Legacy => 0,
        New3dsMemory::Mb124 => 1,
        New3dsMemory::Mb178 => 2,
    };
    let ideal: u8 = if descriptor { 1 << 0 } else { 0 };
    a[0x0E] = ideal | (1 << 2);
    a[0x0F] = if descriptor { 48 / 2 } else { 48 };
    a[0x10] = 0x9E; // MaxCpu
    // storage info: no save or extra data; file system access (the
    // template's categories, including the SD card)
    let fs_access: u32 = 0x003F_FFFF;
    put32(&mut a, 0x30 + 0x18, fs_access);
    if !has_romfs {
        a[0x30 + 0x1F] |= 1; // NOT_USE_ROMFS
    }
    for (i, service) in SERVICES.iter().enumerate() {
        a[0x50 + i * 8..0x50 + i * 8 + service.len()].copy_from_slice(service.as_bytes());
    }
    a[0x16F] = 0; // resource limit category: application
    // kernel capabilities, the rest 0xFFFFFFFF
    let kernel = kernel_descriptors();
    for i in 0..28 {
        put32(&mut a, 0x170 + i * 4, kernel.get(i).copied().unwrap_or(0xFFFF_FFFF));
    }
    // ARM9: the template's I/O access, direct SD card access, an SD
    // application; descriptor version 2
    let arm9: u32 = 0xFF | (1 << 9) | (1 << 8);
    put32(&mut a, 0x1F0, arm9);
    a[0x1FF] = 2;
    a
}

/// The extended header and the access descriptor (0x400 bytes each).
fn extended_header(p: &Program<'_>, seg: &[Option<Segment<'_>>; 3], has_romfs: bool) -> Result<(Vec<u8>, Vec<u8>), NcchError> {
    let mut x = vec![0u8; 0x400];
    put_str(&mut x, 0, 8, p.name, "process name")?;
    x[0x0D] = 0b10; // not compressed, installed on the SD card
    x[0x0E..0x10].copy_from_slice(&p.version.to_le_bytes());
    let info = |x: &mut [u8], at: usize, s: &Option<Segment<'_>>| {
        if let Some(s) = s {
            put32(x, at, s.address);
            put32(x, at + 4, s.pages());
            put32(x, at + 8, s.bytes.len() as u32);
        }
    };
    info(&mut x, 0x10, &seg[0]);
    put32(&mut x, 0x1C, p.stack_size);
    info(&mut x, 0x20, &seg[1]);
    info(&mut x, 0x30, &seg[2]);
    let bss = seg[2].as_ref().map_or(0, |s| s.memory - s.bytes.len() as u32);
    put32(&mut x, 0x3C, bss);
    for (i, id) in DEPENDENCIES.iter().enumerate() {
        put64(&mut x, 0x40 + i * 8, *id);
    }
    // system info: no save data; jump ID = the title's own
    put64(&mut x, 0x1C0 + 8, p.title_id);
    x[0x200..].copy_from_slice(&access_control(p, has_romfs, false));

    // the access descriptor: placeholder signature and key, then the
    // access control it allows
    let mut d = vec![0u8; 0x400];
    d[..0x100].fill(0xFF);
    d[0x200..].copy_from_slice(&access_control(p, has_romfs, true));
    Ok((x, d))
}

/// The ExeFS: a 0x200-byte header (ten files: name, offset, size; their
/// hashes, in reverse order), then each file at a 0x200 boundary.
fn exefs(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = vec![0u8; 0x200];
    let mut offset = 0usize;
    for (i, (name, data)) in files.iter().enumerate() {
        out[i * 16..i * 16 + name.len()].copy_from_slice(name.as_bytes());
        put32(&mut out, i * 16 + 8, offset as u32);
        put32(&mut out, i * 16 + 12, data.len() as u32);
        let hash_at = 0xC0 + (9 - i) * 32;
        out[hash_at..hash_at + 32].copy_from_slice(&sha256(data));
        out.resize(0x200 + offset, 0);
        out.extend_from_slice(data);
        offset = align(offset + data.len(), MEDIA_UNIT);
    }
    out.resize(align(out.len(), MEDIA_UNIT), 0);
    out
}

/// The RomFS in its IVFC hash tree: the header and the master hashes (of
/// level 1), then level 3 (the RomFS image), level 1 and level 2, each on
/// a 0x1000 boundary; each level hashes the next one's 0x1000-byte blocks.
/// Returns the image and the size of its hashed header region.
fn ivfc(level3: &[u8]) -> (Vec<u8>, usize) {
    let hashes = |size: usize| size.div_ceil(IVFC_BLOCK) * 32;
    let sizes = {
        let l3 = level3.len();
        let l2 = hashes(l3);
        let l1 = hashes(l2);
        let master = hashes(l1);
        [align(IVFC_HEADER, 0x10) + master, l1, l2, l3]
    };
    // where each level is in the image, and its "logical" offset
    let l0_at = 0;
    let l3_at = align(sizes[0], IVFC_BLOCK);
    let l1_at = l3_at + align(sizes[3], IVFC_BLOCK);
    let l2_at = l1_at + align(sizes[1], IVFC_BLOCK);
    let total = l2_at + align(sizes[2], IVFC_BLOCK);
    let logical = [0, 0, align(sizes[1], IVFC_BLOCK), align(align(sizes[1], IVFC_BLOCK) + sizes[2], IVFC_BLOCK)];
    let mut out = vec![0u8; total];
    out[l3_at..l3_at + level3.len()].copy_from_slice(level3);
    // level 2 hashes level 3, level 1 hashes level 2, the master hashes level 1
    let hash_into = |out: &mut Vec<u8>, data_at: usize, data_len: usize, hashes_at: usize| {
        for block in 0..data_len.div_ceil(IVFC_BLOCK) {
            let start = data_at + block * IVFC_BLOCK;
            let h = sha256(&out[start..start + IVFC_BLOCK]);
            out[hashes_at + block * 32..hashes_at + block * 32 + 32].copy_from_slice(&h);
        }
    };
    hash_into(&mut out, l3_at, sizes[3], l2_at);
    hash_into(&mut out, l2_at, sizes[2], l1_at);
    let master_at = l0_at + align(IVFC_HEADER, 0x10);
    hash_into(&mut out, l1_at, sizes[1], master_at);
    // the header
    out[0..4].copy_from_slice(b"IVFC");
    put32(&mut out, 4, 0x10000);
    put32(&mut out, 8, hashes(sizes[1]) as u32);
    for level in 1..4 {
        let at = 0x0C + (level - 1) * 0x18;
        put64(&mut out, at, logical[level] as u64);
        put64(&mut out, at + 8, sizes[level] as u64);
        put32(&mut out, at + 16, IVFC_BLOCK.trailing_zeros());
    }
    put32(&mut out, 0x54, IVFC_HEADER as u32);
    (out, sizes[0])
}

/// The program as an NCCH (a `.cxi`).
pub fn build(p: &Program<'_>) -> Result<Vec<u8>, NcchError> {
    let elf = Elf::parse(p.elf)?;
    let seg = segments(&elf)?;
    let has_romfs = p.romfs.is_some();
    let (exheader, access_descriptor) = extended_header(p, &seg, has_romfs)?;

    // .code: the segments, each padded to whole pages
    let mut code = Vec::new();
    for s in seg.iter().flatten() {
        code.extend_from_slice(s.bytes);
        code.resize(align(code.len(), PAGE as usize), 0);
    }
    let mut files: Vec<(&str, &[u8])> = vec![(".code", &code)];
    if let Some(banner) = p.banner {
        files.push(("banner", banner));
    }
    files.push(("icon", p.icon));
    let exefs = exefs(&files);
    let romfs = p.romfs.map(ivfc);

    // the layout, in media units
    let exheader_at = 0x200;
    let logo_at = align(exheader_at + 0x800, MEDIA_UNIT);
    let exefs_at = align(logo_at + LOGO.len(), MEDIA_UNIT);
    let romfs_at = align(exefs_at + exefs.len(), 0x1000);
    let size = match &romfs {
        Some((image, _)) => align(romfs_at + image.len(), MEDIA_UNIT),
        None => exefs_at + exefs.len(),
    };
    let mut out = vec![0u8; size];
    out[exheader_at..exheader_at + 0x400].copy_from_slice(&exheader);
    out[exheader_at + 0x400..exheader_at + 0x800].copy_from_slice(&access_descriptor);
    out[logo_at..logo_at + LOGO.len()].copy_from_slice(LOGO);
    out[exefs_at..exefs_at + exefs.len()].copy_from_slice(&exefs);

    let units = |bytes: usize| (bytes / MEDIA_UNIT) as u32;
    let h = &mut out[..0x200];
    h[..0x100].fill(0xFF); // placeholder signature
    h[0x100..0x104].copy_from_slice(b"NCCH");
    put32(h, 0x104, units(size));
    put64(h, 0x108, p.title_id);
    h[0x110..0x112].copy_from_slice(b"00");
    h[0x112..0x114].copy_from_slice(&2u16.to_le_bytes());
    put64(h, 0x118, p.title_id);
    h[0x130..0x150].copy_from_slice(&sha256(LOGO));
    put_str(h, 0x150, 16, p.product_code, "product code")?;
    h[0x160..0x180].copy_from_slice(&sha256(&exheader));
    put32(h, 0x180, 0x400);
    // flags: CTR platform, an executable (with or without RomFS)
    // application, 0x200-byte media units, not encrypted
    h[0x188 + 4] = 1;
    h[0x188 + 5] = if has_romfs { 3 } else { 2 };
    h[0x188 + 6] = 0;
    h[0x188 + 7] = 0b101 | if has_romfs { 0 } else { 0b010 };
    put32(h, 0x198, units(logo_at));
    put32(h, 0x19C, units(LOGO.len()));
    put32(h, 0x1A0, units(exefs_at));
    put32(h, 0x1A4, units(exefs.len()));
    put32(h, 0x1A8, 1);
    h[0x1C0..0x1E0].copy_from_slice(&sha256(&exefs[..0x200]));
    if let Some((image, hashed)) = &romfs {
        let hashed = align(*hashed, MEDIA_UNIT);
        put32(h, 0x1B0, units(romfs_at));
        put32(h, 0x1B4, units(align(image.len(), MEDIA_UNIT)));
        put32(h, 0x1B8, units(hashed));
        h[0x1E0..0x200].copy_from_slice(&sha256(&image[..hashed]));
        out[romfs_at..romfs_at + image.len()].copy_from_slice(image);
    }
    Ok(out)
}

/// The title ID of an application with this unique ID (20 bits).
pub fn application_title_id(unique_id: u32) -> u64 {
    0x0004_0000_0000_0000 | ((unique_id as u64 & 0xF_FFFF) << 8)
}

/// A unique ID for a program that names none: from its name, in the range
/// homebrew titles use (0xF8000-0xFFFFF).
pub fn default_unique_id(name: &str) -> u32 {
    let h = sha256(name.as_bytes());
    0xF8000 | (u32::from_le_bytes([h[0], h[1], h[2], 0]) & 0x7FFF)
}

/// A product code from a program's name: `CTR-H-` and up to four letters
/// or digits.
pub fn default_product_code(name: &str) -> String {
    let mut code = String::from("CTR-H-");
    code.extend(name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).take(4));
    code
}

/// What a CIA's metadata repeats from an NCCH: its extended header's
/// dependency list (0x180 bytes) and its icon (from the ExeFS).
pub fn dependency_list(ncch: &[u8]) -> Option<&[u8]> {
    ncch.get(0x200 + 0x40..0x200 + 0x1C0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ivfc_tree_hashes() {
        let level3: Vec<u8> = (0..0x2345u32).map(|i| (i * 7) as u8).collect();
        let (image, hashed) = ivfc(&level3);
        assert_eq!(&image[..4], b"IVFC");
        assert_eq!(hashed, 0x60 + 32); // one level-1 block
        // level 3 sits at 0x1000, and level 2's first hash is of its first block
        assert_eq!(&image[0x1000..0x1000 + level3.len()], &level3[..]);
        let l2_size = u64::from_le_bytes(image[0x0C + 0x18 + 8..0x0C + 0x18 + 16].try_into().unwrap()) as usize;
        assert_eq!(l2_size, 3 * 32);
        let l1_at = 0x1000 + align(level3.len(), 0x1000);
        let l2_at = l1_at + 0x1000;
        assert_eq!(&image[l2_at..l2_at + 32], &sha256(&image[0x1000..0x2000]));
        assert_eq!(&image[l1_at..l1_at + 32], &sha256(&image[l2_at..l2_at + 0x1000]));
        assert_eq!(&image[0x60..0x80], &sha256(&image[l1_at..l1_at + 0x1000]));
    }

    #[test]
    fn exefs_header() {
        let code = [1u8; 0x300];
        let icon = [2u8; 0x10];
        let fs = exefs(&[(".code", &code), ("icon", &icon)]);
        assert_eq!(&fs[0..5], b".code");
        assert_eq!(u32::from_le_bytes(fs[0x18..0x1C].try_into().unwrap()), 0x400); // icon offset
        assert_eq!(&fs[0xC0 + 9 * 32..0xC0 + 10 * 32], &sha256(&code));
        assert_eq!(&fs[0xC0 + 8 * 32..0xC0 + 9 * 32], &sha256(&icon));
        assert_eq!(&fs[0x200 + 0x400..0x200 + 0x410], &icon);
    }

    #[test]
    fn kernel_capabilities_fit() {
        let k = kernel_descriptors();
        assert!(k.len() <= 28);
        assert_eq!(k.last(), Some(&0xFC00_0221));
        // svc 0x7D (QueryProcessMemory) in the sixth descriptor
        assert!(k.iter().any(|d| d >> 24 == 0xF5 && d & (1 << (0x7D - 120)) != 0));
    }

    #[test]
    fn ids() {
        assert_eq!(application_title_id(0xF8123), 0x0004_0000_0F81_2300);
        let id = default_unique_id("lemon");
        assert!((0xF8000..=0xFFFFF).contains(&id));
        assert_eq!(default_product_code("smb-3ds"), "CTR-H-SMB3");
    }
}
