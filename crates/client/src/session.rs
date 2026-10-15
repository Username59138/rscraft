//! A loaded world and the player in it: the game state, what the player
//! does to it, and the 3D scene drawn from it each frame.

use std::sync::Arc;

use glam::{IVec3, Vec3};
use rscraft_world::crafting::Grid;
use rscraft_world::dropped_items::DroppedItems;
use rscraft_world::gamemode::GameMode;
use rscraft_world::inventory::{Inventory, ItemStack};
use rscraft_world::movement::{Controls, EYE_HEIGHT, PlayerController};
use rscraft_world::raycast::{Hit, REACH, raycast};
use rscraft_world::{Block, Generator, SEA_LEVEL, TICK_SECONDS};

use crate::camera::{Camera, Perspective};
use crate::chunks::{ChunkManager, Edits};
use crate::item_render;
use crate::mesh::MeshData;
use crate::particles::Particles;
use crate::player::{self, EntityVertex, HandPose, PlayerPose};
use crate::renderer::{PREVIEW_DEPTH, Preview, Underwater};
use crate::saves::{self, Options, PlayerSave, WorldEntry};
use crate::shadows::Caster;
use crate::sky::DayCycle;
use crate::texture::BlockLayers;
use crate::ui::Rect;

/// World shown behind the title screen.
const MENU_SEED: u32 = 1337;

/// View bobbing strength; 1.0 is vanilla.
const VIEW_BOB_STRENGTH: f32 = 0.7;
/// How fast the hand lowers and rises when switching items, per second.
const EQUIP_SPEED: f32 = 0.4 * 20.0;
/// Seconds for a full arm swing.
const SWING_SECONDS: f32 = 0.2;
/// FOV multiplier while sprinting, like vanilla.
const SPRINT_FOV: f32 = 1.1;
/// Zoom while holding C, like OptiFine; the mouse wheel changes it.
const DEFAULT_ZOOM: f32 = 4.0;
const MIN_ZOOM: f32 = 1.5;
const MAX_ZOOM: f32 = 50.0;
/// Each wheel notch zooms in or out by this factor.
const ZOOM_STEP: f32 = 1.25;

/// What `Session::scene` builds for the renderer.
pub struct Scene {
    /// The player's body, in third person.
    pub player: Option<Vec<EntityVertex>>,
    /// The bare first-person arm.
    pub hand: Option<Vec<EntityVertex>>,
    /// Dropped items and the block held in third person.
    pub items: MeshData,
    pub particles: MeshData,
    /// The block held in first person.
    pub held: MeshData,
    pub shadows: Vec<Caster>,
}

/// A loaded world: the title screen backdrop, or a saved world being played.
pub struct Session {
    pub chunks: ChunkManager,
    pub camera: Camera,
    pub day: DayCycle,
    pub world: Option<WorldEntry>,
    /// The player; None for the title backdrop.
    pub player: Option<PlayerController>,
    pub game_mode: GameMode,
    pub previous_game_mode: Option<GameMode>,
    pub inventory: Inventory,
    /// The stack held by the mouse in the inventory screen.
    pub cursor: Option<ItemStack>,
    /// The inventory's 2×2 crafting grid.
    pub crafting: Grid,
    pub items: DroppedItems,
    pub particles: Particles,
    pub hit_particle_timer: f32,
    /// Block being broken and how far along it is (0..1).
    pub breaking: Option<(IVec3, f32)>,
    /// Delays between breaking or placing blocks while a button is held.
    pub break_cooldown: f32,
    pub place_cooldown: f32,
    /// Arm swing progress, 0 when idle.
    pub swing: f32,
    /// What the first-person hand shows (None = bare arm) and how far it is
    /// raised (1 = in place, 0 = out of view below the screen).
    pub shown_item: Option<Block>,
    pub equip: f32,
    /// Smoothed FOV multipliers for sprinting and zooming.
    pub sprint_fov: f32,
    pub zoom_fov: f32,
    /// How many times the view is magnified while zooming.
    pub zoom_level: f32,
    /// Light on the player and what they hold, eased so shade sets in and
    /// lifts gradually instead of flickering block by block.
    pub player_light: f32,
    /// Seconds the camera has been under water, for vanilla's water vision.
    pub underwater_seconds: f32,
}

/// Like vanilla, spawn on dry land: search outwards from the origin in a
/// spiral of 16-block steps for a column above sea level.
fn find_spawn(generator: &Generator) -> Vec3 {
    let mut pos = glam::IVec2::ZERO;
    let mut dir = glam::IVec2::X;
    let mut leg = 1;
    for i in 0..4096 {
        let (x, z) = (pos.x * 16 + 8, pos.y * 16 + 8);
        let height = generator.surface_height(x, z);
        if height > SEA_LEVEL {
            return Vec3::new(x as f32 + 0.5, height as f32 + 1.0, z as f32 + 0.5);
        }
        pos += dir;
        if (i + 1) % leg == 0 {
            dir = glam::IVec2::new(-dir.y, dir.x);
            if dir.y == 0 {
                leg += 1;
            }
        }
    }
    Vec3::new(8.5, SEA_LEVEL as f32 + 1.0, 8.5)
}

/// Lets testers start at a given time of day (0 = sunrise, 0.5 = sunset).
fn time_override() -> Option<f32> {
    std::env::var("RSCRAFT_TIME").ok().and_then(|t| t.parse().ok())
}

impl Session {
    pub fn backdrop(layers: &Arc<BlockLayers>, options: &Options, aspect: f32) -> Self {
        let generator = Arc::new(Generator::new(MENU_SEED));
        let height = generator.surface_height(0, 0).max(SEA_LEVEL) as f32;
        let mut camera = Camera::new(aspect, Vec3::new(0.5, height + 24.0, 0.5), 0.0, -0.15);
        camera.set_fov(options.fov);
        // The backdrop doesn't need the full render distance.
        let chunks = ChunkManager::new(generator, layers.clone(), options.render_distance.min(10), Edits::new());
        Self {
            chunks,
            camera,
            day: DayCycle { time: time_override().unwrap_or(0.15) },
            world: None,
            player: None,
            game_mode: GameMode::Survival,
            previous_game_mode: None,
            inventory: Inventory::default(),
            cursor: None,
            crafting: Grid::default(),
            items: DroppedItems::default(),
            particles: Particles::default(),
            hit_particle_timer: 0.0,
            breaking: None,
            break_cooldown: 0.0,
            place_cooldown: 0.0,
            swing: 0.0,
            shown_item: None,
            equip: 1.0,
            sprint_fov: 1.0,
            zoom_fov: 1.0,
            zoom_level: DEFAULT_ZOOM,
            player_light: 1.0,
            underwater_seconds: 0.0,
        }
    }

    pub fn for_world(entry: WorldEntry, layers: &Arc<BlockLayers>, options: &Options, aspect: f32) -> Self {
        let generator = Arc::new(Generator::new(entry.save.generator_seed()));
        let (player, yaw, pitch) = match &entry.save.player {
            // Saves store the eye position.
            Some(p) => (PlayerController::new(Vec3::from(p.position) - Vec3::Y * EYE_HEIGHT, p.flying), p.yaw, p.pitch),
            None => (PlayerController::new(find_spawn(&generator), false), 0.0, 0.0),
        };
        let mut camera = Camera::new(aspect, player.eye(), yaw, pitch);
        camera.set_fov(options.fov);
        let inventory = entry.save.player.as_ref().map(PlayerSave::load_inventory).unwrap_or_default();
        let edits = entry.read_edits();
        let (game_mode, previous_game_mode) = (entry.save.game_mode, entry.save.previous_game_mode);
        let mut session = Self {
            chunks: ChunkManager::new(generator, layers.clone(), options.render_distance, edits),
            day: DayCycle { time: time_override().unwrap_or(entry.save.time_of_day) },
            camera,
            world: Some(entry),
            player: Some(player),
            game_mode,
            previous_game_mode,
            inventory,
            cursor: None,
            crafting: Grid::default(),
            items: DroppedItems::default(),
            particles: Particles::default(),
            hit_particle_timer: 0.0,
            breaking: None,
            break_cooldown: 0.0,
            place_cooldown: 0.0,
            swing: 0.0,
            shown_item: None,
            equip: 1.0,
            sprint_fov: 1.0,
            zoom_fov: 1.0,
            zoom_level: DEFAULT_ZOOM,
            player_light: 1.0,
            underwater_seconds: 0.0,
        };
        session.apply_game_mode();
        session
    }

    pub fn save(&mut self) {
        let (Some(world), Some(player)) = (&mut self.world, &self.player) else { return };
        let mut save = PlayerSave {
            position: (player.position + Vec3::Y * EYE_HEIGHT).to_array(),
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            flying: player.flying,
            inventory: Vec::new(),
            selected_slot: 0,
        };
        save.store_inventory(&self.inventory);
        world.save.player = Some(save);
        if let Err(err) = world.write_edits(self.chunks.edits()) {
            log::error!("{err:#}");
        }
        world.save.time_of_day = self.day.time;
        world.save.game_mode = self.game_mode;
        world.save.previous_game_mode = self.previous_game_mode;
        world.save.last_played = saves::now();
        match world.write() {
            Ok(()) => log::info!("saved world {}", world.save.name),
            Err(err) => log::error!("{err:#}"),
        }
    }

    /// Advances the player one frame and moves the camera to their eyes.
    pub fn update_player(&mut self, dt: f32, controls: Option<Controls>, fov: f32, zoom: bool) {
        let Some(player) = &mut self.player else { return };
        if let Some(controls) = controls {
            let chunks = &self.chunks;
            player.update(dt, controls, self.camera.yaw, &|pos| chunks.block_at(pos));
        }
        // View bobbing, like vanilla: sway sideways and dip at every step.
        let (walk, bob) = player.bobbing();
        let bob = bob * VIEW_BOB_STRENGTH;
        let phase = walk * std::f32::consts::PI;
        let offset = self.camera.right() * (phase.sin() * bob * 0.5) - Vec3::Y * (phase.cos() * bob).abs();
        self.camera.position = player.eye() + offset;
        // Plus vanilla's small tilt: a sideways roll (3°) and a nod (5°) per step.
        self.camera.bob_roll = (phase.sin() * bob * 3.0).to_radians();
        self.camera.bob_pitch = ((phase - 0.2).cos() * bob * 5.0).abs().to_radians();

        let smoothing = (dt * 12.0).min(1.0);
        let sprint = if player.sprinting { SPRINT_FOV } else { 1.0 };
        self.sprint_fov += (sprint - self.sprint_fov) * smoothing;
        if !zoom {
            self.zoom_level = DEFAULT_ZOOM;
        }
        let zoom_target = if zoom { 1.0 / self.zoom_level } else { 1.0 };
        self.zoom_fov += (zoom_target - self.zoom_fov) * smoothing;
        self.camera.set_fov(fov * self.sprint_fov * self.zoom_fov);
        // Switching items lowers the hand out of view, swaps what it shows,
        // then raises it again, like vanilla (0.4 of the way per tick).
        // The darkest of the blocks the player's body is in (feet, middle,
        // eyes), so standing half under a ledge or canopy already shades
        // them; darkening is quick, brightening slower, so shade lingers.
        let feet = player.position;
        let target =
            [0.1, 0.9, EYE_HEIGHT - 0.1].map(|dy| self.light_at(feet + Vec3::Y * dy)).into_iter().fold(1.0, f32::min);
        let rate = if target < self.player_light { 8.0 } else { 2.5 };
        self.player_light += (target - self.player_light) * (dt * rate).min(1.0);

        let held = self.inventory.selected_stack().map(|s| s.block);
        let target = if held == self.shown_item { 1.0 } else { 0.0 };
        let step = EQUIP_SPEED * dt;
        self.equip += (target - self.equip).clamp(-step, step);
        if self.equip < 0.1 {
            self.shown_item = held;
        }
        if self.swing > 0.0 {
            self.swing += dt / SWING_SECONDS;
            if self.swing >= 1.0 {
                self.swing = 0.0;
            }
        }
    }

    /// Switches game mode, remembering the old one for F3+F4.
    pub fn set_game_mode(&mut self, mode: GameMode) {
        if mode != self.game_mode {
            self.previous_game_mode = Some(self.game_mode);
            self.game_mode = mode;
        }
        self.apply_game_mode();
    }

    /// Tells the player's movement what the game mode allows.
    pub fn apply_game_mode(&mut self) {
        let mode = self.game_mode;
        if let Some(player) = &mut self.player {
            player.may_fly = mode.may_fly();
            player.noclip = mode.is_spectator();
            if !mode.may_fly() {
                player.flying = false;
            }
        }
    }

    /// Breaking and placing blocks with the mouse buttons held. Returns the
    /// block under the crosshair and its breaking progress.
    pub fn interact(&mut self, dt: f32, attack: bool, use_item: bool) -> Option<(Hit, f32)> {
        let eye = self.player.as_ref()?.eye();
        // Spectators can't touch the world; no outline either.
        if self.game_mode.is_spectator() {
            self.breaking = None;
            return None;
        }
        self.break_cooldown = (self.break_cooldown - dt).max(0.0);
        self.place_cooldown = (self.place_cooldown - dt).max(0.0);
        let chunks = &self.chunks;
        let Some(hit) = raycast(eye, self.camera.forward(), REACH, |pos| chunks.block_at(pos)) else {
            self.breaking = None;
            return None;
        };

        let target = chunks.block_at(hit.block).unwrap_or(Block::Air);
        // Like vanilla, the arm keeps swinging while attacking a block, even
        // in the pause between one block breaking and the next.
        if attack && self.swing == 0.0 {
            self.start_swing();
        }
        let mode = self.game_mode;
        let attack = attack && mode.may_build();
        // Creative breaks anything at once, bedrock included, dropping nothing.
        if mode.is_creative() {
            if attack && self.break_cooldown == 0.0 && target != Block::Air {
                self.chunks.set_block(hit.block, Block::Air);
                self.particles.block_broken(hit.block, target, self.chunks.layers());
                self.break_cooldown = 0.25;
            }
            self.breaking = None;
        }
        match (attack && !mode.is_creative(), target.break_seconds()) {
            (true, Some(seconds)) if self.break_cooldown == 0.0 => {
                let progress = match self.breaking {
                    Some((pos, progress)) if pos == hit.block => progress,
                    _ => 0.0,
                } + dt / seconds.max(0.05);
                if progress >= 1.0 {
                    self.chunks.set_block(hit.block, Block::Air);
                    self.particles.block_broken(hit.block, target, self.chunks.layers());
                    if let Some(item) = target.drop() {
                        self.items.spawn_at_block(hit.block, item, 1);
                    }
                    self.breaking = None;
                    // Vanilla waits 5 ticks before starting on the next block.
                    self.break_cooldown = 0.25;
                } else {
                    self.breaking = Some((hit.block, progress));
                    // Chips fly off the face being hit, about one per tick.
                    self.hit_particle_timer -= dt;
                    if self.hit_particle_timer <= 0.0 {
                        self.hit_particle_timer += TICK_SECONDS;
                        self.particles.block_hit(hit.block, hit.normal, target, self.chunks.layers());
                    }
                }
            }
            _ => self.breaking = None,
        }

        if use_item && mode.may_build() && self.place_cooldown == 0.0 && hit.normal != IVec3::ZERO {
            let pos = hit.block + hit.normal;
            let free = matches!(self.chunks.block_at(pos), Some(Block::Air | Block::Water));
            let blocked = self.player.as_ref().is_some_and(|p| p.intersects_block(pos));
            if free
                && !blocked
                && let Some(stack) = self.inventory.selected_stack()
            {
                self.chunks.set_block(pos, stack.block);
                // Creative placing doesn't use up items.
                if !mode.is_creative() {
                    self.inventory.take_selected();
                }
                self.start_swing();
                // Vanilla repeats placement every 4 ticks while held.
                self.place_cooldown = 0.2;
            }
        }
        Some((hit, self.breaking.map_or(0.0, |(_, p)| p)))
    }

    /// Drops one of the selected item, or the whole stack with Ctrl (Q).
    pub fn throw_selected(&mut self, whole_stack: bool) {
        let Some(player) = &self.player else { return };
        let Some(stack) = self.inventory.selected_stack() else { return };
        let count = if whole_stack { stack.count } else { 1 };
        for _ in 0..count {
            self.inventory.take_selected();
        }
        self.items.throw(player.eye(), self.camera.forward(), stack.block, count);
    }

    /// Puts the stack held by the mouse back, dropping what doesn't fit.
    /// Closing the inventory puts the stack on the cursor and whatever is
    /// in the crafting grid back, dropping what doesn't fit.
    pub fn return_cursor(&mut self) {
        let stacks: Vec<ItemStack> =
            self.cursor.take().into_iter().chain(self.crafting.iter_mut().filter_map(Option::take)).collect();
        for stack in stacks {
            let left = self.inventory.add(stack.block, stack.count);
            if left > 0
                && let Some(player) = &self.player
            {
                self.items.throw(player.eye(), self.camera.forward(), stack.block, left);
            }
        }
    }

    /// The player in the inventory screen, turning towards the mouse like
    /// vanilla's renderEntityInInventoryFollowsMouse.
    pub fn preview(&self, rect: Rect, mouse: glam::Vec2, gui_scale: f32, layers: &BlockLayers) -> Preview {
        let center = glam::Vec2::new(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        let (turn, tilt) = (((center.x - mouse.x) / 40.0).atan(), ((center.y - mouse.y) / 40.0).atan());
        let pose = PlayerPose {
            feet: Vec3::new(0.0, 0.0, -PREVIEW_DEPTH),
            // Facing the viewer, turned 20° (body) and 40° (head) per radian.
            yaw: std::f32::consts::PI + turn * 20f32.to_radians(),
            head_yaw: turn * 20f32.to_radians(),
            head_pitch: tilt * 20f32.to_radians(),
            swing: 0.0,
            light: 1.0,
            swim: 0.0,
            limb_phase: 0.0,
        };
        let mut held = MeshData::default();
        if let Some(stack) = self.inventory.selected_stack() {
            held.push_block(stack.block, layers, player::held_block_third_person(&pose), 1.0);
        }
        Preview { rect, gui_scale, body: player::build_mesh(&pose), held }
    }

    /// The underwater look when the camera is in water: vanilla's dark blue
    /// fog that clears up from 24 to 96 blocks over 30 seconds as the eyes
    /// adjust, plus the overlay texture's offset.
    pub fn underwater(&mut self, camera: &Camera, dt: f32) -> Option<Underwater> {
        let eye = camera.position;
        let block = eye.floor().as_ivec3();
        let in_water = self.chunks.block_at(block) == Some(Block::Water)
            && (self.chunks.block_at(block + IVec3::Y) == Some(Block::Water) || eye.y.fract() < 8.0 / 9.0);
        if !in_water || self.world.is_none() {
            self.underwater_seconds = 0.0;
            return None;
        }
        self.underwater_seconds += dt;
        let ticks = self.underwater_seconds * 20.0;
        let vision = (ticks / 100.0).clamp(0.0, 1.0) * 0.6 + ((ticks - 100.0) / 500.0).clamp(0.0, 1.0) * 0.4;
        // Plains' water fog colour; as the eyes adjust it brightens towards
        // full saturation, like vanilla's FogRenderer.
        let base = Vec3::new(5.0, 5.0, 51.0) / 255.0;
        let color = base.lerp(base / base.max_element(), vision);
        Some(Underwater {
            fog_end: 96.0 * vision.max(0.25),
            color,
            overlay_offset: [-camera.yaw.to_degrees() / 64.0, -camera.pitch.to_degrees() / 64.0],
            light: self.light_at(eye),
        })
    }

    /// One frame of the world running: dropped items fall and get picked
    /// up, splashes and particles play.
    pub fn update_world(&mut self, dt: f32) {
        // Spectators don't pick items up.
        let feet = self.player.as_ref().filter(|_| !self.game_mode.is_spectator()).map(PlayerController::feet);
        let chunks = &self.chunks;
        self.items.update(dt, feet, &mut self.inventory, &|pos| chunks.block_at(pos));
        if let Some((at, surface, velocity)) = self.player.as_mut().and_then(|p| p.splash.take()) {
            self.particles.water_splash(chunks.layers(), at, surface, 0.3, velocity);
        }
        self.particles.update(dt, self.camera.position, chunks.layers(), &|pos| chunks.block_at(pos));
    }

    /// The meshes of everything in the world besides terrain, as seen from
    /// `perspective` this frame.
    pub fn scene(&self, perspective: Perspective, layers: &BlockLayers, seconds: f32) -> Scene {
        let spectator = self.game_mode.is_spectator();
        let visible_body = perspective != Perspective::FirstPerson && !spectator;
        let feet = self.player.as_ref().map(PlayerController::feet);
        let light = |p: Vec3| self.light_at(p);
        let mut scene = Scene {
            player: visible_body.then(|| self.pose().map(|pose| player::build_mesh(&pose))).flatten(),
            hand: None,
            items: item_render::build_mesh(&self.items, layers, seconds, feet, light),
            particles: {
                let camera = self.camera.for_perspective(perspective);
                let up = camera.right().cross(camera.forward()).normalize();
                self.particles.build_mesh(camera.right(), up, light)
            },
            held: MeshData::default(),
            shadows: item_render::shadow_casters(&self.items).collect(),
        };

        // Like vanilla, a held block replaces the bare hand in first person
        // (showing what the switching animation says it holds) and sits in
        // Steve's right hand in third person.
        if let (Some(hand), Some(pose), false) = (self.hand(), self.pose(), spectator) {
            if perspective == Perspective::FirstPerson {
                match self.shown_item {
                    Some(block) => {
                        scene.held.push_block(block, layers, player::held_block_first_person(&hand), hand.light)
                    }
                    None => scene.hand = Some(player::build_hand_mesh(&hand)),
                }
            } else if let Some(stack) = self.inventory.selected_stack() {
                scene.items.push_block(stack.block, layers, player::held_block_third_person(&pose), pose.light);
            }
        }
        if visible_body && let Some(feet) = feet {
            scene.shadows.push(Caster { feet, radius: 0.5, strength: 1.0 });
        }
        scene
    }

    /// Mouse wheel while zooming: positive notches zoom in.
    pub fn scroll_zoom(&mut self, notches: f32) {
        self.zoom_level = (self.zoom_level * ZOOM_STEP.powf(notches)).clamp(MIN_ZOOM, MAX_ZOOM);
    }

    pub fn start_swing(&mut self) {
        if self.swing == 0.0 || self.swing > 0.5 {
            self.swing = f32::EPSILON;
        }
    }

    /// Sky light level (0..1) at a position, for lighting entities.
    pub fn light_at(&self, pos: Vec3) -> f32 {
        self.chunks.sky_light_at(pos.floor().as_ivec3()) as f32 / rscraft_world::MAX_LIGHT as f32
    }

    pub fn pose(&self) -> Option<PlayerPose> {
        let player = self.player.as_ref()?;
        let (walk, bob) = player.bobbing();
        Some(PlayerPose {
            feet: player.feet(),
            yaw: self.camera.yaw,
            head_pitch: self.camera.pitch,
            swing: (walk * 3.6).sin() * (bob * 9.0).min(0.9),
            light: self.player_light,
            swim: player.swim_amount(),
            head_yaw: 0.0,
            limb_phase: walk * 3.6,
        })
    }

    pub fn hand(&self) -> Option<HandPose> {
        let player = self.player.as_ref()?;
        let (walk, bob) = player.bobbing();
        let phase = walk * std::f32::consts::PI;
        Some(HandPose {
            eye: player.eye(),
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            swing: self.swing,
            bob: Vec3::new(phase.sin() * bob * 0.5, -(phase.cos() * bob).abs(), 0.0),
            light: self.player_light,
            equip: self.equip,
            aspect: self.camera.aspect(),
            fov_y: self.camera.fov_y() / self.zoom_fov,
        })
    }
}
