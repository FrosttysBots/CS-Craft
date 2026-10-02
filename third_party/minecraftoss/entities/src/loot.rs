//! Entity loot tables from the pinned common data JAR. Unsupported conditions
//! return None; they must never silently invent drops.
use anyhow::{Context, Result};
use minecraftoss_player::{
    inventory::ItemStack,
    rng::{LootRandom, XoroshiroRandom},
};
use serde_json::Value;
use std::{collections::HashMap, fs::File, io::Read, path::Path};
use zip::ZipArchive;

/// What an entity's death gives its loot table (`LootContextParams`): the
/// entity itself (`THIS_ENTITY`), who killed it (`ATTACKING_ENTITY`,
/// `DIRECT_ATTACKING_ENTITY`) and whether a player did (`LAST_DAMAGE_PLAYER`).
#[derive(Clone, Copy, Debug, Default)]
pub struct EntityLootContext {
    pub on_fire: bool,
    pub looting: u32,
    pub has_direct_attacker: bool,
    pub sheep_color: Option<u8>,
    pub sheep_sheared: bool,
    /// A player hurt it within its memory of the attacker.
    pub killed_by_player: bool,
    pub baby: bool,
    /// The entity type it rides.
    pub vehicle: Option<&'static str>,
    /// The killer's entity type.
    pub attacker: Option<&'static str>,
    /// The dead entity's own type.
    pub this_type: Option<&'static str>,
    /// A cube mob's size (`type_specific/cube_mob`).
    pub cube_size: Option<i32>,
}

pub struct EntityLootBook {
    tables: HashMap<String, Value>,
    sequences: HashMap<String, XoroshiroRandom>,
    world_seed: u64,
    /// Smelting results by input item (`furnace_smelt`).
    smelting: HashMap<String, String>,
    /// Item and entity type tags the tables name (without the `#`).
    item_tags: HashMap<String, Vec<String>>,
    entity_tags: HashMap<String, Vec<String>>,
}

/// Source: LivingEntity.dropFromShearingLootTable and the pinned common JAR's
/// data/minecraft/loot_table/shearing/sheep*.json tables. This evaluates the
/// sheep branch while preserving each named loot random sequence.
pub struct ShearingLootBook {
    tables: HashMap<String, Value>,
    sequences: HashMap<String, XoroshiroRandom>,
    world_seed: u64,
}

impl ShearingLootBook {
    pub fn from_jar(path: &Path, world_seed: u64) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("open data JAR {}", path.display()))?;
        let mut archive = ZipArchive::new(file)?;
        let mut tables = HashMap::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let name = entry.name().to_owned();
            let Some(id) = name
                .strip_prefix("data/minecraft/loot_table/shearing/")
                .and_then(|n| n.strip_suffix(".json"))
            else {
                continue;
            };
            let mut source = String::new();
            entry.read_to_string(&mut source)?;
            tables.insert(
                format!("minecraft:shearing/{id}"),
                serde_json::from_str(&source).with_context(|| format!("parse {name}"))?,
            );
        }
        Ok(Self {
            tables,
            sequences: HashMap::new(),
            world_seed,
        })
    }

    pub fn roll_sheep(&mut self, color: &str) -> Option<Vec<ItemStack>> {
        let root = self.tables.get("minecraft:shearing/sheep")?;
        if root["type"] != "minecraft:shearing" {
            return None;
        }
        // NestedLootTable$1 calls getRandomItemsRaw with the parent's
        // LootContext, so child table random_sequence IDs do not reseed it.
        let sequence_id = root["random_sequence"].as_str()?.to_owned();
        let pools = root["pools"].as_array()?;
        if pools.len() != 1 || pools[0]["rolls"] != 1 {
            return None;
        }
        let entries = pools[0]["entries"].as_array()?;
        if entries.len() != 1 || entries[0]["type"] != "minecraft:alternatives" {
            return None;
        }
        let child = entries[0]["children"].as_array()?.iter().find(|child| {
            child["type"] == "minecraft:loot_table"
                && child["condition"]["type"] == "minecraft:entity_properties"
                && child["condition"]["entity"] == "this"
                && child["condition"]["predicate"]["minecraft:components"]["minecraft:sheep/color"]
                    == color
                && child["condition"]["predicate"]["minecraft:type_specific/sheep"]["sheared"]
                    == false
        })?;
        let id = child["value"].as_str()?;
        let table = self.tables.get(id)?;
        if table["type"] != "minecraft:shearing" {
            return None;
        }
        let pools = table["pools"].as_array()?;
        if pools.len() != 1 {
            return None;
        }
        let entries = pools[0]["entries"].as_array()?;
        if entries.len() != 1 || entries[0]["type"] != "minecraft:item" {
            return None;
        }
        let item = entries[0]["name"].as_str()?.to_owned();
        let rolls = &pools[0]["rolls"];
        if rolls["type"] != "minecraft:uniform" {
            return None;
        }
        let min = rolls["min"].as_u64()? as u32;
        let max = rolls["max"].as_u64()? as u32;
        if max < min || max - min >= i32::MAX as u32 {
            return None;
        }
        let random = self
            .sequences
            .entry(sequence_id.clone())
            .or_insert_with(|| XoroshiroRandom::for_sequence(self.world_seed, &sequence_id));
        let count = min + random.next_int(max - min + 1);
        Some(
            (0..count)
                .map(|_| ItemStack::new(item.clone(), 1))
                .collect(),
        )
    }
}

impl EntityLootBook {
    pub fn from_jar(path: &Path, world_seed: u64) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("open data JAR {}", path.display()))?;
        let mut archive = ZipArchive::new(file)?;
        let mut tables = HashMap::new();
        let mut smelting = HashMap::new();
        let mut item_tags = HashMap::new();
        let mut entity_tags = HashMap::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let name = entry.name().to_owned();
            let wanted = name.starts_with("data/minecraft/loot_table/entities/")
                || name.starts_with("data/minecraft/loot_table/charged_creeper/")
                || name.starts_with("data/minecraft/tags/item/")
                || name.starts_with("data/minecraft/tags/entity_type/")
                || name.starts_with("data/minecraft/recipe/");
            if !wanted || !name.ends_with(".json") {
                continue;
            }
            let mut source = String::new();
            entry.read_to_string(&mut source)?;
            let json: Value = serde_json::from_str(&source).with_context(|| format!("parse {name}"))?;
            let stem = name.strip_suffix(".json").unwrap_or(&name);
            if let Some(id) = stem.strip_prefix("data/minecraft/loot_table/entities/") {
                let key = if id.contains('/') { format!("minecraft:entities/{id}") } else { format!("minecraft:{id}") };
                tables.insert(key, json);
            } else if let Some(id) = stem.strip_prefix("data/minecraft/loot_table/") {
                // The heads a charged creeper's blast knocks off.
                tables.insert(format!("minecraft:{id}"), json);
            } else if let Some(id) = stem.strip_prefix("data/minecraft/tags/item/") {
                item_tags.insert(format!("minecraft:{id}"), tag_values(&json));
            } else if let Some(id) = stem.strip_prefix("data/minecraft/tags/entity_type/") {
                entity_tags.insert(format!("minecraft:{id}"), tag_values(&json));
            } else if json["type"] == "minecraft:smelting" {
                if let (Some(input), Some(result)) = (json["ingredient"].as_str(), json["result"]["id"].as_str()) {
                    smelting.insert(input.to_owned(), result.to_owned());
                }
            }
        }
        Ok(Self {
            tables,
            sequences: HashMap::new(),
            world_seed,
            smelting,
            item_tags,
            entity_tags,
        })
    }

    pub fn table_count(&self) -> usize {
        self.tables.len()
    }

    /// The drops of `deaths` (in the order the mobs died), each where its
    /// mob died (`LivingEntity.dropAllDeathLoot`): the head a charged
    /// creeper's blast knocks off its first victim that has one
    /// (`Creeper.killedEntity`), the mob's own loot, then the equipment its
    /// killer shook loose. A table this evaluator cannot reproduce drops
    /// nothing rather than something invented.
    pub fn death_drops(&mut self, deaths: Vec<crate::world::MobDeath>) -> Vec<(ItemStack, glam::DVec3)> {
        let mut drops = Vec::new();
        // Creepers whose blast already dropped a head (`droppedSkulls`).
        let mut skulls = std::collections::HashSet::new();
        for death in deaths {
            let at = |stacks: Vec<ItemStack>| stacks.into_iter().map(move |stack| (stack, death.position));
            if let Some(creeper) = death.charged_creeper.filter(|creeper| !skulls.contains(creeper)) {
                // `dropFromLootTable(level, source, false, CHARGED_CREEPER)`.
                let context = EntityLootContext { killed_by_player: false, ..death.context };
                let heads = self.roll("minecraft:charged_creeper/root", context).unwrap_or_default();
                if !heads.is_empty() {
                    skulls.insert(creeper);
                }
                drops.extend(at(heads));
            }
            if let Some(table) = death.table {
                drops.extend(at(self.roll(table, death.context).unwrap_or_default()));
            }
            drops.extend(at(death.equipment));
        }
        drops
    }

    /// Returns None if the table has a context, condition or modifier this
    /// evaluator cannot yet reproduce. Named RNG persists across kills.
    pub fn roll(&mut self, entity_id: &str, context: EntityLootContext) -> Option<Vec<ItemStack>> {
        let table = self.tables.get(entity_id)?;
        if table["type"].as_str()? != "minecraft:entity" {
            return None;
        }
        let id = table["random_sequence"].as_str()?;
        let random = self
            .sequences
            .entry(id.to_owned())
            .or_insert_with(|| XoroshiroRandom::for_sequence(self.world_seed, id));
        // A table outside the implemented evaluator must not partially advance
        // the persistent named stream before its missing rule is implemented.
        let mut candidate = random.clone();
        let data = LootData { tables: &self.tables, smelting: &self.smelting, item_tags: &self.item_tags, entity_tags: &self.entity_tags };
        let drops = roll_table(&data, table, context, &mut candidate)?;
        *random = candidate;
        Some(drops)
    }
}

/// A tag file's values, nested tags kept as `#` names.
fn tag_values(tag: &Value) -> Vec<String> {
    tag["values"].as_array().into_iter().flatten().filter_map(|v| v.as_str().or_else(|| v["id"].as_str())).map(str::to_owned).collect()
}

/// What the evaluator reads besides the table itself.
struct LootData<'a> {
    tables: &'a HashMap<String, Value>,
    smelting: &'a HashMap<String, String>,
    item_tags: &'a HashMap<String, Vec<String>>,
    entity_tags: &'a HashMap<String, Vec<String>>,
}

impl LootData<'_> {
    /// A tag's members, nested tags expanded.
    fn members(tags: &HashMap<String, Vec<String>>, name: &str, out: &mut Vec<String>) {
        for value in tags.get(name).into_iter().flatten() {
            match value.strip_prefix('#') {
                Some(nested) => Self::members(tags, nested, out),
                None => out.push(value.clone()),
            }
        }
    }

    /// An entity type predicate (an ID or `#tag`) against an entity's type.
    fn type_matches(&self, wanted: &str, actual: Option<&str>) -> bool {
        let Some(actual) = actual else { return false };
        match wanted.strip_prefix('#') {
            Some(tag) => {
                let mut types = Vec::new();
                Self::members(self.entity_tags, tag, &mut types);
                types.iter().any(|t| t == actual)
            }
            None => wanted == actual,
        }
    }
}

/// `LootTable.getRandomItemsRaw`: each pool whose conditions hold rolls
/// its entries (`LootPool.addRandomItems`), in the order vanilla draws.
/// Unsupported rules give `None`, never invented drops.
fn roll_table(data: &LootData, table: &Value, context: EntityLootContext, random: &mut impl LootRandom) -> Option<Vec<ItemStack>> {
    let mut drops = Vec::new();
    // A table without pools (a bat's) drops nothing.
    for pool in table["pools"].as_array().into_iter().flatten() {
        if !conditions(data, pool, context, random)? {
            continue;
        }
        // `bonus_rolls` scale with luck, which is zero here.
        let rolls = int_provider(&pool["rolls"], random)?;
        for _ in 0..rolls.max(0) {
            // `LootPool.addRandomItem`: expand the entries, pick by weight.
            let mut valid: Vec<(Value, u32)> = Vec::new();
            for entry in pool["entries"].as_array()? {
                expand(data, entry, context, random, &mut valid)?;
            }
            let total: u32 = valid.iter().map(|(_, w)| w).sum();
            if total == 0 || valid.is_empty() {
                continue;
            }
            let chosen = if valid.len() == 1 {
                0
            } else {
                let mut index = random.next_int(total) as i64;
                let mut chosen = valid.len() - 1;
                for (i, (_, weight)) in valid.iter().enumerate() {
                    index -= i64::from(*weight);
                    if index < 0 {
                        chosen = i;
                        break;
                    }
                }
                chosen
            };
            let leaf = valid.swap_remove(chosen).0;
            create(data, &leaf, context, random, &mut drops)?;
        }
    }
    Some(drops)
}

/// An entry's, pool's or function's `condition` (or `conditions`, all of
/// which must hold), tested in order.
fn conditions(data: &LootData, holder: &Value, context: EntityLootContext, random: &mut impl LootRandom) -> Option<bool> {
    if let Some(condition) = holder.get("condition") {
        if !condition_true(data, condition, context, random)? {
            return Some(false);
        }
    }
    for condition in holder.get("conditions").and_then(Value::as_array).into_iter().flatten() {
        if !condition_true(data, condition, context, random)? {
            return Some(false);
        }
    }
    Some(true)
}

/// `LootPoolEntryContainer.expand`: an entry that passes its conditions
/// offers itself (a tag with `expand` offers each item; alternatives the
/// first child that expands).
fn expand(data: &LootData, entry: &Value, context: EntityLootContext, random: &mut impl LootRandom, out: &mut Vec<(Value, u32)>) -> Option<bool> {
    if !conditions(data, entry, context, random)? {
        return Some(false);
    }
    // `getWeight(luck)`: quality times luck adds nothing here.
    let weight = entry.get("weight").and_then(Value::as_u64).unwrap_or(1) as u32;
    match entry["type"].as_str()? {
        "minecraft:item" | "minecraft:empty" | "minecraft:loot_table" => {
            out.push((entry.clone(), weight));
            Some(true)
        }
        "minecraft:tag" if entry["expand"].as_bool() == Some(true) => {
            let mut items = Vec::new();
            // `TagEntry`'s `items` (a `#` tag); older data named it `name`.
            let tag = entry.get("items").or_else(|| entry.get("name")).and_then(Value::as_str)?;
            LootData::members(data.item_tags, tag.trim_start_matches('#'), &mut items);
            for item in items {
                let mut leaf = entry.clone();
                leaf["type"] = Value::from("minecraft:item");
                leaf["name"] = Value::from(item);
                out.push((leaf, weight));
            }
            Some(true)
        }
        "minecraft:alternatives" => {
            for child in entry["children"].as_array()? {
                if expand(data, child, context, random, out)? {
                    return Some(true);
                }
            }
            Some(false)
        }
        _ => None,
    }
}

/// `createItemStack` for the chosen entry, through its functions.
fn create(data: &LootData, leaf: &Value, context: EntityLootContext, random: &mut impl LootRandom, out: &mut Vec<ItemStack>) -> Option<()> {
    match leaf["type"].as_str()? {
        "minecraft:empty" => Some(()),
        // NestedLootTable creates drops in the parent's LootContext: its own
        // random_sequence does not reseed this stream.
        "minecraft:loot_table" => {
            let nested = data.tables.get(leaf["value"].as_str()?)?;
            out.extend(roll_table(data, nested, context, random)?);
            Some(())
        }
        "minecraft:item" => {
            let mut item = leaf["name"].as_str()?.to_owned();
            let mut count: i64 = 1;
            let mut potion = None;
            let functions: Vec<&Value> = match leaf.get("modifier").or_else(|| leaf.get("functions")) {
                Some(Value::Array(list)) => list.iter().collect(),
                Some(single) => vec![single],
                None => Vec::new(),
            };
            for function in functions {
                if !conditions(data, function, context, random)? {
                    continue;
                }
                match function["type"].as_str()? {
                    "minecraft:set_count" => {
                        let value = i64::from(int_provider(&function["count"], random)?);
                        count = if function["add"].as_bool() == Some(true) { count + value } else { value };
                    }
                    // Looting 0 adds nothing and draws nothing.
                    "minecraft:enchanted_count_increase" if context.looting == 0 => {}
                    "minecraft:furnace_smelt" => item = data.smelting.get(&item)?.clone(),
                    // `SetPotionFunction`: the potion contents component.
                    "minecraft:set_potion" => potion = Some(function["id"].as_str()?.to_owned()),
                    _ => return None,
                }
            }
            // An empty stack drops nothing.
            if count > 0 {
                let mut stack = ItemStack::new(item, u8::try_from(count).ok()?);
                if let Some(potion) = potion {
                    stack.components = Some(serde_json::json!({ "minecraft:potion_contents": { "potion": potion } }));
                }
                out.push(stack);
            }
            Some(())
        }
        _ => None,
    }
}

/// A number provider's integer (`NumberProvider.getInt`).
fn int_provider(value: &Value, random: &mut impl LootRandom) -> Option<i32> {
    if let Some(n) = value.as_i64() {
        return Some(n as i32);
    }
    if let Some(f) = value.as_f64() {
        return Some(f.floor() as i32);
    }
    match value["type"].as_str()? {
        "minecraft:constant" => Some(value["value"].as_f64()?.floor() as i32),
        // `Mth.randomBetweenInclusive`.
        "minecraft:uniform" => {
            let min = int_provider(&value["min"], random)?;
            let max = int_provider(&value["max"], random)?;
            if max < min {
                return Some(min);
            }
            Some(random.next_int((max - min + 1) as u32) as i32 + min)
        }
        _ => None,
    }
}

/// A number provider's float.
fn float_provider(value: &Value) -> Option<f32> {
    if let Some(f) = value.as_f64() {
        return Some(f as f32);
    }
    match value["type"].as_str()? {
        "minecraft:constant" => Some(value["value"].as_f64()? as f32),
        _ => None,
    }
}

/// `LootItemCondition.test`.
fn condition_true(data: &LootData, value: &Value, context: EntityLootContext, random: &mut impl LootRandom) -> Option<bool> {
    match value["type"].as_str()? {
        "minecraft:any_of" => {
            for term in value["terms"].as_array()? {
                if condition_true(data, term, context, random)? {
                    return Some(true);
                }
            }
            Some(false)
        }
        "minecraft:all_of" => {
            for term in value["terms"].as_array()? {
                if !condition_true(data, term, context, random)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        "minecraft:inverted" => Some(!condition_true(data, &value["term"], context, random)?),
        "minecraft:killed_by_player" => Some(context.killed_by_player),
        "minecraft:random_chance" => Some(random.next_float() < float_provider(&value["chance"])?),
        // Without looting the plain chance; with it, the level-based one.
        "minecraft:random_chance_with_enchanted_bonus" => {
            if context.looting > 0 {
                return None;
            }
            Some(random.next_float() < float_provider(&value["unenchanted_chance"])?)
        }
        "minecraft:entity_properties" => entity_properties(data, value, context),
        // `DamageSourceCondition` on the source's entity type (the killer).
        "minecraft:damage_source_properties" => {
            let predicate = value["predicate"].as_object()?;
            if predicate.keys().any(|k| k != "source_entity") {
                return None;
            }
            let source = predicate.get("source_entity")?.as_object()?;
            if source.keys().any(|k| k != "minecraft:entity_type") {
                return None;
            }
            Some(data.type_matches(source.get("minecraft:entity_type")?.as_str()?, context.attacker))
        }
        _ => None,
    }
}

/// `EntityPropertyCondition` for the predicates entity tables use.
fn entity_properties(data: &LootData, value: &Value, context: EntityLootContext) -> Option<bool> {
    let predicate = value["predicate"].as_object()?;
    match value["entity"].as_str()? {
        "this" => {
            for (key, test) in predicate {
                let holds = match key.as_str() {
                    "minecraft:flags" => {
                        let mut holds = true;
                        for (flag, wanted) in test.as_object()? {
                            let actual = match flag.as_str() {
                                "is_on_fire" => context.on_fire,
                                "is_baby" => context.baby,
                                _ => return None,
                            };
                            holds &= wanted.as_bool()? == actual;
                        }
                        holds
                    }
                    "minecraft:entity_type" => data.type_matches(test.as_str()?, context.this_type),
                    "minecraft:vehicle" => data.type_matches(test["minecraft:entity_type"].as_str()?, context.vehicle),
                    "minecraft:type_specific/sheep" => test["sheared"].as_bool()? == context.sheep_sheared,
                    // `CubeMobPredicate`: its size, exact or a range.
                    "minecraft:type_specific/cube_mob" => {
                        let size = context.cube_size?;
                        match &test["size"] {
                            Value::Number(n) => i64::from(size) == n.as_i64()?,
                            range => range["min"].as_i64().is_none_or(|min| i64::from(size) >= min) && range["max"].as_i64().is_none_or(|max| i64::from(size) <= max),
                        }
                    }
                    "minecraft:components" => {
                        let color = test["minecraft:sheep/color"].as_str()?;
                        let id = context.sheep_color? as usize;
                        *crate::sheep::DYE_NAMES.get(id)? == color
                    }
                    _ => return None,
                };
                if !holds {
                    return Some(false);
                }
            }
            Some(true)
        }
        "attacker" | "killer" => {
            if predicate.keys().any(|k| k != "minecraft:entity_type") {
                return None;
            }
            Some(data.type_matches(predicate.get("minecraft:entity_type")?.as_str()?, context.attacker))
        }
        // No enchantments are modelled: an attacker's weapon never has one.
        "direct_attacker" if !context.has_direct_attacker => Some(false),
        "direct_attacker" if predicate.keys().all(|k| k == "minecraft:equipment") => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_smelt_does_not_advance_named_stream() {
        let table: Value = serde_json::json!({
            "type": "minecraft:entity", "random_sequence": "minecraft:entities/cow",
            "pools": [{"rolls": 1, "entries": [{"type": "minecraft:item", "name": "minecraft:beef", "modifier": [
                {"type": "minecraft:set_count", "count": {"type": "minecraft:uniform", "min": 1, "max": 3}},
                {"type": "minecraft:furnace_smelt", "condition": {"type": "minecraft:entity_properties", "entity": "this", "predicate": {"minecraft:flags": {"is_on_fire": true}}}}
            ]}]}]
        });
        let make = || EntityLootBook {
            tables: HashMap::from([("minecraft:cow".into(), table.clone())]),
            sequences: HashMap::new(),
            world_seed: 0,
            smelting: HashMap::new(),
            item_tags: HashMap::new(),
            entity_tags: HashMap::new(),
        };
        let mut book = make();
        assert!(book
            .roll(
                "minecraft:cow",
                EntityLootContext {
                    on_fire: true,
                    ..Default::default()
                }
            )
            .is_none());
        let first = book
            .roll("minecraft:cow", EntityLootContext::default())
            .unwrap();
        let fresh = make()
            .roll("minecraft:cow", EntityLootContext::default())
            .unwrap();
        assert_eq!(first.len(), fresh.len());
        assert_eq!(first[0].count, fresh[0].count);
    }
}
