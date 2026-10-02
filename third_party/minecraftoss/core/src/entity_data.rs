//! Entities as a proto chunk saves them (`Entity.save`), for the entities
//! world generation creates: each type's default tag from the harness
//! entity catalog (`EntityType.create` at the origin), with what loading a
//! structure template tag (`Entity.load`) or the creating code sets on top.
//!
//! Source-informed from the pinned 26.3 `Entity`, `LivingEntity`, `Mob`
//! `load`/`save` methods and `StructureTemplate.placeEntities`. UUIDs are
//! random in vanilla; generated entities get one from the caller.

use crate::nbt::Tag;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Entity types' default saved tags.
pub struct EntityCatalog {
    defaults: HashMap<String, Tag>,
}

impl EntityCatalog {
    /// Loads `artifacts/entity-catalog/26.3.json`.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let json: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if json["schema_version"] != 1 || json["minecraft_version"] != "26.3" {
            return Err(format!("{}: unsupported entity catalog", path.display()));
        }
        let mut defaults = HashMap::new();
        for (kind, nbt) in json["defaults"].as_object().ok_or("catalog has no defaults")? {
            defaults.insert(kind.clone(), crate::block_entity::decode(nbt.as_str().ok_or("default is not a string")?)?);
        }
        Ok(Self { defaults })
    }

    /// `EntityType.create`'s saved tag, if the type can be created.
    pub fn default_tag(&self, kind: &str) -> Option<Tag> {
        self.defaults.get(kind).cloned()
    }

    /// `EntityType.create(tag)`: a new entity of the tag's type loads the
    /// fields its `load` reads, then saves. Fields of the default tag take
    /// the template's value (numbers converted to the saved type); fields
    /// vanilla does not read, and old field names, are dropped.
    pub fn load_tag(&self, tag: &Tag) -> Option<Tag> {
        let kind = tag.get("id")?.as_str()?;
        let mut out = self.default_tag(kind)?;
        let Tag::Compound(map) = &mut out else { return None };
        let input = tag.as_compound()?;
        let keys: Vec<String> = map.keys().cloned().collect();
        for key in keys {
            if matches!(key.as_str(), "id" | "UUID" | "Brain" | "attributes") {
                continue;
            }
            let Some(value) = input.get(&key) else { continue };
            if let Some(v) = coerce(value, &map[&key]) {
                map.insert(key, v);
            }
        }
        // `LivingEntity.readAdditionalSaveData`: attributes by id, base and modifiers.
        if map.contains_key("attributes") {
            if let Some(list) = input.get("attributes").and_then(Tag::as_list) {
                map.insert("attributes".to_owned(), Tag::List(list.iter().filter_map(attribute).collect()));
            }
        }
        // `Mob.readAdditionalSaveData`: an absent `CanPickUpLoot` is false.
        if map.contains_key("CanPickUpLoot") && !input.contains_key("CanPickUpLoot") {
            map.insert("CanPickUpLoot".to_owned(), Tag::Byte(0));
        }
        for key in ["CustomName", "Tags", "CustomNameVisible", "Silent", "NoGravity", "Glowing"] {
            if let Some(value) = input.get(key) {
                map.insert(key.to_owned(), value.clone());
            }
        }
        // `Mob.readAdditionalSaveData`: equipment by slot, empty stacks dropped.
        if let Some(Tag::Compound(slots)) = input.get("equipment") {
            let equipment: BTreeMap<String, Tag> = slots.iter().filter_map(|(slot, stack)| Some((slot.clone(), item_stack(stack)?))).collect();
            if !equipment.is_empty() {
                map.insert("equipment".to_owned(), Tag::Compound(equipment));
            }
        }
        // `NeutralMob.readPersistentAngerSaveData`: no stored anger is -1.
        if map.contains_key("anger_end_time") {
            let end = match (input.get("anger_end_time").and_then(Tag::as_i64), input.get("AngerTime").and_then(Tag::as_i64)) {
                (Some(end), _) => end,
                (None, Some(time)) => time,
                (None, None) => -1,
            };
            map.insert("anger_end_time".to_owned(), Tag::Long(end));
        }
        // `Piglin.setBaby` (run by its load) touches the movement speed, which
        // then saves with its base.
        if kind == "minecraft:piglin" {
            ensure_attribute(map, "minecraft:movement_speed", 0.349_999_994_039_535_5);
        }
        Some(out)
    }
}

/// A value converted to the type the entity saves it as.
fn coerce(value: &Tag, default: &Tag) -> Option<Tag> {
    Some(match default {
        Tag::Byte(_) => Tag::Byte(value.as_i64()? as i8),
        Tag::Short(_) => Tag::Short(value.as_i64()? as i16),
        Tag::Int(_) => Tag::Int(value.as_i64()? as i32),
        Tag::Long(_) => Tag::Long(value.as_i64()?),
        Tag::Float(_) => Tag::Float(value.as_f64()? as f32),
        Tag::Double(_) => Tag::Double(value.as_f64()?),
        Tag::String(_) => Tag::String(value.as_str()?.to_owned()),
        Tag::List(d) => {
            let list = value.as_list()?;
            match d.first() {
                Some(first) if !matches!(first, Tag::Compound(_)) => Tag::List(list.iter().map(|v| coerce(v, first)).collect::<Option<_>>()?),
                _ => Tag::List(list.to_vec()),
            }
        }
        Tag::Compound(_) => Tag::Compound(value.as_compound()?.clone()),
        _ if std::mem::discriminant(value) == std::mem::discriminant(default) => value.clone(),
        _ => return None,
    })
}

/// One saved attribute: `id`, `base` and non-empty `modifiers`.
fn attribute(tag: &Tag) -> Option<Tag> {
    let id = tag.get("id").and_then(Tag::as_str)?;
    let mut out = BTreeMap::new();
    out.insert("id".to_owned(), Tag::String(id.to_owned()));
    out.insert("base".to_owned(), Tag::Double(tag.get("base").and_then(Tag::as_f64)?));
    if let Some(modifiers) = tag.get("modifiers").and_then(Tag::as_list).filter(|m| !m.is_empty()) {
        out.insert("modifiers".to_owned(), Tag::List(modifiers.to_vec()));
    }
    Some(Tag::Compound(out))
}

/// `ItemStack.CODEC`: `id`, `count` (default 1) and non-empty components;
/// `None` for an empty stack.
pub fn item_stack(tag: &Tag) -> Option<Tag> {
    let id = tag.get("id").and_then(Tag::as_str)?;
    if id == "minecraft:air" {
        return None;
    }
    let mut out = BTreeMap::new();
    out.insert("id".to_owned(), Tag::String(id.to_owned()));
    out.insert("count".to_owned(), Tag::Int(tag.get("count").and_then(Tag::as_i64).unwrap_or(1) as i32));
    if let Some(components) = tag.get("components").filter(|c| c.as_compound().is_some_and(|m| !m.is_empty())) {
        out.insert("components".to_owned(), components.clone());
    }
    Some(Tag::Compound(out))
}

/// An attribute instance saved with its base, added if the entity has none.
fn ensure_attribute(map: &mut BTreeMap<String, Tag>, id: &str, base: f64) {
    if let Tag::List(list) = map.entry("attributes".to_owned()).or_insert_with(|| Tag::List(Vec::new())) {
        if !list.iter().any(|a| a.get("id").and_then(Tag::as_str) == Some(id)) {
            let mut attribute = BTreeMap::new();
            attribute.insert("id".to_owned(), Tag::String(id.to_owned()));
            attribute.insert("base".to_owned(), Tag::Double(base));
            list.push(Tag::Compound(attribute));
        }
    }
}

/// Sets `Pos`, `Rotation` and `UUID` on a saved entity.
pub fn place(tag: &mut Tag, pos: [f64; 3], y_rot: f32, x_rot: f32, uuid: [i32; 4]) {
    if let Tag::Compound(map) = tag {
        map.insert("Pos".to_owned(), Tag::List(pos.iter().map(|&v| Tag::Double(v)).collect()));
        map.insert("Rotation".to_owned(), Tag::List(vec![Tag::Float(y_rot), Tag::Float(x_rot)]));
        map.insert("UUID".to_owned(), Tag::IntArray(uuid.to_vec()));
    }
}

/// `Mth.wrapDegrees` for floats.
pub fn wrap_degrees(angle: f32) -> f32 {
    let mut a = angle % 360.0;
    if a >= 180.0 {
        a -= 360.0;
    }
    if a < -180.0 {
        a += 360.0;
    }
    a
}

/// `Entity.rotate(Rotation)` then `mirror(Mirror)` as
/// `StructureTemplate.placeEntities` combines them: `rotate + mirror - yRot`.
/// `rotation` counts clockwise quarter turns; `mirror` is 0 (none),
/// 1 (`LEFT_RIGHT`) or 2 (`FRONT_BACK`).
pub fn placed_y_rot(y_rot: f32, rotation: u8, mirror: u8) -> f32 {
    let angle = wrap_degrees(y_rot);
    let rotated = match rotation % 4 {
        1 => angle + 90.0,
        2 => angle + 180.0,
        3 => angle + 270.0,
        _ => angle,
    };
    let mirrored = match mirror {
        1 => 180.0 - angle,
        2 => -angle,
        _ => angle,
    };
    rotated + mirrored - y_rot
}

/// `Mob.finalizeSpawn`'s common part: the follow range's random spawn bonus
/// (unless present, `random.triangle(0, 0.11485)`) and left-handedness,
/// drawn from the level random.
pub fn finalize_mob(tag: &mut Tag, random: &mut impl crate::random::RandomSource) {
    let Tag::Compound(map) = tag else { return };
    let attributes = map.entry("attributes".to_owned()).or_insert_with(|| Tag::List(Vec::new()));
    if let Tag::List(list) = attributes {
        let position = list.iter().position(|a| a.get("id").and_then(Tag::as_str) == Some("minecraft:follow_range"));
        let has_bonus = position.and_then(|i| list[i].get("modifiers")).and_then(Tag::as_list).is_some_and(|m| {
            m.iter().any(|m| m.get("id").and_then(Tag::as_str) == Some("minecraft:random_spawn_bonus"))
        });
        if !has_bonus {
            let amount = 0.0 + 0.114_850_000_000_000_01 * (random.next_f64() - random.next_f64());
            let mut modifier = BTreeMap::new();
            modifier.insert("id".to_owned(), Tag::String("minecraft:random_spawn_bonus".to_owned()));
            modifier.insert("amount".to_owned(), Tag::Double(amount));
            modifier.insert("operation".to_owned(), Tag::String("add_multiplied_base".to_owned()));
            match position {
                Some(i) => {
                    if let Tag::Compound(attribute) = &mut list[i] {
                        match attribute.entry("modifiers".to_owned()).or_insert_with(|| Tag::List(Vec::new())) {
                            Tag::List(m) => m.push(Tag::Compound(modifier)),
                            other => *other = Tag::List(vec![Tag::Compound(modifier)]),
                        }
                    }
                }
                None => {
                    let mut attribute = BTreeMap::new();
                    attribute.insert("id".to_owned(), Tag::String("minecraft:follow_range".to_owned()));
                    attribute.insert("base".to_owned(), Tag::Double(16.0));
                    attribute.insert("modifiers".to_owned(), Tag::List(vec![Tag::Compound(modifier)]));
                    list.push(Tag::Compound(attribute));
                }
            }
        }
    }
    map.insert("LeftHanded".to_owned(), Tag::Byte(i8::from(random.next_f32() < 0.05)));
}
