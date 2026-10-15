//! The survival inventory screen (E), drawn with vanilla's container texture.

use winit::keyboard::KeyCode;

use crate::inventory::{HOTBAR, Inventory, ItemStack, SLOTS};
use crate::ui::{Rect, Sprite, Ui};

const WIDTH: f32 = 176.0;
const HEIGHT: f32 = 166.0;

pub enum InventoryAction {
    None,
    Close,
}

/// Top-left of the 16×16 item area of a slot, relative to the screen origin.
fn slot_position(index: usize) -> (f32, f32) {
    if index < HOTBAR {
        (8.0 + index as f32 * 18.0, 142.0)
    } else {
        let i = index - HOTBAR;
        (8.0 + (i % 9) as f32 * 18.0, 84.0 + (i / 9) as f32 * 18.0)
    }
}

pub fn draw(ui: &mut Ui, inventory: &mut Inventory, cursor: &mut Option<ItemStack>, shift: bool) -> InventoryAction {
    // Vanilla's in-game screen background, about 75% black.
    ui.fill(Rect::new(0.0, 0.0, ui.width, ui.height), 0x101010, 0.75);
    let left = ((ui.width - WIDTH) / 2.0).floor();
    let top = ((ui.height - HEIGHT) / 2.0).floor();
    ui.sprite_region(Sprite::InventoryScreen, 0.0, 0.0, Rect::new(left, top, WIDTH, HEIGHT));
    ui.text_flat(left + 97.0, top + 8.0, "Crafting", 0x404040);

    let (clicked, right_clicked) = (ui.input.clicked, ui.input.right_clicked);
    for index in 0..SLOTS {
        let (x, y) = slot_position(index);
        let rect = Rect::new(left + x, top + y, 16.0, 16.0);
        if let Some(stack) = inventory.slots[index] {
            ui.item(stack.block, stack.count, rect.x, rect.y);
        }
        if rect.contains(ui.input.mouse) {
            // Vanilla's white highlight over the hovered slot.
            ui.fill(rect, 0xffffff, 0.5);
            if clicked && shift {
                inventory.quick_move(index);
            } else if clicked || right_clicked {
                inventory.click(index, right_clicked, cursor);
            }
        }
    }

    // The held stack follows the mouse.
    if let Some(stack) = cursor {
        ui.item(stack.block, stack.count, ui.input.mouse.x - 8.0, ui.input.mouse.y - 8.0);
    }

    if ui.input.pressed(KeyCode::Escape) || ui.input.pressed(KeyCode::KeyE) {
        return InventoryAction::Close;
    }
    InventoryAction::None
}
