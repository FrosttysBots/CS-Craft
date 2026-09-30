//! Map loading shared by the headless simulation and the renderer.

use collision::{CollisionWorld, MapCollisionOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

pub struct Level {
    pub name: String,
    pub game_dir: PathBuf,
    pub vfs: vfs::Vfs,
    pub bsp: bsp::Bsp,
    pub collision: Arc<CollisionWorld>,
    /// Weapon catalog from `scripts/items/items_game.txt`.
    pub catalog: Arc<weapons::Catalog>,
    /// Navigation mesh from `maps/<name>.nav`, if the map ships one.
    pub nav: Option<Arc<nav::NavMesh>>,
    pub timings: Vec<(&'static str, f32)>,
}

#[derive(Debug)]
pub enum LoadError {
    GameNotFound,
    Io(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::GameNotFound => write!(
                f,
                "CS:GO install not found. Install the csgo_legacy branch or set CSR_GAME_DIR to the folder holding gameinfo.txt"
            ),
            LoadError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Mounts the game's file system (no map).
pub fn mount_game(game_dir: Option<&Path>) -> Result<(PathBuf, vfs::Vfs), LoadError> {
    let game_dir = match game_dir {
        Some(d) => d.to_path_buf(),
        None => vfs::find_csgo_dir().ok_or(LoadError::GameNotFound)?,
    };
    let mut fs = vfs::Vfs::new();
    fs.mount_game_dir(&game_dir).map_err(|e| LoadError::Io(format!("mounting {}: {e}", game_dir.display())))?;
    Ok((game_dir, fs))
}

impl Level {
    /// Loads `maps/<name>.bsp` and builds its collision world.
    pub fn load(name: &str, game_dir: Option<&Path>) -> Result<Level, LoadError> {
        let mut timings = Vec::new();
        let t = Instant::now();
        let (game_dir, mut fs) = mount_game(game_dir)?;
        timings.push(("mount", t.elapsed().as_secs_f32()));

        let t = Instant::now();
        let map_path = format!("maps/{name}.bsp");
        let bytes = fs.read(&map_path).ok_or_else(|| LoadError::Io(format!("{map_path} not found")))?;
        let bsp = bsp::Bsp::parse(&bytes).map_err(|e| LoadError::Io(format!("{map_path}: {e}")))?;
        drop(bytes);
        fs.mount_map_pakfile(bsp.pakfile.clone())
            .map_err(|e| LoadError::Io(format!("pakfile: {e}")))?;
        timings.push(("bsp", t.elapsed().as_secs_f32()));

        let t = Instant::now();
        let loader = |model: &str| -> Option<Vec<mdl::ConvexHull>> {
            let base = model.trim_end_matches(".mdl");
            let bytes = fs.read(&format!("{base}.phy"))?;
            mdl::Phy::parse(&bytes).ok().map(|p| p.hulls)
        };
        let collision = CollisionWorld::from_bsp(&bsp, &MapCollisionOptions::default(), &loader);
        timings.push(("collision", t.elapsed().as_secs_f32()));

        let t = Instant::now();
        let catalog = fs
            .read_string("scripts/items/items_game.txt")
            .and_then(|text| weapons::Catalog::from_items_game(&text).ok())
            .unwrap_or_default();
        timings.push(("weapons", t.elapsed().as_secs_f32()));

        let nav = fs
            .read(&format!("maps/{name}.nav"))
            .and_then(|bytes| nav::NavMesh::parse(&bytes).ok())
            .map(Arc::new);

        Ok(Level {
            name: name.to_string(),
            game_dir,
            vfs: fs,
            bsp,
            collision: Arc::new(collision),
            catalog: Arc::new(catalog),
            nav,
            timings,
        })
    }
}
