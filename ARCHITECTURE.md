# Architecture

rscraft is a lightweight client compatible with Minecraft: Java Edition,
written in Rust on top of wgpu. The goal is Minecraft's behaviour with a
better engine: where matching vanilla matters to players (seeds, physics,
data, protocol) we match it exactly; how we get there is our own.

## Crates

Dependencies only point downwards in this list.

| Crate | What it is |
|---|---|
| `rscraft-world` | The game itself, without graphics or input: blocks, 16³ sections, terrain generation, sky light, player movement, inventory, crafting, game modes, dropped items. This is the core a server will share. |
| `rscraft-assets` | Getting Minecraft's own assets: downloads the client jar from Mojang (or reuses an installed one), reads textures and JSON from it. Nothing from Mojang is ever committed to this repository. |
| `rscraft-client` | The game window: wgpu rendering, GUI, menus, input, and the glue between them. |

## Inside the client

- `app.rs` — the winit application: window events, input, which screen is
  open, and putting each frame together.
- `session.rs` — a loaded world and the player in it: the game tick, what
  the player does to the world (breaking, placing, crafting), per-frame view
  effects, and building the frame's 3D scene for the renderer.
- `chunks.rs` — loading, generating, lighting and meshing columns around
  the player on a thread pool.
- `renderer/` — everything wgpu: terrain passes, sky, clouds, entities,
  selection, shadows, GUI, the inventory player preview. Shared pipeline
  boilerplate lives in `gpu.rs`; shaders in `shaders/` (`common.wgsl` is
  prepended to every world shader).
- `ui/` — an immediate-mode GUI drawn with Minecraft's sprites and font.
- `screens.rs`, `inventory_screen.rs`, `switcher.rs`, `hud.rs`, `debug.rs`
  — the screens and overlays built on it.
- `scene.rs` — scripted test scenes (see below).

## Ticks and frames

The world runs at 20 ticks per second, like a Minecraft server:
`Session::update` runs the due ticks, in which the player moves, dropped
items fall and get picked up, and particles play. Rendering happens every
frame and interpolates between the last two ticks with one `partial` value.
Things that are purely visual (view bobbing, FOV effects, hand animation)
update per frame in `Session::update_view`.

Only the pause menu stops the world; other screens such as the inventory
don't, like in singleplayer Minecraft.

## Data from Minecraft

Behaviour comes from Minecraft's data wherever it exists: textures, colour
maps, GUI sprites, font definitions and so on are read from the user's
client jar at startup. Numbers and rules that only exist in Mojang's code
(physics constants, light curves, animation timings) are reimplemented in
our own code, with a comment naming the vanilla class or method they mirror.
Decompiled Minecraft code may be read to understand behaviour, but is never
copied or translated into this repository.

## Testing

- `cargo test` — unit tests for game logic (inventory, crafting, physics
  checked against vanilla's walking and sprinting speeds, raycasting).
- `cargo run -- --scene scenes/forest.json` — plays a scripted scene in a
  throwaway world and saves a screenshot read back from the renderer, then
  quits. Scenes set the player's position and view, time of day, game mode,
  perspective, open screen, hotbar, held keys and block fills; see
  `scene.rs` for every field. Screenshots go to `scenes/out/`.
- CI (GitHub Actions) checks formatting, runs clippy with warnings as errors
  and runs the tests on Linux, Windows and macOS.

## Conventions

- `cargo fmt` with the repository's `rustfmt.toml` (120 columns).
- No clippy warnings.
- Files over ~500 lines are a sign to split them.
- Comments explain why; where behaviour copies Minecraft, they say so.
