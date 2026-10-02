Minecraft engine integration
============================
Source: https://github.com/chasmlol/2010-rust-rewrite-mashup
Pinned revision: ab43b8a9e923bdc4ad7effb874f6efec155577f1
The minecraft_terrain crate and five minecraftoss crates are taken from that
revision. See MASHUP-LICENSE and MASHUP-NOTICE, and minecraftoss/README.md.
Local modifications: relocated minecraft_terrain dependency paths and exposed
TerrainStream::save_all so the host can flush chunk edits when leaving a map.
ServerHandle now remembers consumed acknowledgements so wait_idle cannot hang
after poll drained the final response.
rust-audio contains unmodified lewton 0.10.2 and ogg 0.8.0 crates from crates.io,
with their original licenses, for Minecraft Ogg/Vorbis sounds.
Game assets are read from the user's external installation, never vendored.

Survival integration changes (CSGO host):
- Blockstate X rotations and cull directions are applied after local element
  rotation and before Y rotation; End portals get a pack-backed surface model.
- The atlas includes blaze, dragon, crystal, fireball and Eye textures; the host
  can use the shared cuboid renderer for its custom adventure entities.
- Exposes seeded End spike positions so host crystals reuse generated pillars.
- Inventory, item stacks, furnaces, chests and FoodData support serde persistence.
  RecipeBook is skipped and rebound to external assets when loading a save.
- Mining accepts host registry hardness and tool-tag entries. The small fallback
  table also covers the progression ores and obsidian for authored test scenes.
The dragon encounter, portal travel and custom blaze behavior live in
crates/overworld; they are host adaptations, not claims of upstream parity.

Presentation/economy integration additions:
- Exposes mining targets and the same selection boxes used by targeting.
- ItemVisuals supports inherited first-person item display transforms.
- Death records carry player-attributed mob types through the server output for
  host economy rewards; consuming deaths prevents duplicate kill payments.
- Includes the installed dragon eye texture in the shared atlas.
- Exposes the existing entity-type lookup so the host can retain a fatal mob
  hit's attacker name across snapshot replacement and entity despawning.
- Lighting worker results include changed section indices, compared against the
  request's captured light snapshot; epoch validation still discards stale work.

The Minecraft terrain host adapter also forwards accepted/rejected mob attack
results in command order. This connects blood feedback to authoritative damage
rather than clicks; native damage timers and invulnerability rules are retained.

Stutter reduction (host changes):
- ChunkMap has an active worker limit (`set_active_workers`): workers above it
  wait instead of taking jobs. TerrainStream exposes it as
  `set_generation_threads`. The host pre-generates the view with every
  thread, then plays with two, because bursts of generation at each chunk
  border competed with the render thread.
