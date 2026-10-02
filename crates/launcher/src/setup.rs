//! Install setup: where player data lives and where the game assets are.
//!
//! * **Data folder.** A release build ships a `csgo-rewrite.portable` file
//!   next to the executable; its player data (settings, skins, cases,
//!   Minecraft worlds) then lives in `userdata\` beside it, so a freshly
//!   unpacked copy starts clean. Source checkouts keep theirs at the
//!   repository root. `CSR_DATA_DIR` overrides both. The process runs inside the
//!   data folder, so every relative save path (`config\`, `worlds\`) lands
//!   there.
//! * **CS:GO Legacy.** The first run of a release build searches every Steam
//!   library on the PC (Steam's registry entry, its `libraryfolders.vdf`, and
//!   the usual library folders on each drive), asks the player to confirm
//!   the install it found, and falls back to browsing for `csgo.exe`. The
//!   choice is checked and remembered in `config\paths.cfg`. `csr setup`
//!   asks again; `csr find-csgo` prints what the search sees.
//!   `CSR_GAME_DIR` overrides.
//! * **Minecraft.** A release build bundles the Minecraft 26.3 assets in
//!   `minecraft-26.3\` beside the executable. `CSR_MINECRAFT_DIR` overrides.

use std::path::{Path, PathBuf};

pub const PORTABLE_MARKER: &str = "csgo-rewrite.portable";
const PATHS_FILE: &str = "config/paths.cfg";

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

pub fn is_portable() -> bool {
    exe_dir().is_some_and(|d| d.join(PORTABLE_MARKER).is_file())
}

/// The CS:GO game folder (the one holding `gameinfo.txt`) for a folder the
/// player picked: the install root, its `csgo` folder, or a file inside it.
pub fn csgo_game_dir(picked: &Path) -> Result<PathBuf, String> {
    let picked = if picked.is_file() { picked.parent().unwrap_or(picked) } else { picked };
    let candidates = [picked.to_path_buf(), picked.join("csgo"), picked.parent().map(Path::to_path_buf).unwrap_or_default()];
    for dir in &candidates {
        if dir.join("gameinfo.txt").is_file() && dir.join("pak01_dir.vpk").is_file() {
            return Ok(dir.clone());
        }
    }
    if picked.join("game/csgo/gameinfo.gi").is_file() || picked.join("gameinfo.gi").is_file() || picked.join("csgo/gameinfo.gi").is_file() {
        return Err("That folder holds Counter-Strike 2, not CS:GO Legacy.\n\n\
            In Steam, right-click Counter-Strike 2 > Properties > Betas and choose \
            \"csgo_legacy\", let it download, then select its csgo.exe again."
            .into());
    }
    Err(format!(
        "No CS:GO Legacy game files in\n{}\n\nSelect csgo.exe inside the \"Counter-Strike Global Offensive\" \
        folder (next to a \"csgo\" folder with pak01_dir.vpk).",
        picked.display()
    ))
}

fn read_paths(data: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(data.join(PATHS_FILE))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect()
}

fn write_paths(data: &Path, entries: &[(String, String)]) -> std::io::Result<()> {
    let path = data.join(PATHS_FILE);
    std::fs::create_dir_all(path.parent().unwrap_or(data))?;
    let text: String = entries.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    std::fs::write(path, text)
}

/// Runs a PowerShell snippet (Windows dialogs) and returns its output.
fn powershell(script: &str) -> Option<String> {
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-STA", "-Command"])
        .arg(format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8; Add-Type -AssemblyName System.Windows.Forms; {script}"))
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn message(title: &str, text: &str) {
    let shown = powershell(&format!(
        "[void][System.Windows.Forms.MessageBox]::Show({}, {})",
        ps_quote(text),
        ps_quote(title)
    ));
    if shown.is_none() {
        eprintln!("{title}: {text}");
    }
}

/// What a search of this PC found: CS:GO Legacy game folders (the `csgo`
/// folders holding `gameinfo.txt`) and Counter-Strike 2 installs that have
/// not been switched to the legacy branch.
#[derive(Debug, Default)]
pub struct Detected {
    pub legacy: Vec<PathBuf>,
    pub cs2_only: Vec<PathBuf>,
}

/// Steam's install folder from the registry (`reg` ships with Windows).
fn steam_from_registry() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for (key, value) in [
        (r"HKCU\Software\Valve\Steam", "SteamPath"),
        (r"HKLM\SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
        (r"HKLM\SOFTWARE\Valve\Steam", "InstallPath"),
    ] {
        let Ok(o) = std::process::Command::new("reg").args(["query", key, "/v", value]).output() else { continue };
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            if let Some((_, path)) = line.split_once("REG_SZ") {
                out.push(PathBuf::from(path.trim()));
            }
        }
    }
    out
}

/// Steam library folders: registered ones (from every Steam install's
/// `libraryfolders.vdf`) plus the usual places on every drive.
fn steam_libraries() -> Vec<PathBuf> {
    let mut roots = steam_from_registry();
    roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    roots.push(PathBuf::from(r"C:\Program Files\Steam"));
    for letter in b'C'..=b'Z' {
        let drive = PathBuf::from(format!("{}:\\", letter as char));
        if !drive.is_dir() {
            continue;
        }
        for sub in ["SteamLibrary", "Steam", r"Program Files (x86)\Steam", r"Program Files\Steam", r"Games\Steam", r"Games\SteamLibrary", "Games"] {
            roots.push(drive.join(sub));
        }
    }
    libraries_from(roots)
}

/// Each root plus the libraries its `steamapps\libraryfolders.vdf` lists.
fn libraries_from(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut libraries = Vec::new();
    for root in roots {
        if let Ok(bytes) = std::fs::read(root.join("steamapps/libraryfolders.vdf")) {
            if let Some(folders) = kv::parse_bytes(&bytes).ok().as_ref().and_then(|d| d.block("libraryfolders")) {
                for (_, v) in folders.iter() {
                    if let Some(p) = v.as_block().and_then(|b| b.str("path")) {
                        libraries.push(PathBuf::from(p.replace(r"\\", r"\")));
                    }
                }
            }
        }
        libraries.push(root);
    }
    let mut seen = std::collections::HashSet::new();
    libraries.retain(|p| seen.insert(p.to_string_lossy().to_lowercase().replace('/', "\\").trim_end_matches('\\').to_string()));
    libraries
}

/// Looks for CS:GO Legacy in every Steam library on this PC.
pub fn detect() -> Detected {
    detect_in(steam_libraries())
}

fn detect_in(libraries: Vec<PathBuf>) -> Detected {
    let mut found = Detected::default();
    for lib in libraries {
        let install = lib.join(r"steamapps\common\Counter-Strike Global Offensive");
        if !install.is_dir() {
            continue;
        }
        match csgo_game_dir(&install) {
            Ok(game) if !found.legacy.contains(&game) => found.legacy.push(game),
            Ok(_) => {}
            Err(_) if install.join(r"game\csgo\gameinfo.gi").is_file() => found.cs2_only.push(install),
            Err(_) => {}
        }
    }
    found
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    Yes,
    No,
    Cancel,
}

/// "Use this folder?" with Yes / No / Cancel.
fn confirm(game: &Path) -> Answer {
    let install = game.parent().unwrap_or(game);
    let text = format!(
        "CSGO Rewrite found CS:GO Legacy here:\n\n{}\n\nUse this folder?\n\nYes - use it\nNo - choose a different one\nCancel - quit",
        install.display()
    );
    let script = format!(
        "$top=New-Object System.Windows.Forms.Form -Property @{{TopMost=$true}}; \
         [Console]::Out.Write([System.Windows.Forms.MessageBox]::Show($top, {}, 'CSGO Rewrite setup', 'YesNoCancel', 'Question'))",
        ps_quote(&text)
    );
    match powershell(&script).as_deref() {
        Some("Yes") => Answer::Yes,
        Some("No") => Answer::No,
        Some(_) => Answer::Cancel,
        None => {
            eprint!("{text}\n[Y]es / [N]o / [C]ancel: ");
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            match line.trim().chars().next().map(|c| c.to_ascii_lowercase()) {
                Some('y') => Answer::Yes,
                Some('n') => Answer::No,
                _ => Answer::Cancel,
            }
        }
    }
}

/// Asks the player to find `csgo.exe` themselves; `None` if they cancel.
fn pick_csgo_exe(start: Option<&Path>) -> Option<PathBuf> {
    let start = start.map(|p| format!("$d.InitialDirectory={};", ps_quote(&p.display().to_string()))).unwrap_or_default();
    let script = format!(
        "$d=New-Object System.Windows.Forms.OpenFileDialog; \
         $d.Title='Select csgo.exe in your CS:GO Legacy folder'; \
         $d.Filter='CS:GO Legacy (csgo.exe)|csgo.exe|All files (*.*)|*.*'; {start} \
         $top=New-Object System.Windows.Forms.Form -Property @{{TopMost=$true}}; \
         if($d.ShowDialog($top) -eq [System.Windows.Forms.DialogResult]::OK){{[Console]::Out.Write($d.FileName)}}"
    );
    match powershell(&script) {
        Some(p) if !p.is_empty() => Some(PathBuf::from(p)),
        Some(_) => None,
        None => {
            // No PowerShell: ask on the console instead.
            eprint!("Path to csgo.exe (in your CS:GO Legacy folder): ");
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).ok()?;
            let line = line.trim().trim_matches('"');
            (!line.is_empty()).then(|| PathBuf::from(line))
        }
    }
}

/// First-time setup: offers each detected install for confirmation, then
/// falls back to browsing for `csgo.exe`. `None` if the player cancels.
fn ask_for_csgo(found: &Detected) -> Option<PathBuf> {
    for game in &found.legacy {
        match confirm(game) {
            Answer::Yes => return Some(game.clone()),
            Answer::No => continue,
            Answer::Cancel => return None,
        }
    }
    let start = found.legacy.first().and_then(|g| g.parent()).or(found.cs2_only.first().map(PathBuf::as_path)).map(Path::to_path_buf);
    if found.legacy.is_empty() {
        let text = if found.cs2_only.is_empty() {
            "CSGO Rewrite could not find CS:GO Legacy on this PC.\n\n\
             Install it in Steam (right-click Counter-Strike 2 > Properties > Betas > csgo_legacy), \
             or click OK and select csgo.exe in your CS:GO Legacy folder."
                .to_string()
        } else {
            format!(
                "Counter-Strike 2 is installed at\n{}\nbut it is not on the CS:GO Legacy branch.\n\n\
                 In Steam: right-click Counter-Strike 2 > Properties > Betas > choose \"csgo_legacy\", \
                 wait for the download, then start CSGO Rewrite again.\n\n\
                 If CS:GO Legacy is somewhere else, click OK and select its csgo.exe.",
                found.cs2_only[0].display()
            )
        };
        message("CSGO Rewrite setup", &text);
    }
    loop {
        let picked = pick_csgo_exe(start.as_deref())?;
        match csgo_game_dir(&picked) {
            Ok(dir) => return Some(dir),
            Err(e) => message("CSGO Rewrite setup", &e),
        }
    }
}

/// `csr find-csgo`: what the automatic search sees (for troubleshooting).
pub fn print_detected() {
    let found = detect();
    for g in &found.legacy {
        println!("CS:GO Legacy: {}", g.display());
    }
    for c in &found.cs2_only {
        println!("Counter-Strike 2 (not on csgo_legacy): {}", c.display());
    }
    if found.legacy.is_empty() && found.cs2_only.is_empty() {
        println!("No CS:GO install found in any Steam library.");
    }
}

/// Prepares the process before any command runs. `force_ask` re-runs the
/// CS:GO folder question (`csr setup`).
pub fn prepare(force_ask: bool) -> Result<(), String> {
    let portable = is_portable();
    let data = match std::env::var_os("CSR_DATA_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None if portable => exe_dir().map(|d| d.join("userdata")),
        // A source checkout keeps its data at the repository root.
        None => Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")).filter(|r| r.join("Cargo.toml").is_file()),
    };
    if let Some(data) = &data {
        std::fs::create_dir_all(data).map_err(|e| format!("Cannot create {}: {e}", data.display()))?;
        std::env::set_current_dir(data).map_err(|e| format!("Cannot use {}: {e}", data.display()))?;
    }
    let data = std::env::current_dir().map_err(|e| e.to_string())?;

    // Minecraft assets shipped with a release.
    if std::env::var_os("CSR_MINECRAFT_DIR").is_none() {
        if let Some(bundled) = exe_dir().map(|d| d.join("minecraft-26.3")).filter(|d| d.is_dir()) {
            // SAFETY: single-threaded start-up, before anything reads the environment.
            unsafe { std::env::set_var("CSR_MINECRAFT_DIR", bundled) };
        }
    }

    if std::env::var_os("CSR_GAME_DIR").is_some() && !force_ask {
        return Ok(());
    }
    let mut paths = read_paths(&data);
    let saved = paths.iter().find(|(k, _)| k == "csgo").map(|(_, v)| PathBuf::from(v));
    let game = match saved.as_deref().map(csgo_game_dir) {
        Some(Ok(dir)) if !force_ask => Some(dir),
        _ => {
            if !portable && !force_ask {
                // Source checkouts find the Steam install by themselves.
                vfs::find_csgo_dir().or_else(|| detect().legacy.into_iter().next())
            } else {
                let dir = ask_for_csgo(&detect()).ok_or("Setup cancelled: CSGO Rewrite needs your CS:GO Legacy files to run.")?;
                paths.retain(|(k, _)| k != "csgo");
                paths.push(("csgo".into(), dir.display().to_string()));
                write_paths(&data, &paths).map_err(|e| format!("Cannot save {PATHS_FILE}: {e}"))?;
                eprintln!("CS:GO Legacy: {}", dir.display());
                Some(dir)
            }
        }
    };
    if let Some(game) = game {
        // SAFETY: single-threaded start-up, before anything reads the environment.
        unsafe { std::env::set_var("CSR_GAME_DIR", game) };
    }
    Ok(())
}

/// Shows a start-up failure to a player who may not be watching a console.
pub fn report(error: &str) {
    eprintln!("error: {error}");
    if is_portable() {
        message("CSGO Rewrite", error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("csr-setup-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn accepts_install_root_or_csgo_folder_and_rejects_cs2() {
        let root = temp("legacy");
        let game = root.join("csgo");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("gameinfo.txt"), "").unwrap();
        std::fs::write(game.join("pak01_dir.vpk"), "").unwrap();
        assert_eq!(csgo_game_dir(&root).unwrap(), game);
        assert_eq!(csgo_game_dir(&game).unwrap(), game);
        assert_eq!(csgo_game_dir(&game.join("pak01_dir.vpk")).unwrap(), game);

        let cs2 = temp("cs2");
        std::fs::create_dir_all(cs2.join("game/csgo")).unwrap();
        std::fs::write(cs2.join("game/csgo/gameinfo.gi"), "").unwrap();
        assert!(csgo_game_dir(&cs2).unwrap_err().contains("Counter-Strike 2"));
        assert!(csgo_game_dir(&temp("empty")).is_err());
    }

    #[test]
    fn finds_legacy_in_listed_libraries_and_flags_cs2() {
        // Steam on one "drive" lists a second library holding CS:GO Legacy
        // and a third holding an un-switched Counter-Strike 2.
        let steam = temp("steam");
        let legacy_lib = temp("lib-legacy");
        let cs2_lib = temp("lib-cs2");
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        let vdf = format!(
            "\"libraryfolders\"\n{{\n\"0\" {{ \"path\" \"{}\" }}\n\"1\" {{ \"path\" \"{}\" }}\n}}\n",
            legacy_lib.display().to_string().replace('\\', "\\\\"),
            cs2_lib.display().to_string().replace('\\', "\\\\")
        );
        std::fs::write(steam.join(r"steamapps\libraryfolders.vdf"), vdf).unwrap();
        let game = legacy_lib.join(r"steamapps\common\Counter-Strike Global Offensive\csgo");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("gameinfo.txt"), "").unwrap();
        std::fs::write(game.join("pak01_dir.vpk"), "").unwrap();
        let cs2 = cs2_lib.join(r"steamapps\common\Counter-Strike Global Offensive");
        std::fs::create_dir_all(cs2.join(r"game\csgo")).unwrap();
        std::fs::write(cs2.join(r"game\csgo\gameinfo.gi"), "").unwrap();

        let found = detect_in(libraries_from(vec![steam]));
        assert_eq!(found.legacy, vec![game]);
        assert_eq!(found.cs2_only, vec![cs2]);
    }

    #[test]
    fn paths_file_round_trips() {
        let data = temp("paths");
        write_paths(&data, &[("csgo".into(), r"D:\Games\CS:GO\csgo".into())]).unwrap();
        assert_eq!(read_paths(&data), vec![("csgo".to_string(), r"D:\Games\CS:GO\csgo".to_string())]);
    }
}
