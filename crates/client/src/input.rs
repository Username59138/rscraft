use std::collections::HashSet;

use winit::keyboard::KeyCode;

/// Keyboard and mouse state accumulated between frames.
#[derive(Default)]
pub struct Input {
    pressed: HashSet<KeyCode>,
    mouse_delta: (f32, f32),
    /// Mouse buttons held down while playing.
    pub attack: bool,
    pub use_item: bool,
}

impl Input {
    pub fn set_key(&mut self, key: KeyCode, pressed: bool) {
        if pressed {
            self.pressed.insert(key);
        } else {
            self.pressed.remove(&key);
        }
    }

    pub fn is_pressed(&self, key: KeyCode) -> bool {
        self.pressed.contains(&key)
    }

    pub fn add_mouse_delta(&mut self, dx: f32, dy: f32) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }

    /// Returns the mouse movement since the last call and resets it.
    pub fn take_mouse_delta(&mut self) -> (f32, f32) {
        std::mem::take(&mut self.mouse_delta)
    }

    pub fn clear(&mut self) {
        self.pressed.clear();
        self.attack = false;
        self.use_item = false;
        self.mouse_delta = (0.0, 0.0);
    }
}
