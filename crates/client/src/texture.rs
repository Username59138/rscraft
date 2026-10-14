//! Block textures stored as a 2D texture array: one 16×16 layer per distinct
//! (texture, tint, overlay) combination, taken from Minecraft's client jar.

use std::collections::HashMap;

use rscraft_assets::{ClientJar, RgbaImage};
use rscraft_world::{Block, Face, FaceTexture, Tint};

pub const TEXTURE_SIZE: u32 = 16;
const MIP_LEVELS: u32 = TEXTURE_SIZE.ilog2() + 1;
/// Minecraft's default water colour (#3F76E4).
const WATER_COLOR: [u8; 3] = [0x3f, 0x76, 0xe4];
/// Biome climate used for tinting until biomes exist (plains).
const TEMPERATURE: f32 = 0.8;
const DOWNFALL: f32 = 0.4;

/// Texture array layer for every block face. Layer 0 is the "missing" texture.
#[derive(Clone)]
pub struct BlockLayers {
    table: [[u32; 6]; Block::ALL.len()],
}

impl BlockLayers {
    #[inline]
    pub fn get(&self, block: Block, face: Face) -> u32 {
        self.table[block as usize][face as usize]
    }
}

/// CPU-side block textures, ready to be uploaded.
pub struct BlockImages {
    pub layers: BlockLayers,
    images: Vec<RgbaImage>,
}

impl BlockImages {
    /// Loads block textures from the client jar of `version`. Falls back to
    /// the magenta "missing" texture if the jar can't be obtained.
    pub fn load(version: &str) -> Self {
        let mut jar = match rscraft_assets::ensure_client_jar(version).and_then(|path| ClientJar::open(&path)) {
            Ok(jar) => Some(jar),
            Err(err) => {
                log::error!("Minecraft assets unavailable, using placeholder textures: {err:#}");
                None
            }
        };
        let tints = jar.as_mut().map(Tints::load).unwrap_or_default();

        let mut images = vec![missing_texture()];
        let mut by_texture: HashMap<FaceTexture, u32> = HashMap::new();
        let mut table = [[0; 6]; Block::ALL.len()];
        for block in Block::ALL.into_iter().filter(|&b| b != Block::Air) {
            for face in Face::ALL {
                let texture = block.texture(face);
                table[block as usize][face as usize] = *by_texture.entry(texture).or_insert_with(|| {
                    let image = jar.as_mut().map_or_else(missing_texture, |jar| {
                        compose(jar, texture, &tints).unwrap_or_else(|err| {
                            log::warn!("texture {}: {err:#}", texture.name);
                            missing_texture()
                        })
                    });
                    images.push(image);
                    images.len() as u32 - 1
                });
            }
        }
        log::info!("loaded {} block textures", images.len() - 1);
        Self { layers: BlockLayers { table }, images }
    }
}

/// Uploaded block texture array.
pub struct BlockTextures {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
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

        let mut level_images = images.images.clone();
        for level in 0..MIP_LEVELS {
            let size = TEXTURE_SIZE >> level;
            let pixels: Vec<u8> = level_images.iter().flat_map(|img| img.pixels.iter().copied()).collect();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * size), rows_per_image: Some(size) },
                wgpu::Extent3d { width: size, height: size, depth_or_array_layers: layer_count },
            );
            level_images = level_images.iter().map(RgbaImage::downsampled).collect();
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

        Self { view, sampler }
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

/// Builds the final texture for a face: tints it and/or draws the tinted
/// overlay over the base (how grass block sides work).
fn compose(jar: &mut ClientJar, texture: FaceTexture, tints: &Tints) -> anyhow::Result<RgbaImage> {
    let mut image = jar.block_texture(texture.name)?.resized(TEXTURE_SIZE, TEXTURE_SIZE);
    let tint = tints.color(texture.tint);

    match (texture.overlay, tint) {
        (Some(overlay), tint) => {
            let overlay = jar.block_texture(overlay)?.resized(TEXTURE_SIZE, TEXTURE_SIZE);
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
            for px in image.pixels.chunks_exact_mut(4) {
                let tinted = multiply([px[0], px[1], px[2], px[3]], tint);
                px.copy_from_slice(&tinted);
            }
        }
        (None, None) => {}
    }
    Ok(image)
}

/// Minecraft's magenta/black checkerboard.
fn missing_texture() -> RgbaImage {
    let mut image = RgbaImage::new(TEXTURE_SIZE, TEXTURE_SIZE);
    for y in 0..TEXTURE_SIZE {
        for x in 0..TEXTURE_SIZE {
            let magenta = (x / 8 + y / 8) % 2 == 0;
            image.set_pixel(x, y, if magenta { [248, 0, 248, 255] } else { [0, 0, 0, 255] });
        }
    }
    image
}
