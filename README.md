# rscraft

A lightweight client compatible with Minecraft: Java Edition, written in
Rust on top of [wgpu](https://wgpu.rs). The aim is not a copy but a better
engine for the same game: where it matters to players — seeds, physics,
game data, the protocol — rscraft behaves like vanilla Minecraft; how it
gets there is its own, and it should run well on modest hardware.

It is early work: a singleplayer world you can walk, build and swim in,
with a growing part of the game behind it.

> rscraft is not an official Minecraft product and is not affiliated with
> or endorsed by Mojang or Microsoft. It contains no Minecraft assets or
> code; see [Minecraft assets](#minecraft-assets).

## What works

- Terrain generation with trees, caves, lakes and oceans (not yet seed
  compatible with vanilla), loading around the player on all CPU cores.
- Rendering with Minecraft's own textures: smooth lighting and ambient
  occlusion, day and night, sky and fancy clouds, fog, water with
  underwater fog, particles, entity shadows. Render distance up to 48
  chunks.
- Vanilla movement physics: walking, sprinting, jumping, swimming, flying,
  sneaking, with view bobbing.
- Breaking and placing blocks, dropped items, a hotbar and inventory with
  2×2 crafting, game modes (survival, creative, adventure, spectator).
- Flowing water and grass spreading, following vanilla's rules.
- The first-person hand and the player model animated like vanilla.
- Saved worlds, options (FOV, render distance, frame rate limit, VSync,
  GUI scale), a debug screen.

## Minecraft assets

Textures, fonts and other assets come from Minecraft itself and are never
part of this repository. On first start rscraft downloads the Minecraft
client jar from Mojang's servers (or reuses one from an installed
launcher) and reads the assets from it. Please own Minecraft: Java Edition
to play.

## Building

You need a recent stable [Rust](https://rustup.rs) (edition 2024) and a GPU
with Vulkan, Metal or Direct3D 12.

```sh
cargo run --release
```

Worlds, options and the downloaded jar are kept in the platform's data
directory (`~/.local/share/rscraft` on Linux).

## Controls

| Key | Action |
|---|---|
| W A S D | Move |
| Space | Jump; swim up; double-tap to fly in creative |
| Shift | Sneak; fly down |
| Ctrl | Sprint |
| Left / right mouse | Break / place blocks |
| 1–9, mouse wheel | Select hotbar slot |
| E | Inventory |
| Q | Drop item (Ctrl+Q: whole stack) |
| C | Zoom (mouse wheel changes it) |
| F3 | Debug screen |
| F3 + F4 | Switch game mode |
| F5 | Change perspective |
| Esc | Pause |

## Development

See [ARCHITECTURE.md](ARCHITECTURE.md) for how the code is laid out. In
short:

- `cargo test` runs the game logic tests.
- `cargo run --release -- --scene scenes/forest.json` plays a scripted
  scene in a throwaway world and saves a screenshot to `scenes/out/`;
  scenes with `"benchmark": true` report frame times.
- CI checks formatting (`cargo fmt`), runs clippy with warnings as errors
  and the tests on Linux, Windows and macOS.

## License

rscraft is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free
Software Foundation, either version 3 of the License, or (at your option)
any later version. See [LICENSE](LICENSE).
