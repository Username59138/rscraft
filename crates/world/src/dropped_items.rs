//! Items lying in the world: they fall and fly into the player's inventory
//! when they come close. Drawing them is up to the client.

use glam::{IVec3, Vec3};

use crate::inventory::Inventory;
use crate::{Block, TICK_SECONDS};

/// Half the size of a dropped item's box (vanilla draws them at 1/4 scale).
pub const HALF_SIZE: f32 = 0.125;
const GRAVITY: f32 = 0.04;
const PICKUP_RANGE: f32 = 1.5;
/// Items can't be picked up for 10 ticks after they appear, like vanilla.
const PICKUP_DELAY: f32 = 0.5;
/// Items disappear after 5 minutes.
const LIFETIME: f32 = 300.0;
/// Picked-up items fly into the player over 3 ticks (ItemPickupParticle).
const PICKUP_SECONDS: f32 = 3.0 * TICK_SECONDS;
/// Picked-up items fly to this height above the player's feet.
const PICKUP_TARGET_HEIGHT: f32 = 0.5;

/// A picked-up item on its way into the player; already in the inventory.
struct Pickup {
    block: Block,
    from: Vec3,
    elapsed: f32,
}

struct DroppedItem {
    block: Block,
    count: u8,
    position: Vec3,
    /// Position at the previous tick, for smooth rendering between ticks.
    previous: Vec3,
    velocity: Vec3,
    on_ground: bool,
    age: f32,
}

#[derive(Default)]
pub struct DroppedItems {
    items: Vec<DroppedItem>,
    pickups: Vec<Pickup>,
    accumulator: f32,
}

/// Tiny pseudo-random number in -1..1 for scattering drops.
fn jitter(seed: f32) -> f32 {
    // rem_euclid, not fract: fract keeps the sign, which made this -3..1.
    ((seed * 12.9898).sin() * 43758.547).rem_euclid(1.0) * 2.0 - 1.0
}

impl DroppedItems {
    /// Drops an item at the centre of a block, with a little random hop.
    pub fn spawn_at_block(&mut self, pos: IVec3, block: Block, count: u8) {
        let seed = (pos.x * 73 + pos.y * 19 + pos.z * 31 + self.items.len() as i32) as f32;
        // Like vanilla: somewhere around the centre, hopping up and out.
        let position =
            pos.as_vec3() + 0.5 + Vec3::new(jitter(seed + 2.0), jitter(seed + 3.0), jitter(seed + 4.0)) * 0.25;
        self.items.push(DroppedItem {
            block,
            count,
            position,
            previous: position,
            velocity: Vec3::new(jitter(seed) * 0.1, 0.2, jitter(seed + 1.0) * 0.1),
            on_ground: false,
            age: 0.0,
        });
    }

    /// Throws an item forward from the player's eyes (Q).
    pub fn throw(&mut self, eye: Vec3, direction: Vec3, block: Block, count: u8) {
        self.items.push(DroppedItem {
            block,
            count,
            position: eye - Vec3::Y * 0.3,
            previous: eye - Vec3::Y * 0.3,
            velocity: direction * 0.3 + Vec3::Y * 0.1,
            on_ground: false,
            age: -1.0, // A longer pickup delay, so it isn't picked straight back up.
        });
    }

    pub fn update(
        &mut self,
        dt: f32,
        feet: Option<Vec3>,
        inventory: &mut Inventory,
        world: &impl Fn(IVec3) -> Option<Block>,
    ) {
        self.accumulator += dt;
        while self.accumulator >= TICK_SECONDS {
            self.accumulator -= TICK_SECONDS;
            for item in &mut self.items {
                item.tick(world);
            }
        }
        for item in &mut self.items {
            item.age += dt;
        }

        if let Some(feet) = feet {
            let center = feet + Vec3::Y * 0.9;
            let alpha = self.accumulator / TICK_SECONDS;
            for item in &mut self.items {
                if item.age >= PICKUP_DELAY && item.position.distance(center) < PICKUP_RANGE {
                    let before = item.count;
                    item.count = inventory.add(item.block, item.count);
                    if item.count < before {
                        // Start from where it was drawn, bob included.
                        let from = item.previous.lerp(item.position, alpha) + Vec3::Y * 0.1;
                        self.pickups.push(Pickup { block: item.block, from, elapsed: 0.0 });
                    }
                }
            }
        }
        for pickup in &mut self.pickups {
            pickup.elapsed += dt;
        }
        self.pickups.retain(|p| p.elapsed < PICKUP_SECONDS && feet.is_some());
        self.items.retain(|item| item.count > 0 && item.age < LIFETIME);
    }

    fn alpha(&self) -> f32 {
        self.accumulator / TICK_SECONDS
    }

    /// Every item lying in the world and where to draw it this frame,
    /// between its last two ticks.
    pub fn visible(&self) -> impl Iterator<Item = (Block, Vec3)> + '_ {
        let alpha = self.alpha();
        self.items.iter().map(move |item| (item.block, item.previous.lerp(item.position, alpha)))
    }

    /// Picked-up items flying into the player at `feet`, and where they are
    /// now. Vanilla eases them in quadratically towards the middle of the player.
    pub fn flying_to(&self, feet: Vec3) -> impl Iterator<Item = (Block, Vec3)> + '_ {
        let target = feet + Vec3::Y * PICKUP_TARGET_HEIGHT;
        self.pickups.iter().map(move |pickup| {
            let t = (pickup.elapsed / PICKUP_SECONDS).min(1.0);
            (pickup.block, pickup.from.lerp(target, t * t))
        })
    }
}

impl DroppedItem {
    fn tick(&mut self, world: &impl Fn(IVec3) -> Option<Block>) {
        self.previous = self.position;
        self.velocity.y -= GRAVITY;
        // Slide along each axis until something solid is in the way.
        let solid = |p: Vec3| world(p.floor().as_ivec3()).is_none_or(Block::is_solid);
        self.on_ground = false;
        for axis in [1, 0, 2] {
            let mut next = self.position;
            next[axis] += self.velocity[axis];
            let mut edge = next;
            edge[axis] += HALF_SIZE * self.velocity[axis].signum();
            if solid(edge) {
                if axis == 1 && self.velocity.y < 0.0 {
                    self.on_ground = true;
                    self.position.y = edge.y.floor() + 1.0 + HALF_SIZE;
                }
                self.velocity[axis] = 0.0;
            } else {
                self.position = next;
            }
        }
        // Pushed up out of blocks placed on top of it, like vanilla.
        if solid(self.position) {
            self.position.y = self.position.y.floor() + 1.0 + HALF_SIZE;
        }
        let friction = if self.on_ground { 0.6 * 0.98 } else { 0.98 };
        self.velocity *= Vec3::new(friction, 0.98, friction);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_stays_in_range() {
        for i in 0..10_000 {
            let j = jitter(i as f32 * 0.37 - 1800.0);
            assert!((-1.0..=1.0).contains(&j), "jitter({i}) = {j}");
        }
    }
}
