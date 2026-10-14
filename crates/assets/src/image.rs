use std::io::Cursor;

use anyhow::{Result, bail};

/// 8-bit RGBA image, rows top to bottom.
#[derive(Clone, Debug)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl RgbaImage {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, pixels: vec![0; (width * height * 4) as usize] }
    }

    pub fn decode_png(data: &[u8]) -> Result<Self> {
        let mut decoder = png::Decoder::new(Cursor::new(data));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info()?;
        let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
        let info = reader.next_frame(&mut buf)?;
        buf.truncate(info.buffer_size());

        let pixels = match info.color_type {
            png::ColorType::Rgba => buf,
            png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
            png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
            png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            other => bail!("unsupported PNG colour type {other:?}"),
        };
        Ok(Self { width: info.width, height: info.height, pixels })
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        self.pixels[i..i + 4].try_into().unwrap()
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        let i = ((y * self.width + x) * 4) as usize;
        self.pixels[i..i + 4].copy_from_slice(&rgba);
    }

    /// Keeps only the top `width × width` square (first animation frame).
    pub fn crop_top_square(mut self) -> Self {
        if self.height > self.width {
            self.height = self.width;
            self.pixels.truncate((self.width * self.width * 4) as usize);
        }
        self
    }

    /// Splits a vertical strip of square frames (animated textures).
    pub fn split_frames(&self) -> Vec<Self> {
        let frame_bytes = (self.width * self.width * 4) as usize;
        let count = (self.height / self.width).max(1) as usize;
        (0..count)
            .map(|i| Self {
                width: self.width,
                height: self.width.min(self.height),
                pixels: self.pixels[i * frame_bytes..((i + 1) * frame_bytes).min(self.pixels.len())].to_vec(),
            })
            .collect()
    }

    /// Nearest-neighbour resize, used to normalise odd-sized textures.
    pub fn resized(&self, width: u32, height: u32) -> Self {
        if (width, height) == (self.width, self.height) {
            return self.clone();
        }
        let mut out = Self::new(width, height);
        for y in 0..height {
            for x in 0..width {
                out.set_pixel(x, y, self.pixel(x * self.width / width, y * self.height / height));
            }
        }
        out
    }

    /// Half-size image using a 2×2 box filter (for mipmaps). With `cutout`,
    /// colour is averaged over opaque pixels only and alpha takes the maximum,
    /// so cut-out textures such as leaves don't fade away in the distance.
    pub fn downsampled(&self, cutout: bool) -> Self {
        let (w, h) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let mut out = Self::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let samples = [(0, 0), (1, 0), (0, 1), (1, 1)]
                    .map(|(dx, dy)| self.pixel((x * 2 + dx).min(self.width - 1), (y * 2 + dy).min(self.height - 1)));
                let average = |pixels: &[[u8; 4]], channel: usize| {
                    (pixels.iter().map(|p| p[channel] as u32).sum::<u32>() / pixels.len().max(1) as u32) as u8
                };
                let pixel = if cutout {
                    let opaque: Vec<[u8; 4]> = samples.iter().copied().filter(|p| p[3] > 0).collect();
                    let source = if opaque.is_empty() { &samples[..] } else { &opaque[..] };
                    let alpha = samples.iter().map(|p| p[3]).max().unwrap_or(0);
                    [average(source, 0), average(source, 1), average(source, 2), alpha]
                } else {
                    [0, 1, 2, 3].map(|c| average(&samples, c))
                };
                out.set_pixel(x, y, pixel);
            }
        }
        out
    }
}
