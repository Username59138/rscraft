//! Block textures stored as a 2D texture array: one 16×16 layer per distinct
//! (texture, tint, overlay) combination, taken from Minecraft's client jar.
//! Animated textures (water) get their layer rewritten as time passes.

use std::collections::HashMap;

use rscraft_assets::{ClientJar, RgbaImage};
use rscraft_world::{Block, Face, FaceTexture, Tint};

/// Block textures are 16×16, but flowing water is 32×32 and vanilla shows a
/// quarter of it per face, so the array holds 32×32 layers.
pub const TEXTURE_SIZE: u32 = 32;
const MIP_LEVELS: u32 = TEXTURE_SIZE.ilog2() + 1;
/// Minecraft runs at 20 ticks per second; animation frame times are in ticks.
const TICK_SECONDS: f32 = 0.05;
/// Minecraft's default water colour (#3F76E4).
const WATER_COLOR: [u8; 3] = [0x3f, 0x76, 0xe4];
/// Biome climate used for tinting until biomes exist (plains).
const TEMPERATURE: f32 = 0.8;
const DOWNFALL: f32 = 0.4;

/// Texture array layer for every block face. Layer 0 is the "missing" texture.
#[derive(Clone)]
pub struct BlockLayers {
    table: [[u32; 6]; Block::ALL.len()],
    /// First of the ten consecutive block-breaking crack layers.
    destroy_stages: u32,
    /// First of the four splash particle layers.
    pub splash: u32,
    pub bubble: u32,
    /// Vanilla's tiny suspended "underwater" particle, pre-tinted.
    pub underwater: u32,
}

/// Number of splash particle textures, splash_0..3.
pub const SPLASH_FRAMES: u32 = 4;

/// Number of crack textures, destroy_stage_0..9.
pub const DESTROY_STAGES: u32 = 10;

impl BlockLayers {
    /// Crack texture for breaking progress 0..1.
    pub fn destroy_stage(&self, progress: f32) -> u32 {
        self.destroy_stages + ((progress * DESTROY_STAGES as f32) as u32).min(DESTROY_STAGES - 1)
    }

    #[inline]
    pub fn get(&self, block: Block, face: Face) -> u32 {
        self.table[block.index()][block.texture_face(face) as usize]
    }
}

/// One texture array layer: its animation frames, each as a full mip chain.
struct Layer {
    frames: Vec<Vec<RgbaImage>>,
    frametime: u32,
}

impl Layer {
    fn new(frames: Vec<RgbaImage>, frametime: u32) -> Self {
        // Textures with fully transparent pixels are cut-outs (leaves).
        let cutout = frames[0].pixels.as_chunks::<4>().0.iter().any(|p| p[3] == 0);
        let frames = frames
            .into_iter()
            .map(|image| {
                let mut chain = vec![image];
                for _ in 1..MIP_LEVELS {
                    let next = chain.last().unwrap().downsampled(cutout);
                    chain.push(next);
                }
                chain
            })
            .collect();
        Self { frames, frametime }
    }
}

/// CPU-side block textures, ready to be uploaded.
pub struct BlockImages {
    pub layers: BlockLayers,
    images: Vec<Layer>,
}

impl BlockImages {
    /// Builds the block textures from the client jar, or from the magenta
    /// "missing" texture when no jar is available.
    pub fn load(mut jar: Option<&mut ClientJar>) -> Self {
        let tints = jar.as_deref_mut().map(Tints::load).unwrap_or_default();

        let mut images = vec![Layer::new(vec![missing_texture()], 1)];
        let mut by_texture: HashMap<FaceTexture, u32> = HashMap::new();
        let mut table = [[0; 6]; Block::ALL.len()];
        for block in Block::ALL.into_iter().filter(|&b| b != Block::Air) {
            for face in Face::ALL {
                let texture = block.texture(face);
                table[block.index()][face as usize] = *by_texture.entry(texture).or_insert_with(|| {
                    let (frames, frametime) = jar.as_deref_mut().map_or_else(
                        || (vec![missing_texture()], 1),
                        |jar| {
                            compose(jar, texture, &tints).unwrap_or_else(|err| {
                                log::warn!("texture {}: {err:#}", texture.name);
                                (vec![missing_texture()], 1)
                            })
                        },
                    );
                    images.push(Layer::new(frames, frametime));
                    images.len() as u32 - 1
                });
            }
        }
        let destroy_stages = images.len() as u32;
        for stage in 0..DESTROY_STAGES {
            let name = format!("destroy_stage_{stage}");
            let image = jar
                .as_deref_mut()
                .and_then(|jar| jar.block_texture(&name).map_err(|err| log::warn!("{name}: {err:#}")).ok())
                .map_or_else(missing_texture, |image| image.resized(TEXTURE_SIZE, TEXTURE_SIZE));
            images.push(Layer::new(vec![image], 1));
        }

        // Particle sprites, scaled up from 8×8 to share the block array.
        let mut particle = |name: &str, tint: [u8; 3]| {
            let image = jar
                .as_deref_mut()
                .and_then(|jar| {
                    jar.texture(&format!("particle/{name}")).map_err(|err| log::warn!("{name}: {err:#}")).ok()
                })
                .map_or_else(missing_texture, |image| image.resized(TEXTURE_SIZE, TEXTURE_SIZE));
            let image = RgbaImage {
                pixels: image
                    .pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|p| {
                        let [r, g, b] = [0, 1, 2].map(|i| (p[i] as u16 * tint[i] as u16 / 255) as u8);
                        [r, g, b, p[3]]
                    })
                    .collect(),
                ..image
            };
            images.push(Layer::new(vec![image], 1));
            images.len() as u32 - 1
        };
        let splash = particle("splash_0", [255; 3]);
        for frame in 1..SPLASH_FRAMES {
            particle(&format!("splash_{frame}"), [255; 3]);
        }
        let bubble = particle("bubble", [255; 3]);
        // SuspendedParticle's underwater colour: (0.4, 0.4, 0.7).
        let underwater = particle("generic_0", [102, 102, 179]);

        log::info!("loaded {} block textures", images.len() - 1);
        Self { layers: BlockLayers { table, destroy_stages, splash, bubble, underwater }, images }
    }

    /// The top, south and east face images of a block, for inventory icons.
    pub fn icon_faces(&self, block: Block) -> [RgbaImage; 3] {
        [Face::Up, Face::South, Face::East]
            .map(|face| self.images[self.layers.get(block, face) as usize].frames[0][0].clone())
    }
}

/// An animated layer and which frame is currently uploaded.
struct Animation {
    layer: u32,
    frames: Vec<Vec<RgbaImage>>,
    frame_seconds: f32,
    current: usize,
}

/// Uploaded block texture array.
pub struct BlockTextures {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    animations: Vec<Animation>,
}

impl BlockTextures {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, images: &BlockImages) -> Self {
        let layer_count = images.images.len() as u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("block textures"),
            size: wgpu::Extent3d { width: TEXTURE_SIZE, height: TEXTURE_SIZE, depth_or_array_layers: layer_count },
            mip_level_count: MIP_LEVELS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let mut animations = Vec::new();
        for (layer, image) in images.images.iter().enumerate() {
            write_layer(queue, &texture, layer as u32, &image.frames[0]);
            if image.frames.len() > 1 {
                animations.push(Animation {
                    layer: layer as u32,
                    frames: image.frames.clone(),
                    frame_seconds: image.frametime as f32 * TICK_SECONDS,
                    current: 0,
                });
            }
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        // Nearest filtering keeps the pixel-art look up close; mipmaps avoid
        // shimmering in the distance.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("block sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        Self { texture, view, sampler, animations }
    }

    /// Uploads the current frame of every animated texture.
    pub fn animate(&mut self, queue: &wgpu::Queue, seconds: f32) {
        for anim in &mut self.animations {
            let frame = (seconds / anim.frame_seconds) as usize % anim.frames.len();
            if frame != anim.current {
                anim.current = frame;
                write_layer(queue, &self.texture, anim.layer, &anim.frames[frame]);
            }
        }
    }
}

fn write_layer(queue: &wgpu::Queue, texture: &wgpu::Texture, layer: u32, mips: &[RgbaImage]) {
    for (level, image) in mips.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                aspect: wgpu::TextureAspect::All,
            },
            &image.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * image.width),
                rows_per_image: Some(image.height),
            },
            wgpu::Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 },
        );
    }
}

/// Resolved biome tint colours.
struct Tints {
    grass: [u8; 3],
    foliage: [u8; 3],
}

impl Default for Tints {
    fn default() -> Self {
        Self { grass: [0x91, 0xbd, 0x59], foliage: [0x77, 0xab, 0x2f] }
    }
}

impl Tints {
    fn load(jar: &mut ClientJar) -> Self {
        let defaults = Self::default();
        let mut sample = |name: &str, fallback| match jar.colormap(name) {
            Ok(map) => sample_colormap(&map, TEMPERATURE, DOWNFALL),
            Err(err) => {
                log::warn!("colormap {name}: {err:#}");
                fallback
            }
        };
        Self { grass: sample("grass", defaults.grass), foliage: sample("foliage", defaults.foliage) }
    }

    fn color(&self, tint: Tint) -> Option<[u8; 3]> {
        match tint {
            Tint::None => None,
            Tint::Grass => Some(self.grass),
            Tint::Foliage => Some(self.foliage),
            Tint::Water => Some(WATER_COLOR),
        }
    }
}

/// Same lookup as Minecraft: x from temperature, y from downfall × temperature.
fn sample_colormap(map: &RgbaImage, temperature: f32, downfall: f32) -> [u8; 3] {
    let t = temperature.clamp(0.0, 1.0);
    let d = downfall.clamp(0.0, 1.0) * t;
    let x = (((1.0 - t) * 255.0) as u32).min(map.width - 1);
    let y = (((1.0 - d) * 255.0) as u32).min(map.height - 1);
    let [r, g, b, _] = map.pixel(x, y);
    [r, g, b]
}

fn multiply(rgba: [u8; 4], tint: [u8; 3]) -> [u8; 4] {
    let mul = |c: u8, t: u8| (c as u16 * t as u16 / 255) as u8;
    [mul(rgba[0], tint[0]), mul(rgba[1], tint[1]), mul(rgba[2], tint[2]), rgba[3]]
}

/// Builds the final frames for a face: tints them and/or draws the tinted
/// overlay over the base (how grass block sides work).
fn compose(jar: &mut ClientJar, texture: FaceTexture, tints: &Tints) -> anyhow::Result<(Vec<RgbaImage>, u32)> {
    let (frames, frametime) = jar.block_texture_frames(texture.name)?;
    let tint = tints.color(texture.tint);
    let overlay = match texture.overlay {
        Some(name) => Some(jar.block_texture(name)?.resized(TEXTURE_SIZE, TEXTURE_SIZE)),
        None => None,
    };

    let frames = frames
        .into_iter()
        .map(|frame| {
            let mut image = frame.resized(TEXTURE_SIZE, TEXTURE_SIZE);
            match (&overlay, tint) {
                (Some(overlay), tint) => {
                    for y in 0..TEXTURE_SIZE {
                        for x in 0..TEXTURE_SIZE {
                            let top = overlay.pixel(x, y);
                            if top[3] > 0 {
                                image.set_pixel(x, y, tint.map_or(top, |t| multiply(top, t)));
                            }
                        }
                    }
                }
                (None, Some(tint)) => {
                    for px in image.pixels.as_chunks_mut::<4>().0 {
                        let tinted = multiply([px[0], px[1], px[2], px[3]], tint);
                        px.copy_from_slice(&tinted);
                    }
                }
                (None, None) => {}
            }
            image
        })
        .collect();
    Ok((frames, frametime))
}

/// Minecraft's magenta/black checkerboard.
pub fn missing_texture() -> RgbaImage {
    let mut image = RgbaImage::new(TEXTURE_SIZE, TEXTURE_SIZE);
    for y in 0..TEXTURE_SIZE {
        for x in 0..TEXTURE_SIZE {
            let magenta = (x / 8 + y / 8) % 2 == 0;
            image.set_pixel(x, y, if magenta { [248, 0, 248, 255] } else { [0, 0, 0, 255] });
        }
    }
    image
}
