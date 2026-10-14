//! Access to Minecraft's own assets. Nothing from Mojang is shipped with
//! rscraft: the client jar is downloaded from Mojang's servers (or taken from
//! an existing Minecraft installation) and cached locally.

mod download;
mod image;

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use anyhow::{Context, Result};

pub use download::ensure_client_jar;
pub use image::RgbaImage;

/// A Minecraft client jar opened for reading.
pub struct ClientJar {
    zip: zip::ZipArchive<BufReader<File>>,
}

impl ClientJar {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let zip = zip::ZipArchive::new(BufReader::new(file)).context("reading client jar")?;
        Ok(Self { zip })
    }

    /// Reads a file by its path inside the jar, e.g. `assets/minecraft/...`.
    pub fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        let mut entry = self.zip.by_name(path).with_context(|| format!("{path} not found in client jar"))?;
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut data)?;
        Ok(data)
    }

    pub fn read_png(&mut self, path: &str) -> Result<RgbaImage> {
        let data = self.read(path)?;
        RgbaImage::decode_png(&data).with_context(|| format!("decoding {path}"))
    }

    /// A block texture by name (`"stone"`). Animated textures are stored as a
    /// vertical strip of frames; only the first frame is returned.
    pub fn block_texture(&mut self, name: &str) -> Result<RgbaImage> {
        let image = self.read_png(&format!("assets/minecraft/textures/block/{name}.png"))?;
        Ok(image.crop_top_square())
    }

    /// A biome colour map (`"grass"`, `"foliage"`): 256×256, temperature × downfall.
    pub fn colormap(&mut self, name: &str) -> Result<RgbaImage> {
        self.read_png(&format!("assets/minecraft/textures/colormap/{name}.png"))
    }
}
