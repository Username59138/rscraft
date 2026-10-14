use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use glam::Vec3;
use rscraft_world::Generator;

use crate::camera::Camera;
use crate::chunks::ChunkManager;
use crate::input::Input;
use crate::renderer::Renderer;
use crate::texture::BlockImages;

/// Radius in chunks.
const RENDER_DISTANCE: i32 = 16;

/// Everything that only exists while the window is alive.
struct State {
    window: Arc<Window>,
    renderer: Renderer,
    chunks: ChunkManager,
    camera: Camera,
    input: Input,
    last_frame: Instant,
    fps_timer: Instant,
    frames: u32,
    mouse_grabbed: bool,
}

pub struct App {
    images: BlockImages,
    generator: Arc<Generator>,
    state: Option<State>,
}

impl App {
    pub fn new(images: BlockImages, seed: u32) -> Self {
        Self { images, generator: Arc::new(Generator::new(seed)), state: None }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("rscraft")
            .with_inner_size(PhysicalSize::new(1280, 720));
        let window = Arc::new(event_loop.create_window(attrs).expect("failed to create window"));
        let size = window.inner_size();

        let renderer = pollster::block_on(Renderer::new(
            window.clone(),
            event_loop.owned_display_handle(),
            &self.images,
            RENDER_DISTANCE,
        ));
        let chunks = ChunkManager::new(self.generator.clone(), Arc::new(self.images.layers.clone()), RENDER_DISTANCE);
        let spawn_height = self.generator.surface_height(8, 8).max(rscraft_world::SEA_LEVEL) as f32;
        let camera = Camera::new(size.width as f32 / size.height.max(1) as f32, Vec3::new(8.5, spawn_height + 12.0, 8.5));

        self.state = Some(State {
            window,
            renderer,
            chunks,
            camera,
            input: Input::default(),
            last_frame: Instant::now(),
            fps_timer: Instant::now(),
            frames: 0,
            mouse_grabbed: false,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else { return };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                state.renderer.resize(size);
                state.camera.set_aspect(size.width as f32 / size.height.max(1) as f32);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let pressed = event.state == ElementState::Pressed;
                    if code == KeyCode::Escape && pressed {
                        state.set_mouse_grab(false);
                    }
                    state.input.set_key(code, pressed);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if !state.mouse_grabbed {
                    state.set_mouse_grab(true);
                }
            }
            WindowEvent::Focused(false) => {
                state.set_mouse_grab(false);
                state.input.clear();
            }
            WindowEvent::RedrawRequested => state.frame(),
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        let Some(state) = self.state.as_mut() else { return };
        if let DeviceEvent::MouseMotion { delta } = event {
            if state.mouse_grabbed {
                state.input.add_mouse_delta(delta.0 as f32, delta.1 as f32);
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = self.state.as_ref() {
            state.window.request_redraw();
        }
    }
}

impl State {
    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        self.camera.update(&mut self.input, dt);
        self.chunks.update(self.camera.position, &mut self.renderer);
        self.renderer.render(&self.camera);

        self.frames += 1;
        let elapsed = self.fps_timer.elapsed().as_secs_f32();
        if elapsed >= 1.0 {
            let fps = self.frames as f32 / elapsed;
            let p = self.camera.position;
            self.window.set_title(&format!(
                "rscraft — {fps:.0} FPS — pos {:.1} {:.1} {:.1} — {} columns, {} sections drawn",
                p.x,
                p.y,
                p.z,
                self.chunks.loaded_columns(),
                self.renderer.drawn_sections,
            ));
            self.frames = 0;
            self.fps_timer = now;
        }
    }

    fn set_mouse_grab(&mut self, grab: bool) {
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
