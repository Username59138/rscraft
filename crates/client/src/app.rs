use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::camera::Perspective;
use crate::debug::{self, DebugInfo};
use crate::hud;
use crate::input::Input;
use crate::inventory_screen::{self, InventoryAction};
use crate::renderer::{EntityImages, Frame, GuiFrame, Renderer};
use crate::saves::Options;
use crate::scene::{SceneRun, SceneScreen, SceneScript};
use crate::screens::{self, Action, Screen};
use crate::selection::Selection;
use crate::session::Session;
use crate::sky::SkyImages;
use crate::switcher;
use crate::texture::{BlockImages, BlockLayers};
use crate::ui::{GuiAtlas, Rect, Sprite, Ui};
use rscraft_world::gamemode::GameMode;
use rscraft_world::movement::{Controls, PlayerController};

/// Holding T makes time run this much faster.
const TIME_FAST_FORWARD: f32 = 120.0;
/// Columns around the spawn that must be ready before a world opens.
const SPAWN_RADIUS: i32 = 2;
const DOUBLE_CLICK_SECONDS: f32 = 0.35;

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
    /// F3 was used together with another key, so releasing it won't toggle
    /// the debug screen.
    f3_combo: bool,
    /// The F3+F4 game mode switcher's selection, while it's open.
    switcher: Option<GameMode>,
    mouse_grabbed: bool,
    quit: bool,
    started: Instant,
    last_frame: Instant,
    last_click: Instant,
    fps_timer: Instant,
    frames: u32,
    fps: u32,
    scene: Option<SceneRun>,
    /// When the next frame may start, under the frame rate cap.
    next_frame: Instant,
}

pub struct App {
    images: BlockImages,
    sky_images: SkyImages,
    atlas: Option<GuiAtlas>,
    /// A test scene to play instead of showing the title screen.
    scene: Option<SceneScript>,
    entity_images: EntityImages,
    state: Option<State>,
}

impl App {
    pub fn new(
        images: BlockImages,
        sky_images: SkyImages,
        atlas: GuiAtlas,
        entity_images: EntityImages,
        scene: Option<SceneScript>,
    ) -> Self {
        Self { images, sky_images, atlas: Some(atlas), entity_images, scene, state: None }
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
            &options,
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
            f3_combo: false,
            switcher: None,
            mouse_grabbed: false,
            quit: false,
            started: Instant::now(),
            last_frame: Instant::now(),
            last_click: Instant::now(),
            fps_timer: Instant::now(),
            frames: 0,
            fps: 0,
            scene: None,
            next_frame: Instant::now(),
        });
        if let Some(script) = self.scene.take()
            && let Some(state) = &mut self.state
        {
            match SceneRun::new(script) {
                Ok((run, entry)) => {
                    // Only for this run: options are saved from the options screen alone.
                    if let Some(distance) = run.script.render_distance {
                        state.options.render_distance = distance;
                    }
                    state.scene = Some(run);
                    state.apply(Action::Play(entry));
                }
                Err(err) => {
                    log::error!("scene: {err:#}");
                    state.quit = true;
                }
            }
        }
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
                } else if code == KeyCode::F3 {
                    state.f3_released();
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
                        state.session.start_attack();
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
            // Scenes run unattended, focused or not.
            WindowEvent::Focused(false) if state.scene.is_none() => {
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = self.state.as_mut() else { return };
        // The Max Framerate cap: sleep until the next frame is due. Scenes
        // run uncapped so they finish quickly.
        let limit = state.options.fps_limit().filter(|_| state.scene.is_none());
        let now = Instant::now();
        match limit {
            Some(_) if now < state.next_frame => {
                event_loop.set_control_flow(ControlFlow::WaitUntil(state.next_frame));
            }
            Some(fps) => {
                // Catch up at most one frame, so a stall isn't followed by a burst.
                let interval = Duration::from_secs_f64(1.0 / fps as f64);
                state.next_frame = (state.next_frame + interval).max(now);
                event_loop.set_control_flow(ControlFlow::Poll);
                state.window.request_redraw();
            }
            None => {
                event_loop.set_control_flow(ControlFlow::Poll);
                state.window.request_redraw();
            }
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
        self.in_world()
            && !matches!(self.screen, Screen::Pause | Screen::Options { from_pause: true } | Screen::Loading { .. })
    }

    fn key_pressed(&mut self, code: KeyCode, text: Option<&str>) {
        if self.playing() {
            match code {
                KeyCode::Escape => self.open(Screen::Pause),
                // F3 toggles the debug screen on release, unless used in a combo.
                KeyCode::F3 => self.f3_combo = false,
                // F3+F4: open the game mode switcher, then cycle through it.
                KeyCode::F4 if self.input.is_pressed(KeyCode::F3) => {
                    self.f3_combo = true;
                    let session = &self.session;
                    self.switcher = Some(match self.switcher {
                        Some(selected) => selected.next(),
                        // Vanilla offers the previous mode first.
                        None => session.previous_game_mode.filter(|&m| m != session.game_mode).unwrap_or(
                            if session.game_mode == GameMode::Creative {
                                GameMode::Survival
                            } else {
                                GameMode::Creative
                            },
                        ),
                    });
                }
                KeyCode::F5 => self.perspective = self.perspective.next(),
                KeyCode::KeyE => self.open(Screen::Inventory),
                KeyCode::KeyQ => {
                    let whole = self.input.is_pressed(KeyCode::ControlLeft);
                    self.session.throw_selected(whole);
                }
                KeyCode::Digit1
                | KeyCode::Digit2
                | KeyCode::Digit3
                | KeyCode::Digit4
                | KeyCode::Digit5
                | KeyCode::Digit6
                | KeyCode::Digit7
                | KeyCode::Digit8
                | KeyCode::Digit9 => {
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

    /// Releasing F3 applies the switcher's choice, or toggles the debug
    /// screen if F3 was pressed on its own.
    fn f3_released(&mut self) {
        if let Some(mode) = self.switcher.take() {
            self.session.set_game_mode(mode);
        } else if !self.f3_combo && self.playing() {
            self.show_debug = !self.show_debug;
        }
        self.f3_combo = false;
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
        self.step_scene();

        // Simulation: the backdrop slowly turns; the world runs unless paused,
        // but only takes the player's input when no screen is open.
        let mut controls = None;
        if self.world_running() {
            let time_speed =
                if self.playing() && self.input.is_pressed(KeyCode::KeyT) { TIME_FAST_FORWARD } else { 1.0 };
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
        // The world ticks unless paused; the view follows every frame.
        if let Some(controls) = controls {
            self.session.update(dt, controls);
        }
        let zoom = self.playing() && self.input.is_pressed(KeyCode::KeyC);
        self.session.update_view(dt, self.options.fov, zoom);
        let target = if self.playing() {
            self.session.interact(dt, self.input.attack, self.input.use_item)
        } else {
            self.session.breaking = None;
            None
        };

        let t_simulate = Instant::now();
        self.session.chunks.update(self.session.camera.position, &mut self.renderer);
        let t_chunks = Instant::now();
        if let Screen::Loading { progress } = &mut self.screen {
            *progress = self.session.chunks.progress(self.session.camera.position, SPAWN_RADIUS);
            if *progress >= 1.0 {
                let chunks = &self.session.chunks;
                if let Some(player) = &mut self.session.player {
                    player.unstuck(&|pos| chunks.block_at(pos));
                }
                self.open(Screen::None);
                if self.scene.is_some() {
                    self.start_scene();
                }
            }
        }

        // Interface.
        let (width, height) = self.renderer.size();
        let scale = self.gui_scale();
        self.ui.begin(width, height, scale, dt);
        let spectator = self.session.game_mode.is_spectator();
        // Spectators get no crosshair, hotbar, hand or body, like vanilla.
        if self.playing() && self.perspective == Perspective::FirstPerson && !spectator {
            let (cx, cy) = ((self.ui.width / 2.0).floor(), (self.ui.height / 2.0).floor());
            self.ui.sprite_inverted(Sprite::Crosshair, Rect::new(cx - 7.0, cy - 7.0, 15.0, 15.0));
        }
        if self.in_world() && !matches!(self.screen, Screen::Loading { .. }) && !spectator {
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
        if let Some(selected) = self.switcher {
            switcher::draw(&mut self.ui, selected);
        }
        let action = screens::draw(&mut self.screen, &mut self.ui, &mut self.options);
        self.apply(action);

        // World.
        let perspective = if self.in_world() { self.perspective } else { Perspective::FirstPerson };
        let t_gui = Instant::now();
        let scene = self.session.scene(perspective, &self.layers, seconds);
        let chunks = &self.session.chunks;
        self.renderer.prepare_shadows(&scene.shadows, &|pos| chunks.block_at(pos));
        let preview = matches!(self.screen, Screen::Inventory).then(|| {
            let rect = inventory_screen::preview_rect(&self.ui);
            self.session.preview(rect, self.ui.input.mouse, self.ui.scale, &self.layers)
        });
        let camera = self.session.camera.for_perspective(perspective);
        let underwater = self.session.underwater(&camera, dt);
        let t_scene = Instant::now();
        self.renderer.render(Frame {
            camera: &camera,
            sky: &self.session.day.state(),
            seconds,
            player: scene.player.as_deref(),
            hand: scene.hand.as_deref(),
            selection: target.map(|(hit, progress)| Selection {
                block: hit.block,
                crack_layer: (progress > 0.0).then(|| self.layers.destroy_stage(progress)),
            }),
            underwater,
            items: &scene.items,
            particles: &scene.particles,
            held: &scene.held,
            gui: GuiFrame {
                vertices: &self.ui.vertices,
                inverted: self.ui.inverted.clone(),
                size: [self.ui.width, self.ui.height],
            },
            preview: preview.as_ref(),
        });
        self.ui.input.end_frame();
        if let Some(run) = &mut self.scene
            && run.started.is_some()
        {
            let times = &mut run.times;
            let since = |a: Instant, b: Instant| (b - a).as_secs_f64();
            times.frames += 1;
            times.simulate += since(now, t_simulate);
            times.chunks += since(t_simulate, t_chunks);
            times.gui += since(t_chunks, t_gui);
            times.scene += since(t_gui, t_scene);
            times.render += since(t_scene, Instant::now());
        }

        self.frames += 1;
        let elapsed = self.fps_timer.elapsed().as_secs_f32();
        if elapsed >= 1.0 {
            self.fps = (self.frames as f32 / elapsed).round() as u32;
            self.frames = 0;
            self.fps_timer = now;
        }
    }

    /// The scene's world has loaded: set everything up as the script says.
    fn start_scene(&mut self) {
        let Some(run) = &mut self.scene else { return };
        let script = run.script.clone();
        run.started = Some(Instant::now());
        let blocks = script.blocks().unwrap_or_default();
        let session = &mut self.session;
        if let Some(player) = &mut session.player {
            if let Some(feet) = script.feet() {
                *player = PlayerController::new(feet, script.flying);
            }
            player.flying = script.flying;
        }
        session.apply_game_mode();
        session.camera.yaw = script.yaw.to_radians();
        session.camera.pitch = script.pitch.to_radians();
        session.day.time = script.time;
        for (slot, stack) in session.inventory.slots.iter_mut().zip(blocks.hotbar) {
            *slot = Some(stack);
        }
        session.inventory.selected = script.selected.min(8);
        let view = crate::hand::view_angles(&session.camera);
        session.hand.settle(session.inventory.selected_stack().map(|s| s.block), view);
        for (a, b, block) in blocks.fills {
            let (min, max) = (a.min(b), a.max(b));
            for y in min.y..=max.y {
                for z in min.z..=max.z {
                    for x in min.x..=max.x {
                        session.chunks.set_block(glam::IVec3::new(x, y, z), block);
                    }
                }
            }
        }
        self.perspective = script.perspective.into();
        self.show_debug = script.debug;
        if script.benchmark {
            self.renderer.set_vsync(false);
        }
        match script.screen {
            SceneScreen::None => {}
            SceneScreen::Inventory => self.open(Screen::Inventory),
            SceneScreen::Pause => self.open(Screen::Pause),
        }
    }

    /// Each frame of a scene: hold its keys, then take the screenshot when
    /// it's time, and quit the frame after.
    fn step_scene(&mut self) {
        let Some(run) = &mut self.scene else { return };
        if run.screenshot_taken {
            self.quit = true;
            return;
        }
        if run.started.is_none() {
            return;
        }
        for key in run.script.keys().unwrap_or_default() {
            self.input.set_key(key, true);
        }
        self.input.attack = run.script.attack;
        self.input.use_item = run.script.use_item;
        if let Some([x, y]) = run.script.mouse {
            let (w, h) = self.renderer.size();
            self.ui.input.mouse = glam::Vec2::new(x * w as f32, y * h as f32) / self.ui.scale;
        }
        if run.due() {
            self.renderer.request_screenshot(run.script.screenshot.clone());
            run.screenshot_taken = true;
            if run.script.benchmark {
                run.times.report(run.script.wait);
            }
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
                self.renderer.set_vsync(self.options.vsync);
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
