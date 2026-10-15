use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{IVec2, IVec3, Vec3};
use rscraft_assets::RgbaImage;
use rscraft_world::{Block, SECTION_SIZE};
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

mod preview;

pub use preview::{PREVIEW_DEPTH, Preview};

use crate::camera::Camera;
use crate::clouds::CloudRenderer;
use crate::frustum::Frustum;
use crate::gpu::{self, Depth, PipelineDesc};
use crate::mesh::{MeshData, SectionMesh, Vertex};
use crate::player::{EntityVertex, PlayerRenderer};
use crate::saves::Options;
use crate::selection::{Selection, SelectionRenderer};
use crate::shadows::{Caster, ShadowRenderer};
use crate::sky::{self, SkyImages, SkyRenderer, SkyState};
use crate::texture::{BlockImages, BlockTextures};
use crate::ui::render::UiRenderer;
use crate::ui::{GuiAtlas, UiVertex};

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
    water: [f32; 4],
    water_color: [f32; 4],
}

/// Everything drawn in one frame.
pub struct Frame<'a> {
    pub camera: &'a Camera,
    pub sky: &'a SkyState,
    /// Seconds since start, for animations.
    pub seconds: f32,
    /// The player's body (third person) and first-person hand.
    pub player: Option<&'a [EntityVertex]>,
    pub hand: Option<&'a [EntityVertex]>,
    pub selection: Option<Selection>,
    pub underwater: Option<Underwater>,
    /// Dropped and third-person held items, particles, and the
    /// first-person held block.
    pub items: &'a MeshData,
    pub particles: &'a MeshData,
    pub held: &'a MeshData,
    pub gui: GuiFrame<'a>,
    pub preview: Option<&'a Preview>,
}

/// The interface for one frame.
pub struct GuiFrame<'a> {
    pub vertices: &'a [UiVertex],
    /// Vertices drawn inverting what's behind them (the crosshair).
    pub inverted: std::ops::Range<usize>,
    /// Size in GUI units.
    pub size: [f32; 2],
}

/// What the view looks like with the camera under water.
pub struct Underwater {
    /// Distance where the water fog becomes opaque.
    pub fog_end: f32,
    /// Fog colour, sRGB.
    pub color: Vec3,
    /// Offset of the overlay texture, following the camera's rotation.
    pub overlay_offset: [f32; 2],
    /// Sky light at the eyes, 0..1.
    pub light: f32,
}

/// Textures used to draw entities.
pub struct EntityImages {
    pub skin: RgbaImage,
    pub shadow: RgbaImage,
}

/// Compiles a WGSL shader with common.wgsl prepended.
pub fn shader_module(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(format!("{}\n{source}", include_str!("../shaders/common.wgsl")).into()),
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
    crack_pipeline: wgpu::RenderPipeline,
    item_pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    preview_globals: wgpu::Buffer,
    preview_bind_group: wgpu::BindGroup,
    texture_bind_group: wgpu::BindGroup,
    block_textures: BlockTextures,
    sky: SkyRenderer,
    clouds: CloudRenderer,
    player: PlayerRenderer,
    shadows: ShadowRenderer,
    selection: SelectionRenderer,
    ui: UiRenderer,
    sections: HashMap<IVec3, GpuSection>,
    fog_end: f32,
    /// Sections drawn in the last frame, for the debug screen.
    pub drawn_sections: usize,
    /// GPU name and graphics API, for the debug screen.
    pub gpu: String,
    /// Where to save the next frame, if requested.
    screenshot: Option<std::path::PathBuf>,
}

impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        display: OwnedDisplayHandle,
        images: &BlockImages,
        sky_images: &SkyImages,
        atlas: &GuiAtlas,
        entity_images: &EntityImages,
        options: &Options,
    ) -> Self {
        let (render_distance, vsync) = (options.render_distance, options.vsync);
        let size = window.inner_size();

        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(display)));
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
        // The GUI is drawn through a non-sRGB view of the same frame so that
        // its alpha blending happens on sRGB values, like in Minecraft.
        let gamma_format = format.remove_srgb_suffix();
        let can_reinterpret =
            adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::SURFACE_VIEW_FORMATS);
        let ui_format = if can_reinterpret && gamma_format != format { gamma_format } else { format };
        let config = wgpu::SurfaceConfiguration {
            // COPY_SRC, where supported, lets frames be saved as screenshots.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | (caps.usages & wgpu::TextureUsages::COPY_SRC),
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: present_mode(vsync),
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: if ui_format != format { vec![ui_format] } else { vec![] },
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
            entries: &[gpu::uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT)],
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals bind group"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buffer.as_entire_binding() }],
        });
        // A second set of globals with its own camera, for the inventory player.
        let preview_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let preview_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("preview globals bind group"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: preview_globals.as_entire_binding() }],
        });

        // Block textures.
        let block_textures = BlockTextures::new(&device, &queue, images);
        let (texture_layout, texture_bind_group) = gpu::texture_binding(
            &device,
            "block textures",
            &block_textures.view,
            &block_textures.sampler,
            wgpu::TextureViewDimension::D2Array,
        );

        // Pipelines.
        let shader = shader_module(&device, "block shader", include_str!("../shaders/blocks.wgsl"));
        let layout = gpu::pipeline_layout(&device, "block pipeline layout", &[&globals_layout, &texture_layout]);
        // Block cracks multiply the colour underneath by twice their own, like
        // vanilla, so mid-grey leaves it unchanged and darker pixels darken it.
        let multiply = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Dst,
                dst_factor: wgpu::BlendFactor::Src,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let buffers = [Some(Vertex::layout())];
        let pipeline = |label, vertex, fragment, blend, translucent: bool| {
            PipelineDesc {
                vertex,
                fragment,
                buffers: &buffers,
                blend,
                // Water must be visible from below too.
                cull: if translucent { None } else { Some(wgpu::Face::Back) },
                depth: Some(Depth {
                    format: DEPTH_FORMAT,
                    write: !translucent,
                    compare: wgpu::CompareFunction::LessEqual,
                }),
                ..PipelineDesc::new(label, &layout, &shader, config.format)
            }
            .build(&device)
        };
        let solid_pipeline = pipeline("solid pipeline", "vs_main", "fs_solid", wgpu::BlendState::REPLACE, false);
        let item_pipeline = pipeline("item pipeline", "vs_item", "fs_solid", wgpu::BlendState::REPLACE, false);
        let translucent_pipeline =
            pipeline("translucent pipeline", "vs_main", "fs_translucent", wgpu::BlendState::ALPHA_BLENDING, true);
        let crack_pipeline = pipeline("crack pipeline", "vs_main", "fs_crack", multiply, true);
        let sky = SkyRenderer::new(&device, &queue, sky_images, &globals_layout, config.format, DEPTH_FORMAT);
        let clouds = CloudRenderer::new(&device, &sky_images.clouds, &globals_layout, config.format, DEPTH_FORMAT);
        let player =
            PlayerRenderer::new(&device, &queue, &entity_images.skin, &globals_layout, config.format, DEPTH_FORMAT);
        let shadows =
            ShadowRenderer::new(&device, &queue, &entity_images.shadow, &globals_layout, config.format, DEPTH_FORMAT);
        let ui = UiRenderer::new(&device, &queue, atlas, ui_format);
        let selection = SelectionRenderer::new(&device, &globals_layout, config.format, DEPTH_FORMAT);

        Self {
            surface,
            device,
            queue,
            config,
            depth_view,
            solid_pipeline,
            translucent_pipeline,
            crack_pipeline,
            item_pipeline,
            globals_buffer,
            globals_bind_group,
            preview_globals,
            preview_bind_group,
            texture_bind_group,
            block_textures,
            sky,
            clouds,
            player,
            shadows,
            selection,
            ui,
            sections: HashMap::new(),
            fog_end: fog_end(render_distance),
            drawn_sections: 0,
            gpu: format!("{} ({:?})", info.name, info.backend),
            screenshot: None,
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

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Clouds reach twice as far as the terrain.
    fn cloud_extent(&self) -> f32 {
        self.fog_end * 2.0
    }

    /// Switches VSync on or off.
    pub fn set_vsync(&mut self, vsync: bool) {
        self.config.present_mode = present_mode(vsync);
        self.surface.configure(&self.device, &self.config);
    }

    pub fn set_render_distance(&mut self, render_distance: i32) {
        self.fog_end = fog_end(render_distance);
    }

    /// Drops every section, e.g. when switching worlds.
    pub fn clear_sections(&mut self) {
        self.sections.clear();
    }

    /// Builds this frame's entity shadows; call before `render`.
    pub fn prepare_shadows(&mut self, casters: &[Caster], world: &impl Fn(IVec3) -> Option<Block>) {
        self.shadows.prepare(&self.device, casters, world);
    }

    pub fn total_sections(&self) -> usize {
        self.sections.len()
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

    pub fn render(&mut self, frame: Frame<'_>) {
        let Frame { camera, sky, seconds, player, hand, selection, underwater, items, particles, held, gui, preview } =
            frame;
        let (selection, underwater) = (selection.as_ref(), underwater.as_ref());
        self.block_textures.animate(&self.queue, seconds);
        self.selection.prepare(&self.device, &self.queue, selection, self.size());
        let items = GpuMesh::upload(&self.device, items);
        let particles = GpuMesh::upload(&self.device, particles);
        let held = GpuMesh::upload(&self.device, held);
        self.player.prepare(&self.queue, player, hand);
        self.clouds.update(&self.device, camera.position, seconds, self.cloud_extent());

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
            params: [self.fog_end * 0.6, self.fog_end, sky::CLOUD_HEIGHT, self.cloud_extent()],
            water: underwater.map_or([0.0; 4], |w| [1.0, w.fog_end, w.overlay_offset[0], w.overlay_offset[1]]),
            water_color: underwater.map_or([0.0; 4], |w| w.color.extend(w.light).to_array()),
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
            self.sky.draw(&mut pass);

            pass.set_bind_group(1, &self.texture_bind_group, &[]);
            pass.set_pipeline(&self.solid_pipeline);
            for mesh in visible.iter().filter_map(|s| s.solid.as_ref()) {
                mesh.draw(&mut pass);
            }
            if let Some(particles) = &particles {
                particles.draw(&mut pass);
            }
            if let Some(items) = &items {
                pass.set_pipeline(&self.item_pipeline);
                items.draw(&mut pass);
            }
            self.player.draw_body(&mut pass);
            self.shadows.draw(&mut pass);
            // Clouds can't be seen from under water.
            if underwater.is_none() {
                self.clouds.draw(&mut pass);
            }

            // Translucent geometry goes last, over the finished opaque scene.
            pass.set_bind_group(1, &self.texture_bind_group, &[]);
            pass.set_pipeline(&self.translucent_pipeline);
            for mesh in visible.iter().filter_map(|s| s.translucent.as_ref()) {
                mesh.draw(&mut pass);
            }
            pass.set_pipeline(&self.crack_pipeline);
            self.selection.draw_crack(&mut pass);
            self.selection.draw_outline(&mut pass);
            if underwater.is_some() {
                self.sky.draw_underwater(&mut pass);
            }
        }
        if self.player.has_hand() || held.is_some() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hand pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.globals_bind_group, &[]);
            self.player.draw_hand(&mut pass);
            if let Some(held) = &held {
                pass.set_pipeline(&self.item_pipeline);
                pass.set_bind_group(1, &self.texture_bind_group, &[]);
                held.draw(&mut pass);
            }
        }
        let ui_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor { format: Some(self.ui.format()), ..Default::default() });
        self.ui.draw(&self.device, &self.queue, &mut encoder, &ui_view, &gui);
        if let Some(preview) = preview {
            self.draw_preview(&mut encoder, &view, preview);
        }
        self.queue.submit([encoder.finish()]);
        if let Some(path) = self.screenshot.take() {
            match crate::screenshot::save(&self.device, &self.queue, &frame.texture, &path) {
                Ok(()) => log::info!("saved screenshot {}", path.display()),
                Err(err) => log::error!("screenshot: {err:#}"),
            }
        }
        self.queue.present(frame);
    }

    /// Saves the next rendered frame to `path`.
    pub fn request_screenshot(&mut self, path: std::path::PathBuf) {
        self.screenshot = Some(path);
    }
}

/// Without VSync, frames are shown as soon as they're ready (mailbox where
/// available, so there's no tearing).
fn present_mode(vsync: bool) -> wgpu::PresentMode {
    if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync }
}

/// Fog ends a chunk before the edge of loaded terrain.
fn fog_end(render_distance: i32) -> f32 {
    ((render_distance - 1) * SECTION_SIZE) as f32
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
