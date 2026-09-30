# Roadmap

Each milestone should end in a buildable, verified state (`cargo test`,
`csr probe`, `csr sim`, plus a `csr play --screenshot` check for anything
visual).

## M1 — Walk a real map ✅

Format readers, collision, CS:GO movement, 64-tick sim, a renderer with
lightmaps, props and sun, HUD, and the weapon catalog.

## M2 — Look like CS:GO

- [ ] Skybox (2D `skyname` cubemap) and the 3D skybox (`sky_camera`, 1/16 scale)
- [ ] Overlays and decals (`info_overlay`)
- [ ] Detail props (grass and bushes from `detail.vbsp`)
- [ ] Static prop vertex lighting from the pakfile's `sp_hdr_*.vhv` (replaces the ambient-cube approximation)
- [ ] Bump-mapped lightmaps (the three directional lightmaps) and `$bumpmap`
- [ ] Env-map reflections (`env_cubemap` from the pakfile)
- [ ] Water, `Lightmapped_4WayBlend`, and translucent sorting
- [ ] Visibility (PVS clusters) and prop fade distances

## M3 — Shoot

- [ ] Viewmodels: MDL skeletal animation (bones, sequences, anim blocks)
- [ ] Weapon state machine from `WeaponDefinition` (fire rate, reload, deploy)
- [ ] CS:GO spread, inaccuracy and seeded recoil patterns
- [ ] Bullet traces with penetration (`penetration`, surface props) and range falloff
- [ ] Damage, armor ratio, hitgroups from the player model's hitboxes
- [ ] Sounds: `game_sounds_*.txt` soundscripts and WAV/MP3 playback

## M4 — Play a round

- [ ] Player models (T/CT) with the third-person animation state
- [ ] Rounds, freeze time, buy zones and buy menu, money
- [ ] Bomb plant/defuse (`func_bomb_target`), win conditions
- [ ] Grenades: HE, flash, smoke, molotov/incendiary, decoy

## M5 — Play together

- [ ] Server-authoritative netcode over UDP: input commands up, snapshot deltas down
- [ ] Client prediction and reconciliation (reuses `sim::World::step`)
- [ ] Lag compensation for hit registration
- [ ] Bots using the map's `.nav` meshes

## M6 — Stitch

See [STITCHING.md](STITCHING.md).
