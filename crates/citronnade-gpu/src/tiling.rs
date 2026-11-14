//! Texture formats and the GPU's tiled texel order (no hardware access, so
//! it also runs in build scripts and host tools).
//!
//! Texels are stored in 8x8 tiles, the tiles row by row starting from the
//! top of the image (which texture coordinate v = 1 samples; v = 0 is the
//! bottom), and the 64 texels of a tile in Z order (Morton order: x and y
//! bits interleaved, x first).

/// Texture formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexFormat {
    Rgba8 = 0x0,
    Rgb8 = 0x1,
    Rgba5551 = 0x2,
    Rgb565 = 0x3,
    Rgba4 = 0x4,
    /// Luminance and alpha, 8 bits each.
    La8 = 0x5,
    /// Two 8-bit channels (normal maps).
    HiLo8 = 0x6,
    /// Luminance.
    L8 = 0x7,
    /// Alpha (white texels): fonts, masks.
    A8 = 0x8,
    La4 = 0x9,
    L4 = 0xA,
    A4 = 0xB,
    /// Ericsson Texture Compression (load pre-compressed data).
    Etc1 = 0xC,
    Etc1A4 = 0xD,
}

impl TexFormat {
    /// Bits per texel.
    pub const fn bits_per_texel(self) -> usize {
        match self {
            TexFormat::Rgba8 => 32,
            TexFormat::Rgb8 => 24,
            TexFormat::Rgba5551 | TexFormat::Rgb565 | TexFormat::Rgba4 | TexFormat::La8 | TexFormat::HiLo8 => 16,
            TexFormat::L8 | TexFormat::A8 | TexFormat::La4 | TexFormat::Etc1A4 => 8,
            TexFormat::L4 | TexFormat::A4 | TexFormat::Etc1 => 4,
        }
    }
}

/// Texture filtering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Nearest = 0,
    Linear = 1,
}

/// What happens outside 0..1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrap {
    ClampToEdge = 0,
    ClampToBorder = 1,
    Repeat = 2,
    MirroredRepeat = 3,
}

/// The offset of texel (x, y) (y counted from the top of the image) in a
/// tiled image `width` wide and `height` tall, in texels.
pub fn tiled_index(x: u32, y: u32, width: u32, _height: u32) -> u32 {
    let tile = (y / 8) * (width / 8) + x / 8;
    let (tx, ty) = (x & 7, y & 7);
    let morton = (tx & 1) | (ty & 1) << 1 | (tx & 2) << 1 | (ty & 2) << 2 | (tx & 4) << 2 | (ty & 4) << 3;
    tile * 64 + morton
}

/// Encodes an RGBA colour as a texel of `format` (little-endian bytes).
/// Returns the bytes and how many are used (0 for 4-bit and ETC formats,
/// which [`encode_nibble`] handles).
pub fn encode_texel(format: TexFormat, [r, g, b, a]: [u8; 4]) -> ([u8; 4], usize) {
    let l = ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8;
    let h16 = |v: u16| ([v as u8, (v >> 8) as u8, 0, 0], 2);
    match format {
        TexFormat::Rgba8 => ([a, b, g, r], 4),
        TexFormat::Rgb8 => ([b, g, r, 0], 3),
        TexFormat::Rgba5551 => h16((r as u16 >> 3) << 11 | (g as u16 >> 3) << 6 | (b as u16 >> 3) << 1 | (a as u16 >> 7)),
        TexFormat::Rgb565 => h16((r as u16 >> 3) << 11 | (g as u16 >> 2) << 5 | (b as u16 >> 3)),
        TexFormat::Rgba4 => h16((r as u16 >> 4) << 12 | (g as u16 >> 4) << 8 | (b as u16 >> 4) << 4 | (a as u16 >> 4)),
        TexFormat::La8 => ([a, l, 0, 0], 2),
        TexFormat::HiLo8 => ([g, r, 0, 0], 2),
        TexFormat::L8 => ([l, 0, 0, 0], 1),
        TexFormat::A8 => ([a, 0, 0, 0], 1),
        TexFormat::La4 => ([(l >> 4) << 4 | (a >> 4), 0, 0, 0], 1),
        TexFormat::L4 | TexFormat::A4 | TexFormat::Etc1 | TexFormat::Etc1A4 => ([0; 4], 0),
    }
}

/// The 4-bit value of a texel in `L4` or `A4`.
pub fn encode_nibble(format: TexFormat, [r, g, b, a]: [u8; 4]) -> u8 {
    match format {
        TexFormat::A4 => a >> 4,
        _ => (((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8) >> 4,
    }
}

/// Converts and tiles an RGBA8 image (rows top to bottom) into `out`.
pub fn tile_rgba(format: TexFormat, width: u32, height: u32, rgba: &[u8], out: &mut [u8]) {
    assert!(rgba.len() >= (width * height * 4) as usize, "image data too short");
    let bits = format.bits_per_texel();
    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            let px = [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]];
            let t = tiled_index(x, y, width, height) as usize;
            if bits == 4 {
                let n = encode_nibble(format, px);
                let byte = &mut out[t / 2];
                *byte = if t & 1 == 0 { (*byte & 0xF0) | n } else { (*byte & 0x0F) | n << 4 };
            } else {
                let (bytes, n) = encode_texel(format, px);
                out[t * n..t * n + n].copy_from_slice(&bytes[..n]);
            }
        }
    }
}

/// A pre-tiled texture file (`.ctex`), as `citronnade-assets` writes it in
/// build scripts: a 16-byte header (`CTEX`, width and height as u16, the
/// format, a reserved byte and u16, the data size as u32, little-endian),
/// then the tiled texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CtexHeader {
    pub width: u32,
    pub height: u32,
    pub format: TexFormat,
    pub data_len: u32,
}

impl TexFormat {
    /// The format with this register value.
    pub fn from_code(code: u8) -> Option<TexFormat> {
        use TexFormat::*;
        [Rgba8, Rgb8, Rgba5551, Rgb565, Rgba4, La8, HiLo8, L8, A8, La4, L4, A4, Etc1, Etc1A4].into_iter().find(|f| *f as u8 == code)
    }
}

impl CtexHeader {
    pub const SIZE: usize = 16;

    pub fn to_bytes(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..4].copy_from_slice(b"CTEX");
        b[4..6].copy_from_slice(&(self.width as u16).to_le_bytes());
        b[6..8].copy_from_slice(&(self.height as u16).to_le_bytes());
        b[8] = self.format as u8;
        b[12..16].copy_from_slice(&self.data_len.to_le_bytes());
        b
    }

    /// Reads a header; `None` if `data` is not a `.ctex` file.
    pub fn parse(data: &[u8]) -> Option<CtexHeader> {
        if data.len() < 16 || &data[0..4] != b"CTEX" {
            return None;
        }
        let width = u16::from_le_bytes([data[4], data[5]]) as u32;
        let height = u16::from_le_bytes([data[6], data[7]]) as u32;
        let format = TexFormat::from_code(data[8])?;
        let data_len = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
        if data.len() < 16 + data_len as usize || data_len as usize != (width * height) as usize * format.bits_per_texel() / 8 {
            return None;
        }
        Some(CtexHeader { width, height, format, data_len })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_order() {
        // The top-left texel comes first, then its right neighbour, then
        // the one below.
        assert_eq!(tiled_index(0, 0, 8, 8), 0);
        assert_eq!(tiled_index(1, 0, 8, 8), 1);
        assert_eq!(tiled_index(0, 1, 8, 8), 2);
        assert_eq!(tiled_index(1, 1, 8, 8), 3);
        assert_eq!(tiled_index(2, 0, 8, 8), 4);
        assert_eq!(tiled_index(7, 7, 8, 8), 63);
        // the second tile is to the right of the first
        assert_eq!(tiled_index(8, 0, 16, 16), 64);
        // the third is below the first
        assert_eq!(tiled_index(0, 8, 16, 16), 128);
        // every texel gets its own slot
        let mut seen = std::vec![false; 32 * 16];
        for y in 0..16 {
            for x in 0..32 {
                let i = tiled_index(x, y, 32, 16) as usize;
                assert!(!seen[i]);
                seen[i] = true;
            }
        }
    }

    #[test]
    fn ctex_headers() {
        let h = CtexHeader { width: 16, height: 8, format: TexFormat::Rgb565, data_len: 256 };
        let mut file = h.to_bytes().to_vec();
        file.extend_from_slice(&[0; 256]);
        assert_eq!(CtexHeader::parse(&file), Some(h));
        assert_eq!(CtexHeader::parse(&file[..100]), None);
    }

    #[test]
    fn texel_encodings() {
        assert_eq!(encode_texel(TexFormat::Rgba8, [1, 2, 3, 4]), ([4, 3, 2, 1], 4));
        assert_eq!(encode_texel(TexFormat::Rgb565, [255, 0, 0, 255]).0[..2], [0x00, 0xF8]);
        assert_eq!(encode_texel(TexFormat::A8, [0, 0, 0, 200]), ([200, 0, 0, 0], 1));
        assert_eq!(encode_texel(TexFormat::La8, [255, 255, 255, 7]).0[..2], [7, 255]);
    }
}
