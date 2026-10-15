//! The block under the crosshair: its black outline and the cracks drawn
//! over it while it's being broken.

use glam::{IVec3, Vec3};
use rscraft_world::Face;
use wgpu::util::DeviceExt;

use crate::gpu::{self, Depth, PipelineDesc};
use crate::mesh::MeshData;

/// Outlines and cracks sit this far outside the block to avoid z-fighting.
const INFLATE: f32 = 0.002;
/// Thickness of the block outline in pixels at 1080p; scaled with the
/// window height so it looks the same at any resolution.
const OUTLINE_WIDTH: f32 = 3.0;
/// Twelve edges, each a quad of two triangles.
const OUTLINE_VERTICES: u32 = 12 * 6;

/// One corner of an outline edge quad: the vertex shader pushes it
/// sideways (and past the end) in screen space to give the line a width.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OutlineVertex {
    position: [f32; 3],
    /// The edge's other end.
    other: [f32; 3],
    /// Which side of the edge, -1 or 1.
    side: f32,
}

/// Mirrors `Outline` in outline.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OutlineUniform {
    /// Screen size in pixels (xy) and line width (z).
    screen: [f32; 4],
}

pub struct Selection {
    pub block: IVec3,
    /// Texture layer of the current crack stage, while breaking.
    pub crack_layer: Option<u32>,
}

struct CrackMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

pub struct SelectionRenderer {
    outline_pipeline: wgpu::RenderPipeline,
    outline_buffer: wgpu::Buffer,
    outline_uniform: wgpu::Buffer,
    outline_bind_group: wgpu::BindGroup,
    has_outline: bool,
    crack: Option<CrackMesh>,
}

impl SelectionRenderer {
    pub fn new(
        device: &wgpu::Device,
        globals_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = crate::renderer::shader_module(device, "outline shader", include_str!("outline.wgsl"));
        let outline_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("outline uniform"),
            size: size_of::<OutlineUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("outline layout"),
            entries: &[gpu::uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let outline_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("outline bind group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: outline_uniform.as_entire_binding() }],
        });
        let layout = gpu::pipeline_layout(device, "outline pipeline layout", &[globals_layout, &uniform_layout]);
        let outline_pipeline = PipelineDesc {
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<OutlineVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32],
            })],
            // The edge quads face either way.
            cull: None,
            depth: Some(Depth::read_only(depth_format, wgpu::CompareFunction::LessEqual)),
            ..PipelineDesc::new("outline pipeline", &layout, &shader, color_format)
        }
        .build(device);
        let outline_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("outline vertices"),
            size: (OUTLINE_VERTICES as usize * size_of::<OutlineVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { outline_pipeline, outline_buffer, outline_uniform, outline_bind_group, has_outline: false, crack: None }
    }

    /// Uploads this frame's outline and cracks; call before the render pass.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        selection: Option<&Selection>,
        screen: (u32, u32),
    ) {
        self.has_outline = selection.is_some();
        self.crack = None;
        let Some(selection) = selection else { return };

        let width = OUTLINE_WIDTH * (screen.1 as f32 / 1080.0).max(1.0);
        let uniform = OutlineUniform { screen: [screen.0 as f32, screen.1 as f32, width, 0.0] };
        queue.write_buffer(&self.outline_uniform, 0, bytemuck::bytes_of(&uniform));

        // The twelve edges of the block, each as a quad.
        let min = selection.block.as_vec3() - INFLATE;
        let max = min + Vec3::splat(1.0 + 2.0 * INFLATE);
        let corner = |i: u32| {
            Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            )
        };
        const EDGES: [(u32, u32); 12] =
            [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
        let vertices: Vec<OutlineVertex> = EDGES
            .iter()
            .flat_map(|&(a, b)| {
                let (a, b) = (corner(a).to_array(), corner(b).to_array());
                let v = |position, other, side| OutlineVertex { position, other, side };
                // Seen from the other end the sides swap, hence the flipped signs.
                let (a_left, a_right, b_left, b_right) = (v(a, b, 1.0), v(a, b, -1.0), v(b, a, -1.0), v(b, a, 1.0));
                [a_left, a_right, b_right, a_left, b_right, b_left]
            })
            .collect();
        queue.write_buffer(&self.outline_buffer, 0, bytemuck::cast_slice(&vertices));

        if let Some(layer) = selection.crack_layer {
            let mut mesh = MeshData::default();
            for face in Face::ALL {
                mesh.push_face(selection.block, face, layer, [3; 4], [1.0; 4], 1.0);
            }
            let center = selection.block.as_vec3() + 0.5;
            for v in &mut mesh.vertices {
                let p = Vec3::from(v.position);
                v.position = (center + (p - center) * (1.0 + 2.0 * INFLATE)).to_array();
            }
            self.crack = Some(CrackMesh {
                vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("crack vertices"),
                    contents: bytemuck::cast_slice(&mesh.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("crack indices"),
                    contents: bytemuck::cast_slice(&mesh.indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                index_count: mesh.indices.len() as u32,
            });
        }
    }

    /// Draws the cracks; expects the crack pipeline to be bound.
    pub fn draw_crack(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(crack) = &self.crack {
            pass.set_vertex_buffer(0, crack.vertices.slice(..));
            pass.set_index_buffer(crack.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..crack.index_count, 0, 0..1);
        }
    }

    pub fn draw_outline(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.has_outline {
            pass.set_pipeline(&self.outline_pipeline);
            pass.set_bind_group(1, &self.outline_bind_group, &[]);
            pass.set_vertex_buffer(0, self.outline_buffer.slice(..));
            pass.draw(0..OUTLINE_VERTICES, 0..1);
        }
    }
}
