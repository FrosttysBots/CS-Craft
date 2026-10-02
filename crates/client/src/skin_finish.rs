//! Per-instance cosmetic materials; equipped finishes travel with dropped weapons.
use crate::{
    cosmetics::{Catalog, Inventory, OwnedSkin},
    material::WorldMaterial,
    player::{LocalPlayer, SimWorld},
    skinning::SkinnedInstance,
    textures::TextureCache,
};
use bevy::prelude::*;
#[derive(Component, Clone)]
pub enum Target {
    View,
    Gloves,
    Held(sim::PlayerId),
    Ground(u32),
    Preview(String),
}
#[derive(Component)]
pub struct SkinPart {
    pub material: String,
    pub original: Handle<WorldMaterial>,
    pub applied: Option<(u32, u32, u32)>,
}
#[derive(Resource, Default)]
pub struct SkinTextures(pub TextureCache);
pub fn tag(commands: &mut Commands, instance: &SkinnedInstance, target: Target) {
    for &e in &instance.meshes {
        commands.entity(e).insert(target.clone());
    }
}
pub fn assign(mut world: ResMut<SimWorld>, local: Res<LocalPlayer>, inventory: Res<Inventory>) {
    let changed = inventory.is_changed();
    let definitions = world.0.catalog.clone();
    for p in &mut world.0.players {
        for w in &mut p.weapons {
            if w.cosmetic_owner.is_none()
                || (changed && w.cosmetic_owner == Some(local.id) && p.id == local.id)
            {
                if p.id == local.id {
                    if w.def.class == weapons::WeaponClass::Knife {
                        let name = inventory
                            .active("knife")
                            .map(|i| i.weapon.as_str())
                            .unwrap_or(if p.team == sim::Team::Terrorist {
                                "weapon_knife_t"
                            } else {
                                "weapon_knife"
                            });
                        if w.def.class_name != name {
                            if let Some(def) = definitions.get(name) {
                                w.def = std::sync::Arc::new(def.clone());
                            }
                        }
                    }
                    w.cosmetic =
                        inventory
                            .active(&w.def.class_name)
                            .map(|i| sim::weapon::Cosmetic {
                                serial: i.serial,
                                stattrak: i.stattrak,
                                souvenir: i.souvenir,
                                stickers: i.stickers.map(|s| s.map(|s| (s.kit, s.wear))),
                                paint: i.paint,
                                seed: i.seed,
                                wear: i.wear,
                            });
                }
                w.cosmetic_owner = Some(p.id);
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub fn update(
    world: Res<SimWorld>,
    local: Res<LocalPlayer>,
    inventory: Res<Inventory>,
    catalog: Res<Catalog>,
    level: Res<crate::LoadedLevel>,
    cases: Res<crate::case_ui::CaseUi>,
    menu: Res<crate::menu::Menu>,
    mut cache: ResMut<SkinTextures>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<WorldMaterial>>,
    mut parts: Query<(&Target, &mut SkinPart, &MeshMaterial3d<WorldMaterial>)>,
) {
    for (target, mut part, handle) in &mut parts {
        let selected: Option<OwnedSkin> =
            match target {
                Target::Gloves => inventory.active("gloves").cloned(),
                Target::Preview(weapon) => {
                    if menu.page == Some(crate::menu::Page::Cases) {
                        cases
                            .preview_item()
                            .filter(|i| i.weapon == *weapon)
                            .cloned()
                    } else {
                        inventory.active(weapon).cloned()
                    }
                }
                _ => {
                    let weapon =
                        match target {
                            Target::View => world
                                .0
                                .player(local.view_target())
                                .and_then(|p| p.active_weapon()),
                            Target::Held(id) => world.0.player(*id).and_then(|p| p.active_weapon()),
                            Target::Ground(id) => {
                                world.0.items.iter().find(|i| i.id == *id).and_then(|i| {
                                    match &i.kind {
                                        sim::GroundKind::Weapon(w) => Some(w),
                                        _ => None,
                                    }
                                })
                            }
                            _ => None,
                        };
                    weapon.and_then(|w| {
                        w.cosmetic.map(|s| OwnedSkin {
                            serial: s.serial,
                            stattrak: s.stattrak,
                            souvenir: s.souvenir,
                            stickers: s.stickers.map(|s| {
                                s.map(|(kit, wear)| crate::cosmetics::StickerSlot { kit, wear })
                            }),
                            weapon: w.def.class_name.clone(),
                            paint: s.paint,
                            seed: s.seed,
                            wear: s.wear,
                        })
                    })
                }
            };
        let key = selected
            .as_ref()
            .map(|s| (s.paint, s.seed, s.wear.to_bits()));
        if part.applied == key {
            continue;
        }
        let original = materials.get(&part.original).cloned();
        let Some(original) = original else { continue };
        let mut replacement = original.clone();
        if let Some(item) = &selected {
            if let (Some(f), Some(mapping)) =
                (catalog.finish(item), catalog.materials.get(&item.weapon))
            {
                let material = part.material.replace('\\', "/").to_lowercase();
                let basename = material
                    .rsplit('/')
                    .next()
                    .unwrap_or(&material)
                    .trim_end_matches(".vmt");
                if let Some((_, (_, custom, folder))) =
                    mapping.iter().enumerate().find(|(index, (orig, name, _))| {
                        (!f.only_first_material || *index == 0)
                            && (basename == orig.as_str() || basename == name.as_str())
                    })
                {
                    let fs = &level.0.vfs;
                    let pattern = if f.pattern.is_empty() {
                        Some(original.base.clone())
                    } else {
                        cache
                            .0
                            .load(fs, &mut images, &f.pattern, f.style > 6)
                            .map(|t| t.handle)
                    };
                    if let Some(pattern) = pattern {
                        replacement.base2 = pattern;
                        replacement.modulate = cache
                            .0
                            .load(
                                fs,
                                &mut images,
                                &format!("models/weapons/customization/{folder}/{custom}_masks"),
                                false,
                            )
                            .map_or(original.base2.clone(), |t| t.handle);
                        replacement.normal2 = cache
                            .0
                            .load(
                                fs,
                                &mut images,
                                &format!("models/weapons/customization/{folder}/{custom}_ao"),
                                false,
                            )
                            .map_or(original.base2.clone(), |t| t.handle);
                        let mut rng = crate::cosmetics::Random(item.seed as u64);
                        let rotation = (f.rotation[0]
                            + rng.unit() * (f.rotation[1] - f.rotation[0]))
                            .to_radians();
                        replacement.params.skin =
                            Vec4::new(f.style as f32, item.wear, rotation.cos(), rotation.sin());
                        replacement.params.skin_transform = Vec4::new(
                            f.scale,
                            f.scale,
                            f.offset[0] + rng.unit() * (f.offset[1] - f.offset[0]),
                            f.offset[2] + rng.unit() * (f.offset[3] - f.offset[2]),
                        );
                        replacement.params.skin_colors = std::array::from_fn(|i| {
                            let c = f.colors[i % 4];
                            Vec4::new(srgb(c.x), srgb(c.y), srgb(c.z), 1.)
                        });
                        if let Some(t) = cache.0.load(
                            fs,
                            &mut images,
                            &format!("models/weapons/customization/{folder}/{custom}_surface"),
                            false,
                        ) {
                            replacement.skin_surface = t.handle;
                            replacement.params.skin_details.y = 1.;
                        }
                        replacement.params.skin_properties.x = f.albedo_boost.clamp(1., 10.);
                        if matches!(f.style, 3 | 6) {
                            if let Some(t) = cache.0.load(
                                fs,
                                &mut images,
                                &format!("models/weapons/customization/{folder}/{custom}_pos"),
                                false,
                            ) {
                                replacement.skin_position = t.handle;
                                replacement.params.skin_properties.y = 1.;
                            }
                        }
                        replacement.params.phong.x = f.phong[0];
                        replacement.params.phong.y = f.phong[1];
                    } else {
                        warn!("Missing paint texture: {}", f.pattern);
                    }
                }
            }
        }
        if let Some(item) = &selected {
            if let Some(f) = catalog
                .finish(item)
                .filter(|f| !f.glove_material.is_empty())
            {
                if part.material.contains("glove") || part.material.contains("handwrap") {
                    let fs = &level.0.vfs;
                    if let Some(mat) =
                        vmt::Material::load(&f.glove_material, &|p| fs.read_string(p))
                    {
                        for (field, dest, srgb) in [
                            ("$materialmask", &mut replacement.modulate, false),
                            ("$ao", &mut replacement.normal2, false),
                            ("$pattern", &mut replacement.base2, false),
                            ("$grunge", &mut replacement.skin_surface, false),
                        ] {
                            if let Some(t) = mat
                                .str(field)
                                .and_then(|n| cache.0.load(fs, &mut images, n, srgb))
                            {
                                *dest = t.handle;
                            }
                        }
                        replacement.params.skin = Vec4::new(10., item.wear, 1., 0.);
                        replacement.params.skin_details =
                            Vec4::new(mat.f32("$patternreplaceindex").unwrap_or(0.), 1., 0., 0.);
                        let indices =
                            kv::parse_vec(mat.str("$patterncolorindices").unwrap_or("[1 2 3 4]"));
                        replacement.params.skin_pattern_palette =
                            Vec4::from_array(std::array::from_fn(|i| {
                                indices.get(i).copied().unwrap_or(1.)
                            }));
                        replacement.params.skin_colors = std::array::from_fn(|i| {
                            let c = mat
                                .color(&format!("$palettecolor{}", i + 1))
                                .unwrap_or([128.; 3]);
                            Vec4::new(srgb(c[0] / 255.), srgb(c[1] / 255.), srgb(c[2] / 255.), 1.)
                        });
                        let mut rng = crate::cosmetics::Random(item.seed as u64);
                        let scale = mat.f32("$patternscale").unwrap_or(1.);
                        let rotation = mat.f32("$pattern_rotate_start").unwrap_or(0.).to_radians();
                        replacement.params.skin.z = rotation.cos();
                        replacement.params.skin.w = rotation.sin();
                        replacement.params.skin_transform = Vec4::new(
                            if part.material.contains("right") {
                                -scale
                            } else {
                                scale
                            },
                            scale,
                            rng.unit(),
                            rng.unit(),
                        );
                    }
                }
            }
        }
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.base2 = replacement.base2;
            material.skin_surface = replacement.skin_surface;
            material.skin_position = replacement.skin_position;
            material.params.skin_properties = replacement.params.skin_properties;
            material.params.skin_pattern_palette = replacement.params.skin_pattern_palette;
            material.params.skin_details = replacement.params.skin_details;
            material.params.phong = replacement.params.phong;
            material.modulate = replacement.modulate;
            material.normal2 = replacement.normal2;
            material.params.skin = replacement.params.skin;
            material.params.skin_transform = replacement.params.skin_transform;
            material.params.skin_colors = replacement.params.skin_colors;
        }
        part.applied = key;
    }
}
fn srgb(x: f32) -> f32 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

/// Events are read before the effects system drains them. Failed disk writes
/// remain queued and are retried, so a temporary file lock cannot lose kills.
pub fn record(
    events: Res<crate::effects::PendingEvents>,
    local: Res<LocalPlayer>,
    cases: Res<crate::case_ui::CaseUi>,
    mut inventory: ResMut<Inventory>,
    mut pending: Local<std::collections::BTreeMap<u64, u32>>,
) {
    for event in &events.0 {
        if let sim::GameEvent::StatTrakKill { owner, serial } = event {
            if *owner == local.id {
                *pending.entry(*serial).or_default() += 1;
            }
        }
    }
    if pending.is_empty() {
        return;
    }
    let apply = |i: &mut Inventory| {
        for (id, n) in pending.iter() {
            if let Some(count) = i
                .skins
                .iter_mut()
                .find(|s| s.serial == *id)
                .and_then(|s| s.stattrak.as_mut())
            {
                *count = count.saturating_add(*n);
            }
        }
        Ok(())
    };
    let result = if cases.fixture {
        apply(&mut inventory)
    } else {
        inventory.transaction(&crate::cosmetics::profile_path(), apply)
    };
    if result.is_ok() {
        pending.clear();
    }
}
