//! Loot tables (26.3 `LootTable`, `LootPool`, the pool entries, the
//! condition and function types the vanilla data uses, and the int and float
//! context providers), read from the data pack.
//!
//! Tables with a `random_sequence` draw from that per-world named sequence
//! (`RandomSequences`), others from the caller's random. Constructs not
//! ported yet make the evaluation fail with a message instead of inventing
//! loot.

use crate::datapack::DataPack;
use crate::ident::Identifier;
use crate::item::ItemStack;
use crate::random::{RandomSource, XoroshiroRandom};
use crate::{BlockStateId, Registries};
use serde_json::Value as Json;
use std::collections::HashMap;
use std::sync::Mutex;

/// What a loot evaluation knows (`LootParams`).
#[derive(Clone, Debug, Default)]
pub struct LootParams {
    pub origin: Option<[f64; 3]>,
    /// The block broken or dropped (`BLOCK_STATE`).
    pub block_state: Option<BlockStateId>,
    /// The tool (`TOOL`); `Some(empty)` for blocks broken without one.
    pub tool: Option<ItemStack>,
    /// `THIS_ENTITY` is present.
    pub this_entity: bool,
    /// `EXPLOSION_RADIUS`.
    pub explosion_radius: Option<f32>,
    pub luck: f32,
}

/// Parsed loot tables and predicates, loaded on first use.
pub struct LootTables {
    tables: Mutex<HashMap<String, Option<Json>>>,
    predicates: Mutex<HashMap<String, Option<Json>>>,
}

impl Default for LootTables {
    fn default() -> Self {
        Self { tables: Mutex::new(HashMap::new()), predicates: Mutex::new(HashMap::new()) }
    }
}

type Result<T> = std::result::Result<T, String>;

struct Context<'a> {
    registries: &'a Registries,
    tables: &'a LootTables,
    params: &'a LootParams,
    random: &'a mut dyn RandomSource,
    /// Tables being evaluated (`LootContext.visitedElements`).
    visiting: Vec<String>,
}

impl XoroshiroRandom {
    /// `RandomSequence(seed, key)`: the world seed, unmixed, xored with the
    /// key's MD5, then mixed.
    pub fn for_sequence(world_seed: i64, key: &str) -> Self {
        const SILVER: u64 = 0x6a09_e667_f3bc_c909;
        const GOLDEN: u64 = 0x9e37_79b9_7f4a_7c15;
        let lo = (world_seed as u64) ^ SILVER;
        let hi = lo.wrapping_add(GOLDEN);
        let digest = md5::compute(key.as_bytes());
        let hash_lo = u64::from_be_bytes(digest[0..8].try_into().expect("MD5 half"));
        let hash_hi = u64::from_be_bytes(digest[8..16].try_into().expect("MD5 half"));
        let mix = |mut z: u64| {
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        XoroshiroRandom::from_state(mix(lo ^ hash_lo), mix(hi ^ hash_hi))
    }
}

/// Per-world named random sequences (`RandomSequences`).
#[derive(Debug, Default)]
pub struct RandomSequences {
    pub world_seed: i64,
    sequences: HashMap<String, XoroshiroRandom>,
}

impl RandomSequences {
    pub fn new(world_seed: i64) -> Self {
        Self { world_seed, sequences: HashMap::new() }
    }

    pub fn get(&mut self, key: &str) -> &mut XoroshiroRandom {
        let seed = self.world_seed;
        self.sequences.entry(key.to_owned()).or_insert_with(|| XoroshiroRandom::for_sequence(seed, key))
    }
}

/// A chain with one more modifier inside it.
fn with_inner(modifier: Option<&Json>, chain: &[Json]) -> Vec<Json> {
    let mut out: Vec<Json> = modifier.cloned().into_iter().collect();
    out.extend(chain.iter().cloned());
    out
}

fn id_of(text: &str) -> String {
    if text.contains(':') { text.to_owned() } else { format!("minecraft:{text}") }
}

impl LootTables {
    fn load(map: &Mutex<HashMap<String, Option<Json>>>, pack: &DataPack, kind: &str, id: &str) -> Option<Json> {
        let mut map = map.lock().expect("loot cache");
        map.entry(id.to_owned())
            .or_insert_with(|| Identifier::parse(id).ok().and_then(|ident| pack.read_json(kind, &ident).ok()))
            .clone()
    }

    fn table(&self, registries: &Registries, id: &str) -> Option<Json> {
        Self::load(&self.tables, &registries.datapack, "loot_table", id)
    }

    fn predicate(&self, registries: &Registries, id: &str) -> Option<Json> {
        Self::load(&self.predicates, &registries.datapack, "predicate", id)
    }

    /// `LootTable.getRandomItems(params)`: the table's own sequence when it
    /// names one, else `random`, with stacks split to their size.
    pub fn roll(
        &self,
        registries: &Registries,
        id: &str,
        params: &LootParams,
        sequences: &mut RandomSequences,
        random: &mut dyn RandomSource,
    ) -> Result<Vec<ItemStack>> {
        let Some(table) = self.table(registries, id) else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        match table.get("random_sequence").and_then(Json::as_str) {
            Some(sequence) => {
                let random = sequences.get(sequence);
                let mut context = Context { registries, tables: self, params, random, visiting: Vec::new() };
                context.table_items(id, &table, &[], &mut out)?;
            }
            None => {
                let mut context = Context { registries, tables: self, params, random, visiting: Vec::new() };
                context.table_items(id, &table, &[], &mut out)?;
            }
        }
        // `createStackSplitter`.
        let mut split = Vec::new();
        for stack in out {
            let max = registries.items.max_stack(&stack.id);
            if stack.count < max {
                split.push(stack);
            } else {
                let mut count = stack.count;
                while count > 0 {
                    let mut part = stack.clone();
                    part.count = count.min(max);
                    count -= part.count;
                    split.push(part);
                }
            }
        }
        Ok(split)
    }

    /// A block's own loot table (`blocks/<name>`).
    pub fn block_drops(
        &self,
        registries: &Registries,
        state: BlockStateId,
        params: &LootParams,
        sequences: &mut RandomSequences,
        random: &mut dyn RandomSource,
    ) -> Result<Vec<ItemStack>> {
        let blocks = &registries.blocks;
        let name = blocks.block(blocks.block_of(state)).name.as_str().to_owned();
        let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", &name));
        let mut params = params.clone();
        params.block_state = Some(state);
        self.roll(registries, &format!("{namespace}:blocks/{path}"), &params, sequences, random)
    }
}

impl Context<'_> {
    /// `LootTable.getRandomItemsRaw`. `chain` holds the modifiers that
    /// decorate the output, innermost first; each stack runs through them as
    /// soon as it is made, as vanilla's decorated consumers do.
    fn table_items(&mut self, id: &str, table: &Json, chain: &[Json], out: &mut Vec<ItemStack>) -> Result<()> {
        if self.visiting.iter().any(|v| v == id) {
            return Ok(());
        }
        self.visiting.push(id.to_owned());
        let chain = with_inner(table.get("modifier"), chain);
        for pool in table.get("pools").and_then(Json::as_array).into_iter().flatten() {
            self.pool_items(pool, &chain, out)?;
        }
        self.visiting.pop();
        Ok(())
    }

    /// Runs a new stack through a modifier chain into the output.
    fn emit(&mut self, mut stack: ItemStack, chain: &[Json], out: &mut Vec<ItemStack>) -> Result<()> {
        for modifier in chain {
            stack = self.apply_modifier(Some(modifier), stack)?;
        }
        out.push(stack);
        Ok(())
    }

    /// `LootPool.addRandomItems`.
    fn pool_items(&mut self, pool: &Json, chain: &[Json], out: &mut Vec<ItemStack>) -> Result<()> {
        if !self.condition_opt(pool.get("condition"))? {
            return Ok(());
        }
        let rolls = self.int(pool.get("rolls").ok_or("pool without rolls")?)?;
        let bonus = match pool.get("bonus_rolls") {
            Some(b) => self.float(b)?,
            None => 0.0,
        };
        let count = rolls + (bonus * self.params.luck).floor() as i32;
        let chain = with_inner(pool.get("modifier"), chain);
        for _ in 0..count {
            self.add_random_item(pool, &chain, out)?;
        }
        Ok(())
    }

    /// `LootPool.addRandomItem`: expand the entries, then pick by weight.
    fn add_random_item(&mut self, pool: &Json, chain: &[Json], out: &mut Vec<ItemStack>) -> Result<()> {
        let mut valid: Vec<(Json, Vec<Json>, i32)> = Vec::new();
        for entry in pool.get("entries").and_then(Json::as_array).into_iter().flatten() {
            let mut expanded = Vec::new();
            self.expand(entry, &mut Vec::new(), &mut expanded)?;
            for (leaf, modifiers) in expanded {
                let weight = self.weight(&leaf);
                if weight > 0 {
                    valid.push((leaf, modifiers, weight));
                }
            }
        }
        let total: i32 = valid.iter().map(|v| v.2).sum();
        if total == 0 || valid.is_empty() {
            return Ok(());
        }
        let chosen = if valid.len() == 1 {
            0
        } else {
            let mut index = self.random.next_i32_bound(total);
            let mut chosen = valid.len() - 1;
            for (i, v) in valid.iter().enumerate() {
                index -= v.2;
                if index < 0 {
                    chosen = i;
                    break;
                }
            }
            chosen
        };
        let (leaf, modifiers, _) = valid.swap_remove(chosen);
        // The leaf's modifier, then its enclosing composites' from the
        // innermost out, then the pool's and the table's.
        let mut full: Vec<Json> = leaf.get("modifier").cloned().into_iter().collect();
        full.extend(modifiers.iter().rev().cloned());
        full.extend(chain.iter().cloned());
        self.create_items(&leaf, &full, out)
    }

    fn weight(&self, leaf: &Json) -> i32 {
        let weight = leaf.get("weight").and_then(Json::as_i64).unwrap_or(1) as f32;
        let quality = leaf.get("quality").and_then(Json::as_i64).unwrap_or(0) as f32;
        ((weight + quality * self.params.luck).floor() as i32).max(0)
    }

    /// `LootPoolEntryContainer.expand`: a condition gate, then the entry's
    /// own expansion; composite entries' modifiers wrap their children.
    fn expand(&mut self, entry: &Json, modifiers: &mut Vec<Json>, out: &mut Vec<(Json, Vec<Json>)>) -> Result<bool> {
        if !self.condition_opt(entry.get("condition"))? {
            return Ok(false);
        }
        let kind = entry.get("type").and_then(Json::as_str).unwrap_or("minecraft:item");
        match kind.trim_start_matches("minecraft:") {
            "item" | "empty" | "loot_table" | "dynamic" => {
                out.push((entry.clone(), modifiers.clone()));
                Ok(true)
            }
            "alternatives" | "group" | "sequence" => {
                let children: Vec<Json> = entry.get("children").and_then(Json::as_array).cloned().unwrap_or_default();
                let pushed = entry.get("modifier").cloned();
                if let Some(m) = &pushed {
                    modifiers.push(m.clone());
                }
                let result = match kind.trim_start_matches("minecraft:") {
                    "alternatives" => {
                        let mut any = false;
                        for child in &children {
                            if self.expand(child, modifiers, out)? {
                                any = true;
                                break;
                            }
                        }
                        any
                    }
                    "group" => {
                        for child in &children {
                            self.expand(child, modifiers, out)?;
                        }
                        true
                    }
                    _ => {
                        let mut all = true;
                        for child in &children {
                            if !self.expand(child, modifiers, out)? {
                                all = false;
                                break;
                            }
                        }
                        all
                    }
                };
                if pushed.is_some() {
                    modifiers.pop();
                }
                Ok(result)
            }
            other => Err(format!("loot entry type {other} is not supported")),
        }
    }

    /// `createItemStack` of a leaf entry.
    fn create_items(&mut self, leaf: &Json, chain: &[Json], out: &mut Vec<ItemStack>) -> Result<()> {
        let kind = leaf.get("type").and_then(Json::as_str).unwrap_or("minecraft:item");
        match kind.trim_start_matches("minecraft:") {
            "item" => {
                let name = leaf.get("name").and_then(Json::as_str).ok_or("item entry without name")?;
                self.emit(ItemStack::new(&id_of(name), 1), chain, out)
            }
            "empty" => Ok(()),
            "loot_table" => {
                let value = leaf.get("value").ok_or("loot_table entry without value")?;
                match value {
                    Json::String(id) => {
                        let id = id_of(id);
                        let table = self.tables.table(self.registries, &id).ok_or_else(|| format!("missing loot table {id}"))?;
                        self.table_items(&id, &table, chain, out)
                    }
                    inline => self.table_items("<inline>", &inline.clone(), chain, out),
                }
            }
            other => Err(format!("loot entry type {other} is not supported")),
        }
    }

    // ---- providers -------------------------------------------------------------

    /// A `ContextIntProvider`.
    fn int(&mut self, value: &Json) -> Result<i32> {
        if let Some(n) = value.as_f64() {
            return Ok(n as i32);
        }
        let kind = value.get("type").and_then(Json::as_str).unwrap_or("minecraft:constant");
        match kind.trim_start_matches("minecraft:") {
            "constant" => Ok(value.get("value").and_then(Json::as_f64).unwrap_or(0.0) as i32),
            "uniform" => {
                let min = self.int(value.get("min").ok_or("uniform without min")?)?;
                let max = self.int(value.get("max").ok_or("uniform without max")?)?;
                Ok(if min >= max { min } else { self.random.next_i32_bound(max - min + 1) + min })
            }
            "binomial" => {
                let n = self.int(value.get("n").ok_or("binomial without n")?)?;
                let p = self.float(value.get("p").ok_or("binomial without p")?)?;
                let mut result = 0;
                for _ in 0..n {
                    if self.random.next_f32() < p {
                        result += 1;
                    }
                }
                Ok(result)
            }
            other => Err(format!("int provider {other} is not supported")),
        }
    }

    /// A `ContextFloatProvider`.
    fn float(&mut self, value: &Json) -> Result<f32> {
        if let Some(n) = value.as_f64() {
            return Ok(n as f32);
        }
        let kind = value.get("type").and_then(Json::as_str).unwrap_or("minecraft:constant");
        match kind.trim_start_matches("minecraft:") {
            "constant" => Ok(value.get("value").and_then(Json::as_f64).unwrap_or(0.0) as f32),
            "uniform" => {
                let min = self.float(value.get("min").ok_or("uniform without min")?)?;
                let max = self.float(value.get("max").ok_or("uniform without max")?)?;
                Ok(if min >= max { min } else { self.random.next_f32() * (max - min) + min })
            }
            other => Err(format!("float provider {other} is not supported")),
        }
    }

    // ---- conditions ------------------------------------------------------------

    fn condition_opt(&mut self, condition: Option<&Json>) -> Result<bool> {
        match condition {
            None => Ok(true),
            Some(c) => self.condition(c),
        }
    }

    /// `LootItemCondition.test`.
    fn condition(&mut self, condition: &Json) -> Result<bool> {
        if let Some(reference) = condition.as_str() {
            let id = id_of(reference);
            let predicate = self.tables.predicate(self.registries, &id).ok_or_else(|| format!("missing predicate {id}"))?;
            return self.condition(&predicate);
        }
        let kind = condition.get("type").and_then(Json::as_str).ok_or("condition without type")?;
        match kind.trim_start_matches("minecraft:") {
            "survives_explosion" => match self.params.explosion_radius {
                Some(radius) => Ok(self.random.next_f32() <= 1.0 / radius),
                None => Ok(true),
            },
            "inverted" => Ok(!self.condition(condition.get("term").ok_or("inverted without term")?)?),
            "any_of" => {
                for term in condition.get("terms").and_then(Json::as_array).into_iter().flatten() {
                    if self.condition(term)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            "all_of" => {
                for term in condition.get("terms").and_then(Json::as_array).into_iter().flatten() {
                    if !self.condition(term)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            "random_chance" => {
                let chance = self.float(condition.get("chance").ok_or("random_chance without chance")?)?;
                Ok(self.random.next_f32() < chance)
            }
            "table_bonus" => {
                let enchantment = condition.get("enchantment").and_then(Json::as_str).unwrap_or_default();
                let level = self.tool_enchantment(enchantment);
                let chances: Vec<f32> = condition.get("chances").and_then(Json::as_array).into_iter().flatten().filter_map(Json::as_f64).map(|c| c as f32).collect();
                let chance = chances.get((level as usize).min(chances.len().saturating_sub(1))).copied().unwrap_or(0.0);
                Ok(self.random.next_f32() < chance)
            }
            "match_block" => Ok(self.match_block(condition)),
            "match_tool" => {
                let Some(tool) = &self.params.tool else { return Ok(false) };
                match condition.get("predicate") {
                    None => Ok(true),
                    Some(predicate) => self.item_predicate(tool, predicate),
                }
            }
            "entity_properties" => {
                if !self.params.this_entity {
                    return Ok(false);
                }
                match condition.get("predicate") {
                    Some(Json::Object(map)) if map.is_empty() => Ok(true),
                    None => Ok(true),
                    _ => Err("entity property predicates are not supported".to_owned()),
                }
            }
            other => Err(format!("loot condition {other} is not supported")),
        }
    }

    /// `MatchBlock`: the block (id, list or tag) and state properties.
    fn match_block(&self, condition: &Json) -> bool {
        let Some(state) = self.params.block_state else { return false };
        let blocks = &self.registries.blocks;
        let name = blocks.block(blocks.block_of(state)).name.as_str();
        let block_ok = match condition.get("blocks") {
            None => true,
            Some(Json::String(s)) if s.starts_with('#') => {
                let tag = id_of(&s[1..]);
                self.registries.block_tags.id(&tag).is_some_and(|t| self.registries.block_in_tag(state, t))
            }
            Some(Json::String(s)) => id_of(s) == name,
            Some(Json::Array(list)) => list.iter().filter_map(Json::as_str).any(|s| id_of(s) == name),
            _ => false,
        };
        if !block_ok {
            return false;
        }
        for (key, want) in condition.get("state").and_then(Json::as_object).into_iter().flatten() {
            let Some(have) = blocks.property(state, key) else { return false };
            let ok = match want {
                Json::String(v) => have == v,
                Json::Bool(b) => have == if *b { "true" } else { "false" },
                Json::Number(n) => have == n.to_string(),
                Json::Object(range) => {
                    let value: i64 = match have.parse() {
                        Ok(v) => v,
                        Err(_) => return false,
                    };
                    range.get("min").and_then(Json::as_i64).is_none_or(|m| value >= m) && range.get("max").and_then(Json::as_i64).is_none_or(|m| value <= m)
                }
                _ => false,
            };
            if !ok {
                return false;
            }
        }
        true
    }

    /// `ItemPredicate.test` for the parts vanilla loot uses.
    fn item_predicate(&self, stack: &ItemStack, predicate: &Json) -> Result<bool> {
        if stack.is_empty() && (predicate.get("items").is_some() || predicate.get("predicates").is_some()) {
            return Ok(false);
        }
        if let Some(items) = predicate.get("items") {
            let ok = match items {
                Json::String(s) if s.starts_with('#') => return Err("item tag predicates are not supported".to_owned()),
                Json::String(s) => id_of(s) == stack.id,
                Json::Array(list) => list.iter().filter_map(Json::as_str).any(|s| id_of(s) == stack.id),
                _ => false,
            };
            if !ok {
                return Ok(false);
            }
        }
        if let Some(predicates) = predicate.get("predicates").and_then(Json::as_object) {
            for (key, value) in predicates {
                match key.as_str() {
                    "minecraft:enchantments" => {
                        for wanted in value.as_array().into_iter().flatten() {
                            let enchantment = wanted.get("enchantments").and_then(Json::as_str).unwrap_or_default();
                            let level = self.tool_enchantment(enchantment);
                            let min = wanted.get("levels").and_then(|l| l.get("min")).and_then(Json::as_i64).unwrap_or(1);
                            if (level as i64) < min {
                                return Ok(false);
                            }
                        }
                    }
                    other => return Err(format!("item sub-predicate {other} is not supported")),
                }
            }
        }
        Ok(true)
    }

    /// The tool's level of an enchantment (`minecraft:enchantments` component).
    fn tool_enchantment(&self, enchantment: &str) -> i32 {
        let Some(tool) = &self.params.tool else { return 0 };
        let id = id_of(enchantment);
        tool.components
            .as_ref()
            .and_then(|c| c.get("minecraft:enchantments"))
            .and_then(|e| e.get(&id))
            .and_then(crate::nbt::Tag::as_i64)
            .unwrap_or(0) as i32
    }

    // ---- functions ---------------------------------------------------------------

    /// `LootItemFunction.apply` for a `modifier` (one function or a list).
    fn apply_modifier(&mut self, modifier: Option<&Json>, mut stack: ItemStack) -> Result<ItemStack> {
        match modifier {
            None => Ok(stack),
            Some(Json::Array(list)) => {
                for function in list {
                    stack = self.apply_function(function, stack)?;
                }
                Ok(stack)
            }
            Some(function) => self.apply_function(function, stack),
        }
    }

    fn apply_function(&mut self, function: &Json, mut stack: ItemStack) -> Result<ItemStack> {
        if let Some(condition) = function.get("condition") {
            if !self.condition(condition)? {
                return Ok(stack);
            }
        }
        let kind = function.get("type").and_then(Json::as_str).ok_or("function without type")?;
        match kind.trim_start_matches("minecraft:") {
            "set_count" => {
                let add = function.get("add").and_then(Json::as_bool).unwrap_or(false);
                let base = if add { stack.count } else { 0 };
                stack.count = base + self.int(function.get("count").ok_or("set_count without count")?)?;
            }
            "explosion_decay" => {
                if let Some(radius) = self.params.explosion_radius {
                    let probability = 1.0 / radius;
                    let mut result = 0;
                    for _ in 0..stack.count {
                        if self.random.next_f32() <= probability {
                            result += 1;
                        }
                    }
                    stack.count = result;
                }
            }
            "apply_bonus" => {
                if self.params.tool.is_some() {
                    let enchantment = function.get("enchantment").and_then(Json::as_str).unwrap_or_default();
                    let level = self.tool_enchantment(enchantment);
                    let formula = function.get("formula").and_then(Json::as_str).unwrap_or_default();
                    stack.count = match formula.trim_start_matches("minecraft:") {
                        "ore_drops" => {
                            if level > 0 {
                                let bonus = (self.random.next_i32_bound(level + 2) - 1).max(0);
                                stack.count * (bonus + 1)
                            } else {
                                stack.count
                            }
                        }
                        "uniform_bonus_count" => {
                            let multiplier = function.get("parameters").and_then(|p| p.get("bonusMultiplier")).and_then(Json::as_i64).unwrap_or(1) as i32;
                            stack.count + self.random.next_i32_bound(multiplier * level + 1)
                        }
                        "binomial_with_bonus_count" => {
                            let parameters = function.get("parameters");
                            let extra = parameters.and_then(|p| p.get("extra")).and_then(Json::as_i64).unwrap_or(0) as i32;
                            let probability = parameters.and_then(|p| p.get("probability")).and_then(Json::as_f64).unwrap_or(0.0) as f32;
                            let mut count = stack.count;
                            for _ in 0..level + extra {
                                if self.random.next_f32() < probability {
                                    count += 1;
                                }
                            }
                            count
                        }
                        other => return Err(format!("bonus formula {other} is not supported")),
                    };
                }
            }
            "limit_count" => {
                let limit = function.get("limit").ok_or("limit_count without limit")?;
                let (min, max) = match limit {
                    Json::Number(n) => (n.as_i64(), n.as_i64()),
                    other => (other.get("min").and_then(Json::as_i64), other.get("max").and_then(Json::as_i64)),
                };
                if let Some(min) = min {
                    stack.count = stack.count.max(min as i32);
                }
                if let Some(max) = max {
                    stack.count = stack.count.min(max as i32);
                }
            }
            // Components copied from a block entity or state: the copies are
            // cosmetic for drops without either.
            "copy_components" | "copy_state" => {
                if self.params.block_state.is_none() {
                    return Err(format!("{kind} without a block"));
                }
            }
            other => return Err(format!("loot function {other} is not supported")),
        }
        Ok(stack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::AnyRandom;
    use crate::registries::DataPaths;

    #[test]
    fn vanilla_block_tables_roll() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let mut sequences = RandomSequences::new(1);
        let mut random = AnyRandom::new(true, 5);
        let params = LootParams { tool: Some(ItemStack::empty()), ..LootParams::default() };
        for name in ["melon", "cobweb", "torch", "oak_leaves", "gravel", "wheat", "diamond_ore", "acacia_slab", "short_grass"] {
            let state = registries.blocks.parse_state(&format!("minecraft:{name}")).unwrap();
            let drops = registries.loot.block_drops(&registries, state, &params, &mut sequences, &mut random);
            assert!(drops.is_ok(), "{name}: {drops:?}");
        }
    }
}
