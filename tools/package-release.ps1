# Builds a player-ready Windows release of CSGO Rewrite:
#   dist\CSGO-Rewrite\            the game folder (exe, launch scripts, bundled Minecraft assets)
#   dist\CSGO-Rewrite-<version>-windows-x64.zip
#
# The release is a fresh install: no settings, skins, cases or Minecraft
# worlds are included. On first launch the game asks the player for their
# CS:GO Legacy folder (CS:GO files are never shipped).
#
# Usage (from the repository root):
#   powershell -ExecutionPolicy Bypass -File tools\package-release.ps1 [-MinecraftDir <minecraft-26.3 folder>] [-SkipBuild]

param(
    [string]$MinecraftDir = $env:CSR_MINECRAFT_DIR,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$name = 'CSGO-Rewrite'
$dist = Join-Path $root 'dist'
$out = Join-Path $dist $name
$zip = Join-Path $dist "$name-$version-windows-x64.zip"

# --- Minecraft assets to bundle ------------------------------------------
if (-not $MinecraftDir) {
    $MinecraftDir = Join-Path $env:USERPROFILE 'Downloads\2010-Rust-Rewrite-Mashup-windows-x64\2010-Rust-Rewrite-Mashup\iw4l-artifacts\minecraft-26.3'
}
if (-not (Test-Path (Join-Path $MinecraftDir 'artifacts\block-state-catalog\26.3.json'))) {
    throw "Minecraft 26.3 assets not found at '$MinecraftDir'. Pass -MinecraftDir <folder>."
}
$dragon = Get-ChildItem (Join-Path $root 'worlds\_cache') -Filter 'dragon-native-v1-*.json' -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime | Select-Object -Last 1
if (-not $dragon) {
    Write-Warning 'No exported Ender dragon model in worlds\_cache (play Overworld once to create it). The release dragon will be invisible.'
}

# --- Build -----------------------------------------------------------------
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
$targetDir = Join-Path $root 'target\dist'
if (-not $SkipBuild) {
    Write-Host "Building release $version (static C runtime, thin LTO). This takes a while..."
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    & $cargo build --release -p launcher --target-dir $targetDir
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    Remove-Item Env:RUSTFLAGS
}
$exe = Join-Path $targetDir 'release\csr.exe'
if (-not (Test-Path $exe)) { throw "Missing $exe (run without -SkipBuild)" }

# --- Lay out the game folder -----------------------------------------------
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item $exe (Join-Path $out "$name.exe")

# Marks a release: player data goes to userdata\ next to the exe.
Set-Content -Encoding ascii (Join-Path $out 'csgo-rewrite.portable') @'
This file marks a release copy of CSGO Rewrite. Player data (settings, skins,
cases, Minecraft worlds and the CS:GO folder choice) is kept in the userdata
folder next to the game. Delete userdata to start over as a fresh install.
'@

function Write-Bat($file, $body) {
    Set-Content -Encoding ascii (Join-Path $out $file) ("@echo off`r`ncd /d `"%~dp0`"`r`n" + $body)
}
Write-Bat 'Play CSGO.bat' @"
"$name.exe" play de_dust2
if errorlevel 1 pause
"@
Write-Bat 'Play Minecraft.bat' @"
"$name.exe" play overworld
if errorlevel 1 pause
"@
Write-Bat 'Change CSGO Folder.bat' @"
"$name.exe" setup
pause
"@

Write-Host 'Copying Minecraft assets...'
$mcOut = Join-Path $out 'minecraft-26.3'
# robocopy, not Copy-Item: some data-pack paths exceed Windows PowerShell's
# 260-character limit.
robocopy $MinecraftDir $mcOut /E /NFL /NDL /NJH /NJS /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "Copying Minecraft assets failed (robocopy $LASTEXITCODE)" }
$global:LASTEXITCODE = 0
if ($dragon) { Copy-Item $dragon.FullName (Join-Path $mcOut 'dragon-native-v1.json') }

$licenses = Join-Path $out 'licenses'
New-Item -ItemType Directory -Force $licenses | Out-Null
Copy-Item 'third_party\MASHUP-LICENSE', 'third_party\MASHUP-NOTICE', 'third_party\README.md' $licenses
Rename-Item (Join-Path $licenses 'README.md') 'THIRD-PARTY.md'
Copy-Item 'third_party\rust-audio\lewton-0.10.2\LICENSE' (Join-Path $licenses 'lewton-LICENSE')
Copy-Item 'third_party\rust-audio\ogg-0.8.0\LICENSE' (Join-Path $licenses 'ogg-LICENSE')
Copy-Item 'docs\PLAYERS.md' (Join-Path $out 'README.txt')

# Nothing personal goes out.
foreach ($p in 'userdata', 'config', 'worlds', 'logs') {
    if (Test-Path (Join-Path $out $p)) { throw "Release unexpectedly contains $p" }
}

# --- Zip -------------------------------------------------------------------
if (Test-Path $zip) { Remove-Item $zip -Force }
Write-Host 'Zipping...'
# Windows' bsdtar writes zip files and copes with long paths.
$tar = Join-Path $env:SystemRoot 'System32\tar.exe'
& $tar -a -c -f $zip -C $dist $name
if ($LASTEXITCODE -ne 0) { throw 'Creating the zip failed' }
$size = [math]::Round((Get-Item $zip).Length / 1MB)
Write-Host ""
Write-Host "Release ready: $zip ($size MB)"
Write-Host "Folder:        $out"
