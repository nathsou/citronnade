//! The PICA200, the 3DS GPU.
//!
//! - [`Gpu`] records drawing into a command list and runs it: render
//!   targets, shaders, vertex buffers, textures, texture combiners, blending,
//!   depth and stencil;
//! - [`gx`] has the transfer engines GSP drives: memory fills (clears),
//!   display transfers (render target to screen) and texture copies;
//! - [`Shader`] wraps a program from [`citronnade_shader`] (or a SHBIN file
//!   from picasso);
//! - [`Texture`] tiles images into the GPU's formats;
//! - [`vram`] allocates the 6 MiB of video memory.
//!
//! The GPU draws into render targets 240 pixels wide and 400 (top screen) or
//! 320 (bottom) tall, the screens' native orientation; the projection
//! matrices in [`math`] rotate the image so that x runs along the long side.
#![no_std]
#![cfg_attr(not(target_arch = "arm"), allow(dead_code))]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod fog;
pub mod lighting;
pub mod math;
pub mod mesh;
pub mod proctex;
pub mod regs;
mod shader;
mod state;
pub mod tiling;

#[cfg(target_arch = "arm")]
pub mod cmd;
#[cfg(target_arch = "arm")]
mod context;
#[cfg(target_arch = "arm")]
pub mod gx;
#[cfg(target_arch = "arm")]
pub mod linear;
#[cfg(target_arch = "arm")]
mod target;
#[cfg(target_arch = "arm")]
mod texture;
#[cfg(target_arch = "arm")]
mod gpu_mesh;
#[cfg(target_arch = "arm")]
pub mod vram;

pub use citronnade_shader::{self as asm, Program, ShbinError};
pub use fog::FogLut;
pub use lighting::{DistanceAttenuation, Fresnel, Light, LightEnv, LightLut, LutId, LutInput, Material};
pub use proctex::{ProcClamp, ProcLut, ProcMap, ProcNoise, ProcShift, ProcTex};
pub use shader::{Shader, UniformLocation};
pub use state::*;
pub use tiling::{tiled_index, Filter, TexFormat, Wrap};

#[cfg(target_arch = "arm")]
pub use citronnade_services::ResultCode;
#[cfg(target_arch = "arm")]
pub use cmd::CommandList;
#[cfg(target_arch = "arm")]
pub use context::{Attrib, AttribType, Gpu, Primitive, VertexLayout};
#[cfg(target_arch = "arm")]
pub use linear::LinearBuffer;
#[cfg(target_arch = "arm")]
pub use target::{ColorFormat, DepthFormat, RenderTarget};
#[cfg(target_arch = "arm")]
pub use texture::Texture;
#[cfg(target_arch = "arm")]
pub use gpu_mesh::Mesh;
pub use mesh::{MeshData, MeshError, MeshVertex, Submesh};

/// The result of a GPU operation.
#[cfg(target_arch = "arm")]
pub type Result<T> = core::result::Result<T, ResultCode>;

/// "Out of memory" (as the kernel reports it).
#[cfg(target_arch = "arm")]
pub(crate) const OUT_OF_MEMORY: ResultCode = ResultCode(0xD860_07F3);

/// Converts a float to the GPU's 24-bit float (1 sign, 7 exponent, 16
/// mantissa bits).
pub use citronnade_shader::f32_to_f24;

/// Converts a float to the GPU's 31-bit float (1 sign, 7 exponent, 23
/// mantissa bits), as the viewport registers use.
pub fn f32_to_f31(f: f32) -> u32 {
    let i = f.to_bits();
    let mantissa = i & 0x7F_FFFF;
    let exponent = ((i >> 23) & 0xFF) as i32 - 127 + 63;
    let sign = i >> 31;
    if exponent < 0 {
        sign << 30
    } else if exponent > 0x7F {
        (sign << 30) | (0x7F << 23)
    } else {
        (sign << 30) | ((exponent as u32) << 23) | mantissa
    }
}

/// Converts a float to the GPU's 20-bit float (1 sign, 7 exponent, 12
/// mantissa bits).
pub fn f32_to_f20(f: f32) -> u32 {
    let i = f.to_bits();
    let mantissa = (i & 0x7F_FFFF) >> 11;
    let exponent = ((i >> 23) & 0xFF) as i32 - 127 + 63;
    let sign = i >> 31;
    if exponent < 0 {
        sign << 19
    } else if exponent > 0x7F {
        (sign << 19) | (0x7F << 12)
    } else {
        (sign << 19) | ((exponent as u32) << 12) | mantissa
    }
}

/// Converts a float to a 16-bit float (1 sign, 5 exponent, 10 mantissa bits).
pub fn f32_to_f16(f: f32) -> u32 {
    let i = f.to_bits();
    let mantissa = (i & 0x7F_FFFF) >> 13;
    let exponent = ((i >> 23) & 0xFF) as i32 - 127 + 15;
    let sign = i >> 31;
    if exponent < 0 {
        sign << 15
    } else if exponent > 0x1F {
        (sign << 15) | (0x1F << 10)
    } else {
        (sign << 15) | ((exponent as u32) << 10) | mantissa
    }
}

/// A colour as the GPU's colour registers take it: 0xAABBGGRR.
pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (r as u32) | (g as u32) << 8 | (b as u32) << 16 | (a as u32) << 24
}

/// The physical address of linear-heap or VRAM memory.
#[cfg(target_arch = "arm")]
pub(crate) fn phys(ptr: *const u8) -> u32 {
    citronnade_rt::memory::virt_to_phys(ptr as usize)
}

/// Writes the data cache back to memory for the whole linear heap, so that
/// the GPU sees what the CPU wrote.
#[cfg(target_arch = "arm")]
pub fn flush_linear_heap() {
    let s = citronnade_rt::heap::linear_stats();
    if s.size > 0 {
        let _ = citronnade_services::gsp::flush_data_cache(s.base as *const u8, s.size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_formats() {
        assert_eq!(f32_to_f16(1.0), 0x3C00);
        assert_eq!(f32_to_f16(-0.0), 0x8000);
        assert_eq!(f32_to_f31(1.0), 63 << 23);
        assert_eq!(f32_to_f31(0.5) << 1, 62 << 24);
        assert_eq!(rgba(1, 2, 3, 4), 0x04030201);
    }
}
