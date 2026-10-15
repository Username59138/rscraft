//! The survival inventory screen (E), drawn with vanilla's container texture.

use winit::keyboard::KeyCode;

use crate::ui::{Rect, Sprite, Ui};
use rscraft_world::crafting::{self, Grid};
use rscraft_world::inventory::{self as inv, HOTBAR, Inventory, ItemStack, SLOTS};

const WIDTH: f32 = 176.0;
const HEIGHT: f32 = 166.0;
/// The 2×2 crafting grid and its result slot.
const GRID: [(f32, f32); 4] = [(98.0, 18.0), (116.0, 18.0), (98.0, 36.0), (116.0, 36.0)];
const RESULT: (f32, f32) = (154.0, 28.0);
/// The black box where the player is shown.
const PREVIEW: Rect = Rect::new(26.0, 8.0, 49.0, 70.0);

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

fn origin(ui: &Ui) -> (f32, f32) {
    (((ui.width - WIDTH) / 2.0).floor(), ((ui.height - HEIGHT) / 2.0).floor())
}

/// Where the player model goes, in GUI units.
pub fn preview_rect(ui: &Ui) -> Rect {
    let (left, top) = origin(ui);
    Rect::new(left + PREVIEW.x, top + PREVIEW.y, PREVIEW.w, PREVIEW.h)
}

/// A slot: draws its item and hover highlight, returns whether the mouse is over it.
fn slot(ui: &mut Ui, x: f32, y: f32, stack: Option<ItemStack>) -> bool {
    let rect = Rect::new(x, y, 16.0, 16.0);
    if let Some(stack) = stack {
        ui.item(stack.block, stack.count, x, y);
    }
    let hovered = rect.contains(ui.input.mouse);
    if hovered {
        // Vanilla's white highlight over the hovered slot.
        ui.fill(rect, 0xffffff, 0.5);
    }
    hovered
}

pub fn draw(
    ui: &mut Ui,
    inventory: &mut Inventory,
    grid: &mut Grid,
    cursor: &mut Option<ItemStack>,
    shift: bool,
) -> InventoryAction {
    // Vanilla's in-game screen background, about 75% black.
    ui.fill(Rect::new(0.0, 0.0, ui.width, ui.height), 0x101010, 0.75);
    let (left, top) = origin(ui);
    ui.sprite_region(Sprite::InventoryScreen, 0.0, 0.0, Rect::new(left, top, WIDTH, HEIGHT));
    ui.text_flat(left + 97.0, top + 8.0, "Crafting", 0x404040);

    let (clicked, right_clicked) = (ui.input.clicked, ui.input.right_clicked);
    for index in 0..SLOTS {
        let (x, y) = slot_position(index);
        if slot(ui, left + x, top + y, inventory.slots[index]) {
            if clicked && shift {
                inventory.quick_move(index);
            } else if clicked || right_clicked {
                inventory.click(index, right_clicked, cursor);
            }
        }
    }

    for (i, (x, y)) in GRID.into_iter().enumerate() {
        if slot(ui, left + x, top + y, grid[i]) {
            if clicked && shift {
                // Back into the inventory, whatever fits.
                if let Some(stack) = grid[i] {
                    let left = inventory.add(stack.block, stack.count);
                    grid[i] = (left > 0).then_some(ItemStack { count: left, ..stack });
                }
            } else if clicked || right_clicked {
                inv::click_slot(&mut grid[i], right_clicked, cursor);
            }
        }
    }
    if slot(ui, left + RESULT.0, top + RESULT.1, crafting::result(grid)) && clicked {
        if shift {
            crafting::take_all(grid, inventory);
        } else {
            crafting::take_result(grid, cursor);
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
