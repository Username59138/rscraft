//! Random block ticks, like vanilla's ServerLevel.tickChunk: every game tick,
//! a few random blocks of each section near the player get a chance to do
//! something. For now that is grass spreading onto dirt and dying under
//! blocks (SpreadingSnowyBlock).

use glam::IVec3;

use crate::{Block, MAX_LIGHT, SECTION_SIZE, Section};

/// Blocks picked per section per tick: vanilla's `random_tick_speed` rule.
pub const RANDOM_TICK_SPEED: u32 = 3;
/// Light grass needs above it to spread.
const GRASS_SPREAD_LIGHT: u8 = 9;

/// The world as random ticks see it.
pub trait RandomTickWorld {
    /// None where the world isn't loaded.
    fn block(&self, pos: IVec3) -> Option<Block>;
    fn set_block(&mut self, pos: IVec3, block: Block);
    /// Raw sky light level, 0..=15, not darkened by the time of day.
    fn sky_light(&self, pos: IVec3) -> u8;
}

/// How much the sky light is darkened at `time` (fraction of the day, 0 =
/// sunrise): 0 by day, 11 at night. Follows the overworld's
/// `sky_light_level` track in vanilla's `timeline/day.json`.
pub fn sky_darken(time: f32) -> u8 {
    // (tick, sky light factor), repeating every 24000 ticks.
    const KEYFRAMES: [(f32, f32); 4] = [(133.0, 1.0), (11867.0, 1.0), (13670.0, 4.0 / 15.0), (22330.0, 4.0 / 15.0)];
    const DAY: f32 = 24000.0;
    let tick = time.rem_euclid(1.0) * DAY;
    let (first, last) = (KEYFRAMES[0], KEYFRAMES[KEYFRAMES.len() - 1]);
    // The keyframes on either side of `tick`, wrapping around midnight.
    let (from, to) = match KEYFRAMES.iter().position(|k| tick < k.0) {
        Some(0) => ((last.0 - DAY, last.1), first),
        Some(i) => (KEYFRAMES[i - 1], KEYFRAMES[i]),
        None => (last, (first.0 + DAY, first.1)),
    };
    let factor = from.1 + (to.1 - from.1) * (tick - from.0) / (to.0 - from.0);
    (MAX_LIGHT as f32 - MAX_LIGHT as f32 * factor) as u8
}

/// The random number sources of random ticks.
pub struct RandomTicks {
    /// Vanilla's linear congruential generator for picking block positions.
    position_seed: i32,
    /// For everything else, such as where grass spreads.
    rng: u64,
}

impl RandomTicks {
    pub fn new(seed: u64) -> Self {
        Self { position_seed: seed as i32, rng: seed | 1 }
    }

    /// A random block of the section at section coordinates `section`
    /// (Level.getBlockRandomPos).
    fn block_in(&mut self, section: IVec3) -> IVec3 {
        self.position_seed = self.position_seed.wrapping_mul(3).wrapping_add(1013904223);
        let v = self.position_seed >> 2;
        section * SECTION_SIZE + IVec3::new(v & 15, (v >> 16) & 15, (v >> 8) & 15)
    }

    /// 0..n
    fn below(&mut self, n: i32) -> i32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        ((self.rng.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 33) % n as u64) as i32
    }

    /// Random ticks for the section `blocks` at section coordinates
    /// `section`. `darken` is the time of day's `sky_darken`.
    pub fn tick_section(&mut self, world: &mut impl RandomTickWorld, section: IVec3, blocks: &Section, darken: u8) {
        for _ in 0..RANDOM_TICK_SPEED {
            let pos = self.block_in(section);
            let local = pos - section * SECTION_SIZE;
            // `blocks` may be outdated by earlier ticks; the world is not.
            if blocks.get(local.x, local.y, local.z) == Block::Grass && world.block(pos) == Some(Block::Grass) {
                self.tick_grass(world, pos, darken);
            }
        }
    }

    fn tick_grass(&mut self, world: &mut impl RandomTickWorld, pos: IVec3, darken: u8) {
        if !grass_survives(world, pos) {
            world.set_block(pos, Block::Dirt);
            return;
        }
        if world.sky_light(pos + IVec3::Y).saturating_sub(darken) < GRASS_SPREAD_LIGHT {
            return;
        }
        for _ in 0..4 {
            let target = pos + IVec3::new(self.below(3) - 1, self.below(5) - 3, self.below(3) - 1);
            let above_is_water = world.block(target + IVec3::Y).is_some_and(Block::is_water);
            if world.block(target) == Some(Block::Dirt) && grass_survives(world, target) && !above_is_water {
                world.set_block(target, Block::Grass);
            }
        }
    }
}

/// Grass dies under an opaque block or a full block of water.
fn grass_survives(world: &impl RandomTickWorld, pos: IVec3) -> bool {
    match world.block(pos + IVec3::Y) {
        // Sources and falling water are full; thinner flowing water isn't.
        Some(Block::Water(level)) => (1..=7).contains(&level),
        Some(above) => !above.is_opaque(),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// Dirt at y = 0 and below, air above, full sky light everywhere.
    struct TestWorld(HashMap<IVec3, Block>);

    impl RandomTickWorld for TestWorld {
        fn block(&self, pos: IVec3) -> Option<Block> {
            Some(self.0.get(&pos).copied().unwrap_or(if pos.y <= 0 { Block::Dirt } else { Block::Air }))
        }

        fn set_block(&mut self, pos: IVec3, block: Block) {
            self.0.insert(pos, block);
        }

        fn sky_light(&self, _: IVec3) -> u8 {
            MAX_LIGHT
        }
    }

    fn section_of(world: &TestWorld, section: IVec3) -> Section {
        let mut blocks = Section::Uniform(Block::Air);
        for (x, y, z) in cube(SECTION_SIZE) {
            let block = world.block(section * SECTION_SIZE + IVec3::new(x, y, z)).unwrap();
            blocks.set(x, y, z, block);
        }
        blocks
    }

    fn cube(n: i32) -> impl Iterator<Item = (i32, i32, i32)> {
        (0..n).flat_map(move |y| (0..n).flat_map(move |z| (0..n).map(move |x| (x, y, z))))
    }

    fn run(world: &mut TestWorld, ticks: u32, darken: u8) {
        let mut random = RandomTicks::new(42);
        for _ in 0..ticks {
            for x in -1..=1 {
                for z in -1..=1 {
                    let section = IVec3::new(x, 0, z);
                    let blocks = section_of(world, section);
                    random.tick_section(world, section, &blocks, darken);
                }
            }
        }
    }

    #[test]
    fn grass_spreads_over_dirt_by_day_only() {
        let mut world = TestWorld(HashMap::from([(IVec3::new(8, 0, 8), Block::Grass)]));
        run(&mut world, 20 * 60 * 5, sky_darken(0.75));
        assert_eq!(world.0.len(), 1, "no spreading at night");
        run(&mut world, 20 * 60 * 5, sky_darken(0.25));
        // Each block is ticked about once a minute, so growth is slow.
        let grass = world.0.values().filter(|&&b| b == Block::Grass).count();
        assert!(grass > 5, "only {grass} grass blocks after five minutes");
    }

    #[test]
    fn grass_dies_under_opaque_blocks_and_water() {
        let mut world = TestWorld(HashMap::new());
        let covered = [(IVec3::new(2, 0, 2), Block::Stone), (IVec3::new(9, 0, 9), Block::WATER)];
        let open = [(IVec3::new(5, 0, 5), Block::OakLeaves), (IVec3::new(12, 0, 12), Block::Water(5))];
        for (pos, above) in covered.into_iter().chain(open) {
            world.0.insert(pos, Block::Grass);
            world.0.insert(pos + IVec3::Y, above);
        }
        // Dark, so nothing spreads and only dying shows.
        run(&mut world, 20 * 60 * 5, MAX_LIGHT);
        for (pos, _) in covered {
            assert_eq!(world.0[&pos], Block::Dirt);
        }
        for (pos, _) in open {
            assert_eq!(world.0[&pos], Block::Grass);
        }
    }

    #[test]
    fn sky_darkens_at_night() {
        assert_eq!(sky_darken(0.25), 0);
        assert_eq!(sky_darken(0.75), 11);
        assert_eq!(sky_darken(0.0), 0);
        // Dusk, halfway down.
        assert_eq!(sky_darken(0.53), 5);
        assert_eq!(sky_darken(0.99), 2);
    }
}
