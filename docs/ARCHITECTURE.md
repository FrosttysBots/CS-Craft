# Architecture

## Data flow

```
CS:GO install (read-only)
  │  gameinfo.txt search paths
  ▼
vfs::Vfs ── map pakfile (zip, LZMA) ─┐
  │        pak01_dir.vpk ─────────────┤ first hit wins
  │        loose csgo/ files ─────────┘
  ▼
level::Level::load(map)
  ├─ bsp::Bsp            geometry, lightmaps, entities, static props
  └─ collision::CollisionWorld   brushes + displacement triangles + prop hulls
        │
        ├──────────────► sim::World (64 Hz)      ◄── TickInput per player
        │                  movement::player_move      (keyboard, bots, network)
        │                  → Snapshot
        ▼
client (Bevy)
  world_mesh   faces + displacements → one mesh per material + lightmap atlas
  props        MDL → meshes, ambient-cube + sun lighting per instance
  material     WorldMaterial + shaders/world.wgsl
  player       input → sim → interpolated camera
  hud          crosshair, speed, debug
```

## Principles

1. **Read, never write, game data.** Everything Valve made is read at runtime
   from the user's install.
2. **Translate at the boundary.** Format crates (`bsp`, `vtf`, `mdl`, ...)
   decode files into plain Rust data in Source units and axes. Only the client
   converts to Bevy space (`client::coords`: `(x, y, z) → (x, z, -y)`).
3. **One simulation path.** `sim::World::step` is deterministic. The server,
   client prediction, replays and headless tests all run the same code.
   Gameplay never runs in render-frame systems.
4. **Game-agnostic identity.** Content gets `ContentId`s (`csgo_weapon_ak47`)
   and normalised definitions (`weapons::WeaponDefinition`). Other games'
   importers produce the same types.
5. **Verify on real data.** `csr probe` and `csr sim` run against the real
   install. Unit tests use synthetic data, so they pass without the game.

## Coordinate systems

| | Source | Bevy |
|---|---|---|
| Up | +Z | +Y |
| Forward (yaw 0) | +X | +X |
| Left | +Y | −Z |
| Units | inches | inches (unchanged) |

## Collision

`CollisionWorld` holds convex solids (plane sets) and triangles in a uniform
128-unit XY grid. A box sweep expands each plane by the box's support
distance and runs the classic enter/leave-fraction clip. Triangles get a
1-unit back thickness and the full separating-axis plane set (face ±normal,
box axes, edge × axis), which makes box-vs-triangle sweeps exact. Prop hulls
get face, axial and edge bevels in the same way.

## Rendering

- World faces are fan-triangulated and wound counter-clockwise around their
  normal. Displacements are tessellated in `bsp::disp` (checkerboard
  diagonals) with smooth normals.
- Lightmaps come from the HDR lump when present. Style-0 luxels go into an
  `Rgba16Float` atlas with 1-luxel clamped borders.
- DXT textures are uploaded to the GPU as BC1/2/3 with every mip level.
- Props use the leaf ambient cube nearest their centre, plus
  `light_environment` sunlight gated by one shadow ray per prop.
- Tonemapping is ACES in the shader, and the camera's own tonemapping is off.
