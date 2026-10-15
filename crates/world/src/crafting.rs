//! The 2×2 crafting grid in the inventory and its recipes.

use crate::Block;

use crate::inventory::{Inventory, ItemStack, MAX_STACK};

/// The four grid slots, row by row.
pub type Grid = [Option<ItemStack>; 4];

/// What the grid currently makes, if anything. Recipes are vanilla's:
/// a log alone makes 4 planks, and four planks make a crafting table.
pub fn result(grid: &Grid) -> Option<ItemStack> {
    let blocks: Vec<Block> = grid.iter().flatten().map(|s| s.block).collect();
    match blocks.as_slice() {
        [Block::OAK_LOG] => Some(ItemStack { block: Block::OakPlanks, count: 4 }),
        [Block::OakPlanks, Block::OakPlanks, Block::OakPlanks, Block::OakPlanks] => {
            Some(ItemStack { block: Block::CraftingTable, count: 1 })
        }
        _ => None,
    }
}

/// Uses up one item from every filled grid slot.
fn consume(grid: &mut Grid) {
    for slot in grid.iter_mut() {
        if let Some(stack) = slot {
            stack.count -= 1;
            if stack.count == 0 {
                *slot = None;
            }
        }
    }
}

/// Clicking the result slot crafts once into the stack on the cursor,
/// if it fits there.
pub fn take_result(grid: &mut Grid, cursor: &mut Option<ItemStack>) {
    let Some(made) = result(grid) else { return };
    match cursor {
        None => *cursor = Some(made),
        Some(held) if held.block == made.block && held.count + made.count <= MAX_STACK => held.count += made.count,
        Some(_) => return,
    }
    consume(grid);
}

/// Shift-clicking the result crafts as many times as the ingredients and
/// the inventory allow.
pub fn take_all(grid: &mut Grid, inventory: &mut Inventory) {
    while let Some(made) = result(grid).filter(|m| inventory.room_for(m.block) >= m.count as usize) {
        inventory.add(made.block, made.count);
        consume(grid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(block: Block, count: u8) -> Option<ItemStack> {
        Some(ItemStack { block, count })
    }

    #[test]
    fn log_makes_planks_anywhere_and_planks_make_a_table() {
        assert_eq!(result(&[None, None, stack(Block::OAK_LOG, 3), None]), stack(Block::OakPlanks, 4));
        assert_eq!(result(&[stack(Block::OAK_LOG, 1), stack(Block::OAK_LOG, 1), None, None]), None);
        assert_eq!(result(&[stack(Block::OakPlanks, 1); 4]), stack(Block::CraftingTable, 1));
        assert_eq!(result(&[stack(Block::OakPlanks, 1), None, None, None]), None);
    }

    #[test]
    fn taking_the_result_uses_up_ingredients() {
        let mut grid = [stack(Block::OAK_LOG, 2), None, None, None];
        let mut cursor = None;
        take_result(&mut grid, &mut cursor);
        take_result(&mut grid, &mut cursor);
        assert_eq!((grid[0], cursor), (None, stack(Block::OakPlanks, 8)));

        let mut grid = [stack(Block::OAK_LOG, 3), None, None, None];
        let mut inventory = Inventory::default();
        take_all(&mut grid, &mut inventory);
        assert_eq!((grid[0], inventory.slots[0]), (None, stack(Block::OakPlanks, 12)));
    }
}
