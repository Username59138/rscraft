//! Loads and unloads world columns around the player. Generation and meshing
//! run on the rayon thread pool; the main thread only bookkeeps and uploads.

use std::collections::HashMap;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use glam::{IVec2, IVec3, Vec3};
use rscraft_world::{ColumnGroup, Generator, LightRegion, MIN_SECTION_Y, SECTION_SIZE, SECTIONS_PER_COLUMN, Section};

use crate::mesh::{Neighborhood, SectionMesh};
use crate::renderer::Renderer;
use crate::texture::BlockLayers;

/// Columns being generated at once. Keeps the queue short so that the
/// closest columns are always picked first when the player moves.
const MAX_GENERATING: usize = 32;

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

    generated_tx: Sender<(IVec2, Vec<Section>)>,
    generated_rx: Receiver<(IVec2, Vec<Section>)>,
    meshed_tx: Sender<(IVec3, SectionMesh)>,
    meshed_rx: Receiver<(IVec3, SectionMesh)>,
}

pub fn column_of(position: Vec3) -> IVec2 {
    IVec2::new(position.x.floor() as i32, position.z.floor() as i32).div_euclid(IVec2::splat(SECTION_SIZE))
}

impl ChunkManager {
    pub fn new(generator: Arc<Generator>, layers: Arc<BlockLayers>, render_distance: i32) -> Self {
        let r = render_distance;
        let mut load_order: Vec<IVec2> = (-r..=r)
            .flat_map(|z| (-r..=r).map(move |x| IVec2::new(x, z)))
            .filter(|o| o.length_squared() <= r * r)
            .collect();
        load_order.sort_by_key(|o| o.length_squared());

        let (generated_tx, generated_rx) = crossbeam_channel::unbounded();
        let (meshed_tx, meshed_rx) = crossbeam_channel::unbounded();
        Self {
            generator,
            layers,
            render_distance,
            load_order,
            columns: HashMap::new(),
            sections: HashMap::new(),
            generating: 0,
            generated_tx,
            generated_rx,
            meshed_tx,
            meshed_rx,
        }
    }

    pub fn loaded_columns(&self) -> usize {
        self.columns.len()
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
        while let Ok((pos, mesh)) = self.meshed_rx.try_recv() {
            // Drop meshes of columns unloaded in the meantime.
            if self.columns.get(&IVec2::new(pos.x, pos.z)) == Some(&ColumnState::Meshed) {
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
            // Light needs the whole 3×3 group, so a column is lit and meshed in one job.
            rayon::spawn(move || {
                let light = LightRegion::compute(&columns);
                for (index, section) in columns[4].iter().enumerate() {
                    if section.is_empty() {
                        continue;
                    }
                    let pos = IVec3::new(column.x, MIN_SECTION_Y + index as i32, column.y);
                    let mesh = Neighborhood::from_columns(&columns, index).build_mesh(pos, &layers, &light);
                    let _ = tx.send((pos, mesh));
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
            rayon::spawn(move || {
                let _ = tx.send((column, generator.generate_column(column.x, column.y)));
            });
        }
    }
}
