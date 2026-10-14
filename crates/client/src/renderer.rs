use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{IVec2, IVec3, Vec3};
use rscraft_world::SECTION_SIZE;
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

use crate::camera::Camera;
use crate::frustum::Frustum;
use crate::mesh::{MeshData, SectionMesh, Vertex};
use crate::sky::{self, SkyImages, SkyRenderer, SkyState};
use crate::texture::{BlockImages, BlockTextures};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Mirrors `Globals` in common.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    sun_dir: [f32; 4],
    zenith: [f32; 4],
    horizon: [f32; 4],
    sunset: [f32; 4],
    params: [f32; 4],
}

/// Compiles a WGSL shader with common.wgsl prepended.
pub fn shader_module(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(format!("{}\n{source}", include_str!("common.wgsl")).into()),
    })
}

/// A mesh living on the GPU.
struct GpuMesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

impl GpuMesh {
    fn upload(device: &wgpu::Device, mesh: &MeshData) -> Option<Self> {
        if mesh.is_empty() {
            return None;
        }
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("section vertices"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("section indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Some(Self { vertex_buffer, index_buffer, index_count: mesh.indices.len() as u32 })
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

struct GpuSection {
    solid: Option<GpuMesh>,
    translucent: Option<GpuMesh>,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
    solid_pipeline: wgpu::RenderPipeline,
    translucent_pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    texture_bind_group: wgpu::BindGroup,
    block_textures: BlockTextures,
    sky: SkyRenderer,
    sections: HashMap<IVec3, GpuSection>,
    fog_end: f32,
    /// Sections drawn in the last frame, for the debug title.
    pub drawn_sections: usize,
}

impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        display: OwnedDisplayHandle,
        images: &BlockImages,
        sky_images: &SkyImages,
        render_distance: i32,
    ) -> Self {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(display)));
        let surface = instance.create_surface(window).expect("failed to create surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .expect("no suitable GPU adapter");
        let info = adapter.get_info();
        log::info!("GPU: {} ({:?})", info.name, info.backend);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("rscraft device"),
                // Downlevel limits keep the door open for old GPUs and WebGL2.
                required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .expect("failed to create device");

        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        let depth_view = create_depth_view(&device, &config);

        // Per-frame globals: camera and fog.
        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals bind group"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buffer.as_entire_binding() }],
        });

        // Block textures.
        let block_textures = BlockTextures::new(&device, &queue, images);
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("block texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
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
        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("block texture bind group"),
            layout: &texture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&block_textures.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&block_textures.sampler) },
            ],
        });

        // Pipelines.
        let shader = shader_module(&device, "block shader", include_str!("blocks.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("block pipeline layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(&texture_layout)],
            ..Default::default()
        });
        let pipeline = |translucent: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if translucent { "translucent pipeline" } else { "solid pipeline" }),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Some(Vertex::layout())],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if translucent { "fs_translucent" } else { "fs_solid" }),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format,
                        blend: Some(if translucent { wgpu::BlendState::ALPHA_BLENDING } else { wgpu::BlendState::REPLACE }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    // Water must be visible from below too.
                    cull_mode: if translucent { None } else { Some(wgpu::Face::Back) },
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(!translucent),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let solid_pipeline = pipeline(false);
        let translucent_pipeline = pipeline(true);
        let sky = SkyRenderer::new(&device, &queue, sky_images, &globals_layout, config.format, DEPTH_FORMAT);

        Self {
            surface,
            device,
            queue,
            config,
            depth_view,
            solid_pipeline,
            translucent_pipeline,
            globals_buffer,
            globals_bind_group,
            texture_bind_group,
            block_textures,
            sky,
            sections: HashMap::new(),
            fog_end: ((render_distance - 1) * SECTION_SIZE) as f32,
            drawn_sections: 0,
        }
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_view(&self.device, &self.config);
    }

    pub fn upload_section(&mut self, pos: IVec3, mesh: &SectionMesh) {
        let section = GpuSection {
            solid: GpuMesh::upload(&self.device, &mesh.solid),
            translucent: GpuMesh::upload(&self.device, &mesh.translucent),
        };
        if section.solid.is_some() || section.translucent.is_some() {
            self.sections.insert(pos, section);
        } else {
            self.sections.remove(&pos);
        }
    }

    pub fn remove_column(&mut self, column: IVec2) {
        self.sections.retain(|pos, _| pos.x != column.x || pos.z != column.y);
    }

    pub fn render(&mut self, camera: &Camera, sky: &SkyState, seconds: f32) {
        self.block_textures.animate(&self.queue, seconds);

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            other => {
                log::warn!("skipping frame: {other:?}");
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());

        let view_proj = camera.view_proj();
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            camera_pos: camera.position.extend(seconds).to_array(),
            sun_dir: sky.sun_dir.extend(sky.daylight).to_array(),
            zenith: sky.zenith.extend(sky.ambient).to_array(),
            horizon: sky.horizon.extend(0.0).to_array(),
            sunset: sky.sunset_color.extend(sky.sunset).to_array(),
            params: [self.fog_end * 0.6, self.fog_end, sky::CLOUD_HEIGHT, self.fog_end * 2.0],
        };
        self.queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));

        let frustum = Frustum::from_view_proj(view_proj);
        let size = SECTION_SIZE as f32;
        let visible: Vec<&GpuSection> = self
            .sections
            .iter()
            .filter(|(pos, _)| {
                let min = pos.as_vec3() * size;
                frustum.intersects_aabb(min, min + Vec3::splat(size))
            })
            .map(|(_, s)| s)
            .collect();
        self.drawn_sections = visible.len();

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // The sky pass covers every pixel.
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.globals_bind_group, &[]);
            self.sky.draw_sky(&mut pass);

            pass.set_bind_group(1, &self.texture_bind_group, &[]);
            pass.set_pipeline(&self.solid_pipeline);
            for mesh in visible.iter().filter_map(|s| s.solid.as_ref()) {
                mesh.draw(&mut pass);
            }
            self.sky.draw_clouds(&mut pass);

            // Translucent geometry goes last, over the finished opaque scene.
            pass.set_bind_group(1, &self.texture_bind_group, &[]);
            pass.set_pipeline(&self.translucent_pipeline);
            for mesh in visible.iter().filter_map(|s| s.translucent.as_ref()) {
                mesh.draw(&mut pass);
            }
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }
}

fn create_depth_view(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d { width: config.width, height: config.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}
