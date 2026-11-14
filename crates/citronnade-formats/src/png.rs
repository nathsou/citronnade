//! Decoding PNG images to RGBA8: every colour type, bit depths 1-16,
//! non-interlaced (Adam7 interlacing is rejected).

use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use crate::zlib;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PngError {
    NotPng,
    Truncated,
    BadHeader,
    Unsupported(&'static str),
    Inflate(zlib::InflateError),
    BadFilter(u8),
    MissingPalette,
}

impl fmt::Display for PngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PngError::NotPng => f.write_str("not a PNG file"),
            PngError::Truncated => f.write_str("truncated PNG file"),
            PngError::BadHeader => f.write_str("bad PNG header"),
            PngError::Unsupported(what) => write!(f, "unsupported PNG: {what}"),
            PngError::Inflate(e) => write!(f, "bad PNG image data: {e}"),
            PngError::BadFilter(t) => write!(f, "bad PNG filter type {t}"),
            PngError::MissingPalette => f.write_str("indexed PNG without a palette"),
        }
    }
}

/// A decoded image: `width * height` RGBA8 pixels, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// Resizes to `w` x `h` with nearest-neighbour sampling.
    pub fn resized(&self, w: u32, h: u32) -> Image {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let sx = (x * self.width / w) as usize;
                let sy = (y * self.height / h) as usize;
                let s = 4 * (sy * self.width as usize + sx);
                let d = 4 * (y * w + x) as usize;
                rgba[d..d + 4].copy_from_slice(&self.rgba[s..s + 4]);
            }
        }
        Image { width: w, height: h, rgba }
    }
}

pub fn decode(data: &[u8]) -> Result<Image, PngError> {
    if data.len() < 8 || &data[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err(PngError::NotPng);
    }
    let mut pos = 8;
    let (mut width, mut height, mut depth, mut color, mut interlace) = (0u32, 0u32, 0u8, 0u8, 0u8);
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat = Vec::new();
    let mut seen_header = false;
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = &data[pos + 4..pos + 8];
        let body = data.get(pos + 8..pos + 8 + len).ok_or(PngError::Truncated)?;
        pos += 12 + len;
        match kind {
            b"IHDR" => {
                if body.len() < 13 {
                    return Err(PngError::BadHeader);
                }
                width = u32::from_be_bytes(body[0..4].try_into().unwrap());
                height = u32::from_be_bytes(body[4..8].try_into().unwrap());
                depth = body[8];
                color = body[9];
                interlace = body[12];
                seen_header = true;
            }
            b"PLTE" => {
                palette = body.chunks_exact(3).map(|c| [c[0], c[1], c[2], 255]).collect();
            }
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
    }
    if !seen_header || width == 0 || height == 0 {
        return Err(PngError::BadHeader);
    }
    if interlace != 0 {
        return Err(PngError::Unsupported("interlaced images"));
    }
    let channels = match color {
        0 => 1,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => return Err(PngError::BadHeader),
    };
    if !matches!(depth, 1 | 2 | 4 | 8 | 16) {
        return Err(PngError::BadHeader);
    }
    if color == 3 {
        if palette.is_empty() {
            return Err(PngError::MissingPalette);
        }
        for (i, &a) in trns.iter().enumerate() {
            if let Some(p) = palette.get_mut(i) {
                p[3] = a;
            }
        }
    }
    let raw = zlib::decompress(&idat).map_err(PngError::Inflate)?;
    let bits_per_pixel = channels * depth as usize;
    let stride = (width as usize * bits_per_pixel).div_ceil(8);
    let bpp = bits_per_pixel.div_ceil(8).max(1);
    if raw.len() < (stride + 1) * height as usize {
        return Err(PngError::Truncated);
    }

    // Undo the per-row filters.
    let mut pixels = vec![0u8; stride * height as usize];
    for y in 0..height as usize {
        let filter = raw[y * (stride + 1)];
        let src = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        let (done, rest) = pixels.split_at_mut(y * stride);
        let prev = if y == 0 { None } else { Some(&done[(y - 1) * stride..]) };
        let row = &mut rest[..stride];
        for x in 0..stride {
            let a = if x >= bpp { row[x - bpp] } else { 0 };
            let b = prev.map_or(0, |p| p[x]);
            let c = if x >= bpp { prev.map_or(0, |p| p[x - bpp]) } else { 0 };
            row[x] = match filter {
                0 => src[x],
                1 => src[x].wrapping_add(a),
                2 => src[x].wrapping_add(b),
                3 => src[x].wrapping_add(((a as u16 + b as u16) / 2) as u8),
                4 => src[x].wrapping_add(paeth(a, b, c)),
                t => return Err(PngError::BadFilter(t)),
            };
        }
    }

    // Expand to RGBA8.
    let sample = |row: &[u8], i: usize| -> u16 {
        match depth {
            16 => u16::from_be_bytes([row[2 * i], row[2 * i + 1]]),
            8 => row[i] as u16,
            d => {
                let per_byte = 8 / d as usize;
                let byte = row[i / per_byte];
                let shift = 8 - d as usize * (i % per_byte + 1);
                ((byte >> shift) & ((1 << d) - 1)) as u16
            }
        }
    };
    let to8 = |v: u16| -> u8 {
        match depth {
            16 => (v >> 8) as u8,
            8 => v as u8,
            d => (v * 255 / ((1 << d) - 1)) as u8,
        }
    };
    let key = |i: usize| -> Option<u16> {
        trns.get(2 * i..2 * i + 2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    };
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for y in 0..height as usize {
        let row = &pixels[y * stride..(y + 1) * stride];
        for x in 0..width as usize {
            let out = &mut rgba[4 * (y * width as usize + x)..][..4];
            match color {
                0 => {
                    let v = sample(row, x);
                    let g = to8(v);
                    let a = if key(0) == Some(v) { 0 } else { 255 };
                    out.copy_from_slice(&[g, g, g, a]);
                }
                2 => {
                    let (r, g, b) = (sample(row, 3 * x), sample(row, 3 * x + 1), sample(row, 3 * x + 2));
                    let transparent = key(0) == Some(r) && key(1) == Some(g) && key(2) == Some(b);
                    out.copy_from_slice(&[to8(r), to8(g), to8(b), if transparent { 0 } else { 255 }]);
                }
                3 => {
                    let i = sample(row, x) as usize;
                    out.copy_from_slice(palette.get(i).unwrap_or(&[0, 0, 0, 255]));
                }
                4 => {
                    let g = to8(sample(row, 2 * x));
                    out.copy_from_slice(&[g, g, g, to8(sample(row, 2 * x + 1))]);
                }
                _ => {
                    for c in 0..4 {
                        out[c] = to8(sample(row, 4 * x + c));
                    }
                }
            }
        }
    }
    Ok(Image { width, height, rgba })
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_rgba_with_filters() {
        let img = decode(include_bytes!("../tests/gradient.png")).unwrap();
        assert_eq!((img.width, img.height), (13, 7));
        // pixel (x, y) = (x * 19, y * 37, (x + y) * 11, 255 - x * 5)
        for y in 0..7u32 {
            for x in 0..13u32 {
                let p = &img.rgba[4 * (y * 13 + x) as usize..][..4];
                assert_eq!(p, &[(x * 19) as u8, (y * 37) as u8, ((x + y) * 11) as u8, (255 - x * 5) as u8]);
            }
        }
    }

    #[test]
    fn decodes_palette_with_transparency() {
        let img = decode(include_bytes!("../tests/palette.png")).unwrap();
        assert_eq!((img.width, img.height), (4, 2));
        assert_eq!(&img.rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&img.rgba[4..8], &[0, 255, 0, 128]);
        assert_eq!(&img.rgba[8..12], &[0, 0, 255, 0]);
    }

    #[test]
    fn rejects_non_png() {
        assert_eq!(decode(b"GIF89a.."), Err(PngError::NotPng));
    }
}
