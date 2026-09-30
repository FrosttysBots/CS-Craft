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
    then look at the image
- **Bevy APIs:** read the 0.19 sources in `~/.cargo/registry/src/*/bevy_*-0.19.*`
  before using an API. Do not write them from memory.
- **Editing files with non-ASCII** (`×`, `µ`, `—`): Python's default encoding on
  Windows silently breaks replacements. Use `encoding='utf-8'` or the Edit tool.
- Architecture: `docs/ARCHITECTURE.md`. Plan: `docs/ROADMAP.md`. Multi-game
  goals: `docs/STITCHING.md`.
