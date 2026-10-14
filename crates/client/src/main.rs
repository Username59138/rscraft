mod app;
mod camera;
mod chunks;
mod frustum;
mod input;
mod mesh;
mod renderer;
mod sky;
mod texture;

use winit::event_loop::{ControlFlow, EventLoop};

/// Minecraft version whose assets (and later protocol) we target.
pub const MC_VERSION: &str = "26.3";
const WORLD_SEED: u32 = 1337;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn"))
        .init();

    // Before the window opens: the first run downloads the client jar.
    let mut jar = match rscraft_assets::ensure_client_jar(MC_VERSION)
        .and_then(|path| rscraft_assets::ClientJar::open(&path))
    {
        Ok(jar) => Some(jar),
        Err(err) => {
            log::error!("Minecraft assets unavailable, using placeholder textures: {err:#}");
            None
        }
    };
    let block_images = texture::BlockImages::load(jar.as_mut());
    let sky_images = sky::SkyImages::load(jar.as_mut());
    drop(jar);

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = app::App::new(block_images, sky_images, WORLD_SEED);
    event_loop.run_app(&mut app).expect("event loop error");
}
