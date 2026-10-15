//! Scripted scenes for testing: `rscraft --scene scene.json` opens a
//! throwaway world, places the player, holds keys, waits, saves a screenshot
//! and quits. The user's saves are never touched.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use glam::{IVec3, Vec3};
use rscraft_world::Block;
use rscraft_world::gamemode::GameMode;
use rscraft_world::inventory::ItemStack;
use serde::Deserialize;
use winit::keyboard::KeyCode;

use crate::camera::Perspective;
use crate::saves::{self, WorldEntry};

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenePerspective {
    #[default]
    First,
    Back,
    Front,
}

impl From<ScenePerspective> for Perspective {
    fn from(p: ScenePerspective) -> Self {
        match p {
            ScenePerspective::First => Perspective::FirstPerson,
            ScenePerspective::Back => Perspective::ThirdPersonBack,
            ScenePerspective::Front => Perspective::ThirdPersonFront,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneScreen {
    #[default]
    None,
    Inventory,
    Pause,
}

/// Fills a box of blocks (both corners included), e.g. to carve a cave.
#[derive(Clone, Debug, Deserialize)]
pub struct Fill {
    pub from: [i32; 3],
    pub to: [i32; 3],
    pub block: String,
}

/// A fill's two corners and its block.
pub type ResolvedFill = (IVec3, IVec3, Block);

/// The script's block names resolved.
#[derive(Default)]
pub struct SceneBlocks {
    pub hotbar: Vec<ItemStack>,
    pub fills: Vec<ResolvedFill>,
}

/// A scene file. Angles are in degrees; everything has a default.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SceneScript {
    pub seed: String,
    pub game_mode: GameMode,
    /// Feet position; the world's spawn when left out.
    pub position: Option<[f32; 3]>,
    pub yaw: f32,
    pub pitch: f32,
    pub flying: bool,
    /// Time of day, 0 = sunrise, 0.25 = noon, 0.5 = sunset.
    pub time: f32,
    pub perspective: ScenePerspective,
    pub screen: SceneScreen,
    pub debug: bool,
    /// Hotbar contents from slot 1, as (block id, count).
    pub hotbar: Vec<(String, u8)>,
    pub selected: usize,
    /// Keys held for the whole scene, by winit name: "KeyW", "Space", ...
    pub keys: Vec<String>,
    pub attack: bool,
    pub use_item: bool,
    pub fill: Vec<Fill>,
    /// Seconds to run after the world has loaded.
    pub wait: f32,
    pub screenshot: PathBuf,
    /// Mouse cursor as fractions of the window (0..1 from the top left),
    /// held there for the whole scene.
    pub mouse: Option<[f32; 2]>,
    /// Render distance in chunks; the player's own setting when left out.
    pub render_distance: Option<i32>,
    /// Turn VSync off and log frame rate and per-stage frame times.
    pub benchmark: bool,
}

impl Default for SceneScript {
    fn default() -> Self {
        Self {
            seed: "rscraft".into(),
            game_mode: GameMode::Survival,
            position: None,
            yaw: 0.0,
            pitch: 0.0,
            flying: false,
            time: 0.2,
            perspective: ScenePerspective::First,
            screen: SceneScreen::None,
            debug: false,
            hotbar: Vec::new(),
            selected: 0,
            keys: Vec::new(),
            attack: false,
            use_item: false,
            fill: Vec::new(),
            wait: 2.0,
            render_distance: None,
            mouse: None,
            screenshot: PathBuf::from("scene.png"),
            benchmark: false,
        }
    }
}

impl SceneScript {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut script: SceneScript =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        // Relative screenshot paths are next to the scene file.
        if script.screenshot.is_relative()
            && let Some(dir) = path.parent()
        {
            script.screenshot = dir.join(&script.screenshot);
        }
        script.keys().context("in \"keys\"")?;
        script.blocks().context("in \"hotbar\" or \"fill\"")?;
        Ok(script)
    }

    pub fn keys(&self) -> Result<Vec<KeyCode>> {
        self.keys.iter().map(|name| key_code(name)).collect()
    }

    /// The hotbar and fills with block names resolved.
    pub fn blocks(&self) -> Result<SceneBlocks> {
        let block = |name: &str| Block::from_name(name).with_context(|| format!("unknown block {name:?}"));
        let hotbar = self
            .hotbar
            .iter()
            .map(|(name, count)| Ok(ItemStack { block: block(name)?, count: *count }))
            .collect::<Result<_>>()?;
        let fill = self
            .fill
            .iter()
            .map(|f| Ok((IVec3::from(f.from), IVec3::from(f.to), block(&f.block)?)))
            .collect::<Result<_>>()?;
        Ok(SceneBlocks { hotbar, fills: fill })
    }

    pub fn feet(&self) -> Option<Vec3> {
        self.position.map(Vec3::from)
    }
}

fn key_code(name: &str) -> Result<KeyCode> {
    Ok(match name {
        "KeyW" => KeyCode::KeyW,
        "KeyA" => KeyCode::KeyA,
        "KeyS" => KeyCode::KeyS,
        "KeyD" => KeyCode::KeyD,
        "KeyC" => KeyCode::KeyC,
        "KeyT" => KeyCode::KeyT,
        "Space" => KeyCode::Space,
        "ShiftLeft" => KeyCode::ShiftLeft,
        "ControlLeft" => KeyCode::ControlLeft,
        other => bail!("unsupported key {other:?}"),
    })
}

/// Time spent in each part of the frame, summed over the scene.
#[derive(Default)]
pub struct FrameTimes {
    pub frames: u32,
    pub simulate: f64,
    pub chunks: f64,
    pub gui: f64,
    pub scene: f64,
    pub render: f64,
}

impl FrameTimes {
    pub fn report(&self, seconds: f32) {
        let n = self.frames.max(1) as f64;
        let ms = |t: f64| t / n * 1000.0;
        log::info!(
            "benchmark: {} frames in {seconds:.1} s = {:.0} fps; per frame: simulate {:.2} ms, chunks {:.2} ms, \
             gui {:.2} ms, scene {:.2} ms, render {:.2} ms",
            self.frames,
            self.frames as f32 / seconds,
            ms(self.simulate),
            ms(self.chunks),
            ms(self.gui),
            ms(self.scene),
            ms(self.render),
        );
    }
}

/// A scene being played.
pub struct SceneRun {
    pub script: SceneScript,
    folder: PathBuf,
    /// When the world finished loading.
    pub started: Option<Instant>,
    pub screenshot_taken: bool,
    pub times: FrameTimes,
}

impl SceneRun {
    /// Creates the throwaway world the scene plays in.
    pub fn new(script: SceneScript) -> Result<(Self, WorldEntry)> {
        let folder = std::env::temp_dir().join(format!("rscraft-scene-{}", std::process::id()));
        let entry = saves::create_world_at(folder.clone(), "Scene", &script.seed, script.game_mode)?;
        Ok((Self { script, folder, started: None, screenshot_taken: false, times: FrameTimes::default() }, entry))
    }

    /// Whether the scene has run long enough to take its screenshot.
    pub fn due(&self) -> bool {
        self.started.is_some_and(|t| t.elapsed().as_secs_f32() >= self.script.wait)
    }
}

impl Drop for SceneRun {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}
