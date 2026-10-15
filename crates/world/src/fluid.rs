//! Flowing water, with vanilla's rules (FlowingFluid, WaterFluid): a fluid
//! block updates 5 ticks after something next to it changed, falls when it
//! can, otherwise spreads sideways towards the nearest drop within 4 blocks,
//! losing one level per block, and two sources over solid ground make a new
//! source.

use std::collections::{BTreeMap, HashSet};

use glam::IVec3;

use crate::Block;

/// Ticks between a water block noticing a change and reacting to it.
pub const WATER_TICK_DELAY: u64 = 5;
/// How far sideways flowing water looks for a way down.
const SLOPE_FIND_DISTANCE: u32 = 4;
/// Level lost per block of sideways flow.
const DROP_OFF: u8 = 1;
/// The amount of a falling or source block, and the most a block can hold.
const FULL: u8 = 8;
/// More than any real slope distance.
const NO_SLOPE: u32 = 1000;

/// Directions water spreads in, in vanilla's order (north, east, south, west).
const HORIZONTAL: [IVec3; 4] = [IVec3::NEG_Z, IVec3::X, IVec3::Z, IVec3::NEG_X];

/// The world as the fluid simulation sees it.
pub trait FluidWorld {
    /// None where the world isn't loaded.
    fn block(&self, pos: IVec3) -> Option<Block>;
    /// Returns false where the world isn't loaded.
    fn set_block(&mut self, pos: IVec3, block: Block) -> bool;
}

/// The fluid part of a block: vanilla's FluidState for water.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fluid {
    /// 1..=8; 8 for sources and falling water.
    amount: u8,
    source: bool,
    falling: bool,
}

impl Fluid {
    const SOURCE: Fluid = Fluid { amount: FULL, source: true, falling: false };

    fn flowing(amount: u8, falling: bool) -> Self {
        Self { amount, source: false, falling }
    }

    fn of(block: Block) -> Option<Self> {
        match block {
            Block::Water(0) => Some(Fluid::SOURCE),
            Block::Water(level @ 1..=7) => Some(Fluid::flowing(FULL - level, false)),
            Block::Water(_) => Some(Fluid::flowing(FULL, true)),
            _ => None,
        }
    }

    fn block(self) -> Block {
        if self.source {
            Block::WATER
        } else {
            Block::Water(FULL - self.amount.min(FULL) + if self.falling { 8 } else { 0 })
        }
    }
}

/// Height of a water block's surface without water above it, 0..1.
pub fn own_height(block: Block) -> f32 {
    Fluid::of(block).map_or(0.0, |f| f.amount as f32 / 9.0)
}

fn is_source(block: Block) -> bool {
    block == Block::WATER
}

/// Blocks water can flow into or through: vanilla's `washed_away_by_fluids`
/// tag, of which we only have air and water.
fn can_hold_fluid(block: Block) -> bool {
    matches!(block, Block::Air | Block::Water(_))
}

/// Whether water can get from one block into the next. Every solid block we
/// have is a full cube, which walls water off completely.
fn passes_wall(from: Block, to: Block) -> bool {
    !from.is_solid() && !to.is_solid()
}

/// Scheduled fluid updates, run by game tick like vanilla's LevelTicks.
#[derive(Default)]
pub struct FluidTicks {
    /// (due tick, scheduling order) → position.
    queue: BTreeMap<(u64, u64), IVec3>,
    scheduled: HashSet<IVec3>,
    order: u64,
}

impl FluidTicks {
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Positions with an update still to come, to save with the world.
    pub fn pending(&self) -> impl Iterator<Item = IVec3> + '_ {
        self.queue.values().copied()
    }

    /// Schedules an update of `pos` at tick `due`, unless one is already coming.
    pub fn schedule(&mut self, pos: IVec3, due: u64) {
        if self.scheduled.insert(pos) {
            self.queue.insert((due, self.order), pos);
            self.order += 1;
        }
    }

    /// Something changed at `pos`: the water there and next to it reacts,
    /// like vanilla's neighbour and shape updates.
    pub fn block_changed(&mut self, world: &impl FluidWorld, pos: IVec3, now: u64) {
        for p in std::iter::once(pos).chain(neighbours(pos)) {
            if world.block(p).is_some_and(Block::is_water) {
                self.schedule(p, now + WATER_TICK_DELAY);
            }
        }
    }

    /// Runs the updates due by tick `now`.
    pub fn run(&mut self, world: &mut impl FluidWorld, now: u64) {
        let later = self.queue.split_off(&(now + 1, 0));
        let due = std::mem::replace(&mut self.queue, later);
        for pos in due.into_values() {
            self.scheduled.remove(&pos);
            if world.block(pos).is_none() {
                // Not loaded (yet): try again once it may be.
                self.schedule(pos, now + 20);
                continue;
            }
            let mut sim = Simulation { world: &mut *world, ticks: &mut *self, now };
            sim.tick(pos);
        }
    }
}

fn neighbours(pos: IVec3) -> impl Iterator<Item = IVec3> {
    HORIZONTAL.into_iter().chain([IVec3::Y, IVec3::NEG_Y]).map(move |d| pos + d)
}

struct Simulation<'a, W> {
    world: &'a mut W,
    ticks: &'a mut FluidTicks,
    now: u64,
}

impl<W: FluidWorld> Simulation<'_, W> {
    /// Unloaded blocks act like solid walls.
    fn get(&self, pos: IVec3) -> Block {
        self.world.block(pos).unwrap_or(Block::Bedrock)
    }

    fn set(&mut self, pos: IVec3, block: Block) {
        if self.get(pos) != block && self.world.set_block(pos, block) {
            self.ticks.block_changed(&*self.world, pos, self.now);
        }
    }

    /// An update of the water at `pos` (FlowingFluid.tick): flowing water
    /// first settles to what its neighbours feed it, then spreads.
    fn tick(&mut self, pos: IVec3) {
        let mut block = self.get(pos);
        let Some(mut fluid) = Fluid::of(block) else { return };
        if !fluid.source {
            match self.new_fluid(pos) {
                None => {
                    self.set(pos, Block::Air);
                    return;
                }
                Some(new) if new != fluid => {
                    fluid = new;
                    block = new.block();
                    self.set(pos, block);
                }
                Some(_) => {}
            }
        }
        self.spread(pos, block, fluid);
    }

    /// The fluid that `pos` would hold given its neighbours (getNewLiquid).
    fn new_fluid(&self, pos: IVec3) -> Option<Fluid> {
        let block = self.get(pos);
        let mut highest = 0;
        let mut sources = 0;
        for dir in HORIZONTAL {
            let neighbour = self.get(pos + dir);
            if let Some(fluid) = Fluid::of(neighbour).filter(|_| passes_wall(block, neighbour)) {
                sources += fluid.source as u32;
                highest = highest.max(fluid.amount);
            }
        }
        if sources >= 2 {
            let below = self.get(pos - IVec3::Y);
            if below.is_solid() || is_source(below) {
                return Some(Fluid::SOURCE);
            }
        }
        let above = self.get(pos + IVec3::Y);
        if above.is_water() && passes_wall(block, above) {
            return Some(Fluid::flowing(FULL, true));
        }
        let amount = highest.saturating_sub(DROP_OFF);
        (amount > 0).then(|| Fluid::flowing(amount, false))
    }

    /// Whether water at `from` could flow into `to` (canMaybePassThrough).
    fn can_flow(&self, from: Block, to: Block) -> bool {
        !is_source(to) && can_hold_fluid(to) && passes_wall(from, to)
    }

    /// A block water could fall out of: open, with room below.
    fn is_hole(&self, pos: IVec3) -> bool {
        let below = self.get(pos - IVec3::Y);
        passes_wall(self.get(pos), below) && can_hold_fluid(below)
    }

    fn spread(&mut self, pos: IVec3, block: Block, fluid: Fluid) {
        let below_pos = pos - IVec3::Y;
        let below = self.get(below_pos);
        if self.can_flow(block, below) {
            let new = self.new_fluid(below_pos);
            // Water only replaces air below it, not other flowing water.
            if let (Some(new), Block::Air) = (new, below) {
                self.set(below_pos, new.block());
                // A pool's edge keeps spreading even while it pours down.
                if self.source_neighbours(pos) >= 3 {
                    self.spread_to_sides(pos, block, fluid);
                }
                return;
            }
        }
        if fluid.source || !self.is_hole(pos) {
            self.spread_to_sides(pos, block, fluid);
        }
    }

    fn source_neighbours(&self, pos: IVec3) -> usize {
        HORIZONTAL.iter().filter(|&&d| is_source(self.get(pos + d))).count()
    }

    fn spread_to_sides(&mut self, pos: IVec3, block: Block, fluid: Fluid) {
        let next = if fluid.falling { 7 } else { fluid.amount.saturating_sub(DROP_OFF) };
        if next == 0 {
            return;
        }
        for (dir, new) in self.spread_targets(pos, block) {
            self.set(pos + dir, new.map_or(Block::Air, Fluid::block));
        }
    }

    /// The sideways neighbours water flows into: of those it can reach,
    /// the ones closest to a way down (getSpread).
    fn spread_targets(&self, pos: IVec3, block: Block) -> Vec<(IVec3, Option<Fluid>)> {
        let mut lowest = NO_SLOPE;
        let mut targets = Vec::new();
        for dir in HORIZONTAL {
            let target = self.get(pos + dir);
            if !self.can_flow(block, target) {
                continue;
            }
            let distance = if self.is_hole(pos + dir) { 0 } else { self.slope_distance(pos + dir, 1, -dir, target) };
            if distance < lowest {
                targets.clear();
            }
            if distance <= lowest {
                // Flowing water is never replaced sideways, only air is.
                if target == Block::Air {
                    targets.push((dir, self.new_fluid(pos + dir)));
                }
                lowest = distance;
            }
        }
        targets
    }

    /// Steps from `pos` to the nearest hole, not going back the way it came
    /// (`from`); `NO_SLOPE` if there is none within reach.
    fn slope_distance(&self, pos: IVec3, pass: u32, from: IVec3, block: Block) -> u32 {
        let mut lowest = NO_SLOPE;
        for dir in HORIZONTAL.into_iter().filter(|&d| d != from) {
            let target = self.get(pos + dir);
            if !self.can_flow(block, target) {
                continue;
            }
            if self.is_hole(pos + dir) {
                return pass;
            }
            if pass < SLOPE_FIND_DISTANCE {
                lowest = lowest.min(self.slope_distance(pos + dir, pass + 1, -dir, target));
            }
        }
        lowest
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// A stone floor at y = 0 within a 32×32 area, air above it.
    struct TestWorld {
        blocks: HashMap<IVec3, Block>,
    }

    impl TestWorld {
        fn new() -> Self {
            Self { blocks: HashMap::new() }
        }

        fn run(&mut self, ticks: &mut FluidTicks, until: u64) {
            for now in 0..=until {
                ticks.run(self, now);
            }
        }
    }

    impl FluidWorld for TestWorld {
        fn block(&self, pos: IVec3) -> Option<Block> {
            if pos.x.abs() > 16 || pos.z.abs() > 16 || pos.y < -8 {
                return None;
            }
            Some(self.blocks.get(&pos).copied().unwrap_or(if pos.y <= 0 { Block::Stone } else { Block::Air }))
        }

        fn set_block(&mut self, pos: IVec3, block: Block) -> bool {
            self.blocks.insert(pos, block);
            true
        }
    }

    fn place(world: &mut TestWorld, ticks: &mut FluidTicks, pos: IVec3, block: Block) {
        world.set_block(pos, block);
        ticks.block_changed(world, pos, 0);
    }

    #[test]
    fn source_spreads_seven_blocks_on_flat_ground() {
        let (mut world, mut ticks) = (TestWorld::new(), FluidTicks::default());
        place(&mut world, &mut ticks, IVec3::new(0, 1, 0), Block::WATER);
        world.run(&mut ticks, 200);
        for x in 1..=7 {
            assert_eq!(world.block(IVec3::new(x, 1, 0)), Some(Block::Water(x as u8)), "x = {x}");
        }
        assert_eq!(world.block(IVec3::new(8, 1, 0)), Some(Block::Air));
        // Diagonally it takes two steps per block.
        assert_eq!(world.block(IVec3::new(3, 1, 3)), Some(Block::Water(6)));
        assert!(ticks.is_empty());
    }

    #[test]
    fn water_falls_and_spreads_below() {
        let (mut world, mut ticks) = (TestWorld::new(), FluidTicks::default());
        for y in 1..=4 {
            world.set_block(IVec3::new(5, y, 5), Block::Stone);
        }
        place(&mut world, &mut ticks, IVec3::new(5, 5, 5), Block::WATER);
        world.run(&mut ticks, 400);
        // Falling water down the side of the pillar, flowing on the ground.
        assert_eq!(world.block(IVec3::new(6, 4, 5)), Some(Block::Water(8)));
        assert_eq!(world.block(IVec3::new(6, 1, 5)), Some(Block::Water(8)));
        assert_eq!(world.block(IVec3::new(7, 1, 5)), Some(Block::Water(1)));
    }

    #[test]
    fn flow_heads_for_the_nearest_hole() {
        let (mut world, mut ticks) = (TestWorld::new(), FluidTicks::default());
        world.set_block(IVec3::new(2, 0, 0), Block::Air);
        place(&mut world, &mut ticks, IVec3::new(0, 1, 0), Block::WATER);
        world.run(&mut ticks, 30);
        // Only towards the hole, not the other three ways.
        assert_eq!(world.block(IVec3::new(1, 1, 0)), Some(Block::Water(1)));
        for dir in [IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z] {
            assert_eq!(world.block(IVec3::new(0, 1, 0) + dir), Some(Block::Air));
        }
    }

    #[test]
    fn two_sources_make_a_third() {
        let (mut world, mut ticks) = (TestWorld::new(), FluidTicks::default());
        // A 3-long trench, with sources at both ends.
        for x in -1..=3 {
            for z in [-1, 1] {
                world.set_block(IVec3::new(x, 1, z), Block::Stone);
            }
        }
        world.set_block(IVec3::new(-1, 1, 0), Block::Stone);
        world.set_block(IVec3::new(3, 1, 0), Block::Stone);
        place(&mut world, &mut ticks, IVec3::new(0, 1, 0), Block::WATER);
        place(&mut world, &mut ticks, IVec3::new(2, 1, 0), Block::WATER);
        world.run(&mut ticks, 50);
        assert_eq!(world.block(IVec3::new(1, 1, 0)), Some(Block::WATER));
    }

    #[test]
    fn flow_dries_up_without_a_source() {
        let (mut world, mut ticks) = (TestWorld::new(), FluidTicks::default());
        let source = IVec3::new(0, 1, 0);
        place(&mut world, &mut ticks, source, Block::WATER);
        world.run(&mut ticks, 200);
        place(&mut world, &mut ticks, source, Block::Air);
        world.run(&mut ticks, 400);
        assert!(world.blocks.values().all(|&b| !b.is_water()));
    }
}
