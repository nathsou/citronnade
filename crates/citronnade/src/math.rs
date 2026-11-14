//! Float functions `core` lacks (it has no `floor`, `sqrt` or `sin`), and
//! the matrices the GPU uses.

pub use citronnade_gpu::math::{Mat4, Quat, Vec3, Vec4};
pub use citronnade_rt::math::*;
pub use core::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
