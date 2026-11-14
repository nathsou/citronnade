//! Asset conversion for build scripts, so the console loads data ready to
//! use: PNG images become textures in the GPU's tiled layout (`.ctex`
//! files, loaded with `citronnade::gpu::Texture::from_ctex`).
//!
//! ```ignore
//! // build.rs
//! use citronnade_assets::{build_texture, TexFormat};
//! fn main() {
//!     build_texture("assets/tile.png", TexFormat::Rgba5551);
//! }
//!
//! // src/main.rs
//! const TILE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tile.ctex"));
//! let tile = Texture::from_ctex(TILE)?;
//! ```

use std::fs;
use std::path::{Path, PathBuf};

pub use citronnade_gpu::tiling::{CtexHeader, TexFormat};
use citronnade_gpu::tiling::tile_rgba;

/// Converts PNG data to a `.ctex` texture file in `format`. The image's
/// sides must be powers of two from 8 to 1024.
pub fn texture_from_png(png: &[u8], format: TexFormat) -> Result<Vec<u8>, String> {
    let image = citronnade_formats::png::decode(png).map_err(|e| e.to_string())?;
    texture_from_rgba(image.width, image.height, &image.rgba, format)
}

/// Converts an RGBA8 image (rows from the top) to a `.ctex` texture file.
pub fn texture_from_rgba(width: u32, height: u32, rgba: &[u8], format: TexFormat) -> Result<Vec<u8>, String> {
    let ok = |n: u32| n.is_power_of_two() && (8..=1024).contains(&n);
    if !ok(width) || !ok(height) {
        return Err(format!("texture sides must be powers of two from 8 to 1024, not {width}x{height}"));
    }
    if matches!(format, TexFormat::Etc1 | TexFormat::Etc1A4) {
        return Err("ETC1 compression is not supported yet".into());
    }
    let len = (width * height) as usize * format.bits_per_texel() / 8;
    let mut out = CtexHeader { width, height, format, data_len: len as u32 }.to_bytes().to_vec();
    out.resize(CtexHeader::SIZE + len, 0);
    tile_rgba(format, width, height, rgba, &mut out[CtexHeader::SIZE..]);
    Ok(out)
}

/// For build scripts: converts `src` (a PNG, relative to the package) to
/// `$OUT_DIR/<name>.ctex` and asks Cargo to rerun when it changes. Panics
/// with a readable message on errors, failing the build.
pub fn build_texture(src: impl AsRef<Path>, format: TexFormat) -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("run from a build script");
    let src = Path::new(&manifest).join(src.as_ref());
    println!("cargo:rerun-if-changed={}", src.display());
    let png = fs::read(&src).unwrap_or_else(|e| panic!("cannot read {}: {e}", src.display()));
    let ctex = texture_from_png(&png, format).unwrap_or_else(|e| panic!("{}: {e}", src.display()));
    let out_dir = std::env::var("OUT_DIR").expect("run from a build script");
    let name = src.file_stem().unwrap().to_string_lossy();
    let out = Path::new(&out_dir).join(format!("{name}.ctex"));
    fs::write(&out, ctex).unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_rgba() {
        let rgba: Vec<u8> = (0..8 * 8).flat_map(|i| [i as u8, 0, 0, 255]).collect();
        let ctex = texture_from_rgba(8, 8, &rgba, TexFormat::Rgba8).unwrap();
        let h = CtexHeader::parse(&ctex).unwrap();
        assert_eq!((h.width, h.height, h.format, h.data_len), (8, 8, TexFormat::Rgba8, 256));
        // texel (1, 0) is second in memory: bytes A, B, G, R
        assert_eq!(&ctex[16 + 4..16 + 8], &[255, 0, 0, 1]);
        assert!(texture_from_rgba(6, 8, &rgba, TexFormat::Rgba8).is_err());
    }
}
