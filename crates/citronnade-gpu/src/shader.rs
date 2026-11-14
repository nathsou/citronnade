//! Shaders ready for the GPU.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use citronnade_shader::{Constant, Program, ShaderKind, ShbinError, UniformKind};

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

/// A vertex shader (geometry shaders come with v0.7).
pub struct Shader {
    pub(crate) program: Program,
    pub(crate) id: u32,
    pub(crate) outmap: [u32; 8],
    pub(crate) outmap_mask: u32,
    pub(crate) outmap_mode: u32,
    pub(crate) outmap_clock: u32,
    /// Constants as (register, xyzw) in each uniform file.
    pub(crate) float_constants: Vec<(u8, [f32; 4])>,
    pub(crate) int_constants: Vec<(u8, [u8; 4])>,
    pub(crate) bool_constants: Vec<(u8, bool)>,
}

/// Where a uniform lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UniformLocation {
    pub kind: UniformKind,
    /// The first register.
    pub register: u8,
    /// Registers it spans (4 for a matrix).
    pub count: u8,
}

impl Shader {
    /// Loads the first entry point of a SHBIN file (from `shader!` or
    /// picasso).
    pub fn new(shbin: &[u8]) -> Result<Shader, ShbinError> {
        Ok(Shader::from_program(Program::from_shbin(shbin)?))
    }

    /// Uses an assembled program.
    pub fn from_program(program: Program) -> Shader {
        assert!(program.kind == ShaderKind::Vertex, "geometry shaders are not supported yet");
        let (outmap, outmap_mask, outmap_mode, outmap_clock) = program.output_map();
        let mut float_constants = Vec::new();
        let mut int_constants = Vec::new();
        let mut bool_constants = Vec::new();
        for c in &program.constants {
            match *c {
                Constant::Float { register, value } => float_constants.push((register, value)),
                Constant::Int { register, value } => int_constants.push((register, value)),
                Constant::Bool { register, value } => bool_constants.push((register, value)),
            }
        }
        Shader {
            program,
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            outmap,
            outmap_mask,
            outmap_mode,
            outmap_clock,
            float_constants,
            int_constants,
            bool_constants,
        }
    }

    /// Finds a uniform by name.
    pub fn uniform(&self, name: &str) -> Option<UniformLocation> {
        self.program.uniform(name).map(|u| UniformLocation { kind: u.kind, register: u.first, count: u.count })
    }

    /// The assembled program.
    pub fn program(&self) -> &Program {
        &self.program
    }
}
