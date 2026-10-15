//! Menu screens, laid out like Minecraft's. Each frame the active screen
//! draws itself and reports what the player asked for.

use winit::keyboard::KeyCode;

use crate::gamemode::GameMode;
use crate::saves::{self, Options, WorldEntry};
use crate::ui::{GRAY, Rect, Sprite, Ui, WHITE};

const DARK_GRAY: u32 = 0x808080;
const ENTRY_HEIGHT: f32 = 36.0;
const ENTRY_WIDTH: f32 = 270.0;
const HEADER_HEIGHT: f32 = 33.0;
const FOOTER_HEIGHT: f32 = 60.0;

pub enum Screen {
    /// In game, no menu open.
    None,
    Title,
    SelectWorld(SelectWorld),
    CreateWorld {
        name: String,
        seed: String,
        mode: GameMode,
    },
    Options {
        from_pause: bool,
    },
    Pause,
    Loading {
        progress: f32,
    },
    /// The player's inventory; drawn by the game, not here.
    Inventory,
}

pub struct SelectWorld {
    worlds: Vec<WorldEntry>,
    selected: Option<usize>,
    scroll: f32,
    confirm_delete: bool,
}

impl SelectWorld {
    pub fn load() -> Self {
        Self { worlds: saves::list_worlds(), selected: None, scroll: 0.0, confirm_delete: false }
    }
}

pub enum Action {
    None,
    Open(Screen),
    Play(WorldEntry),
    Resume,
    SaveAndQuit,
    Quit,
    /// Options were changed and should be applied right away.
    ApplyOptions,
    /// Options screen closed: save them and go back.
    CloseOptions {
        from_pause: bool,
    },
}

pub fn draw(screen: &mut Screen, ui: &mut Ui, options: &mut Options) -> Action {
    match screen {
        Screen::None => Action::None,
        Screen::Title => title(ui),
        Screen::SelectWorld(state) => select_world(ui, state),
        Screen::CreateWorld { name, seed, mode } => create_world(ui, name, seed, mode),
        Screen::Options { from_pause } => options_screen(ui, options, *from_pause),
        Screen::Pause => pause(ui),
        Screen::Loading { progress } => loading(ui, *progress),
        Screen::Inventory => Action::None,
    }
}

/// Darkens the world behind a menu.
fn dim(ui: &mut Ui, alpha: f32) {
    ui.fill(Rect::new(0.0, 0.0, ui.width, ui.height), 0x000000, alpha);
}

fn title(ui: &mut Ui) -> Action {
    let cx = ui.width / 2.0;
    let y = (ui.height / 4.0 + 48.0).floor();
    let mut action = Action::None;
    if ui.button(Rect::centered(cx, y, 200.0, 20.0), "Singleplayer", true) {
        action = Action::Open(Screen::SelectWorld(SelectWorld::load()));
    }
    ui.button(Rect::centered(cx, y + 24.0, 200.0, 20.0), "Multiplayer", false);
    if ui.button(Rect::new(cx - 100.0, y + 84.0, 98.0, 20.0), "Options...", true) {
        action = Action::Open(Screen::Options { from_pause: false });
    }
    if ui.button(Rect::new(cx + 2.0, y + 84.0, 98.0, 20.0), "Quit Game", true) {
        action = Action::Quit;
    }
    let version = format!("rscraft {} (Minecraft {} assets)", env!("CARGO_PKG_VERSION"), crate::MC_VERSION);
    ui.text(2.0, ui.height - 10.0, &version, WHITE);
    action
}

fn select_world(ui: &mut Ui, state: &mut SelectWorld) -> Action {
    let cx = ui.width / 2.0;
    let (top, bottom) = (HEADER_HEIGHT, ui.height - FOOTER_HEIGHT);
    let mut action = Action::None;

    // List area.
    ui.fill(Rect::new(0.0, top, ui.width, bottom - top), 0x000000, 0.6);
    ui.sprite(Sprite::HeaderSeparator, Rect::new(0.0, top - 2.0, ui.width, 2.0), [1.0; 4]);
    ui.sprite(Sprite::FooterSeparator, Rect::new(0.0, bottom, ui.width, 2.0), [1.0; 4]);
    ui.text_centered(cx, 12.0, "Select World", WHITE);

    let content = state.worlds.len() as f32 * ENTRY_HEIGHT + 8.0;
    let max_scroll = (content - (bottom - top)).max(0.0);
    if ui.input.mouse.y >= top && ui.input.mouse.y < bottom {
        state.scroll = (state.scroll - ui.input.scroll * ENTRY_HEIGHT / 2.0).clamp(0.0, max_scroll);
    }
    let x = (cx - ENTRY_WIDTH / 2.0).floor();
    for (i, world) in state.worlds.iter().enumerate() {
        let rect = Rect::new(x, top + 4.0 + i as f32 * ENTRY_HEIGHT - state.scroll, ENTRY_WIDTH, ENTRY_HEIGHT - 4.0);
        // No clipping in the UI yet, so only fully visible entries are drawn.
        if rect.y < top || rect.y + rect.h > bottom {
            continue;
        }
        if ui.input.clicked && rect.contains(ui.input.mouse) {
            if state.selected == Some(i) && ui.input.double_clicked {
                action = Action::Play(world.clone());
            }
            if state.selected != Some(i) {
                state.confirm_delete = false;
            }
            state.selected = Some(i);
        }
        if state.selected == Some(i) {
            ui.fill(Rect::new(rect.x - 2.0, rect.y - 2.0, rect.w + 4.0, rect.h + 4.0), 0xffffff, 1.0);
            ui.fill(Rect::new(rect.x - 1.0, rect.y - 1.0, rect.w + 2.0, rect.h + 2.0), 0x000000, 1.0);
        }
        let save = &world.save;
        ui.text(rect.x + 4.0, rect.y + 1.0, &save.name, WHITE);
        let details = format!("{} ({})", world.folder_name(), saves::format_date(save.last_played));
        ui.text(rect.x + 4.0, rect.y + 12.0, &details, DARK_GRAY);
        ui.text(rect.x + 4.0, rect.y + 23.0, &format!("Seed: {}", save.seed), DARK_GRAY);
    }
    if state.worlds.is_empty() {
        ui.text_centered(cx, ((top + bottom) / 2.0 - 4.0).floor(), "No worlds yet", GRAY);
    }

    // Buttons.
    let selected = state.selected.filter(|&i| i < state.worlds.len());
    let row1 = ui.height - 52.0;
    let row2 = ui.height - 28.0;
    if ui.button(Rect::new(cx - 154.0, row1, 150.0, 20.0), "Play Selected World", selected.is_some()) {
        action = Action::Play(state.worlds[selected.unwrap()].clone());
    }
    if ui.button(Rect::new(cx + 4.0, row1, 150.0, 20.0), "Create New World", true) {
        action = Action::Open(Screen::CreateWorld {
            name: "New World".into(),
            seed: String::new(),
            mode: GameMode::Survival,
        });
    }
    let delete_label = if state.confirm_delete { "Click again to delete" } else { "Delete" };
    if ui.button(Rect::new(cx - 154.0, row2, 150.0, 20.0), delete_label, selected.is_some()) {
        let i = selected.unwrap();
        if state.confirm_delete {
            if let Err(err) = state.worlds[i].delete() {
                log::error!("{err:#}");
            }
            state.worlds.remove(i);
            state.selected = None;
            state.confirm_delete = false;
        } else {
            state.confirm_delete = true;
        }
    }
    if ui.button(Rect::new(cx + 4.0, row2, 150.0, 20.0), "Cancel", true) || ui.input.pressed(KeyCode::Escape) {
        action = Action::Open(Screen::Title);
    }
    action
}

fn create_world(ui: &mut Ui, name: &mut String, seed: &mut String, mode: &mut GameMode) -> Action {
    let cx = ui.width / 2.0;
    let left = (cx - 100.0).floor();
    dim(ui, 0.6);
    ui.text_centered(cx, 20.0, "Create New World", WHITE);
    ui.text(left, 47.0, "World Name", GRAY);
    ui.text_field(1, Rect::new(left, 60.0, 200.0, 20.0), name, 32);
    ui.text(left, 92.0, "Seed for the World Generator", GRAY);
    ui.text_field(2, Rect::new(left, 105.0, 200.0, 20.0), seed, 32);
    ui.text(left, 130.0, "Leave blank for a random seed", DARK_GRAY);
    // Like vanilla's create world screen: Survival or Creative.
    let label = format!("Game Mode: {}", if *mode == GameMode::Creative { "Creative" } else { "Survival" });
    if ui.button(Rect::new(left, 150.0, 200.0, 20.0), &label, true) {
        *mode = if *mode == GameMode::Creative { GameMode::Survival } else { GameMode::Creative };
    }

    let row = ui.height - 28.0;
    let create = ui.button(Rect::new(cx - 155.0, row, 150.0, 20.0), "Create New World", true)
        || ui.input.pressed(KeyCode::Enter);
    if create {
        ui.clear_focus();
        return match saves::create_world(name, seed, *mode) {
            Ok(entry) => Action::Play(entry),
            Err(err) => {
                log::error!("creating world: {err:#}");
                Action::None
            }
        };
    }
    if ui.button(Rect::new(cx + 5.0, row, 150.0, 20.0), "Cancel", true) || ui.input.pressed(KeyCode::Escape) {
        ui.clear_focus();
        return Action::Open(Screen::SelectWorld(SelectWorld::load()));
    }
    Action::None
}

fn options_screen(ui: &mut Ui, options: &mut Options, from_pause: bool) -> Action {
    let cx = ui.width / 2.0;
    dim(ui, 0.6);
    ui.text_centered(cx, 15.0, "Options", WHITE);
    let y = (ui.height / 6.0).floor();
    let row = |i: f32| Rect::centered(cx, y + i * 24.0, 200.0, 20.0);
    let mut changed = false;

    let mut fov = (options.fov - 30.0) / 80.0;
    let fov_label = match options.fov as i32 {
        70 => "FOV: Normal".to_string(),
        110 => "FOV: Quake Pro".to_string(),
        fov => format!("FOV: {fov}"),
    };
    if ui.slider(10, row(0.0), &fov_label, &mut fov) {
        options.fov = (30.0 + fov * 80.0).round();
        changed = true;
    }

    let mut distance = (options.render_distance - 2) as f32 / 30.0;
    let distance_label = format!("Render Distance: {} chunks", options.render_distance);
    if ui.slider(11, row(1.0), &distance_label, &mut distance) {
        options.render_distance = 2 + (distance * 30.0).round() as i32;
        changed = true;
    }

    let mut sensitivity = options.sensitivity / 2.0;
    let sensitivity_label = format!("Sensitivity: {}%", (options.sensitivity * 100.0).round());
    if ui.slider(12, row(2.0), &sensitivity_label, &mut sensitivity) {
        options.sensitivity = (sensitivity * 2.0 * 100.0).round() / 100.0;
        changed = true;
    }

    let scale_label = match options.gui_scale {
        0 => "GUI Scale: Auto".to_string(),
        n => format!("GUI Scale: {n}"),
    };
    if ui.button(row(3.0), &scale_label, true) {
        options.gui_scale = (options.gui_scale + 1) % 5;
        changed = true;
    }

    if ui.button(Rect::centered(cx, y + 168.0, 200.0, 20.0), "Done", true) || ui.input.pressed(KeyCode::Escape) {
        return Action::CloseOptions { from_pause };
    }
    if changed { Action::ApplyOptions } else { Action::None }
}

fn pause(ui: &mut Ui) -> Action {
    let cx = ui.width / 2.0;
    dim(ui, 0.75);
    let y = (ui.height / 4.0 + 8.0).floor();
    ui.text_centered(cx, y - 24.0, "Game Menu", WHITE);
    if ui.button(Rect::centered(cx, y, 204.0, 20.0), "Back to Game", true) || ui.input.pressed(KeyCode::Escape) {
        return Action::Resume;
    }
    if ui.button(Rect::centered(cx, y + 24.0, 204.0, 20.0), "Options...", true) {
        return Action::Open(Screen::Options { from_pause: true });
    }
    if ui.button(Rect::centered(cx, y + 48.0, 204.0, 20.0), "Save and Quit to Title", true) {
        return Action::SaveAndQuit;
    }
    Action::None
}

fn loading(ui: &mut Ui, progress: f32) -> Action {
    let cx = ui.width / 2.0;
    ui.background(Sprite::MenuBackground, 0.25);
    let y = (ui.height / 2.0).floor();
    ui.text_centered(cx, y - 20.0, "Loading terrain...", WHITE);
    ui.text_centered(cx, y, &format!("{}%", (progress * 100.0) as u32), GRAY);
    Action::None
}
