# Overworld Survival

Start `Play Overworld.cmd`, select **Overworld**, and start. Choose a new seed for
a fresh Survival world or select an existing seed to resume. This Special mode
has one CS player, unlimited time, no rounds, and no CS bots. Minecraft creatures
remain active. There is no Creative mode or noclip in Overworld.

A block is **39.6 Source units**: 55% of the 72-unit standing CS hull. A two-block
opening is 79.2 units high and a one-block-wide opening fits the player's hull.

## Controls

- **N**: toggle Minecraft tools and CS weapons. A fresh session starts with tools.
- **I**: inventory, armor/offhand slots, and 2×2 crafting. **I / Esc** closes it.
- **Left mouse**: hold to mine with the selected tool, or fire in CS weapon mode.
- **Right mouse**: use a targeted container, crafting table, door or other
  interactive block in either hand mode. In Minecraft mode, place a block, use a bucket or
  flint and steel, throw an Eye of Ender, or hold to eat. Shift bypasses opening
  a container when placing beside it.
- **E** (or the configured CS Use key): alternate block interaction.
- **O**: separate Minecraft Options, using the installed Minecraft buttons/font.
  Adjust mob/block sound levels, minimap, view bobbing and menu blur; Done, O or
  Esc saves and returns to play.
- **Esc**: CS pause/settings/inventory/cases over a transparent, GPU-blurred world.
  This is still a paused solo world, so creatures do not attack during settings.
  RMB still scopes/stabs when not using an interactive block.
- **1–9 / mouse wheel**: Minecraft hotbar selection in tool mode.
- **Ladders / vines**: push toward the supporting wall or hold Jump to climb;
  Crouch or Shift holds position. Includes cave, twisting and weeping vines.
- **Scaffolding**: Jump climbs up; Crouch or Shift descends through the platforms.
- **Crouch / Shift on ground**: stops at block edges, including diagonals. Release
  or jump to leave the ledge.
- **Q / Ctrl-Q**: drop one item / the entire selected stack.
- Inventory: left click picks up/places a stack, right click splits/places one,
  shift-click transfers items. Click the result slot to craft; shift-click crafts
  as many as fit. Closing a screen returns crafting ingredients to your inventory.
- Normal CS movement, jump, crouch, weapon switching, scope, reload and inspect
  remain available. In water, jump swims up and crouch dives.

## Survival progression

New worlds begin with an empty Minecraft inventory. Gather wood, craft planks,
use the 2×2 grid for a crafting table and sticks, then place the table and use it
for **3×3 crafting**. The installed game's recipe and item catalogs determine
recipes, stack limits and tool durability. Mining speed and harvest eligibility
come from block hardness and tool tags. Guns damage creatures; they do not mine
valuable resources through the tool progression gates.

Place a furnace and use it to open input, fuel and output slots. Cooking takes
simulation time and consumes fuel. Furnace inventories, fuel and partial cooking
progress are saved. Loaded furnaces continue cooking with their menu closed.
Chest and barrel contents also persist.

Hunger, eating, natural regeneration, air, drowning, lava and fall damage are
active. Death drops Minecraft inventory stacks with their components and returns
the player to the Overworld spawn. Beds set a respawn point; using one at night
advances the clock to morning. CS weapon respawns retain Special mode behavior.

Build an obsidian portal with a 2×3 or larger empty interior (up to 21×21), then
use flint and steel inside it. Stand in the portal for four seconds. Nether
travel preserves inventory and saves the outgoing dimension. Initial travel
uses the 8:1 horizontal scale; subsequent travel returns to the stored arrival
location and creates a safe return portal.

In the Nether, blazes appear near fortress nether-brick surfaces. They attack
with projectiles and drop blaze rods when killed. Craft blaze powder and combine
it with ender pearls to make Eyes of Ender. Using an Eye in the Overworld points
toward a real generated stronghold and reports its direction/coordinates.
Explore the stronghold, insert Eyes into all twelve correctly oriented portal
frames, and enter the activated portal.

The End has obsidian pillars and ten destructible healing crystals. Destroy the
crystals, then kill the dragon with CS weapons. The encounter includes circling,
perching, projectiles, lingering breath areas, a health bar and a death sequence.
Defeating it activates the central exit portal and places the dragon egg. Enter
the exit portal to record completion; **Enter** on the victory screen returns to
the Overworld. Completion and boss state persist across restarts.

## Rendering and saves

The block loader applies both blockstate X and Y rotations. This repairs giant
mushroom caps and other rotated models. Backface culling removes overlapping
leaf backs, and atlas mipmaps plus opaque cutout output reduce motion shimmer
and sky-colored fringes. Centroid UV interpolation keeps antialiased block edges from sampling neighboring atlas sprites. GUI panels and item icons use the external Minecraft
assets; the inventory preview displays the animated CS character.

Animated textures use small GPU tile updates, including each mip level, instead
of re-uploading the entire atlas. Unchanged animation frames are skipped.

Loading avoids stutter in three steps. Before the window opens,
`Runtime::pregenerate` generates, lights and meshes everything within the
render distance, using every core and sweeping the culling camera through all
directions ("Preparing world" in the console). The client then shows a
"Loading terrain" screen while the pre-built sections and atlas upload and
while every material's pipeline compiles. Behind it, one small mesh per voxel
and standard material is drawn, so pipelines that appear only later (block
cracks, water, particles) are compiled up front. The screen lifts once the
render world reports no pipeline waiting. During play only two threads
generate chunks (`MINECRAFTOSS_PLAY_GEN_THREADS`), because bursts at each
chunk border otherwise starved the render thread. `CSR_NO_PREGEN` skips
pre-generation, for comparisons.

Assets stay in the installed Minecraft 26.3 and CSGO Legacy directories.
`CSR_MINECRAFT_DIR` and `CSR_GAME_DIR` override their locations. No game assets
are copied into the repository.

Saves are under `worlds/Overworld-<seed>` in the player data folder (the
repository root for source builds, `userdata\` beside the executable in a
release). Each dimension has its own chunk
storage. Player state, inventory, containers, day clock and adventure progress
save every 30 seconds and on normal exit/travel. Existing version-1 worlds are
backed up to `world-v1-backup.json` before migration; their previous building
sandbox supplies are discarded during migration (the backup retains them).
Existing Survival inventories are preserved. New runs start with empty Minecraft
slots, the team's CS pistol and knife, and $0; no free rifle or armor. Press B to buy
anywhere, at any time while alive. Player-attributed passive kills pay $25;
hostile kills pay $100, capped at $16,000. Wallet, purchased weapons, ammunition
and CS armor save with the world. Death returns the CS loadout to the pistol and knife;
money survives death. Practice collection loadouts cannot grant Survival guns.
`CSR_WORLD_DIR` overrides the save location; `CSR_RENDER_DISTANCE` selects 3–16
chunks. Closing the process forcibly can lose changes since the last save.

## Current parity limits

This implements a completion route inside the Rust runtime. It is still a
custom adaptation, with simplified dragon AI, blaze spawning, portal
linking and effects, and CS-scaled combat. It does not reproduce every vanilla
mechanic. Full enchantment/brewing/advancement interfaces, every mob/item use,
multiplayer, dedicated held renderers for special items such as shields and
shulker boxes, and vanilla-exact dragon behavior remain unfinished. The End
encounter does not imply a complete 1:1 Minecraft rewrite.

## Verification

Run workspace tests and the existing Dust II probe/simulation and audio checks.
Additional external-asset examples:

- `cargo run --offline -p overworld --example progression`: crafting, furnace,
  tool-gated loot and rotated mushroom geometry.
- Set **an isolated `CSR_WORLD_DIR`** and `CSR_TEST_DIMENSIONS=1` for progression's
  dimension, inventory transaction, fortress blaze spawn/drop/pickup, staged boss victory and reload checks.
- `cargo run --offline -p overworld --example portals`: generated stronghold
  frames, Eye insertion and constructed Nether portal activation. Requires an
  isolated save directory.
- `cargo run --offline -p launcher --example overworldcheck`: solo/unlimited CS
  simulation, shooting, damage/respawn and Minecraft audio.

The integration checks use staged fixtures; they do not constitute a manual
start-to-finish Survival playthrough. Engine provenance and local vendor changes
are documented in `third_party/README.md`.

## Held items, mining and dragon presentation

Minecraft mode renders the selected pack item model in first person, including
block models, closed chests, both halves of beds, extruded item sprites, inherited display transforms, equip motion,
mining swings, and a skin-textured right arm. An empty hotbar slot shows an empty
hand. N switches between Minecraft items and CS weapons.

The black target outline follows the same shape and reach as mining. All ten
original destruction textures track actual mining progress and disappear when
targeting ends, the tool changes, or the block breaks. Overlays are depth-tested
and slightly offset from the surface to avoid coplanar flicker.

The dragon uses the installed client's baked 37-part model, UVs, and sampled
flight/perch poses (64 frames per clip, interpolated). The local Java reflection
exporter observes the model without decompiling the client. First launch needs
Java 25 and the installed Minecraft libraries; subsequent launches use a cache
in `worlds/_cache`, keyed by client.jar size and timestamp. No dragon data or
textures are bundled with the project. Boss hitboxes follow the animated body
and head. The battle AI and effects remain the custom adaptation described above.

`cargo run --offline -p overworld --example survival_feedback` checks actual
player/environmental death attribution, duplicate reward prevention, saved money,
eight held item types, mining reset behavior, crack assets, and native dragon mesh
animation. Use an isolated `CSR_WORLD_DIR`.

### Audio and frame pacing

Minecraft positional audio now uses one block per audio meter instead of four,
with a 16-block cutoff for ordinary events and 64 for dragon/explosion events.
Volumes follow player distance during playback. Distant/disabled sounds are
culled before loading; the sound-event catalog is cached once.
Terrain completion processing has a soft 1.5 ms budget, at most two uploads per frame, and a 120,000-vertex batch threshold. A single completed job may exceed the soft budget.
The minimap updates eight rows per frame and swaps only completed images. Mob
meshes retain their entity/asset handles instead of despawning each frame.
Existing saves missing a knife receive one when loaded without replacing gear.

### Minecraft video settings and F3

Press O, then Video Settings. Render distance (3–16 chunks), entity distance
(32–192 blocks), entity frustum culling, clouds (Off/Fast/Fancy), Show FPS,
minimap and menu blur apply live. Done/Escape back to gameplay saves settings.
Clouds default to Off; Fast uses flat geometry, Fancy uses the original pack
mask with 4-block-thick cells. Clouds are Overworld-only.

F3 toggles a debug overlay with rolling FPS, frame times, eye XYZ coordinates,
chunk coordinates, dimension, visible/resident sections, uploads, tracked mob
counts, vertex counts and CPU terrain/upload/entity timings. Measurements are
CPU timings, not GPU timestamps. The minimap is hidden while F3 is open.

Entity geometry is culled before construction using conservative bounds;
simulation and saved progress remain independent of rendering. The dragon
stays visible across its arena. Terrain material passes compact their vertex
buffers, and section preparation schedules at most eight jobs per frame,
prioritizing player edits. The aggregate mob mesh no longer uses stale bounds.

For reproducible screenshot runs only, CSR_TEST_F3 enables diagnostics,
CSR_TEST_MC_SETTINGS accepts newline-separated config overrides, and
CSR_TEST_TRAVERSE moves/rotates the camera through a fixed route. Ordinary
gameplay no longer accumulates the screenshot benchmark frame history.

### Minecraft block audio and Overworld chat

Walking plays the installed pack's sound for the supporting block, based on
horizontal distance travelled (including turns). Air movement, idling and
teleports do not generate footsteps. Mining plays material hit sounds at a
200 ms cadence and a break sound on completion; successful player placement
plays the placed block's sound. Local action sounds use native family pitch and
volume, without distant-emitter attenuation. O > Block Sounds controls them.

Y opens all-chat; U opens team-chat; Enter sends; Escape cancels. Overworld is
single-player, so messages are local to this session. Page Up/Down or the wheel
browse the latest 64 messages while chat is open. Typing consumes gameplay
input. The original CS font, translucent lower-left panel and team-style text
colors are used; closed chat hides old messages after 12 seconds.

World notices, kill rewards and death messages appear in chat. Player-attributed
mob deaths also enter the existing six-second, six-entry CS kill feed. Native
mob deaths use a generic [kill] label because the server death record does not
carry the attacking CS weapon. Fatal mob attacks retain the source mob and
attack kind (melee, arrow, explosion, fireball or dragon breath); ordinary
survival hazards retain their environmental label. Nonfatal hits, despawns and
natural mob deaths do not create kill entries or extra money.

Ignored real-asset client tests `minecraft_block_audio_decodes_and_reaches_local_playback`
and `overworld_chat_consumes_input_and_routes_mob_deaths_once` require an isolated
CSR_WORLD_DIR. CSR_TEST_CHAT adds staged chat/feed rows only in screenshot mode.

### Lighting update and render-entity optimization

Chunk lighting already ran on background workers; changed-column comparisons
now run there too, using the previous accepted light and the existing epoch
checks to reject stale results. The main thread receives only changed section
indices. Torch placement/removal still propagates sky/block lighting normally.

Unchanged daylight/fog materials and clear colors no longer trigger Bevy change
notifications each frame. Rebuilt terrain retains its section entities and mesh
handles; rebuilt geometry uploads, and its AABB is refreshed explicitly.
Empty material passes are removed and recreated only when needed. F3 includes
pending/accepted lighting jobs and their average worker time, separately from
CPU terrain/upload timings. A worker duration is not a main-thread frame cost.

`cargo run --offline --profile play -p overworld --example lightingcheck` tests
lighting additions/removals and superseded edits; use an isolated CSR_WORLD_DIR.

F3 also reports chunk integration time and the last tick's loaded count.
Worker-count reductions and an eight-chunk publication cap were tested but
not retained because the traversal benchmark did not improve consistently.


Mob damage feedback:
- Native hurt overlays now retain their signed value through the Bevy entity
  mesh upload. Damaged mobs flash red for the native ten ticks (half a second),
  without becoming transparent. Blaze and dragon models share that feedback.
- Accepted CS bullet/knife hits pass their ray-contact position to bounded CS
  blood puffs and surface splashes. Rejected/invulnerable hits do not bleed.
  The existing Blood setting controls these effects; the red flash stays on.
- Damage feedback is transient and cleared during dimension travel. No save
  format changes. `CSR_TEST_MOB_HURT=1` in screenshot mode provides a healthy/
  hurt cow comparison at frame 240; it never runs with normal player input.


World seed entry (Play > Overworld): click the seed to select its current value,
then type or paste with Ctrl+V. Ctrl+A replaces everything. Arrow keys, Home/End,
Delete, Backspace, keypad input and key repeat work. Enter, Apply, clicking away,
or GO commits the draft; Escape cancels it. Signed 64-bit numbers are preserved;
other text is converted to a stable numeric seed using a UTF-16 string hash.
Blank input or Random Seed chooses a new seed. Existing worlds still resume by
numeric seed. Clipboard text is read only when the player presses Ctrl+V.
