//! Minecraft's "fancy" clouds: every opaque texel of clouds.png becomes a
//! 12×4×12 box, with the faces between neighbouring boxes left out.

use bytemuck::{Pod, Zeroable};
use glam::{IVec2, Vec3};
use rscraft_assets::RgbaImage;

use crate::gpu::{self, Depth, PipelineDesc};
use crate::sky::CLOUD_HEIGHT;

/// Size of one cloud cell in blocks.
const CELL_SIZE: f32 = 12.0;
const CLOUD_THICKNESS: f32 = 4.0;
/// Clouds drift towards +X at this speed, in blocks per second.
pub const CLOUD_SPEED: f32 = 0.6;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct CloudVertex {
    /// World position before drift is applied.
    position: [f32; 3],
    /// Fixed per-face brightness.
    shade: f32,
}

/// Which cells of the 256×256 cloud map are cloudy; wraps around.
struct CloudMap {
    width: i32,
    height: i32,
    cells: Vec<bool>,
}

impl CloudMap {
    fn new(image: &RgbaImage) -> Self {
        let cells = image.pixels.as_chunks::<4>().0.iter().map(|p| p[3] >= 128).collect();
        Self { width: image.width as i32, height: image.height as i32, cells }
    }

    fn get(&self, x: i32, z: i32) -> bool {
        let (x, z) = (x.rem_euclid(self.width), z.rem_euclid(self.height));
        self.cells[(z * self.width + x) as usize]
    }
}

pub struct CloudRenderer {
    map: CloudMap,
    depth_pipeline: wgpu::RenderPipeline,
    color_pipeline: wgpu::RenderPipeline,
    vertex_buffer: Option<wgpu::Buffer>,
    vertex_count: u32,
    /// Cell the current mesh is centred on, and its radius in cells.
    built_for: Option<(IVec2, i32)>,
}

impl CloudRenderer {
    pub fn new(
        device: &wgpu::Device,
        image: &RgbaImage,
        globals_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = crate::renderer::shader_module(device, "cloud shader", include_str!("clouds.wgsl"));
        let layout = gpu::pipeline_layout(device, "cloud pipeline layout", &[globals_layout]);
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<CloudVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32],
        })];
        // Like vanilla: a depth-only pass first, so the translucent colour
        // pass only shows the nearest cloud surface.
        let depth_pipeline = PipelineDesc {
            buffers: &buffers,
            write_mask: wgpu::ColorWrites::empty(),
            depth: Some(Depth::opaque(depth_format)),
            ..PipelineDesc::new("cloud depth pipeline", &layout, &shader, color_format)
        }
        .build(device);
        let color_pipeline = PipelineDesc {
            buffers: &buffers,
            depth: Some(Depth::read_only(depth_format, wgpu::CompareFunction::LessEqual)),
            ..PipelineDesc::new("cloud pipeline", &layout, &shader, color_format)
        }
        .build(device);
        Self {
            map: CloudMap::new(image),
            depth_pipeline,
            color_pipeline,
            vertex_buffer: None,
            vertex_count: 0,
            built_for: None,
        }
    }

    /// Rebuilds the mesh when the camera (relative to the drifting clouds)
    /// moves to another cell or the visible radius changes.
    pub fn update(&mut self, device: &wgpu::Device, camera: Vec3, seconds: f32, extent: f32) {
        let drift = seconds * CLOUD_SPEED;
        let center = IVec2::new(((camera.x - drift) / CELL_SIZE).floor() as i32, (camera.z / CELL_SIZE).floor() as i32);
        let radius = (extent / CELL_SIZE).ceil() as i32 + 1;
        if self.built_for == Some((center, radius)) {
            return;
        }
        self.built_for = Some((center, radius));

        let mut vertices = Vec::new();
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dz * dz > radius * radius {
                    continue;
                }
                let cell = center + IVec2::new(dx, dz);
                if self.map.get(cell.x, cell.y) {
                    self.push_cell(&mut vertices, cell);
                }
            }
        }

        self.vertex_count = vertices.len() as u32;
        self.vertex_buffer = (!vertices.is_empty()).then(|| {
            wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("cloud vertices"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            )
        });
    }

    fn push_cell(&self, out: &mut Vec<CloudVertex>, cell: IVec2) {
        let min = Vec3::new(cell.x as f32 * CELL_SIZE, CLOUD_HEIGHT, cell.y as f32 * CELL_SIZE);
        let max = min + Vec3::new(CELL_SIZE, CLOUD_THICKNESS, CELL_SIZE);
        let mut quad = |corners: [Vec3; 4], shade: f32| {
            let v = |p: Vec3| CloudVertex { position: p.to_array(), shade };
            out.extend_from_slice(&[
                v(corners[0]),
                v(corners[1]),
                v(corners[2]),
                v(corners[0]),
                v(corners[2]),
                v(corners[3]),
            ]);
        };
        let (x0, y0, z0, x1, y1, z1) = (min.x, min.y, min.z, max.x, max.y, max.z);
        let p = Vec3::new;

        // Vanilla shading: top 1.0, bottom 0.7, X sides 0.9, Z sides 0.8.
        // Corners go counter-clockwise when seen from outside.
        quad([p(x0, y1, z1), p(x1, y1, z1), p(x1, y1, z0), p(x0, y1, z0)], 1.0);
        quad([p(x0, y0, z0), p(x1, y0, z0), p(x1, y0, z1), p(x0, y0, z1)], 0.7);
        if !self.map.get(cell.x + 1, cell.y) {
            quad([p(x1, y0, z1), p(x1, y0, z0), p(x1, y1, z0), p(x1, y1, z1)], 0.9);
        }
        if !self.map.get(cell.x - 1, cell.y) {
            quad([p(x0, y0, z0), p(x0, y0, z1), p(x0, y1, z1), p(x0, y1, z0)], 0.9);
        }
        if !self.map.get(cell.x, cell.y + 1) {
            quad([p(x0, y0, z1), p(x1, y0, z1), p(x1, y1, z1), p(x0, y1, z1)], 0.8);
        }
        if !self.map.get(cell.x, cell.y - 1) {
            quad([p(x1, y0, z0), p(x0, y0, z0), p(x0, y1, z0), p(x1, y1, z0)], 0.8);
        }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(buffer) = &self.vertex_buffer else { return };
        pass.set_vertex_buffer(0, buffer.slice(..));
        for pipeline in [&self.depth_pipeline, &self.color_pipeline] {
            pass.set_pipeline(pipeline);
            pass.draw(0..self.vertex_count, 0..1);
        }
    }
}
