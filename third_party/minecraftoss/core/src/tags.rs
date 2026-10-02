//! Data-pack tags resolved to bit sets over a registry's numeric IDs.
//!
//! Follows vanilla `TagLoader`: entries are element IDs or `#tag` references,
//! either may be `{"id": ..., "required": false}`, and nested references are
//! expanded with cycle detection. Missing required entries are errors.

use crate::datapack::DataPack;
use crate::ident::Identifier;
use serde_json::Value;
use std::collections::HashMap;

/// A resolved tag index; look up once and reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TagId(u32);

#[derive(Clone, Debug)]
struct BitSet(Vec<u64>);

impl BitSet {
    fn new(len: usize) -> Self {
        Self(vec![0; len.div_ceil(64)])
    }
    fn insert(&mut self, i: usize) {
        self.0[i / 64] |= 1 << (i % 64);
    }
    fn contains(&self, i: usize) -> bool {
        self.0.get(i / 64).is_some_and(|w| w & (1 << (i % 64)) != 0)
    }
    fn union(&mut self, other: &Self) {
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            *a |= b;
        }
    }
}

/// All tags of one registry (e.g. `block` or `worldgen/biome`).
#[derive(Clone, Debug)]
pub struct Tags {
    ids: HashMap<Identifier, TagId>,
    sets: Vec<BitSet>,
}

enum Visit {
    InProgress,
    Done(BitSet),
}

impl Tags {
    /// Loads `data/*/tags/<registry>/**.json`. `resolve` maps element IDs to dense indices below `len`.
    pub fn load(
        pack: &DataPack,
        registry: &str,
        len: usize,
        resolve: impl Fn(&Identifier) -> Option<usize>,
    ) -> Result<Self, String> {
        let kind = format!("tags/{registry}");
        let names = pack.list(&kind)?;
        let mut raw = HashMap::new();
        for name in &names {
            raw.insert(name.clone(), pack.read_json(&kind, name)?);
        }
        let mut visits: HashMap<Identifier, Visit> = HashMap::new();
        for name in &names {
            resolve_tag(name, &raw, &mut visits, len, &resolve)?;
        }
        let mut ids = HashMap::new();
        let mut sets = Vec::new();
        for name in names {
            let Some(Visit::Done(set)) = visits.remove(&name) else {
                unreachable!("every listed tag resolved")
            };
            ids.insert(name, TagId(sets.len() as u32));
            sets.push(set);
        }
        Ok(Self { ids, sets })
    }

    pub fn id(&self, name: &str) -> Option<TagId> {
        self.ids
            .get(&Identifier::parse(name.strip_prefix('#').unwrap_or(name)).ok()?)
            .copied()
    }

    /// Like `id`, but errors when the tag is missing.
    pub fn require(&self, name: &str) -> Result<TagId, String> {
        self.id(name).ok_or_else(|| format!("missing tag {name}"))
    }

    pub fn contains(&self, tag: TagId, element: usize) -> bool {
        self.sets[tag.0 as usize].contains(element)
    }

    pub fn len(&self) -> usize {
        self.sets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }
}

fn resolve_tag(
    name: &Identifier,
    raw: &HashMap<Identifier, Value>,
    visits: &mut HashMap<Identifier, Visit>,
    len: usize,
    resolve: &impl Fn(&Identifier) -> Option<usize>,
) -> Result<(), String> {
    match visits.get(name) {
        Some(Visit::Done(_)) => return Ok(()),
        Some(Visit::InProgress) => return Err(format!("tag cycle through #{name}")),
        None => {}
    }
    visits.insert(name.clone(), Visit::InProgress);
    let json = raw
        .get(name)
        .ok_or_else(|| format!("missing tag #{name}"))?;
    let values = json["values"]
        .as_array()
        .ok_or_else(|| format!("tag #{name} has no values array"))?;
    let mut set = BitSet::new(len);
    for entry in values {
        let (text, required) = match entry {
            Value::String(s) => (s.as_str(), true),
            Value::Object(o) => (
                o.get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("tag #{name} entry lacks id"))?,
                o.get("required").and_then(Value::as_bool).unwrap_or(true),
            ),
            other => return Err(format!("tag #{name} has invalid entry {other}")),
        };
        if let Some(reference) = text.strip_prefix('#') {
            let reference = Identifier::parse(reference)?;
            if !raw.contains_key(&reference) {
                if required {
                    return Err(format!("tag #{name} references missing #{reference}"));
                }
                continue;
            }
            resolve_tag(&reference, raw, visits, len, resolve)?;
            let Some(Visit::Done(other)) = visits.get(&reference) else {
                unreachable!("resolved above")
            };
            set.union(&other.clone());
        } else {
            match resolve(&Identifier::parse(text)?) {
                Some(index) => set.insert(index),
                None if required => return Err(format!("tag #{name} references unknown {text}")),
                None => {}
            }
        }
    }
    visits.insert(name.clone(), Visit::Done(set));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_with(files: &[(&str, &str)]) -> (std::path::PathBuf, DataPack) {
        let dir =
            std::env::temp_dir().join(format!("mcoss-tags-{}-{}", std::process::id(), files.len()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, body) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        let pack = DataPack::open(&dir).unwrap();
        (dir, pack)
    }

    #[test]
    fn nested_optional_and_cyclic_tags() {
        let (dir, pack) = pack_with(&[
            (
                "data/minecraft/tags/block/logs.json",
                r##"{"values":["minecraft:oak_log","#minecraft:stems",{"id":"mod:x","required":false}]}"##,
            ),
            (
                "data/minecraft/tags/block/stems.json",
                r#"{"values":["minecraft:crimson_stem"]}"#,
            ),
        ]);
        let names = [
            "minecraft:oak_log",
            "minecraft:crimson_stem",
            "minecraft:stone",
        ];
        let tags = Tags::load(&pack, "block", 3, |id| {
            names.iter().position(|n| *n == id.as_str())
        })
        .unwrap();
        let logs = tags.require("#minecraft:logs").unwrap();
        assert!(tags.contains(logs, 0) && tags.contains(logs, 1) && !tags.contains(logs, 2));
        std::fs::remove_dir_all(dir).unwrap();

        let (dir, pack) = pack_with(&[
            (
                "data/minecraft/tags/block/a.json",
                r##"{"values":["#minecraft:b"]}"##,
            ),
            (
                "data/minecraft/tags/block/b.json",
                r##"{"values":["#minecraft:a"]}"##,
            ),
            (
                "data/minecraft/tags/block/c.json",
                r##"{"values":["#minecraft:a"]}"##,
            ),
        ]);
        assert!(
            Tags::load(&pack, "block", 1, |_| Some(0))
                .unwrap_err()
                .contains("cycle")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
