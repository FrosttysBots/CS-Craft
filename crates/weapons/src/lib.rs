//! Weapon catalog.
//!
//! CS:GO keeps weapon stats in `scripts/items/items_game.txt`: an `items`
//! table whose entries name a `prefab`, and a `prefabs` table where each
//! prefab may inherit from others (`weapon_ak47_prefab` → `rifle` → ...).
//! Stats live in each prefab's `attributes` block with readable names
//! (`"damage"`, `"armor ratio"`, `"cycletime"`...). The file repeats the
//! top-level `items` / `prefabs` blocks, so all of them are merged.
//!
//! [`WeaponDefinition`] is the normalised, game-agnostic form other games'
//! importers can produce too (see the `content` crate's id scheme).

use content::{ContentId, ContentKind};
use kv::{Block, Value};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WeaponClass {
    Knife,
    Pistol,
    Smg,
    Rifle,
    Sniper,
    Shotgun,
    MachineGun,
    Grenade,
    Equipment,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Terrorist,
    CounterTerrorist,
    Both,
}

/// Recoil parameters (CS:GO's seeded recoil pattern inputs).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Recoil {
    pub angle: f32,
    pub angle_variance: f32,
    pub magnitude: f32,
    pub magnitude_variance: f32,
    pub seed: i64,
}

/// Accuracy model inputs, in CS:GO's units (1/1000 radians-ish "inaccuracy").
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Accuracy {
    pub spread: f32,
    pub inaccuracy_stand: f32,
    pub inaccuracy_crouch: f32,
    pub inaccuracy_move: f32,
    pub inaccuracy_jump: f32,
    pub inaccuracy_land: f32,
    pub inaccuracy_fire: f32,
    pub recovery_time_stand: f32,
    pub recovery_time_crouch: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeaponDefinition {
    pub id: ContentId,
    /// Engine class name, e.g. `weapon_ak47`.
    pub class_name: String,
    /// Item definition index from `items_game.txt`.
    pub def_index: u32,
    /// Localisation token, e.g. `#SFUI_WPNHUD_AK47`.
    pub name_token: String,
    pub class: WeaponClass,
    pub side: Side,
    pub price: i32,
    pub damage: f32,
    pub armor_ratio: f32,
    pub penetration: f32,
    pub range: f32,
    pub range_modifier: f32,
    /// Seconds between shots.
    pub cycle_time: f32,
    pub full_auto: bool,
    pub bullets: u32,
    pub clip_size: i32,
    pub reserve_ammo: i32,
    pub max_player_speed: f32,
    pub accuracy: Accuracy,
    pub recoil: Recoil,
    pub view_model: String,
    pub world_model: String,
    /// Soundscript entry for a shot, e.g. `Weapon_AK47.Single`.
    pub fire_sound: String,
    pub kill_award: i32,
    /// Every resolved attribute, for anything not lifted into a field.
    pub attributes: HashMap<String, String>,
}

impl WeaponDefinition {
    pub fn attr_f32(&self, key: &str) -> Option<f32> {
        self.attributes.get(key).and_then(|v| kv::parse_f32(v))
    }

    /// Rounds per minute.
    pub fn rpm(&self) -> f32 {
        if self.cycle_time > 0.0 { 60.0 / self.cycle_time } else { 0.0 }
    }
}

#[derive(Default, Debug)]
pub struct Catalog {
    pub weapons: Vec<WeaponDefinition>,
}

impl Catalog {
    pub fn get(&self, class_name: &str) -> Option<&WeaponDefinition> {
        self.weapons.iter().find(|w| w.class_name == class_name)
    }

    pub fn by_id(&self, id: &ContentId) -> Option<&WeaponDefinition> {
        self.weapons.iter().find(|w| &w.id == id)
    }

    /// Parses `items_game.txt` text.
    pub fn from_items_game(text: &str) -> Result<Catalog, kv::ParseError> {
        let doc = kv::parse(text)?;
        let Some(root) = doc.block("items_game") else {
            return Ok(Catalog::default());
        };
        // Merge repeated top-level tables.
        let mut prefabs: HashMap<String, &Block> = HashMap::new();
        let mut items: Vec<(&str, &Block)> = Vec::new();
        for (key, value) in root.iter() {
            let Some(block) = value.as_block() else { continue };
            if key.eq_ignore_ascii_case("prefabs") {
                for (name, v) in block.iter() {
                    if let Some(b) = v.as_block() {
                        prefabs.insert(name.to_ascii_lowercase(), b);
                    }
                }
            } else if key.eq_ignore_ascii_case("items") {
                for (index, v) in block.iter() {
                    if let Some(b) = v.as_block() {
                        items.push((index, b));
                    }
                }
            }
        }

        let mut weapons = Vec::new();
        for (index, item) in items {
            let Some(class_name) = item.str("name") else { continue };
            if !class_name.starts_with("weapon_") {
                continue;
            }
            // Only base items carry the canonical stats.
            if item.str("baseitem") != Some("1") {
                continue;
            }
            let resolved = resolve(item, &prefabs);
            weapons.push(build(index, class_name, &resolved));
        }
        weapons.sort_by_key(|w| w.def_index);
        Ok(Catalog { weapons })
    }
}

/// Flattened view of an item after prefab inheritance.
#[derive(Default)]
struct Resolved {
    fields: HashMap<String, String>,
    attributes: HashMap<String, String>,
    used_by: Vec<String>,
}

fn resolve(item: &Block, prefabs: &HashMap<String, &Block>) -> Resolved {
    let mut out = Resolved::default();
    // Apply ancestors first (deepest first), then the item itself.
    let mut chain: Vec<&Block> = Vec::new();
    collect_chain(item, prefabs, &mut chain, 0);
    chain.reverse();
    for b in chain {
        for (k, v) in b.iter() {
            let key = k.to_ascii_lowercase();
            match v {
                Value::Str(s) => {
                    if key != "prefab" {
                        out.fields.insert(key, s.clone());
                    }
                }
                Value::Block(child) => {
                    if key == "attributes" {
                        for (ak, av) in child.iter() {
                            let val = match av {
                                Value::Str(s) => Some(s.clone()),
                                // Newer format: { "attribute_class" ... "value" "x" }
                                Value::Block(ab) => ab.str("value").map(str::to_string),
                            };
                            if let Some(val) = val {
                                out.attributes.insert(ak.to_ascii_lowercase(), val);
                            }
                        }
                    } else if key == "visuals" {
                        for (vk, vv) in child.iter() {
                            if let Some(v) = vv.as_str() {
                                out.fields.insert(format!("visuals/{}", vk.to_ascii_lowercase()), v.to_string());
                            }
                        }
                    } else if key == "used_by_classes" {
                        out.used_by = child.iter().map(|(k, _)| k.to_ascii_lowercase()).collect();
                    }
                }
            }
        }
    }
    out
}

fn collect_chain<'a>(b: &'a Block, prefabs: &HashMap<String, &'a Block>, out: &mut Vec<&'a Block>, depth: usize) {
    if depth > 16 {
        return;
    }
    out.push(b);
    if let Some(list) = b.str("prefab") {
        // Several space-separated prefabs; later ones take priority, so
        // walk them right-to-left to keep "item wins, then left-most".
        for name in list.split_whitespace().rev() {
            if let Some(p) = prefabs.get(&name.to_ascii_lowercase()) {
                collect_chain(p, prefabs, out, depth + 1);
            }
        }
    }
}

fn build(index: &str, class_name: &str, r: &Resolved) -> WeaponDefinition {
    let a = |k: &str| r.attributes.get(k).and_then(|v| kv::parse_f32(v));
    let ai = |k: &str| a(k).map(|v| v as i32);
    let f = |k: &str| r.fields.get(k).cloned().unwrap_or_default();
    let short = class_name.trim_start_matches("weapon_");
    let class = classify(class_name, &r.fields);
    let side = match (
        r.used_by.iter().any(|c| c == "terrorists"),
        r.used_by.iter().any(|c| c == "counter-terrorists"),
    ) {
        (true, false) => Side::Terrorist,
        (false, true) => Side::CounterTerrorist,
        _ => Side::Both,
    };
    WeaponDefinition {
        id: ContentId::csgo(ContentKind::Weapon, short),
        class_name: class_name.to_string(),
        def_index: index.parse().unwrap_or(0),
        name_token: f("item_name"),
        class,
        side,
        price: ai("in game price").unwrap_or(0),
        damage: a("damage").unwrap_or(0.0),
        armor_ratio: a("armor ratio").unwrap_or(1.0),
        penetration: a("penetration").unwrap_or(1.0),
        range: a("range").unwrap_or(8192.0),
        range_modifier: a("range modifier").unwrap_or(0.98),
        cycle_time: a("cycletime").unwrap_or(0.15),
        full_auto: a("is full auto").is_some_and(|v| v != 0.0),
        bullets: a("bullets").map(|v| v as u32).unwrap_or(1).max(1),
        clip_size: ai("primary clip size").unwrap_or(-1),
        reserve_ammo: ai("primary reserve ammo max").unwrap_or(0),
        max_player_speed: a("max player speed").unwrap_or(250.0),
        accuracy: Accuracy {
            spread: a("spread").unwrap_or(0.0),
            inaccuracy_stand: a("inaccuracy stand").unwrap_or(0.0),
            inaccuracy_crouch: a("inaccuracy crouch").unwrap_or(0.0),
            inaccuracy_move: a("inaccuracy move").unwrap_or(0.0),
            inaccuracy_jump: a("inaccuracy jump").unwrap_or(0.0),
            inaccuracy_land: a("inaccuracy land").unwrap_or(0.0),
            inaccuracy_fire: a("inaccuracy fire").unwrap_or(0.0),
            recovery_time_stand: a("recovery time stand").unwrap_or(0.0),
            recovery_time_crouch: a("recovery time crouch").unwrap_or(0.0),
        },
        recoil: Recoil {
            angle: a("recoil angle").unwrap_or(0.0),
            angle_variance: a("recoil angle variance").unwrap_or(0.0),
            magnitude: a("recoil magnitude").unwrap_or(0.0),
            magnitude_variance: a("recoil magnitude variance").unwrap_or(0.0),
            seed: a("recoil seed").map(|v| v as i64).unwrap_or(0),
        },
        view_model: f("model_player"),
        world_model: f("model_world"),
        fire_sound: f("visuals/sound_single_shot"),
        kill_award: ai("kill award").unwrap_or(300),
        attributes: r.attributes.clone(),
    }
}

fn classify(class_name: &str, fields: &HashMap<String, String>) -> WeaponClass {
    let slot = fields.get("item_type_name").map(String::as_str).unwrap_or("");
    let n = class_name;
    if n.contains("knife") || n == "weapon_bayonet" || n.starts_with("weapon_knife") {
        return WeaponClass::Knife;
    }
    if matches!(
        n,
        "weapon_hegrenade" | "weapon_flashbang" | "weapon_smokegrenade" | "weapon_molotov" | "weapon_incgrenade" | "weapon_decoy" | "weapon_tagrenade"
    ) || slot.contains("Grenade")
    {
        return WeaponClass::Grenade;
    }
    if matches!(n, "weapon_c4" | "weapon_taser" | "weapon_healthshot" | "weapon_shield" | "weapon_breachcharge") {
        return WeaponClass::Equipment;
    }
    match slot {
        s if s.contains("Pistol") => WeaponClass::Pistol,
        s if s.contains("SMG") => WeaponClass::Smg,
        s if s.contains("Sniper") => WeaponClass::Sniper,
        s if s.contains("Rifle") => WeaponClass::Rifle,
        s if s.contains("Shotgun") => WeaponClass::Shotgun,
        s if s.contains("Machinegun") => WeaponClass::MachineGun,
        _ => match n {
            "weapon_glock" | "weapon_hkp2000" | "weapon_usp_silencer" | "weapon_p250" | "weapon_deagle" | "weapon_elite"
            | "weapon_fiveseven" | "weapon_tec9" | "weapon_cz75a" | "weapon_revolver" => WeaponClass::Pistol,
            "weapon_mac10" | "weapon_mp9" | "weapon_mp7" | "weapon_mp5sd" | "weapon_ump45" | "weapon_p90" | "weapon_bizon" => {
                WeaponClass::Smg
            }
            "weapon_awp" | "weapon_ssg08" | "weapon_g3sg1" | "weapon_scar20" => WeaponClass::Sniper,
            "weapon_nova" | "weapon_xm1014" | "weapon_sawedoff" | "weapon_mag7" => WeaponClass::Shotgun,
            "weapon_m249" | "weapon_negev" => WeaponClass::MachineGun,
            "weapon_ak47" | "weapon_m4a1" | "weapon_m4a1_silencer" | "weapon_famas" | "weapon_galilar" | "weapon_aug"
            | "weapon_sg556" => WeaponClass::Rifle,
            _ => WeaponClass::Other,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"
"items_game"
{
    "prefabs"
    {
        "weapon_base" { "attributes" { "range modifier" "0.98" "kill award" "300" } }
        "rifle" { "prefab" "weapon_base" "item_type_name" "#CSGO_Type_Rifle" "attributes" { "kill award" "300" } }
        "weapon_ak47_prefab"
        {
            "prefab" "rifle"
            "item_class" "weapon_ak47"
            "item_name" "#SFUI_WPNHUD_AK47"
            "model_player" "models/weapons/v_rif_ak47.mdl"
            "used_by_classes" { "terrorists" "1" }
            "attributes" { "damage" "36" "cycletime" "0.100000" "is full auto" "1" "max player speed" "215" "in game price" "2700" "armor ratio" "1.550000" "primary clip size" "30" "recoil seed" "223" }
        }
    }
    "items"
    {
        "7" { "name" "weapon_ak47" "prefab" "weapon_ak47_prefab" "baseitem" "1" }
        "500" { "name" "weapon_ak47_skin" "prefab" "weapon_ak47_prefab" }
    }
    "prefabs" { "unused" { } }
}
"##;

    #[test]
    fn resolves_prefab_chain() {
        let cat = Catalog::from_items_game(SAMPLE).unwrap();
        assert_eq!(cat.weapons.len(), 1);
        let ak = cat.get("weapon_ak47").unwrap();
        assert_eq!(ak.id.to_string(), "csgo_weapon_ak47");
        assert_eq!(ak.def_index, 7);
        assert_eq!(ak.damage, 36.0);
        assert_eq!(ak.price, 2700);
        assert!(ak.full_auto);
        assert_eq!(ak.class, WeaponClass::Rifle);
        assert_eq!(ak.side, Side::Terrorist);
        assert_eq!(ak.range_modifier, 0.98);
        assert_eq!(ak.recoil.seed, 223);
        assert!((ak.rpm() - 600.0).abs() < 0.01);
    }
}
