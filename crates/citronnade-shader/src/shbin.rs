//! SHBIN files (a DVLB holding one DVLP, the code, and DVLEs, the entry
//! points with their tables), as picasso writes them and libctru reads them.
//! One program here is one DVLE.

use alloc::string::String;
use alloc::vec::Vec;

use crate::{f24_to_f32, f32_to_f24, Constant, GeometryMode, Output, OutputKind, Program, ShaderKind, Uniform, UniformKind};

const DVLB: u32 = 0x424C_5644;
const DVLP: u32 = 0x504C_5644;
const DVLE: u32 = 0x454C_5644;

/// Why a SHBIN could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShbinError {
    /// The data is too short or a table points outside it.
    Truncated,
    /// A magic number is wrong.
    BadMagic,
    /// The file has no entry point.
    NoEntry,
    /// An output or constant entry is malformed.
    BadEntry,
}

impl core::fmt::Display for ShbinError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            ShbinError::Truncated => "truncated shader binary",
            ShbinError::BadMagic => "not a shader binary (DVLB)",
            ShbinError::NoEntry => "shader binary has no entry point (DVLE)",
            ShbinError::BadEntry => "malformed shader binary table entry",
        })
    }
}

fn push_u16s(out: &mut Vec<u8>, a: u16, b: u16) {
    out.extend_from_slice(&a.to_le_bytes());
    out.extend_from_slice(&b.to_le_bytes());
}

fn push(out: &mut Vec<u8>, w: u32) {
    out.extend_from_slice(&w.to_le_bytes());
}

fn output_kind_code(kind: OutputKind) -> u16 {
    kind as u16
}

fn output_kind(code: u16) -> Option<OutputKind> {
    Some(match code {
        0 => OutputKind::Position,
        1 => OutputKind::NormalQuat,
        2 => OutputKind::Color,
        3 => OutputKind::TexCoord0,
        4 => OutputKind::TexCoord0W,
        5 => OutputKind::TexCoord1,
        6 => OutputKind::TexCoord2,
        8 => OutputKind::View,
        9 => OutputKind::Dummy,
        _ => return None,
    })
}

// Uniform table register numbers: floats 0x10+, integers 0x70+, booleans 0x78+.
fn uniform_reg(u: &Uniform) -> u16 {
    u.first as u16
        + match u.kind {
            UniformKind::Float => 0x10,
            UniformKind::Int => 0x70,
            UniformKind::Bool => 0x78,
        }
}

impl Program {
    /// Writes the program as a SHBIN file (one DVLP, one DVLE), the format
    /// picasso writes and libctru's `DVLB_ParseFile` reads.
    pub fn to_shbin(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let dvlp_size = 10 * 4 + self.code.len() * 4 + self.opdescs.len() * 8;
        push(&mut out, DVLB);
        push(&mut out, 1);
        push(&mut out, (3 * 4 + dvlp_size) as u32);

        // DVLP
        push(&mut out, DVLP);
        push(&mut out, 0);
        push(&mut out, 10 * 4);
        push(&mut out, self.code.len() as u32);
        push(&mut out, (10 * 4 + self.code.len() * 4) as u32);
        push(&mut out, self.opdescs.len() as u32);
        push(&mut out, dvlp_size as u32);
        push(&mut out, 0);
        push(&mut out, 0);
        push(&mut out, 0);
        for &w in &self.code {
            push(&mut out, w);
        }
        for &d in &self.opdescs {
            push(&mut out, d);
            push(&mut out, 0);
        }

        // DVLE
        let mut uniforms: Vec<&Uniform> = self.uniforms.iter().collect();
        uniforms.sort_by_key(|u| uniform_reg(u));
        let mut symbols = Vec::new();
        let mut symbol_offsets = Vec::new();
        for u in &uniforms {
            symbol_offsets.push(symbols.len() as u32);
            symbols.extend_from_slice(u.name.as_bytes());
            symbols.push(0);
        }
        let consts_off = 16 * 4;
        let outs_off = consts_off + self.constants.len() * 20;
        let unis_off = outs_off + self.outputs.len() * 8;
        let syms_off = unis_off + uniforms.len() * 8;
        let output_mask = self.outputs.iter().fold(0u16, |m, o| m | 1 << o.register);
        let geo = self.geometry.unwrap_or(GeometryMode { mode: 0, fixed_start: 0, variable_count: 0, fixed_count: 0 });
        let merge = self.outputs.iter().any(|o| o.kind == OutputKind::Dummy);
        push(&mut out, DVLE);
        push(&mut out, 0x1002 | ((self.kind == ShaderKind::Geometry) as u32) << 16 | (merge as u32) << 24);
        push(&mut out, self.entry);
        push(&mut out, self.entry_end);
        push_u16s(&mut out, self.input_mask, output_mask);
        out.extend_from_slice(&[geo.mode, geo.fixed_start, geo.variable_count, geo.fixed_count]);
        push(&mut out, consts_off as u32);
        push(&mut out, self.constants.len() as u32);
        push(&mut out, outs_off as u32);
        push(&mut out, 0);
        push(&mut out, outs_off as u32);
        push(&mut out, self.outputs.len() as u32);
        push(&mut out, unis_off as u32);
        push(&mut out, uniforms.len() as u32);
        push(&mut out, syms_off as u32);
        push(&mut out, symbols.len() as u32);
        for c in &self.constants {
            match *c {
                Constant::Bool { register, value } => {
                    push_u16s(&mut out, 0, register as u16);
                    for w in [value as u32, 0, 0, 0] {
                        push(&mut out, w);
                    }
                }
                Constant::Int { register, value } => {
                    push_u16s(&mut out, 1, register as u16);
                    for w in [u32::from_le_bytes(value), 0, 0, 0] {
                        push(&mut out, w);
                    }
                }
                Constant::Float { register, value } => {
                    push_u16s(&mut out, 2, register as u16);
                    for v in value {
                        push(&mut out, f32_to_f24(v));
                    }
                }
            }
        }
        for o in &self.outputs {
            push_u16s(&mut out, output_kind_code(o.kind), o.register as u16);
            push(&mut out, o.mask as u32);
        }
        for (u, off) in uniforms.iter().zip(&symbol_offsets) {
            push(&mut out, *off);
            let reg = uniform_reg(u);
            push_u16s(&mut out, reg, reg + u.count as u16 - 1);
        }
        out.extend_from_slice(&symbols);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }

    /// Reads the first entry point (DVLE) of a SHBIN file.
    pub fn from_shbin(data: &[u8]) -> Result<Program, ShbinError> {
        Self::from_shbin_entry(data, 0)
    }

    /// The number of entry points (DVLEs) in a SHBIN file.
    pub fn shbin_entries(data: &[u8]) -> Result<usize, ShbinError> {
        let r = Reader(data);
        if r.word(0)? != DVLB {
            return Err(ShbinError::BadMagic);
        }
        Ok(r.word(4)? as usize)
    }

    /// Reads one entry point (DVLE) of a SHBIN file.
    pub fn from_shbin_entry(data: &[u8], index: usize) -> Result<Program, ShbinError> {
        let r = Reader(data);
        if r.word(0)? != DVLB {
            return Err(ShbinError::BadMagic);
        }
        let count = r.word(4)? as usize;
        if index >= count {
            return Err(ShbinError::NoEntry);
        }
        let dvlp = 8 + count * 4;
        if r.word(dvlp)? != DVLP {
            return Err(ShbinError::BadMagic);
        }
        let code_off = dvlp + r.word(dvlp + 8)? as usize;
        let code_len = r.word(dvlp + 12)? as usize;
        let op_off = dvlp + r.word(dvlp + 16)? as usize;
        let op_len = r.word(dvlp + 20)? as usize;
        let code = (0..code_len).map(|i| r.word(code_off + i * 4)).collect::<Result<Vec<_>, _>>()?;
        let opdescs = (0..op_len).map(|i| r.word(op_off + i * 8)).collect::<Result<Vec<_>, _>>()?;

        let e = r.word(8 + index * 4)? as usize;
        if r.word(e)? != DVLE {
            return Err(ShbinError::BadMagic);
        }
        let flags = r.word(e + 4)?;
        let kind = if (flags >> 16) & 0xFF == 1 { ShaderKind::Geometry } else { ShaderKind::Vertex };
        let entry = r.word(e + 8)?;
        let entry_end = r.word(e + 12)?;
        let input_mask = r.word(e + 16)? as u16;
        let g = r.word(e + 20)?.to_le_bytes();
        let geometry = (kind == ShaderKind::Geometry)
            .then_some(GeometryMode { mode: g[0], fixed_start: g[1], variable_count: g[2], fixed_count: g[3] });

        let mut constants = Vec::new();
        let (c_off, c_len) = (e + r.word(e + 24)? as usize, r.word(e + 28)? as usize);
        for i in 0..c_len {
            let at = c_off + i * 20;
            let head = r.word(at)?;
            let (ty, reg) = (head & 0xFFFF, (head >> 16) as u8);
            let d = |j: usize| r.word(at + 4 + j * 4);
            constants.push(match ty {
                0 => Constant::Bool { register: reg, value: d(0)? & 1 != 0 },
                1 => Constant::Int { register: reg, value: d(0)?.to_le_bytes() },
                2 => Constant::Float { register: reg, value: [f24_to_f32(d(0)?), f24_to_f32(d(1)?), f24_to_f32(d(2)?), f24_to_f32(d(3)?)] },
                _ => return Err(ShbinError::BadEntry),
            });
        }

        let mut outputs = Vec::new();
        let (o_off, o_len) = (e + r.word(e + 40)? as usize, r.word(e + 44)? as usize);
        for i in 0..o_len {
            let head = r.word(o_off + i * 8)?;
            let kind = output_kind((head & 0xFFFF) as u16).ok_or(ShbinError::BadEntry)?;
            outputs.push(Output { kind, register: (head >> 16) as u8, mask: r.word(o_off + i * 8 + 4)? as u8 });
        }

        let mut uniforms = Vec::new();
        let (u_off, u_len) = (e + r.word(e + 48)? as usize, r.word(e + 52)? as usize);
        let s_off = e + r.word(e + 56)? as usize;
        for i in 0..u_len {
            let sym = r.word(u_off + i * 8)? as usize;
            let regs = r.word(u_off + i * 8 + 4)?;
            let (start, end) = ((regs & 0xFFFF) as u16, (regs >> 16) as u16);
            let (kind, first) = match start {
                0x10..=0x6F => (UniformKind::Float, start - 0x10),
                0x70..=0x73 => (UniformKind::Int, start - 0x70),
                0x78..=0x87 => (UniformKind::Bool, start - 0x78),
                _ => continue, // inputs (`.in` names) and others
            };
            let name_at = s_off + sym;
            let bytes = data.get(name_at..).ok_or(ShbinError::Truncated)?;
            let len = bytes.iter().position(|&b| b == 0).ok_or(ShbinError::Truncated)?;
            let name = String::from_utf8_lossy(&bytes[..len]).into_owned();
            uniforms.push(Uniform { name, kind, first: first as u8, count: (end.saturating_sub(start) + 1) as u8 });
        }

        Ok(Program { kind, code, opdescs, entry, entry_end, outputs, uniforms, constants, input_mask, geometry })
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn word(&self, at: usize) -> Result<u32, ShbinError> {
        let b = self.0.get(at..at + 4).ok_or(ShbinError::Truncated)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

#[cfg(test)]
mod tests {
    use crate::assemble;

    #[test]
    fn round_trips_through_shbin() {
        let src = r#"
            .fvec projection[4], tint
            .ivec loops
            .bool flag
            .constf half(0.5, 0.5, 1.0, -2.0)
            .consti counts(3, 0, 1, 0)
            .in pos v0
            .in clr v1
            .out outpos position
            .out outclr color
            .out outtc texcoord0
            .proc main
                dp4 outpos.x, projection[0], pos
                dp4 outpos.y, projection[1], pos
                dp4 outpos.z, projection[2], pos
                dp4 outpos.w, projection[3], pos
                mul outclr, tint, clr
                mov outtc, half
                end
            .end
        "#;
        let p = assemble(src).unwrap();
        let bin = p.to_shbin();
        assert_eq!(&bin[0..4], b"DVLB");
        assert_eq!(bin.len() % 4, 0);
        let q = super::Program::from_shbin(&bin).unwrap();
        assert_eq!(q.code, p.code);
        assert_eq!(q.opdescs, p.opdescs);
        assert_eq!(q.entry, p.entry);
        assert_eq!(q.outputs, p.outputs);
        assert_eq!(q.constants, p.constants);
        assert_eq!(q.input_mask, 0b11);
        let mut a = p.uniforms.clone();
        a.sort_by_key(|u| (u.kind as u8, u.first));
        let mut b = q.uniforms.clone();
        b.sort_by_key(|u| (u.kind as u8, u.first));
        assert_eq!(a, b);
        assert_eq!(q.output_map(), p.output_map());
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(super::Program::from_shbin(b"nope"), Err(super::ShbinError::BadMagic));
        assert_eq!(super::Program::from_shbin(b"DV"), Err(super::ShbinError::Truncated));
    }
}
