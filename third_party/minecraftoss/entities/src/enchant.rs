//! Enchantments from the pinned 26.3 data JAR (`data/minecraft/enchantment`,
//! their tags, the item tags they name and each item's `enchantable`
//! value), with `EnchantmentHelper.selectEnchantment` /
//! `getAvailableEnchantmentResults` / `enchantItem` and the loot functions
//! `enchant_randomly` and `enchant_with_levels`. The registry is in ID
//! order (data-pack registries load sorted by resource location); a tag
//! lists its values in file order with nested tags expanded in place.
use minecraftoss_player::rng::LootRandom;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// One enchantment's selection data.
#[derive(Clone, Debug)]
pub struct Enchantment {
    pub id: String,
    pub max_level: i32,
    pub weight: i32,
    /// `Enchantment.Cost`: base and per level above the first.
    min_cost: (i32, i32),
    max_cost: (i32, i32),
    supported: HashSet<String>,
    primary: Option<HashSet<String>>,
    exclusive: HashSet<String>,
}

impl Enchantment {
    pub fn min_cost(&self, level: i32) -> i32 {
        self.min_cost.0 + self.min_cost.1 * (level - 1)
    }

    pub fn max_cost(&self, level: i32) -> i32 {
        self.max_cost.0 + self.max_cost.1 * (level - 1)
    }

    /// `isPrimaryItem`: its primary items, or failing those its supported
    /// ones.
    fn primary_item(&self, item: &str) -> bool {
        self.primary.as_ref().unwrap_or(&self.supported).contains(item)
    }

    /// `canEnchant`: a supported item.
    pub fn can_enchant(&self, item: &str) -> bool {
        self.supported.contains(item)
    }
}

/// The enchantment registry and what selection reads.
#[derive(Clone, Debug, Default)]
pub struct Enchantments {
    /// In registry order.
    pub list: Vec<Enchantment>,
    tags: HashMap<String, Vec<String>>,
    /// `Enchantable.value` by item.
    enchantable: HashMap<String, i32>,
}

/// A tag's values in order, nested tags expanded (`required: false` entries
/// that name nothing are skipped).
pub fn resolve_tag(tags: &HashMap<String, Value>, id: &str, out: &mut Vec<String>, depth: usize) {
    let Some(tag) = tags.get(id) else { return };
    for value in tag["values"].as_array().into_iter().flatten() {
        let Some(name) = value.as_str().or_else(|| value["id"].as_str()) else { continue };
        match name.strip_prefix('#') {
            Some(nested) if depth < 16 => resolve_tag(tags, nested, out, depth + 1),
            Some(_) => {}
            None => {
                if !out.iter().any(|v| v == name) {
                    out.push(name.to_owned());
                }
            }
        }
    }
}

/// A holder set's members: a `#tag`, one ID, or a list of IDs.
fn holder_set(value: &Value, tags: &HashMap<String, Value>) -> Vec<String> {
    match value {
        Value::String(s) => match s.strip_prefix('#') {
            Some(tag) => {
                let mut out = Vec::new();
                resolve_tag(tags, tag, &mut out, 0);
                out
            }
            None => vec![s.clone()],
        },
        Value::Array(list) => list.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        _ => Vec::new(),
    }
}

fn cost(value: &Value) -> (i32, i32) {
    (value["base"].as_i64().unwrap_or(0) as i32, value["per_level_above_first"].as_i64().unwrap_or(0) as i32)
}

/// `Math.round(float)`.
pub fn java_round(value: f32) -> i32 {
    let bits = value.to_bits() as i32;
    let biased = (bits & 0x7f80_0000) >> 23;
    let shift = (23 - 1 + 127) - biased;
    if shift & -32 == 0 {
        let mut r = (bits & 0x007f_ffff) | 0x0080_0000;
        if bits < 0 {
            r = -r;
        }
        ((r >> shift) + 1) >> 1
    } else {
        value as i32
    }
}

impl Enchantments {
    /// From the JAR's enchantments (`id` → JSON), enchantment and item
    /// tags (`id` → tag JSON) and the items' `enchantable` values.
    pub fn new(mut definitions: Vec<(String, Value)>, enchantment_tags: &HashMap<String, Value>, item_tags: &HashMap<String, Value>, enchantable: HashMap<String, i32>) -> Self {
        definitions.sort_by(|a, b| a.0.cmp(&b.0));
        let list = definitions
            .into_iter()
            .map(|(id, json)| Enchantment {
                max_level: json["max_level"].as_i64().unwrap_or(1) as i32,
                weight: json["weight"].as_i64().unwrap_or(1) as i32,
                min_cost: cost(&json["min_cost"]),
                max_cost: cost(&json["max_cost"]),
                supported: holder_set(&json["supported_items"], item_tags).into_iter().collect(),
                primary: json.get("primary_items").map(|p| holder_set(p, item_tags).into_iter().collect()),
                exclusive: json.get("exclusive_set").map(|e| holder_set(e, enchantment_tags).into_iter().collect()).unwrap_or_default(),
                id,
            })
            .collect();
        let tags = enchantment_tags
            .keys()
            .map(|id| {
                let mut out = Vec::new();
                resolve_tag(enchantment_tags, id, &mut out, 0);
                (id.clone(), out)
            })
            .collect();
        Self { list, tags, enchantable }
    }

    pub fn index(&self, id: &str) -> Option<usize> {
        self.list.iter().position(|e| e.id == id)
    }

    /// A holder set of enchantments as registry indices, in its order; all
    /// of them in registry order for none.
    pub fn source(&self, options: Option<&Value>) -> Vec<usize> {
        match options {
            None => (0..self.list.len()).collect(),
            Some(Value::String(s)) => match s.strip_prefix('#') {
                Some(tag) => self.tags.get(tag).into_iter().flatten().filter_map(|id| self.index(id)).collect(),
                None => self.index(s).into_iter().collect(),
            },
            Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).filter_map(|id| self.index(id)).collect(),
            Some(_) => Vec::new(),
        }
    }

    /// Whether an enchantment tag holds `id`.
    pub fn tag_contains(&self, tag: &str, id: &str) -> bool {
        self.tags.get(tag).is_some_and(|ids| ids.iter().any(|e| e == id))
    }

    /// `Enchantment.areCompatible`.
    fn compatible(&self, a: usize, b: usize) -> bool {
        a != b && !self.list[a].exclusive.contains(&self.list[b].id) && !self.list[b].exclusive.contains(&self.list[a].id)
    }

    /// `getAvailableEnchantmentResults`: for each enchantment of `source`
    /// fit for the item (any for a book), its highest level the cost
    /// reaches.
    fn available(&self, cost: i32, item: &str, source: &[usize]) -> Vec<(usize, i32)> {
        let book = item == "minecraft:book";
        let mut out = Vec::new();
        for &index in source {
            let e = &self.list[index];
            if !(e.primary_item(item) || book) {
                continue;
            }
            for level in (1..=e.max_level).rev() {
                if cost >= e.min_cost(level) && cost <= e.max_cost(level) {
                    out.push((index, level));
                    break;
                }
            }
        }
        out
    }

    /// `WeightedRandom.getRandomItem` over enchantment instances.
    fn weighted(&self, random: &mut dyn LootRandom, list: &[(usize, i32)]) -> Option<(usize, i32)> {
        let total: i32 = list.iter().map(|&(e, _)| self.list[e].weight).sum();
        if total <= 0 {
            return None;
        }
        let mut selection = random.next_int(total as u32) as i32;
        for &(e, level) in list {
            selection -= self.list[e].weight;
            if selection < 0 {
                return Some((e, level));
            }
        }
        None
    }

    /// `EnchantmentHelper.selectEnchantment`: the cost raised by the item's
    /// enchantability and spread by up to 15%, one enchantment by weight,
    /// then while a roll in 50 stays under the (halving) cost, another
    /// compatible one.
    pub fn select(&self, random: &mut dyn LootRandom, item: &str, mut cost: i32, source: &[usize]) -> Vec<(usize, i32)> {
        let mut results = Vec::new();
        let Some(&enchantable) = self.enchantable.get(item) else { return results };
        cost += 1 + random.next_int((enchantable / 4 + 1) as u32) as i32 + random.next_int((enchantable / 4 + 1) as u32) as i32;
        let span = (random.next_float() + random.next_float() - 1.0) * 0.15;
        cost = java_round(cost as f32 + cost as f32 * span).max(1);
        let mut available = self.available(cost, item, source);
        if available.is_empty() {
            return results;
        }
        if let Some(first) = self.weighted(random, &available) {
            results.push(first);
        }
        while random.next_int(50) as i32 <= cost {
            if let Some(&(last, _)) = results.last() {
                available.retain(|&(e, _)| self.compatible(last, e));
            }
            if available.is_empty() {
                break;
            }
            if let Some(next) = self.weighted(random, &available) {
                results.push(next);
            }
            cost /= 2;
        }
        results
    }
}

/// An item's enchantments as its component holds them (`minecraft:enchantments`,
/// or `minecraft:stored_enchantments` on an enchanted book), each raised
/// to at least `level` (`ItemEnchantments.Mutable.upgrade`).
pub fn add_enchantment(item: &str, components: &mut serde_json::Map<String, Value>, id: &str, level: i32) {
    if level <= 0 {
        return;
    }
    let key = if item == "minecraft:enchanted_book" { "minecraft:stored_enchantments" } else { "minecraft:enchantments" };
    let map = components.entry(key).or_insert_with(|| Value::Object(serde_json::Map::new()));
    if let Value::Object(map) = map {
        let current = map.get(id).and_then(Value::as_i64).unwrap_or(0) as i32;
        map.insert(id.to_owned(), Value::from(current.max(level.min(255))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_as_java_does() {
        assert_eq!(java_round(2.5), 3);
        assert_eq!(java_round(-2.5), -2);
        assert_eq!(java_round(0.49999997), 0);
        assert_eq!(java_round(7.4), 7);
        assert_eq!(java_round(1.0e10), i32::MAX);
    }
}
