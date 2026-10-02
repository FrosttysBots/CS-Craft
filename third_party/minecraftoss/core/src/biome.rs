//! The biome registry, loaded from a data pack's `worldgen/biome` entries.

use crate::datapack::DataPack;
use crate::ident::Identifier;
use serde_json::Value;
use std::collections::HashMap;

/// A dense biome index in sorted identifier order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BiomeId(pub u16);

#[derive(Clone, Debug)]
pub struct BiomeInfo {
    pub name: Identifier,
    pub temperature: f32,
    pub downfall: f32,
    pub has_precipitation: bool,
    /// `temperature_modifier: "frozen"` (vanilla `Biome.TemperatureModifier.FROZEN`).
    pub frozen_temperature_modifier: bool,
    /// Raw `effects` and `attributes` objects for presentation code.
    pub effects: Value,
    pub attributes: Value,
}

#[derive(Clone, Debug)]
pub struct BiomeRegistry {
    entries: Vec<BiomeInfo>,
    by_name: HashMap<Identifier, BiomeId>,
}

impl BiomeRegistry {
    pub fn load(pack: &DataPack) -> Result<Self, String> {
        let mut entries = Vec::new();
        let mut by_name = HashMap::new();
        for name in pack.list("worldgen/biome")? {
            let json = pack.read_json("worldgen/biome", &name)?;
            let number = |key: &str| {
                json[key]
                    .as_f64()
                    .map(|v| v as f32)
                    .ok_or_else(|| format!("biome {name} lacks {key}"))
            };
            let info = BiomeInfo {
                temperature: number("temperature")?,
                downfall: number("downfall")?,
                has_precipitation: json["has_precipitation"]
                    .as_bool()
                    .ok_or_else(|| format!("biome {name} lacks has_precipitation"))?,
                frozen_temperature_modifier: match json.get("temperature_modifier").and_then(Value::as_str) {
                    None | Some("none") => false,
                    Some("frozen") => true,
                    Some(other) => return Err(format!("biome {name} has unknown temperature_modifier {other}")),
                },
                effects: json.get("effects").cloned().unwrap_or(Value::Null),
                attributes: json.get("attributes").cloned().unwrap_or(Value::Null),
                name: name.clone(),
            };
            let id = BiomeId(u16::try_from(entries.len()).map_err(|_| "too many biomes")?);
            by_name.insert(name, id);
            entries.push(info);
        }
        if entries.is_empty() {
            return Err("data pack defines no biomes".into());
        }
        Ok(Self { entries, by_name })
    }

    pub fn get(&self, id: BiomeId) -> &BiomeInfo {
        &self.entries[usize::from(id.0)]
    }

    pub fn id(&self, name: &str) -> Option<BiomeId> {
        self.by_name.get(&Identifier::parse(name).ok()?).copied()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (BiomeId, &BiomeInfo)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, b)| (BiomeId(i as u16), b))
    }
}
