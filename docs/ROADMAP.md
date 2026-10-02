# Roadmap

Each milestone should end in a buildable, verified state (`cargo test`,
`csr probe`, `csr sim`, plus a `csr play --screenshot` check for anything
visual).

## M1 — Walk a real map ✅

Format readers, collision, CS:GO movement, 64-tick sim, a renderer with
lightmaps, props and sun, HUD, and the weapon catalog.

## M2 — Look like CS:GO

- [x] 3D skybox (`sky_camera`, drawn by a second camera at 16x)
- [x] 2D skybox cube faces loaded from the original materials
- [ ] Overlays and decals (`info_overlay`)
- [ ] Detail props (grass and bushes from `detail.vbsp`)
- [ ] Static prop vertex lighting from the pakfile's `sp_hdr_*.vhv` (replaces the ambient-cube approximation)
- [x] Bump-mapped directional lightmaps, `$bumpmap`/`$bumpmap2`, SSBump and model normal maps
- [ ] Env-map reflections (`env_cubemap` from the pakfile)
- [ ] Water, `Lightmapped_4WayBlend`, and translucent sorting
- [x] Conservative PVS cluster visibility and spatial batches with frustum culling
- [ ] Prop fade distances and model LOD selection

## M3 — Shoot

- [x] Viewmodels: MDL skeletal animation (bones, sequences, anim blocks, frame anims)
- [x] Weapon state machine from `WeaponDefinition` (fire rate, ammo, reload, deploy)
- [x] CS:GO inaccuracy model and a seeded per-weapon recoil pattern (tuning pending)
- [x] Range falloff, armor ratio, hit groups (hull-height approximation)
- [x] Separate knife slash/stab events, sounds and animations; no firearm effects
- [x] Right-click zoom for all six scoped weapons, original item FOV/speed/accuracy data
- [x] Weapon deployment consumes old attack events instead of replaying them
- [ ] Exact knife sweep volume, impact timing, and model animation-event dispatch
- [ ] Original rifle-optic refraction, lens effects and transition parity
- [ ] Burst modes, suppressor toggling, revolver alternate fire/cocking, weapon inspection
- [ ] Wall penetration (`penetration`, surface props)
- [ ] Hit groups from the character models' hitboxes
- [x] Sounds: soundscripts and WAV/MP3 playback (weapons)
- [ ] Footsteps, impacts, reload sounds

## M4 — Play a round

- [x] Player models (T/CT) with a simplified third-person animation state
- [ ] Ragdoll/death animations, lean/strafe layers, aim yaw, foot IK
- [x] Rounds, freeze time, buy zones and buy menu, money
- [x] Bomb plant/defuse (`func_bomb_target`), win conditions
- [x] Halftime, match results, spectator camera and Tab scoreboard
- [x] Dropped weapons/C4/defuse kits; blocked pickups and defuses respect world collision
- [ ] Grenades: HE, flash, smoke, molotov/incendiary, decoy

## M5 — Play together

- [ ] Server-authoritative netcode over UDP: input commands up, snapshot deltas down
- [ ] Client prediction and reconciliation (reuses `sim::World::step`)
- [ ] Lag compensation for hit registration
- [x] First-pass bots (wander, react, aim, burst fire)
- [x] Bots using the map's `.nav` meshes
- [x] Bot buying, bomb carrying/recovery, planting, guarding and defusing

## M6 — Stitch

See [STITCHING.md](STITCHING.md).

### Movement and visual fidelity pass (2026-10-01)

Implemented shared render-pose interpolation, smooth bot aim error, corrected
spectator viewmodels, displacement grid lightmap UVs, six-face sky rendering,
runtime original Stratum fonts/HUD art, radial buying and an overview radar.
Remaining fidelity work: Panorama layout parity/main menu, rotating radar,
VHV prop lighting, reflections/specular material paths, Source colour correction
and distant sky geometry. See PLAYTEST.md for current checks and limitations.

### Rendering and bot tactics pass (2026-10-01)

World lighting now reads the flat and three directional lightmap layers. The
shader samples normal maps in linear colour space; transition materials blend
both normal maps and SSBump uses its stored directional weights. Static props
retain their own ambient cubes/sun visibility in a compact texture; authored
VVD tangents feed normal shading on models. VHV lighting remains unfinished.

Static meshes are partitioned into spatial batches, rejected conservatively by
the BSP PVS and then by the camera frustum. Player animation clips are decoded
during loading. Bot sightlines gate target updates and firing; remembered
positions expire. Movement follows the path independently of head turning,
uses bounded angular acceleration, persistent stuck recovery and teammate
separation. Defusers get covering teammates; guards choose reachable hiding
spots and visible approaches. Reloading bots can move toward nearby cover.
Camera stair transitions are eased while local mouse look stays immediate.


### Menus and combat feedback pass (2026-10-01)

Implemented original-asset main/pause/settings pages, installed map selection,
Competitive/Deathmatch selection, saved video/audio/control/crosshair settings,
bounded impact/blood effects and original damage sounds. Fixed orphan held
weapons at team changes and resting pivots for ground models. Exact Panorama
layout, complete settings parity, hostage rules and original particle-system
fidelity are still open. See PLAYTEST.md for verification and limitations.

### Tracers, inspection and inventory pass (2026-10-01)

Tracers use animated MDL muzzle attachments, including parent-bone offsets,
and account for the first-person camera's separate FOV. Trails are short,
muted and fading; the simulation's hit rays are unchanged. F plays installed
lookat inspection clips, interrupted by fire/reload/scope/switch actions.

The front end uses a late-CS:GO Panorama arrangement with a narrow navigation
rail, content panels, local profile panel and animated 3D character. Installed
map thumbnails are direct cards with hover lift/tilt/shadow and a persistent
selection border. Inventory provides 36 base weapon cards, filters, rotating
3D previews and saved team-specific primary/pistol choices for Deathmatch.
Steam inventories, skins, online friends/store services, exact Panorama option
parity and the gameplay gaps listed above remain unfinished.
