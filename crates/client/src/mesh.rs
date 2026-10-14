use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::IVec3;
use rscraft_world::{Block, Face, RenderLayer, SECTION_SIZE, Section};

use crate::texture::BlockLayers;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub normal: [f32; 3],
    /// Layer in the block texture array.
    pub layer: u32,
}

impl Vertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Uint32];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// Corners of a unit cube face, counter-clockwise when seen from outside,
/// starting at the bottom-left of the texture.
fn corners(face: Face) -> [[f32; 3]; 4] {
    match face {
        Face::East => [[1., 0., 1.], [1., 0., 0.], [1., 1., 0.], [1., 1., 1.]],
        Face::West => [[0., 0., 0.], [0., 0., 1.], [0., 1., 1.], [0., 1., 0.]],
        Face::Up => [[0., 1., 1.], [1., 1., 1.], [1., 1., 0.], [0., 1., 0.]],
        Face::Down => [[0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.]],
        Face::South => [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]],
        Face::North => [[1., 0., 0.], [0., 0., 0.], [0., 1., 0.], [1., 1., 0.]],
    }
}

/// CPU-side mesh, ready to be uploaded to the GPU.
#[derive(Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Appends one block face at world block position `pos`.
    pub fn push_face(&mut self, pos: IVec3, face: Face, layer: u32) {
        const UVS: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

        let base = self.vertices.len() as u32;
        let normal = face.offset().map(|c| c as f32);
        let origin = pos.as_vec3();
        for (corner, uv) in corners(face).into_iter().zip(UVS) {
            self.vertices.push(Vertex {
                position: (origin + glam::Vec3::from(corner)).to_array(),
                uv,
                normal,
                layer,
            });
        }
        self.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Meshes of one section, split by render pass.
#[derive(Default)]
pub struct SectionMesh {
    pub solid: MeshData,
    pub translucent: MeshData,
}

/// A section plus its six face neighbours, enough to decide face visibility.
/// Missing neighbours (above/below the world) count as air.
pub struct Neighborhood {
    pub center: Arc<Section>,
    pub neighbors: [Option<Arc<Section>>; 6],
}

impl Neighborhood {
    /// Local coordinates in -1..=16; at most one axis may be out of range.
    fn get(&self, x: i32, y: i32, z: i32) -> Block {
        let inside = |v: i32| (0..SECTION_SIZE).contains(&v);
        if inside(x) && inside(y) && inside(z) {
            return self.center.get(x, y, z);
        }
        let face = match (x, y, z) {
            (16, ..) => Face::East,
            (-1, ..) => Face::West,
            (_, 16, _) => Face::Up,
            (_, -1, _) => Face::Down,
            (.., 16) => Face::South,
            _ => Face::North,
        };
        let wrap = |v: i32| v.rem_euclid(SECTION_SIZE);
        self.neighbors[face as usize].as_ref().map_or(Block::Air, |s| s.get(wrap(x), wrap(y), wrap(z)))
    }

    /// Builds the mesh of the centre section located at section coords `pos`,
    /// emitting only faces not hidden by an adjacent opaque block.
    pub fn build_mesh(&self, pos: IVec3, layers: &BlockLayers) -> SectionMesh {
        let mut mesh = SectionMesh::default();
        if self.center.is_empty() {
            return mesh;
        }
        let origin = pos * SECTION_SIZE;
        for y in 0..SECTION_SIZE {
            for z in 0..SECTION_SIZE {
                for x in 0..SECTION_SIZE {
                    let block = self.center.get(x, y, z);
                    let Some(layer) = block.render_layer() else { continue };
                    let target = match layer {
                        RenderLayer::Solid => &mut mesh.solid,
                        RenderLayer::Translucent => &mut mesh.translucent,
                    };
                    for face in Face::ALL {
                        let [dx, dy, dz] = face.offset();
                        if block.face_visible(self.get(x + dx, y + dy, z + dz)) {
                            target.push_face(origin + IVec3::new(x, y, z), face, layers.get(block, face));
                        }
                    }
                }
            }
        }
        mesh
    }
}
