//! Finding the block under the crosshair.

use glam::{IVec3, Vec3};
use rscraft_world::Block;

/// Survival reach, like vanilla.
pub const REACH: f32 = 4.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub block: IVec3,
    /// Normal of the face that was hit; the block placed would go at `block + normal`.
    pub normal: IVec3,
}

/// Walks the voxel grid along the ray (Amanatides & Woo DDA) and returns the
/// first targetable block within `max_distance`. Stops at unloaded blocks.
pub fn raycast(origin: Vec3, direction: Vec3, max_distance: f32, world: impl Fn(IVec3) -> Option<Block>) -> Option<Hit> {
    let dir = direction.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let mut block = origin.floor().as_ivec3();
    let step = dir.signum().as_ivec3();
    // Ray distance between grid lines on each axis, and to the first one.
    let delta = (Vec3::ONE / dir).abs();
    let mut next = Vec3::new(
        if dir.x > 0.0 { block.x as f32 + 1.0 - origin.x } else { origin.x - block.x as f32 } * delta.x,
        if dir.y > 0.0 { block.y as f32 + 1.0 - origin.y } else { origin.y - block.y as f32 } * delta.y,
        if dir.z > 0.0 { block.z as f32 + 1.0 - origin.z } else { origin.z - block.z as f32 } * delta.z,
    );
    let mut normal = IVec3::ZERO;
    let mut distance = 0.0;
    while distance <= max_distance {
        // The normal stays zero if the eye itself is inside the block.
        match world(block) {
            None => return None,
            Some(b) if b.is_targetable() => return Some(Hit { block, normal }),
            _ => {}
        }
        // Step along the axis whose next grid line is closest.
        let axis = if next.x < next.y && next.x < next.z {
            0
        } else if next.y < next.z {
            1
        } else {
            2
        };
        distance = next[axis];
        next[axis] += delta[axis];
        block[axis] += step[axis];
        normal = IVec3::ZERO;
        normal[axis] = -step[axis];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_ground_looking_down_and_forward() {
        let world = |p: IVec3| Some(if p.y <= 64 { Block::Sand } else { Block::Air });
        let hit = raycast(Vec3::new(-7.5, 66.62, 24.5), Vec3::new(0.0000022, -0.4794, 0.8776), REACH, world);
        assert_eq!(hit.map(|h| (h.block.y, h.normal)), Some((64, IVec3::Y)), "{hit:?}");
    }
}
