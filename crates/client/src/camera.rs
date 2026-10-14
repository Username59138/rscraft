use glam::camera::rh::{proj, view};
use glam::{Mat4, Vec3};
use winit::keyboard::KeyCode;

use crate::input::Input;

const MOUSE_SENSITIVITY: f32 = 0.0025;
const FLY_SPEED: f32 = 10.0;
const SPRINT_MULTIPLIER: f32 = 4.0;

/// Free-flying first-person camera. Y is up, like in Minecraft.
pub struct Camera {
    pub position: Vec3,
    /// Rotation around Y in radians; 0 looks towards -Z.
    yaw: f32,
    /// Rotation up/down in radians, clamped to avoid flipping.
    pitch: f32,
    fov_y: f32,
    aspect: f32,
    near: f32,
    far: f32,
}

impl Camera {
    pub fn new(aspect: f32, position: Vec3) -> Self {
        Self {
            position,
            yaw: 0.0,
            pitch: -0.3,
            fov_y: 70f32.to_radians(),
            aspect,
            near: 0.05,
            far: 1000.0,
        }
    }

    pub fn set_aspect(&mut self, aspect: f32) {
        self.aspect = aspect;
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        )
    }

    pub fn update(&mut self, input: &mut Input, dt: f32) {
        let (dx, dy) = input.take_mouse_delta();
        self.yaw += dx * MOUSE_SENSITIVITY;
        self.pitch = (self.pitch - dy * MOUSE_SENSITIVITY).clamp(-1.55, 1.55);

        // Horizontal movement ignores pitch, like Minecraft's creative flight.
        let flat_forward = Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(self.yaw.cos(), 0.0, self.yaw.sin());

        let mut dir = Vec3::ZERO;
        if input.is_pressed(KeyCode::KeyW) {
            dir += flat_forward;
        }
        if input.is_pressed(KeyCode::KeyS) {
            dir -= flat_forward;
        }
        if input.is_pressed(KeyCode::KeyD) {
            dir += right;
        }
        if input.is_pressed(KeyCode::KeyA) {
            dir -= right;
        }
        if input.is_pressed(KeyCode::Space) {
            dir += Vec3::Y;
        }
        if input.is_pressed(KeyCode::ShiftLeft) {
            dir -= Vec3::Y;
        }

        let mut speed = FLY_SPEED;
        if input.is_pressed(KeyCode::ControlLeft) {
            speed *= SPRINT_MULTIPLIER;
        }
        self.position += dir.normalize_or_zero() * speed * dt;
    }

    pub fn view_proj(&self) -> Mat4 {
        let view = view::look_to_mat4(self.position, self.forward(), Vec3::Y);
        // wgpu's clip space matches DirectX: depth 0..1, Y up.
        let proj = proj::directx::perspective(self.fov_y, self.aspect, self.near, self.far);
        proj * view
    }
}
