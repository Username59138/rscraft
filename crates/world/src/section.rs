use crate::Block;

pub const SECTION_SIZE: i32 = 16;
pub const SECTION_VOLUME: usize = (SECTION_SIZE * SECTION_SIZE * SECTION_SIZE) as usize;

/// A 16×16×16 cube of blocks. Sections made of a single block (air high up,
/// stone deep down) take no extra memory.
#[derive(Clone, Debug)]
pub enum Section {
    Uniform(Block),
    Blocks(Box<[Block; SECTION_VOLUME]>),
}

impl Section {
    #[inline]
    pub fn index(x: i32, y: i32, z: i32) -> usize {
        ((y * SECTION_SIZE + z) * SECTION_SIZE + x) as usize
    }

    /// Coordinates are local, 0..16.
    #[inline]
    pub fn get(&self, x: i32, y: i32, z: i32) -> Block {
        match self {
            Section::Uniform(block) => *block,
            Section::Blocks(blocks) => blocks[Self::index(x, y, z)],
        }
    }

    /// Coordinates are local, 0..16.
    pub fn set(&mut self, x: i32, y: i32, z: i32, block: Block) {
        if let Section::Uniform(current) = *self {
            if current == block {
                return;
            }
            *self = Section::Blocks(Box::new([current; SECTION_VOLUME]));
        }
        if let Section::Blocks(blocks) = self {
            blocks[Self::index(x, y, z)] = block;
        }
    }

    /// Collapses a full block array into `Uniform` when possible.
    pub fn from_blocks(blocks: Box<[Block; SECTION_VOLUME]>) -> Self {
        let first = blocks[0];
        if blocks.iter().all(|&b| b == first) { Section::Uniform(first) } else { Section::Blocks(blocks) }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Section::Uniform(Block::Air))
    }
}
