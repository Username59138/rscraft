//! Saved worlds and options, stored as JSON in the user data directory.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use std::collections::HashMap;

use anyhow::{Context, Result};
use glam::{IVec2, IVec3};
use rscraft_world::Block;
use serde::{Deserialize, Serialize};

use crate::chunks::Edits;
use crate::gamemode::GameMode;
use crate::inventory::{HOTBAR, Inventory, ItemStack, MAX_STACK};

fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("rscraft")
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerSave {
    pub position: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    #[serde(default)]
    pub flying: bool,
    #[serde(default)]
    pub inventory: Vec<SlotSave>,
    #[serde(default)]
    pub selected_slot: usize,
}

/// One non-empty inventory slot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SlotSave {
    pub slot: usize,
    /// Block id (see `Block::from_id`).
    pub block: u8,
    pub count: u8,
}

impl PlayerSave {
    pub fn load_inventory(&self) -> Inventory {
        let mut inventory = Inventory { selected: self.selected_slot.min(HOTBAR - 1), ..Default::default() };
        for s in &self.inventory {
            if let (Some(slot), Some(block)) = (inventory.slots.get_mut(s.slot), Block::from_id(s.block)) {
                *slot = Some(ItemStack { block, count: s.count.clamp(1, MAX_STACK) });
            }
        }
        inventory
    }

    pub fn store_inventory(&mut self, inventory: &Inventory) {
        self.selected_slot = inventory.selected;
        self.inventory = inventory
            .slots
            .iter()
            .enumerate()
            .filter_map(|(slot, s)| s.map(|s| SlotSave { slot, block: s.block as u8, count: s.count }))
            .collect();
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldSave {
    pub name: String,
    pub seed: i64,
    pub created: u64,
    pub last_played: u64,
    pub time_of_day: f32,
    /// None until the world has been played once.
    pub player: Option<PlayerSave>,
    #[serde(default)]
    pub game_mode: GameMode,
    /// What F3+F4 offers first.
    #[serde(default)]
    pub previous_game_mode: Option<GameMode>,
}

impl WorldSave {
    /// The 32-bit seed our generator uses.
    pub fn generator_seed(&self) -> u32 {
        (self.seed as u64 ^ (self.seed as u64 >> 32)) as u32
    }
}

#[derive(Clone, Debug)]
pub struct WorldEntry {
    pub folder: PathBuf,
    pub save: WorldSave,
}

impl WorldEntry {
    pub fn folder_name(&self) -> String {
        self.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn write(&self) -> Result<()> {
        fs::create_dir_all(&self.folder)?;
        let json = serde_json::to_string_pretty(&self.save)?;
        fs::write(self.folder.join("world.json"), json).with_context(|| format!("saving {}", self.folder.display()))
    }

    /// Player block edits, stored as a small binary file: per column its
    /// coordinates and edit count, then (x, y, z, block id) per edit.
    pub fn write_edits(&self, edits: &Edits) -> Result<()> {
        let mut out = Vec::from(*EDITS_MAGIC);
        out.extend_from_slice(&(edits.len() as u32).to_le_bytes());
        for (column, blocks) in edits {
            for v in [column.x, column.y] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&(blocks.len() as u32).to_le_bytes());
            for (pos, block) in blocks {
                for v in [pos.x, pos.y, pos.z] {
                    out.extend_from_slice(&v.to_le_bytes());
                }
                out.push(*block as u8);
            }
        }
        fs::write(self.folder.join("edits.bin"), out).context("saving block edits")
    }

    pub fn read_edits(&self) -> Edits {
        let Ok(data) = fs::read(self.folder.join("edits.bin")) else { return Edits::new() };
        parse_edits(&data).unwrap_or_else(|| {
            log::error!("{}: corrupt edits.bin, ignoring it", self.folder.display());
            Edits::new()
        })
    }

    pub fn delete(&self) -> Result<()> {
        fs::remove_dir_all(&self.folder).with_context(|| format!("deleting {}", self.folder.display()))
    }
}

const EDITS_MAGIC: &[u8; 4] = b"RSE1";

/// Reads little-endian values from the front of a byte slice.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn u8(&mut self) -> Option<u8> {
        let (&byte, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(byte)
    }

    fn i32(&mut self) -> Option<i32> {
        let (bytes, rest) = self.0.split_first_chunk::<4>()?;
        self.0 = rest;
        Some(i32::from_le_bytes(*bytes))
    }
}

fn parse_edits(data: &[u8]) -> Option<Edits> {
    let mut reader = Reader(data.strip_prefix(EDITS_MAGIC)?);
    let mut edits = Edits::new();
    for _ in 0..reader.i32()? {
        let column = IVec2::new(reader.i32()?, reader.i32()?);
        let blocks: &mut HashMap<IVec3, Block> = edits.entry(column).or_default();
        for _ in 0..reader.i32()? {
            let pos = IVec3::new(reader.i32()?, reader.i32()?, reader.i32()?);
            blocks.insert(pos, Block::from_id(reader.u8()?)?);
        }
    }
    Some(edits)
}

fn worlds_dir() -> PathBuf {
    data_dir().join("worlds")
}

/// All saved worlds, most recently played first.
pub fn list_worlds() -> Vec<WorldEntry> {
    let Ok(dirs) = fs::read_dir(worlds_dir()) else { return Vec::new() };
    let mut worlds: Vec<WorldEntry> = dirs
        .flatten()
        .filter_map(|entry| {
            let folder = entry.path();
            let data = fs::read(folder.join("world.json")).ok()?;
            match serde_json::from_slice(&data) {
                Ok(save) => Some(WorldEntry { folder, save }),
                Err(err) => {
                    log::warn!("skipping world {}: {err}", folder.display());
                    None
                }
            }
        })
        .collect();
    worlds.sort_by_key(|w| std::cmp::Reverse(w.save.last_played));
    worlds
}

/// Creates and saves a new world. The folder name is derived from the world
/// name, made unique if needed.
pub fn create_world(name: &str, seed_text: &str, game_mode: GameMode) -> Result<WorldEntry> {
    let name = if name.trim().is_empty() { "New World" } else { name.trim() };
    let base: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' })
        .collect();
    let mut folder = worlds_dir().join(&base);
    let mut n = 2;
    while folder.exists() {
        folder = worlds_dir().join(format!("{base} ({n})"));
        n += 1;
    }
    let now = now();
    let entry = WorldEntry {
        folder,
        save: WorldSave {
            name: name.to_string(),
            seed: parse_seed(seed_text),
            created: now,
            last_played: now,
            time_of_day: 0.04,
            player: None,
            game_mode,
            previous_game_mode: None,
        },
    };
    entry.write()?;
    Ok(entry)
}

/// Like Minecraft: a number is used as-is, other text is hashed with Java's
/// `String.hashCode`, and an empty seed is random.
pub fn parse_seed(text: &str) -> i64 {
    let text = text.trim();
    if text.is_empty() {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        return (nanos as i64).wrapping_mul(0x5851_f42d_4c95_7f2d);
    }
    text.parse().unwrap_or_else(|_| {
        text.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32)) as i64
    })
}

/// Formats a Unix timestamp as `YYYY-MM-DD HH:MM` in local time.
pub fn format_date(timestamp: u64) -> String {
    chrono::DateTime::from_timestamp(timestamp as i64, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    /// Vertical field of view in degrees.
    pub fov: f32,
    /// In chunks.
    pub render_distance: i32,
    /// 1.0 = 100%.
    pub sensitivity: f32,
    /// 0 = auto.
    pub gui_scale: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self { fov: 70.0, render_distance: 12, sensitivity: 1.0, gui_scale: 0 }
    }
}

impl Options {
    fn path() -> PathBuf {
        data_dir().join("options.json")
    }

    pub fn load() -> Self {
        fs::read(Self::path()).ok().and_then(|data| serde_json::from_slice(&data).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let result = fs::create_dir_all(data_dir())
            .and_then(|_| fs::write(Self::path(), serde_json::to_string_pretty(self).unwrap_or_default()));
        if let Err(err) = result {
            log::warn!("saving options: {err}");
        }
    }
}
