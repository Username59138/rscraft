//! Small helpers for the wgpu boilerplate shared by every renderer.

/// Depth testing for a pipeline.
#[derive(Clone, Copy)]
pub struct Depth {
    pub format: wgpu::TextureFormat,
    pub write: bool,
    pub compare: wgpu::CompareFunction,
}

impl Depth {
    /// Tested and written, like opaque geometry.
    pub fn opaque(format: wgpu::TextureFormat) -> Self {
        Self { format, write: true, compare: wgpu::CompareFunction::Less }
    }

    /// Tested only, like translucent geometry and overlays.
    pub fn read_only(format: wgpu::TextureFormat, compare: wgpu::CompareFunction) -> Self {
        Self { format, write: false, compare }
    }
}

/// A render pipeline with one colour target; everything not set here uses
/// wgpu's defaults (triangle lists, counter-clockwise front faces).
pub struct PipelineDesc<'a> {
    pub label: &'a str,
    pub layout: &'a wgpu::PipelineLayout,
    pub shader: &'a wgpu::ShaderModule,
    pub vertex: &'a str,
    pub fragment: &'a str,
    pub buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    pub format: wgpu::TextureFormat,
    pub blend: wgpu::BlendState,
    pub write_mask: wgpu::ColorWrites,
    pub cull: Option<wgpu::Face>,
    pub depth: Option<Depth>,
}

impl<'a> PipelineDesc<'a> {
    /// Alpha blended, culling back faces, no depth, no vertex buffers.
    pub fn new(
        label: &'a str,
        layout: &'a wgpu::PipelineLayout,
        shader: &'a wgpu::ShaderModule,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            label,
            layout,
            shader,
            vertex: "vs_main",
            fragment: "fs_main",
            buffers: &[],
            format,
            blend: wgpu::BlendState::ALPHA_BLENDING,
            write_mask: wgpu::ColorWrites::ALL,
            cull: Some(wgpu::Face::Back),
            depth: None,
        }
    }

    pub fn build(&self, device: &wgpu::Device) -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(self.label),
            layout: Some(self.layout),
            vertex: wgpu::VertexState {
                module: self.shader,
                entry_point: Some(self.vertex),
                buffers: self.buffers,
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: self.shader,
                entry_point: Some(self.fragment),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.format,
                    blend: Some(self.blend),
                    write_mask: self.write_mask,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { cull_mode: self.cull, ..Default::default() },
            depth_stencil: self.depth.map(|depth| wgpu::DepthStencilState {
                format: depth.format,
                depth_write_enabled: Some(depth.write),
                depth_compare: Some(depth.compare),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        })
    }
}

pub fn pipeline_layout(
    device: &wgpu::Device,
    label: &str,
    bind_groups: &[&wgpu::BindGroupLayout],
) -> wgpu::PipelineLayout {
    let layouts: Vec<Option<&wgpu::BindGroupLayout>> = bind_groups.iter().copied().map(Some).collect();
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &layouts,
        ..Default::default()
    })
}

pub fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// A filterable float texture read by the fragment shader.
pub fn texture_entry(binding: u32, dimension: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}

pub fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// A texture at binding 0 and its sampler at binding 1: the layout and a
/// bind group for them.
pub fn texture_binding(
    device: &wgpu::Device,
    label: &str,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    dimension: wgpu::TextureViewDimension,
) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[texture_entry(0, dimension), sampler_entry(1)],
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });
    (layout, group)
}

/// Uploads an RGBA8 sRGB image as a 2D texture with one mip level.
pub fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> wgpu::Texture {
    let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
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
        pixels,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * width), rows_per_image: None },
        size,
    );
    texture
}
