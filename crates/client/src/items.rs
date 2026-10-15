//! Items lying in the world: they fall, bob and spin, and fly into the
//! player's inventory when they come close.

use glam::{IVec3, Mat4, Vec3};
use rscraft_world::{Block, Face};

use crate::inventory::Inventory;
use crate::mesh::MeshData;
use crate::movement::TICK_SECONDS;
use crate::texture::BlockLayers;

/// Half the size of the dropped block cube (vanilla draws them at 1/4 scale).
const HALF_SIZE: f32 = 0.125;
const GRAVITY: f32 = 0.04;
const PICKUP_RANGE: f32 = 1.5;
/// Items can't be picked up for 10 ticks after they appear, like vanilla.
const PICKUP_DELAY: f32 = 0.5;
/// Items disappear after 5 minutes.
const LIFETIME: f32 = 300.0;

struct DroppedItem {
    block: Block,
    count: u8,
    position: Vec3,
    velocity: Vec3,
    on_ground: bool,
    age: f32,
}

#[derive(Default)]
pub struct Items {
    items: Vec<DroppedItem>,
    accumulator: f32,
}

/// Tiny pseudo-random number in -1..1 for scattering drops.
fn jitter(seed: f32) -> f32 {
    ((seed * 12.9898).sin() * 43758.547).fract() * 2.0 - 1.0
}

impl Items {
    /// Drops an item at the centre of a block, with a little random hop.
    pub fn spawn_at_block(&mut self, pos: IVec3, block: Block, count: u8) {
        let seed = (pos.x * 73 + pos.y * 19 + pos.z * 31 + self.items.len() as i32) as f32;
        self.items.push(DroppedItem {
            block,
            count,
            position: pos.as_vec3() + 0.5,
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
            velocity: direction * 0.3 + Vec3::Y * 0.1,
            on_ground: false,
            age: -1.0, // A longer pickup delay, so it isn't picked straight back up.
        });
    }

    pub fn update(&mut self, dt: f32, feet: Option<Vec3>, inventory: &mut Inventory, world: &impl Fn(IVec3) -> Option<Block>) {
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
            for item in &mut self.items {
                if item.age >= PICKUP_DELAY && item.position.distance(center) < PICKUP_RANGE {
                    item.count = inventory.add(item.block, item.count);
                }
            }
        }
        self.items.retain(|item| item.count > 0 && item.age < LIFETIME);
    }

    /// Spinning, bobbing cubes, ready to draw with the solid block pipeline.
    pub fn build_mesh(&self, layers: &BlockLayers, seconds: f32) -> MeshData {
        let mut mesh = MeshData::default();
        for (i, item) in self.items.iter().enumerate() {
            let start = mesh.vertices.len();
            for face in Face::ALL {
                mesh.push_face(IVec3::ZERO, face, layers.get(item.block, face), [3; 4], [1.0; 4], 1.0);
            }
            let phase = seconds + i as f32 * 1.7;
            let bob = (phase * 2.0).sin() * 0.05 + 0.1;
            let transform = Mat4::from_translation(item.position + Vec3::Y * bob)
                * Mat4::from_rotation_y(phase)
                * Mat4::from_scale(Vec3::splat(HALF_SIZE * 2.0))
                * Mat4::from_translation(Vec3::splat(-0.5));
            for v in &mut mesh.vertices[start..] {
                v.position = transform.transform_point3(Vec3::from(v.position)).to_array();
                v.normal = transform.transform_vector3(Vec3::from(v.normal)).normalize().to_array();
            }
        }
        mesh
    }
}

impl DroppedItem {
    fn tick(&mut self, world: &impl Fn(IVec3) -> Option<Block>) {
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
