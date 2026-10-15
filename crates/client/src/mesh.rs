use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Mat4, Vec3};
use rscraft_world::{Block, ColumnGroup, Face, LightRegion, MAX_LIGHT, RenderLayer, SECTION_SIZE, Section, fluid};

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

/// Compact vertex of a terrain section, 16 bytes. Positions are relative to
/// the section's origin, which comes from a per-instance attribute.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TerrainVertex {
    /// x, y, z in 1/`POSITION_SCALE` blocks, then the texture layer.
    pub position: [u16; 4],
    /// Texture coordinates, 0..1 as 0..65535.
    pub uv: [u16; 2],
    /// Face shade, ambient occlusion and sky light, 0..1 as 0..255.
    pub light: [u8; 4],
}

/// Sub-block steps of a terrain vertex position; must match the shader.
pub const POSITION_SCALE: f32 = 2048.0;

impl TerrainVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Uint16x4, 1 => Unorm16x2, 2 => Unorm8x4];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<TerrainVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// Heights of the four top corners of a block, indexed `[x][z]` (0 = west
/// or north, 1 = east or south).
type CornerHeights = [[f32; 2]; 2];
const FULL_HEIGHT: CornerHeights = [[1.0; 2]; 2];

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

/// Corners and texture coordinates of `face`, with the texture turned by
/// `turn` corners (see `Block::texture_turn`) and the top corners lowered
/// to `heights` (water surfaces).
fn face_vertices(face: Face, turn: usize, heights: CornerHeights) -> [([f32; 3], [f32; 2]); 4] {
    const UVS: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    let mut out = [([0.0; 3], [0.0; 2]); 4];
    for (i, mut corner) in corners(face).into_iter().enumerate() {
        let mut uv = UVS[(i + turn) % 4];
        if corner[1] > 0.5 {
            let height = heights[corner[0] as usize][corner[2] as usize];
            corner[1] = height;
            // Crop side textures instead of squashing them.
            if face != Face::Up && face != Face::Down {
                uv[1] = 1.0 - height;
            }
        }
        out[i] = (corner, uv);
    }
    out
}

/// Whether a quad must be split along its other diagonal: the one that keeps
/// darkness in the corners; otherwise AO shows an ugly anisotropic streak.
fn flip_quad(ao: [u8; 4]) -> bool {
    (ao[0] as u32 + ao[2] as u32) < (ao[1] as u32 + ao[3] as u32)
}

/// Minecraft-style fixed face shading: top bright, bottom dark, sides in between.
fn face_shade(face: Face) -> f32 {
    match face {
        Face::Up => 1.0,
        Face::Down => 0.5,
        Face::East | Face::West => 0.6,
        Face::South | Face::North => 0.8,
    }
}

/// Which texture a face shows and how it is turned.
#[derive(Clone, Copy)]
struct FaceTexturing {
    /// Layer in the block texture array.
    layer: u32,
    /// Corners the texture is turned by, see `Block::texture_turn`.
    turn: usize,
}

/// Appends one block face at section-local block position `pos` as four
/// vertices; sections are drawn with a shared index buffer of quads.
fn push_terrain_face(
    out: &mut Vec<TerrainVertex>,
    pos: IVec3,
    face: Face,
    texture: FaceTexturing,
    ao: [u8; 4],
    sky: [f32; 4],
    heights: CornerHeights,
) {
    let unorm8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let shade = unorm8(face_shade(face));
    let mut quad = [TerrainVertex::zeroed(); 4];
    for (i, (corner, uv)) in face_vertices(face, texture.turn, heights).into_iter().enumerate() {
        let p = (pos.as_vec3() + Vec3::from(corner)) * POSITION_SCALE;
        quad[i] = TerrainVertex {
            position: [p.x as u16, p.y as u16, p.z as u16, texture.layer as u16],
            uv: uv.map(|c| (c * 65535.0).round() as u16),
            light: [shade, unorm8(AO_CURVE[ao[i] as usize]), unorm8(sky[i]), 0],
        };
    }
    // The shared indices split quads along 0-2; rotating the corners by one
    // moves the split to the other diagonal.
    if flip_quad(ao) {
        quad.rotate_left(1);
    }
    out.extend_from_slice(&quad);
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
        let base = self.vertices.len() as u32;
        let normal = face.offset().map(|c| c as f32);
        let origin = pos.as_vec3();
        for (((corner, uv), ao), sky) in face_vertices(face, 0, [[height; 2]; 2]).into_iter().zip(ao).zip(sky) {
            self.vertices.push(Vertex {
                position: (origin + Vec3::from(corner)).to_array(),
                uv,
                normal,
                layer,
                ao: AO_CURVE[ao as usize],
                sky,
            });
        }
        if flip_quad(ao) {
            self.indices.extend_from_slice(&[base + 1, base + 2, base + 3, base + 1, base + 3, base]);
        } else {
            self.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
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
    /// Quads, four vertices each, positioned relative to the section origin.
    pub solid: Vec<TerrainVertex>,
    pub translucent: Vec<TerrainVertex>,
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

    /// Vanilla's fluid height of a block for smoothing water surfaces:
    /// 1 under more water, the water's own height, 0 for other open blocks
    /// and -1 for solid ones.
    fn water_height(&self, p: IVec3) -> f32 {
        let block = self.get(p.x, p.y, p.z);
        if block.is_water() {
            if self.get(p.x, p.y + 1, p.z).is_water() { 1.0 } else { fluid::own_height(block) }
        } else if block.is_solid() {
            -1.0
        } else {
            0.0
        }
    }

    /// Heights of the top corners of the water at `pos`: each averages the
    /// four blocks around it, with near-full blocks weighing ten times more,
    /// like vanilla's FluidRenderer, so flowing water forms slopes.
    fn water_corner_heights(&self, pos: IVec3) -> CornerHeights {
        let own = self.water_height(pos);
        if own >= 1.0 {
            return FULL_HEIGHT;
        }
        let mut heights = [[0.0; 2]; 2];
        for (xi, dx) in [(0, -1), (1, 1)] {
            for (zi, dz) in [(0, -1), (1, 1)] {
                let side_x = self.water_height(pos + IVec3::new(dx, 0, 0));
                let side_z = self.water_height(pos + IVec3::new(0, 0, dz));
                heights[xi][zi] = self.corner_height(own, side_x, side_z, pos + IVec3::new(dx, 0, dz));
            }
        }
        heights
    }

    fn corner_height(&self, own: f32, side_x: f32, side_z: f32, diagonal: IVec3) -> f32 {
        if side_x >= 1.0 || side_z >= 1.0 {
            return 1.0;
        }
        let (mut sum, mut weight) = (0.0, 0.0);
        let mut add = |h: f32| {
            let w = if h >= 0.8 {
                10.0
            } else if h >= 0.0 {
                1.0
            } else {
                0.0
            };
            sum += h * w;
            weight += w;
        };
        // The diagonal block only counts when water can reach it.
        if side_x > 0.0 || side_z > 0.0 {
            let corner = self.water_height(diagonal);
            if corner >= 1.0 {
                return 1.0;
            }
            add(corner);
        }
        add(own);
        add(side_x);
        add(side_z);
        sum / weight
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
        let origin_y = pos.y * SECTION_SIZE;
        for y in 0..SECTION_SIZE {
            for z in 0..SECTION_SIZE {
                for x in 0..SECTION_SIZE {
                    let block = center.get(x, y, z);
                    let Some(layer) = block.render_layer() else { continue };
                    let local = IVec3::new(x, y, z);
                    let heights = if block.is_water() { self.water_corner_heights(local) } else { FULL_HEIGHT };
                    for face in Face::ALL {
                        let [dx, dy, dz] = face.offset();
                        let neighbour = self.get(x + dx, y + dy, z + dz);
                        // A lowered water surface shows under a block too.
                        let lowered = face == Face::Up && heights.as_flattened().iter().any(|&h| h < 1.0);
                        if !block.face_visible(neighbour) && !(lowered && block.is_water() && !neighbour.is_water()) {
                            continue;
                        }
                        let texture = FaceTexturing { layer: layers.get(block, face), turn: block.texture_turn(face) };
                        let (ao, sky) = self.face_lighting(local, face, light, origin_y);
                        match layer {
                            RenderLayer::Solid => {
                                push_terrain_face(&mut mesh.solid, local, face, texture, ao, sky, FULL_HEIGHT)
                            }
                            // No AO on water, it looks wrong on a flat surface.
                            RenderLayer::Translucent => {
                                push_terrain_face(&mut mesh.translucent, local, face, texture, [3; 4], sky, heights)
                            }
                        }
                    }
                }
            }
        }
        mesh
    }
}
