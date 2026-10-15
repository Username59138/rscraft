//! Immediate-mode GUI in Minecraft's style: every frame the current screen
//! describes itself, and the result is a list of textured quads.
//! Coordinates are in GUI units (screen pixels divided by the GUI scale).

pub mod atlas;
pub mod render;

use bytemuck::{Pod, Zeroable};
use glam::Vec2;
use rscraft_world::Block;
use winit::keyboard::KeyCode;

pub use atlas::{GuiAtlas, Sprite};
use atlas::{AtlasRect, Border};

pub const WHITE: u32 = 0xffffff;
pub const GRAY: u32 = 0xa0a0a0;
pub const LIGHT_GRAY: u32 = 0xe0e0e0;
pub const LINE_HEIGHT: f32 = 9.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct UiVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// A rectangle of the given size horizontally centred on `center_x`.
    pub fn centered(center_x: f32, y: f32, w: f32, h: f32) -> Self {
        Self::new((center_x - w / 2.0).floor(), y, w, h)
    }

    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }
}

/// Input gathered since the last frame.
#[derive(Default)]
pub struct UiInput {
    pub mouse: Vec2,
    pub mouse_down: bool,
    pub clicked: bool,
    pub right_clicked: bool,
    pub double_clicked: bool,
    pub scroll: f32,
    pub typed: String,
    pub keys: Vec<KeyCode>,
}

impl UiInput {
    pub fn pressed(&self, key: KeyCode) -> bool {
        self.keys.contains(&key)
    }

    /// Clears the per-frame events, keeping held state.
    pub fn end_frame(&mut self) {
        self.clicked = false;
        self.right_clicked = false;
        self.double_clicked = false;
        self.scroll = 0.0;
        self.typed.clear();
        self.keys.clear();
    }
}

/// Converts `0xRRGGBB` plus alpha to RGBA. GUI colours and tints are sRGB
/// values, like in Minecraft; the GUI shader takes care of the rest.
pub fn color(rgb: u32, alpha: f32) -> [f32; 4] {
    let channel = |shift: u32| ((rgb >> shift) & 0xff) as f32 / 255.0;
    [channel(16), channel(8), channel(0), alpha]
}

pub struct Ui {
    pub atlas: GuiAtlas,
    pub vertices: Vec<UiVertex>,
    /// Vertices drawn inverting what's behind them (the crosshair).
    pub inverted: std::ops::Range<usize>,
    pub input: UiInput,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    /// Id of the text field with keyboard focus.
    focused: Option<u32>,
    /// Id of the slider being dragged.
    dragging: Option<u32>,
    time: f32,
}

impl Ui {
    pub fn new(atlas: GuiAtlas) -> Self {
        Self {
            atlas,
            vertices: Vec::new(),
            inverted: 0..0,
            input: UiInput::default(),
            width: 1.0,
            height: 1.0,
            scale: 1.0,
            focused: None,
            dragging: None,
            time: 0.0,
        }
    }

    /// Minecraft's automatic GUI scale: the largest that keeps at least 320×240 units.
    pub fn auto_scale(width: u32, height: u32, max: u32) -> u32 {
        let mut scale = 1;
        while scale < max && width / (scale + 1) >= 320 && height / (scale + 1) >= 240 {
            scale += 1;
        }
        scale
    }

    pub fn begin(&mut self, width_px: u32, height_px: u32, scale: u32, dt: f32) {
        self.vertices.clear();
        self.inverted = 0..0;
        self.scale = scale as f32;
        self.width = (width_px as f32 / self.scale).ceil();
        self.height = (height_px as f32 / self.scale).ceil();
        self.time += dt;
        if !self.input.mouse_down {
            self.dragging = None;
        }
    }

    pub fn clear_focus(&mut self) {
        self.focused = None;
    }

    // ---- Primitives -------------------------------------------------------

    fn quad(&mut self, dst: Rect, src: AtlasRect, color: [f32; 4]) {
        let size = Vec2::new(self.atlas.image.width as f32, self.atlas.image.height as f32);
        let (u0, v0) = (src.x / size.x, src.y / size.y);
        let (u1, v1) = ((src.x + src.w) / size.x, (src.y + src.h) / size.y);
        let (x0, y0, x1, y1) = (dst.x, dst.y, dst.x + dst.w, dst.y + dst.h);
        let v = |x, y, u, v| UiVertex { position: [x, y], uv: [u, v], color };
        self.vertices.extend_from_slice(&[
            v(x0, y0, u0, v0),
            v(x1, y0, u1, v0),
            v(x1, y1, u1, v1),
            v(x0, y0, u0, v0),
            v(x1, y1, u1, v1),
            v(x0, y1, u0, v1),
        ]);
    }

    /// A textured quad with arbitrary corners, mapped to the source
    /// rectangle's top-left, top-right, bottom-right and bottom-left.
    fn quad_points(&mut self, p: [Vec2; 4], src: AtlasRect, color: [f32; 4]) {
        let size = Vec2::new(self.atlas.image.width as f32, self.atlas.image.height as f32);
        let (u0, v0) = (src.x / size.x, src.y / size.y);
        let (u1, v1) = ((src.x + src.w) / size.x, (src.y + src.h) / size.y);
        let uv = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
        let v = |i: usize| UiVertex { position: p[i].to_array(), uv: uv[i], color };
        self.vertices.extend_from_slice(&[v(0), v(1), v(2), v(0), v(2), v(3)]);
    }

    /// A block drawn as a small isometric cube in a 16×16 slot, like
    /// vanilla's inventory icons, with the stack size in the corner.
    pub fn item(&mut self, block: Block, count: u8, x: f32, y: f32) {
        if let Some([top, left, right]) = self.atlas.icon(block) {
            let p = |px: f32, py: f32| Vec2::new(x + px, y + py);
            let (l, t, r, c) = (p(1.0, 4.5), p(8.0, 1.0), p(15.0, 4.5), p(8.0, 8.0));
            let (lb, cb, rb) = (p(1.0, 12.0), p(8.0, 15.5), p(15.0, 12.0));
            let shade = |s: f32| [s, s, s, 1.0];
            self.quad_points([l, t, r, c], top, shade(1.0));
            self.quad_points([l, c, cb, lb], left, shade(0.8));
            self.quad_points([c, r, rb, cb], right, shade(0.6));
        }
        if count > 1 {
            let text = count.to_string();
            let width = self.text_width(&text);
            self.text(x + 17.0 - width, y + 9.0, &text, WHITE);
        }
    }

    /// Repeats `src` over `dst`, cropping the last row/column.
    fn tile(&mut self, dst: Rect, src: AtlasRect, color: [f32; 4]) {
        if src.w <= 0.0 || src.h <= 0.0 {
            return;
        }
        let mut y = 0.0;
        while y < dst.h {
            let h = src.h.min(dst.h - y);
            let mut x = 0.0;
            while x < dst.w {
                let w = src.w.min(dst.w - x);
                self.quad(
                    Rect::new(dst.x + x, dst.y + y, w, h),
                    AtlasRect { x: src.x, y: src.y, w, h },
                    color,
                );
                x += w;
            }
            y += h;
        }
    }

    pub fn fill(&mut self, rect: Rect, rgb: u32, alpha: f32) {
        let src = self.atlas.sprite(Sprite::White).rect;
        self.quad(rect, AtlasRect { w: 1.0, h: 1.0, ..src }, color(rgb, alpha));
    }

    /// Draws a sprite, honouring nine-slice borders; tiles otherwise.
    /// Draws a sprite that inverts the colours behind it, like the crosshair.
    /// Only one such sprite per frame.
    pub fn sprite_inverted(&mut self, sprite: Sprite, dst: Rect) {
        let start = self.vertices.len();
        self.sprite(sprite, dst, [1.0; 4]);
        self.inverted = start..self.vertices.len();
    }

    pub fn sprite(&mut self, sprite: Sprite, dst: Rect, tint: [f32; 4]) {
        let info = self.atlas.sprite(sprite);
        let Some(Border { left, top, right, bottom }) = info.border else {
            self.tile(dst, info.rect, tint);
            return;
        };
        let s = info.rect;
        let cols = [(0.0, left, 0.0, left), (left, s.w - left - right, left, dst.w - left - right), (s.w - right, right, dst.w - right, right)];
        let rows = [(0.0, top, 0.0, top), (top, s.h - top - bottom, top, dst.h - top - bottom), (s.h - bottom, bottom, dst.h - bottom, bottom)];
        for (sy, sh, dy, dh) in rows {
            for (sx, sw, dx, dw) in cols {
                if dw > 0.0 && dh > 0.0 {
                    self.tile(
                        Rect::new(dst.x + dx, dst.y + dy, dw, dh),
                        AtlasRect { x: s.x + sx, y: s.y + sy, w: sw, h: sh },
                        tint,
                    );
                }
            }
        }
    }

    /// Draws part of a sprite (offset and size in source pixels) at 1:1 scale.
    pub fn sprite_region(&mut self, sprite: Sprite, src_x: f32, src_y: f32, dst: Rect) {
        let rect = self.atlas.sprite(sprite).rect;
        let src = AtlasRect { x: rect.x + src_x, y: rect.y + src_y, w: dst.w, h: dst.h };
        self.quad(dst, src, [1.0; 4]);
    }

    /// Fills the screen with a darkened tiled texture, like Minecraft's menus.
    pub fn background(&mut self, sprite: Sprite, brightness: f32) {
        let tint = [brightness, brightness, brightness, 1.0];
        self.sprite(sprite, Rect::new(0.0, 0.0, self.width, self.height), tint);
    }

    // ---- Text -------------------------------------------------------------

    pub fn text_width(&self, text: &str) -> f32 {
        let width: f32 = text.chars().filter_map(|c| self.atlas.glyph(c)).map(|g| g.advance).sum();
        (width - 1.0).max(0.0)
    }

    fn glyphs(&mut self, x: f32, y: f32, text: &str, color: [f32; 4]) {
        let mut pen = x;
        for ch in text.chars() {
            let Some(glyph) = self.atlas.glyph(ch) else { continue };
            if glyph.rect.w > 0.0 {
                self.quad(Rect::new(pen, y, glyph.rect.w, glyph.rect.h), glyph.rect, color);
            }
            pen += glyph.advance;
        }
    }

    /// Draws text with Minecraft's drop shadow (a quarter-brightness copy offset by 1).
    pub fn text(&mut self, x: f32, y: f32, text: &str, rgb: u32) {
        let shadow = ((rgb & 0xfcfcfc) >> 2) & 0xffffff;
        self.glyphs(x + 1.0, y + 1.0, text, color(shadow, 1.0));
        self.glyphs(x, y, text, color(rgb, 1.0));
    }

    pub fn text_flat(&mut self, x: f32, y: f32, text: &str, rgb: u32) {
        self.glyphs(x, y, text, color(rgb, 1.0));
    }

    pub fn text_centered(&mut self, center_x: f32, y: f32, text: &str, rgb: u32) {
        let x = (center_x - self.text_width(text) / 2.0).floor();
        self.text(x, y, text, rgb);
    }

    // ---- Widgets ----------------------------------------------------------

    /// A standard button; returns true when clicked.
    pub fn button(&mut self, rect: Rect, label: &str, enabled: bool) -> bool {
        let hovered = enabled && rect.contains(self.input.mouse);
        let sprite = match (enabled, hovered) {
            (false, _) => Sprite::ButtonDisabled,
            (true, true) => Sprite::ButtonHighlighted,
            (true, false) => Sprite::Button,
        };
        self.sprite(sprite, rect, [1.0; 4]);
        let text_y = rect.y + ((rect.h - 8.0) / 2.0).floor();
        self.text_centered(rect.x + rect.w / 2.0, text_y, label, if enabled { WHITE } else { GRAY });
        hovered && self.input.clicked
    }

    /// A single-line text field; the caret is always at the end.
    pub fn text_field(&mut self, id: u32, rect: Rect, value: &mut String, max_len: usize) -> bool {
        if self.input.clicked {
            if rect.contains(self.input.mouse) {
                self.focused = Some(id);
            } else if self.focused == Some(id) {
                self.focused = None;
            }
        }
        let focused = self.focused == Some(id);
        let mut changed = false;
        if focused {
            for ch in self.input.typed.chars().filter(|c| !c.is_control()) {
                if value.chars().count() < max_len {
                    value.push(ch);
                    changed = true;
                }
            }
            if self.input.pressed(KeyCode::Backspace) && value.pop().is_some() {
                changed = true;
            }
        }

        let sprite = if focused { Sprite::TextFieldHighlighted } else { Sprite::TextField };
        self.sprite(sprite, rect, [1.0; 4]);
        let text_y = rect.y + ((rect.h - 8.0) / 2.0).floor();
        // Show the end of long text, like scrolling to the caret.
        let mut shown: &str = value;
        while self.text_width(shown) > rect.w - 12.0 && !shown.is_empty() {
            shown = &shown[shown.char_indices().nth(1).map_or(shown.len(), |(i, _)| i)..];
        }
        self.text(rect.x + 4.0, text_y, shown, LIGHT_GRAY);
        if focused && (self.time * 3.0) as u32 % 2 == 0 {
            let caret_x = rect.x + 4.0 + self.text_width(shown) + if shown.is_empty() { 0.0 } else { 1.0 };
            self.text(caret_x, text_y, "_", LIGHT_GRAY);
        }
        changed
    }

    /// A slider over 0..1 with a centred label; returns true when the value changed.
    pub fn slider(&mut self, id: u32, rect: Rect, label: &str, value: &mut f32) -> bool {
        let hovered = rect.contains(self.input.mouse);
        if hovered && self.input.clicked {
            self.dragging = Some(id);
        }
        let mut changed = false;
        if self.dragging == Some(id) {
            let new = ((self.input.mouse.x - rect.x - 4.0) / (rect.w - 8.0)).clamp(0.0, 1.0);
            changed = new != *value;
            *value = new;
        }
        let active = hovered || self.dragging == Some(id);
        self.sprite(if active { Sprite::SliderHighlighted } else { Sprite::Slider }, rect, [1.0; 4]);
        let handle = Rect::new((rect.x + *value * (rect.w - 8.0)).floor(), rect.y, 8.0, rect.h);
        self.sprite(if active { Sprite::SliderHandleHighlighted } else { Sprite::SliderHandle }, handle, [1.0; 4]);
        let text_y = rect.y + ((rect.h - 8.0) / 2.0).floor();
        self.text_centered(rect.x + rect.w / 2.0, text_y, label, WHITE);
        changed
    }
}
