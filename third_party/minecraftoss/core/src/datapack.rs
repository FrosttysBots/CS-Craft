//! Read access to an unpacked data pack directory (`data/<namespace>/...`).

use crate::ident::Identifier;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// One unpacked data pack. Vanilla's built-in pack is imported to
/// `datapacks/local/minecraft-26.3/` by `tools/import_datapack.py`.
#[derive(Clone, Debug)]
pub struct DataPack {
    root: PathBuf,
}

impl DataPack {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root = root.into();
        if !root.join("data").is_dir() {
            return Err(format!(
                "not a data pack (no data/ directory): {}",
                root.display()
            ));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, kind: &str, id: &Identifier) -> PathBuf {
        self.root
            .join("data")
            .join(id.namespace())
            .join(kind)
            .join(format!("{}.json", id.path()))
    }

    /// Reads `data/<namespace>/<kind>/<path>.json`.
    pub fn read_json(&self, kind: &str, id: &Identifier) -> Result<Value, String> {
        let path = self.path(kind, id);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn contains(&self, kind: &str, id: &Identifier) -> bool {
        self.path(kind, id).is_file()
    }

    /// Every JSON entry of one registry directory across namespaces, sorted by identifier.
    pub fn list(&self, kind: &str) -> Result<Vec<Identifier>, String> {
        let mut ids = Vec::new();
        let data = self.root.join("data");
        for namespace in std::fs::read_dir(&data).map_err(|e| format!("{}: {e}", data.display()))? {
            let namespace = namespace.map_err(|e| e.to_string())?;
            let base = namespace.path().join(kind);
            if !base.is_dir() {
                continue;
            }
            let ns = namespace.file_name().to_string_lossy().into_owned();
            collect(&base, &base, &ns, &mut ids)?;
        }
        ids.sort();
        Ok(ids)
    }
}

fn collect(
    base: &Path,
    dir: &Path,
    namespace: &str,
    out: &mut Vec<Identifier>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect(base, &path, namespace, out)?;
        } else if path.extension().is_some_and(|e| e == "json") {
            let relative = path
                .strip_prefix(base)
                .expect("walked under base")
                .with_extension("");
            let relative = relative.to_string_lossy().replace('\\', "/");
            out.push(Identifier::parse(&format!("{namespace}:{relative}"))?);
        }
    }
    Ok(())
}
