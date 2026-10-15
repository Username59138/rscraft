//! Saving a rendered frame to a PNG file.

use std::path::Path;

use anyhow::{Context, Result, bail};

/// Copies `texture` (the frame just rendered) to the CPU and writes it as PNG.
pub fn save(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, path: &Path) -> Result<()> {
    let (width, height) = (texture.width(), texture.height());
    let bgra = match texture.format() {
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => true,
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => false,
        other => bail!("can't save frames in {other:?}"),
    };
    // Rows of a texture copy must be 256-byte aligned.
    let row = 4 * width;
    let padded_row = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot"),
        size: (padded_row * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded_row), rows_per_image: None },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);

    let (tx, rx) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).context("waiting for the GPU")?;
    rx.recv().context("screenshot mapping was dropped")?.context("mapping the screenshot buffer")?;

    let data = buffer.slice(..).get_mapped_range().map_err(|err| anyhow::anyhow!("reading the screenshot: {err:?}"))?;
    let mut pixels = Vec::with_capacity((row * height) as usize);
    for y in 0..height as usize {
        let line = &data[y * padded_row as usize..][..row as usize];
        for px in line.as_chunks::<4>().0 {
            let [a, b, c, _] = [px[0], px[1], px[2], px[3]];
            // Opaque: the window's alpha isn't meaningful here.
            pixels.extend_from_slice(&if bgra { [c, b, a, 255] } else { [a, b, c, 255] });
        }
    }

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let file = std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&pixels)?;
    Ok(())
}
