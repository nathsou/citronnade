//! Meshes in the `.cmsh` format that `citronnade-assets` writes from OBJ
//! files: interleaved vertices (position, normal, texture coordinates as
//! floats), 16-bit indices for triangles, and named submeshes (OBJ objects
//! or materials) as index ranges.
//!
//! Layout (little-endian): `CMSH`, version 1, vertex count, index count,
//! submesh count (u32 each); per submesh the first index, the index count
//! (u32) and a 16-byte zero-padded name; the vertices (32 bytes each); the
//! indices, padded to 4 bytes.

use alloc::string::String;
use alloc::vec::Vec;

/// One vertex: attributes v0 (position), v1 (normal), v2 (uv).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// A range of indices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submesh {
    pub name: String,
    pub first: u32,
    pub count: u32,
}

/// A parsed mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshData {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u16>,
    pub submeshes: Vec<Submesh>,
}

/// Why a `.cmsh` could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshError {
    BadMagic,
    Truncated,
    IndexOutOfRange,
}

impl core::fmt::Display for MeshError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            MeshError::BadMagic => "not a .cmsh mesh",
            MeshError::Truncated => "truncated mesh",
            MeshError::IndexOutOfRange => "mesh index out of range",
        })
    }
}

fn word(d: &[u8], at: usize) -> Result<u32, MeshError> {
    d.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or(MeshError::Truncated)
}

impl MeshData {
    /// Writes the `.cmsh` bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"CMSH");
        for w in [1u32, self.vertices.len() as u32, self.indices.len() as u32, self.submeshes.len() as u32] {
            out.extend_from_slice(&w.to_le_bytes());
        }
        for s in &self.submeshes {
            out.extend_from_slice(&s.first.to_le_bytes());
            out.extend_from_slice(&s.count.to_le_bytes());
            let mut name = [0u8; 16];
            let n = s.name.len().min(15);
            name[..n].copy_from_slice(&s.name.as_bytes()[..n]);
            out.extend_from_slice(&name);
        }
        for v in &self.vertices {
            for f in v.position.iter().chain(&v.normal).chain(&v.uv) {
                out.extend_from_slice(&f.to_le_bytes());
            }
        }
        for i in &self.indices {
            out.extend_from_slice(&i.to_le_bytes());
        }
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }

    /// Reads `.cmsh` bytes.
    pub fn parse(d: &[u8]) -> Result<MeshData, MeshError> {
        if d.get(0..4) != Some(b"CMSH") || word(d, 4)? != 1 {
            return Err(MeshError::BadMagic);
        }
        let (nv, ni, ns) = (word(d, 8)? as usize, word(d, 12)? as usize, word(d, 16)? as usize);
        let mut at = 20;
        let mut submeshes = Vec::with_capacity(ns);
        for _ in 0..ns {
            let (first, count) = (word(d, at)?, word(d, at + 4)?);
            let name = d.get(at + 8..at + 24).ok_or(MeshError::Truncated)?;
            let len = name.iter().position(|&b| b == 0).unwrap_or(16);
            submeshes.push(Submesh { name: String::from_utf8_lossy(&name[..len]).into_owned(), first, count });
            at += 24;
        }
        let f = |at: usize| word(d, at).map(f32::from_bits);
        let mut vertices = Vec::with_capacity(nv);
        for _ in 0..nv {
            vertices.push(MeshVertex {
                position: [f(at)?, f(at + 4)?, f(at + 8)?],
                normal: [f(at + 12)?, f(at + 16)?, f(at + 20)?],
                uv: [f(at + 24)?, f(at + 28)?],
            });
            at += 32;
        }
        let bytes = d.get(at..at + ni * 2).ok_or(MeshError::Truncated)?;
        let indices: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        if indices.iter().any(|&i| i as usize >= nv) || submeshes.iter().any(|s| (s.first + s.count) as usize > ni) {
            return Err(MeshError::IndexOutOfRange);
        }
        Ok(MeshData { vertices, indices, submeshes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let m = MeshData {
            vertices: alloc::vec![
                MeshVertex { position: [0.0, 1.0, 2.0], normal: [0.0, 0.0, 1.0], uv: [0.5, 0.25] },
                MeshVertex::default(),
                MeshVertex::default(),
            ],
            indices: alloc::vec![0, 1, 2],
            submeshes: alloc::vec![Submesh { name: "peel".into(), first: 0, count: 3 }],
        };
        let b = m.to_bytes();
        assert_eq!(MeshData::parse(&b), Ok(m));
        assert_eq!(MeshData::parse(&b[..30]), Err(MeshError::Truncated));
    }
}
