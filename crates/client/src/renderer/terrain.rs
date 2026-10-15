//! Terrain sections on the GPU. Instead of a buffer pair per section, the
//! vertices of all sections share a few large pages, every section is drawn
//! with one shared index buffer of quads, and a frame's visible sections are
//! drawn with one indirect multi-draw per page and render layer.

use std::collections::HashMap;
use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::{IVec2, IVec3, Vec3};
use rscraft_world::SECTION_SIZE;

use crate::frustum::Frustum;
use crate::mesh::{SectionMesh, TerrainVertex};

/// Vertices per page: 32 MiB of `TerrainVertex`, far more than any section.
const PAGE_VERTICES: u32 = 1 << 21;

/// One draw of `DrawIndexedIndirectArgs`, laid out like the GPU expects it.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DrawArgs {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

/// First-fit allocation of ranges, in units of vertices.
struct FreeList {
    /// Free ranges, sorted and never touching each other.
    free: Vec<Range<u32>>,
}

impl FreeList {
    // One free range spanning everything, not the numbers 0..len.
    #[allow(clippy::single_range_in_vec_init)]
    fn new(len: u32) -> Self {
        Self { free: vec![0..len] }
    }

    fn alloc(&mut self, len: u32) -> Option<Range<u32>> {
        let i = self.free.iter().position(|r| r.len() as u32 >= len)?;
        let start = self.free[i].start;
        self.free[i].start += len;
        if self.free[i].is_empty() {
            self.free.remove(i);
        }
        Some(start..start + len)
    }

    fn release(&mut self, range: Range<u32>) {
        let free = &mut self.free;
        let i = free.partition_point(|r| r.start < range.start);
        let merges_prev = i > 0 && free[i - 1].end == range.start;
        let merges_next = i < free.len() && free[i].start == range.end;
        match (merges_prev, merges_next) {
            (true, true) => {
                free[i - 1].end = free[i].end;
                free.remove(i);
            }
            (true, false) => free[i - 1].end = range.end,
            (false, true) => free[i].start = range.start,
            (false, false) => free.insert(i, range),
        }
    }
}

/// A vertex buffer shared by many sections.
struct Page {
    buffer: wgpu::Buffer,
    free: FreeList,
}

impl Page {
    fn new(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain vertex page"),
            size: PAGE_VERTICES as u64 * size_of::<TerrainVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { buffer, free: FreeList::new(PAGE_VERTICES) }
    }
}

/// Where a section's mesh of one layer lives.
struct Allocation {
    page: usize,
    vertices: Range<u32>,
}

struct GpuSection {
    solid: Option<Allocation>,
    translucent: Option<Allocation>,
}

/// A buffer rewritten every frame, grown when the data outgrows it.
struct FrameBuffer {
    buffer: wgpu::Buffer,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl FrameBuffer {
    fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages) -> Self {
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: 4096,
            usage,
            mapped_at_creation: false,
        });
        Self { buffer, usage, label }
    }

    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        if data.len() as u64 > self.buffer.size() {
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: (data.len() as u64).next_power_of_two(),
                usage: self.usage,
                mapped_at_creation: false,
            });
        }
        if !data.is_empty() {
            queue.write_buffer(&self.buffer, 0, data);
        }
    }
}

/// Consecutive draws in the indirect buffer that share a page.
struct Batch {
    page: usize,
    draws: Range<u32>,
}

pub struct TerrainRenderer {
    solid_pipeline: wgpu::RenderPipeline,
    translucent_pipeline: wgpu::RenderPipeline,
    pages: Vec<Page>,
    sections: HashMap<IVec3, GpuSection>,
    /// Indices of quads `[0, 1, 2, 0, 2, 3] + 4 * n`, shared by all sections.
    quad_indices: wgpu::Buffer,
    quad_capacity: u32,
    /// Per visible section: its origin, read as an instance attribute.
    instances: FrameBuffer,
    indirect: FrameBuffer,
    /// Indirect multi-draws need `first_instance`; without it every section
    /// is a direct draw call.
    multi_draw: bool,
    solid_batches: Vec<Batch>,
    translucent_batches: Vec<Batch>,
    /// The frame's draws, kept for the fallback path.
    draws: Vec<DrawArgs>,
}

impl TerrainRenderer {
    pub fn new(
        device: &wgpu::Device,
        solid_pipeline: wgpu::RenderPipeline,
        translucent_pipeline: wgpu::RenderPipeline,
        multi_draw: bool,
    ) -> Self {
        let quad_capacity = 16 * 1024;
        Self {
            solid_pipeline,
            translucent_pipeline,
            pages: Vec::new(),
            sections: HashMap::new(),
            quad_indices: quad_index_buffer(device, quad_capacity),
            quad_capacity,
            instances: FrameBuffer::new(device, "section origins", wgpu::BufferUsages::VERTEX),
            indirect: FrameBuffer::new(device, "section draws", wgpu::BufferUsages::INDIRECT),
            multi_draw,
            solid_batches: Vec::new(),
            translucent_batches: Vec::new(),
            draws: Vec::new(),
        }
    }

    /// The per-instance part of the terrain pipelines' vertex input.
    pub fn instance_layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![3 => Sint32x4];
        wgpu::VertexBufferLayout {
            array_stride: size_of::<[i32; 4]>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTRIBUTES,
        }
    }

    pub fn len(&self) -> usize {
        self.sections.len()
    }

    pub fn clear(&mut self) {
        self.sections.clear();
        self.pages.clear();
    }

    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, pos: IVec3, mesh: &SectionMesh) {
        self.remove(pos);
        let section = GpuSection {
            solid: self.upload_layer(device, queue, &mesh.solid),
            translucent: self.upload_layer(device, queue, &mesh.translucent),
        };
        if section.solid.is_some() || section.translucent.is_some() {
            self.sections.insert(pos, section);
        }
    }

    fn upload_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[TerrainVertex],
    ) -> Option<Allocation> {
        if vertices.is_empty() {
            return None;
        }
        let len = vertices.len() as u32;
        let quads = len / 4;
        if quads > self.quad_capacity {
            self.quad_capacity = quads.next_power_of_two();
            self.quad_indices = quad_index_buffer(device, self.quad_capacity);
        }
        let (page, range) = match self.pages.iter_mut().enumerate().find_map(|(i, p)| Some((i, p.free.alloc(len)?))) {
            Some(found) => found,
            None => {
                let mut page = Page::new(device);
                let range = page.free.alloc(len).expect("section mesh larger than a vertex page");
                self.pages.push(page);
                (self.pages.len() - 1, range)
            }
        };
        let offset = range.start as u64 * size_of::<TerrainVertex>() as u64;
        queue.write_buffer(&self.pages[page].buffer, offset, bytemuck::cast_slice(vertices));
        Some(Allocation { page, vertices: range })
    }

    fn release(&mut self, section: GpuSection) {
        for allocation in [section.solid, section.translucent].into_iter().flatten() {
            self.pages[allocation.page].free.release(allocation.vertices);
        }
    }

    fn remove(&mut self, pos: IVec3) {
        if let Some(section) = self.sections.remove(&pos) {
            self.release(section);
        }
    }

    pub fn remove_column(&mut self, column: IVec2) {
        let positions: Vec<IVec3> =
            self.sections.keys().copied().filter(|pos| pos.x == column.x && pos.z == column.y).collect();
        for pos in positions {
            self.remove(pos);
        }
    }

    /// Picks the sections in view and writes their draws; returns how many
    /// sections will be drawn.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frustum: &Frustum, camera: Vec3) -> usize {
        let size = SECTION_SIZE as f32;
        let mut visible: Vec<(f32, IVec3, &GpuSection)> = self
            .sections
            .iter()
            .filter_map(|(&pos, section)| {
                let min = pos.as_vec3() * size;
                let center = min + Vec3::splat(size / 2.0);
                frustum
                    .intersects_aabb(min, min + Vec3::splat(size))
                    .then(|| (center.distance_squared(camera), pos, section))
            })
            .collect();
        // Front to back, so the depth test rejects hidden fragments early.
        visible.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));

        let instances: Vec<[i32; 4]> =
            visible.iter().map(|(_, pos, _)| (*pos * SECTION_SIZE).extend(0).to_array()).collect();
        self.draws.clear();
        let solid = visible.iter().enumerate().filter_map(|(i, (_, _, s))| Some((i as u32, s.solid.as_ref()?)));
        self.solid_batches = batch_draws(&mut self.draws, self.pages.len(), solid);
        // Translucent faces back to front, so nearer water blends over farther.
        let translucent =
            visible.iter().enumerate().rev().filter_map(|(i, (_, _, s))| Some((i as u32, s.translucent.as_ref()?)));
        self.translucent_batches = batch_draws(&mut self.draws, self.pages.len(), translucent);

        self.instances.write(device, queue, bytemuck::cast_slice(&instances));
        if self.multi_draw {
            self.indirect.write(device, queue, bytemuck::cast_slice(&self.draws));
        }
        visible.len()
    }

    pub fn draw_solid(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw(pass, &self.solid_pipeline, &self.solid_batches);
    }

    pub fn draw_translucent(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw(pass, &self.translucent_pipeline, &self.translucent_batches);
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline, batches: &[Batch]) {
        if batches.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_index_buffer(self.quad_indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_vertex_buffer(1, self.instances.buffer.slice(..));
        for batch in batches {
            pass.set_vertex_buffer(0, self.pages[batch.page].buffer.slice(..));
            if self.multi_draw {
                let offset = batch.draws.start as u64 * size_of::<DrawArgs>() as u64;
                pass.multi_draw_indexed_indirect(&self.indirect.buffer, offset, batch.draws.len() as u32);
            } else {
                for draw in &self.draws[batch.draws.start as usize..batch.draws.end as usize] {
                    let instance = draw.first_instance;
                    pass.draw_indexed(0..draw.index_count, draw.base_vertex, instance..instance + 1);
                }
            }
        }
    }
}

/// Appends the draws of `meshes` (instance index, allocation), grouped by
/// page but otherwise in the given order, and returns the groups.
fn batch_draws<'a>(
    draws: &mut Vec<DrawArgs>,
    pages: usize,
    meshes: impl Iterator<Item = (u32, &'a Allocation)>,
) -> Vec<Batch> {
    let mut per_page: Vec<Vec<DrawArgs>> = vec![Vec::new(); pages];
    for (instance, allocation) in meshes {
        per_page[allocation.page].push(DrawArgs {
            index_count: allocation.vertices.len() as u32 / 4 * 6,
            instance_count: 1,
            first_index: 0,
            base_vertex: allocation.vertices.start as i32,
            first_instance: instance,
        });
    }
    let mut batches = Vec::new();
    for (page, page_draws) in per_page.into_iter().enumerate() {
        if page_draws.is_empty() {
            continue;
        }
        let start = draws.len() as u32;
        draws.extend(page_draws);
        batches.push(Batch { page, draws: start..draws.len() as u32 });
    }
    batches
}

fn quad_index_buffer(device: &wgpu::Device, quads: u32) -> wgpu::Buffer {
    let indices: Vec<u32> = (0..quads).flat_map(|q| [0, 1, 2, 0, 2, 3].map(|i| q * 4 + i)).collect();
    wgpu::util::DeviceExt::create_buffer_init(
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("quad indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn after_release(free: &[Range<u32>], range: Range<u32>) -> Vec<Range<u32>> {
        let mut list = FreeList { free: free.to_vec() };
        list.release(range);
        list.free
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn released_ranges_merge() {
        assert_eq!(after_release(&[0..10, 20..30], 10..20), vec![0..30]);
        assert_eq!(after_release(&[0..10, 30..40], 10..20), vec![0..20, 30..40]);
        assert_eq!(after_release(&[0..10, 30..40], 20..30), vec![0..10, 20..40]);
        assert_eq!(after_release(&[30..40], 0..10), vec![0..10, 30..40]);
    }

    #[test]
    fn alloc_is_first_fit() {
        let mut list = FreeList::new(100);
        let a = list.alloc(30).unwrap();
        let b = list.alloc(30).unwrap();
        assert_eq!((a.clone(), b), (0..30, 30..60));
        list.release(a);
        assert_eq!(list.alloc(20), Some(0..20));
        assert_eq!(list.alloc(50), None);
        assert_eq!(list.alloc(40), Some(60..100));
    }
}
