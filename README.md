# CSGO Rewrite

A from-scratch **Rust** rewrite of Counter-Strike: Global Offensive's runtime,
built on [Bevy](https://bevyengine.org/) 0.19. Point it at a CS:GO install you
already have and it loads that install's maps, textures, models and weapon data
into its own engine. It uses no Valve code.

**Written by Mr. Claude** (Anthropic's Claude), for Frostty.

It follows the approach of [IW4L](https://github.com/vladtrc/iw4L) and
[2010-rust-rewrite-mashup](https://github.com/chasmlol/2010-rust-rewrite-mashup)
(MW2 rewritten in Rust). It shares their Bevy version and content-ID scheme so
the games can later be **stitched together** into one runtime. See
[docs/STITCHING.md](docs/STITCHING.md).

## Status

**Overworld is available in the map selector.** `Play Overworld.cmd` opens the
Minecraft Special mode with CS:GO movement, weapons and HUD. It is solo, with
unlimited time and no CS bots. It uses the installed Minecraft assets for streamed
terrain, creatures, Survival inventory, 2×2/3×3 crafting and furnaces. The
completion route includes Nether portals, fortress blazes, real strongholds,
Eyes of Ender and a custom Ender Dragon encounter. **N** toggles tools/weapons;
**I** opens inventory. New seeds start with an empty Minecraft inventory. See
[docs/OVERWORLD.md](docs/OVERWORLD.md) for controls, saves and parity limits.

You can load de_dust2 and play a local 5v5 competitive match against bots,
with freeze time, buying, money, bomb planting/defusing, halftime and match
results. Respawn deathmatch is also available. This is a playable prototype;
grenades, wall penetration and multiplayer networking are still unfinished.

| Area | State |
|---|---|
| VPK archives, loose files, map pakfiles | ✅ read, including LZMA entries |
| BSP v21 (CS:GO), with LZMA-compressed lumps | ✅ faces, displacements, brushes, entities, HDR lightmaps, ambient light, static props |
| VTF textures (DXT1/3/5 + uncompressed) | ✅ DXT is uploaded compressed with its full mip chain |
| VMT materials with `patch` includes | ✅ |
| MDL / VVD / VTX models, PHY collision hulls | ✅ incl. skeletons, hitboxes, sequences, body groups |
| Animations | ✅ classic RLE tracks and CS:GO's newer frame-animation format (incl. its compressed quaternions), sections, `.ani` blocks, include models, weight lists |
| Collision | ✅ swept-box traces against brushes, displacements and prop hulls |
| Player movement | ✅ clean-room CS:GO movement: friction, accel, air-strafe, stairs, crouch-jump, stamina, bhop cap |
| Simulation | ✅ deterministic fixed 64-tick `TickInput → step → Snapshot` |
| Renderer | ✅ world plus lightmaps, WorldVertexTransition blends, batched props lit by ambient cube + sun, 3D skybox (≈120 fps on an RTX 3070 laptop GPU) |
| Weapon catalog | ✅ `items_game.txt` → typed `WeaponDefinition`s (prefab inheritance) |
| Shooting | ✅ fire rate, ammo, reloads, CS:GO inaccuracy model, seeded recoil pattern, range falloff, armor, hit groups (damage matches CS:GO: AK 111 helmet HS, 27 armored body) |
| Sound | ✅ soundscripts + weapon sounds from the VPK, spatial |
| Characters | ✅ GPU-skinned T/CT models animated from CS:GO's own animation set: 9-way run/walk/crouch blends, aim poses by pitch, jump/fall, reload and fire gestures, holding their real weapon models |
| First-person weapons | ✅ viewmodels with team arms (bone-merged), draw/idle/fire/reload/inspect animations from the game, CS:GO viewmodel FOV and offsets, sway, own camera so guns never clip walls |
| Bots | ✅ nav-mesh path-finding to callouts (Long A, Mid, bombsites...) |
| Competitive rounds | ✅ freeze time, buy zones/menu, economy, halftime, match results; rules read from the install |
| Bomb objectives | ✅ plant, interrupt, drop/pick up, defuse with/without kit, explosion and win conditions; objective-aware bots |
| Main menu / settings | ✅ 2023 Panorama-style layout with a 3D character preview and installed fonts/art; direct map cards, mode/team selection, saved settings |
| Inventory | ✅ local base-item collection, rotatable 3D previews and saved T/CT Deathmatch loadouts |
| Hit feedback | ✅ surface impacts, blood masks/splats, original damage sounds, directional hurt feedback; optional hit marker |
| Match UI | ✅ round/score HUD, plant/defuse progress, kill feed, spectating and Tab scoreboard |
| Grenades, wall penetration, netcode | ⏳ see [docs/ROADMAP.md](docs/ROADMAP.md) |

## Download and play (Windows)

1. Switch Counter-Strike 2 to CS:GO Legacy in Steam: right-click Counter-Strike 2
   → Properties → Betas → `csgo_legacy`, and let it download.
2. Download `CSGO-Rewrite-<version>-windows-x64.zip` from this repository's
   **Releases** page and unzip it anywhere.
3. Run **Play CSGO.bat** (or **Play Minecraft.bat**). The first launch searches
   every Steam library on the PC for CS:GO Legacy and asks you to confirm the
   folder it found. If it finds none, you select `csgo.exe` yourself. The
   choice is remembered; **Change CSGO Folder.bat** asks again.

Every release starts as a fresh install: no settings, skins, cases or
Minecraft worlds. Your progress is saved in `userdata\` next to the game.
The zip bundles the Minecraft 26.3 assets the Overworld mode uses. CS:GO
files are never included; they are read from your own install. See
[docs/PLAYERS.md](docs/PLAYERS.md) (shipped as `README.txt`).

To build a release yourself, run **Package Release.cmd**. It writes
`dist\CSGO-Rewrite-<version>-windows-x64.zip`.

## Building from source: what you need

- **CS:GO (legacy)**. In Steam: right-click Counter-Strike 2 → Properties →
  Betas → choose `csgo_legacy`. The install is found automatically. Otherwise set
  `CSR_GAME_DIR` to the folder that holds `gameinfo.txt`.
- Rust (stable, 2024 edition).

Game files are only ever **read**. Nothing from CS:GO is stored in this
repository.

## Run it

On Windows, double-click **Play CSGO.cmd**. It builds the current source and
opens the main menu. Choose **PLAY**, click a map card, pick mode and team, then **GO**.
Cards lift and tilt on hover; the selected map keeps a green border.
**INVENTORY** shows 36 installed base weapons with rotatable 3D previews.
Equip a primary/pistol for T or CT; `config/inventory.cfg` saves the choice
for your next Deathmatch spawn. Competitive retains its buy economy.
**CASES** (also inside Inventory) opens the local cosmetic collection.
Click **CONSOLE**, type **give_case Revolution 5**, and press Enter to add five
Revolution Cases. Select the case, then **OPEN CASE**; no key is needed.
The reel slows to the saved reward, which you can equip immediately.
**MY SKINS** lists owned finishes with wear, seed and equipped state.
Skins are cosmetic in both game modes and remain attached to dropped weapons.
The local collection supplies 39 cases, 120 souvenir packages and 1,732 finishes,
including rare knives/gloves, persistent StatTrak counters and applied stickers.
Use **skins <search>** and **stickers <search>** to find IDs, then **give_skin**
or **give_sticker** to grant specific items. Existing collections migrate on save.
See [docs/COSMETICS.md](docs/COSMETICS.md) for commands and fidelity limits.
Settings include video, audio, keyboard/mouse and crosshair customization.
**APPLY / SAVE** persists settings in `config/user.cfg` (ignored by Git).

From a terminal in this folder:

```bash
cargo run --profile play -p launcher -- play de_dust2
cargo run --profile play -p launcher -- play de_dust2 --direct --team t
cargo run --profile play -p launcher -- play de_dust2 --direct --deathmatch
```

Competitive starts with a 15-second freeze time. Buy with **B**, then move
when the countdown ends. As a T carrying the bomb, enter a bomb site,
select **5** and hold left mouse to plant. As a CT, look at the planted
bomb and hold **E** nearby to defuse. The match keeps running while the
buy menu is open. **Esc** opens the pause menu and pauses the local match.
Use **Resume** or **Esc** to continue; the menu click cannot fire your weapon.

| Key | Action |
|---|---|
| Mouse | Look (click the window to capture) |
| Esc | Pause / resume local match |
| WASD | Move |
| Space | Jump |
| Ctrl | Crouch |
| Shift | Walk |
| Left mouse | Fire |
| Right mouse | Scope / cycle zoom (scoped weapons), knife stab |
| R | Reload |
| F | Inspect held weapon (rebindable in Controls) |
| 1 / 2 / 3 / 5, wheel | Primary / pistol / knife / C4 |
| Q | Last weapon |
| G | Drop held weapon |
| B | Buy menu (number keys or mouse; 0 goes back) |
| E (hold) | Defuse while looking at a nearby bomb |
| Tab (hold) | Scoreboard; also shown at match end |
| Left mouse while dead | Cycle spectated players |
| V | Toggle noclip |
| F3 | Toggle FPS/position diagnostics (hidden by default) |
| Alt+F4 | Close game |

Other commands:

```bash
cargo run --release -p launcher -- probe de_dust2   # load every asset the map uses, report failures
cargo run --release -p launcher -- sim de_dust2     # headless movement test on the real map collision
cargo test --workspace                              # unit tests
```

`csr play` also takes `--bots n`, `--screenshot out.png`,
`--camera x y z pitch yaw`, `--exposure f` and `--size w h`.
`csr weapons` prints the weapon catalog read from your install.
`csr --help` lists the available commands and options.

See [docs/PLAYTEST.md](docs/PLAYTEST.md) for a short playtest and current limits.

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
