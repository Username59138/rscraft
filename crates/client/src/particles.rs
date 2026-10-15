//! Block particles, following vanilla's TerrainParticle: a block breaks into
//! a 4×4×4 grid of small textured squares, and hitting a block chips bits
//! off the face being hit.

use glam::{IVec3, Vec3};
use rscraft_world::{Block, Face};

use crate::mesh::{MeshData, Vertex};
use crate::movement::TICK_SECONDS;
use crate::texture::BlockLayers;

/// Particles alive at once; the oldest go first.
const MAX_PARTICLES: usize = 4000;
const GRAVITY: f32 = 0.04;

struct Particle {
    position: Vec3,
    previous: Vec3,
    velocity: Vec3,
    on_ground: bool,
    /// Ticks lived and ticks to live.
    age: u32,
    lifetime: u32,
    /// Half the side of the square, in blocks.
    half_size: f32,
    layer: u32,
    /// Top-left of the quarter-texture square shown, in UV units.
    uv: [f32; 2],
}

/// Small xorshift generator; particles only need cheap randomness.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1 << 24) as f32
    }
}

pub struct Particles {
    particles: Vec<Particle>,
    rng: Rng,
    accumulator: f32,
}

impl Default for Particles {
    fn default() -> Self {
        Self { particles: Vec::new(), rng: Rng(0x9e37_79b9), accumulator: 0.0 }
    }
}

/// The texture a block's particles use, like vanilla's model "particle".
fn particle_layer(block: Block, layers: &BlockLayers) -> u32 {
    match block {
        Block::Grass => layers.get(Block::Dirt, Face::North),
        _ => layers.get(block, Face::North),
    }
}

impl Particles {
    /// A particle with vanilla's base motion: the given direction plus
    /// random spread, rescaled to a random speed.
    fn spawn(&mut self, block: Block, layers: &BlockLayers, position: Vec3, direction: Vec3, power: f32) {
        let rng = &mut self.rng;
        let mut velocity = direction + Vec3::new(rng.next() * 2.0 - 1.0, rng.next() * 2.0 - 1.0, rng.next() * 2.0 - 1.0) * 0.4;
        let speed = (rng.next() + rng.next() + 1.0) * 0.15;
        velocity = velocity / velocity.length().max(1e-4) * speed * 0.4;
        velocity.y += 0.1;
        // Vanilla's setPower: scales the motion but keeps the upward kick.
        velocity = Vec3::new(velocity.x * power, (velocity.y - 0.1) * power + 0.1, velocity.z * power);
        let lifetime = (4.0 / (rng.next() * 0.9 + 0.1)) as u32;
        let half_size = 0.1 * (rng.next() * 0.5 + 0.5) * 0.5;
        let uv = [(rng.next() * 3.0).floor() / 4.0, (rng.next() * 3.0).floor() / 4.0];
        if self.particles.len() >= MAX_PARTICLES {
            self.particles.remove(0);
        }
        self.particles.push(Particle {
            position,
            previous: position,
            velocity,
            on_ground: false,
            age: 0,
            lifetime,
            half_size,
            layer: particle_layer(block, layers),
            uv,
        });
    }

    /// A broken block bursts into a 4×4×4 grid of particles.
    pub fn block_broken(&mut self, pos: IVec3, block: Block, layers: &BlockLayers) {
        for x in 0..4 {
            for y in 0..4 {
                for z in 0..4 {
                    let offset = (Vec3::new(x as f32, y as f32, z as f32) + 0.5) / 4.0;
                    self.spawn(block, layers, pos.as_vec3() + offset, offset - 0.5, 1.0);
                }
            }
        }
    }

    /// Hitting a block chips one small, slow particle off the face being hit.
    pub fn block_hit(&mut self, pos: IVec3, face: IVec3, block: Block, layers: &BlockLayers) {
        let rng = &mut self.rng;
        let mut p = pos.as_vec3() + Vec3::new(rng.next(), rng.next(), rng.next()) * 0.8 + 0.1;
        for axis in 0..3 {
            match face[axis] {
                1 => p[axis] = pos[axis] as f32 + 1.1,
                -1 => p[axis] = pos[axis] as f32 - 0.1,
                _ => {}
            }
        }
        self.spawn(block, layers, p, Vec3::ZERO, 0.2);
        // Vanilla also scales hit particles down to 60%.
        if let Some(particle) = self.particles.last_mut() {
            particle.half_size *= 0.6;
        }
    }

    pub fn update(&mut self, dt: f32, world: &impl Fn(IVec3) -> Option<Block>) {
        self.accumulator += dt;
        while self.accumulator >= TICK_SECONDS {
            self.accumulator -= TICK_SECONDS;
            for particle in &mut self.particles {
                particle.tick(world);
            }
            self.particles.retain(|p| p.age < p.lifetime);
        }
    }

    /// Camera-facing squares, drawn with the solid block pipeline.
    pub fn build_mesh(&self, camera_right: Vec3, camera_up: Vec3, light: impl Fn(Vec3) -> f32) -> MeshData {
        let alpha = self.accumulator / TICK_SECONDS;
        let mut mesh = MeshData::default();
        for p in &self.particles {
            let center = p.previous.lerp(p.position, alpha);
            let (right, up) = (camera_right * p.half_size, camera_up * p.half_size);
            let sky = light(center);
            let [u, v] = p.uv;
            let base = mesh.vertices.len() as u32;
            // Counter-clockwise as seen by the camera; a 4×4-pixel cut of the texture.
            let corners = [
                (center - right - up, [u, v + 0.25]),
                (center + right - up, [u + 0.25, v + 0.25]),
                (center + right + up, [u + 0.25, v]),
                (center - right + up, [u, v]),
            ];
            for (position, uv) in corners {
                mesh.vertices.push(Vertex {
                    position: position.to_array(),
                    uv,
                    // Facing up gets full brightness from the block shader.
                    normal: [0.0, 1.0, 0.0],
                    layer: p.layer,
                    ao: 1.0,
                    sky,
                });
            }
            mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        mesh
    }
}

impl Particle {
    fn tick(&mut self, world: &impl Fn(IVec3) -> Option<Block>) {
        self.previous = self.position;
        self.age += 1;
        self.velocity.y -= GRAVITY;
        let solid = |p: Vec3| world(p.floor().as_ivec3()).is_some_and(Block::is_solid);
        self.on_ground = false;
        for axis in [1, 0, 2] {
            let mut next = self.position;
            next[axis] += self.velocity[axis];
            let mut edge = next;
            edge[axis] += self.half_size * self.velocity[axis].signum();
            if solid(edge) {
                if axis == 1 && self.velocity.y < 0.0 {
                    self.on_ground = true;
                }
                self.velocity[axis] = 0.0;
            } else {
                self.position = next;
            }
        }
        self.velocity *= 0.98;
        if self.on_ground {
            self.velocity.x *= 0.7;
            self.velocity.z *= 0.7;
        }
    }
}
