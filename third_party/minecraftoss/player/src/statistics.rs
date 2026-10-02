//! Vanilla 26.3 statistic categories, values, formatting, and save shape.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

pub const CUSTOM: &str = "minecraft:custom";
pub const MINED: &str = "minecraft:mined";
pub const CRAFTED: &str = "minecraft:crafted";
pub const USED: &str = "minecraft:used";
pub const BROKEN: &str = "minecraft:broken";
pub const PICKED_UP: &str = "minecraft:picked_up";
pub const DROPPED: &str = "minecraft:dropped";
pub const KILLED: &str = "minecraft:killed";
pub const KILLED_BY: &str = "minecraft:killed_by";
pub const DATA_VERSION: i32 = 5023;

/// The custom registry entries in the pinned 26.3 `Stats` class.
pub const CUSTOM_IDS: &[&str] = &[
    "leave_game",
    "play_time",
    "total_world_time",
    "time_since_death",
    "time_since_rest",
    "sneak_time",
    "walk_one_cm",
    "crouch_one_cm",
    "sprint_one_cm",
    "walk_on_water_one_cm",
    "fall_one_cm",
    "climb_one_cm",
    "fly_one_cm",
    "walk_under_water_one_cm",
    "minecart_one_cm",
    "boat_one_cm",
    "pig_one_cm",
    "happy_ghast_one_cm",
    "horse_one_cm",
    "aviate_one_cm",
    "swim_one_cm",
    "strider_one_cm",
    "nautilus_one_cm",
    "jump",
    "drop",
    "damage_dealt",
    "damage_dealt_absorbed",
    "damage_dealt_resisted",
    "damage_taken",
    "damage_blocked_by_shield",
    "damage_absorbed",
    "damage_resisted",
    "deaths",
    "mob_kills",
    "animals_bred",
    "player_kills",
    "fish_caught",
    "talked_to_villager",
    "traded_with_villager",
    "eat_cake_slice",
    "fill_cauldron",
    "use_cauldron",
    "clean_armor",
    "clean_banner",
    "clean_shulker_box",
    "interact_with_brewingstand",
    "interact_with_beacon",
    "inspect_dropper",
    "inspect_hopper",
    "inspect_dispenser",
    "play_noteblock",
    "tune_noteblock",
    "pot_flower",
    "trigger_trapped_chest",
    "open_enderchest",
    "enchant_item",
    "play_record",
    "interact_with_furnace",
    "interact_with_crafting_table",
    "open_chest",
    "sleep_in_bed",
    "sleep_in_straw_bed",
    "open_shulker_box",
    "open_barrel",
    "interact_with_blast_furnace",
    "interact_with_smoker",
    "interact_with_lectern",
    "interact_with_campfire",
    "interact_with_cartography_table",
    "interact_with_loom",
    "interact_with_stonecutter",
    "bell_ring",
    "raid_trigger",
    "raid_win",
    "interact_with_anvil",
    "interact_with_grindstone",
    "target_hit",
    "interact_with_smithing_table",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Statistics {
    #[serde(default)]
    pub stats: BTreeMap<String, BTreeMap<String, i32>>,
    #[serde(rename = "DataVersion", default = "data_version")]
    pub data_version: i32,
}

const fn data_version() -> i32 {
    DATA_VERSION
}

impl Default for Statistics {
    fn default() -> Self {
        Self {
            stats: BTreeMap::new(),
            data_version: DATA_VERSION,
        }
    }
}

impl Statistics {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("tmp");
        let mut current = self.clone();
        current.data_version = DATA_VERSION;
        std::fs::write(&temp, serde_json::to_vec_pretty(&current)?)?;
        std::fs::rename(temp, path)?;
        Ok(())
    }

    pub fn get(&self, category: &str, id: &str) -> i32 {
        self.stats
            .get(category)
            .and_then(|values| values.get(id))
            .copied()
            .unwrap_or(0)
    }

    pub fn add(&mut self, category: &str, id: &str, amount: i32) -> bool {
        if amount <= 0 {
            return false;
        }
        let value = self
            .stats
            .entry(category.to_owned())
            .or_default()
            .entry(id.to_owned())
            .or_default();
        let next = (*value as i64 + amount as i64).min(i32::MAX as i64) as i32;
        let changed = next != *value;
        *value = next;
        changed
    }

    pub fn custom(&self, id: &str) -> i32 {
        self.get(CUSTOM, &format!("minecraft:{id}"))
    }

    pub fn add_custom(&mut self, id: &str, amount: i32) -> bool {
        self.add(CUSTOM, &format!("minecraft:{id}"), amount)
    }

    pub fn item_ids(&self) -> Vec<String> {
        let mut ids = std::collections::BTreeSet::new();
        for category in [MINED, BROKEN, CRAFTED, USED, PICKED_UP, DROPPED] {
            if let Some(values) = self.stats.get(category) {
                ids.extend(
                    values
                        .iter()
                        .filter(|(_, count)| **count > 0)
                        .map(|(id, _)| id.clone()),
                );
            }
        }
        ids.remove("minecraft:air");
        ids.into_iter().collect()
    }

    pub fn mob_ids(&self) -> Vec<String> {
        let mut ids = std::collections::BTreeSet::new();
        for category in [KILLED, KILLED_BY] {
            if let Some(values) = self.stats.get(category) {
                ids.extend(
                    values
                        .iter()
                        .filter(|(_, count)| **count > 0)
                        .map(|(id, _)| id.clone()),
                );
            }
        }
        ids.into_iter().collect()
    }
}

pub fn format_custom(id: &str, value: i32) -> String {
    if id.ends_with("_one_cm") {
        let meters = value as f64 / 100.0;
        let kilometers = meters / 1000.0;
        if kilometers > 0.5 {
            format!("{kilometers:.2} km")
        } else if meters > 0.5 {
            format!("{meters:.2} m")
        } else {
            format!("{value} cm")
        }
    } else if matches!(
        id,
        "play_time" | "total_world_time" | "time_since_death" | "time_since_rest" | "sneak_time"
    ) {
        let seconds = value as f64 / 20.0;
        let minutes = seconds / 60.0;
        let hours = minutes / 60.0;
        let days = hours / 24.0;
        let years = days / 365.0;
        if years > 0.5 {
            format!("{years:.2} y")
        } else if days > 0.5 {
            format!("{days:.2} d")
        } else if hours > 0.5 {
            format!("{hours:.2} h")
        } else if minutes > 0.5 {
            format!("{minutes:.2} min")
        } else {
            let mut text = seconds.to_string();
            if !text.contains('.') {
                text.push_str(".0");
            }
            format!("{text} s")
        }
    } else if matches!(
        id,
        "damage_dealt"
            | "damage_dealt_absorbed"
            | "damage_dealt_resisted"
            | "damage_taken"
            | "damage_blocked_by_shield"
            | "damage_absorbed"
            | "damage_resisted"
    ) {
        format!("{:.2}", value as f64 / 10.0)
    } else {
        let digits = value.to_string();
        let mut formatted = String::new();
        for (index, digit) in digits.chars().enumerate() {
            if index > 0 && (digits.len() - index) % 3 == 0 {
                formatted.push(',');
            }
            formatted.push(digit);
        }
        formatted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_shape_caps_values_and_round_trips() {
        let mut stats = Statistics::default();
        stats.add(MINED, "minecraft:stone", 4);
        stats.add(CUSTOM, "minecraft:play_time", i32::MAX);
        stats.add_custom("play_time", 1);
        assert_eq!(stats.custom("play_time"), i32::MAX);
        let value = serde_json::to_value(&stats).unwrap();
        assert_eq!(value["DataVersion"], 5023);
        assert_eq!(value["stats"]["minecraft:mined"]["minecraft:stone"], 4);
        assert_eq!(
            serde_json::from_value::<Statistics>(value)
                .unwrap()
                .item_ids(),
            ["minecraft:stone"]
        );
        let path = std::env::temp_dir().join(format!(
            "minecraftoss-statistics-{}.json",
            std::process::id()
        ));
        stats.save(&path).unwrap();
        stats.save(&path).unwrap();
        assert_eq!(
            Statistics::load(&path)
                .unwrap()
                .get(MINED, "minecraft:stone"),
            4
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn source_formatter_thresholds() {
        assert_eq!(format_custom("walk_one_cm", 50), "50 cm");
        assert_eq!(format_custom("walk_one_cm", 51), "0.51 m");
        assert_eq!(format_custom("play_time", 20), "1.0 s");
        assert_eq!(format_custom("jump", 12345), "12,345");
    }
}
