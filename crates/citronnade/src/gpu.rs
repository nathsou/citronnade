//! The GPU: render targets, shaders, vertex buffers, textures and the
//! texture combiners (see [`citronnade_gpu`]), plus showing a render target
//! on a screen.
//!
//! ```ignore
//! use citronnade::gpu::{self, shader, Gpu, RenderTarget, Shader};
//!
//! let mut gpu = Gpu::new()?;
//! let target = RenderTarget::top()?;
//! let shader = Shader::new(shader!(r#" ... "#)).unwrap();
//! loop {
//!     gpu.clear(&target, gpu::rgba(20, 30, 60, 255), 0)?;
//!     gpu.bind_target(&target);
//!     gpu.bind_shader(&shader);
//!     // set uniforms, layout, vertices, texenv; draw
//!     screens.show_target(&mut gpu, &target, Screen::Top)?;
//!     screens.present();
//! }
//! ```

pub use citronnade_gpu::*;
/// Assembles a PICA200 shader at compile time (see [`citronnade_gpu::asm`]).
pub use citronnade_macros::shader;

use crate::gfx::{Screen, Screens};

impl Screens {
    /// Converts `target` into a screen's back buffer (the left eye of the
    /// top screen in stereo mode), after the GPU finishes drawing. The
    /// target must be the screen's size (400x240 or 320x240).
    pub fn show_target(&mut self, gpu: &mut Gpu, target: &RenderTarget, screen: Screen) -> crate::Result<()> {
        self.show_target_eye(gpu, target, screen, false)
    }

    /// Like [`Screens::show_target`], into the right eye's back buffer
    /// (the top screen in stereo mode).
    pub fn show_target_right(&mut self, gpu: &mut Gpu, target: &RenderTarget) -> crate::Result<()> {
        self.show_target_eye(gpu, target, Screen::Top, true)
    }

    fn show_target_eye(&mut self, gpu: &mut Gpu, target: &RenderTarget, screen: Screen, right: bool) -> crate::Result<()> {
        let (ptr, width, format) = self.back_buffer(screen, right);
        assert_eq!(target.width() as u32, width, "render target and screen widths differ");
        let bytes = 240 * width as usize * format.bytes_per_pixel();
        // Drop cached CPU writes to the buffer so they cannot land on top of
        // the GPU's image later.
        let _ = citronnade_services::gsp::invalidate_data_cache(ptr, bytes);
        let out = match format {
            crate::Format::Rgba8 => gx::TransferFormat::Rgba8,
            crate::Format::Bgr8 => gx::TransferFormat::Rgb8,
            crate::Format::Rgb565 => gx::TransferFormat::Rgb565,
            crate::Format::Rgb5A1 => gx::TransferFormat::Rgb5A1,
            crate::Format::Rgba4 => gx::TransferFormat::Rgba4,
        };
        gpu.transfer(target, ptr, out, 0)
    }
}
