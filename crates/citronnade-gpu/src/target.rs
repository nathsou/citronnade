//! Render targets: a colour buffer and an optional depth (and stencil)
//! buffer in VRAM, in the GPU's tiled layout.

use crate::vram::{self, Bank, VramBlock};
use crate::Result;

/// Colour buffer formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorFormat {
    Rgba8 = 0,
    Rgb8 = 1,
    Rgba5551 = 2,
    Rgb565 = 3,
    Rgba4 = 4,
}

impl ColorFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            ColorFormat::Rgba8 => 4,
            ColorFormat::Rgb8 => 3,
            _ => 2,
        }
    }

    /// The colour buffer format register's size code.
    pub(crate) const fn size_code(self) -> u32 {
        match self {
            ColorFormat::Rgba8 => 2,
            ColorFormat::Rgb8 => 1,
            _ => 0,
        }
    }

    pub(crate) fn transfer_format(self) -> crate::gx::TransferFormat {
        use crate::gx::TransferFormat as T;
        match self {
            ColorFormat::Rgba8 => T::Rgba8,
            ColorFormat::Rgb8 => T::Rgb8,
            ColorFormat::Rgba5551 => T::Rgb5A1,
            ColorFormat::Rgb565 => T::Rgb565,
            ColorFormat::Rgba4 => T::Rgba4,
        }
    }

    /// The value a memory fill writes for `rgba` (0xAABBGGRR).
    pub(crate) fn fill_value(self, c: u32) -> u32 {
        let (r, g, b, a) = (c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF, c >> 24);
        match self {
            ColorFormat::Rgba8 => r << 24 | g << 16 | b << 8 | a,
            ColorFormat::Rgb8 => r << 16 | g << 8 | b,
            ColorFormat::Rgba5551 => (r >> 3) << 11 | (g >> 3) << 6 | (b >> 3) << 1 | (a >> 7),
            ColorFormat::Rgb565 => (r >> 3) << 11 | (g >> 2) << 5 | (b >> 3),
            ColorFormat::Rgba4 => (r >> 4) << 12 | (g >> 4) << 8 | (b >> 4) << 4 | (a >> 4),
        }
    }
}

/// Depth buffer formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepthFormat {
    Depth16 = 0,
    Depth24 = 2,
    /// 24-bit depth and an 8-bit stencil.
    Depth24Stencil8 = 3,
}

impl DepthFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            DepthFormat::Depth16 => 2,
            DepthFormat::Depth24 => 3,
            DepthFormat::Depth24Stencil8 => 4,
        }
    }
}

/// A surface the GPU draws into.
pub struct RenderTarget {
    pub(crate) color: VramBlock,
    pub(crate) depth: Option<VramBlock>,
    /// Size in the framebuffer's orientation (240 x 400 for the top screen).
    pub(crate) fb_width: u16,
    pub(crate) fb_height: u16,
    pub(crate) color_format: ColorFormat,
    pub(crate) depth_format: Option<DepthFormat>,
}

impl RenderTarget {
    /// A target for a `width` x `height` screen-oriented image (400x240 for
    /// the top screen, 320x240 for the bottom one). Both must be multiples
    /// of 8.
    pub fn new(width: u16, height: u16, color: ColorFormat, depth: Option<DepthFormat>) -> Result<RenderTarget> {
        assert!(width % 8 == 0 && height % 8 == 0, "render target sizes must be multiples of 8");
        let (fb_width, fb_height) = (height, width);
        let pixels = fb_width as usize * fb_height as usize;
        let color_buf = vram::alloc(pixels * color.bytes_per_pixel(), 0x80, Bank::A)?;
        let depth_buf = match depth {
            Some(d) => Some(vram::alloc(pixels * d.bytes_per_pixel(), 0x80, Bank::B)?),
            None => None,
        };
        Ok(RenderTarget { color: color_buf, depth: depth_buf, fb_width, fb_height, color_format: color, depth_format: depth })
    }

    /// A target the size of the top screen (400x240), RGBA8 with depth and
    /// stencil.
    pub fn top() -> Result<RenderTarget> {
        RenderTarget::new(400, 240, ColorFormat::Rgba8, Some(DepthFormat::Depth24Stencil8))
    }

    /// A target the size of the bottom screen (320x240), RGBA8 with depth
    /// and stencil.
    pub fn bottom() -> Result<RenderTarget> {
        RenderTarget::new(320, 240, ColorFormat::Rgba8, Some(DepthFormat::Depth24Stencil8))
    }

    /// Width as seen on screen.
    pub fn width(&self) -> u16 {
        self.fb_height
    }

    /// Height as seen on screen.
    pub fn height(&self) -> u16 {
        self.fb_width
    }

    pub fn color_format(&self) -> ColorFormat {
        self.color_format
    }

    pub fn depth_format(&self) -> Option<DepthFormat> {
        self.depth_format
    }

    /// The colour buffer (tiled), for reading back or using as a texture.
    pub fn color_ptr(&self) -> *mut u8 {
        self.color.as_ptr()
    }

    pub(crate) fn color_bytes(&self) -> usize {
        self.fb_width as usize * self.fb_height as usize * self.color_format.bytes_per_pixel()
    }

    pub(crate) fn depth_bytes(&self) -> usize {
        self.fb_width as usize * self.fb_height as usize * self.depth_format.map_or(0, |d| d.bytes_per_pixel())
    }
}
