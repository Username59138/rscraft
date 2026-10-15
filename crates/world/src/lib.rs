//! World data model: blocks, 16³ sections and terrain generation.
//! Knows nothing about rendering, so it can be shared with networking later.

pub mod block;
pub mod crafting;
pub mod dropped_items;
pub mod fluid;
pub mod gamemode;
pub mod generator;
pub mod inventory;
pub mod light;
pub mod movement;
pub mod random_ticks;
pub mod raycast;
pub mod section;

pub use block::{Block, Face, FaceTexture, RenderLayer, Tint};
pub use generator::Generator;
pub use light::{ColumnGroup, ColumnLight, LightRegion, MAX_LIGHT};
pub use section::{SECTION_SIZE, SECTION_VOLUME, Section};

/// World height range, matching Minecraft's overworld.
pub const MIN_Y: i32 = -64;
pub const MAX_Y: i32 = 320;
pub const MIN_SECTION_Y: i32 = MIN_Y / SECTION_SIZE;
pub const SECTIONS_PER_COLUMN: usize = ((MAX_Y - MIN_Y) / SECTION_SIZE) as usize;
pub const SEA_LEVEL: i32 = 63;
/// The game runs at 20 ticks per second, like Minecraft.
pub const TICK_SECONDS: f32 = 0.05;
