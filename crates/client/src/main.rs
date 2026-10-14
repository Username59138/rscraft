mod app;
mod camera;
mod chunks;
mod frustum;
mod input;
mod mesh;
mod renderer;
mod texture;

use winit::event_loop::{ControlFlow, EventLoop};

/// Minecraft version whose assets (and later protocol) we target.
pub const MC_VERSION: &str = "26.3";
const WORLD_SEED: u32 = 1337;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn"))
        .init();

    // Before the window opens: the first run downloads the client jar.
    let images = texture::BlockImages::load(MC_VERSION);

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = app::App::new(images, WORLD_SEED);
    event_loop.run_app(&mut app).expect("event loop error");
}
