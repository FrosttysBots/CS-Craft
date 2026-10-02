//! Default item properties the simulation needs, from the sparse 26.3 item
//! catalog (`harness/export_item_catalog.py`): absent items stack to 64 and
//! are not fuel.

use crate::nbt::Tag;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct ItemCatalog {
    max_stack: HashMap<String, i32>,
    fuel: HashMap<String, u32>,
}

impl ItemCatalog {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let root: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if root["minecraft_version"] != "26.3" {
            return Err(format!("{}: not a 26.3 item catalog", path.display()));
        }
        let items = root["items"].as_object().ok_or("item catalog without items")?;
        let mut catalog = Self::default();
        for (id, item) in items {
            if let Some(max) = item["max_stack"].as_i64() {
                catalog.max_stack.insert(id.clone(), max as i32);
            }
            if item["fuel_component"].as_bool() == Some(true) || item["burn_ticks"].as_u64().is_some_and(|t| t > 0) {
                catalog.fuel.insert(id.clone(), item["burn_ticks"].as_u64().unwrap_or(0) as u32);
            }
        }
        Ok(catalog)
    }

    /// The item's default `max_stack_size` component.
    pub fn max_stack(&self, id: &str) -> i32 {
        self.max_stack.get(id).copied().unwrap_or(64)
    }

    /// `FuelValues.isFuel` for the default stack.
    pub fn is_fuel(&self, id: &str) -> bool {
        self.fuel.contains_key(id)
    }
}

/// An item stack: id, count (0 or less is empty) and component patch.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemStack {
    pub id: String,
    pub count: i32,
    pub components: Option<Tag>,
}

impl ItemStack {
    pub fn empty() -> Self {
        Self { id: "minecraft:air".to_owned(), count: 0, components: None }
    }

    pub fn new(id: &str, count: i32) -> Self {
        Self { id: id.to_owned(), count, components: None }
    }

    pub fn is_empty(&self) -> bool {
        self.count <= 0 || self.id == "minecraft:air"
    }

    /// `ItemStack.isSameItemSameComponents`.
    pub fn same_item_same_components(&self, other: &ItemStack) -> bool {
        self.id == other.id && self.components == other.components
    }

    pub fn from_tag(tag: &Tag) -> Option<(usize, ItemStack)> {
        let slot = tag.get("Slot")?.as_i64()?;
        let id = tag.get("id")?.as_str()?.to_owned();
        let count = tag.get("count").and_then(Tag::as_i64).unwrap_or(1) as i32;
        let components = tag.get("components").filter(|c| c.as_compound().is_some_and(|m| !m.is_empty())).cloned();
        (slot >= 0).then_some((slot as usize, ItemStack { id, count, components }))
    }

    pub fn to_tag(&self, slot: usize) -> Tag {
        let mut map = BTreeMap::new();
        map.insert("Slot".to_owned(), Tag::Byte(slot as i8));
        map.insert("id".to_owned(), Tag::String(self.id.clone()));
        map.insert("count".to_owned(), Tag::Int(self.count));
        if let Some(components) = &self.components {
            map.insert("components".to_owned(), components.clone());
        }
        Tag::Compound(map)
    }
}

