CSGO REWRITE
============

A from-scratch Rust rewrite of Counter-Strike: Global Offensive's game
runtime, with a Minecraft "Overworld" mode. Written by Mr. Claude
(Anthropic's Claude), for hawk.

This is a fan project. It is not affiliated with or endorsed by Valve or
Mojang/Microsoft. It contains no Valve game files: it reads the CS:GO Legacy
install you already own.


WHAT YOU NEED
-------------
* Windows 10 or 11 (64-bit) and a graphics card with DirectX 12 or Vulkan.
* CS:GO Legacy, installed through Steam:
  Steam > Library > right-click Counter-Strike 2 > Properties > Betas >
  choose "csgo_legacy" and let it download.


FIRST LAUNCH
------------
1. Unzip this folder anywhere (for example your Desktop).
2. Double-click "Play CSGO.bat" (or "Play Minecraft.bat").
3. The game searches your PC for CS:GO Legacy (every Steam library on every
   drive) and asks "Use this folder?". Click Yes.
   - No lets you pick a different install. If none is found, a file
     browser opens: go to your "Counter-Strike Global Offensive" folder
     (normally C:\Program Files (x86)\Steam\steamapps\common\) and select
     csgo.exe.
   - If it says Counter-Strike 2 is not on the CS:GO Legacy branch, switch
     the beta in Steam (see WHAT YOU NEED), let it download, and start again.
4. The game starts. It remembers the folder from then on.

Every copy starts fresh: no skins, no cases, no Minecraft worlds.


PLAYING
-------
Play CSGO.bat         de_dust2 with bots; the main menu has maps and modes
Play Minecraft.bat    the Minecraft Overworld (CS:GO movement and weapons)
Change CSGO Folder.bat  pick the CS:GO Legacy folder again

CS:GO:     WASD move, Space jump, Ctrl crouch, Shift walk, LMB fire,
           R reload, E use/defuse, 1-5 / Q / wheel weapons, G drop,
           B buy menu, Tab scoreboard, Esc menu.
           Plant: select the bomb (5) in a bomb site and hold LMB.
Minecraft: N switches tools/weapons, LMB mine, RMB use/place,
           I inventory, F3 debug overlay.

The Minecraft world is built before it appears ("Preparing world" in the
console, then a short "Loading terrain" screen while shaders compile), so
it plays without stutters. A lower Render Distance in the video settings
loads faster.


YOUR DATA
---------
Everything the game saves (settings, skins, cases, Minecraft worlds, the
CS:GO folder choice) is in the "userdata" folder next to the game. Delete
that folder to reset to a fresh install. Copy it to keep your progress when
updating to a new version.


TROUBLESHOOTING
---------------
* "That folder holds Counter-Strike 2": switch Counter-Strike 2 to the
  csgo_legacy beta (see WHAT YOU NEED), wait for the download, try again.
* The console window shows what the game is doing; if something fails,
  the message there (or in the pop-up) says why.


CREDITS AND LICENCES
--------------------
Written by Mr. Claude. Built on the Bevy engine. The Minecraft mode uses the
MinecraftOSS / minecraft_terrain crates from 2010-rust-rewrite-mashup
(see licenses\). Minecraft game assets in minecraft-26.3\ belong to
Mojang/Microsoft; CS:GO assets are read from your own install and belong
to Valve.
