use glam::camera::rh::{proj, view};
use glam::{Mat4, Vec3};
use winit::keyboard::KeyCode;

use crate::input::Input;

const MOUSE_SENSITIVITY: f32 = 0.0025;
const FLY_SPEED: f32 = 10.0;
const SPRINT_MULTIPLIER: f32 = 4.0;
/// Distance of the third-person camera from the player's eyes.
const THIRD_PERSON_DISTANCE: f32 = 4.0;

/// First person or one of the two third-person views (F5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Perspective {
    FirstPerson,
    ThirdPersonBack,
    ThirdPersonFront,
}

impl Perspective {
    pub fn next(self) -> Self {
        match self {
            Perspective::FirstPerson => Perspective::ThirdPersonBack,
            Perspective::ThirdPersonBack => Perspective::ThirdPersonFront,
            Perspective::ThirdPersonFront => Perspective::FirstPerson,
        }
    }
}

/// Free-flying first-person camera at the player's eyes. Y is up, like in Minecraft.
#[derive(Clone)]
pub struct Camera {
    pub position: Vec3,
    /// Rotation around Y in radians; 0 looks towards -Z (north).
    pub yaw: f32,
    /// Rotation up/down in radians, clamped to avoid flipping.
    pub pitch: f32,
    fov_y: f32,
    aspect: f32,
    near: f32,
    far: f32,
}

impl Camera {
    pub fn new(aspect: f32, position: Vec3, yaw: f32, pitch: f32) -> Self {
        Self { position, yaw, pitch, fov_y: 70f32.to_radians(), aspect, near: 0.05, far: 1000.0 }
    }

    pub fn set_aspect(&mut self, aspect: f32) {
        self.aspect = aspect;
    }

    pub fn set_fov(&mut self, degrees: f32) {
        self.fov_y = degrees.to_radians();
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }

    pub fn update(&mut self, input: &mut Input, dt: f32, sensitivity: f32) {
        let (dx, dy) = input.take_mouse_delta();
        self.yaw += dx * MOUSE_SENSITIVITY * sensitivity;
        self.pitch = (self.pitch - dy * MOUSE_SENSITIVITY * sensitivity).clamp(-1.55, 1.55);

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

    /// The camera actually used for rendering in the given perspective.
    pub fn for_perspective(&self, perspective: Perspective) -> Camera {
        let mut camera = self.clone();
        match perspective {
            Perspective::FirstPerson => {}
            Perspective::ThirdPersonBack => camera.position -= self.forward() * THIRD_PERSON_DISTANCE,
            Perspective::ThirdPersonFront => {
                camera.position += self.forward() * THIRD_PERSON_DISTANCE;
                camera.yaw += std::f32::consts::PI;
                camera.pitch = -self.pitch;
            }
        }
        camera
    }

    pub fn view_proj(&self) -> Mat4 {
        let view = view::look_to_mat4(self.position, self.forward(), Vec3::Y);
        // wgpu's clip space matches DirectX: depth 0..1, Y up.
        let proj = proj::directx::perspective(self.fov_y, self.aspect, self.near, self.far);
        proj * view
    }
}
