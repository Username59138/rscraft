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

    /// How many of `block` would fit.
    pub fn room_for(&self, block: Block) -> usize {
        self.slots
            .iter()
            .map(|slot| match slot {
                None => MAX_STACK as usize,
                Some(s) if s.block == block => (MAX_STACK - s.count) as usize,
                Some(_) => 0,
            })
            .sum()
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

    /// Shift-click: moves a stack between the hotbar and the main inventory.
    pub fn quick_move(&mut self, index: usize) {
        let Some(stack) = self.slots[index].take() else { return };
        let targets = if index < HOTBAR { HOTBAR..SLOTS } else { 0..HOTBAR };
        let mut count = stack.count;
        for slot in self.slots[targets.clone()].iter_mut().flatten() {
            if slot.block == stack.block && slot.count < MAX_STACK {
                let moved = count.min(MAX_STACK - slot.count);
                slot.count += moved;
                count -= moved;
            }
        }
        if count > 0 && let Some(i) = targets.clone().find(|&i| self.slots[i].is_none()) {
            self.slots[i] = Some(ItemStack { count, ..stack });
            count = 0;
        }
        if count > 0 {
            self.slots[index] = Some(ItemStack { count, ..stack });
        }
    }

    /// A click on one of the 36 slots; see [`click_slot`].
    pub fn click(&mut self, index: usize, right: bool, cursor: &mut Option<ItemStack>) {
        click_slot(&mut self.slots[index], right, cursor);
    }

    /// Mouse wheel over the hotbar: positive notches go left, like vanilla.
    pub fn scroll(&mut self, notches: i32) {
        self.selected = (self.selected as i32 - notches).rem_euclid(HOTBAR as i32) as usize;
    }
}

/// A click on a slot with `cursor` being the stack held by the mouse,
/// following vanilla: left click picks up, puts down, merges or swaps;
/// right click picks up half or puts down one.
pub fn click_slot(slot: &mut Option<ItemStack>, right: bool, cursor: &mut Option<ItemStack>) {
    let with_count = |stack: ItemStack, count: u8| (count > 0).then_some(ItemStack { count, ..stack });
    (*slot, *cursor) = match (*slot, *cursor, right) {
        // Pick up all, or half rounded up.
        (Some(s), None, false) => (None, Some(s)),
        (Some(s), None, true) => (with_count(s, s.count / 2), with_count(s, s.count.div_ceil(2))),
        // Put down all, or one.
        (None, Some(c), false) => (Some(c), None),
        (None, Some(c), true) => (with_count(c, 1), with_count(c, c.count - 1)),
        // Same item: merge all, or one, up to a full stack.
        (Some(s), Some(c), _) if s.block == c.block => {
            let moved = if right { 1 } else { c.count }.min(MAX_STACK - s.count);
            (with_count(s, s.count + moved), with_count(c, c.count - moved))
        }
        // Different items: swap.
        (Some(s), Some(c), false) => (Some(c), Some(s)),
        unchanged => (unchanged.0, unchanged.1),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(block: Block, count: u8) -> Option<ItemStack> {
        Some(ItemStack { block, count })
    }

    #[test]
    fn add_fills_existing_stacks_then_hotbar() {
        let mut inv = Inventory::default();
        assert_eq!(inv.add(Block::Dirt, 70), 0);
        assert_eq!(inv.slots[0], stack(Block::Dirt, 64));
        assert_eq!(inv.slots[1], stack(Block::Dirt, 6));
        assert_eq!(inv.add(Block::Dirt, 3), 0);
        assert_eq!(inv.slots[1], stack(Block::Dirt, 9));
    }

    #[test]
    fn click_picks_up_splits_places_and_swaps() {
        let mut inv = Inventory::default();
        inv.slots[0] = stack(Block::Stone, 5);
        inv.slots[1] = stack(Block::Dirt, 2);
        let mut cursor = None;

        inv.click(0, true, &mut cursor); // right click takes half, rounded up
        assert_eq!((inv.slots[0], cursor), (stack(Block::Stone, 2), stack(Block::Stone, 3)));
        inv.click(2, true, &mut cursor); // right click puts down one
        assert_eq!((inv.slots[2], cursor), (stack(Block::Stone, 1), stack(Block::Stone, 2)));
        inv.click(1, false, &mut cursor); // different item: swap
        assert_eq!((inv.slots[1], cursor), (stack(Block::Stone, 2), stack(Block::Dirt, 2)));
        inv.click(3, false, &mut cursor); // empty slot: put all down
        assert_eq!((inv.slots[3], cursor), (stack(Block::Dirt, 2), None));
        inv.click(1, false, &mut cursor);
        inv.click(0, false, &mut cursor); // same item: merge
        assert_eq!((inv.slots[0], cursor), (stack(Block::Stone, 4), None));
    }

    #[test]
    fn quick_move_goes_between_hotbar_and_main() {
        let mut inv = Inventory::default();
        inv.slots[0] = stack(Block::Sand, 10);
        inv.quick_move(0);
        assert_eq!((inv.slots[0], inv.slots[HOTBAR]), (None, stack(Block::Sand, 10)));
        inv.quick_move(HOTBAR);
        assert_eq!(inv.slots[0], stack(Block::Sand, 10));
    }
}
