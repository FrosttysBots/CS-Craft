//! Installed item metadata plus the publicly released rare-item case families.
use crate::cosmetics::*;
use bevy::prelude::*;
use std::collections::BTreeMap;

pub fn table(root: &kv::Block, key: &str) -> kv::Block {
    kv::Block {
        entries: root
            .get_all(key)
            .filter_map(kv::Value::as_block)
            .flat_map(|b| b.entries.clone())
            .collect(),
    }
}
fn resolve(item: &kv::Block, prefabs: &kv::Block, depth: u8) -> kv::Block {
    let mut out = kv::Block::default();
    if depth < 12 {
        for name in item.str("prefab").unwrap_or("").split_whitespace() {
            if let Some(b) = prefabs.block(name) {
                for (k, v) in resolve(b, prefabs, depth + 1).entries {
                    out.set(&k, v);
                }
            }
        }
    }
    for (k, v) in &item.entries {
        out.set(k, v.clone());
    }
    out
}
pub fn model_items(fs: &vfs::Vfs) -> Vec<(String, String, String)> {
    let Some(text) = fs.read_string("scripts/items/items_game.txt") else {
        return vec![];
    };
    let Ok(doc) = kv::parse(&text) else {
        return vec![];
    };
    let Some(root) = doc.block("items_game") else {
        return vec![];
    };
    table(root, "items")
        .iter()
        .filter_map(|(_, v)| {
            let b = v.as_block()?;
            if b.str("prefab") != Some("hands_paintable") {
                return None;
            }
            Some((
                b.str("name")?.into(),
                b.str("model_player")?.into(),
                b.str("model_world")?.into(),
            ))
        })
        .collect()
}
fn rarity(s: &str) -> u8 {
    match s {
        "common" => 1,
        "uncommon" => 2,
        "rare" => 3,
        "mythical" => 4,
        "legendary" => 5,
        "ancient" => 6,
        _ => 1,
    }
}
fn leaves(loot: &kv::Block, key: &str, depth: u8, r: u8, out: &mut Vec<(String, u8)>) {
    if depth > 14 {
        return;
    }
    let r = key
        .rsplit('_')
        .next()
        .map(rarity)
        .filter(|x| *x > 1)
        .unwrap_or(r);
    if let Some(b) = loot.block(key) {
        for (k, _) in b.iter() {
            if k.starts_with('[') {
                out.push((k.into(), r));
            } else {
                leaves(loot, k, depth + 1, r, out);
            }
        }
    }
}
pub fn load(fs: &vfs::Vfs) -> Result<Catalog, String> {
    let doc = kv::parse(
        &fs.read_string("scripts/items/items_game.txt")
            .ok_or("items_game.txt missing")?,
    )
    .map_err(|e| e.to_string())?;
    let root = doc.block("items_game").ok_or("items_game root missing")?;
    let items = table(root, "items");
    let prefabs = table(root, "prefabs");
    let paints = table(root, "paint_kits");
    let paint_rarity = table(root, "paint_kits_rarity");
    let loot = table(root, "client_loot_lists");
    let revolving = table(root, "revolving_loot_lists");
    let loc = crate::localize::Localization::load(fs);
    let mut out = Catalog::default();
    let resolved: BTreeMap<u32, kv::Block> = items
        .iter()
        .filter_map(|(k, v)| Some((k.parse().ok()?, resolve(v.as_block()?, &prefabs, 0))))
        .collect();
    let mut paint_ids = BTreeMap::new();
    for (id, v) in paints.iter() {
        if let Some(b) = v.as_block() {
            if let (Ok(id), Some(name)) = (id.parse::<u32>(), b.str("name")) {
                paint_ids.insert(name.to_string(), id);
            }
        }
    }
    for item in resolved.values() {
        let name = item.str("name").unwrap_or("");
        if !(name.starts_with("weapon_") || is_glove(name)) {
            continue;
        }
        let Some(view) = item.str("model_player") else {
            continue;
        };
        let world = item.str("model_world").unwrap_or(view);
        let stickers = item
            .block("stickers")
            .map(|b| {
                b.iter()
                    .filter_map(|(_, v)| v.as_block()?.str("viewmodel_geometry").map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        out.models.insert(
            name.into(),
            ModelInfo {
                view: view.into(),
                world: world.into(),
                stickers,
            },
        );
        if let Some(p) = item.block("paint_data") {
            out.materials.insert(
                name.into(),
                p.iter()
                    .filter_map(|(_, v)| {
                        let b = v.as_block()?;
                        let name = b.str("Name")?;
                        Some((
                            b.str("OrigMat").unwrap_or(name).to_lowercase(),
                            name.to_lowercase(),
                            b.str("FolderName").unwrap_or(name).to_lowercase(),
                        ))
                    })
                    .collect(),
            );
        }
    }
    // The icon index encodes item definition, paint kit and wear variant. It
    // provides valid weapon/paint pairs without inventing incompatible finishes.
    if let Some(icons) = root
        .path(&["alternate_icons2", "weapon_icons"])
        .and_then(kv::Value::as_block)
    {
        for (encoded, v) in icons.iter() {
            let Ok(encoded) = encoded.parse::<u64>() else {
                continue;
            };
            if encoded & 3 != 0 {
                continue;
            }
            let def = (encoded >> 16) as u32;
            let paint = ((encoded >> 2) & 0x3fff) as u32;
            let (Some(item), Some(p)) = (resolved.get(&def), paints.block(&paint.to_string()))
            else {
                continue;
            };
            let name = item.str("name").unwrap_or("");
            if !out.models.contains_key(name) {
                continue;
            }
            let Some(icon) = v.as_block().and_then(|v| v.str("icon_path")) else {
                continue;
            };
            let icon = format!("{icon}_large");
            if !fs.exists(&format!("resource/flash/{icon}.png")) {
                continue;
            }
            let special = is_glove(name) || is_rare_knife(name);
            let mut f = finish(fs, item, paint, p, &loc);
            f.icon = icon;
            f.rarity = if special {
                7
            } else {
                (rarity(item.str("item_rarity").unwrap_or("common"))
                    + rarity(paint_rarity.str(&f.paint).unwrap_or("common"))
                    - 1)
                .min(6)
            };
            out.finishes.push(f);
        }
    }
    for (&id, item) in &resolved {
        let name = item.str("name").unwrap_or("");
        if is_rare_knife(name) {
            let display = loc
                .get(item.str("item_name").unwrap_or(name))
                .unwrap_or(name);
            out.finishes.push(Finish {
                weapon: name.into(),
                name: format!("★ {display}"),
                icon: item.str("image_inventory").unwrap_or("").into(),
                rarity: 7,
                ..default()
            });
        }
        let prefab = item.str("prefab").unwrap_or("");
        if prefab != "weapon_case" && !prefab.contains("souvenirpkg") {
            continue;
        }
        let Some(series) = item
            .path(&["attributes", "set supply crate series", "value"])
            .and_then(kv::Value::as_str)
        else {
            continue;
        };
        let Some(list) = revolving.str(series) else {
            continue;
        };
        let souvenir = prefab.contains("souvenirpkg");
        let event = item
            .path(&["attributes", "tournament event id", "value"])
            .and_then(kv::Value::as_str)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut entries = vec![];
        leaves(&loot, list, 0, 0, &mut entries);
        let mut contents = vec![];
        for (entry, r) in entries {
            let Some((p, w)) = entry.trim_start_matches('[').split_once(']') else {
                continue;
            };
            if let Some(&paint) = paint_ids.get(p) {
                if let Some(i) = out
                    .finishes
                    .iter()
                    .position(|f| f.id == paint && f.weapon == w)
                {
                    if r > 0 {
                        out.finishes[i].rarity = r;
                    }
                    contents.push(i);
                }
            }
        }
        if contents.is_empty() {
            continue;
        }
        if !souvenir {
            contents.extend(
                out.finishes
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| rare_for_case(id, f))
                    .map(|(i, _)| i),
            );
        }
        contents.sort_by_key(|&i| (out.finishes[i].rarity, out.finishes[i].name.clone()));
        contents.dedup();
        out.cases.push(Case {
            id,
            key: name.into(),
            name: loc
                .get(item.str("item_name").unwrap_or(name))
                .unwrap_or(name)
                .into(),
            icon: item.str("image_inventory").unwrap_or("").into(),
            contents,
            souvenir,
            event,
        });
    }
    // Item order is not chronological. Add rare pools only after every
    // vanilla knife has been registered.
    let finishes = &out.finishes;
    for c in &mut out.cases {
        if !c.souvenir {
            c.contents.extend(
                finishes
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| rare_for_case(c.id, f))
                    .map(|(i, _)| i),
            );
            c.contents.sort_unstable();
            c.contents.dedup();
            c.contents
                .sort_by_key(|&i| (finishes[i].rarity, finishes[i].name.clone()));
        }
    }
    for (id, v) in table(root, "sticker_kits").iter() {
        let Some(b) = v.as_block() else { continue };
        let Some(material) = b.str("sticker_material") else {
            continue;
        };
        if b.str("item_name").unwrap_or("").contains("Patch") {
            continue;
        }
        let path = format!("models/weapons/customization/stickers/{material}");
        let Some(mat) = vmt::Material::load(&path, &|p| fs.read_string(p)) else {
            continue;
        };
        if mat.base_texture().is_none() {
            continue;
        }
        let name = b.str("item_name").unwrap_or(material);
        out.stickers.insert(
            id.parse().unwrap_or(0),
            StickerDefinition {
                name: loc.get(name).unwrap_or(material).into(),
                material: path,
                event: b.i64("tournament_event_id").unwrap_or(0) as u32,
                team: b.i64("tournament_team_id").unwrap_or(0) as u32,
                player: b.i64("tournament_player_id").unwrap_or(0) as u32,
            },
        );
    }
    out.cases
        .sort_by_key(|c| (c.souvenir, std::cmp::Reverse(c.id)));
    Ok(out)
}
fn finish(
    fs: &vfs::Vfs,
    item: &kv::Block,
    id: u32,
    p: &kv::Block,
    loc: &crate::localize::Localization,
) -> Finish {
    let weapon = item.str("name").unwrap_or("");
    let paint = p.str("name").unwrap_or("");
    let style = p.i64("style").unwrap_or(0) as u32;
    let pattern = p.str("pattern").unwrap_or("");
    let folder = match style {
        2 => "hydrographic",
        3 => "spray",
        4 => "anodized",
        5 => "anodized_multi",
        6 => "anodized_air",
        7 => "custom",
        8 => "antiqued",
        9 => "gunsmith",
        _ => "solid",
    };
    let mut texture = format!("models/weapons/customization/paints/{folder}/{pattern}");
    if !pattern.is_empty() && !fs.exists(&format!("materials/{texture}.vtf")) {
        for folder in [
            "custom",
            "gunsmith",
            "antiqued",
            "hydrographic",
            "spray",
            "anodized_multi",
            "anodized_air",
        ] {
            let path = format!("models/weapons/customization/paints/{folder}/{pattern}");
            if fs.exists(&format!("materials/{path}.vtf")) {
                texture = path;
                break;
            }
        }
    }
    let name = loc
        .get(item.str("item_name").unwrap_or(weapon))
        .unwrap_or(weapon);
    let tag = loc
        .get(p.str("description_tag").unwrap_or(paint))
        .unwrap_or(paint);
    Finish {
        id,
        weapon: weapon.into(),
        paint: paint.into(),
        name: format!(
            "{}{name} | {tag}",
            if is_glove(weapon) || is_rare_knife(weapon) {
                "★ "
            } else {
                ""
            }
        ),
        style,
        pattern: if pattern.is_empty() {
            String::new()
        } else {
            texture
        },
        colors: std::array::from_fn(|i| {
            let c = kv::parse_vec(p.str(&format!("color{i}")).unwrap_or("128 128 128"));
            Vec3::new(
                *c.first().unwrap_or(&128.),
                *c.get(1).unwrap_or(&128.),
                *c.get(2).unwrap_or(&128.),
            ) / 255.
        }),
        wear: [
            p.f32("wear_remap_min")
                .unwrap_or(if is_glove(weapon) { 0.06 } else { 0. })
                .clamp(0., 1.),
            p.f32("wear_remap_max")
                .unwrap_or(if is_glove(weapon) { 0.8 } else { 1. })
                .clamp(0., 1.),
        ],
        scale: p.f32("pattern_scale").unwrap_or(1.),
        offset: [
            p.f32("pattern_offset_x_start").unwrap_or(0.),
            p.f32("pattern_offset_x_end").unwrap_or(0.),
            p.f32("pattern_offset_y_start").unwrap_or(0.),
            p.f32("pattern_offset_y_end").unwrap_or(0.),
        ],
        rotation: [
            p.f32("pattern_rotate_start").unwrap_or(0.),
            p.f32("pattern_rotate_end").unwrap_or(0.),
        ],
        only_first_material: p.bool("only_first_material").unwrap_or(false),
        glove_material: p.str("vmt_path").unwrap_or("").into(),
        albedo_boost: p.f32("phongalbedoboost").unwrap_or(10.) / 10.,
        phong: [
            p.f32("phongexponent").unwrap_or(32.),
            p.f32("phongintensity").unwrap_or(32.) / 32.,
        ],
        ..default()
    }
}
/// Public case releases define model families; the installation supplies each
/// family's compatible finishes (including Doppler phases).
pub fn rare_for_case(case: u32, f: &Finish) -> bool {
    if f.rarity != 7 {
        return false;
    }
    if matches!(case, 4288 | 4352 | 4471 | 4717 | 4747 | 4846 | 4880) {
        return is_glove(&f.weapon)
            && match case {
                4288 | 4352 => (10006..=10040).contains(&f.id),
                4471 | 4880 => (10041..=10064).contains(&f.id),
                _ => (10065..=10088).contains(&f.id),
            };
    }
    if !is_rare_knife(&f.weapon) {
        return false;
    }
    let original = [
        "weapon_bayonet",
        "weapon_knife_flip",
        "weapon_knife_gut",
        "weapon_knife_karambit",
        "weapon_knife_m9_bayonet",
    ];
    let second = [
        "weapon_knife_tactical",
        "weapon_knife_butterfly",
        "weapon_knife_falchion",
        "weapon_knife_survival_bowie",
        "weapon_knife_push",
    ];
    let horizon = [
        "weapon_knife_ursus",
        "weapon_knife_gypsy_jackknife",
        "weapon_knife_stiletto",
        "weapon_knife_widowmaker",
    ];
    let web = [
        "weapon_knife_cord",
        "weapon_knife_canis",
        "weapon_knife_outdoor",
        "weapon_knife_skeleton",
    ];
    let (models, family): (&[&str], u8) = match case {
        4017 => (&["weapon_knife_tactical"], 0),
        4018 => (&["weapon_knife_butterfly"], 0),
        4091 => (&["weapon_knife_falchion"], 0),
        4138 => (&["weapon_knife_push"], 0),
        4187 => (&["weapon_knife_survival_bowie"], 0),
        4669 => (&["weapon_knife_css"], 0),
        4482 | 4548 => (&horizon, 0),
        4620 | 4698 => (&web, 0),
        4061 | 4089 | 4233 => (&original, 1),
        4351 | 4403 => (&second, 1),
        4598 | 4695 => (&horizon, 1),
        4236 | 4281 => (&original, 2),
        4790 | 4818 => (&second, 2),
        4001 | 4002 | 4003 | 4004 | 4005 | 4009 | 4010 | 4011 | 4019 | 4029 | 4186 => {
            (&original, 0)
        }
        _ => return false,
    };
    if !models.contains(&f.weapon.as_str()) {
        return false;
    }
    let name = f.name.rsplit(" | ").next().unwrap_or("");
    match family {
        1 => [
            "Doppler",
            "Marble Fade",
            "Tiger Tooth",
            "Damascus Steel",
            "Rust Coat",
            "Ultraviolet",
        ]
        .contains(&name),
        2 => [
            "Gamma Doppler",
            "Lore",
            "Autotronic",
            "Black Laminate",
            "Bright Water",
            "Freehand",
        ]
        .contains(&name),
        _ => {
            f.id == 0
                || [
                    "Blue Steel",
                    "Boreal Forest",
                    "Case Hardened",
                    "Crimson Web",
                    "Fade",
                    "Forest DDPAT",
                    "Night",
                    "Night Stripe",
                    "Safari Mesh",
                    "Scorched",
                    "Slaughter",
                    "Stained",
                    "Urban Masked",
                ]
                .contains(&name)
        }
    }
}
