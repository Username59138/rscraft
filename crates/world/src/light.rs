//! Vanilla-style sky light: level 15 falls straight down until something
//! blocks it, then spreads in every direction losing at least 1 per block.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::{Block, MIN_Y, SECTION_SIZE, SECTIONS_PER_COLUMN, Section};

pub const MAX_LIGHT: u8 = 15;
/// The region covers the centre column and its 8 neighbours.
const REGION: i32 = SECTION_SIZE * 3;

/// Sections of a 3×3 group of columns, bottom to top. Index of the column at
/// offset (dx, dz) is `(dx + 1) + (dz + 1) * 3`; the centre is 4.
pub type ColumnGroup = [Vec<Arc<Section>>; 9];

/// Sky light of a 3×3 column region. Coordinates are relative to the centre
/// column's origin on X/Z (-16..32) and world Y.
pub struct LightRegion {
    /// Lowest Y stored; everything below is dark.
    y0: i32,
    /// Y above the highest light-blocking block; everything above is full light.
    y1: i32,
    light: Vec<u8>,
}

fn block_at(columns: &ColumnGroup, x: i32, y: i32, z: i32) -> Block {
    let (cx, cz) = (x.div_euclid(SECTION_SIZE), z.div_euclid(SECTION_SIZE));
    let section = (y - MIN_Y) / SECTION_SIZE;
    let column = &columns[((cx + 1) + (cz + 1) * 3) as usize];
    column[section as usize].get(x.rem_euclid(SECTION_SIZE), (y - MIN_Y) % SECTION_SIZE, z.rem_euclid(SECTION_SIZE))
}

impl LightRegion {
    pub fn compute(columns: &ColumnGroup) -> Self {
        // Highest light-blocking block and where direct sunlight stops, per column.
        let mut top = vec![MIN_Y; (REGION * REGION) as usize];
        let mut lowest_sunlit = i32::MAX;
        for z in 0..REGION {
            for x in 0..REGION {
                let (lx, lz) = (x - SECTION_SIZE, z - SECTION_SIZE);
                let column =
                    &columns[((lx.div_euclid(SECTION_SIZE) + 1) + (lz.div_euclid(SECTION_SIZE) + 1) * 3) as usize];
                // Skip all-air sections from the top down.
                let mut y = MIN_Y + SECTIONS_PER_COLUMN as i32 * SECTION_SIZE - 1;
                while y >= MIN_Y {
                    let section = &column[((y - MIN_Y) / SECTION_SIZE) as usize];
                    if section.is_empty() {
                        y -= SECTION_SIZE;
                        continue;
                    }
                    if block_at(columns, lx, y, lz).light_opacity() > 0 {
                        break;
                    }
                    y -= 1;
                }
                top[(z * REGION + x) as usize] = y + 1;
                lowest_sunlit = lowest_sunlit.min(y + 1);
            }
        }

        let y1 = top.iter().copied().max().unwrap_or(MIN_Y) + 1;
        let y0 = (lowest_sunlit - MAX_LIGHT as i32).max(MIN_Y);
        let height = (y1 - y0).max(0);
        let index = |x: i32, y: i32, z: i32| (((y - y0) * REGION + z) * REGION + x) as usize;

        // Opacity of every cell, and direct sunlight.
        let volume = (REGION * REGION * height) as usize;
        let mut opacity = vec![0u8; volume];
        let mut light = vec![0u8; volume];
        for y in y0..y1 {
            for z in 0..REGION {
                for x in 0..REGION {
                    let i = index(x, y, z);
                    opacity[i] = block_at(columns, x - SECTION_SIZE, y, z - SECTION_SIZE).light_opacity();
                    if y >= top[(z * REGION + x) as usize] {
                        light[i] = MAX_LIGHT;
                    }
                }
            }
        }

        // Flood fill from sunlit cells next to anything darker.
        let mut queue = VecDeque::new();
        for y in y0..y1 {
            for z in 0..REGION {
                for x in 0..REGION {
                    if light[index(x, y, z)] == MAX_LIGHT {
                        queue.push_back((x, y, z));
                    }
                }
            }
        }
        const DIRS: [(i32, i32, i32); 6] = [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)];
        while let Some((x, y, z)) = queue.pop_front() {
            let level = light[index(x, y, z)];
            for (dx, dy, dz) in DIRS {
                let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                if !(0..REGION).contains(&nx) || !(0..REGION).contains(&nz) || !(y0..y1).contains(&ny) {
                    continue;
                }
                let n = index(nx, ny, nz);
                let next = level.saturating_sub(opacity[n].max(1));
                if opacity[n] < MAX_LIGHT && next > light[n] {
                    light[n] = next;
                    queue.push_back((nx, ny, nz));
                }
            }
        }

        Self { y0, y1, light }
    }

    /// Sky light at a position (see the type docs for coordinates).
    #[inline]
    pub fn get(&self, x: i32, y: i32, z: i32) -> u8 {
        if y >= self.y1 {
            return MAX_LIGHT;
        }
        if y < self.y0 {
            return 0;
        }
        let (rx, rz) = (x + SECTION_SIZE, z + SECTION_SIZE);
        if !(0..REGION).contains(&rx) || !(0..REGION).contains(&rz) {
            return 0;
        }
        self.light[(((y - self.y0) * REGION + rz) * REGION + rx) as usize]
    }

    /// Just the centre column's light, to keep for lighting entities.
    pub fn column(&self) -> ColumnLight {
        let mut light = Vec::with_capacity(((self.y1 - self.y0).max(0) * SECTION_SIZE * SECTION_SIZE) as usize);
        for y in self.y0..self.y1 {
            for z in 0..SECTION_SIZE {
                for x in 0..SECTION_SIZE {
                    light.push(self.get(x, y, z));
                }
            }
        }
        ColumnLight { y0: self.y0, y1: self.y1, light }
    }
}

/// Sky light of a single 16×16 column, with the same layout and limits as
/// [`LightRegion`]. Coordinates are local on X/Z (0..16) and world Y.
#[derive(Clone, Debug)]
pub struct ColumnLight {
    y0: i32,
    y1: i32,
    light: Vec<u8>,
}

impl ColumnLight {
    pub fn get(&self, x: i32, y: i32, z: i32) -> u8 {
        if y >= self.y1 {
            MAX_LIGHT
        } else if y < self.y0 {
            0
        } else {
            self.light[(((y - self.y0) * SECTION_SIZE + z) * SECTION_SIZE + x) as usize]
        }
    }
}
