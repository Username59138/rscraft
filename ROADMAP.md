# Roadmap

## Next, small

- Dropped items float up in water like vanilla (`ItemEntity`), instead of
  sinking.
- Occlusion culling of sections hidden behind terrain (the next big
  rendering win; Sodium does it).
- Creative inventory with every block.

## 1. Blocks from Minecraft's data

Replace the hand-written `Block` enum (12 blocks) with a registry built from
vanilla data: every block and state, models with rotations, uvlock and
multipart, non-cube shapes (slabs, stairs, fences, crosses), cutout layers,
collision shapes, hardness, light emission. Sections store palette indices;
saves store blocks by name.

- Open question, decide first: the jar only has models, blockstates and
  tags. The block list with properties, collision shapes, hardness and so
  on live in vanilla's Java code, so they need an extractor: a small Java
  tool run against the jar that dumps them to JSON (or vanilla's
  `--reports` data generator plus our own additions).
- Rough size: 5–6k lines, several sessions. Design before coding, then one
  step per session (registry and states, model loading, meshing of
  non-cube models, migrating the code, saves).
- Makes the following easier: items that aren't blocks (sticks, tools),
  the 3×3 crafting table, block light (torches), the network protocol.

## 2. Seed-exact world generation

Vanilla's terrain for any seed: bit-exact noises and Java random
generators, the density function graph from the jar's worldgen JSON,
aquifers, surface rules, multi-noise biomes. Terrain, biomes and surface
are roughly 4–6k lines; features (trees, ores) and structures are a much
larger stage after that. Agreed: design carefully before starting; after
step 1.

## 3. Our own server

One client with two kinds of server behind one interface:

```rust
trait ServerLink {
    fn send(&mut self, action: Action); // broke a block, placed one, moved
    fn poll(&mut self) -> Vec<Event>;   // a block changed, an entity moved
}
```

- `LocalServer` (singleplayer): the server runs inside the client and works
  on the same world in memory; actions and events are plain Rust values, no
  packets, no second copy of the world, no tick of delay.
- `RemoteServer`: the same actions and events as protocol packets, with
  client-side prediction of the player's own movement.
- The same executable runs headless as a dedicated server (`--server`); it
  must not touch winit or wgpu then.

Steps: after step 1, move world changes in `session.rs` behind
`Action`/`Event` while still in one process; then a `rscraft-server` crate
running ticks, players and saves; then the protocol.

## Also wanted

- Sticks, tools and the crafting table (3×3).
- Block light: torches, glowing blocks; grass then spreads by torchlight.
- Mobs (their models are Java code, the hardest part of the data).
