//! Textures: power-of-two images in the GPU's tiled layout (see
//! [`crate::tiling`]).

use crate::linear::LinearBuffer;
use crate::tiling::{encode_nibble, encode_texel, tile_rgba, tiled_index, Filter, TexFormat, Wrap};
use crate::{Result, OUT_OF_MEMORY};

/// A 2D texture in linear memory.
pub struct Texture {
    pub(crate) data: LinearBuffer<u8>,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) format: TexFormat,
    pub(crate) param: u32,
    pub(crate) border: u32,
}


fn valid_size(n: u32) -> bool {
    n.is_power_of_two() && (8..=1024).contains(&n)
}

impl Texture {
    /// An empty (zeroed) texture. Sizes must be powers of two from 8 to 1024.
    pub fn new(width: u32, height: u32, format: TexFormat) -> Result<Texture> {
        if !valid_size(width) || !valid_size(height) {
            return Err(OUT_OF_MEMORY);
        }
        let bytes = (width * height) as usize * format.bits_per_texel() / 8;
        let data = LinearBuffer::zeroed(bytes)?;
        let mut param = 0;
        if format == TexFormat::Etc1 {
            param |= 1 << 5;
        }
        let mut t = Texture {
            data,
            width: width as u16,
            height: height as u16,
            format,
            param,
            border: 0,
        };
        t.set_filter(Filter::Linear, Filter::Linear);
        t.set_wrap(Wrap::ClampToEdge, Wrap::ClampToEdge);
        Ok(t)
    }

    /// A texture from an RGBA8 image (rows from the top), converted to
    /// `format`.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8], format: TexFormat) -> Result<Texture> {
        let mut t = Texture::new(width, height, format)?;
        t.load_rgba(rgba);
        Ok(t)
    }

    /// A texture from a `.ctex` file (pre-tiled by `citronnade-assets` in a
    /// build script). Panics if the data is not a valid `.ctex` file.
    pub fn from_ctex(file: &[u8]) -> Result<Texture> {
        let h = crate::tiling::CtexHeader::parse(file).expect("not a .ctex texture");
        let mut t = Texture::new(h.width, h.height, h.format)?;
        t.data.copy_from_slice(&file[crate::tiling::CtexHeader::SIZE..][..h.data_len as usize]);
        Ok(t)
    }

    /// Replaces the texels with an RGBA8 image (rows from the top).
    pub fn load_rgba(&mut self, rgba: &[u8]) {
        let (w, h) = (self.width as u32, self.height as u32);
        tile_rgba(self.format, w, h, rgba, &mut self.data);
    }

    /// Sets one texel (y from the top). Slow; for procedural images.
    pub fn set_texel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        let (w, h) = (self.width as u32, self.height as u32);
        if x >= w || y >= h {
            return;
        }
        let t = tiled_index(x, y, w, h) as usize;
        if self.format.bits_per_texel() == 4 {
            let n = encode_nibble(self.format, rgba);
            let byte = &mut self.data[t / 2];
            *byte = if t & 1 == 0 { (*byte & 0xF0) | n } else { (*byte & 0x0F) | n << 4 };
        } else {
            let (bytes, n) = encode_texel(self.format, rgba);
            self.data[t * n..t * n + n].copy_from_slice(&bytes[..n]);
        }
    }

    /// The raw tiled texel data, for pre-converted images.
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    pub fn width(&self) -> u32 {
        self.width as u32
    }

    pub fn height(&self) -> u32 {
        self.height as u32
    }

    pub fn format(&self) -> TexFormat {
        self.format
    }

    pub fn set_filter(&mut self, mag: Filter, min: Filter) {
        self.param = (self.param & !0b110) | (mag as u32) << 1 | (min as u32) << 2;
    }

    pub fn set_wrap(&mut self, s: Wrap, t: Wrap) {
        self.param = (self.param & !(0x3 << 12 | 0x3 << 8)) | (s as u32) << 12 | (t as u32) << 8;
    }

    /// The colour used outside the texture with [`Wrap::ClampToBorder`]
    /// (0xAABBGGRR).
    pub fn set_border(&mut self, rgba: u32) {
        self.border = rgba;
    }
}

