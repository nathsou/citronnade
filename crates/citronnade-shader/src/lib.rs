//! An assembler for the PICA200's vertex and geometry shaders.
//!
//! The syntax follows picasso, the assembler most 3DS homebrew uses, so
//! existing shader sources assemble the same way:
//!
//! ```text
//! ; uniforms, constants, aliases
//! .fvec projection[4]
//! .constf ones(1.0, 1.0, 1.0, 1.0)
//! .alias  inpos v0
//! .alias  inclr v1
//! ; outputs
//! .out outpos position
//! .out outclr color
//!
//! .proc main
//!     mov r0.xyz, inpos
//!     mov r0.w, ones
//!     dp4 outpos.x, projection[0], r0
//!     dp4 outpos.y, projection[1], r0
//!     dp4 outpos.z, projection[2], r0
//!     dp4 outpos.w, projection[3], r0
//!     mov outclr, inclr
//!     end
//! .end
//! ```
//!
//! Registers: inputs `v0`-`v15`, temporaries `r0`-`r15`, float uniforms
//! `c0`-`c95`, integer uniforms `i0`-`i3`, boolean uniforms `b0`-`b15`,
//! outputs `o0`-`o15`, address registers `a0.x`, `a0.y` and the loop
//! counter `aL` (for relative addressing: `c[a0.x + 2]`).
//!
//! Instructions: `add dp3 dp4 dph dst ex2 lg2 litp mul sge slt flr max min
//! rcp rsq mova mov mad cmp nop end emit setemit break breakc call callc
//! callu ifc ifu for jmpc jmpu`, with `.else` / `.end` closing `ifc`, `ifu`
//! and `for` blocks. Short swizzles repeat their last component (`.xy` is
//! `.xyyy`), as in picasso.
//!
//! The result is a [`Program`]: code words, operand descriptors, the entry
//! point, the output map and the uniform table, ready to upload.
#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

mod shbin;
pub use shbin::ShbinError;

mod opcodes {
    pub const ADD: u32 = 0x00;
    pub const DP3: u32 = 0x01;
    pub const DP4: u32 = 0x02;
    pub const DPH: u32 = 0x03;
    pub const DST: u32 = 0x04;
    pub const EX2: u32 = 0x05;
    pub const LG2: u32 = 0x06;
    pub const LITP: u32 = 0x07;
    pub const MUL: u32 = 0x08;
    pub const SGE: u32 = 0x09;
    pub const SLT: u32 = 0x0A;
    pub const FLR: u32 = 0x0B;
    pub const MAX: u32 = 0x0C;
    pub const MIN: u32 = 0x0D;
    pub const RCP: u32 = 0x0E;
    pub const RSQ: u32 = 0x0F;
    pub const MOVA: u32 = 0x12;
    pub const MOV: u32 = 0x13;
    pub const DPHI: u32 = 0x18;
    pub const DSTI: u32 = 0x19;
    pub const SGEI: u32 = 0x1A;
    pub const SLTI: u32 = 0x1B;
    pub const BREAK: u32 = 0x20;
    pub const NOP: u32 = 0x21;
    pub const END: u32 = 0x22;
    pub const BREAKC: u32 = 0x23;
    pub const CALL: u32 = 0x24;
    pub const CALLC: u32 = 0x25;
    pub const CALLU: u32 = 0x26;
    pub const IFU: u32 = 0x27;
    pub const IFC: u32 = 0x28;
    pub const FOR: u32 = 0x29;
    pub const EMIT: u32 = 0x2A;
    pub const SETEMIT: u32 = 0x2B;
    pub const JMPC: u32 = 0x2C;
    pub const JMPU: u32 = 0x2D;
    pub const CMP: u32 = 0x2E;
    pub const MADI: u32 = 0x30;
    pub const MAD: u32 = 0x38;
}

/// Vertex or geometry shader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderKind {
    Vertex,
    Geometry,
}

/// What an output register carries (its semantic).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputKind {
    Position = 0,
    NormalQuat = 1,
    Color = 2,
    TexCoord0 = 3,
    TexCoord0W = 4,
    TexCoord1 = 5,
    TexCoord2 = 6,
    View = 8,
    /// Passed to a geometry shader without meaning to the rasteriser.
    Dummy = 9,
}

impl OutputKind {
    fn parse(s: &str) -> Option<OutputKind> {
        Some(match s.to_ascii_lowercase().as_str() {
            "position" | "pos" => OutputKind::Position,
            "normalquat" | "nquat" => OutputKind::NormalQuat,
            "color" | "clr" => OutputKind::Color,
            "texcoord0" | "tcoord0" => OutputKind::TexCoord0,
            "texcoord0w" | "tcoord0w" => OutputKind::TexCoord0W,
            "texcoord1" | "tcoord1" => OutputKind::TexCoord1,
            "texcoord2" | "tcoord2" => OutputKind::TexCoord2,
            "view" => OutputKind::View,
            "dummy" => OutputKind::Dummy,
            _ => return None,
        })
    }

    /// The default number of components.
    fn components(self) -> u32 {
        match self {
            OutputKind::Position | OutputKind::NormalQuat | OutputKind::Color | OutputKind::Dummy => 4,
            OutputKind::TexCoord0 | OutputKind::TexCoord1 | OutputKind::TexCoord2 => 2,
            OutputKind::TexCoord0W => 1,
            OutputKind::View => 3,
        }
    }
}

/// An output register and its semantic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub kind: OutputKind,
    pub register: u8,
    /// Components written: bit 0 = x ... bit 3 = w.
    pub mask: u8,
}

/// Uniform kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UniformKind {
    Float,
    Int,
    Bool,
}

/// A named uniform (or uniform array).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uniform {
    pub name: String,
    pub kind: UniformKind,
    /// The first register (`c`, `i` or `b` index).
    pub first: u8,
    pub count: u8,
}

/// A constant the program sets before running.
#[derive(Clone, Debug, PartialEq)]
pub enum Constant {
    Float { register: u8, value: [f32; 4] },
    Int { register: u8, value: [u8; 4] },
    Bool { register: u8, value: bool },
}

/// Geometry shader configuration (as in a DVLE header).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryMode {
    /// 0 point, 1 variable-size primitives, 2 fixed-size primitives.
    pub mode: u8,
    /// The first float uniform receiving vertex data (fixed-size mode).
    pub fixed_start: u8,
    /// Fully defined vertices per primitive (variable-size mode).
    pub variable_count: u8,
    /// Vertices per primitive (fixed-size mode).
    pub fixed_count: u8,
}

/// An assembled shader.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub kind: ShaderKind,
    pub code: Vec<u32>,
    pub opdescs: Vec<u32>,
    /// Offsets of the entry point and the end of the entry procedure.
    pub entry: u32,
    pub entry_end: u32,
    pub outputs: Vec<Output>,
    pub uniforms: Vec<Uniform>,
    pub constants: Vec<Constant>,
    /// Input registers the program reads: bit n = v<n>.
    pub input_mask: u16,
    pub geometry: Option<GeometryMode>,
}

impl Program {
    /// Looks up a uniform's first register.
    pub fn uniform(&self, name: &str) -> Option<&Uniform> {
        self.uniforms.iter().find(|u| u.name == name)
    }

    /// The output map the GPU needs: [count, then for each output register
    /// four 5-bit semantic codes (0x1F unused)], plus the mask of output
    /// registers, the output mode and the clock control bits.
    pub fn output_map(&self) -> ([u32; 8], u32, u32, u32) {
        let mut data = [0x1F1F_1F1Fu32; 8];
        data[0] = 0;
        let (mut mask, mut mode, mut clock) = (0u32, 0u32, 0u32);
        for o in &self.outputs {
            let reg = o.register as usize;
            if reg >= 7 {
                continue;
            }
            if mask & (1 << reg) == 0 {
                mask |= 1 << reg;
                data[0] += 1;
            }
            let (mut sem, num) = match o.kind {
                OutputKind::Position => (0x00, 4),
                OutputKind::NormalQuat => {
                    clock |= 1 << 24;
                    (0x04, 4)
                }
                OutputKind::Color => {
                    clock |= 1 << 1;
                    (0x08, 4)
                }
                OutputKind::TexCoord0 => {
                    clock |= 1 << 8;
                    mode = 1;
                    (0x0C, 2)
                }
                OutputKind::TexCoord0W => {
                    clock |= 1 << 16;
                    mode = 1;
                    (0x10, 1)
                }
                OutputKind::TexCoord1 => {
                    clock |= 1 << 9;
                    mode = 1;
                    (0x0E, 2)
                }
                OutputKind::TexCoord2 => {
                    clock |= 1 << 10;
                    mode = 1;
                    (0x16, 2)
                }
                OutputKind::View => {
                    clock |= 1 << 24;
                    (0x12, 3)
                }
                OutputKind::Dummy => continue,
            };
            let out = &mut data[reg + 1];
            let mut written = 0;
            for j in 0..4 {
                if written >= num {
                    break;
                }
                if o.mask & (1 << j) != 0 {
                    *out &= !(0xFF << (j * 8));
                    *out |= (sem as u32) << (j * 8);
                    sem += 1;
                    written += 1;
                    if o.kind == OutputKind::Position && written == 3 {
                        clock |= 1;
                    }
                }
            }
        }
        (data, mask, mode, clock)
    }
}

/// An assembly error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub line: usize,
    pub message: String,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

// Register numbers in instructions.
const REG_INPUT: u32 = 0x00;
const REG_TEMP: u32 = 0x10;
const REG_FLOAT: u32 = 0x20;
const REG_INT: u32 = 0x80;
const REG_BOOL: u32 = 0x88;
const REG_ADDR: u32 = 0x98; // a0 (pseudo, for mova)

const DEFAULT_SWIZZLE: u32 = 0x1B; // xyzw

/// A parsed operand.
#[derive(Clone, Copy, Debug)]
struct Operand {
    reg: u32,
    swizzle: u32, // 8 bits
    negate: bool,
    index: u32, // 0 none, 1 a0.x, 2 a0.y, 3 aL
    /// For destinations: whether the register is an output.
    output: bool,
}

impl Operand {
    fn src_bits(&self) -> u32 {
        (self.negate as u32) | (self.swizzle << 1)
    }

    fn dest_mask(&self) -> u32 {
        // Components named in the swizzle (repeats count once), x = bit 3.
        let mut m = 0;
        for i in 0..4 {
            let c = (self.swizzle >> (6 - 2 * i)) & 3;
            m |= 1 << (3 - c);
        }
        m
    }

    fn is_low(&self) -> bool {
        self.reg < 0x20 && !self.output
    }
}

#[derive(Clone)]
enum Alias {
    Register { reg: u32, swizzle: u32, negate: bool, output: bool },
}

struct Block {
    kind: BlockKind,
    start: usize,
    else_at: Option<usize>,
    name: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Proc,
    If,
    For,
}

/// An instruction before operand descriptors get their final indices.
struct Pending {
    word: u32,
    opdesc: Option<(u32, bool)>, // descriptor value, used by mad
    line: usize,
}

struct Assembler {
    kind: ShaderKind,
    code: Vec<Pending>,
    aliases: BTreeMap<String, Alias>,
    outputs: Vec<Output>,
    uniforms: Vec<Uniform>,
    constants: Vec<Constant>,
    input_mask: core::cell::Cell<u16>,
    next_float: u32,
    next_int: u32,
    next_bool: u32,
    next_output: u32,
    next_input: u32,
    blocks: Vec<Block>,
    procs: BTreeMap<String, (usize, usize)>,
    labels: BTreeMap<String, usize>,
    proc_fixups: Vec<(usize, String, usize)>,
    label_fixups: Vec<(usize, String, usize)>,
    last_was_end: bool,
    entry_name: String,
    geometry: Option<GeometryMode>,
}

fn err<T>(line: usize, message: impl Into<String>) -> Result<T, Error> {
    Err(Error { line, message: message.into() })
}

fn parse_swizzle(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 4 {
        return None;
    }
    let mut out = 0;
    let mut last = 0;
    let mut i = 0;
    for ch in s.chars() {
        last = match ch.to_ascii_lowercase() {
            'x' | 'r' | 's' => 0,
            'y' | 'g' | 't' => 1,
            'z' | 'b' | 'p' => 2,
            'w' | 'a' | 'q' => 3,
            _ => return None,
        };
        out |= last << (6 - 2 * i);
        i += 1;
    }
    for j in i..4 {
        out |= last << (6 - 2 * j);
    }
    Some(out)
}

fn compose(outer: u32, inner: u32) -> u32 {
    // outer selects components of inner
    let mut out = 0;
    for j in 0..4 {
        let comp = (outer >> (6 - 2 * j)) & 3;
        let sel = (inner >> (6 - 2 * comp)) & 3;
        out |= sel << (6 - 2 * j);
    }
    out
}

fn parse_number(s: &str) -> Option<f32> {
    s.trim().parse::<f32>().ok()
}

fn split_args(s: &str) -> Vec<String> {
    // Split on commas outside brackets/parentheses.
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for ch in s.chars() {
        match ch {
            '[' | '(' => {
                depth += 1;
                cur.push(ch);
            }
            ']' | ')' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

impl Assembler {
    fn new(kind: ShaderKind) -> Self {
        Assembler {
            kind,
            code: Vec::new(),
            aliases: BTreeMap::new(),
            outputs: Vec::new(),
            uniforms: Vec::new(),
            constants: Vec::new(),
            input_mask: core::cell::Cell::new(0),
            next_float: 0,
            next_int: 0,
            next_bool: 0,
            next_output: 0,
            next_input: 0,
            blocks: Vec::new(),
            procs: BTreeMap::new(),
            labels: BTreeMap::new(),
            proc_fixups: Vec::new(),
            label_fixups: Vec::new(),
            last_was_end: false,
            entry_name: "main".to_string(),
            geometry: None,
        }
    }

    fn emit(&mut self, word: u32, opdesc: Option<(u32, bool)>, line: usize) {
        self.code.push(Pending { word, opdesc, line });
        self.last_was_end = false;
    }

    fn pc(&self) -> usize {
        self.code.len()
    }

    fn nop(&mut self, line: usize) {
        self.emit(opcodes::NOP << 26, None, line);
    }

    fn define_name(&self, line: usize, name: &str) -> Result<(), Error> {
        if self.aliases.contains_key(name) {
            return err(line, format!("`{name}` is already defined"));
        }
        if name.is_empty() || !name.chars().next().unwrap().is_ascii_alphabetic() && !name.starts_with('_') {
            return err(line, format!("`{name}` is not a valid name"));
        }
        Ok(())
    }

    /// Parses a register reference: `[-]name[index][.swizzle]`.
    fn operand(&self, line: usize, text: &str) -> Result<Operand, Error> {
        let mut s = text.trim();
        let mut negate = false;
        while let Some(rest) = s.strip_prefix('-') {
            negate = !negate;
            s = rest.trim_start();
        }
        // Split the swizzle (after the last '.' outside brackets).
        let (base, swz) = match s.rfind('.') {
            Some(i) if !s[i..].contains(']') => (&s[..i], Some(&s[i + 1..])),
            _ => (s, None),
        };
        let (name, index_expr) = match base.find('[') {
            Some(i) => {
                if !base.ends_with(']') {
                    return err(line, format!("malformed index in `{text}`"));
                }
                (&base[..i], Some(&base[i + 1..base.len() - 1]))
            }
            None => (base, None),
        };
        let mut index = 0u32;
        let mut offset = 0u32;
        if let Some(expr) = index_expr {
            for part in expr.split('+') {
                let part = part.trim().to_ascii_lowercase();
                match part.as_str() {
                    "a0.x" | "a0" => index = 1,
                    "a0.y" | "a1" => index = 2,
                    "al" | "lcnt" | "a2" => index = 3,
                    p => match p.parse::<u32>() {
                        Ok(n) => offset += n,
                        Err(_) => return err(line, format!("bad index `{part}` in `{text}`")),
                    },
                }
            }
        }
        let (mut reg, mut swizzle, alias_neg, mut output);
        if let Some(Alias::Register { reg: r, swizzle: sw, negate: n, output: o }) = self.aliases.get(name) {
            reg = *r;
            swizzle = *sw;
            alias_neg = *n;
            output = *o;
        } else {
            swizzle = DEFAULT_SWIZZLE;
            alias_neg = false;
            output = false;
            let lower = name.to_ascii_lowercase();
            let (kind, digits) = lower.split_at(1.min(lower.len()));
            let n: u32 = match digits.parse() {
                Ok(n) => n,
                Err(_) => return err(line, format!("unknown register or name `{name}`")),
            };
            reg = match kind {
                "v" if n < 16 => REG_INPUT + n,
                "r" if n < 16 => REG_TEMP + n,
                "c" if n < 96 => REG_FLOAT + n,
                "i" if n < 4 => REG_INT + n,
                "b" if n < 16 => REG_BOOL + n,
                "o" if n < 16 => {
                    output = true;
                    n
                }
                "a" if n < 2 => REG_ADDR + n,
                _ => return err(line, format!("invalid register `{name}`")),
            };
        }
        reg += offset;
        if index != 0 && !(REG_FLOAT..REG_INT).contains(&reg) {
            return err(line, format!("relative addressing needs a float uniform: `{text}`"));
        }
        if let Some(sw) = swz {
            let parsed = parse_swizzle(sw).ok_or_else(|| Error { line, message: format!("bad swizzle `.{sw}`") })?;
            swizzle = compose(parsed, swizzle);
        }
        Ok(Operand { reg, swizzle, negate: negate ^ alias_neg, index, output })
    }

    fn dest(&self, line: usize, text: &str) -> Result<Operand, Error> {
        let op = self.operand(line, text)?;
        if op.negate || op.index != 0 {
            return err(line, format!("a destination cannot be negated or indexed: `{text}`"));
        }
        let ok = op.output || (REG_TEMP..REG_TEMP + 16).contains(&op.reg);
        if !ok {
            return err(line, format!("`{text}` is not a writable register (o0-o15 or r0-r15)"));
        }
        Ok(op)
    }

    fn src(&self, line: usize, text: &str) -> Result<Operand, Error> {
        let op = self.operand(line, text)?;
        if op.output || op.reg >= REG_INT {
            return err(line, format!("`{text}` cannot be read here"));
        }
        if op.reg < REG_TEMP {
            self.input_mask.set(self.input_mask.get() | 1 << op.reg);
        }
        Ok(op)
    }

    fn dest_reg(op: &Operand) -> u32 {
        if op.output { op.reg } else { op.reg }
    }

    fn arith(&mut self, line: usize, name: &str, args: &[String]) -> Result<(), Error> {
        use opcodes::*;
        let two = |op: u32| -> Option<u32> { Some(op) };
        let (op, inverted_op, commutative, sources) = match name {
            "add" => (ADD, None, true, 2),
            "dp3" => (DP3, None, true, 2),
            "dp4" => (DP4, None, true, 2),
            "dph" => (DPH, two(DPHI), false, 2),
            "dst" => (DST, two(DSTI), false, 2),
            "mul" => (MUL, None, true, 2),
            "sge" => (SGE, two(SGEI), false, 2),
            "slt" => (SLT, two(SLTI), false, 2),
            "max" => (MAX, None, true, 2),
            "min" => (MIN, None, true, 2),
            "ex2" => (EX2, None, false, 1),
            "lg2" => (LG2, None, false, 1),
            "litp" => (LITP, None, false, 1),
            "flr" => (FLR, None, false, 1),
            "rcp" => (RCP, None, false, 1),
            "rsq" => (RSQ, None, false, 1),
            "mov" => (MOV, None, false, 1),
            _ => return err(line, format!("unknown instruction `{name}`")),
        };
        if args.len() != sources + 1 {
            return err(line, format!("`{name}` takes {} operands", sources + 1));
        }
        let d = self.dest(line, &args[0])?;
        let mut a = self.src(line, &args[1])?;
        let dreg = Self::dest_reg(&d);
        if sources == 1 {
            let desc = d.dest_mask() | (a.src_bits() << 4);
            self.emit((op << 26) | (a.reg << 12) | (a.index << 19) | (dreg << 21), Some((desc, false)), line);
            return Ok(());
        }
        let mut b = self.src(line, &args[2])?;
        // src2 holds only inputs and temporaries; a uniform must go to src1
        // (swap commutative operands) or use the inverted form.
        let mut opcode = op;
        let mut inverted = false;
        if !b.is_low() || b.index != 0 {
            if a.is_low() && a.index == 0 {
                if commutative {
                    core::mem::swap(&mut a, &mut b);
                } else if let Some(i) = inverted_op {
                    opcode = i;
                    inverted = true;
                } else {
                    return err(line, format!("`{name}` cannot take a uniform as its second operand"));
                }
            } else {
                return err(line, format!("`{name}`: only one operand may be a uniform"));
            }
        }
        let desc = d.dest_mask() | (a.src_bits() << 4) | (b.src_bits() << 13);
        let word = if inverted {
            (opcode << 26) | (b.reg << 7) | (a.reg << 14) | (b.index << 19) | (dreg << 21)
        } else {
            (opcode << 26) | (b.reg << 7) | (a.reg << 12) | (a.index << 19) | (dreg << 21)
        };
        self.emit(word, Some((desc, false)), line);
        Ok(())
    }

    fn mad(&mut self, line: usize, args: &[String]) -> Result<(), Error> {
        if args.len() != 4 {
            return err(line, "`mad` takes 4 operands");
        }
        let d = self.dest(line, &args[0])?;
        let a = self.src(line, &args[1])?;
        let b = self.src(line, &args[2])?;
        let c = self.src(line, &args[3])?;
        if !a.is_low() {
            return err(line, "`mad`: the first source must be an input or temporary");
        }
        let desc = d.dest_mask() | (a.src_bits() << 4) | (b.src_bits() << 13) | (c.src_bits() << 22);
        let dreg = Self::dest_reg(&d);
        let word = if c.is_low() && c.index == 0 {
            (opcodes::MAD << 26) | (c.reg << 5) | (b.reg << 10) | (a.reg << 17) | (b.index << 22) | (dreg << 24)
        } else if b.is_low() && b.index == 0 {
            (opcodes::MADI << 26) | (c.reg << 5) | (b.reg << 12) | (a.reg << 17) | (c.index << 22) | (dreg << 24)
        } else {
            return err(line, "`mad`: only one of the last two sources may be a uniform");
        };
        self.emit(word, Some((desc, true)), line);
        Ok(())
    }

    fn cond(&self, line: usize, text: &str) -> Result<u32, Error> {
        // cmp.x, !cmp.y, cmp.x && cmp.y, cmp.x || !cmp.y
        let mut flags = (1 << 24) | (1 << 25);
        let parse_one = |t: &str, flags: &mut u32| -> Result<u32, Error> {
            let mut t = t.trim();
            let mut neg = false;
            while let Some(r) = t.strip_prefix('!') {
                neg = !neg;
                t = r.trim_start();
            }
            match t.to_ascii_lowercase().as_str() {
                "cmp.x" => {
                    if neg {
                        *flags ^= 1 << 25;
                    }
                    Ok(0)
                }
                "cmp.y" => {
                    if neg {
                        *flags ^= 1 << 24;
                    }
                    Ok(1)
                }
                _ => err(line, format!("bad condition `{t}` (expected cmp.x or cmp.y)")),
            }
        };
        if let Some(i) = text.find("&&").or_else(|| text.find('&')) {
            let w = if text[i..].starts_with("&&") { 2 } else { 1 };
            let a = parse_one(&text[..i], &mut flags)?;
            let b = parse_one(&text[i + w..], &mut flags)?;
            if a == b {
                return err(line, "a condition must use cmp.x and cmp.y once each");
            }
            flags |= 1 << 22; // AND
        } else if let Some(i) = text.find("||").or_else(|| text.find('|')) {
            let w = if text[i..].starts_with("||") { 2 } else { 1 };
            let a = parse_one(&text[..i], &mut flags)?;
            let b = parse_one(&text[i + w..], &mut flags)?;
            if a == b {
                return err(line, "a condition must use cmp.x and cmp.y once each");
            }
            // OR is mode 0
        } else {
            let a = parse_one(text, &mut flags)?;
            flags |= (a + 2) << 22;
        }
        Ok(flags)
    }

    fn bool_reg(&self, line: usize, text: &str) -> Result<u32, Error> {
        let op = self.operand(line, text)?;
        if !(REG_BOOL..REG_BOOL + 16).contains(&op.reg) {
            return err(line, format!("`{text}` is not a boolean uniform"));
        }
        Ok(op.reg - REG_BOOL)
    }

    fn close_check(&mut self, line: usize, start: usize, min_len: usize, kind: BlockKind) {
        if self.last_was_end {
            self.nop(line);
            return;
        }
        let p = self.pc();
        if p == 0 {
            return;
        }
        let last = self.code[p - 1].word >> 26;
        use opcodes::*;
        let jumpy = matches!(last, JMPC | JMPU | CALL | CALLC | CALLU);
        let breaky = kind == BlockKind::For && matches!(last, BREAK | BREAKC);
        if jumpy || breaky || p - start < min_len {
            self.nop(line);
        }
    }

    fn instruction(&mut self, line: usize, name: &str, rest: &str) -> Result<(), Error> {
        use opcodes::*;
        let args = split_args(rest);
        let lname = name.to_ascii_lowercase();
        match lname.as_str() {
            "nop" => self.emit(NOP << 26, None, line),
            "end" => self.emit(END << 26, None, line),
            "emit" => self.emit(EMIT << 26, None, line),
            "break" => self.emit(BREAK << 26, None, line),
            "setemit" => {
                let vtx: u32 = args.first().and_then(|a| a.parse().ok()).ok_or_else(|| Error { line, message: "setemit needs a vertex id".into() })?;
                let prim = args.iter().skip(1).any(|a| a.eq_ignore_ascii_case("prim"));
                let inv = args.iter().skip(1).any(|a| a.eq_ignore_ascii_case("inv"));
                self.emit((SETEMIT << 26) | ((inv as u32) << 22) | ((prim as u32) << 23) | ((vtx & 3) << 24), None, line);
            }
            "mad" => self.mad(line, &args)?,
            "mova" => {
                if args.len() != 2 {
                    return err(line, "`mova` takes 2 operands");
                }
                let d = self.operand(line, &args[0])?;
                if !(REG_ADDR..REG_ADDR + 2).contains(&d.reg) {
                    return err(line, "`mova` writes a0.x or a0.y (a0.xy)");
                }
                let s = self.src(line, &args[1])?;
                let desc = (d.dest_mask() & 0b1100) | (s.src_bits() << 4);
                self.emit((MOVA << 26) | (s.reg << 12) | (s.index << 19), Some((desc, false)), line);
            }
            "cmp" => {
                if args.len() != 4 {
                    return err(line, "`cmp` takes src1, op.x, op.y, src2");
                }
                let a = self.src(line, &args[0])?;
                let b = self.src(line, &args[3])?;
                let cmp_op = |s: &str| -> Result<u32, Error> {
                    Ok(match s.to_ascii_lowercase().as_str() {
                        "eq" => 0,
                        "ne" => 1,
                        "lt" => 2,
                        "le" => 3,
                        "gt" => 4,
                        "ge" => 5,
                        _ => return err(line, format!("bad comparison `{s}`")),
                    })
                };
                let (cx, cy) = (cmp_op(&args[1])?, cmp_op(&args[2])?);
                if !b.is_low() || b.index != 0 {
                    return err(line, "`cmp`: the second source must be an input or temporary");
                }
                let desc = (a.src_bits() << 4) | (b.src_bits() << 13);
                self.emit((CMP << 26) | (b.reg << 7) | (a.reg << 12) | (a.index << 19) | (cy << 21) | (cx << 24), Some((desc, false)), line);
            }
            "breakc" => {
                let f = self.cond(line, rest)?;
                self.emit((BREAKC << 26) | f, None, line);
            }
            "call" | "callc" | "callu" => {
                let (op, flags, target) = match lname.as_str() {
                    "call" => (CALL, 0, args.first()),
                    "callc" => (CALLC, self.cond(line, args.first().map(|s| s.as_str()).unwrap_or(""))?, args.get(1)),
                    _ => (CALLU, self.bool_reg(line, args.first().map(|s| s.as_str()).unwrap_or(""))? << 22, args.get(1)),
                };
                let target = target.ok_or_else(|| Error { line, message: format!("`{lname}` needs a procedure") })?;
                self.proc_fixups.push((self.pc(), target.clone(), line));
                self.emit((op << 26) | flags, None, line);
            }
            "jmpc" | "jmpu" => {
                let (op, flags) = if lname == "jmpc" {
                    (JMPC, self.cond(line, args.first().map(|s| s.as_str()).unwrap_or(""))?)
                } else {
                    let t = args.first().map(|s| s.as_str()).unwrap_or("");
                    let (neg, t) = match t.strip_prefix('!') {
                        Some(r) => (1, r),
                        None => (0, t),
                    };
                    (JMPU, (self.bool_reg(line, t)? << 22) | neg)
                };
                let target = args.get(1).ok_or_else(|| Error { line, message: format!("`{lname}` needs a label") })?;
                self.label_fixups.push((self.pc(), target.clone(), line));
                self.emit((op << 26) | flags, None, line);
            }
            "ifc" | "ifu" => {
                let flags = if lname == "ifc" { self.cond(line, rest)? } else { self.bool_reg(line, rest.trim())? << 22 };
                let op = if lname == "ifc" { IFC } else { IFU };
                self.blocks.push(Block { kind: BlockKind::If, start: self.pc(), else_at: None, name: String::new() });
                self.emit((op << 26) | flags, None, line);
            }
            "for" => {
                let op = self.operand(line, rest)?;
                if !(REG_INT..REG_INT + 4).contains(&op.reg) {
                    return err(line, "`for` needs an integer uniform (i0-i3)");
                }
                self.blocks.push(Block { kind: BlockKind::For, start: self.pc(), else_at: None, name: String::new() });
                self.emit((FOR << 26) | ((op.reg - REG_INT) << 22), None, line);
            }
            _ => self.arith(line, &lname, &args)?,
        }
        Ok(())
    }

    fn directive(&mut self, line: usize, name: &str, rest: &str) -> Result<(), Error> {
        match name {
            "fvec" | "ivec" | "bool" => {
                for item in split_args(rest) {
                    let (n, count) = match item.find('[') {
                        Some(i) => {
                            let c: u32 = item[i + 1..item.len() - 1]
                                .trim()
                                .parse()
                                .map_err(|_| Error { line, message: format!("bad array size in `{item}`") })?;
                            (item[..i].trim().to_string(), c)
                        }
                        None => (item.trim().to_string(), 1),
                    };
                    self.define_name(line, &n)?;
                    let (kind, base, next, limit) = match name {
                        "fvec" => (UniformKind::Float, REG_FLOAT, &mut self.next_float, 96),
                        "ivec" => (UniformKind::Int, REG_INT, &mut self.next_int, 4),
                        _ => (UniformKind::Bool, REG_BOOL, &mut self.next_bool, 16),
                    };
                    if *next + count > limit {
                        return err(line, format!("too many {name} uniforms"));
                    }
                    let first = *next;
                    *next += count;
                    self.uniforms.push(Uniform { name: n.clone(), kind, first: first as u8, count: count as u8 });
                    self.aliases.insert(n, Alias::Register { reg: base + first, swizzle: DEFAULT_SWIZZLE, negate: false, output: false });
                }
            }
            "constf" | "consti" | "constb" => {
                let open = rest.find('(').ok_or_else(|| Error { line, message: "expected `name(values)`".into() })?;
                let n = rest[..open].trim().to_string();
                let vals: Vec<String> = split_args(rest[open + 1..rest.rfind(')').unwrap_or(rest.len())].trim());
                self.define_name(line, &n)?;
                match name {
                    "constf" => {
                        if vals.len() != 4 {
                            return err(line, "`.constf` needs 4 values");
                        }
                        let mut v = [0.0f32; 4];
                        for (i, s) in vals.iter().enumerate() {
                            v[i] = parse_number(s).ok_or_else(|| Error { line, message: format!("bad number `{s}`") })?;
                        }
                        if self.next_float >= 96 {
                            return err(line, "too many float uniforms and constants");
                        }
                        // Constants take registers from the top, uniforms from the bottom.
                        let reg = 95 - self.constants.iter().filter(|c| matches!(c, Constant::Float { .. })).count() as u32;
                        self.constants.push(Constant::Float { register: reg as u8, value: v });
                        self.aliases.insert(n, Alias::Register { reg: REG_FLOAT + reg, swizzle: DEFAULT_SWIZZLE, negate: false, output: false });
                    }
                    "consti" => {
                        if vals.len() != 4 {
                            return err(line, "`.consti` needs 4 values");
                        }
                        let mut v = [0u8; 4];
                        for (i, s) in vals.iter().enumerate() {
                            v[i] = s.trim().parse::<i32>().map_err(|_| Error { line, message: format!("bad integer `{s}`") })? as u8;
                        }
                        let reg = 3 - self.constants.iter().filter(|c| matches!(c, Constant::Int { .. })).count() as u32;
                        self.constants.push(Constant::Int { register: reg as u8, value: v });
                        self.aliases.insert(n, Alias::Register { reg: REG_INT + reg, swizzle: DEFAULT_SWIZZLE, negate: false, output: false });
                    }
                    _ => {
                        let v = vals.first().map(|s| s.trim().eq_ignore_ascii_case("true") || s.trim() == "1").unwrap_or(false);
                        let reg = 15 - self.constants.iter().filter(|c| matches!(c, Constant::Bool { .. })).count() as u32;
                        self.constants.push(Constant::Bool { register: reg as u8, value: v });
                        self.aliases.insert(n, Alias::Register { reg: REG_BOOL + reg, swizzle: DEFAULT_SWIZZLE, negate: false, output: false });
                    }
                }
            }
            "alias" => {
                let mut parts = rest.split_whitespace();
                let n = parts.next().ok_or_else(|| Error { line, message: "`.alias name register`".into() })?.to_string();
                let target = parts.next().ok_or_else(|| Error { line, message: "`.alias name register`".into() })?;
                self.define_name(line, &n)?;
                let op = self.operand(line, target)?;
                self.aliases.insert(n, Alias::Register { reg: op.reg, swizzle: op.swizzle, negate: op.negate, output: op.output });
            }
            "in" => {
                let mut parts = rest.split_whitespace();
                let n = parts.next().ok_or_else(|| Error { line, message: "`.in name [register]`".into() })?.to_string();
                self.define_name(line, &n)?;
                let reg = match parts.next() {
                    Some(r) => {
                        let op = self.operand(line, r)?;
                        if op.reg >= REG_TEMP {
                            return err(line, "`.in` needs an input register (v0-v15)");
                        }
                        op.reg
                    }
                    None => {
                        let r = self.next_input;
                        self.next_input += 1;
                        r
                    }
                };
                if reg >= 16 {
                    return err(line, "too many inputs");
                }
                self.next_input = self.next_input.max(reg + 1);
                self.input_mask.set(self.input_mask.get() | 1 << reg);
                self.aliases.insert(n, Alias::Register { reg, swizzle: DEFAULT_SWIZZLE, negate: false, output: false });
            }
            "out" => {
                // .out name semantic [register]  (name `-` for none)
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.len() < 2 {
                    return err(line, "`.out name semantic [register]`");
                }
                let (sem_text, mask_text) = match parts[1].find('.') {
                    Some(i) => (&parts[1][..i], Some(&parts[1][i + 1..])),
                    None => (parts[1], None),
                };
                let kind = OutputKind::parse(sem_text).ok_or_else(|| Error { line, message: format!("unknown output semantic `{sem_text}`") })?;
                let reg = match parts.get(2) {
                    Some(r) => {
                        let op = self.operand(line, r)?;
                        if !op.output {
                            return err(line, "`.out` needs an output register (o0-o15)");
                        }
                        op.reg
                    }
                    None => {
                        let r = self.next_output;
                        self.next_output += 1;
                        r
                    }
                };
                self.next_output = self.next_output.max(reg + 1);
                let mask = match mask_text {
                    Some(m) => {
                        let sw = parse_swizzle(m).ok_or_else(|| Error { line, message: format!("bad mask `.{m}`") })?;
                        let op = Operand { reg: 0, swizzle: sw, negate: false, index: 0, output: true };
                        let rev = op.dest_mask();
                        // dest_mask has x at bit 3; outputs use x at bit 0
                        ((rev & 8) >> 3 | (rev & 4) >> 1 | (rev & 2) << 1 | (rev & 1) << 3) as u8
                    }
                    None => ((1u32 << kind.components()) - 1) as u8,
                };
                self.outputs.push(Output { kind, register: reg as u8, mask });
                if parts[0] != "-" {
                    self.define_name(line, parts[0])?;
                    self.aliases.insert(parts[0].to_string(), Alias::Register { reg, swizzle: DEFAULT_SWIZZLE, negate: false, output: true });
                }
            }
            "proc" => {
                let n = rest.trim().to_string();
                if n.is_empty() {
                    return err(line, "`.proc` needs a name");
                }
                if self.procs.contains_key(&n) {
                    return err(line, format!("procedure `{n}` is already defined"));
                }
                if !self.blocks.is_empty() {
                    return err(line, "procedures cannot be nested");
                }
                self.blocks.push(Block { kind: BlockKind::Proc, start: self.pc(), else_at: None, name: n });
            }
            "else" => {
                let top = self.blocks.len().checked_sub(1).ok_or_else(|| Error { line, message: "`.else` outside an `if`".into() })?;
                if self.blocks[top].kind != BlockKind::If || self.blocks[top].else_at.is_some() {
                    return err(line, "`.else` outside an `if`");
                }
                let start = self.blocks[top].start;
                self.close_check(line, start, 2, BlockKind::If);
                let pc = self.pc();
                self.blocks[top].else_at = Some(pc);
                let w = &mut self.code[start].word;
                *w = (*w & !(0xFFF << 10)) | ((pc as u32) << 10);
                self.last_was_end = false;
            }
            "end" => {
                let block = self.blocks.pop().ok_or_else(|| Error { line, message: "`.end` without a block".into() })?;
                let min = if block.kind == BlockKind::Proc { 1 } else { 2 };
                self.close_check(line, block.start, min, block.kind);
                let pc = self.pc();
                match block.kind {
                    BlockKind::Proc => {
                        self.procs.insert(block.name, (block.start, pc - block.start));
                    }
                    BlockKind::For => {
                        let w = &mut self.code[block.start].word;
                        *w = (*w & !(0xFFF << 10)) | (((pc - 1) as u32) << 10);
                        self.last_was_end = true;
                    }
                    BlockKind::If => {
                        let w = &mut self.code[block.start].word;
                        match block.else_at {
                            None => *w = (*w & !(0xFFF << 10)) | ((pc as u32) << 10),
                            Some(e) => *w = (*w & !0x3FF) | (pc - e) as u32,
                        }
                        self.last_was_end = true;
                    }
                }
                if block.kind == BlockKind::Proc {
                    self.last_was_end = false;
                }
            }
            "entry" => self.entry_name = rest.trim().to_string(),
            "gsh" => {
                // .gsh point cN  (point mode, vertex data in float uniforms from cN)
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.first().map(|s| s.eq_ignore_ascii_case("point")) != Some(true) {
                    return err(line, "only `.gsh point cN` is supported");
                }
                let first = parts.get(1).and_then(|r| r.strip_prefix('c')).and_then(|n| n.parse::<u8>().ok()).unwrap_or(0);
                self.kind = ShaderKind::Geometry;
                self.geometry = Some(GeometryMode { mode: 0, fixed_start: first, variable_count: 0, fixed_count: 0 });
            }
            "nodvle" | "setf" | "seti" | "setb" => {}
            _ => return err(line, format!("unknown directive `.{name}`")),
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Program, Error> {
        if let Some(b) = self.blocks.last() {
            return err(self.code.last().map_or(0, |p| p.line), format!("unclosed block (started at instruction {})", b.start));
        }
        for (at, name, line) in core::mem::take(&mut self.proc_fixups) {
            let (start, len) = *self.procs.get(&name).ok_or_else(|| Error { line, message: format!("no procedure `{name}`") })?;
            let w = &mut self.code[at].word;
            *w = (*w & !((0xFFF << 10) | 0x3FF)) | ((start as u32) << 10) | len as u32;
        }
        for (at, name, line) in core::mem::take(&mut self.label_fixups) {
            let target = *self.labels.get(&name).ok_or_else(|| Error { line, message: format!("no label `{name}`") })?;
            let w = &mut self.code[at].word;
            *w = (*w & !(0xFFF << 10)) | ((target as u32) << 10);
        }
        let (entry, len) = *self.procs.get(&self.entry_name).ok_or_else(|| Error { line: 0, message: format!("no `{}` procedure", self.entry_name) })?;

        // Operand descriptors: deduplicate, those used by mad first (5-bit field).
        let mut mad_descs: Vec<u32> = Vec::new();
        let mut other_descs: Vec<u32> = Vec::new();
        for p in &self.code {
            if let Some((d, mad)) = p.opdesc {
                if mad {
                    if !mad_descs.contains(&d) {
                        mad_descs.push(d);
                    }
                }
            }
        }
        for p in &self.code {
            if let Some((d, false)) = p.opdesc {
                if !mad_descs.contains(&d) && !other_descs.contains(&d) {
                    other_descs.push(d);
                }
            }
        }
        if mad_descs.len() > 32 {
            return err(0, "too many distinct operand combinations in `mad` instructions (32 at most)");
        }
        let mut opdescs = mad_descs;
        opdescs.extend(other_descs);
        if opdescs.len() > 128 {
            return err(0, "too many distinct operand combinations (128 at most)");
        }
        let code: Vec<u32> = self
            .code
            .iter()
            .map(|p| match p.opdesc {
                Some((d, _)) => p.word | opdescs.iter().position(|&x| x == d).unwrap() as u32,
                None => p.word,
            })
            .collect();
        Ok(Program {
            kind: self.kind,
            code,
            opdescs,
            entry: entry as u32,
            entry_end: (entry + len) as u32,
            outputs: self.outputs,
            uniforms: self.uniforms,
            constants: self.constants,
            input_mask: self.input_mask.get(),
            geometry: self.geometry,
        })
    }
}

/// Assembles shader source.
pub fn assemble(source: &str) -> Result<Program, Error> {
    let mut a = Assembler::new(ShaderKind::Vertex);
    for (i, raw) in source.lines().enumerate() {
        let line = i + 1;
        let text = raw.split(';').next().unwrap_or("").split("//").next().unwrap_or("").trim();
        if text.is_empty() {
            continue;
        }
        // Labels.
        let mut text = text;
        if let Some(colon) = text.find(':') {
            let label = text[..colon].trim();
            if !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                if a.labels.insert(label.to_string(), a.pc()).is_some() {
                    return err(line, format!("label `{label}` is already defined"));
                }
                text = text[colon + 1..].trim();
                if text.is_empty() {
                    continue;
                }
            }
        }
        let (word, rest) = match text.find(char::is_whitespace) {
            Some(i) => (&text[..i], text[i..].trim()),
            None => (text, ""),
        };
        if let Some(d) = word.strip_prefix('.') {
            a.directive(line, &d.to_ascii_lowercase(), rest)?;
        } else {
            if a.blocks.is_empty() {
                return err(line, "instructions must be inside a `.proc`");
            }
            a.instruction(line, word, rest)?;
        }
    }
    a.finish()
}

/// Converts the GPU's 24-bit float to a float.
pub fn f24_to_f32(v: u32) -> f32 {
    let sign = (v >> 23) & 1;
    let exp = (v >> 16) & 0x7F;
    let mantissa = v & 0xFFFF;
    if exp == 0 && mantissa == 0 {
        return f32::from_bits(sign << 31);
    }
    f32::from_bits((sign << 31) | ((exp + 127 - 63) << 23) | (mantissa << 7))
}

/// Converts a float to the GPU's 24-bit float (1 sign, 7 exponent, 16
/// mantissa bits).
pub fn f32_to_f24(f: f32) -> u32 {
    let bits = f.to_bits();
    let sign = bits >> 31;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let mantissa = (bits >> 7) & 0xFFFF;
    let e = exp - 127 + 63;
    if e < 0 {
        sign << 23
    } else if e > 0x7F {
        (sign << 23) | (0x7F << 16)
    } else {
        (sign << 23) | ((e as u32) << 16) | mantissa
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const BASIC: &str = r#"
        ; a basic vertex shader
        .fvec projection[4]
        .constf ones(1.0, 1.0, 1.0, 1.0)
        .alias inpos v0
        .alias inclr v1
        .out outpos position
        .out outclr color

        .proc main
            mov r0.xyz, inpos
            mov r0.w, ones
            dp4 outpos.x, projection[0], r0
            dp4 outpos.y, projection[1], r0
            dp4 outpos.z, projection[2], r0
            dp4 outpos.w, projection[3], r0
            mov outclr, inclr
            end
        .end
    "#;

    #[test]
    fn assembles_a_basic_shader() {
        let p = assemble(BASIC).unwrap();
        assert_eq!(p.code.len(), 8);
        assert_eq!(p.entry, 0);
        assert_eq!(p.entry_end, 8);
        // mov r0.xyz, v0: opcode 0x13, src1 v0 (0), dest r0 (0x10)
        assert_eq!(p.code[0] >> 26, 0x13);
        assert_eq!((p.code[0] >> 21) & 0x1F, 0x10);
        assert_eq!((p.code[0] >> 12) & 0x7F, 0x00);
        // its descriptor: mask xyz (bits 3,2,1), src1 xyzw
        let d0 = p.opdescs[(p.code[0] & 0x7F) as usize];
        assert_eq!(d0 & 0xF, 0b1110);
        assert_eq!((d0 >> 4) & 0x1FF, 0x1B << 1);
        // dp4 o0.x, c0, r0: uniform in src1, temp in src2
        let w = p.code[2];
        assert_eq!(w >> 26, 0x02);
        assert_eq!((w >> 12) & 0x7F, 0x20);
        assert_eq!((w >> 7) & 0x1F, 0x10);
        assert_eq!((w >> 21) & 0x1F, 0x00);
        assert_eq!(p.code[7], 0x22 << 26);
        assert_eq!(p.uniform("projection"), Some(&Uniform { name: "projection".into(), kind: UniformKind::Float, first: 0, count: 4 }));
        assert_eq!(p.constants, vec![Constant::Float { register: 95, value: [1.0; 4] }]);
        // mov r0.w, ones: c95
        assert_eq!((p.code[1] >> 12) & 0x7F, 0x20 + 95);
        let (map, mask, _, _) = p.output_map();
        assert_eq!(map[0], 2);
        assert_eq!(mask, 0b11);
        assert_eq!(map[1], 0x03020100);
        assert_eq!(map[2], 0x0B0A0908);
    }

    #[test]
    fn swaps_commutative_operands_and_uses_inverted_forms() {
        let p = assemble(".fvec k\n.proc main\nmul r0, r1, k\nsge r2, r1, k\nend\n.end").unwrap();
        // mul r0, r1, c0 -> c0 moved to src1
        assert_eq!((p.code[0] >> 12) & 0x7F, 0x20);
        assert_eq!((p.code[0] >> 7) & 0x1F, 0x11);
        // sge r2, r1, c0 -> sgei with src1 r1 at bit 14, src2 c0 at bit 7
        assert_eq!(p.code[1] >> 26, 0x1A);
        assert_eq!((p.code[1] >> 14) & 0x1F, 0x11);
        assert_eq!((p.code[1] >> 7) & 0x7F, 0x20);
    }

    #[test]
    fn short_swizzles_repeat_and_aliases_compose() {
        let p = assemble(".alias pos v0.zyxw\n.proc main\nmov r0, -pos.xy\nend\n.end").unwrap();
        let d = p.opdescs[(p.code[0] & 0x7F) as usize];
        let src = (d >> 4) & 0x1FF;
        assert_eq!(src & 1, 1, "negated");
        // pos.xy = pos.xyyy -> v0.zyyy = 2,1,1,1
        assert_eq!(src >> 1, (2 << 6) | (1 << 4) | (1 << 2) | 1);
    }

    #[test]
    fn if_else_and_loops_get_addresses() {
        let src = r#"
            .ivec count
            .proc main
                cmp r0, lt, lt, r1
                ifc cmp.x
                    mov r2, r0
                    mov r3, r0
                .else
                    mov r2, r1
                    mov r3, r1
                .end
                for count
                    add r4, r4, r5
                    add r4, r4, r5
                .end
                end
            .end
        "#;
        let p = assemble(src).unwrap();
        let ifc = p.code[1];
        assert_eq!(ifc >> 26, 0x28);
        assert_eq!((ifc >> 10) & 0xFFF, 4, "else block starts at 4");
        assert_eq!(ifc & 0x3FF, 2, "else block is 2 long");
        assert_eq!((ifc >> 22) & 3, 2, "condition: cmp.x only");
        let f = p.code[6];
        assert_eq!(f >> 26, 0x29);
        assert_eq!((f >> 10) & 0xFFF, 8, "loop ends at its last instruction");
        assert_eq!(p.code[9], 0x22 << 26);
        // an `.end` right after another needs a padding nop
        let q = assemble(".proc main\nifu b0\nmov r0, r1\nmov r0, r1\n.end\n.end").unwrap();
        assert_eq!(q.code.last().unwrap() >> 26, 0x21);
    }

    #[test]
    fn call_targets_and_mad() {
        let src = r#"
            .fvec k
            .proc helper
                mad r0, r1, k, r2
            .end
            .proc main
                call helper
                end
            .end
        "#;
        let p = assemble(src).unwrap();
        assert_eq!(p.entry, 1);
        let call = p.code[1];
        assert_eq!(call >> 26, 0x24);
        assert_eq!((call >> 10) & 0xFFF, 0);
        assert_eq!(call & 0x3FF, 1);
        // a nop pads after `call` before the end of main
        let mad = p.code[0];
        assert_eq!(mad >> 29, 0x38 >> 3);
        assert!((mad & 0x1F) < 32);
        assert_eq!((mad >> 10) & 0x7F, 0x20, "uniform in src2");
    }

    #[test]
    fn reports_errors_with_lines() {
        let e = assemble(".proc main\nfoo r0, r1\nend\n.end").unwrap_err();
        assert_eq!(e.line, 2);
        let e = assemble(".proc main\nmov v0, r1\nend\n.end").unwrap_err();
        assert!(e.message.contains("not a writable register"));
        let e = assemble(".proc main\nadd r0, c0, c1\nend\n.end").unwrap_err();
        assert!(e.message.contains("only one operand"));
        assert!(assemble(".proc other\nend\n.end").is_err(), "needs main");
    }

    #[test]
    fn f24_values() {
        assert_eq!(f32_to_f24(1.0), 0x3F0000);
        assert_eq!(f32_to_f24(-2.0), 0x800000 | 0x400000);
        assert_eq!(f32_to_f24(0.0), 0);
        assert_eq!(f32_to_f24(0.5), 0x3E0000);
    }
}
