//! Scratch folders for tests, removed when dropped.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

pub(crate) struct TestDir(PathBuf);

impl TestDir {
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A new empty folder under the system's temporary folder.
pub(crate) fn tempdir() -> std::io::Result<TestDir> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "minecraft_terrain-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path)?;
    Ok(TestDir(path))
}
