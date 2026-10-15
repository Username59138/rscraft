use glam::camera::rh::{proj, view};
use glam::{Mat4, Vec3};

use crate::input::Input;

const MOUSE_SENSITIVITY: f32 = 0.0025;
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

/// First-person camera at the player's eyes. Y is up, like in Minecraft.
#[derive(Clone)]
pub struct Camera {
    pub position: Vec3,
    /// Rotation around Y in radians; 0 looks towards -Z (north).
    pub yaw: f32,
    /// Rotation up/down in radians, clamped to avoid flipping.
    pub pitch: f32,
    fov_y: f32,
    /// View bobbing tilt (radians): sideways roll and a slight nod down.
    /// Only affects the view, not where the player is looking.
    pub bob_roll: f32,
    pub bob_pitch: f32,
    aspect: f32,
    near: f32,
    far: f32,
}

impl Camera {
    pub fn new(aspect: f32, position: Vec3, yaw: f32, pitch: f32) -> Self {
        Self {
            position,
            yaw,
            pitch,
            fov_y: 70f32.to_radians(),
            bob_roll: 0.0,
            bob_pitch: 0.0,
            aspect,
            near: 0.05,
            far: 1000.0,
        }
    }

    pub fn set_aspect(&mut self, aspect: f32) {
        self.aspect = aspect;
    }

    pub fn set_fov(&mut self, degrees: f32) {
        self.fov_y = degrees.to_radians();
    }

    /// Width divided by height of the view.
    pub fn aspect(&self) -> f32 {
        self.aspect
    }

    /// Vertical field of view in radians.
    pub fn fov_y(&self) -> f32 {
        self.fov_y
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }

    /// Turns the camera by the mouse movement since the last frame.
    pub fn look(&mut self, input: &mut Input, sensitivity: f32) {
        let (dx, dy) = input.take_mouse_delta();
        self.yaw += dx * MOUSE_SENSITIVITY * sensitivity;
        self.pitch = (self.pitch - dy * MOUSE_SENSITIVITY * sensitivity).clamp(-1.55, 1.55);
    }

    /// Horizontal right vector, for view bobbing.
    pub fn right(&self) -> Vec3 {
        Vec3::new(self.yaw.cos(), 0.0, self.yaw.sin())
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
        let pitch = self.pitch - self.bob_pitch;
        let forward = Vec3::new(self.yaw.sin() * pitch.cos(), pitch.sin(), -self.yaw.cos() * pitch.cos());
        let view = Mat4::from_rotation_z(self.bob_roll) * view::look_to_mat4(self.position, forward, Vec3::Y);
        // wgpu's clip space matches DirectX: depth 0..1, Y up.
        let proj = proj::directx::perspective(self.fov_y, self.aspect, self.near, self.far);
        proj * view
    }
}
