use fastnoise_lite::{FastNoiseLite, FractalType, NoiseType};

use crate::section::{SECTION_SIZE, SECTION_VOLUME, Section};
use crate::{Block, MIN_Y, SEA_LEVEL, SECTIONS_PER_COLUMN};

/// Trees reach this far from their trunk, so neighbouring columns must be scanned.
const TREE_RADIUS: i32 = 2;

/// Deterministic terrain generator. Cheap to share between threads.
pub struct Generator {
    seed: u32,
    continents: FastNoiseLite,
    hills: FastNoiseLite,
    mountains: FastNoiseLite,
    forests: FastNoiseLite,
}

fn fbm(seed: i32, frequency: f32, octaves: i32) -> FastNoiseLite {
    let mut noise = FastNoiseLite::with_seed(seed);
    noise.set_noise_type(Some(NoiseType::OpenSimplex2));
    noise.set_fractal_type(Some(FractalType::FBm));
    noise.set_fractal_octaves(Some(octaves));
    noise.set_frequency(Some(frequency));
    noise
}

/// Cheap integer hash used for per-column random decisions.
fn hash(x: i32, z: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d) ^ (z as u32).wrapping_mul(0x1656_67b1) ^ seed;
    h = (h ^ (h >> 15)).wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^ (h >> 16)
}

impl Generator {
    pub fn new(seed: u32) -> Self {
        let s = seed as i32;
        Self {
            seed,
            continents: fbm(s, 0.0025, 4),
            hills: fbm(s.wrapping_add(1), 0.012, 3),
            mountains: fbm(s.wrapping_add(2), 0.004, 3),
            forests: fbm(s.wrapping_add(3), 0.008, 2),
        }
    }

    /// Y of the topmost solid block in a column.
    pub fn surface_height(&self, x: i32, z: i32) -> i32 {
        let (fx, fz) = (x as f32, z as f32);
        let base = self.continents.get_noise_2d(fx, fz) * 24.0;
        let hills = self.hills.get_noise_2d(fx, fz) * 6.0;
        let mountain = ((self.mountains.get_noise_2d(fx, fz) - 0.15).max(0.0) / 0.85).powi(2) * 110.0;
        SEA_LEVEL + 2 + (base + hills + mountain) as i32
    }

    fn surface_block(height: i32) -> Block {
        match height {
            h if h < SEA_LEVEL - 5 => Block::Gravel,
            h if h <= SEA_LEVEL + 1 => Block::Sand,
            h if h > SEA_LEVEL + 70 => Block::Stone,
            _ => Block::Grass,
        }
    }

    /// Height of a tree trunk growing in this column, if any.
    fn tree_at(&self, x: i32, z: i32, height: i32) -> Option<i32> {
        if Self::surface_block(height) != Block::Grass {
            return None;
        }
        let density = (self.forests.get_noise_2d(x as f32, z as f32) + 0.2).max(0.0) * 0.04 + 0.002;
        let roll = hash(x, z, self.seed);
        ((roll & 0xffff) as f32 / 65536.0 < density).then(|| 4 + (roll >> 16) as i32 % 3)
    }

    /// Generates all sections of the column at chunk coordinates (cx, cz),
    /// bottom to top.
    pub fn generate_column(&self, cx: i32, cz: i32) -> Vec<Section> {
        let mut sections: Vec<Box<[Block; SECTION_VOLUME]>> =
            (0..SECTIONS_PER_COLUMN).map(|_| Box::new([Block::Air; SECTION_VOLUME])).collect();
        let (x0, z0) = (cx * SECTION_SIZE, cz * SECTION_SIZE);

        let mut set = |x: i32, y: i32, z: i32, block: Block, replace: bool| {
            let (lx, lz, sy) = (x - x0, z - z0, (y - MIN_Y) / SECTION_SIZE);
            if !(0..SECTION_SIZE).contains(&lx)
                || !(0..SECTION_SIZE).contains(&lz)
                || y < MIN_Y
                || sy as usize >= SECTIONS_PER_COLUMN
            {
                return;
            }
            let slot = &mut sections[sy as usize][Section::index(lx, (y - MIN_Y) % SECTION_SIZE, lz)];
            if replace || *slot == Block::Air {
                *slot = block;
            }
        };

        // Terrain.
        for lz in 0..SECTION_SIZE {
            for lx in 0..SECTION_SIZE {
                let (x, z) = (x0 + lx, z0 + lz);
                let height = self.surface_height(x, z);
                let top = Self::surface_block(height);
                let filler = if top == Block::Grass { Block::Dirt } else { top };
                let bedrock_top = MIN_Y + (hash(x, z, self.seed ^ 0xbed) % 4) as i32;

                for y in MIN_Y..=height.max(SEA_LEVEL - 1) {
                    let block = if y <= bedrock_top {
                        Block::Bedrock
                    } else if y > height {
                        Block::Water
                    } else if y == height {
                        top
                    } else if y > height - 4 {
                        filler
                    } else {
                        Block::Stone
                    };
                    set(x, y, z, block, true);
                }
            }
        }

        // Trees, including those rooted in neighbouring columns whose leaves reach in.
        for z in z0 - TREE_RADIUS..z0 + SECTION_SIZE + TREE_RADIUS {
            for x in x0 - TREE_RADIUS..x0 + SECTION_SIZE + TREE_RADIUS {
                let ground = self.surface_height(x, z);
                let Some(trunk) = self.tree_at(x, z, ground) else { continue };
                let top = ground + trunk;
                for y in ground + 1..=top {
                    set(x, y, z, Block::OakLog, true);
                }
                set(x, ground, z, Block::Dirt, true);
                for y in top - 2..=top + 1 {
                    let radius: i32 = if y < top { 2 } else { 1 };
                    for dz in -radius..=radius {
                        for dx in -radius..=radius {
                            let corner = dx.abs() == radius && dz.abs() == radius;
                            // Randomly trim corners so trees don't look like cubes.
                            if corner && (y > top || hash(x + dx, z + dz, self.seed ^ y as u32).is_multiple_of(2)) {
                                continue;
                            }
                            set(x + dx, y, z + dz, Block::OakLeaves, false);
                        }
                    }
                }
            }
        }

        sections.into_iter().map(Section::from_blocks).collect()
    }
}
