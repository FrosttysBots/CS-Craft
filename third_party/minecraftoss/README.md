# MinecraftOSS engine crates

Five crates of MinecraftOSS, a Rust engine that aims to match Minecraft Java
Edition 26.3 exactly, down to its world generation from a seed. The Minecraft
map is built on them:

| Crate | What it does |
| --- | --- |
| `core` | Positions, block-state and biome registries, tags, chunk storage |
| `generator` | World generation matching vanilla 26.3 |
| `world` | The server-side chunk map: player views and parallel generation |
| `player` | The player: inventory, items, recipes, loot, block placement |
| `entities` | Mobs and their AI |

They're copied from MinecraftOSS commit `4013a68` with only their sources, and
without the tests, benchmarks and examples that need the full MinecraftOSS
checkout. They're built as part of this workspace through relative paths.

No Minecraft code, JARs or assets are included. The game downloads Minecraft's
data files from Mojang on first run (see `crates/assets/src/minecraft_setup.rs`).
