use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Mat4, Vec3};
use rscraft_world::{Block, ColumnGroup, Face, LightRegion, MAX_LIGHT, RenderLayer, SECTION_SIZE, Section};

use crate::texture::BlockLayers;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub normal: [f32; 3],
    /// Layer in the block texture array.
    pub layer: u32,
    /// Ambient occlusion brightness, 0..1.
    pub ao: f32,
    /// Smoothed sky light level, 0..1 (15 = 1).
    pub sky: f32,
}

impl Vertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Uint32, 4 => Float32, 5 => Float32
    ];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// Height of a still water surface, like vanilla (8/9 of a block).
const WATER_SURFACE: f32 = 8.0 / 9.0;

/// Brightness for 0..=3 unoccluded neighbours around a vertex.
const AO_CURVE: [f32; 4] = [0.45, 0.65, 0.82, 1.0];

/// Corners of a unit cube face, counter-clockwise when seen from outside,
/// starting at the bottom-left of the texture.
pub fn corners(face: Face) -> [[f32; 3]; 4] {
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

    /// Appends one block face at world block position `pos`, with an
    /// ambient occlusion level (0..=3) and sky light (0..1) per corner.
    /// `height` lowers the top of the block (water surfaces); 1.0 is a full block.
    pub fn push_face(&mut self, pos: IVec3, face: Face, layer: u32, ao: [u8; 4], sky: [f32; 4], height: f32) {
        const UVS: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

        let base = self.vertices.len() as u32;
        let normal = face.offset().map(|c| c as f32);
        let origin = pos.as_vec3();
        for (((mut corner, mut uv), ao), sky) in corners(face).into_iter().zip(UVS).zip(ao).zip(sky) {
            if corner[1] > 0.5 {
                corner[1] = height;
                // Crop side textures instead of squashing them.
                if face != Face::Up && face != Face::Down {
                    uv[1] = 1.0 - height;
                }
            }
            self.vertices.push(Vertex {
                position: (origin + glam::Vec3::from(corner)).to_array(),
                uv,
                normal,
                layer,
                ao: AO_CURVE[ao as usize],
                sky,
            });
        }
        // Split the quad along the diagonal that keeps darkness in the
        // corners; otherwise AO shows an ugly anisotropic streak.
        if ao[0] as u32 + ao[2] as u32 >= ao[1] as u32 + ao[3] as u32 {
            self.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        } else {
            self.indices.extend_from_slice(&[base + 1, base + 2, base + 3, base + 1, base + 3, base]);
        }
    }
}

impl MeshData {
    /// Appends a whole block as a cube: `transform` places the unit cube
    /// 0..1, and the block is lit evenly with sky light `light` (0..1).
    pub fn push_block(&mut self, block: Block, layers: &BlockLayers, transform: Mat4, light: f32) {
        let start = self.vertices.len();
        for face in Face::ALL {
            self.push_face(IVec3::ZERO, face, layers.get(block, face), [3; 4], [light; 4], 1.0);
        }
        for v in &mut self.vertices[start..] {
            v.position = transform.transform_point3(Vec3::from(v.position)).to_array();
            v.normal = transform.transform_vector3(Vec3::from(v.normal)).normalize().to_array();
        }
    }
}

/// Meshes of one section, split by render pass.
#[derive(Default)]
pub struct SectionMesh {
    pub solid: MeshData,
    pub translucent: MeshData,
}

/// A section plus all 26 surrounding sections, enough for face culling and
/// ambient occlusion. Missing ones (above/below the world) count as air.
pub struct Neighborhood {
    /// Indexed by `(dx + 1) + (dy + 1) * 3 + (dz + 1) * 9`.
    pub sections: [Option<Arc<Section>>; 27],
}

impl Neighborhood {
    pub fn index(offset: IVec3) -> usize {
        (offset.x + 1 + (offset.y + 1) * 3 + (offset.z + 1) * 9) as usize
    }

    /// The neighbourhood of section number `index` (bottom = 0) of the centre column.
    pub fn from_columns(columns: &ColumnGroup, index: usize) -> Self {
        let mut sections: [Option<Arc<Section>>; 27] = Default::default();
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let column = &columns[((dx + 1) + (dz + 1) * 3) as usize];
                    let y = index as i32 + dy;
                    if y >= 0 {
                        sections[Self::index(IVec3::new(dx, dy, dz))] = column.get(y as usize).cloned();
                    }
                }
            }
        }
        Self { sections }
    }

    /// Local coordinates, -16..32 on every axis.
    #[inline]
    fn get(&self, x: i32, y: i32, z: i32) -> Block {
        let section = IVec3::new(x, y, z).div_euclid(IVec3::splat(SECTION_SIZE));
        let wrap = |v: i32| v.rem_euclid(SECTION_SIZE);
        self.sections[Self::index(section)].as_ref().map_or(Block::Air, |s| s.get(wrap(x), wrap(y), wrap(z)))
    }

    fn occludes(&self, p: IVec3) -> bool {
        self.get(p.x, p.y, p.z).is_opaque()
    }

    /// Ambient occlusion (0..=3) and smooth sky light (0..1) for the four
    /// corners of `face` of the block at local `pos`. Like vanilla smooth
    /// lighting, each corner looks at the four cells in front of it: straight
    /// ahead, the two sides and the diagonal.
    fn face_lighting(&self, pos: IVec3, face: Face, light: &LightRegion, section_y: i32) -> ([u8; 4], [f32; 4]) {
        let normal = IVec3::from(face.offset());
        let axes: Vec<usize> = (0..3).filter(|&i| normal[i] == 0).collect();
        let sky_at = |p: IVec3| light.get(p.x, section_y + p.y, p.z) as f32;
        let mut ao = [0; 4];
        let mut sky = [0.0; 4];
        for (i, corner) in corners(face).into_iter().enumerate() {
            let step = |axis: usize| {
                let mut v = IVec3::ZERO;
                v[axis] = if corner[axis] > 0.5 { 1 } else { -1 };
                v
            };
            let front = pos + normal;
            let (p1, p2) = (front + step(axes[0]), front + step(axes[1]));
            let diagonal_pos = p1 + step(axes[1]);
            let side1 = self.occludes(p1);
            let side2 = self.occludes(p2);
            let diagonal = self.occludes(diagonal_pos);
            ao[i] = if side1 && side2 { 0 } else { 3 - side1 as u8 - side2 as u8 - diagonal as u8 };

            // Average the light of the cells that light can actually be in.
            let mut total = sky_at(front);
            let mut count = 1.0;
            for (open, p) in [(!side1, p1), (!side2, p2), (!diagonal && !(side1 && side2), diagonal_pos)] {
                if open {
                    total += sky_at(p);
                    count += 1.0;
                }
            }
            sky[i] = total / count / MAX_LIGHT as f32;
        }
        (ao, sky)
    }

    /// Builds the mesh of the centre section located at section coords `pos`,
    /// emitting only faces not hidden by an adjacent opaque block.
    pub fn build_mesh(&self, pos: IVec3, layers: &BlockLayers, light: &LightRegion) -> SectionMesh {
        let mut mesh = SectionMesh::default();
        let Some(center) = &self.sections[13] else { return mesh };
        if center.is_empty() {
            return mesh;
        }
        let origin = pos * SECTION_SIZE;
        for y in 0..SECTION_SIZE {
            for z in 0..SECTION_SIZE {
                for x in 0..SECTION_SIZE {
                    let block = center.get(x, y, z);
                    let Some(layer) = block.render_layer() else { continue };
                    let local = IVec3::new(x, y, z);
                    for face in Face::ALL {
                        let [dx, dy, dz] = face.offset();
                        if !block.face_visible(self.get(x + dx, y + dy, z + dz)) {
                            continue;
                        }
                        let texture = layers.get(block, face);
                        let (ao, sky) = self.face_lighting(local, face, light, origin.y);
                        match layer {
                            RenderLayer::Solid => mesh.solid.push_face(origin + local, face, texture, ao, sky, 1.0),
                            // No AO on water, it looks wrong on a flat surface.
                            RenderLayer::Translucent => {
                                let height = if self.get(x, y + 1, z) == Block::Water { 1.0 } else { WATER_SURFACE };
                                mesh.translucent.push_face(origin + local, face, texture, [3; 4], sky, height)
                            }
                        }
                    }
                }
            }
        }
        mesh
    }
}
