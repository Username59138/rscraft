//! GUI textures from the client jar packed into a single atlas: the bitmap
//! font, widget sprites (with their nine-slice borders) and HUD sprites.

use std::collections::HashMap;

use rscraft_assets::{ClientJar, RgbaImage};
use rscraft_world::Block;

const ATLAS_WIDTH: u32 = 256;
const FONT_CELL: u32 = 8;
/// Width of a space in the default font.
const SPACE_ADVANCE: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sprite {
    White,
    Button,
    ButtonHighlighted,
    ButtonDisabled,
    TextField,
    TextFieldHighlighted,
    Slider,
    SliderHighlighted,
    SliderHandle,
    SliderHandleHighlighted,
    MenuBackground,
    InworldMenuBackground,
    HeaderSeparator,
    FooterSeparator,
    Crosshair,
    Hotbar,
    HotbarSelection,
    InventoryScreen,
    GameModeSwitcher,
    SwitcherSlot,
    SwitcherSelection,
    IronSword,
    Map,
    EnderEye,
}

impl Sprite {
    const ALL: [Sprite; 24] = [
        Sprite::White,
        Sprite::Button,
        Sprite::ButtonHighlighted,
        Sprite::ButtonDisabled,
        Sprite::TextField,
        Sprite::TextFieldHighlighted,
        Sprite::Slider,
        Sprite::SliderHighlighted,
        Sprite::SliderHandle,
        Sprite::SliderHandleHighlighted,
        Sprite::MenuBackground,
        Sprite::InworldMenuBackground,
        Sprite::HeaderSeparator,
        Sprite::FooterSeparator,
        Sprite::Crosshair,
        Sprite::Hotbar,
        Sprite::HotbarSelection,
        Sprite::InventoryScreen,
        Sprite::GameModeSwitcher,
        Sprite::SwitcherSlot,
        Sprite::SwitcherSelection,
        Sprite::IronSword,
        Sprite::Map,
        Sprite::EnderEye,
    ];

    /// Path under `assets/minecraft/textures/`.
    fn path(self) -> Option<&'static str> {
        Some(match self {
            Sprite::White => return None,
            Sprite::Button => "gui/sprites/widget/button",
            Sprite::ButtonHighlighted => "gui/sprites/widget/button_highlighted",
            Sprite::ButtonDisabled => "gui/sprites/widget/button_disabled",
            Sprite::TextField => "gui/sprites/widget/text_field",
            Sprite::TextFieldHighlighted => "gui/sprites/widget/text_field_highlighted",
            Sprite::Slider => "gui/sprites/widget/slider",
            Sprite::SliderHighlighted => "gui/sprites/widget/slider_highlighted",
            Sprite::SliderHandle => "gui/sprites/widget/slider_handle",
            Sprite::SliderHandleHighlighted => "gui/sprites/widget/slider_handle_highlighted",
            Sprite::MenuBackground => "gui/menu_background",
            Sprite::InworldMenuBackground => "gui/inworld_menu_background",
            Sprite::HeaderSeparator => "gui/header_separator",
            Sprite::FooterSeparator => "gui/footer_separator",
            Sprite::Crosshair => "gui/sprites/hud/crosshair",
            Sprite::Hotbar => "gui/sprites/hud/hotbar",
            Sprite::HotbarSelection => "gui/sprites/hud/hotbar_selection",
            Sprite::InventoryScreen => "gui/container/inventory",
            Sprite::GameModeSwitcher => "gui/container/gamemode_switcher",
            Sprite::SwitcherSlot => "gui/sprites/gamemode_switcher/slot",
            Sprite::SwitcherSelection => "gui/sprites/gamemode_switcher/selection",
            Sprite::IronSword => "item/iron_sword",
            Sprite::Map => "item/map",
            Sprite::EnderEye => "item/ender_eye",
        })
    }
}

/// Border widths for nine-slice scaling (left, top, right, bottom).
#[derive(Clone, Copy, Debug, Default)]
pub struct Border {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// A region of the atlas, in pixels.
#[derive(Clone, Copy, Debug)]
pub struct AtlasRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SpriteInfo {
    pub rect: AtlasRect,
    pub border: Option<Border>,
}

#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub rect: AtlasRect,
    pub advance: f32,
}

/// The packed GUI atlas and where everything is in it.
pub struct GuiAtlas {
    pub image: RgbaImage,
    sprites: HashMap<Sprite, SpriteInfo>,
    glyphs: HashMap<char, Glyph>,
    /// Top, left and right faces of each block's inventory icon.
    icons: HashMap<Block, [AtlasRect; 3]>,
}

/// What an image in the atlas is.
enum Entry {
    Sprite(Sprite, Option<Border>),
    Font,
    Icon(Block, usize),
}

impl GuiAtlas {
    /// `icons` are the top, south and east face images of each block.
    pub fn load(mut jar: Option<&mut ClientJar>, icons: &[(Block, [RgbaImage; 3])]) -> Self {
        let mut images: Vec<(Entry, RgbaImage)> = Vec::new();
        for sprite in Sprite::ALL {
            let (image, border) = match sprite.path() {
                None => (solid(1, 1, [255; 4]), None),
                Some(path) => match jar.as_deref_mut().map(|jar| load_sprite(jar, path)) {
                    Some(Ok(loaded)) => loaded,
                    Some(Err(err)) => {
                        log::warn!("GUI sprite {path}: {err:#}");
                        (solid(16, 16, [110, 110, 110, 255]), None)
                    }
                    None => (solid(16, 16, [110, 110, 110, 255]), None),
                },
            };
            images.push((Entry::Sprite(sprite, border), image));
        }
        for (block, faces) in icons {
            for (i, face) in faces.iter().enumerate() {
                images.push((Entry::Icon(*block, i), face.clone()));
            }
        }
        let (font_image, font_chars) = match jar.as_deref_mut().map(load_font) {
            Some(Ok(font)) => font,
            Some(Err(err)) => {
                log::warn!("font: {err:#}");
                (solid(1, 1, [0; 4]), Vec::new())
            }
            None => (solid(1, 1, [0; 4]), Vec::new()),
        };
        images.push((Entry::Font, font_image));

        // Shelf packing: tallest first, left to right, new row when full.
        let mut order: Vec<usize> = (0..images.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(images[i].1.height));
        let mut positions = vec![(0, 0); images.len()];
        let (mut x, mut y, mut row_height) = (0, 0, 0);
        for &i in &order {
            let image = &images[i].1;
            if x + image.width > ATLAS_WIDTH {
                x = 0;
                y += row_height;
                row_height = 0;
            }
            positions[i] = (x, y);
            x += image.width;
            row_height = row_height.max(image.height);
        }
        let height = (y + row_height).next_power_of_two();
        let mut atlas = RgbaImage::new(ATLAS_WIDTH, height);

        let mut sprites = HashMap::new();
        let mut glyphs = HashMap::new();
        let mut icon_rects: HashMap<Block, [AtlasRect; 3]> = HashMap::new();
        for (i, (entry, image)) in images.iter().enumerate() {
            let (px, py) = positions[i];
            for row in 0..image.height {
                for col in 0..image.width {
                    atlas.set_pixel(px + col, py + row, image.pixel(col, row));
                }
            }
            let rect = AtlasRect { x: px as f32, y: py as f32, w: image.width as f32, h: image.height as f32 };
            match entry {
                Entry::Sprite(sprite, border) => {
                    sprites.insert(*sprite, SpriteInfo { rect, border: *border });
                }
                Entry::Icon(block, face) => {
                    icon_rects.entry(*block).or_insert([rect; 3])[*face] = rect;
                }
                Entry::Font => {
                    for (ch, cx, cy, width) in &font_chars {
                        let rect = AtlasRect {
                            x: (px + cx * FONT_CELL) as f32,
                            y: (py + cy * FONT_CELL) as f32,
                            w: *width as f32,
                            h: FONT_CELL as f32,
                        };
                        let advance = if *ch == ' ' { SPACE_ADVANCE } else { *width as f32 + 1.0 };
                        glyphs.insert(*ch, Glyph { rect, advance });
                    }
                }
            }
        }

        Self { image: atlas, sprites, glyphs, icons: icon_rects }
    }

    pub fn icon(&self, block: Block) -> Option<[AtlasRect; 3]> {
        self.icons.get(&block).copied()
    }

    pub fn sprite(&self, sprite: Sprite) -> SpriteInfo {
        self.sprites[&sprite]
    }

    pub fn glyph(&self, ch: char) -> Option<Glyph> {
        self.glyphs.get(&ch).or_else(|| self.glyphs.get(&'?')).copied()
    }
}

fn solid(width: u32, height: u32, rgba: [u8; 4]) -> RgbaImage {
    let mut image = RgbaImage::new(width, height);
    image.pixels.chunks_exact_mut(4).for_each(|p| p.copy_from_slice(&rgba));
    image
}

/// Loads a sprite and its nine-slice border from the `.mcmeta` next to it.
fn load_sprite(jar: &mut ClientJar, path: &str) -> anyhow::Result<(RgbaImage, Option<Border>)> {
    let image = jar.texture(path)?;
    let border = jar
        .read(&format!("assets/minecraft/textures/{path}.png.mcmeta"))
        .ok()
        .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
        .and_then(|meta| {
            let scaling = &meta["gui"]["scaling"];
            if scaling["type"] != "nine_slice" {
                return None;
            }
            let border = &scaling["border"];
            let side = |name: &str| border.as_f64().or_else(|| border[name].as_f64()).unwrap_or(0.0) as f32;
            Some(Border { left: side("left"), top: side("top"), right: side("right"), bottom: side("bottom") })
        });
    Ok((image, border))
}

/// Loads `font/ascii.png` and returns it with (char, cell x, cell y, pixel width)
/// for every glyph listed in the font definition.
fn load_font(jar: &mut ClientJar) -> anyhow::Result<(RgbaImage, Vec<(char, u32, u32, u32)>)> {
    let definition: serde_json::Value = serde_json::from_slice(&jar.read("assets/minecraft/font/include/default.json")?)?;
    let provider = definition["providers"]
        .as_array()
        .and_then(|providers| providers.iter().find(|p| p["file"] == "minecraft:font/ascii.png"))
        .ok_or_else(|| anyhow::anyhow!("ascii font provider not found"))?;
    let image = jar.texture("font/ascii")?;

    let mut chars = Vec::new();
    for (row, line) in provider["chars"].as_array().into_iter().flatten().enumerate() {
        for (col, ch) in line.as_str().unwrap_or_default().chars().enumerate() {
            if ch == '\0' {
                continue;
            }
            let (cx, cy) = (col as u32, row as u32);
            // Glyph width: up to the rightmost non-transparent column.
            let width = (0..FONT_CELL)
                .rev()
                .find(|&x| (0..FONT_CELL).any(|y| image.pixel(cx * FONT_CELL + x, cy * FONT_CELL + y)[3] > 0))
                .map_or(0, |x| x + 1);
            chars.push((ch, cx, cy, width));
        }
    }
    Ok((image, chars))
}
