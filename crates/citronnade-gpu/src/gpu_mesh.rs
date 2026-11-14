//! Meshes in linear memory, ready to draw.

use alloc::vec::Vec;

use crate::context::{AttribType, Gpu, Primitive, VertexLayout};
use crate::linear::LinearBuffer;
use crate::mesh::{MeshData, MeshError, MeshVertex, Submesh};

/// A mesh's vertices and indices in linear memory.
pub struct Mesh {
    pub vertices: LinearBuffer<MeshVertex>,
    pub indices: LinearBuffer<u16>,
    pub submeshes: Vec<Submesh>,
}

impl Mesh {
    /// Loads `.cmsh` bytes.
    pub fn from_cmsh(bytes: &[u8]) -> Result<Mesh, MeshError> {
        Mesh::from_data(&MeshData::parse(bytes)?).map_err(|_| MeshError::Truncated)
    }

    pub fn from_data(data: &MeshData) -> crate::Result<Mesh> {
        Ok(Mesh {
            vertices: LinearBuffer::from_slice(&data.vertices)?,
            indices: LinearBuffer::from_slice(&data.indices)?,
            submeshes: data.submeshes.clone(),
        })
    }

    /// The vertex layout: position in v0, normal in v1, uv in v2.
    pub fn layout() -> VertexLayout {
        VertexLayout::new().attrib(0, AttribType::Float, 3).attrib(1, AttribType::Float, 3).attrib(2, AttribType::Float, 2)
    }

    /// Finds a submesh by name.
    pub fn submesh(&self, name: &str) -> Option<&Submesh> {
        self.submeshes.iter().find(|s| s.name == name)
    }

    /// Binds the layout and vertices.
    pub fn bind(&self, gpu: &mut Gpu) {
        gpu.set_layout(&Mesh::layout());
        gpu.bind_vertices(&self.vertices);
    }

    /// Draws one submesh (after [`Mesh::bind`]).
    pub fn draw_submesh(&self, gpu: &mut Gpu, submesh: &Submesh) {
        gpu.draw_elements_range(Primitive::Triangles, &self.indices, submesh.first, submesh.count);
    }

    /// Binds and draws everything.
    pub fn draw(&self, gpu: &mut Gpu) {
        self.bind(gpu);
        gpu.draw_elements_range(Primitive::Triangles, &self.indices, 0, self.indices.len() as u32);
    }
}
