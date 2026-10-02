# Movement, lighting and interface checkpoint

This pass addresses camera jitter, displacement lightmap corruption and the
placeholder interface. Existing rounds, economy and bomb rules are retained.

## Try it

1. Double-click `Play CSGO.cmd`. Choose **PLAY**, your map, mode and team, then
   **GO**. The launcher builds current sources before opening the main menu.
2. Click the window to capture the mouse. Competitive starts with freeze time.
3. Press **B**, select a category, then an item with number keys or the mouse.
   **0** or right mouse returns to categories; **B** or **Esc** closes the menu.
4. Hold **Tab** for teams, kills/deaths, and your team's money/equipment.
5. Play a round. As T, the HUD tells you when you carry C4; select **5** and
   hold left mouse inside a site to plant. As CT, look at the bomb and hold
   **E** close to it. A kit shortens the defuse. Releasing use cancels progress.
6. If killed, click to cycle living players. Wait for the next round to spawn.
7. **Esc** opens the pause menu and pauses simulation. **Esc** or **RESUME**
   resumes; **MAIN MENU** leaves the match. **Alt+F4** or **QUIT** closes the game.

Useful feedback: the mode and team you chose, where on the map it happened,
what you pressed, expected versus observed behaviour, and whether it repeats.

## Changes in this pass

- Camera, characters and weapon share a render interpolation timeline. Spectator
  angles cross 0/360 by the short path. Respawns and teleports reset snapshots.
- Mouse input runs before simulation; local mouse look remains immediate.
- Bot aim error now drifts rather than changing randomly every tick.
- Spectator weapons use the watched player's inventory and animations; changing
  player resets weapon sway and animation state.
- Fixed displacement lightmap UVs. 2,784 Dust II faces previously sampled outside
  their atlas rectangles, causing black patches and blocky light changes.
- Original six-face sky textures now render behind the 3D skybox.
- Original Stratum regular/bold fonts decode in memory from the installed VFONT
  files. HUD art, equipment silhouettes, team badges, character images and radar
  textures are read from the install at runtime.
- Radial buy menu supports mouse wedges, keys 1–9, and right-click/0 to go back.
  Prices, ownership and availability still use simulation rules.
- HUD includes the map overview, living teammate positions, team avatars and
  weapon silhouettes. The scoreboard uses original fonts and team badges.
- Hit markers are optional in Game Settings and disabled by default. Blood,
  impact sounds and directional damage feedback provide normal hit feedback.

## Current limits

- Local bots only; no multiplayer networking, grenades or wall penetration.
- Main/pause/settings menus now exist; the layout is a CS:GO-style Bevy
  implementation, not the original Panorama runtime or an exact layout copy.
- This uses original fonts/art with Bevy interface layouts, not the original
  Panorama runtime. It is not yet a pixel-exact reproduction. The buy-menu
  character is an original inventory image, not an animated preview.
- Rendering still lacks normal-mapped directional lighting, VHV baked prop
  lighting, exact Source exposure/colour correction, and some material effects.
  Some distant sky geometry remains visibly incorrect. Character animation and
  hit registration remain approximate.
- Radar currently uses a north-up full-map view; it does not rotate/zoom with
  the camera. Enemy spotting and vertical-floor switching are not implemented.
- Navigation can get bots stuck; strategies and weapon behaviour need tuning.
- The scoreboard is designed around the default 5v5 match.
- Dust II is the validated map. Other maps may load but have not been verified
  in this pass.

## Verification commands

```text
cargo test --workspace
cargo run --release -p launcher -- probe de_dust2
cargo run --release -p launcher -- sim de_dust2
cargo run -p launcher --example soundcheck
cargo build --profile play -p launcher
target\play\csr.exe play de_dust2 --screenshot csr-artifacts\ct.png
```

For UI screenshot checks, `CSR_SCOREBOARD=1` shows the scoreboard and
`CSR_BUY_MENU=1` shows the pistols category during `--screenshot` runs.
`CSR_SPECTATE_BOT=0` follows the first living bot in first person; use
`--deathmatch --frames 500` to check moving bots without freeze time.
These hooks apply to screenshot runs only. The game files remain read-only.

## Verification

The checks below are recorded after the final build. Screenshots are automated
render checks; they do not substitute for a hands-on movement playtest.
- Workspace tests: **93 passed, 0 failed** (including font decoding,
  displacement UV bounds, angle wrapping, buy-wheel selection, knife feedback,
  scope lifecycle/recoil recovery and weapon-switch animation events).
- Release asset probe: **0 failures**, **8,324 displacement lightmaps** checked,
  **1,258 / 1,258 prop models** loaded.
- Release simulation: **0 failures**, ten rounds, 171 purchases, 68 kills,
  seven plants, three defuses and one explosion.
- Weapon audio: **590 decodable**, zero failures.
- HUD, radial buying, scoreboard, outdoor lighting and first-person spectator
  screenshots rendered successfully and were inspected.
- Release spectator smoke run (1600×900, 500 frames, RTX 3070 Laptop):
  **72.6 FPS average / 14.22 ms average frame time**. This is one short automated
  run, not a general performance guarantee or a manual smoothness verdict.
- `git diff --check`: passed. No commits/resets; existing work preserved.


## Weapon behaviour correction (2026-10-01)

- Fire an AK, switch to the knife, then back. Only the draw animation should
  play on each switch. Deployment snapshots a monotonic attack-event cursor,
  and subsequent animation events must belong to the held weapon.
- Left-click slashes; right-click stabs. Knife miss, player-hit and wall-hit
  sounds come from the installed knife soundscripts. Melee produces neither
  firearm sound events nor bullet tracers/decals; it consumes no ammo/recoil.
- Right-click cycles the installed zoom levels on AWP, SSG 08, G3SG1, SCAR-20,
  AUG and SG 553. Check returning to unscoped after the last level. Movement
  speed, spread/inaccuracy, recovery, FOV and zoom sounds use original data.
- Switching, dropping, reloading, death and round reset clear scope state.
  AWP/SSG shots unzoom and resume the previous level after cycling if ammo
  remains. Spectators follow the observed player's scope state.
- Snipers use the original scope arc texture and hide the held model; AUG/SG
  align their original optic geometry with a coloured reticle. The rear lens
  aperture reveals the scene so the barrel does not occlude the magnified view.

Additional check: `cargo run --profile play -p launcher --example weaponcheck`
validates installed draw/knife sequences, knife sound entries, all six scope
cycles, and original zoom sounds. `CSR_TEST_WEAPON=weapon_awp` plus
`CSR_TEST_ZOOM=1` selects a deterministic screenshot loadout; `CSR_TEST_SWITCH=1`
scripts a shot followed by knife/rifle switches. These input/loadout controls
apply only when `--screenshot` disables interactive input. `CSR_WEAPON_TRACE=1`
logs viewmodel activities for diagnosis.

Remaining parity limitations: knife traces use a ray against player hulls,
not the original swept melee volume; impact timing and cooldown/damage tuning
need measured comparison. AUG/SG use an aperture rendering approximation,
not Source's refractive scope shader. Magnification and scope transitions are
functional but optical distortion, edge blur, exact recoil/aimsight motion,
and moving sniper crosshair blur remain unfinished. Other right-click weapon
functions (burst/suppressor/revolver), inspection, event-timed reload/deploy
sounds, grenades, bullet penetration and exact character hitboxes are still
missing. The original menu runtime, several materials/lighting paths, and bot
behaviour are also unfinished. This build must not be labelled a 1:1 rewrite.


Final weapon-pass validation: workspace tests **93/93**, installed-data weapon
checks **0 failures**, release probe **0 failures**, release simulation **10
rounds / 0 failures** (171 purchases, 68 kills, seven plants, three defuses,
one explosion). Audio check: 590 files decoded, zero failures. Both `play`
and `release` executables were rebuilt. The scripted shot/switch check and
scope/knife/map captures were inspected. `git diff --check` passed.


## Rendering, performance and bot tactics (2026-10-01)

New checks: walk upstairs/downstairs, spectate a running bot through corners,
watch an enemy disappear behind a wall, and observe CTs retaking a planted
bomb. Bots retain a last-seen point rather than tracking hidden movement; only
one CT is assigned to defuse while others choose guarding positions. Cover
selection uses navigation hiding spots, static sightlines and teammate spacing.
It is a tactical heuristic, not a reproduction of Valve's complete bot AI.

Normal mapping now works with directional baked world light, prop ambient/sun
lighting, player models and held weapons. Transition materials support a second
normal map. Authored model tangents load directly from VVD; game assets remain
read only. Geometry is spatially batched and PVS/frustum culled. Set CSR_NO_PVS=1
to compare visibility when diagnosing any disappearing geometry.

Measured in the play build, RTX 3070 Laptop, 1600x900, 720-frame runs with the
first 120 frames excluded, no simultaneous compilation:

| Scene | Before FPS | After FPS | Before p99 | After p99 | Before max | After max |
|---|---:|---:|---:|---:|---:|---:|
| Fixed T-spawn view | 108.1 | 115.2 | 13.45 ms | 11.85 ms | 17.71 ms | 16.79 ms |
| Bot spectator | 80.6 | 109.9 | 19.81 ms | 12.24 ms | 31.51 ms | 17.91 ms |

The fixed scene uses identical coordinates. The spectator runs use the same
setup but follow different trajectories after the AI change, so their numbers
are indicative rather than a controlled GPU comparison. These are short smoke
runs, not a guarantee of hitch-free play. Additional shading and prewarming
increase startup time: world/prop setup roughly 4.2s versus 1.5–1.9s before,
and character/viewmodel setup about 1.5s versus 0.4s.

Remaining visual gaps include baked VHV prop lighting, reflections/specular
material paths, Source colour correction, overlays/detail props, and exact
shadow/material parity. Navigation can still need local recovery at tight
doors/ledges; squad tactics, grenades, sound-driven awareness and exact player
collision/animation behavior are unfinished. Do not label this a 1:1 rewrite.

Final rendering/bot validation: **104 workspace tests passed**, release probe
**0 failures** (1,258/1,258 prop models, 8,324 displacement lightmaps), and release
simulation **10 rounds / 0 failures** (165 purchases, 68 kills, nine plants,
zero defuses, one explosion, T 9–1 CT). The 60-second deathmatch logged 104,239
units walked and a longest continuous commanded-but-blocked movement of 0.61s.
Audio: **590 decoded / 0 failures**. Spawn, spectator and both bombsite captures
were inspected. World/prop loading reported zero missing textures. CT retakes
remain a tuning issue despite passing the current functional checks.


## Menus, dropped weapons and hit feedback (2026-10-01)

- Team changes remove both the old character and held weapon, preventing
  orphan guns at the previous hand position. Dropped-model pivots are corrected
  using rotated vertex bounds so the model rests on the collision surface.
- Bullet impacts create bounded surface marks and short dust puffs. Damage
  creates blood puffs and traced wall/floor splats. The installed multiply
  blood texture becomes an alpha mask in memory; no game assets are copied.
  Impact/blood shaders are queued during startup to avoid losing a first puff
  while its pipeline compiles. These approximate the original particle effects.
- Installed soundscripts supply flesh, armor, helmet, headshot and surface
  impact sounds. Master volume changes affect active positional sounds too.
- Main, pause, game, video, audio, controls and crosshair pages use original
  Stratum fonts, logos, characters and map thumbnails. Settings persist in
  `config/user.cfg`. Keyboard action bindings swap to avoid duplicates;
  Esc, Q, V, number keys, function keys and mouse actions remain reserved.
- Video offers windowed/borderless, four resolutions, VSync, 4x MSAA and
  brightness. Crosshair offers colour, length, gap, thickness and centre dot.
- Play discovers installed root de_/cs_ BSPs (27 on this installation).
  Competitive and Deathmatch are implemented; cs_ maps are Deathmatch-only
  because hostage rules are not implemented. Bot count means enemies, with
  one fewer teammate. Changing maps starts a fresh game process after the old
  window closes; startup currently reloads assets rather than streaming them.

Regression checks cover settings validation/round trips, floor placement,
material-family sounds, blood alpha conversion, pause/resume cursor capture,
clearing pending actions, resume-click gating and hostage-map mode selection.
`CSR_TEST_MENU=main|play|graphics|audio|controls|crosshair` captures menu pages.
`CSR_TEST_FEEDBACK=1` injects an explicitly synthetic rendering/event fixture;
it does not prove combat accuracy. `CSR_TEST_DROP=1` drops the real held weapon
shortly before capture and looks down. All hooks require `--screenshot`.

Desktop interaction verification was limited by the computer-use capture error
`SetIsBorderRequired failed: No such interface supported (0x80004002)` on two
attempts. Menu systems were tested directly and game screenshots inspected;
the full mouse-driven map-change/settings flow still needs a manual check.
Exact Panorama layouts, full graphics/audio option parity, hostage rules,
grenades, penetration, ragdolls and network play remain unfinished.

Final validation: **112 workspace tests passed**, plus the explicitly enabled
installed-world-weapon test (AK, M4A1-S, Glock and knife at three yaw angles).
Both `play` and `release` binaries were rebuilt. Release asset probes for
Dust II and Mirage reported **0 failures** (1,258 and 241 prop models loaded).
The final release simulation completed **10 rounds / 0 check failures**, with
165 purchases, 68 kills, nine plants, zero defuses and one explosion. CTs lost
9–1; retake tuning remains a known weakness. Audio: **622 files decoded / 0
failures**, including every referenced damage/impact sound. Menu pages were
inspected at 1280×720 and 1600×900, along with the synthetic damage fixture,
real dropped M4A1-S, Mirage gameplay and Office's Deathmatch selection.
Dust II, Mirage and Office render loading reported zero missing textures;
this does not establish full material fidelity or validate every selectable map.
`git diff --check` passed. Captures made during compilation are not performance
benchmarks. Full mouse-driven interaction remains unverified due to the desktop
capture failure described above.

## Tracer / inventory / map-card pass (2026-10-01)

- Tracers start at posed model attachments rather than the eye ray. Attachment
  offsets cover the P90 and revolver, which lack dedicated flash bones. First-
  person projection compensates for the separate weapon FOV. Wall clipping,
  maximum 85-unit streaks, 0.12-second life and 25% initial alpha keep them subtle.
- F (rebindable) plays the installed inspection sequence. Fire, reload, plant,
  scope and weapon switch override it. All 36 base guns/knives have decodable
  inspect clips; muzzle attachments are audited for every base gun.
- Main uses the late-CS:GO Panorama arrangement with original fonts/images and
  a live character model. Visual references: [Valve Panorama announcement](https://blog.counter-strike.net/2018/06/20482/)
  and [late-CS:GO menu screenshot](https://insights.gg/blog/how-to-record-cs-go-gameplay).
  This is a clean-room local front end, not complete online Panorama parity.
- Play shows direct map cards; hover lifts/tilts and strengthens the shadow,
  while selection retains its green border. All 27 installed maps fit together
  at 1280x720. The grid also supports wheel scrolling for larger installations.
- Inventory is a local base-item collection, not a Steam account inventory.
  Weapon filters, 3D rotation, team eligibility and primary/pistol loadouts are
  implemented. Equip writes `config/inventory.cfg`; choices apply at the next
  Deathmatch spawn and cannot grant free weapons in Competitive.
- `CSR_TEST_MENU=inventory` and `CSR_TEST_HOVER=1` add screenshot coverage;
  `CSR_INSPECT=1` requests a real inspect animation 90 frames before capture.
  `cargo run --offline --profile play -p launcher --example inspectcheck` audits
  installed inspection clips and muzzle attachments without the renderer.

Manual mouse-driven interaction remains limited by the desktop capture failure
noted above; menu input systems and rendered screenshots are checked separately.

Validation: 119 workspace tests passed, plus the explicitly enabled installed
world-weapon placement test. All 36 base-weapon inspection/muzzle audits passed.
The release Dust II asset probe loaded all 1,258 prop models with zero failures;
audio decoded all 622 referenced weapon/feedback files without failures. Both
play and release builds completed. Inspected captures cover the main menu,
all 27 map cards including selection/hover, 720p inventory and controls, AK and
knife inspection poses, and P90 firing. The firing screenshot is a single frame,
so attachment/projection tests establish tracer origin independently of capture
timing. Captures during compilation are not performance benchmarks.

The final release simulation completed 10 competitive rounds with zero check
failures: 165 purchases, 68 kills, nine plants, zero defuses and one explosion.
T won 9-1, preserving the known CT retake weakness. `git diff --check` passed.

## Team AI and model lighting overhaul (2026-10-01)

This section supersedes the older bot, grenade and model-lighting limitations above.

- Competitive bots choose a team full buy, eco or force buy and preserve an eco
  reserve. Roles distribute entries, carrier support, lurking, anchors and a
  CT rotator. Survivors keep equipment; utility comes from ordinary purchases.
- T teams stage on a navigable approach before executing, with a timeout so
  one delayed player cannot stall the round. Recent reported stacks can prompt
  one site rotation. CTs rotate using reports, approach retakes before holding
  covering angles, select a defuser by path time plus kit duration, and save
  equipment when their remaining defuse window is impossible.
- Visual reports have radio delay and expiry; sound reports supply coarse
  locations. Hidden positions never become firing targets. Smoke and flashes
  affect sight. Aim uses recoil compensation, distance-sensitive bursts, a
  movement accuracy gate, target stickiness and teammate firing-line checks.
  Remembered enemies no longer suspend objective navigation.
- Buy grenades in B > Grenades. Press 4 to select/cycle, then left-click to
  throw. Four total, two flashes maximum, one of each other grenade. Grenades
  follow collision-tested ballistic trajectories. Smoke obscures vision and
  extinguishes fire; flashes respect geometry and facing; HE has blast falloff
  and armour absorption; fire damages exposed players; decoys emit false shots.
  Bots check friend safety, team spacing/cooldowns and the throw arc, and avoid
  burning nav areas. Bot utility is carried, purchased and consumed normally.
- Player materials now read Phong exponent textures, tint, Fresnel and rim
  controls. Normal maps affect these highlights. Ambient and sunlight sampling
  uses multiple body heights with temporal blending rather than abrupt changes.
- Original grenade models, smoke texture, animated fire sprite sheet and sound
  entries load from the installed game at runtime. No game assets are copied.

These are clean-room implementations, not exact original bot/particle code.
Grenade timings, blast/flash curves, smoke shape and fire spread are approximate;
pin-pull/throw animation timing, grenade bounce audio, dynamic character shadows,
lightwarp/cubemap shader parity, hostage tactics and team weapon donations remain
unfinished. Utility is conservative and can be underused when no safe arc exists.
Multi-map checks and fixture captures do not establish competitive balance or
replace a human movement/spectator playtest.

Screenshot-only utility hooks: `CSR_TEST_UTILITY=smoke|fire|flash`.
`CSR_WATCH_BOT=0` inspects a T model; `CSR_WATCH_BOT=5` inspects a CT at default
team sizes. Simulation logs now count throws by kind and detonations.

Final validation: 127 workspace tests passed, plus the separately enabled
installed-weapon placement test. All 622 weapon/feedback audio files decoded.
Both profiles rebuilt; the final release Dust II asset probe loaded all 1,258
prop models with zero failures. Final ten-round samples had zero check failures:
Dust II T 6-4 CT (four plants, one defuse), Mirage 5-5 (four plants, zero defuses),
Inferno 5-5 (seven plants, two defuses). Together these produced 39 utility
detonations, covering smoke, flash, HE, molotov and incendiary use. Mirage's
retakes remain a weakness despite its even score. These are samples, not a
balance guarantee. T/CT lighting and original smoke/fire rendering were
inspected; flash rendering was checked after compilation stopped.

## Local skins, cases and menu animation (2026-10-01)

The local collection now reads 39 cases and 606 weapon finishes from the
installed schema. Cases open without keys; the console command
**give_case Revolution 5** grants five cases. **cases** lists names/IDs.
The terminal accepts **csr give-case "Revolution Case" 5**.
See [COSMETICS.md](COSMETICS.md) for persistence, commands and current limits.

The opening saves its item before animating a decelerating reel. The selected
reward becomes a rotating skinned 3D preview, with equip/default controls.
Original case icons, item artwork, fonts and sounds are read at runtime.
Equipment applies to first-person, held and dropped weapon instances; cosmetic
metadata stays with the weapon through pickups. The main menu uses the original
551-frame rifle idle with real-time skeletal animation and weapon bone merging.

Final validation: play/release builds passed; 132 workspace tests and the
separately enabled installed-weapon placement test passed. The installed
cosmetic audit found all icons, paint textures and paintable model bindings,
completed 1,000 durable openings, verified exhaustion/save/reload/equip, decoded
six case sounds and confirmed that the menu clip changes bone poses.
All 628 weapon/feedback/case sound files decoded. The final release Dust II
probe loaded 1,258/1,258 models with zero failures. Ten simulated rounds ended
T 6–4 CT, with 68 kills, four plants, one defuse and eight utility detonations;
simulation failures: zero. UI action tests check single consumption and the
reel's winning item. Case, reward, first-person skin and character captures
were visually inspected. These checks do not replace manual mouse/keyboard
playtesting.

The initial limitations above are superseded by the expanded collection below.
Audit fixtures use temporary/in-memory inventories and never grant rewards to
the user's profile.

## Expanded collection (2026-10-02)

Added the rare pools for all 39 cases, covering 19 knife models and eight glove
models, plus 120 souvenir packages. The installed catalog now exposes 1,732
finishes and 6,711 sticker definitions. Separate souvenir browsing, quality
labels, sticker application/scraping, searchable grants and v1-to-v2 profile
migration accompany the expanded data.

Rare knives use their own models and original animations. Equipped gloves use
their original hand meshes and retain the team's sleeves. Third-person gloves
and StatTrak modules follow the character/weapon skeleton. StatTrak counters
persist per serial; only enemy kills by the original owner increment them.
Both original digital modules and knife engravings display the saved value.
Four sticker positions use original first-person geometry; world placement is
fitted from texture landmarks. Holographic masks and spectra respond to angle.

Rendering now reads the glove palette index from the authored alpha channel,
preserves original normal maps, applies paint-kit Phong controls and separates
UV-mapped finishes from triplanar spray/airbrush finishes. Anodized color strength
no longer leaves the Fade blade almost black. The catalog is parsed once.

Fidelity boundaries remain in COSMETICS.md: local odds, approximate
paint/reflection/holo compositing and world sticker transforms, and generated
event-linked souvenir stickers without real match provenance. None of these
checks establish pixel-identical Source behavior.

Final checks: 136 workspace tests passed; the expanded installed-content audit
passed all 138 world sticker mappings, both character skeletons for all glove
families, all souvenir award rules and 1,000 durable openings. CLI grants and
save migration were exercised in an isolated profile; failed grants preserved
the file byte-for-byte. All 628 sound files decoded.

The final release build passed. Dust II loaded 1,258/1,258 prop models with zero
probe failures. Ten release-simulated rounds ended T 6–4 CT with 68 kills, four
plants, one defuse and eight utility detonations; zero simulation failures.
Rendered captures were inspected for StatTrak, stickers, rare knife inspection,
gloves, souvenir rewards and the menu character. Manual interactive playtesting
remains distinct from these automated checks.
