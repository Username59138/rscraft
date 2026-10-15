use bytemuck::{Pod, Zeroable};

use super::{GuiAtlas, UiVertex};
use crate::gpu::{self, PipelineDesc};
use crate::renderer::GuiFrame;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ScreenUniform {
    /// GUI size in units (xy); z = 1 when the target is a non-sRGB view.
    size: [f32; 4],
}

/// Draws the GUI quads over the finished 3D frame.
pub struct UiRenderer {
    pipeline: wgpu::RenderPipeline,
    invert_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    capacity: usize,
    format: wgpu::TextureFormat,
}

impl UiRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, atlas: &GuiAtlas, format: wgpu::TextureFormat) -> Self {
        let image = &atlas.image;
        let size = wgpu::Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gui atlas"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &image.pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * image.width), rows_per_image: None },
            size,
        );
        let view = texture.create_view(&Default::default());
        let sampler =
            device.create_sampler(&wgpu::SamplerDescriptor { label: Some("gui sampler"), ..Default::default() });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gui screen"),
            size: size_of::<ScreenUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gui layout"),
            entries: &[
                gpu::uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                gpu::texture_entry(1, wgpu::TextureViewDimension::D2),
                gpu::sampler_entry(2),
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gui bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/ui.wgsl"));
        let pipeline_layout = gpu::pipeline_layout(device, "gui pipeline layout", &[&layout]);
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<UiVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
        })];
        let pipeline = |label, blend| {
            PipelineDesc {
                buffers: &buffers,
                blend,
                cull: None,
                ..PipelineDesc::new(label, &pipeline_layout, &shader, format)
            }
            .build(device)
        };

        // The crosshair inverts what's behind it, like vanilla's
        // ONE_MINUS_DST_COLOR, ONE_MINUS_SRC_COLOR blending.
        let invert = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::OneMinusDst,
                dst_factor: wgpu::BlendFactor::OneMinusSrc,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };

        let capacity = 4096;
        let vertex_buffer = Self::create_buffer(device, capacity);
        Self {
            pipeline: pipeline("gui pipeline", wgpu::BlendState::ALPHA_BLENDING),
            invert_pipeline: pipeline("gui invert pipeline", invert),
            bind_group,
            uniform,
            vertex_buffer,
            capacity,
            format,
        }
    }

    /// The view format the GUI must be drawn into.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    fn create_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gui vertices"),
            size: (capacity * size_of::<UiVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        gui: &GuiFrame<'_>,
    ) {
        let (vertices, inverted, gui_size) = (gui.vertices, gui.inverted.clone(), gui.size);
        if vertices.is_empty() {
            return;
        }
        if vertices.len() > self.capacity {
            self.capacity = vertices.len().next_power_of_two();
            self.vertex_buffer = Self::create_buffer(device, self.capacity);
        }
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(vertices));
        let gamma_target = if self.format.is_srgb() { 0.0 } else { 1.0 };
        let screen = ScreenUniform { size: [gui_size[0], gui_size[1], gamma_target, 0.0] };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&screen));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gui pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        // Everything in order, switching blending for the inverted range.
        let (start, end, len) = (inverted.start as u32, inverted.end as u32, vertices.len() as u32);
        for (pipeline, range) in
            [(&self.pipeline, 0..start), (&self.invert_pipeline, start..end), (&self.pipeline, end..len)]
        {
            if !range.is_empty() {
                pass.set_pipeline(pipeline);
                pass.draw(range, 0..1);
            }
        }
    }
}
