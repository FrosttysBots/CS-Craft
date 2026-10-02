//! A world's generation settings (`WorldGenSettings`), saved as
//! `data/minecraft/world_gen_settings.dat` in the 26.3 layout.

use minecraftoss_core::chunk_nbt::DATA_VERSION;
use minecraftoss_core::nbt::{self, Tag};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldSettings {
    pub seed: i64,
    pub generate_structures: bool,
    pub bonus_chest: bool,
}

fn path(world: &Path) -> PathBuf {
    world.join("data").join("minecraft").join("world_gen_settings.dat")
}

fn compound(entries: Vec<(&str, Tag)>) -> Tag {
    Tag::Compound(entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect::<BTreeMap<_, _>>())
}

fn string(s: &str) -> Tag {
    Tag::String(s.to_owned())
}

/// The default dimensions: noise generators with each vanilla preset.
fn dimensions() -> Tag {
    let noise = |settings: &str, source: Tag, kind: &str| {
        compound(vec![("generator", compound(vec![("settings", string(settings)), ("biome_source", source), ("type", string("minecraft:noise"))])), ("type", string(kind))])
    };
    let multi = |preset: &str| compound(vec![("preset", string(preset)), ("type", string("minecraft:multi_noise"))]);
    compound(vec![
        ("minecraft:overworld", noise("minecraft:overworld", multi("minecraft:overworld"), "minecraft:overworld")),
        ("minecraft:the_nether", noise("minecraft:nether", multi("minecraft:nether"), "minecraft:the_nether")),
        ("minecraft:the_end", noise("minecraft:end", compound(vec![("type", string("minecraft:the_end"))]), "minecraft:the_end")),
    ])
}

impl WorldSettings {
    pub fn new(seed: i64) -> Self {
        Self { seed, generate_structures: true, bonus_chest: false }
    }

    /// The saved settings, or `None` for a new world.
    pub fn read(world: &Path) -> Result<Option<Self>, String> {
        let path = path(world);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let tag = nbt::read(&bytes)?;
        let data = tag.get("data").ok_or("world_gen_settings lacks data")?;
        let flag = |key: &str, default: bool| data.get(key).and_then(Tag::as_i64).map_or(default, |v| v != 0);
        Ok(Some(Self {
            seed: data.get("seed").and_then(Tag::as_i64).ok_or("world_gen_settings lacks a seed")?,
            generate_structures: flag("generate_structures", true),
            bonus_chest: flag("bonus_chest", false),
        }))
    }

    pub fn write(&self, world: &Path) -> Result<(), String> {
        let path = path(world);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let data = compound(vec![
            ("bonus_chest", Tag::Byte(self.bonus_chest as i8)),
            ("seed", Tag::Long(self.seed)),
            ("generate_structures", Tag::Byte(self.generate_structures as i8)),
            ("dimensions", dimensions()),
        ]);
        let root = compound(vec![("data", data), ("DataVersion", Tag::Int(DATA_VERSION))]);
        std::fs::write(&path, nbt::write_gzip(&root, "")).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let dir = std::env::temp_dir().join(format!("minecraftoss-settings-{}", std::process::id()));
        let settings = WorldSettings::new(-42);
        settings.write(&dir).unwrap();
        assert_eq!(WorldSettings::read(&dir).unwrap(), Some(settings));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
