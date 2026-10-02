//! Data-driven advancement definitions and per-criterion player progress.
use crate::inventory::ItemStack;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
};
use zip::ZipArchive;

#[derive(Clone, Debug)]
pub struct Criterion {
    pub trigger: String,
    pub conditions: Value,
}

#[derive(Clone, Debug)]
pub struct Display {
    pub title: Value,
    pub description: Value,
    pub icon: String,
    pub frame: String,
    pub background: Option<String>,
    pub show_toast: bool,
    pub announce_to_chat: bool,
    pub hidden: bool,
}

#[derive(Clone, Debug)]
pub struct Advancement {
    pub id: String,
    pub parent: Option<String>,
    pub criteria: BTreeMap<String, Criterion>,
    /// One criterion from each inner group is required.
    pub requirements: Vec<Vec<String>>,
    pub display: Option<Display>,
    pub reward_recipes: Vec<String>,
    pub reward_experience: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub entries: BTreeMap<String, Advancement>,
    item_tags: HashMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Progress {
    /// Millisecond award times, retaining criterion identity across reloads.
    #[serde(default)]
    pub criteria: BTreeMap<String, BTreeMap<String, u64>>,
}

#[derive(Clone, Debug)]
pub struct Award {
    pub advancement: String,
    pub completed: bool,
    pub reward_recipes: Vec<String>,
    pub reward_experience: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Event<'a> {
    pub trigger: &'a str,
    pub item: Option<&'a str>,
    pub block: Option<&'a str>,
    pub recipe: Option<&'a str>,
    pub inventory: Option<&'a [Option<ItemStack>]>,
    pub occupied_slots: Option<usize>,
}

impl Catalog {
    pub fn from_jar(path: &Path) -> Result<Self> {
        let file =
            File::open(path).with_context(|| format!("open advancement JAR {}", path.display()))?;
        let mut archive = ZipArchive::new(file)?;
        let mut catalog = Self::default();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let name = entry.name().to_owned();
            if !name.ends_with(".json")
                || !(name.starts_with("data/minecraft/advancement/")
                    || name.starts_with("data/minecraft/tags/item/"))
            {
                continue;
            }
            let mut source = String::new();
            entry.read_to_string(&mut source)?;
            let value: Value = serde_json::from_str(&source)
                .with_context(|| format!("parse advancement data {name}"))?;
            if let Some(path) = name.strip_prefix("data/minecraft/tags/item/") {
                let tag = format!("minecraft:{}", path.trim_end_matches(".json"));
                let values = value["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().or_else(|| v["id"].as_str()))
                    .map(str::to_owned)
                    .collect();
                catalog.item_tags.insert(tag, values);
            } else if let Some(path) = name.strip_prefix("data/minecraft/advancement/") {
                let id = format!("minecraft:{}", path.trim_end_matches(".json"));
                if let Some(advancement) = Advancement::from_value(id.clone(), &value) {
                    catalog.entries.insert(id, advancement);
                }
            }
        }
        Ok(catalog)
    }

    pub fn item_matches(&self, predicate: &Value, item: &str) -> bool {
        let value = predicate
            .get("items")
            .or_else(|| predicate.get("item"))
            .unwrap_or(predicate);
        if let Some(name) = value.as_str() {
            if let Some(tag) = name.strip_prefix('#') {
                return self.tag_contains(tag, item, &mut HashSet::new());
            }
            return name == item;
        }
        if let Some(array) = value.as_array() {
            return array.iter().any(|part| self.item_matches(part, item));
        }
        false
    }

    fn tag_contains(&self, tag: &str, item: &str, seen: &mut HashSet<String>) -> bool {
        if !seen.insert(tag.to_owned()) {
            return false;
        }
        self.item_tags.get(tag).is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry == item
                    || entry
                        .strip_prefix('#')
                        .is_some_and(|nested| self.tag_contains(nested, item, seen))
            })
        })
    }

    pub fn roots(&self) -> Vec<&Advancement> {
        let mut roots = self
            .entries
            .values()
            .filter(|entry| entry.parent.is_none() && entry.display.is_some())
            .collect::<Vec<_>>();
        roots.sort_by_key(|entry| {
            let index = [
                "minecraft:story/root",
                "minecraft:nether/root",
                "minecraft:end/root",
                "minecraft:adventure/root",
                "minecraft:husbandry/root",
            ]
            .iter()
            .position(|id| *id == entry.id)
            .unwrap_or(usize::MAX);
            (index, entry.id.as_str())
        });
        roots
    }

    pub fn descendants(&self, id: &str) -> Vec<String> {
        let mut found = vec![id.to_owned()];
        let mut cursor = 0;
        while cursor < found.len() {
            let parent = found[cursor].clone();
            found.extend(
                self.entries
                    .values()
                    .filter(|entry| entry.parent.as_deref() == Some(&parent))
                    .map(|entry| entry.id.clone()),
            );
            cursor += 1;
        }
        found
    }

    pub fn ancestors(&self, id: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut current = Some(id);
        while let Some(id) = current {
            let Some(entry) = self.entries.get(id) else {
                break;
            };
            found.push(id.to_owned());
            current = entry.parent.as_deref();
        }
        found
    }
}

impl Advancement {
    fn from_value(id: String, value: &Value) -> Option<Self> {
        let criteria = value
            .get("criteria")?
            .as_object()?
            .iter()
            .filter_map(|(name, criterion)| {
                Some((
                    name.clone(),
                    Criterion {
                        trigger: criterion.get("trigger")?.as_str()?.to_owned(),
                        conditions: criterion.get("conditions").cloned().unwrap_or(Value::Null),
                    },
                ))
            })
            .collect::<BTreeMap<_, _>>();
        if criteria.is_empty() {
            return None;
        }
        let requirements = value
            .get("requirements")
            .and_then(Value::as_array)
            .map(|groups| {
                groups
                    .iter()
                    .map(|group| {
                        group
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_else(|| criteria.keys().map(|name| vec![name.clone()]).collect());
        let display = value.get("display").map(|v| Display {
            title: v.get("title").cloned().unwrap_or(Value::Null),
            description: v.get("description").cloned().unwrap_or(Value::Null),
            icon: v["icon"]["id"]
                .as_str()
                .unwrap_or("minecraft:stone")
                .to_owned(),
            frame: v["frame"].as_str().unwrap_or("task").to_owned(),
            background: v["background"].as_str().map(str::to_owned),
            show_toast: v["show_toast"].as_bool().unwrap_or(true),
            announce_to_chat: v["announce_to_chat"].as_bool().unwrap_or(true),
            hidden: v["hidden"].as_bool().unwrap_or(false),
        });
        Some(Self {
            id,
            parent: value["parent"].as_str().map(str::to_owned),
            criteria,
            requirements,
            display,
            reward_recipes: value["rewards"]["recipes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            reward_experience: value["rewards"]["experience"].as_u64().unwrap_or(0) as u32,
        })
    }
}

impl Progress {
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
        std::fs::write(&temp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(temp, path)?;
        Ok(())
    }

    pub fn has(&self, id: &str, criterion: &str) -> bool {
        self.criteria
            .get(id)
            .is_some_and(|entry| entry.contains_key(criterion))
    }

    pub fn done(&self, advancement: &Advancement) -> bool {
        advancement.requirements.iter().all(|group| {
            group
                .iter()
                .any(|criterion| self.has(&advancement.id, criterion))
        })
    }

    pub fn award(
        &mut self,
        advancement: &Advancement,
        criterion: &str,
        time_ms: u64,
    ) -> Option<Award> {
        if !advancement.criteria.contains_key(criterion) || self.has(&advancement.id, criterion) {
            return None;
        }
        let was_done = self.done(advancement);
        self.criteria
            .entry(advancement.id.clone())
            .or_default()
            .insert(criterion.to_owned(), time_ms);
        let completed = !was_done && self.done(advancement);
        Some(Award {
            advancement: advancement.id.clone(),
            completed,
            reward_recipes: if completed {
                advancement.reward_recipes.clone()
            } else {
                Vec::new()
            },
            reward_experience: if completed {
                advancement.reward_experience
            } else {
                0
            },
        })
    }

    pub fn revoke(&mut self, advancement: &Advancement, criterion: Option<&str>) -> bool {
        let Some(progress) = self.criteria.get_mut(&advancement.id) else {
            return false;
        };
        let changed = if let Some(name) = criterion {
            progress.remove(name).is_some()
        } else {
            !progress.is_empty()
        };
        if criterion.is_none() {
            progress.clear();
        }
        if progress.is_empty() {
            self.criteria.remove(&advancement.id);
        }
        changed
    }

    pub fn observe(&mut self, catalog: &Catalog, event: Event<'_>, time_ms: u64) -> Vec<Award> {
        let mut awards = Vec::new();
        for advancement in catalog.entries.values() {
            for (name, criterion) in &advancement.criteria {
                if criterion.trigger == event.trigger
                    && !self.has(&advancement.id, name)
                    && matches_event(catalog, &criterion.conditions, event)
                {
                    if let Some(award) = self.award(advancement, name, time_ms) {
                        awards.push(award);
                    }
                }
            }
        }
        awards
    }

    pub fn completed_ids(&self, catalog: &Catalog) -> BTreeSet<String> {
        catalog
            .entries
            .values()
            .filter(|entry| self.done(entry))
            .map(|entry| entry.id.clone())
            .collect()
    }
}

fn matches_event(catalog: &Catalog, conditions: &Value, event: Event<'_>) -> bool {
    match event.trigger {
        "minecraft:inventory_changed" => {
            if let Some(slots) = conditions.get("slots") {
                let Some(occupied) = event.occupied_slots else {
                    return false;
                };
                if let Some(rule) = slots.get("occupied") {
                    let min = rule.get("min").and_then(Value::as_u64).unwrap_or(0) as usize;
                    let max = rule
                        .get("max")
                        .and_then(Value::as_u64)
                        .unwrap_or(usize::MAX as u64) as usize;
                    if !(min..=max).contains(&occupied) {
                        return false;
                    }
                }
            }
            if let Some(items) = conditions.get("items") {
                let Some(item) = event.item else { return false };
                return items.as_array().is_some_and(|entries| {
                    if entries.len() == 1 {
                        return catalog.item_matches(&entries[0], item);
                    }
                    event.inventory.is_some_and(|inventory| {
                        entries.iter().all(|predicate| {
                            inventory
                                .iter()
                                .flatten()
                                .any(|stack| catalog.item_matches(predicate, &stack.id))
                        })
                    })
                });
            }
            true
        }
        "minecraft:consume_item" | "minecraft:filled_bucket" => {
            let Some(item) = event.item else { return false };
            conditions
                .get("item")
                .is_none_or(|predicate| catalog.item_matches(predicate, item))
        }
        "minecraft:placed_block" => {
            let Some(block) = event.block else {
                return false;
            };
            conditions.get("location").is_some_and(|location| {
                location["type"] == "minecraft:match_block"
                    && location["blocks"].as_str() == Some(block)
                    && location.get("state").is_none()
            })
        }
        "minecraft:enter_block" => event
            .block
            .is_some_and(|block| conditions["blocks"].as_str() == Some(block)),
        "minecraft:recipe_unlocked" => {
            let Some(recipe) = event.recipe else {
                return false;
            };
            conditions
                .get("recipes")
                .is_some_and(|predicate| predicate.as_str().is_some_and(|id| id == recipe))
        }
        "minecraft:tick" => {
            conditions.is_null() || conditions.as_object().is_some_and(|v| v.is_empty())
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requirements_are_and_of_or_groups_and_rewards_only_once() {
        let value = serde_json::json!({
            "criteria":{"a":{"trigger":"minecraft:inventory_changed"},"b":{"trigger":"minecraft:inventory_changed"},"c":{"trigger":"minecraft:inventory_changed"}},
            "requirements":[["a","b"],["c"]],
            "rewards":{"recipes":["minecraft:oak_planks"],"experience":10}
        });
        let advancement = Advancement::from_value("minecraft:test".into(), &value).unwrap();
        let mut progress = Progress::default();
        assert!(!progress.award(&advancement, "a", 1).unwrap().completed);
        let award = progress.award(&advancement, "c", 2).unwrap();
        assert!(award.completed);
        assert_eq!(award.reward_recipes, ["minecraft:oak_planks"]);
        assert_eq!(award.reward_experience, 10);
        assert!(progress.award(&advancement, "b", 3).is_some());
        assert!(progress.award(&advancement, "b", 4).is_none());
        assert!(progress.revoke(&advancement, Some("c")));
        assert!(!progress.done(&advancement));
    }

    #[test]
    fn progress_survives_repeated_saves_and_reload() {
        let value = serde_json::json!({"criteria":{"one":{"trigger":"minecraft:tick"}}});
        let advancement = Advancement::from_value("minecraft:test".into(), &value).unwrap();
        let mut progress = Progress::default();
        progress.award(&advancement, "one", 12345).unwrap();
        let path = std::env::temp_dir().join(format!(
            "minecraftoss-advancement-{}-{}.json",
            std::process::id(),
            12345
        ));
        progress.save(&path).unwrap();
        progress.save(&path).unwrap();
        let loaded = Progress::load(&path).unwrap();
        assert!(loaded.done(&advancement));
        assert_eq!(loaded.criteria["minecraft:test"]["one"], 12345);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn changed_item_and_recipe_events_unlock_matching_criteria() {
        let value = serde_json::json!({
            "criteria": {
                "item": {"trigger":"minecraft:inventory_changed","conditions":{"items":[{"items":"#minecraft:logs"}]}},
                "recipe": {"trigger":"minecraft:recipe_unlocked","conditions":{"recipes":"minecraft:oak_planks"}}
            },
            "requirements":[["item","recipe"]]
        });
        let mut catalog = Catalog::default();
        catalog
            .item_tags
            .insert("minecraft:logs".into(), vec!["minecraft:oak_log".into()]);
        catalog.entries.insert(
            "minecraft:test".into(),
            Advancement::from_value("minecraft:test".into(), &value).unwrap(),
        );
        let mut progress = Progress::default();
        let inventory = vec![Some(ItemStack::new("minecraft:oak_log", 1))];
        let awards = progress.observe(
            &catalog,
            Event {
                trigger: "minecraft:inventory_changed",
                item: Some("minecraft:oak_log"),
                block: None,
                recipe: None,
                inventory: Some(&inventory),
                occupied_slots: Some(1),
            },
            1,
        );
        assert_eq!(awards.len(), 1);
        assert!(awards[0].completed);
        assert!(progress
            .observe(
                &catalog,
                Event {
                    trigger: "minecraft:recipe_unlocked",
                    item: None,
                    block: None,
                    recipe: Some("minecraft:oak_planks"),
                    inventory: None,
                    occupied_slots: None,
                },
                2
            )
            .iter()
            .all(|award| !award.completed));
    }

    #[test]
    fn block_state_predicate_is_not_earned_from_block_id_alone() {
        let value = serde_json::json!({
            "criteria": {"awake": {
                "trigger": "minecraft:placed_block",
                "conditions": {"location": {
                    "type": "minecraft:match_block",
                    "blocks": "minecraft:creaking_heart",
                    "state": {"creaking_heart_state": "awake"}
                }}
            }}
        });
        let mut catalog = Catalog::default();
        catalog.entries.insert(
            "minecraft:test".into(),
            Advancement::from_value("minecraft:test".into(), &value).unwrap(),
        );
        let mut progress = Progress::default();
        let awards = progress.observe(
            &catalog,
            Event {
                trigger: "minecraft:placed_block",
                item: None,
                block: Some("minecraft:creaking_heart"),
                recipe: None,
                inventory: None,
                occupied_slots: None,
            },
            123,
        );
        assert!(awards.is_empty());
    }
}
