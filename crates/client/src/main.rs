mod app;
mod camera;
mod chunks;
mod clouds;
mod debug;
mod frustum;
mod hud;
mod inventory;
mod inventory_screen;
mod items;
mod input;
mod mesh;
mod movement;
mod player;
mod raycast;
mod renderer;
mod saves;
mod selection;
mod shadows;
mod screens;
mod sky;
mod texture;
mod ui;

use winit::event_loop::{ControlFlow, EventLoop};

/// Minecraft version whose assets (and later protocol) we target.
pub const MC_VERSION: &str = "26.3";

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
    let icons: Vec<_> = rscraft_world::Block::ALL
        .into_iter()
        .filter(|b| b.drop() == Some(*b))
        .map(|b| (b, block_images.icon_faces(b)))
        .collect();
    let atlas = ui::GuiAtlas::load(jar.as_mut(), &icons);
    let skin = jar
        .as_mut()
        .and_then(|jar| jar.texture("entity/player/wide/steve").map_err(|err| log::warn!("skin: {err:#}")).ok())
        .unwrap_or_else(|| {
            let mut skin = rscraft_assets::RgbaImage::new(64, 64);
            skin.pixels.chunks_exact_mut(4).for_each(|p| p.copy_from_slice(&[200, 120, 90, 255]));
            skin
        });
    let shadow = jar
        .as_mut()
        .and_then(|jar| jar.texture("misc/shadow").map_err(|err| log::warn!("shadow: {err:#}")).ok())
        .unwrap_or_else(|| {
            // A soft dark disc, in case the jar is missing.
            let mut image = rscraft_assets::RgbaImage::new(16, 16);
            for y in 0..16 {
                for x in 0..16 {
                    let d = ((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt();
                    image.set_pixel(x, y, [0, 0, 0, if d < 7.5 { 160 } else { 0 }]);
                }
            }
            image
        });
    drop(jar);

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = app::App::new(block_images, sky_images, atlas, renderer::EntityImages { skin, shadow });
    event_loop.run_app(&mut app).expect("event loop error");
}
