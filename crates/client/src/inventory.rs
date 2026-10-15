//! The player's inventory: 9 hotbar slots followed by 27 main slots, like vanilla.

use rscraft_world::Block;

pub const SLOTS: usize = 36;
pub const HOTBAR: usize = 9;
pub const MAX_STACK: u8 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemStack {
    pub block: Block,
    pub count: u8,
}

pub struct Inventory {
    pub slots: [Option<ItemStack>; SLOTS],
    /// Selected hotbar slot, 0..9.
    pub selected: usize,
}

impl Default for Inventory {
    fn default() -> Self {
        Self { slots: [None; SLOTS], selected: 0 }
    }
}

impl Inventory {
    /// Adds items, topping up existing stacks first, then filling empty
    /// slots (hotbar first). Returns how many didn't fit.
    pub fn add(&mut self, block: Block, mut count: u8) -> u8 {
        for slot in self.slots.iter_mut().flatten() {
            if slot.block == block && slot.count < MAX_STACK {
                let moved = count.min(MAX_STACK - slot.count);
                slot.count += moved;
                count -= moved;
                if count == 0 {
                    return 0;
                }
            }
        }
        for slot in self.slots.iter_mut().filter(|s| s.is_none()) {
            let moved = count.min(MAX_STACK);
            *slot = Some(ItemStack { block, count: moved });
            count -= moved;
            if count == 0 {
                return 0;
            }
        }
        count
    }

    pub fn selected_stack(&self) -> Option<ItemStack> {
        self.slots[self.selected]
    }

    /// Removes one item from the selected slot and returns its block.
    pub fn take_selected(&mut self) -> Option<Block> {
        let slot = &mut self.slots[self.selected];
        let stack = slot.as_mut()?;
        let block = stack.block;
        stack.count -= 1;
        if stack.count == 0 {
            *slot = None;
        }
        Some(block)
    }

    /// Mouse wheel over the hotbar: positive notches go left, like vanilla.
    pub fn scroll(&mut self, notches: i32) {
        self.selected = (self.selected as i32 - notches).rem_euclid(HOTBAR as i32) as usize;
    }
}
