# Notes for agents working on CSGO Rewrite

- **Owner:** hawk. **Author credit:** "Mr. Claude" (keep it in the README and Cargo `authors`).
- **Game data:** Steam app 730 on the `csgo_legacy` beta branch, at
  `C:\Program Files (x86)\Steam\steamapps\common\Counter-Strike Global Offensive\csgo`.
  Read only. Never copy game files into the repo. `CSR_GAME_DIR` overrides the path.
- **Clean-room:** never use the leaked CS:GO/TF2 source, and never paste Source SDK
  code. Work from public format documentation and observed behaviour.
- **Toolchain:** cargo lives in `%USERPROFILE%\.cargo\bin`. In Git Bash:
  `export PATH="$HOME/.cargo/bin:$PATH"`.
- **Builds:** Bevy makes the first `client` build slow (several minutes). Use
  `--profile play` for day-to-day runs; `release` uses thin LTO.
- **Verify every change** with:
  - `cargo test --workspace`
  - `cargo run --release -p launcher -- probe de_dust2` (expects `failures: 0`)
  - `cargo run --release -p launcher -- sim de_dust2` (expects `failures: 0`)
  - for visual changes: `csr play de_dust2 --screenshot shot.png --camera x y z pitch yaw`,
    then look at the image. The smoke test logs average FPS. Debug env vars:
    `CSR_WATCH_BOT=n` (camera on bot n), `CSR_AUTOFIRE=1` (hold fire),
    `CSR_NO_PROPS`, `CSR_NO_HUD`, `CSR_NO_CULL=world|props|all`,
    `CSR_PRESENT=immediate|mailbox|fifo`, `CSR_FRAME_LOG=ms` (log frames slower
    than that, with renderer and stream timings), `CSR_TEST_WEAPON=weapon_x`
    with `CSR_TEST_RELOAD=frame` (empty the clip and reload) and
    `CSR_WEAPON_TRACE=1` (log viewmodel activities and shell inserts)
  - Minecraft pacing: `csr play overworld --seed 42 --frames 1200 --screenshot x.png`
    logs `frame pacing after warmup` (p95/p99/max); add `CSR_TEST_TRAVERSE=1`
    for a fast flight through new terrain. Use a scratch `CSR_WORLD_DIR`.
  - audio: `cargo run -p launcher --example soundcheck` decodes every weapon sound
    through the same checks as the game (Bevy panics on undecodable audio)
- **Bevy APIs:** read the 0.19 sources in `~/.cargo/registry/src/*/bevy_*-0.19.*`
  before using an API. Do not write them from memory.
- **Editing files with non-ASCII** (`×`, `µ`, `—`): Python's default encoding on
  Windows silently breaks replacements. Use `encoding='utf-8'` or the Edit tool.
- **Player data and setup:** the launcher (`crates/launcher/src/setup.rs`)
  runs inside the data folder: the repository root for source builds, or
  `userdata\` beside the exe when a `csgo-rewrite.portable` file sits next to
  it (release builds). Save paths are relative to it; never use
  `CARGO_MANIFEST_DIR` for runtime files. `CSR_DATA_DIR` overrides.
- **Releases:** `Package Release.cmd` (or `tools\package-release.ps1`) builds
  `dist\CSGO-Rewrite-<version>-windows-x64.zip`: the exe, launch scripts, the
  portable marker and the bundled Minecraft 26.3 assets, without any player
  data. The first launch asks for the CS:GO Legacy folder. `dist\` is
  ignored by git; upload the zip to GitHub Releases.
- Architecture: `docs/ARCHITECTURE.md`. Plan: `docs/ROADMAP.md`. Multi-game
  goals: `docs/STITCHING.md`.
