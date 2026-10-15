//! Loads and unloads world columns around the player. Generation and meshing
//! run on the rayon thread pool; the main thread only bookkeeps and uploads.

use std::collections::HashMap;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use glam::{IVec2, IVec3, Vec3};
use rscraft_world::{Block, ColumnGroup, ColumnLight, Generator, LightRegion, MIN_SECTION_Y, SECTION_SIZE, SECTIONS_PER_COLUMN, Section};

use crate::mesh::{Neighborhood, SectionMesh};
use crate::renderer::Renderer;
use crate::texture::BlockLayers;

/// Columns being generated at once. Keeps the queue short so that the
/// closest columns are always picked first when the player moves.
const MAX_GENERATING: usize = 32;

/// Blocks changed by the player, per column, by world position. Applied on
/// top of freshly generated terrain and saved with the world.
pub type Edits = HashMap<IVec2, HashMap<IVec3, Block>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColumnState {
    Generating,
    /// Generated, waiting for its neighbours before meshing.
    Generated,
    Meshed,
}

pub struct ChunkManager {
    generator: Arc<Generator>,
    layers: Arc<BlockLayers>,
    render_distance: i32,
    /// Column offsets within render distance, closest first.
    load_order: Vec<IVec2>,

    columns: HashMap<IVec2, ColumnState>,
    sections: HashMap<IVec3, Arc<Section>>,
    generating: usize,
    edits: Edits,
    /// Bumped every time a column is (re)meshed, so results of outdated
    /// meshing jobs can be told apart and dropped.
    mesh_versions: HashMap<IVec2, u32>,

    generated_tx: Sender<(IVec2, Vec<Section>)>,
    generated_rx: Receiver<(IVec2, Vec<Section>)>,
    meshed_tx: Sender<(IVec3, u32, SectionMesh)>,
    meshed_rx: Receiver<(IVec3, u32, SectionMesh)>,
    /// Sky light of each meshed column, for lighting entities.
    light: HashMap<IVec2, ColumnLight>,
    lit_tx: Sender<(IVec2, u32, ColumnLight)>,
    lit_rx: Receiver<(IVec2, u32, ColumnLight)>,
}

pub fn column_of(position: Vec3) -> IVec2 {
    IVec2::new(position.x.floor() as i32, position.z.floor() as i32).div_euclid(IVec2::splat(SECTION_SIZE))
}

impl ChunkManager {
    pub fn new(generator: Arc<Generator>, layers: Arc<BlockLayers>, render_distance: i32, edits: Edits) -> Self {
        let load_order = Self::load_order(render_distance);
        let (generated_tx, generated_rx) = crossbeam_channel::unbounded();
        let (meshed_tx, meshed_rx) = crossbeam_channel::unbounded();
        let (lit_tx, lit_rx) = crossbeam_channel::unbounded();
        Self {
            generator,
            layers,
            render_distance,
            load_order,
            columns: HashMap::new(),
            sections: HashMap::new(),
            generating: 0,
            edits,
            mesh_versions: HashMap::new(),
            generated_tx,
            generated_rx,
            meshed_tx,
            meshed_rx,
            light: HashMap::new(),
            lit_tx,
            lit_rx,
        }
    }

    fn load_order(r: i32) -> Vec<IVec2> {
        let mut order: Vec<IVec2> = (-r..=r)
            .flat_map(|z| (-r..=r).map(move |x| IVec2::new(x, z)))
            .filter(|o| o.length_squared() <= r * r)
            .collect();
        order.sort_by_key(|o| o.length_squared());
        order
    }

    pub fn set_render_distance(&mut self, render_distance: i32) {
        self.render_distance = render_distance;
        self.load_order = Self::load_order(render_distance);
    }

    /// The block at a world position, or None if its chunk isn't loaded.
    pub fn block_at(&self, pos: IVec3) -> Option<Block> {
        if pos.y < rscraft_world::MIN_Y || pos.y >= rscraft_world::MAX_Y {
            return Some(Block::Air);
        }
        let section = pos.div_euclid(IVec3::splat(SECTION_SIZE));
        let local = pos.rem_euclid(IVec3::splat(SECTION_SIZE));
        self.sections.get(&section).map(|s| s.get(local.x, local.y, local.z))
    }

    /// Changes a block, records the edit and re-meshes the columns whose
    /// geometry or light it can affect. Returns false if it isn't loaded.
    pub fn set_block(&mut self, pos: IVec3, block: Block) -> bool {
        let section_pos = pos.div_euclid(IVec3::splat(SECTION_SIZE));
        let local = pos.rem_euclid(IVec3::splat(SECTION_SIZE));
        let Some(section) = self.sections.get_mut(&section_pos) else { return false };
        Arc::make_mut(section).set(local.x, local.y, local.z, block);

        let column = IVec2::new(section_pos.x, section_pos.z);
        self.edits.entry(column).or_default().insert(pos, block);
        // Light spreads up to 15 blocks, so all neighbouring columns may change.
        for dz in -1..=1 {
            for dx in -1..=1 {
                let c = column + IVec2::new(dx, dz);
                if self.columns.get(&c) == Some(&ColumnState::Meshed) {
                    self.columns.insert(c, ColumnState::Generated);
                }
            }
        }
        true
    }

    pub fn edits(&self) -> &Edits {
        &self.edits
    }

    /// Sky light level (0..=15) at a world position; full light where it
    /// isn't known yet.
    pub fn sky_light_at(&self, pos: IVec3) -> u8 {
        let column = IVec2::new(pos.x, pos.z).div_euclid(IVec2::splat(SECTION_SIZE));
        let local = IVec2::new(pos.x, pos.z).rem_euclid(IVec2::splat(SECTION_SIZE));
        self.light.get(&column).map_or(rscraft_world::MAX_LIGHT, |l| l.get(local.x, pos.y, local.y))
    }

    pub fn loaded_columns(&self) -> usize {
        self.columns.len()
    }

    /// Fraction of the columns within `radius` of `player` that are meshed.
    pub fn progress(&self, player: Vec3, radius: i32) -> f32 {
        let center = column_of(player);
        let offsets = Self::load_order(radius);
        let done = offsets.iter().filter(|&&o| self.columns.get(&(center + o)) == Some(&ColumnState::Meshed)).count();
        done as f32 / offsets.len() as f32
    }

    fn in_range(&self, column: IVec2, center: IVec2, margin: i32) -> bool {
        let r = self.render_distance + margin;
        (column - center).length_squared() <= r * r
    }

    pub fn update(&mut self, player: Vec3, renderer: &mut Renderer) {
        let center = column_of(player);
        self.receive_generated(center);
        self.receive_meshes(renderer);
        self.unload_far(center, renderer);
        self.schedule_meshing();
        self.schedule_generation(center);
    }

    fn receive_generated(&mut self, center: IVec2) {
        while let Ok((column, sections)) = self.generated_rx.try_recv() {
            self.generating -= 1;
            // The player may have moved away while it was generating.
            if !self.in_range(column, center, 1) {
                self.columns.remove(&column);
                continue;
            }
            for (i, section) in sections.into_iter().enumerate() {
                let pos = IVec3::new(column.x, MIN_SECTION_Y + i as i32, column.y);
                self.sections.insert(pos, Arc::new(section));
            }
            self.columns.insert(column, ColumnState::Generated);
        }
    }

    fn receive_meshes(&mut self, renderer: &mut Renderer) {
        while let Ok((column, version, light)) = self.lit_rx.try_recv() {
            if self.columns.contains_key(&column) && self.mesh_versions.get(&column) == Some(&version) {
                self.light.insert(column, light);
            }
        }
        while let Ok((pos, version, mesh)) = self.meshed_rx.try_recv() {
            // Drop meshes of unloaded columns and of outdated jobs.
            let column = IVec2::new(pos.x, pos.z);
            if self.columns.contains_key(&column) && self.mesh_versions.get(&column) == Some(&version) {
                renderer.upload_section(pos, &mesh);
            }
        }
    }

    fn unload_far(&mut self, center: IVec2, renderer: &mut Renderer) {
        let far: Vec<IVec2> = self
            .columns
            .iter()
            .filter(|&(&c, &state)| state != ColumnState::Generating && !self.in_range(c, center, 1))
            .map(|(&c, _)| c)
            .collect();
        for column in far {
            self.columns.remove(&column);
            self.mesh_versions.remove(&column);
            self.light.remove(&column);
            for i in 0..SECTIONS_PER_COLUMN as i32 {
                self.sections.remove(&IVec3::new(column.x, MIN_SECTION_Y + i, column.y));
            }
            renderer.remove_column(column);
        }
    }

    /// Meshes every generated column whose eight neighbours are generated too,
    /// so border faces are culled and ambient occlusion is correct.
    fn schedule_meshing(&mut self) {
        let ready: Vec<IVec2> = self
            .columns
            .iter()
            .filter(|&(_, &state)| state == ColumnState::Generated)
            .map(|(&c, _)| c)
            .filter(|c| {
                (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| IVec2::new(dx, dz))).all(|d| {
                    matches!(self.columns.get(&(c + d)), Some(ColumnState::Generated | ColumnState::Meshed))
                })
            })
            .collect();

        for column in ready {
            self.columns.insert(column, ColumnState::Meshed);
            let columns: ColumnGroup = std::array::from_fn(|i| {
                let c = column + IVec2::new(i as i32 % 3 - 1, i as i32 / 3 - 1);
                (0..SECTIONS_PER_COLUMN as i32)
                    .map(|y| self.sections[&IVec3::new(c.x, MIN_SECTION_Y + y, c.y)].clone())
                    .collect()
            });
            let layers = self.layers.clone();
            let tx = self.meshed_tx.clone();
            let lit_tx = self.lit_tx.clone();
            let version = self.mesh_versions.entry(column).or_insert(0);
            *version += 1;
            let version = *version;
            // Light needs the whole 3×3 group, so a column is lit and meshed in one job.
            rayon::spawn(move || {
                let light = LightRegion::compute(&columns);
                let _ = lit_tx.send((column, version, light.column()));
                for (index, section) in columns[4].iter().enumerate() {
                    let pos = IVec3::new(column.x, MIN_SECTION_Y + index as i32, column.y);
                    // Empty sections are sent too, so a re-mesh clears what was dug out.
                    let mesh = if section.is_empty() {
                        SectionMesh::default()
                    } else {
                        Neighborhood::from_columns(&columns, index).build_mesh(pos, &layers, &light)
                    };
                    let _ = tx.send((pos, version, mesh));
                }
            });
        }
    }

    fn schedule_generation(&mut self, center: IVec2) {
        for i in 0..self.load_order.len() {
            if self.generating >= MAX_GENERATING {
                break;
            }
            let column = center + self.load_order[i];
            if self.columns.contains_key(&column) {
                continue;
            }
            self.columns.insert(column, ColumnState::Generating);
            self.generating += 1;
            let generator = self.generator.clone();
            let tx = self.generated_tx.clone();
            let edits = self.edits.get(&column).cloned().unwrap_or_default();
            rayon::spawn(move || {
                let mut sections = generator.generate_column(column.x, column.y);
                for (pos, block) in edits {
                    let local = pos.rem_euclid(IVec3::splat(SECTION_SIZE));
                    let index = (pos.y.div_euclid(SECTION_SIZE) - MIN_SECTION_Y) as usize;
                    if let Some(section) = sections.get_mut(index) {
                        section.set(local.x, local.y, local.z, block);
                    }
                }
                let _ = tx.send((column, sections));
            });
        }
    }
}
