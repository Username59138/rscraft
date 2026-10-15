use std::sync::Arc;
use std::time::Instant;

use glam::Vec3;
use glam::IVec3;
use rscraft_world::{Block, Generator, SEA_LEVEL};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::camera::{Camera, Perspective};
use crate::chunks::{ChunkManager, Edits};
use crate::debug::{self, DebugInfo};
use crate::hud;
use crate::input::Input;
use crate::inventory::{Inventory, ItemStack};
use crate::inventory_screen::{self, InventoryAction};
use crate::items::Items;
use crate::particles::Particles;
use crate::mesh::MeshData;
use crate::movement::{Controls, EYE_HEIGHT, PlayerController, TICK_SECONDS};
use crate::player::{self, HandPose, PlayerPose};
use crate::raycast::{Hit, REACH, raycast};
use crate::crafting::Grid;
use crate::renderer::{EntityImages, PREVIEW_DEPTH, Preview, Renderer, Underwater};
use crate::shadows::Caster;
use crate::selection::Selection;
use crate::saves::{self, Options, PlayerSave, WorldEntry};
use crate::screens::{self, Action, Screen};
use crate::sky::{DayCycle, SkyImages};
use crate::texture::{BlockImages, BlockLayers};
use crate::ui::{GuiAtlas, Rect, Sprite, Ui};

/// World shown behind the title screen.
const MENU_SEED: u32 = 1337;
/// Holding T makes time run this much faster.
const TIME_FAST_FORWARD: f32 = 120.0;
/// Columns around the spawn that must be ready before a world opens.
const SPAWN_RADIUS: i32 = 2;
const DOUBLE_CLICK_SECONDS: f32 = 0.35;

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

/// A loaded world: the title screen backdrop, or a saved world being played.
struct Session {
    chunks: ChunkManager,
    camera: Camera,
    day: DayCycle,
    world: Option<WorldEntry>,
    /// The player; None for the title backdrop.
    player: Option<PlayerController>,
    inventory: Inventory,
    /// The stack held by the mouse in the inventory screen.
    cursor: Option<ItemStack>,
    /// The inventory's 2×2 crafting grid.
    crafting: Grid,
    items: Items,
    particles: Particles,
    hit_particle_timer: f32,
    /// Block being broken and how far along it is (0..1).
    breaking: Option<(IVec3, f32)>,
    /// Delays between breaking or placing blocks while a button is held.
    break_cooldown: f32,
    place_cooldown: f32,
    /// Arm swing progress, 0 when idle.
    swing: f32,
    /// What the first-person hand shows (None = bare arm) and how far it is
    /// raised (1 = in place, 0 = out of view below the screen).
    shown_item: Option<Block>,
    equip: f32,
    /// Smoothed FOV multipliers for sprinting and zooming.
    sprint_fov: f32,
    zoom_fov: f32,
    /// How many times the view is magnified while zooming.
    zoom_level: f32,
    /// Light on the player and what they hold, eased so shade sets in and
    /// lifts gradually instead of flickering block by block.
    player_light: f32,
    /// Seconds the camera has been under water, for vanilla's water vision.
    underwater_seconds: f32,
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
    fn backdrop(layers: &Arc<BlockLayers>, options: &Options, aspect: f32) -> Self {
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
            inventory: Inventory::default(),
            cursor: None,
            crafting: Grid::default(),
            items: Items::default(),
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

    fn for_world(entry: WorldEntry, layers: &Arc<BlockLayers>, options: &Options, aspect: f32) -> Self {
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
        Self {
            chunks: ChunkManager::new(generator, layers.clone(), options.render_distance, edits),
            day: DayCycle { time: time_override().unwrap_or(entry.save.time_of_day) },
            camera,
            world: Some(entry),
            player: Some(player),
            inventory,
            cursor: None,
            crafting: Grid::default(),
            items: Items::default(),
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

    fn save(&mut self) {
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
        world.save.last_played = saves::now();
        match world.write() {
            Ok(()) => log::info!("saved world {}", world.save.name),
            Err(err) => log::error!("{err:#}"),
        }
    }

    /// Advances the player one frame and moves the camera to their eyes.
    fn update_player(&mut self, dt: f32, controls: Option<Controls>, fov: f32, zoom: bool) {
        let Some(player) = &mut self.player else { return };
        if let Some(controls) = controls {
            let chunks = &self.chunks;
            player.update(dt, controls, self.camera.yaw, &|pos| chunks.block_at(pos));
        }
        // View bobbing, like vanilla: sway sideways and dip at every step.
        let (walk, bob) = player.bobbing();
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
        let target = [0.1, 0.9, EYE_HEIGHT - 0.1]
            .map(|dy| self.light_at(feet + Vec3::Y * dy))
            .into_iter()
            .fold(1.0, f32::min);
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

    /// Breaking and placing blocks with the mouse buttons held. Returns the
    /// block under the crosshair and its breaking progress.
    fn interact(&mut self, dt: f32, attack: bool, use_item: bool) -> Option<(Hit, f32)> {
        let eye = self.player.as_ref()?.eye();
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
        match (attack, target.break_seconds()) {
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

        if use_item && self.place_cooldown == 0.0 && hit.normal != IVec3::ZERO {
            let pos = hit.block + hit.normal;
            let free = matches!(self.chunks.block_at(pos), Some(Block::Air | Block::Water));
            let blocked = self.player.as_ref().is_some_and(|p| p.intersects_block(pos));
            if free && !blocked
                && let Some(stack) = self.inventory.selected_stack()
            {
                self.chunks.set_block(pos, stack.block);
                self.inventory.take_selected();
                self.start_swing();
                // Vanilla repeats placement every 4 ticks while held.
                self.place_cooldown = 0.2;
            }
        }
        Some((hit, self.breaking.map_or(0.0, |(_, p)| p)))
    }

    /// Drops one of the selected item, or the whole stack with Ctrl (Q).
    fn throw_selected(&mut self, whole_stack: bool) {
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
    fn return_cursor(&mut self) {
        let stacks: Vec<ItemStack> = self.cursor.take().into_iter().chain(self.crafting.iter_mut().filter_map(Option::take)).collect();
        for stack in stacks {
            let left = self.inventory.add(stack.block, stack.count);
            if left > 0 && let Some(player) = &self.player {
                self.items.throw(player.eye(), self.camera.forward(), stack.block, left);
            }
        }
    }

    /// The player in the inventory screen, turning towards the mouse like
    /// vanilla's renderEntityInInventoryFollowsMouse.
    fn preview(&self, rect: Rect, mouse: glam::Vec2, gui_scale: f32, layers: &BlockLayers) -> Preview {
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
    fn underwater(&mut self, camera: &Camera, dt: f32) -> Option<Underwater> {
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

    /// Mouse wheel while zooming: positive notches zoom in.
    fn scroll_zoom(&mut self, notches: f32) {
        self.zoom_level = (self.zoom_level * ZOOM_STEP.powf(notches)).clamp(MIN_ZOOM, MAX_ZOOM);
    }

    fn start_swing(&mut self) {
        if self.swing == 0.0 || self.swing > 0.5 {
            self.swing = f32::EPSILON;
        }
    }

    /// Sky light level (0..1) at a position, for lighting entities.
    fn light_at(&self, pos: Vec3) -> f32 {
        self.chunks.sky_light_at(pos.floor().as_ivec3()) as f32 / rscraft_world::MAX_LIGHT as f32
    }

    fn pose(&self) -> Option<PlayerPose> {
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

    fn hand(&self) -> Option<HandPose> {
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

/// Everything that only exists while the window is alive.
struct State {
    window: Arc<Window>,
    renderer: Renderer,
    ui: Ui,
    layers: Arc<BlockLayers>,
    session: Session,
    screen: Screen,
    options: Options,
    input: Input,
    perspective: Perspective,
    show_debug: bool,
    mouse_grabbed: bool,
    quit: bool,
    started: Instant,
    last_frame: Instant,
    last_click: Instant,
    fps_timer: Instant,
    frames: u32,
    fps: u32,
}

pub struct App {
    images: BlockImages,
    sky_images: SkyImages,
    atlas: Option<GuiAtlas>,
    entity_images: EntityImages,
    state: Option<State>,
}

impl App {
    pub fn new(images: BlockImages, sky_images: SkyImages, atlas: GuiAtlas, entity_images: EntityImages) -> Self {
        Self { images, sky_images, atlas: Some(atlas), entity_images, state: None }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Some(atlas) = self.atlas.take() else { return };

        let attrs = Window::default_attributes().with_title("rscraft").with_inner_size(PhysicalSize::new(1280, 720));
        let window = Arc::new(event_loop.create_window(attrs).expect("failed to create window"));
        let size = window.inner_size();
        let aspect = size.width as f32 / size.height.max(1) as f32;

        let options = Options::load();
        let renderer = pollster::block_on(Renderer::new(
            window.clone(),
            event_loop.owned_display_handle(),
            &self.images,
            &self.sky_images,
            &atlas,
            &self.entity_images,
            options.render_distance,
        ));
        let layers = Arc::new(self.images.layers.clone());
        let session = Session::backdrop(&layers, &options, aspect);

        self.state = Some(State {
            window,
            renderer,
            ui: Ui::new(atlas),
            layers,
            session,
            screen: Screen::Title,
            options,
            input: Input::default(),
            perspective: Perspective::FirstPerson,
            show_debug: false,
            mouse_grabbed: false,
            quit: false,
            started: Instant::now(),
            last_frame: Instant::now(),
            last_click: Instant::now(),
            fps_timer: Instant::now(),
            frames: 0,
            fps: 0,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else { return };

        match event {
            WindowEvent::CloseRequested => {
                state.session.save();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                state.renderer.resize(size);
                state.session.camera.set_aspect(size.width as f32 / size.height.max(1) as f32);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                let pressed = event.state == ElementState::Pressed;
                state.input.set_key(code, pressed);
                if pressed {
                    state.key_pressed(code, event.text.as_deref());
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                state.ui.input.mouse = glam::Vec2::new(position.x as f32, position.y as f32) / state.ui.scale;
            }
            WindowEvent::MouseInput { state: button_state, button: MouseButton::Right, .. } => {
                let pressed = button_state == ElementState::Pressed;
                state.input.use_item = pressed && state.playing() && state.mouse_grabbed;
                if pressed && !state.playing() {
                    state.ui.input.right_clicked = true;
                }
                if pressed {
                    // A fresh click places right away; holding repeats.
                    state.session.place_cooldown = 0.0;
                }
            }
            WindowEvent::MouseInput { state: button_state, button: MouseButton::Left, .. } => {
                let pressed = button_state == ElementState::Pressed;
                state.ui.input.mouse_down = pressed;
                state.input.attack = pressed && state.playing() && state.mouse_grabbed;
                if !pressed {
                    return;
                }
                if state.playing() {
                    if state.mouse_grabbed {
                        state.session.start_swing();
                    } else {
                        state.set_mouse_grab(true);
                    }
                } else {
                    state.ui.input.clicked = true;
                    state.ui.input.double_clicked = state.last_click.elapsed().as_secs_f32() < DOUBLE_CLICK_SECONDS;
                    state.last_click = Instant::now();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let notches = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 20.0,
                };
                if state.playing() && state.input.is_pressed(KeyCode::KeyC) {
                    state.session.scroll_zoom(notches);
                } else if state.playing() {
                    state.session.inventory.scroll(notches.round() as i32);
                } else {
                    state.ui.input.scroll += notches;
                }
            }
            WindowEvent::Focused(false) => {
                state.input.clear();
                if state.playing() {
                    state.open(Screen::Pause);
                }
            }
            WindowEvent::RedrawRequested => {
                state.frame();
                if state.quit {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        let Some(state) = self.state.as_mut() else { return };
        if let DeviceEvent::MouseMotion { delta } = event
            && state.mouse_grabbed
        {
            state.input.add_mouse_delta(delta.0 as f32, delta.1 as f32);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = self.state.as_ref() {
            state.window.request_redraw();
        }
    }
}

impl State {
    fn in_world(&self) -> bool {
        self.session.world.is_some()
    }

    /// In a world with no menu open.
    fn playing(&self) -> bool {
        self.in_world() && matches!(self.screen, Screen::None)
    }

    /// Whether the world keeps running. Like vanilla singleplayer, only the
    /// pause menu (and the options opened from it) stops it; the inventory
    /// doesn't.
    fn world_running(&self) -> bool {
        self.in_world() && !matches!(self.screen, Screen::Pause | Screen::Options { from_pause: true } | Screen::Loading { .. })
    }

    fn key_pressed(&mut self, code: KeyCode, text: Option<&str>) {
        if self.playing() {
            match code {
                KeyCode::Escape => self.open(Screen::Pause),
                KeyCode::F3 => self.show_debug = !self.show_debug,
                KeyCode::F5 => self.perspective = self.perspective.next(),
                KeyCode::KeyE => self.open(Screen::Inventory),
                KeyCode::KeyQ => {
                    let whole = self.input.is_pressed(KeyCode::ControlLeft);
                    self.session.throw_selected(whole);
                }
                KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 | KeyCode::Digit5
                | KeyCode::Digit6 | KeyCode::Digit7 | KeyCode::Digit8 | KeyCode::Digit9 => {
                    self.session.inventory.selected = code as usize - KeyCode::Digit1 as usize;
                }
                _ => {}
            }
            return;
        }
        // Menus handle their own keys.
        self.ui.input.keys.push(code);
        if let Some(text) = text {
            self.ui.input.typed.push_str(text);
        }
    }

    fn controls(&self) -> Controls {
        let axis = |plus, minus| self.input.is_pressed(plus) as i32 as f32 - self.input.is_pressed(minus) as i32 as f32;
        Controls {
            forward: axis(KeyCode::KeyW, KeyCode::KeyS),
            strafe: axis(KeyCode::KeyD, KeyCode::KeyA),
            jump: self.input.is_pressed(KeyCode::Space),
            sneak: self.input.is_pressed(KeyCode::ShiftLeft),
            sprint: self.input.is_pressed(KeyCode::ControlLeft),
            look_y: self.session.camera.forward().y,
        }
    }

    fn open(&mut self, screen: Screen) {
        let playing = matches!(screen, Screen::None);
        self.screen = screen;
        self.input.clear();
        self.set_mouse_grab(playing);
    }

    fn gui_scale(&self) -> u32 {
        let (w, h) = self.renderer.size();
        let max = Ui::auto_scale(w, h, u32::MAX);
        if self.options.gui_scale == 0 { max } else { self.options.gui_scale.min(max) }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let seconds = (now - self.started).as_secs_f32();

        // Simulation: the backdrop slowly turns; the world runs unless paused,
        // but only takes the player's input when no screen is open.
        let mut controls = None;
        if self.world_running() {
            let time_speed = if self.playing() && self.input.is_pressed(KeyCode::KeyT) { TIME_FAST_FORWARD } else { 1.0 };
            self.session.day.advance(dt, time_speed);
            controls = Some(Controls { look_y: self.session.camera.forward().y, ..Controls::default() });
        }
        if self.playing() {
            // Turning slows down with the zoom so the view doesn't jump around.
            let zoom_scale = self.session.zoom_fov;
            self.session.camera.look(&mut self.input, self.options.sensitivity * zoom_scale);
            controls = Some(self.controls());
        } else if !self.in_world() {
            self.session.camera.yaw += dt * 0.03;
        }
        let zoom = self.playing() && self.input.is_pressed(KeyCode::KeyC);
        self.session.update_player(dt, controls, self.options.fov, zoom);
        let target = if self.playing() {
            self.session.interact(dt, self.input.attack, self.input.use_item)
        } else {
            self.session.breaking = None;
            None
        };

        self.session.chunks.update(self.session.camera.position, &mut self.renderer);
        // Items and particles run with the world, inventory open or not.
        if self.world_running() {
            let session = &mut self.session;
            let feet = session.player.as_ref().map(PlayerController::feet);
            let chunks = &session.chunks;
            session.items.update(dt, feet, &mut session.inventory, &|pos| chunks.block_at(pos));
            if let Some((at, surface, velocity)) = session.player.as_mut().and_then(|p| p.splash.take()) {
                session.particles.water_splash(chunks.layers(), at, surface, 0.3, velocity);
            }
            session.particles.update(dt, session.camera.position, chunks.layers(), &|pos| chunks.block_at(pos));
        }
        if let Screen::Loading { progress } = &mut self.screen {
            *progress = self.session.chunks.progress(self.session.camera.position, SPAWN_RADIUS);
            if *progress >= 1.0 {
                let chunks = &self.session.chunks;
                if let Some(player) = &mut self.session.player {
                    player.unstuck(&|pos| chunks.block_at(pos));
                }
                self.open(Screen::None);
            }
        }

        // Interface.
        let (width, height) = self.renderer.size();
        let scale = self.gui_scale();
        self.ui.begin(width, height, scale, dt);
        if self.playing() && self.perspective == Perspective::FirstPerson {
            let (cx, cy) = ((self.ui.width / 2.0).floor(), (self.ui.height / 2.0).floor());
            self.ui.sprite_inverted(Sprite::Crosshair, Rect::new(cx - 7.0, cy - 7.0, 15.0, 15.0));
        }
        if self.in_world() && !matches!(self.screen, Screen::Loading { .. }) {
            hud::hotbar(&mut self.ui, &self.session.inventory);
        }
        if self.show_debug && self.in_world() && !matches!(self.screen, Screen::Loading { .. }) {
            let camera = &self.session.camera;
            let info = DebugInfo {
                fps: self.fps,
                drawn_sections: self.renderer.drawn_sections,
                total_sections: self.renderer.total_sections(),
                loaded_columns: self.session.chunks.loaded_columns(),
                position: camera.position,
                yaw: camera.yaw,
                pitch: camera.pitch,
                time_of_day: self.session.day.time,
                seed: self.session.world.as_ref().map_or(0, |w| w.save.seed),
                render_distance: self.options.render_distance,
                gpu: &self.renderer.gpu,
                screen: (width, height),
            };
            debug::draw(&mut self.ui, &info);
        }
        if matches!(self.screen, Screen::Inventory) {
            let shift = self.input.is_pressed(KeyCode::ShiftLeft) || self.input.is_pressed(KeyCode::ShiftRight);
            let session = &mut self.session;
            let (inventory, crafting, cursor) = (&mut session.inventory, &mut session.crafting, &mut session.cursor);
            if let InventoryAction::Close = inventory_screen::draw(&mut self.ui, inventory, crafting, cursor, shift) {
                session.return_cursor();
                self.open(Screen::None);
            }
        }
        let action = screens::draw(&mut self.screen, &mut self.ui, &mut self.options);
        self.apply(action);

        // World.
        let perspective = if self.in_world() { self.perspective } else { Perspective::FirstPerson };
        let player_mesh = (perspective != Perspective::FirstPerson)
            .then(|| self.session.pose().map(|pose| player::build_mesh(&pose)))
            .flatten();
        // Like vanilla, a held block replaces the bare hand in first person
        // and sits in Steve's right hand in third person.
        let held_block = self.in_world().then(|| self.session.inventory.selected_stack()).flatten().map(|s| s.block);
        // First person shows what the switching animation says it holds.
        let shown_block = self.session.shown_item;
        let player_feet = self.session.player.as_ref().map(PlayerController::feet);
        let mut world_items =
            self.session.items.build_mesh(&self.layers, seconds, player_feet, |p| self.session.light_at(p));
        let view_camera = self.session.camera.for_perspective(perspective);
        let particle_mesh = self.session.particles.build_mesh(
            view_camera.right(),
            view_camera.right().cross(view_camera.forward()).normalize(),
            |p| self.session.light_at(p),
        );
        let mut held_mesh = MeshData::default();
        let mut hand_mesh = None;
        if let (Some(hand), Some(pose)) = (self.session.hand(), self.session.pose()) {
            let in_hand = if perspective == Perspective::FirstPerson { shown_block } else { held_block };
            match (perspective, in_hand) {
                (Perspective::FirstPerson, Some(block)) => {
                    held_mesh.push_block(block, &self.layers, player::held_block_first_person(&hand), hand.light);
                }
                (Perspective::FirstPerson, None) => hand_mesh = Some(player::build_hand_mesh(&hand)),
                (_, Some(block)) => {
                    world_items.push_block(block, &self.layers, player::held_block_third_person(&pose), pose.light);
                }
                (_, None) => {}
            }
        }
        // Shadows: dropped items always, the player only when visible (F5).
        let mut casters: Vec<Caster> = self.session.items.shadow_casters().collect();
        if let Some(player) = &self.session.player
            && perspective != Perspective::FirstPerson
        {
            casters.push(Caster { feet: player.feet(), radius: 0.5, strength: 1.0 });
        }
        let chunks = &self.session.chunks;
        self.renderer.prepare_shadows(&casters, &|pos| chunks.block_at(pos));
        let preview = matches!(self.screen, Screen::Inventory).then(|| {
            let rect = inventory_screen::preview_rect(&self.ui);
            self.session.preview(rect, self.ui.input.mouse, self.ui.scale, &self.layers)
        });
        let camera = self.session.camera.for_perspective(perspective);
        let underwater = self.session.underwater(&camera, dt);
        self.renderer.render(
            &camera,
            &self.session.day.state(),
            seconds,
            player_mesh.as_deref(),
            hand_mesh.as_deref(),
            target
                .map(|(hit, progress)| Selection {
                    block: hit.block,
                    crack_layer: (progress > 0.0).then(|| self.layers.destroy_stage(progress)),
                })
                .as_ref(),
            underwater.as_ref(),
            &world_items,
            &particle_mesh,
            &held_mesh,
            &self.ui.vertices,
            self.ui.inverted.clone(),
            [self.ui.width, self.ui.height],
            preview.as_ref(),
        );
        self.ui.input.end_frame();

        self.frames += 1;
        let elapsed = self.fps_timer.elapsed().as_secs_f32();
        if elapsed >= 1.0 {
            self.fps = (self.frames as f32 / elapsed).round() as u32;
            self.frames = 0;
            self.fps_timer = now;
        }
    }

    fn switch_session(&mut self, session: Session) {
        self.renderer.clear_sections();
        self.session = session;
        self.renderer.set_render_distance(self.options.render_distance);
    }

    fn aspect(&self) -> f32 {
        let (w, h) = self.renderer.size();
        w as f32 / h.max(1) as f32
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::Open(screen) => self.open(screen),
            Action::Play(entry) => {
                log::info!("opening world {} (seed {})", entry.save.name, entry.save.seed);
                let session = Session::for_world(entry, &self.layers, &self.options, self.aspect());
                self.switch_session(session);
                self.perspective = Perspective::FirstPerson;
                self.open(Screen::Loading { progress: 0.0 });
            }
            Action::Resume => self.open(Screen::None),
            Action::SaveAndQuit => {
                self.session.save();
                let backdrop = Session::backdrop(&self.layers, &self.options, self.aspect());
                self.switch_session(backdrop);
                self.open(Screen::Title);
            }
            Action::Quit => self.quit = true,
            Action::ApplyOptions => {
                self.session.camera.set_fov(self.options.fov);
                if self.in_world() {
                    self.session.chunks.set_render_distance(self.options.render_distance);
                    self.renderer.set_render_distance(self.options.render_distance);
                }
            }
            Action::CloseOptions { from_pause } => {
                self.options.save();
                self.open(if from_pause { Screen::Pause } else { Screen::Title });
            }
        }
    }

    fn set_mouse_grab(&mut self, grab: bool) {
        if grab == self.mouse_grabbed {
            return;
        }
        if grab {
            // Wayland supports Locked, X11 only Confined: try both.
            let ok = self
                .window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
            if !ok {
                log::warn!("failed to grab cursor");
                return;
            }
            self.window.set_cursor_visible(false);
        } else {
            let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            self.window.set_cursor_visible(true);
        }
        self.mouse_grabbed = grab;
    }
}
