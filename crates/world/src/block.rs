/// Block types known to the client. Later this becomes a full block-state
/// registry generated from Minecraft's data.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Block {
    #[default]
    Air,
    Stone,
    Dirt,
    Grass,
    Sand,
    Gravel,
    Bedrock,
    /// Water with vanilla's `level` block state: 0 is a source, 1..=7 flow
    /// away from one (7 is the thinnest), 8..=15 are falling.
    Water(u8),
    OakLog,
    OakLeaves,
    OakPlanks,
    CraftingTable,
}

/// The six cube faces, named after the direction they point to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    East,  // +X
    West,  // -X
    Up,    // +Y
    Down,  // -Y
    South, // +Z
    North, // -Z
}

impl Face {
    pub const ALL: [Face; 6] = [Face::East, Face::West, Face::Up, Face::Down, Face::South, Face::North];

    pub fn offset(self) -> [i32; 3] {
        match self {
            Face::East => [1, 0, 0],
            Face::West => [-1, 0, 0],
            Face::Up => [0, 1, 0],
            Face::Down => [0, -1, 0],
            Face::South => [0, 0, 1],
            Face::North => [0, 0, -1],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderLayer {
    /// Opaque, or fully transparent pixels cut out (leaves).
    Solid,
    /// Alpha blended, drawn after everything else (water).
    Translucent,
}

/// Colour applied to grayscale textures. Will depend on the biome later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tint {
    None,
    Grass,
    Foliage,
    Water,
}

/// Which texture a block face uses, by Minecraft texture name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FaceTexture {
    pub name: &'static str,
    pub tint: Tint,
    /// Tinted texture drawn on top of the untinted base (grass block sides).
    pub overlay: Option<&'static str>,
}

impl FaceTexture {
    const fn plain(name: &'static str) -> Self {
        Self { name, tint: Tint::None, overlay: None }
    }

    const fn tinted(name: &'static str, tint: Tint) -> Self {
        Self { name, tint, overlay: None }
    }
}

impl Block {
    /// A water source block.
    pub const WATER: Block = Block::Water(0);

    pub const ALL: [Block; 12] = [
        Block::Air,
        Block::Stone,
        Block::Dirt,
        Block::Grass,
        Block::Sand,
        Block::Gravel,
        Block::Bedrock,
        Block::WATER,
        Block::OakLog,
        Block::OakLeaves,
        Block::OakPlanks,
        Block::CraftingTable,
    ];

    /// Full opaque cube that hides the faces of its neighbours.
    pub fn is_opaque(self) -> bool {
        !matches!(self, Block::Air | Block::Water(_) | Block::OakLeaves)
    }

    /// Numeric id of the block type, its index in `ALL`; block state such
    /// as the water level is not part of it.
    pub fn index(self) -> usize {
        match self {
            Block::Air => 0,
            Block::Stone => 1,
            Block::Dirt => 2,
            Block::Grass => 3,
            Block::Sand => 4,
            Block::Gravel => 5,
            Block::Bedrock => 6,
            Block::Water(_) => 7,
            Block::OakLog => 8,
            Block::OakLeaves => 9,
            Block::OakPlanks => 10,
            Block::CraftingTable => 11,
        }
    }

    /// The block type with this numeric id, in its default state.
    pub fn from_id(id: u8) -> Option<Block> {
        Block::ALL.get(id as usize).copied()
    }

    /// Block state beyond the type, 0 for blocks without any.
    pub fn state(self) -> u8 {
        match self {
            Block::Water(level) => level,
            _ => 0,
        }
    }

    /// The block with this id and state, as stored by `index` and `state`.
    pub fn from_id_and_state(id: u8, state: u8) -> Option<Block> {
        match Block::from_id(id)? {
            Block::Water(_) if state < 16 => Some(Block::Water(state)),
            _ if state != 0 => None,
            block => Some(block),
        }
    }

    pub fn is_water(self) -> bool {
        matches!(self, Block::Water(_))
    }

    /// Whether the crosshair can select this block.
    pub fn is_targetable(self) -> bool {
        !matches!(self, Block::Air | Block::Water(_))
    }

    /// Vanilla hardness; None means it can't be broken.
    pub fn hardness(self) -> Option<f32> {
        match self {
            Block::Air | Block::Water(_) | Block::Bedrock => None,
            Block::Stone => Some(1.5),
            Block::Dirt | Block::Sand => Some(0.5),
            Block::Grass | Block::Gravel => Some(0.6),
            Block::OakLog => Some(2.0),
            Block::OakLeaves => Some(0.2),
            Block::OakPlanks => Some(2.0),
            Block::CraftingTable => Some(2.5),
        }
    }

    /// Seconds to break by hand. There are no tools yet, so every block
    /// breaks at the "right tool" speed: hardness × 1.5 s.
    pub fn break_seconds(self) -> Option<f32> {
        self.hardness().map(|h| h * 1.5)
    }

    /// The item dropped when this block is broken.
    pub fn drop(self) -> Option<Block> {
        match self {
            Block::Grass => Some(Block::Dirt),
            Block::Air | Block::Water(_) | Block::Bedrock | Block::OakLeaves => None,
            other => Some(other),
        }
    }

    /// Minecraft's id without the namespace, e.g. `oak_log`.
    pub fn id(self) -> &'static str {
        match self {
            Block::Air => "air",
            Block::Stone => "stone",
            Block::Dirt => "dirt",
            Block::Grass => "grass_block",
            Block::Sand => "sand",
            Block::Gravel => "gravel",
            Block::Bedrock => "bedrock",
            Block::Water(_) => "water",
            Block::OakLog => "oak_log",
            Block::OakLeaves => "oak_leaves",
            Block::OakPlanks => "oak_planks",
            Block::CraftingTable => "crafting_table",
        }
    }

    /// The block with this id, with or without the `minecraft:` namespace.
    pub fn from_name(name: &str) -> Option<Block> {
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        Block::ALL.into_iter().find(|b| b.id() == name)
    }

    /// Display name, like vanilla's.
    pub fn name(self) -> &'static str {
        match self {
            Block::Air => "Air",
            Block::Stone => "Stone",
            Block::Dirt => "Dirt",
            Block::Grass => "Grass Block",
            Block::Sand => "Sand",
            Block::Gravel => "Gravel",
            Block::Bedrock => "Bedrock",
            Block::Water(_) => "Water",
            Block::OakLog => "Oak Log",
            Block::OakLeaves => "Oak Leaves",
            Block::OakPlanks => "Oak Planks",
            Block::CraftingTable => "Crafting Table",
        }
    }

    /// Whether entities collide with this block.
    pub fn is_solid(self) -> bool {
        !matches!(self, Block::Air | Block::Water(_))
    }

    /// How much sky light this block absorbs: 0 = transparent, 15 = opaque.
    pub fn light_opacity(self) -> u8 {
        match self {
            Block::Air => 0,
            Block::Water(_) | Block::OakLeaves => 1,
            _ => 15,
        }
    }

    pub fn render_layer(self) -> Option<RenderLayer> {
        match self {
            Block::Air => None,
            Block::Water(_) => Some(RenderLayer::Translucent),
            _ => Some(RenderLayer::Solid),
        }
    }

    /// Whether the face of this block towards `neighbor` should be drawn.
    pub fn face_visible(self, neighbor: Block) -> bool {
        match self {
            Block::Air => false,
            Block::Water(_) => !neighbor.is_water() && !neighbor.is_opaque(),
            _ => !neighbor.is_opaque(),
        }
    }

    pub fn texture(self, face: Face) -> FaceTexture {
        match self {
            Block::Air | Block::Stone => FaceTexture::plain("stone"),
            Block::Dirt => FaceTexture::plain("dirt"),
            Block::Grass => match face {
                Face::Up => FaceTexture::tinted("grass_block_top", Tint::Grass),
                Face::Down => FaceTexture::plain("dirt"),
                _ => FaceTexture {
                    name: "grass_block_side",
                    tint: Tint::Grass,
                    overlay: Some("grass_block_side_overlay"),
                },
            },
            Block::Sand => FaceTexture::plain("sand"),
            Block::Gravel => FaceTexture::plain("gravel"),
            Block::Bedrock => FaceTexture::plain("bedrock"),
            Block::Water(_) => FaceTexture::tinted("water_still", Tint::Water),
            Block::OakLog => match face {
                Face::Up | Face::Down => FaceTexture::plain("oak_log_top"),
                _ => FaceTexture::plain("oak_log"),
            },
            Block::OakLeaves => FaceTexture::tinted("oak_leaves", Tint::Foliage),
            Block::OakPlanks => FaceTexture::plain("oak_planks"),
            // Like vanilla's model: the front texture on north and west.
            Block::CraftingTable => FaceTexture::plain(match face {
                Face::Up => "crafting_table_top",
                Face::Down => "oak_planks",
                Face::North | Face::West => "crafting_table_front",
                Face::South | Face::East => "crafting_table_side",
            }),
        }
    }
}
