//! SMDH: the title, publisher and icons the Homebrew Launcher shows.
//!
//! Layout (0x36C0 bytes): `"SMDH"`, version, 16 per-language titles (short
//! description, long description and publisher in UTF-16), settings
//! (ratings, region lock, flags, ...), then a 24x24 and a 48x48 icon in
//! RGB565, tiled in 8x8 blocks in Morton order.

use alloc::vec;
use alloc::vec::Vec;

pub const SIZE: usize = 0x36C0;
const TITLE_SIZE: usize = 0x200;
const SHORT_LEN: usize = 0x40;
const LONG_LEN: usize = 0x80;
const PUBLISHER_LEN: usize = 0x40;

pub const FLAG_VISIBLE: u32 = 1 << 0;
pub const FLAG_AUTOBOOT: u32 = 1 << 1;
pub const FLAG_USE_3D: u32 = 1 << 2;
pub const FLAG_REGION_RATING_USED: u32 = 1 << 6;
pub const FLAG_RECORD_USAGE: u32 = 1 << 8;

/// A 48x48 RGBA8 image (row-major, 4 bytes per pixel).
pub type Icon = Vec<u8>;

/// SMDH contents. Titles are the same in every language.
#[derive(Clone, Debug)]
pub struct Smdh<'a> {
    pub short_description: &'a str,
    pub long_description: &'a str,
    pub publisher: &'a str,
    /// 48x48 RGBA8; the 24x24 icon is downscaled from it.
    pub icon: &'a [u8],
    pub flags: u32,
}

impl Smdh<'_> {
    pub fn to_bytes(&self) -> Vec<u8> {
        assert_eq!(self.icon.len(), 48 * 48 * 4, "the icon must be 48x48 RGBA8");
        let mut out = vec![0u8; SIZE];
        out[0..4].copy_from_slice(b"SMDH");
        for lang in 0..16 {
            let at = 8 + lang * TITLE_SIZE;
            write_utf16(&mut out[at..at + SHORT_LEN * 2], self.short_description);
            write_utf16(&mut out[at + SHORT_LEN * 2..at + (SHORT_LEN + LONG_LEN) * 2], self.long_description);
            write_utf16(
                &mut out[at + (SHORT_LEN + LONG_LEN) * 2..at + (SHORT_LEN + LONG_LEN + PUBLISHER_LEN) * 2],
                self.publisher,
            );
        }
        let settings = 8 + 16 * TITLE_SIZE;
        // Ratings: enabled with no restriction, for each rating board in use.
        for board in [0, 1, 3, 4, 6, 7, 8, 9, 10] {
            out[settings + board] = 0x80 | 0x20;
        }
        out[settings + 0x10..settings + 0x14].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // all regions
        out[settings + 0x20..settings + 0x24].copy_from_slice(&self.flags.to_le_bytes());

        let small = downscale(self.icon);
        let small_at = 0x2040;
        let large_at = small_at + 24 * 24 * 2;
        write_icon(&mut out[small_at..large_at], &small, 24);
        write_icon(&mut out[large_at..SIZE], self.icon, 48);
        out
    }
}

/// The default flags for homebrew: visible, stereoscopic 3D allowed (an
/// installed title gets 3D only with it), usage recorded, ratings used.
pub const DEFAULT_FLAGS: u32 = FLAG_VISIBLE | FLAG_USE_3D | FLAG_RECORD_USAGE | FLAG_REGION_RATING_USED;

fn write_utf16(dst: &mut [u8], s: &str) {
    let max = dst.len() / 2 - 1; // keep a terminating NUL
    let mut n = 0;
    for unit in s.encode_utf16() {
        if n == max {
            break;
        }
        dst[n * 2..n * 2 + 2].copy_from_slice(&unit.to_le_bytes());
        n += 1;
    }
}

/// Position of pixel (x, y) inside an 8x8 tile in Morton (Z) order.
pub fn morton8(x: usize, y: usize) -> usize {
    let mut m = 0;
    for bit in 0..3 {
        m |= ((x >> bit) & 1) << (2 * bit);
        m |= ((y >> bit) & 1) << (2 * bit + 1);
    }
    m
}

fn rgb565(px: &[u8]) -> u16 {
    // Premultiply by alpha (over black), as the launcher shows no alpha.
    let a = px[3] as u32;
    let r = px[0] as u32 * a / 255;
    let g = px[1] as u32 * a / 255;
    let b = px[2] as u32 * a / 255;
    ((r >> 3) << 11 | (g >> 2) << 5 | (b >> 3)) as u16
}

fn write_icon(dst: &mut [u8], rgba: &[u8], side: usize) {
    let mut n = 0;
    for ty in (0..side).step_by(8) {
        for tx in (0..side).step_by(8) {
            let mut tile = [0u16; 64];
            for y in 0..8 {
                for x in 0..8 {
                    let p = 4 * ((ty + y) * side + tx + x);
                    tile[morton8(x, y)] = rgb565(&rgba[p..p + 4]);
                }
            }
            for px in tile {
                dst[n..n + 2].copy_from_slice(&px.to_le_bytes());
                n += 2;
            }
        }
    }
}

fn downscale(rgba48: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; 24 * 24 * 4];
    for y in 0..24 {
        for x in 0..24 {
            for c in 0..4 {
                let at = |xx: usize, yy: usize| rgba48[4 * (yy * 48 + xx) + c] as u32;
                let sum = at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1);
                out[4 * (y * 24 + x) + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn morton_order_matches_the_hardware_table() {
        // The first row of tile positions, as the GPU and the launcher expect.
        let expected = [0, 1, 4, 5, 16, 17, 20, 21];
        for x in 0..8 {
            assert_eq!(morton8(x, 0), expected[x]);
        }
        assert_eq!(morton8(0, 1), 2);
        assert_eq!(morton8(7, 7), 63);
    }

    #[test]
    fn writes_titles_and_icon() {
        let mut icon = vec![0u8; 48 * 48 * 4];
        for px in icon.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }
        let s = Smdh {
            short_description: "Hello",
            long_description: "A long description",
            publisher: "Citronnade",
            icon: &icon,
            flags: DEFAULT_FLAGS,
        };
        let b = s.to_bytes();
        assert_eq!(b.len(), SIZE);
        assert_eq!(&b[0..4], b"SMDH");
        assert_eq!(&b[8..18], &[b'H', 0, b'e', 0, b'l', 0, b'l', 0, b'o', 0]);
        // second language has the same title
        assert_eq!(&b[8 + 0x200..8 + 0x200 + 2], &[b'H', 0]);
        // red in RGB565 is 0xF800
        assert_eq!(&b[0x2040..0x2042], &0xF800u16.to_le_bytes());
        assert_eq!(&b[SIZE - 2..], &0xF800u16.to_le_bytes());
    }

    #[test]
    fn truncates_long_titles() {
        let long: alloc::string::String = core::iter::repeat('x').take(500).collect();
        let icon = vec![0u8; 48 * 48 * 4];
        let s = Smdh { short_description: &long, long_description: "", publisher: "", icon: &icon, flags: 0 };
        let b = s.to_bytes();
        // the last UTF-16 unit of the short description stays NUL
        assert_eq!(&b[8 + 0x7E..8 + 0x80], &[0, 0]);
        assert_eq!(&b[8 + 0x7C..8 + 0x7E], &[b'x', 0]);
    }
}
