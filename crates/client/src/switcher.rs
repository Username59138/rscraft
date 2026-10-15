//! The F3+F4 game mode switcher, laid out like vanilla's.

use rscraft_world::Block;

use crate::gamemode::GameMode;
use crate::ui::{Rect, Sprite, Ui, WHITE};

const AQUA: u32 = 0x55ffff;
const SLOT: f32 = 26.0;
const SPACING: f32 = 31.0;

/// Draws the switcher with `selected` highlighted.
pub fn draw(ui: &mut Ui, selected: GameMode) {
    let (cx, cy) = ((ui.width / 2.0).floor(), (ui.height / 2.0).floor());
    let top = cy - 31.0 - 27.0;
    ui.sprite_region(Sprite::GameModeSwitcher, 0.0, 0.0, Rect::new(cx - 62.0, top, 125.0, 75.0));
    ui.text_centered(cx, cy - 31.0 - 20.0, selected.name(), WHITE);

    let count = GameMode::SWITCHER.len() as f32;
    let left = cx - (count * SPACING - 5.0) / 2.0;
    for (i, mode) in GameMode::SWITCHER.into_iter().enumerate() {
        let (x, y) = (left + i as f32 * SPACING, cy - 31.0);
        ui.sprite(Sprite::SwitcherSlot, Rect::new(x, y, SLOT, SLOT), [1.0; 4]);
        let icon = Rect::new(x + 5.0, y + 5.0, 16.0, 16.0);
        match mode {
            GameMode::Creative => ui.item(Block::Grass, 1, icon.x, icon.y),
            GameMode::Survival => ui.sprite(Sprite::IronSword, icon, [1.0; 4]),
            GameMode::Adventure => ui.sprite(Sprite::Map, icon, [1.0; 4]),
            GameMode::Spectator => ui.sprite(Sprite::EnderEye, icon, [1.0; 4]),
        }
        if mode == selected {
            ui.sprite(Sprite::SwitcherSelection, Rect::new(x, y, SLOT, SLOT), [1.0; 4]);
        }
    }

    // "[ F4 ] Next", with the key in aqua.
    let (key, rest) = ("[ F4 ]", " Next");
    let width = ui.text_width(key) + ui.text_width(rest) + 1.0;
    let x = (cx - width / 2.0).floor();
    ui.text(x, cy + 5.0, key, AQUA);
    ui.text(x + ui.text_width(key) + 1.0, cy + 5.0, rest, WHITE);
}
