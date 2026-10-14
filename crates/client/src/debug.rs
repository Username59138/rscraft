//! The F3 debug screen.

use glam::Vec3;
use rscraft_world::SECTION_SIZE;

use crate::ui::{LIGHT_GRAY, LINE_HEIGHT, Rect, Ui};

pub struct DebugInfo<'a> {
    pub fps: u32,
    pub drawn_sections: usize,
    pub total_sections: usize,
    pub loaded_columns: usize,
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub time_of_day: f32,
    pub seed: i64,
    pub render_distance: i32,
    pub gpu: &'a str,
    pub screen: (u32, u32),
}

/// Resident memory of this process, Linux only.
fn memory_mib() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096 / (1024 * 1024))
}

fn facing(yaw: f32) -> &'static str {
    let (x, z) = (yaw.sin(), -yaw.cos());
    if x.abs() > z.abs() {
        if x > 0.0 { "east (Towards positive X)" } else { "west (Towards negative X)" }
    } else if z < 0.0 {
        "north (Towards negative Z)"
    } else {
        "south (Towards positive Z)"
    }
}

pub fn draw(ui: &mut Ui, info: &DebugInfo) {
    let p = info.position;
    let block = p.floor().as_ivec3();
    let chunk = block.div_euclid(glam::IVec3::splat(SECTION_SIZE));
    let local = block.rem_euclid(glam::IVec3::splat(SECTION_SIZE));
    // Minecraft's yaw: 0 = south, growing clockwise.
    let mc_yaw = (info.yaw.to_degrees() + 180.0 + 180.0).rem_euclid(360.0) - 180.0;
    // Day 0 starts at sunrise, 06:00.
    let hours = (info.time_of_day * 24.0 + 6.0) % 24.0;

    let left = [
        format!("rscraft {} (Minecraft {} assets)", env!("CARGO_PKG_VERSION"), crate::MC_VERSION),
        format!("{} fps", info.fps),
        format!("C: {}/{} sections, {} columns, D: {}", info.drawn_sections, info.total_sections, info.loaded_columns, info.render_distance),
        String::new(),
        format!("XYZ: {:.3} / {:.5} / {:.3}", p.x, p.y, p.z),
        format!("Block: {} {} {}", block.x, block.y, block.z),
        format!("Chunk: {} {} {} in {} {} {}", local.x, local.y, local.z, chunk.x, chunk.y, chunk.z),
        format!("Facing: {} ({:.1} / {:.1})", facing(info.yaw), mc_yaw, -info.pitch.to_degrees()),
        format!("Time: {:02}:{:02}", hours as u32, (hours.fract() * 60.0) as u32),
        format!("Seed: {}", info.seed),
    ];
    let right = [
        format!("Mem: {} MiB", memory_mib().map_or("?".to_string(), |m| m.to_string())),
        format!("CPU: {} threads", rayon::current_num_threads()),
        String::new(),
        format!("Display: {}x{}", info.screen.0, info.screen.1),
        info.gpu.to_string(),
    ];

    for (i, line) in left.iter().enumerate() {
        draw_line(ui, 2.0, 2.0 + i as f32 * LINE_HEIGHT, line);
    }
    for (i, line) in right.iter().enumerate() {
        let x = ui.width - 2.0 - ui.text_width(line);
        draw_line(ui, x, 2.0 + i as f32 * LINE_HEIGHT, line);
    }
}

/// A line of text on a translucent gray background, like vanilla F3.
fn draw_line(ui: &mut Ui, x: f32, y: f32, text: &str) {
    if text.is_empty() {
        return;
    }
    ui.fill(Rect::new(x - 1.0, y - 1.0, ui.text_width(text) + 2.0, LINE_HEIGHT), 0x505050, 0.56);
    ui.text_flat(x, y, text, LIGHT_GRAY);
}
