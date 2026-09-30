# CSGO Rewrite

A from-scratch **Rust** rewrite of Counter-Strike: Global Offensive's runtime,
built on [Bevy](https://bevyengine.org/) 0.19. Point it at a CS:GO install you
already have and it loads that install's maps, textures, models and weapon data
into its own engine. It uses no Valve code.

**Written by Mr. Claude** (Anthropic's Claude), for hawk.

It follows the approach of [IW4L](https://github.com/vladtrc/iw4L) and
[2010-rust-rewrite-mashup](https://github.com/chasmlol/2010-rust-rewrite-mashup)
(MW2 rewritten in Rust). It shares their Bevy version and content-ID scheme so
the games can later be **stitched together** into one runtime. See
[docs/STITCHING.md](docs/STITCHING.md).

## Status

Milestone 1 (load and walk a real map) works:

| Area | State |
|---|---|
| VPK archives, loose files, map pakfiles | ✅ read, including LZMA entries |
| BSP v21 (CS:GO), with LZMA-compressed lumps | ✅ faces, displacements, brushes, entities, HDR lightmaps, ambient light, static props |
| VTF textures (DXT1/3/5 + uncompressed) | ✅ DXT is uploaded compressed with its full mip chain |
| VMT materials with `patch` includes | ✅ |
| MDL / VVD / VTX models, PHY collision hulls | ✅ bind pose (no animation yet) |
| Collision | ✅ swept-box traces against brushes, displacements and prop hulls |
| Player movement | ✅ clean-room CS:GO movement: friction, accel, air-strafe, stairs, crouch-jump, stamina, bhop cap |
| Simulation | ✅ deterministic fixed 64-tick `TickInput → step → Snapshot` |
| Renderer | ✅ world plus lightmaps, WorldVertexTransition blends, props lit by ambient cube + sun |
| Weapon catalog | ✅ `items_game.txt` → typed `WeaponDefinition`s (prefab inheritance) |
| Shooting, rounds, economy, bots, netcode, sound, viewmodels | ⏳ see [docs/ROADMAP.md](docs/ROADMAP.md) |

## What you need

- **CS:GO (legacy)**. In Steam: right-click Counter-Strike 2 → Properties →
  Betas → choose `csgo_legacy`. The install is found automatically. Otherwise set
  `CSR_GAME_DIR` to the folder that holds `gameinfo.txt`.
- Rust (stable, 2024 edition).

Game files are only ever **read**. Nothing from CS:GO is stored in this
repository.

## Run it

```bash
cargo run --profile play -p launcher -- play de_dust2
```

| Key | Action |
|---|---|
| Mouse | Look (click the window to capture, Esc to release) |
| WASD | Move |
| Space | Jump |
| Ctrl | Crouch |
| Shift | Walk |
| V | Toggle noclip |

Other commands:

```bash
cargo run --release -p launcher -- probe de_dust2   # load every asset the map uses, report failures
cargo run --release -p launcher -- sim de_dust2     # headless movement test on the real map collision
cargo test --workspace                              # unit tests
```

`csr play` also takes `--screenshot out.png`, `--camera x y z pitch yaw`,
`--exposure f` and `--size w h` for automated visual checks.

## Layout

```
crates/
  kv         KeyValues (VDF) parser
  vpk        VPK archive reader
  vfs        search-path file system (pakfile → VPK → loose)
  bsp        BSP map reader (+ displacements, static props, ambient light)
  vtf        texture decoder
  vmt        material reader (patch resolution)
  mdl        studio model + physics hull reader
  collision  swept-box / ray traces
  movement   CS:GO player movement
  sim        fixed-tick game simulation
  level      map loading shared by sim and client
  content    GameOrigin + ContentId (cross-game identity)
  weapons    items_game.txt → WeaponDefinition catalog
  client     Bevy renderer, input, HUD (CsgoPlugin)
  launcher   the `csr` binary
```

More detail in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Legal

This is a clean-room reimplementation. The file formats come from public
community documentation (Valve Developer Community wiki and similar), and the
gameplay behaviour from documented cvars and observation. It uses **no leaked
source code and no Valve SDK code.** Counter-Strike and CS:GO are trademarks
of Valve Corporation. This project is not affiliated with or endorsed by Valve,
and you need your own copy of the game.

Licensed under Apache-2.0.
