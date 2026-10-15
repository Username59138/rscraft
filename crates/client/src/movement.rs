//! Player movement with Minecraft's physics: fixed 20 Hz ticks, the vanilla
//! constants for walking, sprinting, jumping, swimming and flying, and
//! axis-by-axis collision against block boxes.

use glam::{IVec3, Vec2, Vec3};
use rscraft_world::Block;

pub const TICK_SECONDS: f32 = 0.05;
const HALF_WIDTH: f32 = 0.3;
const HEIGHT: f32 = 1.8;
pub const EYE_HEIGHT: f32 = 1.62;
const SNEAK_EYE_HEIGHT: f32 = 1.27;

const GRAVITY: f32 = 0.08;
const AIR_DRAG: f32 = 0.98;
const JUMP_VELOCITY: f32 = 0.42;
const SPRINT_JUMP_BOOST: f32 = 0.2;
const WALK_SPEED: f32 = 0.1;
const SPRINT_MULTIPLIER: f32 = 1.3;
const BLOCK_FRICTION: f32 = 0.6;
const FLY_SPEED: f32 = 0.05;
/// Two jump presses within this many ticks toggle flying.
const DOUBLE_TAP_TICKS: u32 = 7;

/// What the player is pressing this tick.
#[derive(Clone, Copy, Default)]
pub struct Controls {
    pub forward: f32,
    pub strafe: f32,
    pub jump: bool,
    pub sneak: bool,
    pub sprint: bool,
}

#[derive(Clone, Copy)]
struct Aabb {
    min: Vec3,
    max: Vec3,
}

impl Aabb {
    fn player(feet: Vec3) -> Self {
        Self {
            min: feet - Vec3::new(HALF_WIDTH, 0.0, HALF_WIDTH),
            max: feet + Vec3::new(HALF_WIDTH, HEIGHT, HALF_WIDTH),
        }
    }

    fn offset(self, d: Vec3) -> Self {
        Self { min: self.min + d, max: self.max + d }
    }

    /// How far `self` can move along `axis` by `d` before hitting `other`.
    fn clip(&self, other: &Aabb, axis: usize, mut d: f32) -> f32 {
        let overlaps = (0..3).filter(|&a| a != axis).all(|a| self.max[a] > other.min[a] && self.min[a] < other.max[a]);
        if !overlaps {
            return d;
        }
        if d > 0.0 && self.max[axis] <= other.min[axis] {
            d = d.min(other.min[axis] - self.max[axis]);
        } else if d < 0.0 && self.min[axis] >= other.max[axis] {
            d = d.max(other.max[axis] - self.min[axis]);
        }
        d
    }
}

pub struct PlayerController {
    /// Feet position at the last two ticks, for interpolation.
    pub position: Vec3,
    previous: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    /// Bumped into a wall during the last move.
    horizontal_collision: bool,
    pub flying: bool,
    pub sprinting: bool,
    pub sneaking: bool,
    in_water: bool,
    accumulator: f32,
    ticks: u32,
    jump_held: bool,
    last_jump_press: Option<u32>,
    eye_height: f32,
    previous_eye_height: f32,
    /// Distance walked on the ground, drives view bobbing and limb swing.
    pub walk_distance: f32,
    previous_walk_distance: f32,
    bob: f32,
    previous_bob: f32,
}

impl PlayerController {
    pub fn new(feet: Vec3, flying: bool) -> Self {
        Self {
            position: feet,
            previous: feet,
            velocity: Vec3::ZERO,
            on_ground: false,
            horizontal_collision: false,
            flying,
            sprinting: false,
            sneaking: false,
            in_water: false,
            accumulator: 0.0,
            ticks: 0,
            jump_held: false,
            last_jump_press: None,
            eye_height: EYE_HEIGHT,
            previous_eye_height: EYE_HEIGHT,
            walk_distance: 0.0,
            previous_walk_distance: 0.0,
            bob: 0.0,
            previous_bob: 0.0,
        }
    }

    fn alpha(&self) -> f32 {
        self.accumulator / TICK_SECONDS
    }

    /// Interpolated eye position for rendering.
    pub fn eye(&self) -> Vec3 {
        let a = self.alpha();
        let eye = self.previous_eye_height + (self.eye_height - self.previous_eye_height) * a;
        self.previous.lerp(self.position, a) + Vec3::Y * eye
    }

    /// Interpolated feet position for rendering.
    pub fn feet(&self) -> Vec3 {
        self.previous.lerp(self.position, self.alpha())
    }

    /// View bobbing phase and strength, interpolated.
    pub fn bobbing(&self) -> (f32, f32) {
        let a = self.alpha();
        let walk = self.previous_walk_distance + (self.walk_distance - self.previous_walk_distance) * a;
        (walk, self.previous_bob + (self.bob - self.previous_bob) * a)
    }

    /// Whether the player's hitbox overlaps the block at `pos`.
    pub fn intersects_block(&self, pos: IVec3) -> bool {
        let player = Aabb::player(self.position);
        let min = pos.as_vec3();
        player.min.cmplt(min + Vec3::ONE).all() && player.max.cmpgt(min).all()
    }

    /// Moves up out of any blocks the player is stuck in, e.g. a tree at spawn.
    pub fn unstuck(&mut self, world: &impl Fn(IVec3) -> Option<Block>) {
        for _ in 0..64 {
            if !Self::collides(&Aabb::player(self.position), world) {
                break;
            }
            self.position.y = self.position.y.floor() + 1.0;
        }
        self.previous = self.position;
    }

    /// Runs as many ticks as `dt` covers. `world` returns None for unloaded
    /// blocks, which are treated as solid so nobody falls out of the world.
    pub fn update(&mut self, dt: f32, controls: Controls, yaw: f32, world: &impl Fn(IVec3) -> Option<Block>) {
        self.accumulator += dt;
        let mut steps = 0;
        while self.accumulator >= TICK_SECONDS && steps < 5 {
            self.accumulator -= TICK_SECONDS;
            self.tick(controls, yaw, world);
            steps += 1;
        }
        self.accumulator = self.accumulator.min(TICK_SECONDS);
    }

    fn tick(&mut self, controls: Controls, yaw: f32, world: &impl Fn(IVec3) -> Option<Block>) {
        self.ticks += 1;
        self.previous = self.position;
        self.previous_eye_height = self.eye_height;
        self.previous_walk_distance = self.walk_distance;
        self.previous_bob = self.bob;

        // Double-tapping jump toggles flying.
        let jump_pressed = controls.jump && !self.jump_held;
        self.jump_held = controls.jump;
        if jump_pressed {
            if self.last_jump_press.is_some_and(|t| self.ticks - t <= DOUBLE_TAP_TICKS) {
                self.flying = !self.flying;
                self.last_jump_press = None;
            } else {
                self.last_jump_press = Some(self.ticks);
            }
        }

        self.sneaking = controls.sneak && !self.flying;
        self.sprinting = controls.sprint && controls.forward > 0.0 && !self.sneaking;
        let target_eye = if self.sneaking { SNEAK_EYE_HEIGHT } else { EYE_HEIGHT };
        self.eye_height += (target_eye - self.eye_height) * 0.5;
        self.in_water = world((self.position + Vec3::Y * 0.4).floor().as_ivec3()) == Some(Block::Water);

        let mut input = Vec2::new(controls.strafe, controls.forward);
        if input.length_squared() > 1.0 {
            input = input.normalize();
        }
        input *= 0.98;
        if self.sneaking {
            input *= 0.3;
        }
        let forward = Vec3::new(yaw.sin(), 0.0, -yaw.cos());
        let right = Vec3::new(yaw.cos(), 0.0, yaw.sin());
        let accelerate = |velocity: &mut Vec3, amount: f32| {
            *velocity += (right * input.x + forward * input.y) * amount;
        };

        if self.flying {
            let speed = if controls.sprint { FLY_SPEED * 2.0 } else { FLY_SPEED };
            accelerate(&mut self.velocity, speed);
            if controls.jump {
                self.velocity.y += FLY_SPEED * 3.0;
            }
            if controls.sneak {
                self.velocity.y -= FLY_SPEED * 3.0;
            }
            self.move_by(world);
            self.velocity *= Vec3::new(0.91, 0.6, 0.91);
            if self.on_ground {
                self.flying = false;
            }
        } else if self.in_water {
            accelerate(&mut self.velocity, 0.02);
            if controls.jump {
                self.velocity.y += 0.04;
            }
            let start_y = self.position.y;
            self.move_by(world);
            self.velocity *= 0.8;
            self.velocity.y -= 0.02;
            // Vanilla's way out of water: swimming against a ledge with room
            // above it gives a boost up and over.
            let above = Vec3::new(self.velocity.x, self.velocity.y + 0.6 - self.position.y + start_y, self.velocity.z);
            if self.horizontal_collision && !Self::collides(&Aabb::player(self.position).offset(above), world) {
                self.velocity.y = 0.3;
            }
        } else {
            let friction = if self.on_ground { BLOCK_FRICTION * 0.91 } else { 0.91 };
            let speed = if self.sprinting { WALK_SPEED * SPRINT_MULTIPLIER } else { WALK_SPEED };
            let acceleration = if self.on_ground {
                speed * 0.216 / (friction * friction * friction)
            } else if self.sprinting {
                0.026
            } else {
                0.02
            };
            if controls.jump && self.on_ground {
                self.velocity.y = JUMP_VELOCITY;
                if self.sprinting {
                    self.velocity += forward * SPRINT_JUMP_BOOST;
                }
            }
            accelerate(&mut self.velocity, acceleration);
            self.move_by(world);
            self.velocity.y = (self.velocity.y - GRAVITY) * AIR_DRAG;
            self.velocity.x *= friction;
            self.velocity.z *= friction;
        }

        // Walking animation, only while on the ground.
        let moved = (self.position - self.previous) * Vec3::new(1.0, 0.0, 1.0);
        let target_bob = if self.on_ground && !self.flying { moved.length().min(0.1) } else { 0.0 };
        self.bob += (target_bob - self.bob) * 0.4;
        self.walk_distance += moved.length() * 0.6;
    }

    fn collides(aabb: &Aabb, world: &impl Fn(IVec3) -> Option<Block>) -> bool {
        let (min, max) = (aabb.min.floor().as_ivec3(), (aabb.max - 1e-4).floor().as_ivec3());
        for y in min.y..=max.y {
            for z in min.z..=max.z {
                for x in min.x..=max.x {
                    if world(IVec3::new(x, y, z)).is_none_or(Block::is_solid) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Moves by the current velocity, sliding along whatever is in the way.
    fn move_by(&mut self, world: &impl Fn(IVec3) -> Option<Block>) {
        let mut d = self.velocity;
        let start = Aabb::player(self.position);

        // Sneaking keeps you from walking off edges.
        if self.sneaking && self.on_ground {
            let below = |dx: f32, dz: f32| !Self::collides(&start.offset(Vec3::new(dx, -0.6, dz)), world);
            let shrink = |v: f32| if v.abs() < 0.05 { 0.0 } else { v - 0.05 * v.signum() };
            while d.x != 0.0 && below(d.x, 0.0) {
                d.x = shrink(d.x);
            }
            while d.z != 0.0 && below(0.0, d.z) {
                d.z = shrink(d.z);
            }
            while d.x != 0.0 && d.z != 0.0 && below(d.x, d.z) {
                d.x = shrink(d.x);
                d.z = shrink(d.z);
            }
        }

        // Every solid block the move could touch.
        let swept_min = start.min + d.min(Vec3::ZERO);
        let swept_max = start.max + d.max(Vec3::ZERO);
        let (lo, hi) = (swept_min.floor().as_ivec3(), swept_max.floor().as_ivec3());
        let mut boxes = Vec::new();
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                for x in lo.x..=hi.x {
                    let pos = IVec3::new(x, y, z);
                    if world(pos).is_none_or(Block::is_solid) {
                        let min = pos.as_vec3();
                        boxes.push(Aabb { min, max: min + Vec3::ONE });
                    }
                }
            }
        }

        // Y first, then the larger horizontal axis, like vanilla.
        let order = if d.x.abs() < d.z.abs() { [1, 2, 0] } else { [1, 0, 2] };
        let mut aabb = start;
        let mut moved = Vec3::ZERO;
        for axis in order {
            let mut amount = d[axis];
            for b in &boxes {
                amount = aabb.clip(b, axis, amount);
            }
            let mut step = Vec3::ZERO;
            step[axis] = amount;
            aabb = aabb.offset(step);
            moved[axis] = amount;
        }

        self.on_ground = d.y < 0.0 && moved.y != d.y;
        self.horizontal_collision = moved.x != d.x || moved.z != d.z;
        // Stop on any axis that hit something.
        for axis in 0..3 {
            if moved[axis] != d[axis] {
                self.velocity[axis] = 0.0;
            }
        }
        self.position += moved;
    }
}
