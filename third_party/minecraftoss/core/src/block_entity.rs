//! Block entity data as vanilla saves it (`BlockEntity.saveWithFullMetadata`).
//!
//! A block entity is kept as its saved NBT compound. `BlockEntities` knows
//! each entity block's type and default data (from the harness block entity
//! catalog, `newBlockEntity` at the default state) and reproduces
//! `BlockEntity.loadStatic` followed by a save for the data generation
//! writes: structure template tags, loot tables and spawner entities.
//!
//! Source-informed from the pinned 26.3 `loadAdditional`/`saveAdditional`
//! methods of the block entity classes, `RandomizableContainer`,
//! `ContainerHelper`, `ItemStack.MAP_CODEC` and `BaseSpawner`.

use crate::chunk::BlockEntityStore;
use crate::nbt::Tag;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Entity blocks' block entity types and default saved data.
pub struct BlockEntities {
    /// Block name to (block entity type, default NBT without position).
    defaults: HashMap<String, (String, Tag)>,
}

impl BlockEntities {
    /// Loads `artifacts/block-entity-catalog/26.3.json`.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let json: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if json["schema_version"] != 1 || json["minecraft_version"] != "26.3" {
            return Err(format!("{}: unsupported block entity catalog", path.display()));
        }
        let mut defaults = HashMap::new();
        for (block, info) in json["defaults"].as_object().ok_or("catalog has no defaults")? {
            let kind = info["type"].as_str().ok_or("default without a type")?.to_owned();
            let mut nbt = decode(info["nbt"].as_str().ok_or("default without NBT")?)?;
            if let Tag::Compound(map) = &mut nbt {
                for key in ["x", "y", "z"] {
                    map.remove(key);
                }
            }
            defaults.insert(block.clone(), (kind, nbt));
        }
        Ok(Self { defaults })
    }

    /// `WorldGenRegion.getBlockEntity` for a proto chunk's store: an existing
    /// block entity, or one made from the pending tag (a `DUMMY` becomes the
    /// block's default) and kept. `block` is the block at the position.
    pub fn get_mut<'s>(&self, store: &'s mut BlockEntityStore, pos: (i32, i32, i32), block: &str) -> Option<&'s mut Tag> {
        if !store.entities.contains_key(&pos) {
            let tag = store.pending.get(&pos)?;
            let entity = if is_dummy(tag) { self.default_nbt(block, pos) } else { self.load_and_save(block, pos, tag) }?;
            store.entities.insert(pos, entity);
        }
        store.entities.get_mut(&pos)
    }

    /// `LevelChunk(ServerLevel, ProtoChunk)` and
    /// `registerAllBlockEntitiesAfterLevelLoad`: block entities stay where
    /// the block still takes their type, and pending tags are promoted.
    /// `block_at` names the block at a position.
    pub fn promote(&self, store: &mut BlockEntityStore, block_at: impl Fn((i32, i32, i32)) -> String) {
        let mut entities = std::mem::take(&mut store.entities);
        entities.retain(|&pos, nbt| {
            let kind = nbt.get("id").and_then(Tag::as_str);
            kind.is_some() && self.type_of(&block_at(pos)) == kind
        });
        for (pos, tag) in std::mem::take(&mut store.pending) {
            if entities.contains_key(&pos) {
                continue;
            }
            let block = block_at(pos);
            let entity = if is_dummy(&tag) { self.default_nbt(&block, pos) } else { self.load_and_save(&block, pos, &tag) };
            if let Some(entity) = entity {
                entities.insert(pos, entity);
            }
        }
        store.entities = entities;
    }

    /// The block entity type of an entity block (`minecraft:chest`).
    pub fn type_of(&self, block: &str) -> Option<&str> {
        self.defaults.get(block).map(|(kind, _)| kind.as_str())
    }

    /// `EntityBlock.newBlockEntity` saved at a position.
    pub fn default_nbt(&self, block: &str, pos: (i32, i32, i32)) -> Option<Tag> {
        let (_, nbt) = self.defaults.get(block)?;
        let mut nbt = nbt.clone();
        place(&mut nbt, pos);
        Some(nbt)
    }

    /// `BlockEntity.loadStatic(pos, state, tag)` saved with full metadata:
    /// what a block entity created from `input` saves. `None` when the block
    /// has no block entity.
    pub fn load_and_save(&self, block: &str, pos: (i32, i32, i32), input: &Tag) -> Option<Tag> {
        let (kind, defaults) = self.defaults.get(block)?;
        let empty = BTreeMap::new();
        let input = input.as_compound().unwrap_or(&empty);
        let mut out = BTreeMap::new();
        out.insert("id".to_owned(), Tag::String(kind.clone()));
        out.insert("components".to_owned(), input.get("components").filter(|c| c.as_compound().is_some()).cloned().unwrap_or_else(|| Tag::Compound(BTreeMap::new())));
        let get = |key: &str| input.get(key);
        let int = |key: &str, default: i32| get(key).and_then(Tag::as_i64).map_or(default, |v| v as i32);
        let long = |key: &str, default: i64| get(key).and_then(Tag::as_i64).unwrap_or(default);
        let float = |key: &str, default: f32| get(key).and_then(Tag::as_f64).map_or(default, |v| v as f32);
        let boolean = |key: &str| get(key).and_then(Tag::as_i64).is_some_and(|v| v != 0);
        let mut put = |key: &str, tag: Tag| {
            out.insert(key.to_owned(), tag);
        };
        let custom_name = |put: &mut dyn FnMut(&str, Tag), key: &str| {
            if let Some(name) = get(key).and_then(component) {
                put(key, name);
            }
        };
        match kind.as_str() {
            "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:barrel" | "minecraft:dispenser" | "minecraft:dropper" | "minecraft:hopper" | "minecraft:shulker_box" | "minecraft:crafter" => {
                lock(input, &mut put);
                custom_name(&mut put, "CustomName");
                if kind == "minecraft:crafter" {
                    put("crafting_ticks_remaining", Tag::Int(int("crafting_ticks_remaining", 0)));
                }
                if !loot_table(input, &mut put) {
                    let items = items(get("Items"), container_size(kind));
                    if kind != "minecraft:shulker_box" || !items.is_empty() {
                        put("Items", Tag::List(items));
                    }
                }
                match kind.as_str() {
                    "minecraft:hopper" => put("TransferCooldown", Tag::Int(int("TransferCooldown", -1))),
                    "minecraft:crafter" => {
                        let disabled: Vec<i32> = get("disabled_slots").and_then(Tag::as_ints).unwrap_or_default().into_iter().filter(|s| (0..9).contains(s)).collect();
                        let mut slots = [false; 9];
                        for s in disabled {
                            slots[s as usize] = true;
                        }
                        put("disabled_slots", Tag::IntArray((0..9).filter(|&i| slots[i as usize]).collect()));
                        put("triggered", Tag::Int(int("triggered", 0)));
                    }
                    _ => {}
                }
            }
            "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => {
                lock(input, &mut put);
                custom_name(&mut put, "CustomName");
                put("cooking_time_spent", Tag::Int(int("cooking_time_spent", 0)));
                put("cooking_total_time", Tag::Int(int("cooking_total_time", 0)));
                put("lit_time_remaining", Tag::Int(int("lit_time_remaining", 0)));
                put("lit_total_time", Tag::Int(int("lit_total_time", 0)));
                put("speed_multiplier", Tag::Float(float("speed_multiplier", 1.0)));
                put("Items", Tag::List(items(get("Items"), 3)));
                put("RecipesUsed", recipes_used(get("RecipesUsed")));
            }
            "minecraft:brewing_stand" => {
                lock(input, &mut put);
                custom_name(&mut put, "CustomName");
                put("BrewTime", Tag::Int(int("BrewTime", 0)));
                put("total_brew_time", Tag::Int(int("total_brew_time", 400)));
                put("Items", Tag::List(items(get("Items"), 5)));
                put("Fuel", Tag::Int(int("Fuel", 0)));
                put("total_fuel", Tag::Int(int("total_fuel", 20)));
                put("speed_multiplier", Tag::Float(float("speed_multiplier", 1.0)));
            }
            "minecraft:campfire" => {
                put("Items", Tag::List(items(get("Items"), 4)));
                let times = |key: &str| {
                    let mut out = vec![0; 4];
                    if let Some(v) = get(key).and_then(Tag::as_ints) {
                        out[..v.len().min(4)].copy_from_slice(&v[..v.len().min(4)]);
                    }
                    Tag::IntArray(out)
                };
                put("CookingTimes", times("CookingTimes"));
                put("CookingTotalTimes", times("CookingTotalTimes"));
            }
            "minecraft:chiseled_bookshelf" => {
                put("Items", Tag::List(items(get("Items"), 6)));
                put("last_interacted_slot", Tag::Int(int("last_interacted_slot", -1)));
            }
            "minecraft:shelf" => {
                put("Items", Tag::List(items(get("Items"), 3)));
                put("align_items_to_bottom", Tag::Byte(i8::from(boolean("align_items_to_bottom"))));
            }
            "minecraft:banner" => {
                if let Some(patterns) = get("patterns").and_then(Tag::as_list).filter(|p| !p.is_empty()) {
                    put("patterns", Tag::List(patterns.to_vec()));
                }
                custom_name(&mut put, "CustomName");
            }
            "minecraft:sign" | "minecraft:hanging_sign" => {
                if boolean("allow_op_features") {
                    put("allow_op_features", Tag::Byte(1));
                }
                put("front_text", sign_text(get("front_text")));
                put("back_text", sign_text(get("back_text")));
                put("is_waxed", Tag::Byte(i8::from(boolean("is_waxed"))));
            }
            "minecraft:mob_spawner" => {
                put("Delay", Tag::Short(int("Delay", 20) as i16));
                put("MinSpawnDelay", Tag::Short(int("MinSpawnDelay", 200) as i16));
                put("MaxSpawnDelay", Tag::Short(int("MaxSpawnDelay", 800) as i16));
                put("SpawnCount", Tag::Short(int("SpawnCount", 4) as i16));
                put("MaxNearbyEntities", Tag::Short(int("MaxNearbyEntities", 6) as i16));
                put("RequiredPlayerRange", Tag::Short(int("RequiredPlayerRange", 16) as i16));
                put("SpawnRange", Tag::Short(int("SpawnRange", 4) as i16));
                let data = get("SpawnData").filter(|d| d.as_compound().is_some()).cloned();
                if let Some(data) = &data {
                    put("SpawnData", data.clone());
                }
                let potentials = match get("SpawnPotentials").and_then(Tag::as_list) {
                    Some(list) => list.to_vec(),
                    None => {
                        let mut entry = BTreeMap::new();
                        entry.insert("data".to_owned(), data.unwrap_or_else(empty_spawn_data));
                        entry.insert("weight".to_owned(), Tag::Int(1));
                        vec![Tag::Compound(entry)]
                    }
                };
                put("SpawnPotentials", Tag::List(potentials));
            }
            "minecraft:beehive" => {
                put("bees", Tag::List(get("bees").and_then(Tag::as_list).map(<[Tag]>::to_vec).unwrap_or_default()));
                if let Some(flower) = get("flower_pos").and_then(Tag::as_ints).filter(|p| p.len() == 3) {
                    put("flower_pos", Tag::IntArray(flower));
                }
            }
            "minecraft:brushable_block" | "minecraft:decorated_pot" => {
                if kind == "minecraft:decorated_pot" {
                    if let Some(sherds) = get("sherds").filter(|s| s.as_compound().is_some_and(|m| !m.is_empty())) {
                        put("sherds", sherds.clone());
                    }
                }
                if !loot_table(input, &mut put) {
                    if let Some(item) = get("item").and_then(item_stack) {
                        put("item", item);
                    }
                }
            }
            "minecraft:end_gateway" => {
                put("Age", Tag::Long(long("Age", 0)));
                if let Some(exit) = get("exit_portal").and_then(Tag::as_ints).filter(|p| p.len() == 3) {
                    put("exit_portal", Tag::IntArray(exit));
                }
                if boolean("ExactTeleport") {
                    put("ExactTeleport", Tag::Byte(1));
                }
            }
            "minecraft:comparator" => put("OutputSignal", Tag::Int(int("OutputSignal", 0))),
            "minecraft:lectern" => {
                if let Some(book) = get("Book").and_then(item_stack) {
                    put("Book", book);
                    put("Page", Tag::Int(int("Page", 0).max(0)));
                }
            }
            "minecraft:skull" => {
                for key in ["profile", "note_block_sound"] {
                    if let Some(value) = get(key) {
                        put(key, value.clone());
                    }
                }
                custom_name(&mut put, "custom_name");
            }
            "minecraft:jigsaw" => {
                for (key, default) in [("name", "minecraft:empty"), ("target", "minecraft:empty"), ("pool", "minecraft:empty"), ("final_state", "minecraft:air")] {
                    put(key, Tag::String(get(key).and_then(Tag::as_str).unwrap_or(default).to_owned()));
                }
                // `StructureTemplate.getDefaultJointType` needs the block state;
                // generation never saves jigsaw blocks, so the catalog default stands in.
                let joint = get("joint").and_then(Tag::as_str).or_else(|| defaults.get("joint").and_then(Tag::as_str)).unwrap_or("rollable");
                put("joint", Tag::String(joint.to_owned()));
                put("placement_priority", Tag::Int(int("placement_priority", 0)));
                put("selection_priority", Tag::Int(int("selection_priority", 0)));
            }
            "minecraft:sculk_sensor" | "minecraft:calibrated_sculk_sensor" | "minecraft:sculk_shrieker" => {
                if kind == "minecraft:sculk_shrieker" {
                    put("warning_level", Tag::Int(int("warning_level", 0)));
                } else {
                    put("last_vibration_frequency", Tag::Int(int("last_vibration_frequency", 0)));
                }
                let listener = get("listener").filter(|l| l.as_compound().is_some()).or_else(|| defaults.get("listener")).cloned();
                if let Some(listener) = listener {
                    put("listener", listener);
                }
            }
            "minecraft:sculk_catalyst" => put("cursors", Tag::List(get("cursors").and_then(Tag::as_list).map(<[Tag]>::to_vec).unwrap_or_default())),
            "minecraft:creaking_heart" => {
                if let Some(uuid) = get("creaking").and_then(Tag::as_ints).filter(|u| u.len() == 4) {
                    put("creaking", Tag::IntArray(uuid));
                }
            }
            "minecraft:vault" => {
                put("config", get("config").map_or_else(|| defaults.get("config").cloned().unwrap_or_else(|| Tag::Compound(BTreeMap::new())), vault_config));
                for key in ["shared_data", "server_data"] {
                    let value = get(key).filter(|v| v.as_compound().is_some()).or_else(|| defaults.get(key)).cloned();
                    if let Some(value) = value {
                        put(key, value);
                    }
                }
            }
            "minecraft:trial_spawner" => {
                // `TrialSpawner.store` writes back the configs and state data
                // templates carry, in the same form.
                for (key, value) in input {
                    if !["id", "x", "y", "z", "components", "keepPacked"].contains(&key.as_str()) {
                        put(key, value.clone());
                    }
                }
            }
            _ => {
                // Types generation does not configure: their default fields,
                // taking an input value of the same kind where one exists.
                if let Tag::Compound(map) = defaults {
                    for (key, default) in map {
                        if key == "id" || key == "components" {
                            continue;
                        }
                        let value = get(key).and_then(|v| coerce(v, default)).unwrap_or_else(|| default.clone());
                        put(key, value);
                    }
                }
            }
        }
        let mut nbt = Tag::Compound(out);
        place(&mut nbt, pos);
        Some(nbt)
    }
}

/// A proto chunk's placeholder for a block entity not yet created.
pub fn dummy((x, y, z): (i32, i32, i32)) -> Tag {
    let mut map = BTreeMap::new();
    map.insert("id".to_owned(), Tag::String("DUMMY".to_owned()));
    map.insert("x".to_owned(), Tag::Int(x));
    map.insert("y".to_owned(), Tag::Int(y));
    map.insert("z".to_owned(), Tag::Int(z));
    Tag::Compound(map)
}

fn is_dummy(tag: &Tag) -> bool {
    tag.get("id").and_then(Tag::as_str) == Some("DUMMY")
}

/// `RandomizableContainer.setLootTable(table, seed)` on a saved block
/// entity: the loot table replaces saved items.
pub fn set_loot_table(nbt: &mut Tag, table: &str, seed: i64) {
    if let Tag::Compound(map) = nbt {
        map.remove("Items");
        map.remove("item");
        map.insert("LootTable".to_owned(), Tag::String(table.to_owned()));
        if seed != 0 {
            map.insert("LootTableSeed".to_owned(), Tag::Long(seed));
        } else {
            map.remove("LootTableSeed");
        }
    }
}

/// Whether a block entity type is a `RandomizableContainer`.
pub fn is_randomizable(kind: &str) -> bool {
    matches!(
        kind,
        "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:barrel" | "minecraft:dispenser" | "minecraft:dropper" | "minecraft:hopper" | "minecraft:shulker_box" | "minecraft:crafter" | "minecraft:decorated_pot"
    )
}

fn place(nbt: &mut Tag, (x, y, z): (i32, i32, i32)) {
    if let Tag::Compound(map) = nbt {
        map.insert("x".into(), Tag::Int(x));
        map.insert("y".into(), Tag::Int(y));
        map.insert("z".into(), Tag::Int(z));
    }
}

/// A value converted to the default's tag kind, as `getIntOr` and friends
/// read any numeric tag.
fn coerce(value: &Tag, default: &Tag) -> Option<Tag> {
    Some(match default {
        Tag::Byte(_) => Tag::Byte(value.as_i64()? as i8),
        Tag::Short(_) => Tag::Short(value.as_i64()? as i16),
        Tag::Int(_) => Tag::Int(value.as_i64()? as i32),
        Tag::Long(_) => Tag::Long(value.as_i64()?),
        Tag::Float(_) => Tag::Float(value.as_f64()? as f32),
        Tag::Double(_) => Tag::Double(value.as_f64()?),
        Tag::String(_) => Tag::String(value.as_str()?.to_owned()),
        _ if std::mem::discriminant(value) == std::mem::discriminant(default) => value.clone(),
        _ => return None,
    })
}

/// `RandomizableContainer.tryLoadLootTable` then `trySaveLootTable`.
fn loot_table(input: &BTreeMap<String, Tag>, put: &mut impl FnMut(&str, Tag)) -> bool {
    let Some(table) = input.get("LootTable").and_then(Tag::as_str) else { return false };
    put("LootTable", Tag::String(table.to_owned()));
    let seed = input.get("LootTableSeed").and_then(Tag::as_i64).unwrap_or(0);
    if seed != 0 {
        put("LootTableSeed", Tag::Long(seed));
    }
    true
}

/// `LockCode`: a non-empty `lock` predicate is kept.
fn lock(input: &BTreeMap<String, Tag>, put: &mut impl FnMut(&str, Tag)) {
    if let Some(lock) = input.get("lock").filter(|l| l.as_compound().is_some_and(|m| !m.is_empty())) {
        put("lock", lock.clone());
    }
}

fn container_size(kind: &str) -> usize {
    match kind {
        "minecraft:dispenser" | "minecraft:dropper" | "minecraft:crafter" => 9,
        "minecraft:hopper" => 5,
        _ => 27,
    }
}

/// `ContainerHelper.loadAllItems` into a container of `size` slots, then
/// `saveAllItems`: stacks in slot order, the last one per slot winning.
fn items(list: Option<&Tag>, size: usize) -> Vec<Tag> {
    let mut slots: Vec<Option<Tag>> = vec![None; size];
    for entry in list.and_then(Tag::as_list).unwrap_or(&[]) {
        let Some(slot) = entry.get("Slot").and_then(Tag::as_i64).map(|s| (s as i8 as i32) & 255) else { continue };
        let Some(Tag::Compound(mut stack)) = item_stack(entry) else { continue };
        if (slot as usize) < size {
            stack.insert("Slot".into(), Tag::Byte(slot as i8));
            slots[slot as usize] = Some(Tag::Compound(stack));
        }
    }
    slots.into_iter().flatten().collect()
}

/// `ItemStack.MAP_CODEC`: `id`, `count` (default 1) and a non-empty
/// component patch. Air and empty stacks are dropped.
fn item_stack(tag: &Tag) -> Option<Tag> {
    let id = tag.get("id").and_then(Tag::as_str)?;
    if id == "minecraft:air" {
        return None;
    }
    let count = tag.get("count").and_then(Tag::as_i64).unwrap_or(1);
    if !(1..=99).contains(&count) {
        return None;
    }
    let mut out = BTreeMap::new();
    out.insert("id".to_owned(), Tag::String(id.to_owned()));
    out.insert("count".to_owned(), Tag::Int(count as i32));
    if let Some(components) = tag.get("components").filter(|c| c.as_compound().is_some_and(|m| !m.is_empty())) {
        out.insert("components".to_owned(), components.clone());
    }
    Some(Tag::Compound(out))
}

/// `AbstractFurnaceBlockEntity.RECIPES_USED_CODEC`: recipe ids to counts.
fn recipes_used(tag: Option<&Tag>) -> Tag {
    let mut out = BTreeMap::new();
    for (key, value) in tag.and_then(Tag::as_compound).into_iter().flatten() {
        if let Some(count) = value.as_i64() {
            out.insert(key.clone(), Tag::Int(count as i32));
        }
    }
    Tag::Compound(out)
}

/// `ComponentSerialization.CODEC` round trip for the forms templates use:
/// a plain text component saves as its string.
fn component(tag: &Tag) -> Option<Tag> {
    match tag {
        Tag::String(_) => Some(tag.clone()),
        Tag::Compound(map) => match (map.len(), map.get("text")) {
            (1, Some(Tag::String(text))) => Some(Tag::String(text.clone())),
            _ => Some(tag.clone()),
        },
        Tag::List(list) if !list.is_empty() => Some(tag.clone()),
        _ => None,
    }
}

/// `SignText.CODEC`: four messages, filtered messages only when they
/// differ, color (default black) and the glowing flag.
fn sign_text(tag: Option<&Tag>) -> Tag {
    let lines = |key: &str| -> Option<Vec<Tag>> {
        let list = tag?.get(key)?.as_list()?;
        (list.len() == 4).then(|| list.iter().map(|l| component(l).unwrap_or(Tag::String(String::new()))).collect())
    };
    let messages = lines("messages").unwrap_or_else(|| vec![Tag::String(String::new()); 4]);
    let mut out = BTreeMap::new();
    if let Some(filtered) = lines("filtered_messages").filter(|f| *f != messages) {
        out.insert("filtered_messages".to_owned(), Tag::List(filtered));
    }
    out.insert("messages".to_owned(), Tag::List(messages));
    let color = tag.and_then(|t| t.get("color")).and_then(Tag::as_str).unwrap_or("black");
    out.insert("color".to_owned(), Tag::String(color.to_owned()));
    let glowing = tag.and_then(|t| t.get("has_glowing_text")).and_then(Tag::as_i64).is_some_and(|v| v != 0);
    out.insert("has_glowing_text".to_owned(), Tag::Byte(i8::from(glowing)));
    Tag::Compound(out)
}

/// `VaultConfig.CODEC`: fields equal to `VaultConfig.DEFAULT` are left out,
/// except the key item.
fn vault_config(tag: &Tag) -> Tag {
    let mut out = BTreeMap::new();
    if let Some(table) = tag.get("loot_table").and_then(Tag::as_str).filter(|t| *t != "minecraft:chests/trial_chambers/reward") {
        out.insert("loot_table".to_owned(), Tag::String(table.to_owned()));
    }
    for (key, default) in [("activation_range", 4.0), ("deactivation_range", 4.5)] {
        if let Some(v) = tag.get(key).and_then(Tag::as_f64).filter(|v| *v != default) {
            out.insert(key.to_owned(), Tag::Double(v));
        }
    }
    if let Some(item) = tag.get("key_item").and_then(item_stack) {
        out.insert("key_item".to_owned(), item);
    }
    if let Some(table) = tag.get("override_loot_table_to_display").and_then(Tag::as_str) {
        out.insert("override_loot_table_to_display".to_owned(), Tag::String(table.to_owned()));
    }
    Tag::Compound(out)
}

/// `new SpawnData()`: an entity tag with no id.
fn empty_spawn_data() -> Tag {
    let mut data = BTreeMap::new();
    data.insert("entity".to_owned(), Tag::Compound(BTreeMap::new()));
    Tag::Compound(data)
}

/// Base64 (standard alphabet) to an unnamed-root NBT compound.
pub fn decode(text: &str) -> Result<Tag, String> {
    let mut bytes = Vec::with_capacity(text.len() * 3 / 4);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in text.bytes().filter(|&c| c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(format!("bad base64 byte {c}")),
        };
        buffer = (buffer << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
        }
    }
    crate::nbt::parse(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip_of_a_compound() {
        // {"a": Int(1)} as unnamed-root binary NBT.
        let tag = decode("CgAAAwABYQAAAAEA").unwrap();
        assert_eq!(tag.get("a"), Some(&Tag::Int(1)));
    }
}
