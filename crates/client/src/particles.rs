//! Particles, following vanilla's: terrain bits when blocks are hit and
//! broken (TerrainParticle), water splashes (WaterDropParticle), bubbles
//! (BubbleParticle) and the specks suspended in water (SuspendedParticle).

use glam::{IVec3, Vec3};
use rscraft_world::{Block, Face};

use crate::mesh::{MeshData, Vertex};
use crate::movement::TICK_SECONDS;
use crate::texture::{BlockLayers, SPLASH_FRAMES};

/// Particles alive at once; the oldest go first.
const MAX_PARTICLES: usize = 6000;
/// Random water blocks checked for suspended specks per tick, within 16 and
/// within 32 blocks, like vanilla's animateTick.
const AMBIENT_SAMPLES: usize = 667;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Terrain,
    Splash,
    Bubble,
    Suspended,
}

struct Particle {
    kind: Kind,
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
    /// Top-left and size of the part of the texture shown, in UV units.
    uv: [f32; 2],
    uv_size: f32,
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

    /// -1..1
    fn signed(&mut self) -> f32 {
        self.next() * 2.0 - 1.0
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
    /// Vanilla's base particle: the given motion plus random spread, rescaled
    /// to a random speed, with a small upward kick.
    fn spawn(&mut self, kind: Kind, layer: u32, position: Vec3, direction: Vec3) -> &mut Particle {
        let rng = &mut self.rng;
        let mut velocity = direction + Vec3::new(rng.signed(), rng.signed(), rng.signed()) * 0.4;
        let speed = (rng.next() + rng.next() + 1.0) * 0.15;
        velocity = velocity / velocity.length().max(1e-4) * speed * 0.4;
        velocity.y += 0.1;
        let lifetime = (4.0 / (rng.next() * 0.9 + 0.1)) as u32;
        let half_size = 0.1 * (rng.next() * 0.5 + 0.5);
        if self.particles.len() >= MAX_PARTICLES {
            self.particles.remove(0);
        }
        self.particles.push(Particle {
            kind,
            position,
            previous: position,
            velocity,
            on_ground: false,
            age: 0,
            lifetime,
            half_size,
            layer,
            uv: [0.0, 0.0],
            uv_size: 1.0,
        });
        self.particles.last_mut().unwrap()
    }

    /// A terrain bit: a random 4×4-pixel cut of the block's texture, half
    /// the usual particle size.
    fn spawn_terrain(
        &mut self,
        block: Block,
        layers: &BlockLayers,
        position: Vec3,
        direction: Vec3,
        power: f32,
    ) -> &mut Particle {
        let uv = [(self.rng.next() * 3.0).floor() / 4.0, (self.rng.next() * 3.0).floor() / 4.0];
        let particle = self.spawn(Kind::Terrain, particle_layer(block, layers), position, direction);
        let v = particle.velocity;
        // Vanilla's setPower: scales the motion but keeps the upward kick.
        particle.velocity = Vec3::new(v.x * power, (v.y - 0.1) * power + 0.1, v.z * power);
        particle.half_size *= 0.5;
        particle.uv = uv;
        particle.uv_size = 0.25;
        particle
    }

    /// A broken block bursts into a 4×4×4 grid of particles.
    pub fn block_broken(&mut self, pos: IVec3, block: Block, layers: &BlockLayers) {
        for x in 0..4 {
            for y in 0..4 {
                for z in 0..4 {
                    let offset = (Vec3::new(x as f32, y as f32, z as f32) + 0.5) / 4.0;
                    self.spawn_terrain(block, layers, pos.as_vec3() + offset, offset - 0.5, 1.0);
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
        // Vanilla also scales hit particles down to 60%.
        self.spawn_terrain(block, layers, p, Vec3::ZERO, 0.2).half_size *= 0.6;
    }

    /// A water splash droplet (WaterDropParticle).
    fn spawn_splash(&mut self, layers: &BlockLayers, position: Vec3, velocity: Vec3) {
        let frame = (self.rng.next() * SPLASH_FRAMES as f32) as u32 % SPLASH_FRAMES;
        let (vy, lifetime) = (self.rng.next() * 0.2 + 0.1, (8.0 / (self.rng.next() * 0.8 + 0.2)) as u32);
        let particle = self.spawn(Kind::Splash, layers.splash + frame, position, Vec3::ZERO);
        particle.velocity = Vec3::new(
            particle.velocity.x * 0.3 + velocity.x,
            vy + velocity.y.max(0.0),
            particle.velocity.z * 0.3 + velocity.z,
        );
        particle.lifetime = lifetime;
    }

    /// A rising bubble (BubbleParticle).
    fn spawn_bubble(&mut self, layers: &BlockLayers, position: Vec3, velocity: Vec3) {
        let rng = &mut self.rng;
        let spread = Vec3::new(rng.signed(), rng.signed(), rng.signed()) * 0.02;
        let (size, lifetime) = (rng.next() * 0.6 + 0.2, (8.0 / (rng.next() * 0.8 + 0.2)) as u32);
        let particle = self.spawn(Kind::Bubble, layers.bubble, position, Vec3::ZERO);
        particle.velocity = velocity * 0.2 + spread;
        particle.half_size *= size;
        particle.lifetime = lifetime;
    }

    /// Something fell into water: bubbles and splashes over the surface,
    /// like vanilla's doWaterSplashEffect.
    pub fn water_splash(&mut self, layers: &BlockLayers, center: Vec3, surface: f32, half_width: f32, velocity: Vec3) {
        for _ in 0..(1.0 + half_width * 2.0 * 20.0) as usize {
            let (dx, dz) = (self.rng.signed() * half_width, self.rng.signed() * half_width);
            let at = Vec3::new(center.x + dx, surface, center.z + dz);
            let down = velocity.y - self.rng.next() * 0.2;
            self.spawn_bubble(layers, at, Vec3::new(velocity.x, down, velocity.z));
        }
        for _ in 0..(1.0 + half_width * 2.0 * 20.0) as usize {
            let (dx, dz) = (self.rng.signed() * half_width, self.rng.signed() * half_width);
            self.spawn_splash(layers, Vec3::new(center.x + dx, surface, center.z + dz), velocity);
        }
    }

    pub fn update(&mut self, dt: f32, camera: Vec3, layers: &BlockLayers, world: &impl Fn(IVec3) -> Option<Block>) {
        self.accumulator += dt;
        while self.accumulator >= TICK_SECONDS {
            self.accumulator -= TICK_SECONDS;
            self.spawn_ambient(camera, layers, world);
            for particle in &mut self.particles {
                particle.tick(world);
            }
            self.particles.retain(|p| p.age < p.lifetime);
        }
    }

    /// Still water around the camera now and then releases a suspended speck,
    /// like vanilla's WaterFluid.animateTick (1 in 10 for every block checked).
    fn spawn_ambient(&mut self, camera: Vec3, layers: &BlockLayers, world: &impl Fn(IVec3) -> Option<Block>) {
        let origin = camera.floor().as_ivec3();
        for range in [16.0, 32.0] {
            for _ in 0..AMBIENT_SAMPLES {
                let offset = Vec3::new(self.rng.signed(), self.rng.signed(), self.rng.signed()) * range;
                let pos = origin + offset.as_ivec3();
                if self.rng.next() < 0.1 && world(pos) == Some(Block::Water) {
                    let at = pos.as_vec3() + Vec3::new(self.rng.next(), self.rng.next(), self.rng.next());
                    let (size, lifetime) = (self.rng.next() * 0.6 + 0.2, (16.0 / (self.rng.next() * 0.8 + 0.2)) as u32);
                    let particle = self.spawn(Kind::Suspended, layers.underwater, at, Vec3::ZERO);
                    particle.velocity = Vec3::ZERO;
                    particle.half_size *= size * 0.5;
                    particle.lifetime = lifetime;
                }
            }
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
            let ([u, v], s) = (p.uv, p.uv_size);
            let base = mesh.vertices.len() as u32;
            // Counter-clockwise as seen by the camera.
            let corners = [
                (center - right - up, [u, v + s]),
                (center + right - up, [u + s, v + s]),
                (center + right + up, [u + s, v]),
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
        let in_water = world(self.position.floor().as_ivec3()) == Some(Block::Water);
        match self.kind {
            Kind::Terrain => self.velocity.y -= 0.04,
            Kind::Splash => self.velocity.y -= 0.06,
            Kind::Bubble => {
                // Bubbles rise, and pop as soon as they leave the water.
                self.velocity.y += 0.002;
                if !in_water {
                    self.age = self.lifetime;
                    return;
                }
            }
            Kind::Suspended => {
                if !in_water {
                    self.age = self.lifetime;
                    return;
                }
            }
        }
        self.move_by(world);
        let drag = match self.kind {
            Kind::Bubble => 0.85,
            _ => 0.98,
        };
        self.velocity *= drag;
        if self.on_ground {
            self.velocity.x *= 0.7;
            self.velocity.z *= 0.7;
            // Half of the splash droplets die as soon as they land.
            if self.kind == Kind::Splash && self.age.is_multiple_of(2) {
                self.age = self.lifetime;
            }
        }
    }

    fn move_by(&mut self, world: &impl Fn(IVec3) -> Option<Block>) {
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
    }
}
