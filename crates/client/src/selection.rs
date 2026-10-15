//! The block under the crosshair: its black outline and the cracks drawn
//! over it while it's being broken.

use glam::{IVec3, Vec3};
use rscraft_world::Face;
use wgpu::util::DeviceExt;

use crate::mesh::MeshData;

/// Outlines and cracks sit this far outside the block to avoid z-fighting.
const INFLATE: f32 = 0.002;

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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("outline pipeline layout"),
            bind_group_layouts: &[Some(globals_layout)],
            ..Default::default()
        });
        let outline_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("outline pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<[f32; 3]>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let outline_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("outline vertices"),
            size: (24 * size_of::<[f32; 3]>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { outline_pipeline, outline_buffer, has_outline: false, crack: None }
    }

    /// Uploads this frame's outline and cracks; call before the render pass.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, selection: Option<&Selection>) {
        self.has_outline = selection.is_some();
        self.crack = None;
        let Some(selection) = selection else { return };

        // The twelve edges of the block as a line list.
        let min = selection.block.as_vec3() - INFLATE;
        let max = min + Vec3::splat(1.0 + 2.0 * INFLATE);
        let corner = |i: u32| Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        );
        const EDGES: [(u32, u32); 12] =
            [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
        let lines: Vec<[f32; 3]> = EDGES.iter().flat_map(|&(a, b)| [corner(a).to_array(), corner(b).to_array()]).collect();
        queue.write_buffer(&self.outline_buffer, 0, bytemuck::cast_slice(&lines));

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
            pass.set_vertex_buffer(0, self.outline_buffer.slice(..));
            pass.draw(0..24, 0..1);
        }
    }
}
