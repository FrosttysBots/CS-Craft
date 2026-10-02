//! Block loot from the pinned external data JAR. Evaluations that need an
//! unimplemented random/context rule return None instead of inventing a drop.
use crate::{
    inventory::ItemStack,
    rng::{LootRandom, XoroshiroRandom},
    Block,
};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
};
use zip::ZipArchive;

#[derive(Default)]
pub struct LootBook {
    tables: HashMap<String, Value>,
    item_tags: HashMap<String, Vec<String>>,
}

impl LootBook {
    pub fn from_jar(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("open data JAR {}", path.display()))?;
        let mut archive = ZipArchive::new(file)?;
        let mut book = Self::default();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let name = entry.name().to_owned();
            let is_table =
                name.starts_with("data/minecraft/loot_table/blocks/") && name.ends_with(".json");
            let is_tag = name.starts_with("data/minecraft/tags/item/") && name.ends_with(".json");
            if !is_table && !is_tag {
                continue;
            }
            let mut source = String::new();
            entry.read_to_string(&mut source)?;
            let value: Value =
                serde_json::from_str(&source).with_context(|| format!("parse {name}"))?;
            if is_table {
                let id = name
                    .trim_start_matches("data/minecraft/loot_table/blocks/")
                    .trim_end_matches(".json");
                book.tables.insert(format!("minecraft:{id}"), value);
            } else {
                let id = name
                    .trim_start_matches("data/minecraft/tags/item/")
                    .trim_end_matches(".json");
                let members = value["values"]
                    .as_array()
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|v| v.as_str().or_else(|| v["id"].as_str()))
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                book.item_tags.insert(format!("minecraft:{id}"), members);
            }
        }
        Ok(book)
    }

    pub fn table_count(&self) -> usize {
        self.tables.len()
    }

    /// Diagnostic coverage for a bare hand and a block with no supplied state properties.
    pub fn bare_hand_coverage_count(&self) -> usize {
        self.tables
            .keys()
            .filter(|id| {
                self.deterministic_drops(&Block::new((*id).clone()), None)
                    .is_some()
            })
            .count()
    }

    /// Some(empty) is a validated no-drop outcome; None means no table or a
    /// rule needing unsupported context or randomness.
    pub fn deterministic_drops(
        &self,
        block: &Block,
        held_item: Option<&str>,
    ) -> Option<Vec<ItemStack>> {
        self.evaluate(block, held_item, 0, false, None)
    }

    /// Roll a table with an explicit random stream and current tool data.
    /// The caller supplies the table's named sequence when one is present.
    pub fn roll_drops(
        &self,
        block: &Block,
        held_item: Option<&ItemStack>,
        random: &mut impl LootRandom,
    ) -> Option<Vec<ItemStack>> {
        let (fortune, silk_touch) = held_item.map_or((0, false), tool_enchantments);
        self.evaluate(
            block,
            held_item.map(|stack| stack.id.as_str()),
            fortune,
            silk_touch,
            Some(random),
        )
    }

    /// Use the table's persisted, world-seeded random sequence. A missing
    /// sequence is unresolved rather than silently borrowing another stream.
    pub fn roll_drops_named(
        &self,
        block: &Block,
        held_item: Option<&ItemStack>,
        world_seed: u64,
        sequences: &mut HashMap<String, XoroshiroRandom>,
    ) -> Option<Vec<ItemStack>> {
        let id = self.tables.get(&block.id)?["random_sequence"].as_str()?;
        let random = sequences
            .entry(id.to_owned())
            .or_insert_with(|| XoroshiroRandom::for_sequence(world_seed, id));
        self.roll_drops(block, held_item, random)
    }

    fn evaluate<'a>(
        &'a self,
        block: &'a Block,
        held_item: Option<&'a str>,
        fortune: u32,
        silk_touch: bool,
        random: Option<&'a mut dyn LootRandom>,
    ) -> Option<Vec<ItemStack>> {
        let table = self.tables.get(&block.id)?;
        let context = LootContext {
            block,
            held_item,
            tags: &self.item_tags,
            fortune,
            silk_touch,
            random: random.map(RefCell::new),
        };
        if table["type"].as_str()? != "minecraft:block" {
            return None;
        }
        let mut drops = Vec::new();
        for pool in table["pools"].as_array()? {
            if !condition(pool.get("condition"), &context)? {
                continue;
            }
            if pool["rolls"].as_u64()? != 1 {
                return None;
            }
            let entries = pool["entries"].as_array()?;
            if entries.len() != 1 {
                return None;
            } // Multiple entries require a random choice.
            let (_, mut produced) = entry(&entries[0], &context)?;
            apply_modifiers(pool.get("modifier"), &context, &mut produced)?;
            drops.extend(produced);
        }
        apply_modifiers(table.get("modifier"), &context, &mut drops)?;
        Some(drops)
    }
}

struct LootContext<'a> {
    block: &'a Block,
    held_item: Option<&'a str>,
    tags: &'a HashMap<String, Vec<String>>,
    fortune: u32,
    silk_touch: bool,
    random: Option<RefCell<&'a mut dyn LootRandom>>,
}

fn tool_enchantments(stack: &ItemStack) -> (u32, bool) {
    let levels = stack
        .components
        .as_ref()
        .and_then(|value| value.get("minecraft:enchantments"))
        .and_then(|value| value.get("levels"))
        .unwrap_or(&Value::Null);
    (
        levels["minecraft:fortune"].as_u64().unwrap_or(0) as u32,
        levels["minecraft:silk_touch"].as_u64().unwrap_or(0) > 0,
    )
}

fn condition(value: Option<&Value>, context: &LootContext<'_>) -> Option<bool> {
    let Some(value) = value else {
        return Some(true);
    };
    if let Some(name) = value.as_str() {
        return match name {
            "minecraft:tool/can_shear" => Some(context.held_item == Some("minecraft:shears")),
            "minecraft:tool/can_silk_touch" => Some(context.silk_touch),
            _ => None,
        };
    }
    match value["type"].as_str()? {
        "minecraft:survives_explosion" => Some(true), // Player mining has no explosion context.
        "minecraft:random_chance" => {
            let chance = value["chance"].as_f64()? as f32;
            Some(context.random.as_ref()?.borrow_mut().next_float() < chance)
        }
        "minecraft:table_bonus" => {
            let chances = value["chances"].as_array()?;
            let index = (context.fortune as usize).min(chances.len().checked_sub(1)?);
            let chance = chances[index].as_f64()? as f32;
            Some(context.random.as_ref()?.borrow_mut().next_float() < chance)
        }
        "minecraft:match_block" => {
            if value["blocks"].as_str()? != context.block.id {
                return Some(false);
            }
            let Some(states) = value.get("state") else {
                return Some(true);
            };
            let properties = states.as_object()?;
            for (name, expected) in properties {
                let actual = context.block.property(name)?;
                if actual != expected.as_str()? {
                    return Some(false);
                }
            }
            Some(true)
        }
        "minecraft:inverted" => Some(!condition(value.get("term"), context)?),
        "minecraft:any_of" => {
            let mut unknown = false;
            for term in value["terms"].as_array()? {
                match condition(Some(term), context) {
                    Some(true) => return Some(true),
                    Some(false) => {}
                    None => unknown = true,
                }
            }
            (!unknown).then_some(false)
        }
        "minecraft:all_of" => {
            let mut unknown = false;
            for term in value["terms"].as_array()? {
                match condition(Some(term), context) {
                    Some(false) => return Some(false),
                    Some(true) => {}
                    None => unknown = true,
                }
            }
            (!unknown).then_some(true)
        }
        "minecraft:match_tool" => {
            let item = context.held_item.unwrap_or("minecraft:air");
            let predicate = value["predicate"].as_object()?;
            if predicate.len() != 1 {
                return None;
            } // Components/enchantments need the full stack.
            let allowed = predicate.get("items")?.as_str()?;
            if let Some(tag) = allowed.strip_prefix('#') {
                tag_contains(context.tags, tag, item, &mut HashSet::new())
            } else {
                Some(allowed == item)
            }
        }
        _ => None,
    }
}

fn tag_contains(
    tags: &HashMap<String, Vec<String>>,
    tag: &str,
    item: &str,
    visited: &mut HashSet<String>,
) -> Option<bool> {
    if !visited.insert(tag.to_owned()) {
        return Some(false);
    }
    let mut found = false;
    for member in tags.get(tag)? {
        if let Some(nested) = member.strip_prefix('#') {
            found |= tag_contains(tags, nested, item, visited)?;
        } else {
            found |= member == item;
        }
    }
    visited.remove(tag);
    Some(found)
}

/// The bool records whether the entry was selected, even if it produced no item.
fn entry(value: &Value, context: &LootContext<'_>) -> Option<(bool, Vec<ItemStack>)> {
    if !condition(value.get("condition"), context)? {
        return Some((false, vec![]));
    }
    match value["type"].as_str()? {
        "minecraft:item" => {
            let mut stacks = vec![ItemStack::new(value["name"].as_str()?, 1)];
            apply_modifiers(value.get("modifier"), context, &mut stacks)?;
            Some((true, stacks))
        }
        "minecraft:alternatives" => {
            for child in value["children"].as_array()? {
                let (selected, stacks) = entry(child, context)?;
                if selected {
                    return Some((true, stacks));
                }
            }
            Some((false, vec![]))
        }
        _ => None,
    }
}

fn apply_modifiers(
    value: Option<&Value>,
    context: &LootContext<'_>,
    stacks: &mut Vec<ItemStack>,
) -> Option<()> {
    let Some(value) = value else { return Some(()) };
    let modifiers: Vec<&Value> = if let Some(values) = value.as_array() {
        values.iter().collect()
    } else {
        vec![value]
    };
    for modifier in modifiers {
        if !condition(modifier.get("condition"), context)? {
            continue;
        }
        match modifier["type"].as_str()? {
            "minecraft:explosion_decay" => {} // No explosion for player mining.
            "minecraft:copy_components" => {
                // Current chest/furnace state has no custom name; other copied
                // block-entity components (notably shulker contents) need data.
                if modifier["source"] != "block_entity"
                    || modifier["include"]
                        .as_array()?
                        .iter()
                        .any(|id| id != "minecraft:custom_name")
                {
                    return None;
                }
            }
            "minecraft:apply_bonus" if modifier["enchantment"] == "minecraft:fortune" => {
                for stack in stacks.iter_mut() {
                    let count = stack.count as u32;
                    let next = match modifier["formula"].as_str()? {
                        "minecraft:ore_drops" if context.fortune == 0 => count,
                        "minecraft:ore_drops" => {
                            let bonus = context
                                .random
                                .as_ref()?
                                .borrow_mut()
                                .next_int(context.fortune + 2)
                                .saturating_sub(1);
                            count * (bonus + 1)
                        }
                        "minecraft:uniform_bonus_count" => {
                            let multiplier = modifier["parameters"]["bonusMultiplier"]
                                .as_u64()
                                .unwrap_or(1) as u32;
                            if let Some(random) = &context.random {
                                count
                                    + random
                                        .borrow_mut()
                                        .next_int(multiplier * context.fortune + 1)
                            } else if context.fortune == 0 {
                                count
                            } else {
                                return None;
                            }
                        }
                        "minecraft:binomial_with_bonus_count" => {
                            let extra = modifier["parameters"]["extra"].as_u64()? as u32;
                            let chance = modifier["parameters"]["probability"].as_f64()? as f32;
                            let random = context.random.as_ref()?;
                            let mut count = count;
                            for _ in 0..context.fortune + extra {
                                count += u32::from(random.borrow_mut().next_float() < chance);
                            }
                            count
                        }
                        _ => return None,
                    };
                    stack.count = next.min(u8::MAX as u32) as u8;
                }
            }
            "minecraft:set_count" => {
                let count = modifier["count"]
                    .as_u64()
                    .or_else(|| {
                        (modifier["count"]["type"] == "minecraft:constant")
                            .then(|| modifier["count"]["value"].as_u64())
                            .flatten()
                    })
                    .or_else(|| {
                        if modifier["count"]["type"] != "minecraft:uniform" {
                            return None;
                        }
                        let min = modifier["count"]["min"].as_u64()?;
                        let max = modifier["count"]["max"].as_u64()?;
                        if min > max || max > i32::MAX as u64 {
                            return None;
                        }
                        Some(
                            min + context
                                .random
                                .as_ref()?
                                .borrow_mut()
                                .next_int((max - min + 1) as u32)
                                as u64,
                        )
                    })?;
                if count > u8::MAX as u64 {
                    return None;
                }
                for stack in stacks.iter_mut() {
                    stack.count = count as u8;
                }
                stacks.retain(|stack| stack.count > 0);
            }
            "minecraft:limit_count" => {
                let limit = &modifier["limit"];
                let min = limit["min"].as_u64().unwrap_or(0).min(u8::MAX as u64) as u8;
                let max = limit["max"]
                    .as_u64()
                    .unwrap_or(u8::MAX as u64)
                    .min(u8::MAX as u64) as u8;
                for stack in stacks.iter_mut() {
                    stack.count = stack.count.clamp(min, max);
                }
            }
            _ => return None,
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::LegacyRandom;

    #[test]
    fn pinned_jar_default_block_drops_when_available() {
        let jar = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../harness/.gradle/loom-cache/minecraftMaven/net/minecraft/minecraft-common-1fad6b3808/26.3/minecraft-common-1fad6b3808-26.3.jar",
        );
        if !jar.is_file() {
            return;
        }
        let book = LootBook::from_jar(&jar).unwrap();
        assert_eq!(book.table_count(), 1201);
        for (block, tool, expected) in [
            (
                "minecraft:dirt",
                None,
                vec![ItemStack::new("minecraft:dirt", 1)],
            ),
            (
                "minecraft:stone",
                Some("minecraft:wooden_pickaxe"),
                vec![ItemStack::new("minecraft:cobblestone", 1)],
            ),
            (
                "minecraft:deepslate",
                Some("minecraft:wooden_pickaxe"),
                vec![ItemStack::new("minecraft:cobbled_deepslate", 1)],
            ),
            ("minecraft:glass", None, vec![]),
            (
                "minecraft:oak_leaves",
                Some("minecraft:shears"),
                vec![ItemStack::new("minecraft:oak_leaves", 1)],
            ),
            (
                "minecraft:chest",
                None,
                vec![ItemStack::new("minecraft:chest", 1)],
            ),
        ] {
            assert_eq!(
                book.deterministic_drops(&Block::new(block), tool),
                Some(expected),
                "{block}"
            );
        }
        assert_eq!(
            book.deterministic_drops(
                &Block::new("minecraft:oak_slab").with("type", "double"),
                None
            ),
            Some(vec![ItemStack::new("minecraft:oak_slab", 2)])
        );
        for block in [
            Block::new("minecraft:oak_leaves"),
            Block::new("minecraft:redstone_ore"),
            Block::new("minecraft:wheat").with("age", "7"),
            Block::new("minecraft:yellow_shulker_box"),
        ] {
            assert_eq!(book.deterministic_drops(&block, None), None, "{}", block.id);
        }
    }

    #[test]
    fn unknown_random_rule_does_not_invent_a_drop() {
        let mut book = LootBook::default();
        book.tables.insert(
            "minecraft:sample".into(),
            serde_json::json!({
                "type": "minecraft:block",
                "pools": [{"rolls": 1, "entries": [{
                    "type": "minecraft:item",
                    "name": "minecraft:diamond",
                    "condition": {"type": "minecraft:random_chance", "chance": 0.5}
                }]}]
            }),
        );
        assert_eq!(
            book.deterministic_drops(&Block::new("minecraft:sample"), None),
            None
        );
        let mut low_roll = LegacyRandom::new(0);
        assert_eq!(
            book.roll_drops(&Block::new("minecraft:sample"), None, &mut low_roll),
            Some(vec![])
        );
        let mut high_roll = LegacyRandom::new(4096);
        let result = book.roll_drops(&Block::new("minecraft:sample"), None, &mut high_roll);
        assert!(result.is_some());
    }

    #[test]
    fn named_loot_sequence_persists_across_block_breaks() {
        let mut book = LootBook::default();
        book.tables.insert(
            "minecraft:sample".into(),
            serde_json::json!({
                "type": "minecraft:block",
                "random_sequence": "minecraft:blocks/sample",
                "pools": [{"rolls": 1, "entries": [{
                    "type": "minecraft:item",
                    "name": "minecraft:stone",
                    "modifier": {"type": "minecraft:set_count", "count": {
                        "type": "minecraft:uniform", "min": 1, "max": 5
                    }}
                }]}]
            }),
        );
        let mut expected = XoroshiroRandom::for_sequence(123456789, "minecraft:blocks/sample");
        let mut sequences = HashMap::new();
        for _ in 0..3 {
            let actual = book
                .roll_drops_named(
                    &Block::new("minecraft:sample"),
                    None,
                    123456789,
                    &mut sequences,
                )
                .unwrap();
            assert_eq!(actual[0].count, 1 + expected.next_int(5) as u8);
        }
        assert_eq!(sequences.len(), 1);
    }

    #[test]
    fn oak_leaves_named_sequence_matches_eighty_one_server_rolls() {
        // Two isolated 26.3 runs of scenarios/loot-oak-leaves.json repeat
        // these canonical item/count results with world seed 123456789.
        let jar = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../harness/.gradle/loom-cache/minecraftMaven/net/minecraft/minecraft-common-1fad6b3808/26.3/minecraft-common-1fad6b3808-26.3.jar",
        );
        if !jar.is_file() {
            return;
        }
        let book = LootBook::from_jar(&jar).unwrap();
        let mut sequences = HashMap::new();
        for tick in 0..=80 {
            let expected = match tick {
                20 | 40 | 75 => vec![ItemStack::new("minecraft:stick", 1)],
                72 => vec![ItemStack::new("minecraft:oak_sapling", 1)],
                _ => vec![],
            };
            assert_eq!(
                book.roll_drops_named(
                    &Block::new("minecraft:oak_leaves").with("persistent", "true"),
                    None,
                    123456789,
                    &mut sequences,
                ),
                Some(expected),
                "vanilla loot roll tick {tick}"
            );
        }
    }

    #[test]
    fn pinned_random_loot_rules_roll_with_a_supplied_stream() {
        let jar = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../harness/.gradle/loom-cache/minecraftMaven/net/minecraft/minecraft-common-1fad6b3808/26.3/minecraft-common-1fad6b3808-26.3.jar",
        );
        if !jar.is_file() {
            return;
        }
        let book = LootBook::from_jar(&jar).unwrap();
        let mut random = LegacyRandom::new(0);
        let tool = ItemStack::new("minecraft:wooden_pickaxe", 1);
        let ore = book
            .roll_drops(
                &Block::new("minecraft:redstone_ore"),
                Some(&tool),
                &mut random,
            )
            .unwrap();
        assert_eq!(ore[0].id, "minecraft:redstone");
        assert!((4..=5).contains(&ore[0].count));
        let crop = book
            .roll_drops(
                &Block::new("minecraft:wheat").with("age", "7"),
                None,
                &mut LegacyRandom::new(0),
            )
            .unwrap();
        assert_eq!(crop[0].id, "minecraft:wheat");
        assert!(crop.iter().any(|stack| stack.id == "minecraft:wheat_seeds"));
    }
}
