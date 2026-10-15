//! The player model shown in the inventory screen, drawn over the GUI with
//! its own camera.

use super::{Globals, GpuMesh, Renderer};
use crate::mesh::MeshData;
use crate::player::EntityVertex;
use crate::ui::Rect;

/// The player shown in the inventory screen's black box.
pub struct Preview {
    /// The box in GUI units, and the GUI scale.
    pub rect: Rect,
    pub gui_scale: f32,
    /// Model and held block, posed around feet at (0, 0, -PREVIEW_DEPTH).
    pub body: Vec<EntityVertex>,
    pub held: MeshData,
}

/// How far in front of the preview camera the model stands.
pub const PREVIEW_DEPTH: f32 = 5.0;
/// Vanilla draws the inventory player 30 GUI units per block.
const PREVIEW_SCALE: f32 = 30.0;

impl Renderer {
    /// Draws the inventory player in its box, over the GUI: an orthographic
    /// camera at the origin looking down -Z, with full light and no fog.
    pub(super) fn draw_preview(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        preview: &Preview,
    ) {
        let (w, h) = self.size();
        let s = preview.gui_scale;
        let r = preview.rect;
        let (x, y) = ((r.x * s).max(0.0), (r.y * s).max(0.0));
        let (pw, ph) = ((r.w * s).min(w as f32 - x), (r.h * s).min(h as f32 - y));
        if pw < 1.0 || ph < 1.0 {
            return;
        }
        let (half_w, height) = (r.w / PREVIEW_SCALE / 2.0, r.h / PREVIEW_SCALE);
        // Feet a little above the bottom of the box, like vanilla.
        let bottom = -0.12;
        let proj = glam::camera::rh::proj::directx::orthographic(-half_w, half_w, bottom, bottom + height, 0.1, 10.0);
        let globals = Globals {
            view_proj: proj.to_cols_array_2d(),
            inv_view_proj: proj.inverse().to_cols_array_2d(),
            camera_pos: [0.0, 0.0, -PREVIEW_DEPTH, 0.0],
            sun_dir: [0.0, 1.0, 0.0, 1.0],
            zenith: [0.0, 0.0, 0.0, 1.0],
            horizon: [0.0; 4],
            sunset: [0.0; 4],
            params: [1e6, 2e6, 0.0, 0.0],
            water: [0.0; 4],
            water_color: [0.0; 4],
        };
        self.queue.write_buffer(&self.preview_globals, 0, bytemuck::bytes_of(&globals));
        self.player.prepare_preview(&self.queue, &preview.body);
        let held = GpuMesh::upload(&self.device, &preview.held);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("preview pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
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
        pass.set_viewport(x, y, pw, ph, 0.0, 1.0);
        pass.set_scissor_rect(x as u32, y as u32, pw as u32, ph as u32);
        pass.set_bind_group(0, &self.preview_bind_group, &[]);
        self.player.draw_preview(&mut pass);
        if let Some(held) = &held {
            pass.set_pipeline(&self.item_pipeline);
            pass.set_bind_group(1, &self.texture_bind_group, &[]);
            held.draw(&mut pass);
        }
    }
}
