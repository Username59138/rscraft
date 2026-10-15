//! Round entity shadows, like vanilla: the shadow texture projected onto the
//! tops of the blocks under an entity, fading with height above them.

use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Vec3};
use rscraft_assets::RgbaImage;
use rscraft_world::Block;
use wgpu::util::DeviceExt;

use crate::gpu::{self, Depth, PipelineDesc};

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
        let view = gpu::upload_texture(device, queue, "shadow texture", texture.width, texture.height, &texture.pixels)
            .create_view(&Default::default());
        let sampler =
            device.create_sampler(&wgpu::SamplerDescriptor { label: Some("shadow sampler"), ..Default::default() });
        let (layout, bind_group) =
            gpu::texture_binding(device, "shadow texture", &view, &sampler, wgpu::TextureViewDimension::D2);

        let shader = crate::renderer::shader_module(device, "shadow shader", include_str!("shadow.wgsl"));
        let pipeline_layout = gpu::pipeline_layout(device, "shadow pipeline layout", &[globals_layout, &layout]);
        let pipeline = PipelineDesc {
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<ShadowVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32],
            })],
            depth: Some(Depth::read_only(depth_format, wgpu::CompareFunction::LessEqual)),
            ..PipelineDesc::new("shadow pipeline", &pipeline_layout, &shader, color_format)
        }
        .build(device);
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
