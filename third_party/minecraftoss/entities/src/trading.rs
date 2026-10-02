//! Villager trading offers from the pinned 26.3 data-driven trade sets
//! (`data/minecraft/trade_set`, `villager_trade` and their tags):
//! `AbstractVillager.addOffersFromTradeSet` picks the set's `amount` of
//! trades (without repeats unless `allow_duplicates`), each drawing from
//! the set's named random sequence (`RandomSequence`) through
//! `VillagerTrade.getOffer`: its `merchant_predicate` (the villager's type),
//! the result item and its `given_item_modifier` (enchanting, dyes, potions,
//! stew effects, maps; `filtered`/`discard`), the added cost an
//! enchantment brings, and the costs, uses, experience and price
//! multiplier. `MerchantOffer` carries uses, demand and special prices for
//! trading. Maps need a structure search, which is not ported: a map trade
//! comes out unmarked and its filter discards it, as vanilla's does where no
//! structure is found.
use crate::enchant::{add_enchantment, resolve_tag, Enchantments};
use anyhow::{Context as _, Result};
use minecraftoss_player::rng::{LootRandom, XoroshiroRandom};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

/// `ItemCost`: an item, how many, and the components it must have.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemCost {
    pub id: String,
    pub count: i32,
    pub components: Option<Value>,
}

/// A stack with its component patch.
#[derive(Clone, Debug, PartialEq)]
pub struct TradeItem {
    pub id: String,
    pub count: i32,
    pub components: Map<String, Value>,
}

/// `MerchantOffer`.
#[derive(Clone, Debug, PartialEq)]
pub struct MerchantOffer {
    pub buy: ItemCost,
    pub buy_b: Option<ItemCost>,
    pub sell: TradeItem,
    pub uses: i32,
    pub max_uses: i32,
    pub reward_exp: bool,
    pub special_price: i32,
    pub demand: i32,
    pub price_multiplier: f32,
    pub xp: i32,
}

impl ItemCost {
    /// `ItemCost.test`: the item, with every component the cost names
    /// (`DataComponentExactPredicate`).
    pub fn test(&self, item: &str, components: Option<&Value>) -> bool {
        if self.id != item {
            return false;
        }
        let Some(expected) = self.components.as_ref().and_then(Value::as_object) else { return true };
        expected.iter().all(|(key, value)| components.and_then(|c| c.get(key)) == Some(value))
    }
}

/// `Mth.floor(float)`.
fn floor_f32(value: f32) -> i32 {
    let i = value as i32;
    if value < i as f32 { i - 1 } else { i }
}

impl MerchantOffer {
    /// `getModifiedCostCount(baseCostA)`: the first cost raised by demand
    /// (never lowered by it) and moved by the special price, at least one
    /// and at most a stack of the item.
    pub fn cost_a_count(&self, max_stack: i32) -> i32 {
        let base = self.buy.count;
        let demand = floor_f32((base * self.demand) as f32 * self.price_multiplier).max(0);
        (base + demand + self.special_price).clamp(1, max_stack)
    }

    /// `satisfiedBy`: the first payment is the first cost's item in at
    /// least the modified count; the second is the second cost in at least
    /// its count, or empty when there is none.
    pub fn satisfied_by(&self, a: Option<(&str, i32, Option<&Value>)>, b: Option<(&str, i32, Option<&Value>)>, max_stack: i32) -> bool {
        let Some((id, count, components)) = a else { return false };
        if !self.buy.test(id, components) || count < self.cost_a_count(max_stack) {
            return false;
        }
        match (&self.buy_b, b) {
            (None, b) => b.is_none(),
            (Some(cost), Some((id, count, components))) => cost.test(id, components) && count >= cost.count,
            (Some(_), None) => false,
        }
    }

    /// `isOutOfStock`.
    pub fn out_of_stock(&self) -> bool {
        self.uses >= self.max_uses
    }

    /// `updateDemand`.
    pub fn update_demand(&mut self) {
        self.demand = self.demand + self.uses - (self.max_uses - self.uses);
    }

    /// `MerchantOffer.CODEC` as JSON (defaults left out).
    pub fn to_json(&self) -> Value {
        let cost = |c: &ItemCost| {
            let mut o = json!({ "id": c.id, "count": c.count });
            if let Some(components) = &c.components {
                o["components"] = components.clone();
            }
            o
        };
        let mut sell = json!({ "id": self.sell.id, "count": self.sell.count });
        if !self.sell.components.is_empty() {
            sell["components"] = Value::Object(self.sell.components.clone());
        }
        let mut o = json!({ "buy": cost(&self.buy), "sell": sell });
        if let Some(b) = &self.buy_b {
            o["buyB"] = cost(b);
        }
        if self.uses != 0 {
            o["uses"] = self.uses.into();
        }
        if self.max_uses != 4 {
            o["maxUses"] = self.max_uses.into();
        }
        if !self.reward_exp {
            o["rewardExp"] = false.into();
        }
        if self.special_price != 0 {
            o["specialPrice"] = self.special_price.into();
        }
        if self.demand != 0 {
            o["demand"] = self.demand.into();
        }
        if self.price_multiplier != 0.0 {
            o["priceMultiplier"] = json!(self.price_multiplier);
        }
        if self.xp != 1 {
            o["xp"] = self.xp.into();
        }
        o
    }

    /// `MerchantOffer.CODEC` from JSON (a saved villager's `Offers`).
    pub fn from_json(o: &Value) -> Option<Self> {
        let cost = |c: &Value| Some(ItemCost { id: c["id"].as_str()?.to_owned(), count: c["count"].as_i64().unwrap_or(1) as i32, components: c.get("components").cloned() });
        Some(Self {
            buy: cost(&o["buy"])?,
            buy_b: o.get("buyB").and_then(cost),
            sell: TradeItem {
                id: o["sell"]["id"].as_str()?.to_owned(),
                count: o["sell"]["count"].as_i64().unwrap_or(1) as i32,
                components: o["sell"]["components"].as_object().cloned().unwrap_or_default(),
            },
            uses: o["uses"].as_i64().unwrap_or(0) as i32,
            max_uses: o["maxUses"].as_i64().unwrap_or(4) as i32,
            // A boolean, or a byte from NBT.
            reward_exp: o["rewardExp"].as_bool().or_else(|| o["rewardExp"].as_i64().map(|v| v != 0)).unwrap_or(true),
            special_price: o["specialPrice"].as_i64().unwrap_or(0) as i32,
            demand: o["demand"].as_i64().unwrap_or(0) as i32,
            price_multiplier: o["priceMultiplier"].as_f64().unwrap_or(0.0) as f32,
            xp: o["xp"].as_i64().unwrap_or(1) as i32,
        })
    }
}

/// `DyeColor.getTextureDiffuseColor`, in `DyeColor.VALUES` order.
const DYE_COLORS: [u32; 16] = [
    16383998, 16351261, 13061821, 3847130, 16701501, 8439583, 15961002, 4673362, 10329495, 1481884, 8991416, 3949738, 8606770, 6192150, 11546150, 1908001,
];

/// The instantaneous effects (`MobEffect.isInstantaneous`), whose stew
/// durations stay in ticks.
fn instantaneous(effect: &str) -> bool {
    matches!(effect, "minecraft:instant_health" | "minecraft:instant_damage" | "minecraft:saturation")
}

/// What a trade's loot context carries: the villager's type
/// (`THIS_ENTITY`), and the random its trade set's sequence gives.
struct TradeContext<'a> {
    villager_type: &'a str,
    random: &'a mut XoroshiroRandom,
}

/// The level's named random sequences for trade sets (`RandomSequences`).
#[derive(Clone, Debug)]
pub struct TradeSequences {
    world_seed: u64,
    map: HashMap<String, XoroshiroRandom>,
}

impl TradeSequences {
    pub fn new(world_seed: u64) -> Self {
        Self { world_seed, map: HashMap::new() }
    }
}

impl Default for TradeSequences {
    fn default() -> Self {
        Self::new(0)
    }
}

/// The trade data offers are made from.
pub struct TradeBook {
    sets: HashMap<String, Value>,
    trades: HashMap<String, Value>,
    trade_tags: HashMap<String, Value>,
    potion_tags: HashMap<String, Value>,
    item_tags: HashMap<String, Value>,
    pub enchantments: Enchantments,
    /// `getDefaultMaxStackSize` of the items that are not 64.
    max_stack: HashMap<String, i32>,
}

impl TradeBook {
    /// From the data JAR and the item catalog (`enchantable`, `max_stack`).
    pub fn from_jar(jar: &Path, item_catalog: &Path) -> Result<Self> {
        let file = std::fs::File::open(jar).with_context(|| format!("open data JAR {}", jar.display()))?;
        let mut archive = zip::ZipArchive::new(file)?;
        let (mut sets, mut trades, mut trade_tags, mut potion_tags, mut item_tags, mut enchantment_tags) = (HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new());
        let mut enchantments = Vec::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let name = entry.name().to_owned();
            let Some(stem) = name.strip_prefix("data/minecraft/").and_then(|n| n.strip_suffix(".json")) else { continue };
            let slot = [
                ("trade_set/", &mut sets as &mut HashMap<String, Value>),
                ("villager_trade/", &mut trades),
                ("tags/villager_trade/", &mut trade_tags),
                ("tags/potion/", &mut potion_tags),
                ("tags/item/", &mut item_tags),
                ("tags/enchantment/", &mut enchantment_tags),
            ]
            .into_iter()
            .find_map(|(prefix, map)| stem.strip_prefix(prefix).map(|id| (id.to_owned(), map)));
            let enchantment = stem.strip_prefix("enchantment/").map(str::to_owned);
            if slot.is_none() && enchantment.is_none() {
                continue;
            }
            let mut source = String::new();
            entry.read_to_string(&mut source)?;
            let json: Value = serde_json::from_str(&source).with_context(|| format!("parse {name}"))?;
            if let Some((id, map)) = slot {
                map.insert(format!("minecraft:{id}"), json);
            } else if let Some(id) = enchantment {
                enchantments.push((format!("minecraft:{id}"), json));
            }
        }
        let catalog: Value = serde_json::from_str(&std::fs::read_to_string(item_catalog).with_context(|| format!("read {}", item_catalog.display()))?)?;
        let mut enchantable = HashMap::new();
        let mut max_stack = HashMap::new();
        for (id, item) in catalog["items"].as_object().into_iter().flatten() {
            if let Some(value) = item["enchantable"].as_i64() {
                enchantable.insert(id.clone(), value as i32);
            }
            if let Some(max) = item["max_stack"].as_i64() {
                max_stack.insert(id.clone(), max as i32);
            }
        }
        let enchantments = Enchantments::new(enchantments, &enchantment_tags, &item_tags, enchantable);
        Ok(Self { sets, trades, trade_tags, potion_tags, item_tags, enchantments, max_stack })
    }

    /// An item's maximum stack size (`getDefaultMaxStackSize`).
    pub fn max_stack(&self, item: &str) -> i32 {
        self.max_stack.get(item).copied().unwrap_or(64)
    }

    /// `VillagerProfession.getTrades(level)`: `minecraft:<profession>/level_<n>`.
    pub fn trade_set_for(&self, profession: &str, level: i32) -> Option<String> {
        let name = profession.strip_prefix("minecraft:").unwrap_or(profession);
        let key = format!("minecraft:{name}/level_{level}");
        self.sets.contains_key(&key).then_some(key)
    }

    /// `addOffersFromTradeSet`: the trade set's offers for a villager of
    /// `villager_type`.
    pub fn offers_from(&self, set_key: &str, villager_type: &str, sequences: &mut TradeSequences) -> Vec<MerchantOffer> {
        let Some(set) = self.sets.get(set_key).cloned() else { return Vec::new() };
        let sequence = set["random_sequence"].as_str().unwrap_or(set_key).to_owned();
        let seed = sequences.world_seed;
        let mut random = sequences.map.remove(&sequence).unwrap_or_else(|| XoroshiroRandom::for_sequence(seed, &sequence));
        let offers = {
            let mut ctx = TradeContext { villager_type, random: &mut random };
            let amount = self.int(&set["amount"], &mut ctx);
            let mut candidates = self.trade_list(&set["trades"]);
            let duplicates = set["allow_duplicates"].as_bool().unwrap_or(false);
            let mut offers = Vec::new();
            while (offers.len() as i32) < amount && !candidates.is_empty() {
                let roll = ctx.random.next_int(candidates.len() as u32) as usize;
                let trade = if duplicates { candidates[roll].clone() } else { candidates.remove(roll) };
                match self.offer(&trade, &mut ctx) {
                    Some(offer) => offers.push(offer),
                    None if duplicates => {
                        candidates.remove(roll);
                    }
                    None => {}
                }
            }
            offers
        };
        sequences.map.insert(sequence, random);
        offers
    }

    /// `Villager.updateTrades`: the offers of the profession's set for its
    /// level.
    pub fn villager_offers(&self, villager_type: &str, profession: &str, level: i32, sequences: &mut TradeSequences) -> Vec<MerchantOffer> {
        self.trade_set_for(profession, level).map_or_else(Vec::new, |set| self.offers_from(&set, villager_type, sequences))
    }

    /// A holder set of trades, by ID, in its order.
    fn trade_list(&self, value: &Value) -> Vec<String> {
        match value {
            Value::String(s) => match s.strip_prefix('#') {
                Some(tag) => {
                    let mut out = Vec::new();
                    resolve_tag(&self.trade_tags, tag, &mut out, 0);
                    out
                }
                None => vec![s.clone()],
            },
            Value::Array(list) => list.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
            _ => Vec::new(),
        }
    }

    /// `ContextIntProvider.getInt`.
    fn int(&self, value: &Value, ctx: &mut TradeContext) -> i32 {
        match value {
            Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
            Value::Object(o) => match o.get("type").and_then(Value::as_str).unwrap_or("minecraft:constant").trim_start_matches("minecraft:") {
                "constant" => o.get("value").and_then(Value::as_f64).unwrap_or(0.0) as i32,
                "uniform" => {
                    let (min, max) = (self.int(&o["min"], ctx), self.int(&o["max"], ctx));
                    if min >= max {
                        min
                    } else {
                        ctx.random.next_int((max - min + 1) as u32) as i32 + min
                    }
                }
                "binomial" => {
                    let n = self.int(&o["n"], ctx);
                    let p = self.float(&o["p"], ctx);
                    (0..n).filter(|_| ctx.random.next_float() < p).count() as i32
                }
                "add" => o["inputs"].as_array().into_iter().flatten().map(|v| i64::from(self.int(v, ctx))).sum::<i64>().clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
                _ => 0,
            },
            _ => 0,
        }
    }

    /// `ContextFloatProvider.getFloat`.
    fn float(&self, value: &Value, ctx: &mut TradeContext) -> f32 {
        match value {
            Value::Number(n) => n.as_f64().unwrap_or(0.0) as f32,
            Value::Object(o) => match o.get("type").and_then(Value::as_str).unwrap_or("minecraft:constant").trim_start_matches("minecraft:") {
                "constant" => o.get("value").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                "uniform" => {
                    let (min, max) = (self.float(&o["min"], ctx), self.float(&o["max"], ctx));
                    if min >= max {
                        min
                    } else {
                        ctx.random.next_float() * (max - min) + min
                    }
                }
                _ => 0.0,
            },
            _ => 0.0,
        }
    }

    /// `merchant_predicate`: `entity_properties` of the villager (its type).
    fn condition(&self, condition: &Value, ctx: &TradeContext) -> bool {
        let kind = condition["type"].as_str().or_else(|| condition["condition"].as_str()).unwrap_or("");
        match kind.trim_start_matches("minecraft:") {
            "entity_properties" => {
                let variants = &condition["predicate"]["minecraft:predicates"]["minecraft:villager/variant"];
                match variants {
                    Value::Array(list) => list.iter().any(|v| v.as_str() == Some(ctx.villager_type)),
                    Value::String(s) => s == ctx.villager_type,
                    _ => true,
                }
            }
            "inverted" => !self.condition(&condition["term"], ctx),
            "all_of" => condition["terms"].as_array().into_iter().flatten().all(|t| self.condition(t, ctx)),
            "any_of" => condition["terms"].as_array().into_iter().flatten().any(|t| self.condition(t, ctx)),
            _ => true,
        }
    }

    /// `VillagerTrade.getOffer`.
    fn offer(&self, id: &str, ctx: &mut TradeContext) -> Option<MerchantOffer> {
        let trade = self.trades.get(id)?;
        if let Some(predicate) = trade.get("merchant_predicate") {
            if !self.condition(predicate, ctx) {
                return None;
            }
        }
        let gives = &trade["gives"];
        let mut item = TradeItem {
            id: gives["id"].as_str()?.to_owned(),
            count: gives["count"].as_i64().unwrap_or(1) as i32,
            components: gives["components"].as_object().cloned().unwrap_or_default(),
        };
        if let Some(modifier) = trade.get("given_item_modifier") {
            item = self.apply(modifier, item, ctx)?;
        }
        let mut additional = item.components.remove("minecraft:additional_trade_cost").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        if let Some(Value::String(set)) = trade.get("double_trade_price_enchantments") {
            let stored = item.components.get("minecraft:stored_enchantments").and_then(Value::as_object);
            let tag = set.trim_start_matches('#');
            if stored.is_some_and(|m| m.keys().any(|e| self.enchantments.tag_contains(tag, e))) {
                additional *= 2;
            }
        }
        let buy = self.cost(&trade["wants"], ctx, additional);
        if buy.count < 1 {
            return None;
        }
        let buy_b = match trade.get("additional_wants") {
            Some(wants) => {
                let cost = self.cost(wants, ctx, 0);
                if cost.count < 1 {
                    return None;
                }
                Some(cost)
            }
            None => None,
        };
        let max_uses = trade.get("max_uses").map_or(4, |v| self.int(v, ctx)).max(1);
        let xp = trade.get("xp").map_or(1, |v| self.int(v, ctx)).max(0);
        let price_multiplier = trade.get("reputation_discount").map_or(0.0, |v| self.float(v, ctx)).max(0.0);
        Some(MerchantOffer { buy, buy_b, sell: item, uses: 0, max_uses, reward_exp: true, special_price: 0, demand: 0, price_multiplier, xp })
    }

    /// `TradeCost.toItemCost`: its count plus the added cost, within the
    /// item's stack size.
    fn cost(&self, wants: &Value, ctx: &mut TradeContext, additional: i32) -> ItemCost {
        let id = wants["id"].as_str().unwrap_or("minecraft:air").to_owned();
        let count = wants.get("count").map_or(1, |v| self.int(v, ctx));
        let max = self.max_stack.get(&id).copied().unwrap_or(64);
        ItemCost { count: (count + additional).clamp(0, max), components: wants.get("components").cloned(), id }
    }

    /// A loot function (or a list of them) on the result; `None` once
    /// discarded (`ItemStack.EMPTY`).
    fn apply(&self, function: &Value, item: TradeItem, ctx: &mut TradeContext) -> Option<TradeItem> {
        if let Value::Array(list) = function {
            let mut item = item;
            for f in list {
                item = self.apply(f, item, ctx)?;
            }
            return Some(item);
        }
        let mut item = item;
        match function["function"].as_str().or_else(|| function["type"].as_str()).unwrap_or("").trim_start_matches("minecraft:") {
            "discard" => return None,
            "filtered" => {
                let branch = if self.item_matches(&function["item_filter"], &item) { function.get("on_pass") } else { function.get("on_fail") };
                if let Some(branch) = branch {
                    return self.apply(branch, item, ctx);
                }
            }
            "enchant_randomly" => {
                let book = item.id == "minecraft:book";
                let check = !book && function["only_compatible"].as_bool().unwrap_or(true);
                let candidates: Vec<usize> = self.enchantments.source(function.get("options")).into_iter().filter(|&e| !check || self.enchantments.list[e].can_enchant(&item.id)).collect();
                if candidates.is_empty() {
                    return Some(item);
                }
                let chosen = candidates[ctx.random.next_int(candidates.len() as u32) as usize];
                let e = &self.enchantments.list[chosen];
                let level = if 1 >= e.max_level { 1 } else { ctx.random.next_int(e.max_level as u32) as i32 + 1 };
                if book {
                    item = TradeItem { id: "minecraft:enchanted_book".into(), count: 1, components: Map::new() };
                }
                add_enchantment(&item.id, &mut item.components, &e.id, level);
                if function["include_additional_cost_component"].as_bool().unwrap_or(false) {
                    let cost = 2 + ctx.random.next_int((5 + level * 10) as u32) as i32 + 3 * level;
                    item.components.insert("minecraft:additional_trade_cost".into(), cost.into());
                }
            }
            "enchant_with_levels" => {
                let cost = self.int(&function["levels"], ctx);
                let source = self.enchantments.source(function.get("options"));
                let chosen = self.enchantments.select(ctx.random, &item.id, cost, &source);
                if item.id == "minecraft:book" {
                    item = TradeItem { id: "minecraft:enchanted_book".into(), count: 1, components: Map::new() };
                }
                for (e, level) in chosen {
                    let id = self.enchantments.list[e].id.clone();
                    add_enchantment(&item.id, &mut item.components, &id, level);
                }
                if function["include_additional_cost_component"].as_bool().unwrap_or(false) && cost > 0 {
                    item.components.insert("minecraft:additional_trade_cost".into(), cost.into());
                }
            }
            "set_random_dyes" => {
                let rolls = self.int(&function["number_of_dyes"], ctx);
                if rolls > 0 {
                    let dyes: Vec<u32> = (0..rolls).map(|_| DYE_COLORS[ctx.random.next_int(16) as usize]).collect();
                    let current = item.components.get("minecraft:dyed_color").and_then(Value::as_u64).map(|c| c as u32);
                    item.count = 1;
                    item.components.insert("minecraft:dyed_color".into(), apply_dyes(current, &dyes).into());
                }
            }
            "set_stew_effect" => {
                let effects = function["effects"].as_array().cloned().unwrap_or_default();
                if item.id == "minecraft:suspicious_stew" && !effects.is_empty() {
                    let entry = &effects[ctx.random.next_int(effects.len() as u32) as usize];
                    let effect = entry["type"].as_str().unwrap_or("").to_owned();
                    let mut duration = self.int(&entry["duration"], ctx);
                    if !instantaneous(&effect) {
                        duration *= 20;
                    }
                    let list = item.components.entry("minecraft:suspicious_stew_effects").or_insert_with(|| Value::Array(Vec::new()));
                    if let Value::Array(list) = list {
                        let mut e = json!({ "id": effect });
                        if duration != 160 {
                            e["duration"] = duration.into();
                        }
                        list.push(e);
                    }
                }
            }
            "set_random_potion" => {
                let options: Vec<String> = match function["options"].as_str().and_then(|s| s.strip_prefix('#')) {
                    Some(tag) => {
                        let mut out = Vec::new();
                        resolve_tag(&self.potion_tags, tag, &mut out, 0);
                        out
                    }
                    None => function["options"].as_str().map(|s| vec![s.to_owned()]).unwrap_or_default(),
                };
                if !options.is_empty() {
                    let potion = options[ctx.random.next_int(options.len() as u32) as usize].clone();
                    item.components.insert("minecraft:potion_contents".into(), json!({ "potion": potion }));
                }
            }
            "set_potion" => {
                if let Some(potion) = function["id"].as_str() {
                    item.components.insert("minecraft:potion_contents".into(), json!({ "potion": potion }));
                }
            }
            // `exploration_map`: no structure search here, so no map.
            "exploration_map" => {}
            _ => {}
        }
        Some(item)
    }

    /// `ItemPredicate` as `filtered` uses it: the items, and component
    /// predicates asking a component be there (with stored enchantments,
    /// at least one).
    fn item_matches(&self, filter: &Value, item: &TradeItem) -> bool {
        if let Some(items) = filter.get("items") {
            let ids: Vec<String> = match items {
                Value::String(s) => match s.strip_prefix('#') {
                    Some(tag) => {
                        let mut out = Vec::new();
                        resolve_tag(&self.item_tags, tag, &mut out, 0);
                        out
                    }
                    None => vec![s.clone()],
                },
                Value::Array(list) => list.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
                _ => Vec::new(),
            };
            if !ids.contains(&item.id) {
                return false;
            }
        }
        for (key, predicate) in filter["predicates"].as_object().into_iter().flatten() {
            let Some(component) = item.components.get(key) else { return false };
            if key == "minecraft:stored_enchantments" || key == "minecraft:enchantments" {
                let wanted = predicate.as_array().map_or(0, Vec::len);
                if wanted > 0 && component.as_object().is_none_or(Map::is_empty) {
                    return false;
                }
            }
        }
        true
    }
}

/// `DyedItemColor.applyDyes`: the colours averaged, scaled back to their
/// average brightness.
pub fn apply_dyes(current: Option<u32>, dyes: &[u32]) -> u32 {
    let (mut red_total, mut green_total, mut blue_total, mut intensity_total, mut count) = (0i32, 0i32, 0i32, 0i32, 0i32);
    for color in current.into_iter().chain(dyes.iter().copied()) {
        let (r, g, b) = (((color >> 16) & 0xff) as i32, ((color >> 8) & 0xff) as i32, (color & 0xff) as i32);
        intensity_total += r.max(g).max(b);
        red_total += r;
        green_total += g;
        blue_total += b;
        count += 1;
    }
    let (mut r, mut g, mut b) = (red_total / count, green_total / count, blue_total / count);
    let average = intensity_total as f32 / count as f32;
    let result = r.max(g).max(b) as f32;
    r = (r as f32 * average / result) as i32;
    g = (g as f32 * average / result) as i32;
    b = (b as f32 * average / result) as i32;
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}
