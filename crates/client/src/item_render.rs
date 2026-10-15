//! Drawing dropped items: small spinning, bobbing blocks with shadows.

use glam::{Mat4, Vec3};
use rscraft_world::dropped_items::{DroppedItems, HALF_SIZE};

use crate::mesh::MeshData;
use crate::shadows::Caster;
use crate::texture::BlockLayers;

/// Model transform of a dropped block centred at `position`, turned by `angle`.
fn item_transform(position: Vec3, angle: f32) -> Mat4 {
    Mat4::from_translation(position)
        * Mat4::from_rotation_y(angle)
        * Mat4::from_scale(Vec3::splat(HALF_SIZE * 2.0))
        * Mat4::from_translation(Vec3::splat(-0.5))
}

/// Spinning, bobbing cubes, ready to draw with the item pipeline. `light`
/// gives the sky light level (0..1) at a position; `player` is where
/// picked-up items fly to (the player's feet).
pub fn build_mesh(
    items: &DroppedItems,
    layers: &BlockLayers,
    seconds: f32,
    player: Option<Vec3>,
    light: impl Fn(Vec3) -> f32,
) -> MeshData {
    let mut mesh = MeshData::default();
    if let Some(feet) = player {
        for (block, position) in items.flying_to(feet) {
            mesh.push_block(block, layers, item_transform(position, seconds), light(position));
        }
    }
    for (i, (block, position)) in items.visible().enumerate() {
        let phase = seconds + i as f32 * 1.7;
        let bob = (phase * 2.0).sin() * 0.05 + 0.1;
        mesh.push_block(block, layers, item_transform(position + Vec3::Y * bob, phase), light(position));
    }
    mesh
}

/// Shadow casters for every item, like vanilla's (radius 0.15, strength 0.75).
pub fn shadow_casters(items: &DroppedItems) -> impl Iterator<Item = Caster> + '_ {
    items.visible().map(|(_, p)| Caster { feet: p - Vec3::Y * HALF_SIZE, radius: 0.15, strength: 0.75 })
}
