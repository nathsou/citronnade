//! Asset conversion for build scripts, so the console loads data ready to
//! use: PNG images become textures in the GPU's tiled layout (`.ctex`
//! files, loaded with `citronnade::gpu::Texture::from_ctex`), and OBJ
//! models become meshes (`.cmsh`, loaded with `citronnade::gpu::Mesh`).
//!
//! ```text
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

pub use citronnade_gpu::mesh::{MeshData, MeshVertex, Submesh};

/// Converts a Wavefront OBJ model to a mesh: triangles (polygons are split
/// into fans), vertices deduplicated, one submesh per object (`o`), group
/// (`g`) or material (`usemtl`) change. Missing normals are computed from
/// the faces; missing texture coordinates are zero.
pub fn mesh_from_obj(obj: &str) -> Result<MeshData, String> {
    use std::collections::HashMap;
    let (mut positions, mut normals, mut uvs) = (Vec::new(), Vec::new(), Vec::new());
    let mut mesh = MeshData::default();
    let mut index_of: HashMap<(usize, Option<usize>, Option<usize>), u16> = HashMap::new();
    let mut missing_normals = false;
    let mut name = String::from("default");
    let mut start = 0u32;
    let close = |mesh: &mut MeshData, name: &str, start: &mut u32| {
        let count = mesh.indices.len() as u32 - *start;
        if count > 0 {
            mesh.submeshes.push(Submesh { name: name.to_string(), first: *start, count });
        }
        *start = mesh.indices.len() as u32;
    };
    for (n, line) in obj.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        let Some(kind) = words.next() else { continue };
        let nums = |w: std::str::SplitWhitespace| -> Result<Vec<f32>, String> {
            w.map(|x| x.parse::<f32>().map_err(|_| format!("line {}: bad number `{x}`", n + 1))).collect()
        };
        match kind {
            "v" => {
                let v = nums(words)?;
                positions.push([v[0], v[1], v[2]]);
            }
            "vn" => {
                let v = nums(words)?;
                normals.push([v[0], v[1], v[2]]);
            }
            "vt" => {
                let v = nums(words)?;
                uvs.push([v[0], *v.get(1).unwrap_or(&0.0)]);
            }
            "o" | "g" | "usemtl" => {
                close(&mut mesh, &name, &mut start);
                name = words.next().unwrap_or("default").to_string();
            }
            "f" => {
                let mut corners = Vec::new();
                for w in words {
                    let mut parts = w.split('/');
                    let idx = |p: Option<&str>, len: usize| -> Result<Option<usize>, String> {
                        match p {
                            None | Some("") => Ok(None),
                            Some(t) => {
                                let i: i64 = t.parse().map_err(|_| format!("line {}: bad index `{t}`", n + 1))?;
                                let i = if i < 0 { len as i64 + i } else { i - 1 };
                                if i < 0 || i as usize >= len {
                                    return Err(format!("line {}: index {t} out of range", n + 1));
                                }
                                Ok(Some(i as usize))
                            }
                        }
                    };
                    let v = idx(parts.next(), positions.len())?.ok_or(format!("line {}: face without a position", n + 1))?;
                    let t = idx(parts.next(), uvs.len())?;
                    let vn = idx(parts.next(), normals.len())?;
                    missing_normals |= vn.is_none();
                    let key = (v, t, vn);
                    let i = match index_of.get(&key) {
                        Some(&i) => i,
                        None => {
                            if mesh.vertices.len() >= 65536 {
                                return Err("more than 65536 vertices".into());
                            }
                            let i = mesh.vertices.len() as u16;
                            mesh.vertices.push(MeshVertex {
                                position: positions[v],
                                normal: vn.map_or([0.0; 3], |k| normals[k]),
                                uv: t.map_or([0.0; 2], |k| uvs[k]),
                            });
                            index_of.insert(key, i);
                            i
                        }
                    };
                    corners.push(i);
                }
                for k in 1..corners.len().saturating_sub(1) {
                    mesh.indices.extend_from_slice(&[corners[0], corners[k], corners[k + 1]]);
                }
            }
            _ => {}
        }
    }
    close(&mut mesh, &name, &mut start);
    if missing_normals {
        // Area-weighted face normals, accumulated per vertex.
        let mut acc = vec![[0.0f32; 3]; mesh.vertices.len()];
        for t in mesh.indices.chunks_exact(3) {
            let p = |i: u16| mesh.vertices[i as usize].position;
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
            let nrm = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            for &i in t {
                for k in 0..3 {
                    acc[i as usize][k] += nrm[k];
                }
            }
        }
        for (v, n) in mesh.vertices.iter_mut().zip(acc) {
            if v.normal == [0.0; 3] {
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
                v.normal = [n[0] / l, n[1] / l, n[2] / l];
            }
        }
    }
    Ok(mesh)
}

/// For build scripts: converts `src` (an OBJ file, relative to the package)
/// to `<dest_dir>/<name>.cmsh` (relative to the package; a RomFS directory,
/// say) and asks Cargo to rerun when it changes.
pub fn build_mesh(src: impl AsRef<Path>, dest_dir: impl AsRef<Path>) -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("run from a build script");
    let src = Path::new(&manifest).join(src.as_ref());
    println!("cargo:rerun-if-changed={}", src.display());
    let obj = fs::read_to_string(&src).unwrap_or_else(|e| panic!("cannot read {}: {e}", src.display()));
    let mesh = mesh_from_obj(&obj).unwrap_or_else(|e| panic!("{}: {e}", src.display()));
    let dir = Path::new(&manifest).join(dest_dir.as_ref());
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("cannot create {}: {e}", dir.display()));
    let out = dir.join(format!("{}.cmsh", src.file_stem().unwrap().to_string_lossy()));
    let bytes = mesh.to_bytes();
    // Leave the file alone when unchanged so packaging stays incremental.
    if fs::read(&out).ok().as_deref() != Some(&bytes[..]) {
        fs::write(&out, bytes).unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));
    }
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

    #[test]
    fn converts_obj() {
        let obj = "o square\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 1\nf 1/1 2/1 3/2 4/2\no tri\nf 1 2 3\n";
        let m = mesh_from_obj(obj).unwrap();
        assert_eq!(m.indices.len(), 9);
        assert_eq!(m.submeshes.len(), 2);
        assert_eq!(m.submeshes[0].name, "square");
        assert_eq!((m.submeshes[1].first, m.submeshes[1].count), (6, 3));
        // computed normals face +z
        assert!(m.vertices.iter().all(|v| (v.normal[2] - 1.0).abs() < 1e-5));
        assert!(mesh_from_obj("f 1 2 3").is_err());
    }
}
