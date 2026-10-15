//! The first-person hand's animation, ticked like vanilla: the attack swing
//! (LivingEntity.SwingState), lowering and raising the hand when the held
//! item changes, after attacking and after placing (FirstPersonHandsAndItems),
//! and the arm lagging behind the view when turning (LocalPlayer's xBob/yBob).

use glam::Vec2;
use rscraft_world::Block;

use crate::camera::Camera;

/// Ticks of a swing with the bare hand or a block: vanilla's default swing.
const SWING_TICKS: u32 = 6;
/// Ticks for the hand to come back up after attacking or switching items:
/// vanilla's attack strength delay, 20 / attack speed 4.
const ATTACK_STRENGTH_TICKS: f32 = 5.0;
/// The most the hand moves towards its target height in one tick.
const HEIGHT_STEP: f32 = 0.4;

/// The camera's rotation as vanilla counts it, in degrees: pitch down, yaw.
pub fn view_angles(camera: &Camera) -> Vec2 {
    Vec2::new(-camera.pitch.to_degrees(), camera.yaw.to_degrees())
}

pub struct HandAnimation {
    /// Ticks into the current swing; None when not swinging.
    swing_ticks: Option<u32>,
    swing: f32,
    old_swing: f32,
    /// Ticks since the last attack or item switch.
    swap_ticks: u32,
    last_selected: Option<Block>,
    /// 1 when the hand is in place, 0 when it is hidden below the screen.
    height: f32,
    old_height: f32,
    /// What the hand shows; lags behind the selected item while switching.
    pub shown: Option<Block>,
    /// The view rotation the arm follows, in degrees (pitch down, yaw).
    view: Vec2,
    old_view: Vec2,
}

impl HandAnimation {
    /// The hand starts lowered and rises, like when joining a world.
    pub fn new(view: Vec2) -> Self {
        Self {
            swing_ticks: None,
            swing: 0.0,
            old_swing: 0.0,
            swap_ticks: 0,
            last_selected: None,
            height: 0.0,
            old_height: 0.0,
            shown: None,
            view,
            old_view: view,
        }
    }

    /// Skips straight to holding `selected` with the hand in place and
    /// caught up with the view, for test scenes.
    pub fn settle(&mut self, selected: Option<Block>, view: Vec2) {
        (self.view, self.old_view) = (view, view);
        (self.shown, self.last_selected) = (selected, selected);
        (self.height, self.old_height) = (1.0, 1.0);
        self.swap_ticks = ATTACK_STRENGTH_TICKS as u32;
    }

    /// Starts a swing, unless the current one is still in its first half;
    /// so holding attack keeps the arm swinging.
    pub fn swing(&mut self) {
        match self.swing_ticks {
            Some(ticks) if ticks > 0 && ticks <= SWING_TICKS / 2 => {}
            Some(_) => self.swing_ticks = Some(0),
            None => {
                (self.swing, self.old_swing) = (0.0, 0.0);
                self.swing_ticks = Some(0);
            }
        }
    }

    /// After an attack at nothing or giving up on a block: the hand dips
    /// and comes back up.
    pub fn reset_attack_strength(&mut self) {
        self.swap_ticks = 0;
    }

    /// A block was placed from the held stack: the hand drops and rises.
    pub fn item_used(&mut self) {
        self.height = 0.0;
    }

    /// One game tick. `view` is the camera's pitch (down) and yaw in degrees.
    pub fn tick(&mut self, selected: Option<Block>, view: Vec2) {
        self.old_swing = self.swing;
        if let Some(ticks) = self.swing_ticks {
            self.swing = (ticks as f32 / SWING_TICKS as f32).min(1.0);
            self.swing_ticks = Some(ticks + 1);
            if ticks > SWING_TICKS {
                self.swing_ticks = None;
                self.swing = 0.0;
            }
        } else {
            self.swing = 0.0;
        }

        // The hand moves towards the attack strength cubed, or all the way
        // down while the held item differs from the shown one.
        self.old_height = self.height;
        let strength = ((self.swap_ticks as f32 + 1.0) / ATTACK_STRENGTH_TICKS).clamp(0.0, 1.0);
        let target = if self.shown == selected { strength.powi(3) } else { 0.0 };
        self.height += (target - self.height).clamp(-HEIGHT_STEP, HEIGHT_STEP);
        if self.height < 0.1 {
            self.shown = selected;
        }

        self.swap_ticks += 1;
        if selected != self.last_selected {
            self.swap_ticks = 0;
            self.last_selected = selected;
        }

        self.old_view = self.view;
        self.view += (view - self.view) * 0.5;
    }

    /// Swing progress 0..1 between ticks; a restarted swing keeps going
    /// forward through 1 instead of jumping back.
    pub fn swing_progress(&self, partial: f32) -> f32 {
        let end = if self.swing >= self.old_swing { self.swing } else { self.swing + 1.0 };
        self.old_swing + (end - self.old_swing) * partial
    }

    /// How far the hand is lowered, 0 (in place) to 1 (hidden).
    pub fn lowered(&self, partial: f32) -> f32 {
        1.0 - (self.old_height + (self.height - self.old_height) * partial)
    }

    /// The arm's lag behind the current view, in degrees: vanilla turns it
    /// by a tenth of how far the view is ahead of the smoothed rotation.
    pub fn sway(&self, view: Vec2, partial: f32) -> Vec2 {
        (view - self.old_view.lerp(self.view, partial)) * 0.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swing_lasts_six_ticks_and_restarts_after_half() {
        let mut hand = HandAnimation::new(Vec2::ZERO);
        hand.swing();
        let mut progress = Vec::new();
        for _ in 0..9 {
            hand.tick(None, Vec2::ZERO);
            progress.push(hand.swing);
        }
        assert_eq!(progress[..7], [0.0, 1.0 / 6.0, 2.0 / 6.0, 0.5, 4.0 / 6.0, 5.0 / 6.0, 1.0]);
        assert_eq!(progress[7..], [0.0, 0.0]);

        // Restarting early in a swing does nothing, later it starts over.
        hand.swing();
        hand.tick(None, Vec2::ZERO);
        hand.tick(None, Vec2::ZERO);
        hand.swing();
        assert_eq!(hand.swing_ticks, Some(2));
        hand.tick(None, Vec2::ZERO);
        hand.tick(None, Vec2::ZERO);
        hand.swing();
        assert_eq!(hand.swing_ticks, Some(0));
    }

    #[test]
    fn switching_items_lowers_then_raises_the_hand() {
        let mut hand = HandAnimation::new(Vec2::ZERO);
        for _ in 0..20 {
            hand.tick(None, Vec2::ZERO);
        }
        assert_eq!(hand.lowered(1.0), 0.0);
        hand.tick(Some(Block::Stone), Vec2::ZERO);
        assert!(hand.lowered(1.0) > 0.0);
        assert_eq!(hand.shown, None);
        for _ in 0..3 {
            hand.tick(Some(Block::Stone), Vec2::ZERO);
        }
        assert_eq!(hand.shown, Some(Block::Stone));
        for _ in 0..20 {
            hand.tick(Some(Block::Stone), Vec2::ZERO);
        }
        assert_eq!(hand.lowered(1.0), 0.0);
    }
}
