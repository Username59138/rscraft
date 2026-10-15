//! The in-game overlay: the hotbar.

use crate::inventory::{HOTBAR, Inventory};
use crate::ui::{Rect, Sprite, Ui};

/// Draws the hotbar at the bottom of the screen, like vanilla.
pub fn hotbar(ui: &mut Ui, inventory: &Inventory) {
    let x = (ui.width / 2.0 - 91.0).floor();
    let y = ui.height - 22.0;
    ui.sprite(Sprite::Hotbar, Rect::new(x, y, 182.0, 22.0), [1.0; 4]);
    ui.sprite(
        Sprite::HotbarSelection,
        Rect::new(x - 1.0 + inventory.selected as f32 * 20.0, y - 1.0, 24.0, 23.0),
        [1.0; 4],
    );
    for (i, stack) in inventory.slots[..HOTBAR].iter().enumerate() {
        if let Some(stack) = stack {
            ui.item(stack.block, stack.count, x + 3.0 + i as f32 * 20.0, y + 3.0);
        }
    }
}
