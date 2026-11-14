//! Just enough of ELF32 (little-endian) to convert an executable.

use alloc::vec::Vec;
use core::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElfError {
    Truncated,
    NotElf,
    Not32BitLittleEndian,
    NotArm,
    NotExecutable,
}

impl fmt::Display for ElfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ElfError::Truncated => "the ELF file is truncated",
            ElfError::NotElf => "not an ELF file",
            ElfError::Not32BitLittleEndian => "not a 32-bit little-endian ELF file",
            ElfError::NotArm => "not an ARM ELF file",
            ElfError::NotExecutable => "not an executable (ET_EXEC) ELF file",
        })
    }
}

pub const PT_LOAD: u32 = 1;
pub const SHT_SYMTAB: u32 = 2;
pub const SHT_RELA: u32 = 4;
pub const SHT_NOBITS: u32 = 8;
pub const SHT_REL: u32 = 9;
pub const SHF_ALLOC: u32 = 2;
pub const STB_WEAK: u8 = 2;

pub const R_ARM_ABS32: u32 = 2;
pub const R_ARM_REL32: u32 = 3;
pub const R_ARM_TARGET1: u32 = 38;
pub const R_ARM_TARGET2: u32 = 41;
pub const R_ARM_PREL31: u32 = 42;
pub const R_ARM_TLS_IE32: u32 = 107;

#[derive(Debug, Clone, Copy)]
pub struct ProgramHeader {
    pub kind: u32,
    pub offset: u32,
    pub vaddr: u32,
    pub file_size: u32,
    pub mem_size: u32,
    pub flags: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct SectionHeader {
    pub name: u32,
    pub kind: u32,
    pub flags: u32,
    pub addr: u32,
    pub offset: u32,
    pub size: u32,
    pub link: u32,
    pub info: u32,
    pub entsize: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Symbol {
    pub name: u32,
    pub value: u32,
    pub size: u32,
    pub info: u8,
    pub shndx: u16,
}

impl Symbol {
    pub fn binding(&self) -> u8 {
        self.info >> 4
    }
}

pub struct Elf<'a> {
    pub data: &'a [u8],
    pub entry: u32,
    pub programs: Vec<ProgramHeader>,
    pub sections: Vec<SectionHeader>,
    shstrndx: usize,
}

pub(crate) fn u16_at(d: &[u8], o: usize) -> Result<u16, ElfError> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or(ElfError::Truncated)
}

pub(crate) fn u32_at(d: &[u8], o: usize) -> Result<u32, ElfError> {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(ElfError::Truncated)
}

impl<'a> Elf<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Elf<'a>, ElfError> {
        if data.len() < 52 {
            return Err(ElfError::Truncated);
        }
        if &data[0..4] != b"\x7fELF" {
            return Err(ElfError::NotElf);
        }
        if data[4] != 1 || data[5] != 1 {
            return Err(ElfError::Not32BitLittleEndian);
        }
        if u16_at(data, 16)? != 2 {
            return Err(ElfError::NotExecutable);
        }
        if u16_at(data, 18)? != 40 {
            return Err(ElfError::NotArm);
        }
        let entry = u32_at(data, 24)?;
        let phoff = u32_at(data, 28)? as usize;
        let shoff = u32_at(data, 32)? as usize;
        let phentsize = u16_at(data, 42)? as usize;
        let phnum = u16_at(data, 44)? as usize;
        let shentsize = u16_at(data, 46)? as usize;
        let shnum = u16_at(data, 48)? as usize;
        let shstrndx = u16_at(data, 50)? as usize;

        let mut programs = Vec::with_capacity(phnum);
        for i in 0..phnum {
            let o = phoff + i * phentsize;
            programs.push(ProgramHeader {
                kind: u32_at(data, o)?,
                offset: u32_at(data, o + 4)?,
                vaddr: u32_at(data, o + 8)?,
                file_size: u32_at(data, o + 16)?,
                mem_size: u32_at(data, o + 20)?,
                flags: u32_at(data, o + 24)?,
            });
        }
        let mut sections = Vec::with_capacity(shnum);
        for i in 0..shnum {
            let o = shoff + i * shentsize;
            sections.push(SectionHeader {
                name: u32_at(data, o)?,
                kind: u32_at(data, o + 4)?,
                flags: u32_at(data, o + 8)?,
                addr: u32_at(data, o + 12)?,
                offset: u32_at(data, o + 16)?,
                size: u32_at(data, o + 20)?,
                link: u32_at(data, o + 24)?,
                info: u32_at(data, o + 28)?,
                entsize: u32_at(data, o + 36)?,
            });
        }
        Ok(Elf { data, entry, programs, sections, shstrndx })
    }

    pub fn section_data(&self, s: &SectionHeader) -> Result<&'a [u8], ElfError> {
        if s.kind == SHT_NOBITS {
            return Ok(&[]);
        }
        self.data
            .get(s.offset as usize..(s.offset as usize + s.size as usize))
            .ok_or(ElfError::Truncated)
    }

    pub fn section_name(&self, s: &SectionHeader) -> &'a str {
        let Some(strtab) = self.sections.get(self.shstrndx) else { return "" };
        let Ok(names) = self.section_data(strtab) else { return "" };
        cstr(names, s.name as usize)
    }

    pub fn symbols(&self, symtab: &SectionHeader) -> Result<Vec<Symbol>, ElfError> {
        let data = self.section_data(symtab)?;
        let mut out = Vec::with_capacity(data.len() / 16);
        for chunk in data.chunks_exact(16) {
            out.push(Symbol {
                name: u32_at(chunk, 0)?,
                value: u32_at(chunk, 4)?,
                size: u32_at(chunk, 8)?,
                info: chunk[12],
                shndx: u16_at(chunk, 14)?,
            });
        }
        Ok(out)
    }

    /// The symbol table's name for a symbol.
    pub fn symbol_name(&self, symtab: &SectionHeader, sym: &Symbol) -> &'a str {
        let Some(strtab) = self.sections.get(symtab.link as usize) else { return "" };
        let Ok(names) = self.section_data(strtab) else { return "" };
        cstr(names, sym.name as usize)
    }

    /// Looks up a symbol's address by name.
    pub fn find_symbol(&self, name: &str) -> Option<u32> {
        let symtab = self.sections.iter().find(|s| s.kind == SHT_SYMTAB)?;
        let syms = self.symbols(symtab).ok()?;
        syms.iter().find(|s| self.symbol_name(symtab, s) == name).map(|s| s.value)
    }
}

fn cstr(data: &[u8], start: usize) -> &str {
    let Some(rest) = data.get(start..) else { return "" };
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    core::str::from_utf8(&rest[..end]).unwrap_or("")
}
