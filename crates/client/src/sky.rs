//! Day/night cycle and the sky dome (gradient, sun, moon, stars).

use std::f32::consts::TAU;

use glam::Vec3;
use rscraft_assets::{ClientJar, RgbaImage};

/// A full day lasts 20 minutes, like in Minecraft.
const DAY_SECONDS: f32 = 1200.0;
pub const CLOUD_HEIGHT: f32 = 192.33;
const CELESTIAL_SIZE: u32 = 32;

// Linear-space colours (the surface is sRGB).
const DAY_ZENITH: Vec3 = Vec3::new(0.19, 0.39, 1.0);
const DAY_HORIZON: Vec3 = Vec3::new(0.53, 0.69, 1.0);
const NIGHT_ZENITH: Vec3 = Vec3::new(0.002, 0.003, 0.012);
const NIGHT_HORIZON: Vec3 = Vec3::new(0.01, 0.013, 0.035);
const SUNSET: Vec3 = Vec3::new(0.95, 0.28, 0.04);
/// Ambient light at night, relative to full daylight.
const NIGHT_AMBIENT: f32 = 0.16;

pub struct DayCycle {
    /// Fraction of the day: 0 = sunrise, 0.25 = noon, 0.5 = sunset, 0.75 = midnight.
    pub time: f32,
}

/// Everything the shaders need to know about the sky right now.
pub struct SkyState {
    pub sun_dir: Vec3,
    pub daylight: f32,
    pub ambient: f32,
    pub zenith: Vec3,
    pub horizon: Vec3,
    pub sunset_color: Vec3,
    /// 0..1, peaks while the sun crosses the horizon.
    pub sunset: f32,
}

impl DayCycle {
    pub fn advance(&mut self, dt: f32, speed: f32) {
        self.time = (self.time + dt * speed / DAY_SECONDS).fract();
    }

    pub fn state(&self) -> SkyState {
        let angle = self.time * TAU;
        // The sun rises in the east (+X) and sets in the west.
        let sun_dir = Vec3::new(angle.cos(), angle.sin(), 0.0);
        let height = sun_dir.y;
        let daylight = smoothstep(-0.2, 0.25, height);
        // Sunrise/sunset glow peaks while the sun is near the horizon.
        let sunset = (1.0 - smoothstep(0.0, 0.3, height.abs())) * smoothstep(-0.3, 0.0, height);
        SkyState {
            sun_dir,
            daylight,
            ambient: NIGHT_AMBIENT + (1.0 - NIGHT_AMBIENT) * daylight,
            zenith: NIGHT_ZENITH.lerp(DAY_ZENITH, daylight),
            horizon: NIGHT_HORIZON.lerp(DAY_HORIZON, daylight),
            sunset_color: SUNSET,
            sunset,
        }
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Sky textures from the client jar.
pub struct SkyImages {
    sun: RgbaImage,
    moon: RgbaImage,
    /// misc/underwater.png, the faint texture over the view under water.
    underwater: RgbaImage,
    pub clouds: RgbaImage,
}

impl SkyImages {
    pub fn load(mut jar: Option<&mut ClientJar>) -> Self {
        let mut texture = |name: &str| {
            jar.as_deref_mut().and_then(|jar| {
                jar.texture(name).map_err(|err| log::warn!("sky texture {name}: {err:#}")).ok()
            })
        };
        let sun = texture("environment/celestial/sun").unwrap_or_else(|| placeholder_disc([255, 240, 160, 255]));
        let moon = texture("environment/celestial/moon/full_moon")
            .unwrap_or_else(|| placeholder_disc([200, 200, 210, 255]));
        let clouds = texture("environment/clouds").unwrap_or_else(|| RgbaImage::new(1, 1));
        let underwater = texture("misc/underwater").unwrap_or_else(|| RgbaImage::new(1, 1));
        Self {
            sun: sun.resized(CELESTIAL_SIZE, CELESTIAL_SIZE),
            moon: moon.resized(CELESTIAL_SIZE, CELESTIAL_SIZE),
            underwater: underwater.resized(CELESTIAL_SIZE, CELESTIAL_SIZE),
            clouds,
        }
    }
}

/// A bright square on black, drawn additively like the real sun texture.
fn placeholder_disc(color: [u8; 4]) -> RgbaImage {
    let mut image = RgbaImage::new(CELESTIAL_SIZE, CELESTIAL_SIZE);
    let (lo, hi) = (CELESTIAL_SIZE / 4, CELESTIAL_SIZE * 3 / 4);
    for y in 0..CELESTIAL_SIZE {
        for x in 0..CELESTIAL_SIZE {
            let inside = (lo..hi).contains(&x) && (lo..hi).contains(&y);
            image.set_pixel(x, y, if inside { color } else { [0, 0, 0, 255] });
        }
    }
    image
}

/// Draws the sky behind everything else.
pub struct SkyRenderer {
    pipeline: wgpu::RenderPipeline,
    underwater_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
}

impl SkyRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &SkyImages,
        globals_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let celestial = upload(device, queue, "celestial", &[&images.sun, &images.moon, &images.underwater]);
        let celestial_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("celestial sampler"),
            ..Default::default()
        });

        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky layout"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&celestial) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&celestial_sampler) },
            ],
        });

        let shader = crate::renderer::shader_module(device, "sky shader", include_str!("sky.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky pipeline layout"),
            bind_group_layouts: &[Some(globals_layout), Some(&layout)],
            ..Default::default()
        });
        let pipeline = |label, fragment, blend| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sky"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(fragment),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline: pipeline("sky pipeline", "fs_sky", wgpu::BlendState::REPLACE),
            underwater_pipeline: pipeline("underwater overlay pipeline", "fs_underwater", wgpu::BlendState::ALPHA_BLENDING),
            bind_group,
        }
    }

    /// The faint moving water texture over the whole view, while under water.
    pub fn draw_underwater(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.underwater_pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// A full-screen triangle; must be drawn first.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Uploads same-sized images as layers of a texture array.
fn upload(device: &wgpu::Device, queue: &wgpu::Queue, label: &str, layers: &[&RgbaImage]) -> wgpu::TextureView {
    let (width, height) = (layers[0].width, layers[0].height);
    let size = wgpu::Extent3d { width, height, depth_or_array_layers: layers.len() as u32 };
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
    let pixels: Vec<u8> = layers.iter().flat_map(|image| image.pixels.iter().copied()).collect();
    queue.write_texture(
        texture.as_image_copy(),
        &pixels,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * width), rows_per_image: Some(height) },
        size,
    );
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}
