# Roadmap

Each milestone should end in a buildable, verified state (`cargo test`,
`csr probe`, `csr sim`, plus a `csr play --screenshot` check for anything
visual).

## M1 — Walk a real map ✅

Format readers, collision, CS:GO movement, 64-tick sim, a renderer with
lightmaps, props and sun, HUD, and the weapon catalog.

## M2 — Look like CS:GO

- [x] 3D skybox (`sky_camera`, drawn by a second camera at 16x)
- [ ] 2D skybox cube faces (currently the sky's average colour)
- [ ] Overlays and decals (`info_overlay`)
- [ ] Detail props (grass and bushes from `detail.vbsp`)
- [ ] Static prop vertex lighting from the pakfile's `sp_hdr_*.vhv` (replaces the ambient-cube approximation)
- [ ] Bump-mapped lightmaps (the three directional lightmaps) and `$bumpmap`
- [ ] Env-map reflections (`env_cubemap` from the pakfile)
- [ ] Water, `Lightmapped_4WayBlend`, and translucent sorting
- [ ] Visibility (PVS clusters) and prop fade distances

## M3 — Shoot

- [x] Viewmodels: MDL skeletal animation (bones, sequences, anim blocks, frame anims)
- [x] Weapon state machine from `WeaponDefinition` (fire rate, ammo, reload, deploy)
- [x] CS:GO inaccuracy model and a seeded per-weapon recoil pattern (tuning pending)
- [x] Range falloff, armor ratio, hit groups (hull-height approximation)
- [ ] Wall penetration (`penetration`, surface props)
- [ ] Hit groups from the character models' hitboxes
- [x] Sounds: soundscripts and WAV/MP3 playback (weapons)
- [ ] Footsteps, impacts, reload sounds

## M4 — Play a round

- [x] Player models (T/CT) with a simplified third-person animation state
- [ ] Ragdoll/death animations, lean/strafe layers, aim yaw, foot IK
- [ ] Rounds, freeze time, buy zones and buy menu, money
- [ ] Bomb plant/defuse (`func_bomb_target`), win conditions
- [ ] Grenades: HE, flash, smoke, molotov/incendiary, decoy

## M5 — Play together

- [ ] Server-authoritative netcode over UDP: input commands up, snapshot deltas down
- [ ] Client prediction and reconciliation (reuses `sim::World::step`)
- [ ] Lag compensation for hit registration
- [x] First-pass bots (wander, react, aim, burst fire)
- [ ] Bots using the map's `.nav` meshes

## M6 — Stitch

See [STITCHING.md](STITCHING.md).
