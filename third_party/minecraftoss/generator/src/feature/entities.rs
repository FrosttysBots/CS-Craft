//! Entities world generation creates, as vanilla saves them in the proto
//! chunk (`ProtoChunk.addEntity`): structure template entities
//! (`StructureTemplate.placeEntities`) and the ones structures and features
//! create in code, with `Mob.finalizeSpawn` drawing from the level random
//! (`WorldGenRegion.getRandom`) in vanilla's order.

use super::template::transform::{Mirror, Rotation};
use super::Ctx;
use minecraftoss_core::entity_data::{finalize_mob, place, placed_y_rot};
use minecraftoss_core::nbt::Tag;
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::BlockPos;
use serde_json::Value;
use std::collections::BTreeMap;

/// Cat variants and sound variants in registry order (identifier order).
#[derive(Debug, Default)]
pub struct CatVariants {
    /// Per variant: its spawn selectors (priority, condition).
    variants: Vec<(String, Vec<(i32, CatCondition)>)>,
    sounds: Vec<String>,
}

#[derive(Debug)]
enum CatCondition {
    Always,
    /// In a structure of the `#minecraft:cats_spawn_as_black` tag.
    BlackCatStructure,
    MoonBrightness(Option<f64>, Option<f64>),
}

impl CatVariants {
    pub fn load(pack: &minecraftoss_core::datapack::DataPack) -> Result<Self, String> {
        let mut out = Self::default();
        for id in pack.list("cat_variant")? {
            let json = pack.read_json("cat_variant", &id)?;
            let mut selectors = Vec::new();
            for s in json["spawn_conditions"].as_array().into_iter().flatten() {
                let priority = s["priority"].as_i64().unwrap_or(0) as i32;
                let condition = match s["condition"]["type"].as_str() {
                    None => CatCondition::Always,
                    Some("minecraft:structure") => CatCondition::BlackCatStructure,
                    Some("minecraft:moon_brightness") => {
                        let range = &s["condition"]["range"];
                        let bound = |v: &Value| v.as_f64();
                        match range {
                            Value::Number(n) => CatCondition::MoonBrightness(n.as_f64(), n.as_f64()),
                            _ => CatCondition::MoonBrightness(bound(&range["min"]), bound(&range["max"])),
                        }
                    }
                    Some(other) => return Err(format!("cat variant condition {other}")),
                };
                selectors.push((priority, condition));
            }
            out.variants.push((id.as_str().to_owned(), selectors));
        }
        out.sounds = pack.list("cat_sound_variant")?.into_iter().map(|id| id.as_str().to_owned()).collect();
        Ok(out)
    }

    /// `PriorityProvider.select`: the variants of the highest priority whose
    /// condition holds, in registry order.
    fn candidates(&self, black_cat_structure: bool, moon_brightness: f64) -> Vec<&str> {
        let mut unpacked: Vec<(&str, i32, &CatCondition)> =
            self.variants.iter().flat_map(|(id, selectors)| selectors.iter().map(move |(p, c)| (id.as_str(), *p, c))).collect();
        unpacked.sort_by(|a, b| b.1.cmp(&a.1));
        let mut highest = i32::MIN;
        let mut out = Vec::new();
        for (id, priority, condition) in unpacked {
            if priority < highest {
                continue;
            }
            let holds = match condition {
                CatCondition::Always => true,
                CatCondition::BlackCatStructure => black_cat_structure,
                CatCondition::MoonBrightness(min, max) => min.is_none_or(|m| moon_brightness >= m) && max.is_none_or(|m| moon_brightness <= m),
            };
            if holds {
                highest = priority;
                out.push(id);
            }
        }
        out
    }
}

/// `VillagerType.byBiome`.
fn villager_type(biome: &str) -> &'static str {
    match biome.trim_start_matches("minecraft:") {
        "badlands" | "desert" | "eroded_badlands" | "wooded_badlands" => "minecraft:desert",
        "bamboo_jungle" | "jungle" | "sparse_jungle" => "minecraft:jungle",
        "savanna_plateau" | "savanna" | "windswept_savanna" => "minecraft:savanna",
        "deep_frozen_ocean" | "frozen_ocean" | "frozen_river" | "ice_spikes" | "snowy_beach" | "snowy_taiga" | "snowy_plains" | "grove" | "snowy_slopes"
        | "frozen_peaks" | "jagged_peaks" => "minecraft:snow",
        "swamp" | "mangrove_swamp" => "minecraft:swamp",
        "old_growth_spruce_taiga" | "old_growth_pine_taiga" | "windswept_gravelly_hills" | "windswept_hills" | "taiga" | "windswept_forest" => "minecraft:taiga",
        _ => "minecraft:plains",
    }
}

fn is_mob(tag: &Tag) -> bool {
    tag.get("LeftHanded").is_some()
}

fn block_position(tag: &Tag) -> Option<BlockPos> {
    let pos = tag.get("Pos")?.as_list()?;
    let c = |i: usize| pos.get(i).and_then(Tag::as_f64).map(|v| v.floor() as i32);
    Some(BlockPos::new(c(0)?, c(1)?, c(2)?))
}

/// `Mob.finalizeSpawn` with the overrides of the types generation creates.
/// `black_cat_structure`: the position is in a `#cats_spawn_as_black` structure.
pub fn finalize_spawn(ctx: &mut Ctx, tag: &mut Tag, black_cat_structure: bool) {
    if !is_mob(tag) {
        return;
    }
    let kind = tag.get("id").and_then(Tag::as_str).unwrap_or_default().to_owned();
    let pos = block_position(tag);
    if matches!(kind.as_str(), "minecraft:villager" | "minecraft:zombie_villager") {
        // `VillagerDataHolder.finalizeVillagerType`.
        let finalized = tag.get("VillagerDataFinalized").and_then(Tag::as_i64).is_some_and(|v| v != 0);
        if !finalized {
            if let Some(biome) = pos.and_then(|p| ctx.region.biome(p.x, p.y, p.z)) {
                let name = ctx.registries().biomes.get(biome).name.as_str().to_owned();
                if let Tag::Compound(map) = tag {
                    if let Some(Tag::Compound(data)) = map.get_mut("VillagerData") {
                        data.insert("type".to_owned(), Tag::String(villager_type(&name).to_owned()));
                    }
                    map.insert("VillagerDataFinalized".to_owned(), Tag::Byte(1));
                }
            }
        }
    }
    if kind == "minecraft:piglin" {
        finalize_piglin(ctx, tag);
    }
    // `AgeableMob.finalizeSpawn` draws nothing for the first of a group.
    finalize_mob(tag, ctx.region.level_random());
    if kind == "minecraft:cat" {
        let lib = ctx.lib;
        // Generation runs on day 0: a full moon (`MOON_BRIGHTNESS_PER_PHASE[0]`).
        let candidates = lib.cats.candidates(black_cat_structure, 1.0);
        let random = ctx.region.level_random();
        if !candidates.is_empty() {
            let variant = candidates[random.next_i32_bound(candidates.len() as i32) as usize].to_owned();
            if let Tag::Compound(map) = tag {
                map.insert("variant".to_owned(), Tag::String(variant));
            }
        }
        if !lib.cats.sounds.is_empty() {
            let sound = lib.cats.sounds[random.next_i32_bound(lib.cats.sounds.len() as i32) as usize].clone();
            if let Tag::Compound(map) = tag {
                map.insert("sound_variant".to_owned(), Tag::String(sound));
            }
        }
    }
}

/// `StructureTemplate.placeEntities` for one template entity.
#[allow(clippy::too_many_arguments)]
pub fn place_template_entity(
    ctx: &mut Ctx,
    template_tag: &Tag,
    pos: [f64; 3],
    block_pos: BlockPos,
    rotation: Rotation,
    mirror: Mirror,
    finalize: bool,
) {
    let Some(catalog) = ctx.registries().entities.as_ref() else { return };
    let mut input = template_tag.clone();
    if let Tag::Compound(map) = &mut input {
        map.remove("UUID");
        if map.contains_key("block_pos") {
            map.insert("block_pos".to_owned(), Tag::IntArray(vec![block_pos.x, block_pos.y, block_pos.z]));
        }
    }
    let Some(mut tag) = catalog.load_tag(&input) else { return };
    let rot = template_tag.get("Rotation").and_then(Tag::as_list);
    let angle = |i: usize| rot.and_then(|r| r.get(i)).and_then(Tag::as_f64).unwrap_or(0.0) as f32;
    let quarter_turns = match rotation {
        Rotation::None => 0,
        Rotation::Clockwise90 => 1,
        Rotation::Clockwise180 => 2,
        Rotation::Counterclockwise90 => 3,
    };
    let mirror = match mirror {
        Mirror::None => 0,
        Mirror::LeftRight => 1,
        Mirror::FrontBack => 2,
    };
    let y_rot = placed_y_rot(angle(0), quarter_turns, mirror);
    let uuid = ctx.region.next_uuid();
    place(&mut tag, pos, y_rot, angle(1), uuid);
    if finalize {
        finalize_spawn(ctx, &mut tag, false);
    }
    ctx.region.add_entity(tag);
}

/// A new entity of a type (`EntityType.create`) at a position.
pub fn create(ctx: &mut Ctx, kind: &str, pos: [f64; 3], y_rot: f32, x_rot: f32) -> Option<Tag> {
    let mut tag = ctx.registries().entities.as_ref()?.default_tag(kind)?;
    let uuid = ctx.region.next_uuid();
    place(&mut tag, pos, y_rot, x_rot, uuid);
    Some(tag)
}

/// Sets fields on a saved entity.
pub fn set(tag: &mut Tag, fields: impl IntoIterator<Item = (&'static str, Tag)>) {
    if let Tag::Compound(map) = tag {
        for (key, value) in fields {
            map.insert(key.to_owned(), value);
        }
    }
}

/// An empty compound, for building nested tags.
pub fn compound(fields: impl IntoIterator<Item = (&'static str, Tag)>) -> Tag {
    Tag::Compound(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect::<BTreeMap<_, _>>())
}

/// `Piglin.finalizeSpawn` for a structure spawn, before `Mob.finalizeSpawn`:
/// the first hunt delay (`PiglinAi.initMemories`), golden armour at 10% a
/// piece, and the enchantment rolls of the equipped slots. Structure
/// piglins are adults, and generation runs on peaceful difficulty's special
/// multiplier of 0 in the harness, so no enchantment applies.
fn finalize_piglin(ctx: &mut Ctx, tag: &mut Tag) {
    let random = ctx.region.level_random();
    // `TIME_BETWEEN_HUNTS = rangeOfSeconds(30, 120)`: 600..=2400 ticks.
    let delay = random.next_i32_bound(2400 - 600 + 1) + 600;
    let Tag::Compound(map) = tag else { return };
    let memory = compound([("value", Tag::Byte(1)), ("ttl", Tag::Long(i64::from(delay)))]);
    map.insert("Brain".to_owned(), compound([("memories", compound([("minecraft:hunted_recently", memory)]))]));
    let adult = map.get("IsBaby").and_then(Tag::as_i64).is_none_or(|b| b == 0);
    let mut equipment = match map.remove("equipment") {
        Some(Tag::Compound(e)) => e,
        _ => BTreeMap::new(),
    };
    if adult {
        for (slot, item) in [("head", "minecraft:golden_helmet"), ("chest", "minecraft:golden_chestplate"), ("legs", "minecraft:golden_leggings"), ("feet", "minecraft:golden_boots")] {
            if random.next_f32() < 0.1 {
                equipment.insert(slot.to_owned(), compound([("id", Tag::String(item.to_owned())), ("count", Tag::Int(1))]));
            }
        }
    }
    // `populateDefaultEquipmentEnchantments`: the weapon, then the armour in
    // `EquipmentSlot.VALUES` order, one draw per filled slot.
    for slot in ["mainhand", "feet", "legs", "chest", "head"] {
        if equipment.contains_key(slot) {
            random.next_f32();
        }
    }
    if !equipment.is_empty() {
        map.insert("equipment".to_owned(), Tag::Compound(equipment));
    }
}
