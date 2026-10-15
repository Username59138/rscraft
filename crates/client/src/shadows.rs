//! Round entity shadows, like vanilla: the shadow texture projected onto the
//! tops of the blocks under an entity, fading with height above them.

use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Vec3};
use rscraft_assets::RgbaImage;
use rscraft_world::Block;
use wgpu::util::DeviceExt;

/// How far below an entity its shadow still shows.
const MAX_HEIGHT: f32 = 2.0;
/// Shadows sit this far above the block to avoid z-fighting.
const LIFT: f32 = 0.002;

/// An entity casting a shadow.
pub struct Caster {
    pub feet: Vec3,
    /// Vanilla's shadow radius: 0.5 for players, 0.15 for items.
    pub radius: f32,
    /// Vanilla's shadow strength: 1.0 for players, 0.75 for items.
    pub strength: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowVertex {
    position: [f32; 3],
    uv: [f32; 2],
    alpha: f32,
}

/// Builds the shadow quads for every caster.
fn build(casters: &[Caster], world: &impl Fn(IVec3) -> Option<Block>) -> Vec<ShadowVertex> {
    let mut out = Vec::new();
    for caster in casters {
        let (r, p) = (caster.radius, caster.feet);
        let (min, max) = (Vec3::new(p.x - r, 0.0, p.z - r), Vec3::new(p.x + r, 0.0, p.z + r));
        for bz in min.z.floor() as i32..=max.z.floor() as i32 {
            for bx in min.x.floor() as i32..=max.x.floor() as i32 {
                // The first solid block at or below the feet in this column.
                let Some(top) = (0..=MAX_HEIGHT as i32 + 1)
                    .map(|dy| p.y.floor() as i32 - dy)
                    .find(|&y| world(IVec3::new(bx, y, bz)).is_some_and(Block::is_solid))
                    .map(|y| y as f32 + 1.0)
                else {
                    continue;
                };
                let height = p.y - top;
                if height < -0.01 {
                    continue;
                }
                // Vanilla: fades out linearly over two blocks.
                let alpha = (1.0 - height / MAX_HEIGHT) * 0.5 * caster.strength;
                if alpha <= 0.0 {
                    continue;
                }
                // The part of this block's top inside the shadow square.
                let x0 = (bx as f32).max(min.x);
                let x1 = (bx as f32 + 1.0).min(max.x);
                let z0 = (bz as f32).max(min.z);
                let z1 = (bz as f32 + 1.0).min(max.z);
                let uv = |x: f32, z: f32| [(x - min.x) / (2.0 * r), (z - min.z) / (2.0 * r)];
                let v = |x, z| ShadowVertex { position: [x, top + LIFT, z], uv: uv(x, z), alpha };
                // Counter-clockwise seen from above.
                let (a, b, c, d) = (v(x0, z1), v(x1, z1), v(x1, z0), v(x0, z0));
                out.extend_from_slice(&[a, b, c, a, c, d]);
            }
        }
    }
    out
}

pub struct ShadowRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    buffer: Option<(wgpu::Buffer, u32)>,
}

impl ShadowRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &RgbaImage,
        globals_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let size = wgpu::Extent3d { width: texture.width, height: texture.height, depth_or_array_layers: 1 };
        let gpu_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            gpu_texture.as_image_copy(),
            &texture.pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * texture.width), rows_per_image: None },
            size,
        );
        let view = gpu_texture.create_view(&Default::default());
        let sampler =
            device.create_sampler(&wgpu::SamplerDescriptor { label: Some("shadow sampler"), ..Default::default() });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        let shader = crate::renderer::shader_module(device, "shadow shader", include_str!("shadow.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow pipeline layout"),
            bind_group_layouts: &[Some(globals_layout), Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<ShadowVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32],
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
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
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
        Self { pipeline, bind_group, buffer: None }
    }

    /// Rebuilds this frame's shadows; call before the render pass.
    pub fn prepare(&mut self, device: &wgpu::Device, casters: &[Caster], world: &impl Fn(IVec3) -> Option<Block>) {
        let vertices = build(casters, world);
        self.buffer = (!vertices.is_empty()).then(|| {
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shadow vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            (buffer, vertices.len() as u32)
        });
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some((buffer, count)) = &self.buffer else { return };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..*count, 0..1);
    }
}
