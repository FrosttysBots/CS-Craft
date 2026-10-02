//! Original bone-merged sticker meshes and StatTrak modules.
use crate::{
    cosmetics::{Catalog, OwnedSkin},
    material::{WorldMaterial, flags},
    skinning::{self, SkinnedAsset, SkinnedInstance},
};
use bevy::{camera::visibility::RenderLayers, prelude::*};
use std::collections::HashMap;

#[derive(Component)]
pub struct Parent {
    pub target: crate::skin_finish::Target,
    pub bones: Vec<(String, Entity)>,
    pub layer: usize,
}
pub fn tag(
    commands: &mut Commands,
    asset: &SkinnedAsset,
    instance: &SkinnedInstance,
    target: crate::skin_finish::Target,
    layer: usize,
) {
    if let Some(e) = instance.meshes.first() {
        commands.entity(*e).insert(Parent {
            target,
            bones: asset
                .model
                .bones
                .iter()
                .zip(&instance.joints)
                .map(|(b, e)| (b.name.clone(), *e))
                .collect(),
            layer,
        });
    }
}
struct Part {
    asset: SkinnedAsset,
    instance: SkinnedInstance,
    materials: Vec<Handle<WorldMaterial>>,
}
#[derive(Resource, Default)]
pub struct Decorations {
    instances: HashMap<Entity, (OwnedSkin, Vec<Part>)>,
    textures: crate::textures::TextureCache,
    digits: HashMap<(String, usize), Handle<Image>>,
}
fn owned(w: &sim::weapon::WeaponState) -> Option<OwnedSkin> {
    w.cosmetic.map(|s| OwnedSkin {
        serial: s.serial,
        weapon: w.def.class_name.clone(),
        paint: s.paint,
        seed: s.seed,
        wear: s.wear,
        stattrak: s.stattrak,
        souvenir: s.souvenir,
        stickers: s
            .stickers
            .map(|s| s.map(|(kit, wear)| crate::cosmetics::StickerSlot { kit, wear })),
    })
}
fn despawn(c: &mut Commands, parts: Vec<Part>) {
    for p in parts {
        for e in p.instance.meshes.iter().chain(&p.instance.joints) {
            c.entity(*e).despawn();
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub fn update(
    mut commands: Commands,
    parents: Query<(Entity, &Parent, &Visibility, &MeshMaterial3d<WorldMaterial>)>,
    world: Res<crate::player::SimWorld>,
    local: Res<crate::player::LocalPlayer>,
    inventory: Res<crate::cosmetics::Inventory>,
    catalog: Res<Catalog>,
    cases: Res<crate::case_ui::CaseUi>,
    models: Res<crate::viewmodel::ViewModels>,
    level: Res<crate::LoadedLevel>,
    mut state: ResMut<Decorations>,
    mut transforms: Query<&mut Transform>,
    mut visibility: Query<&mut Visibility, Without<Parent>>,
    mut materials: ResMut<Assets<WorldMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let stale: Vec<_> = state
        .instances
        .keys()
        .filter(|e| parents.get(**e).is_err())
        .copied()
        .collect();
    for e in stale {
        if let Some((_, p)) = state.instances.remove(&e) {
            despawn(&mut commands, p);
        }
    }
    for (entity, parent, visible, base) in &parents {
        let selected = match &parent.target {
            crate::skin_finish::Target::View => world
                .0
                .player(local.view_target())
                .and_then(|p| p.active_weapon())
                .and_then(|w| {
                    w.cosmetic.map(|s| OwnedSkin {
                        serial: s.serial,
                        weapon: w.def.class_name.clone(),
                        paint: s.paint,
                        seed: s.seed,
                        wear: s.wear,
                        stattrak: s.stattrak,
                        souvenir: s.souvenir,
                        stickers: s.stickers.map(|s| {
                            s.map(|(kit, wear)| crate::cosmetics::StickerSlot { kit, wear })
                        }),
                    })
                }),
            crate::skin_finish::Target::Preview(w) => cases
                .preview_item()
                .filter(|i| i.weapon == *w)
                .cloned()
                .or_else(|| inventory.active(w).cloned()),
            crate::skin_finish::Target::Held(id) => world
                .0
                .player(*id)
                .and_then(|p| p.active_weapon())
                .and_then(owned),
            crate::skin_finish::Target::Ground(id) => world
                .0
                .items
                .iter()
                .find(|i| i.id == *id)
                .and_then(|i| match &i.kind {
                    sim::GroundKind::Weapon(w) => owned(w),
                    _ => None,
                }),
            crate::skin_finish::Target::Gloves => inventory.active("gloves").cloned(),
        };
        let Some(item) = selected else {
            if let Some((_, p)) = state.instances.remove(&entity) {
                despawn(&mut commands, p);
            }
            continue;
        };
        if state
            .instances
            .get(&entity)
            .is_none_or(|(old, _)| *old != item)
        {
            if let Some((_, parts)) = state.instances.remove(&entity) {
                despawn(&mut commands, parts);
            }
            let mut additions = vec![];
            let paths = catalog
                .models
                .get(&item.weapon)
                .map(|m| m.stickers.as_slice())
                .unwrap_or(&[]);
            let mut wanted: Vec<(String, Option<crate::cosmetics::StickerSlot>)> = item
                .stickers
                .iter()
                .enumerate()
                .filter_map(|(i, s)| Some((paths.get(i)?.clone(), Some((*s)?))))
                .collect();
            if crate::cosmetics::is_glove(&item.weapon) {
                wanted.push((format!("glove:{}", item.weapon), None));
            }
            if item.stattrak.is_some()
                && (parent.layer == 0
                    || parent.bones.iter().any(|(n, _)| n == "v_weapon.stattrack"))
            {
                wanted.push((
                    if crate::cosmetics::is_rare_knife(&item.weapon) {
                        "models/weapons/stattrack_cut.mdl"
                    } else {
                        "models/weapons/stattrack.mdl"
                    }
                    .into(),
                    None,
                ));
            }
            for (path, sticker) in wanted {
                let glove = path.starts_with("glove:");
                let path = if parent.layer == 0 && !glove {
                    if path.contains("stattrack") {
                        format!("world:{}:{path}", item.weapon)
                    } else {
                        format!("world:{path}")
                    }
                } else {
                    path
                };
                let Some(asset) = models.decoration(&path) else {
                    continue;
                };
                if !asset
                    .model
                    .bones
                    .iter()
                    .filter(|b| b.parent<0)
                    .all(|b| parent.bones.iter().any(|(n, _)| *n == b.name))
                {
                    continue;
                }
                let mut asset = asset.clone();
                let mut own = vec![];
                for (index, (_, mat)) in asset.parts.iter_mut().enumerate() {
                    let Some(mut m) = materials.get(&*mat).cloned() else {
                        continue;
                    };
                    let name = &asset.model.textures[asset
                        .model
                        .skin_texture(0, asset.model.meshes[index].material)];
                    if let Some(sticker) = sticker {
                        let Some(def) = catalog.stickers.get(&sticker.kit) else {
                            continue;
                        };
                        if let Some(vmt) =
                            vmt::Material::load(&def.material, &|p| level.0.vfs.read_string(p))
                        {
                            if let Some(t) = vmt.base_texture().and_then(|n| {
                                state.textures.load(&level.0.vfs, &mut images, n, true)
                            }) {
                                m.base = t.handle;
                            }
                            m.params.flags = flags::DYNAMIC_LIT | flags::TRANSLUCENT;
                            m.params.color = Vec4::ONE;
                            m.params.skin = Vec4::ZERO;
                            m.params.skin_details.z = sticker.wear;
                            m.params.skin_details.w = 1.;
                            m.params.phong = Vec4::new(
                                vmt.f32("$phongexponent").unwrap_or(32.),
                                vmt.f32("$phongboost").unwrap_or(0.3),
                                0.,
                                0.,
                            );
                            if let (Some(mask), Some(spectrum)) =
                                (vmt.str("$holomask"), vmt.str("$holospectrum"))
                            {
                                if let Some(t) =
                                    state.textures.load(&level.0.vfs, &mut images, mask, false)
                                {
                                    m.modulate = t.handle;
                                }
                                if let Some(t) =
                                    state
                                        .textures
                                        .load(&level.0.vfs, &mut images, spectrum, true)
                                {
                                    m.base2 = t.handle;
                                    m.params.skin_properties.z = 1.;
                                }
                            }
                            if let Some(t) = vmt
                                .str("$normalmap")
                                .or_else(|| vmt.str("$bumpmap"))
                                .and_then(|n| {
                                    state.textures.load(&level.0.vfs, &mut images, n, false)
                                })
                            {
                                m.normal = t.handle;
                                m.params.flags |= flags::NORMAL;
                            }
                            m.alpha_mode = AlphaMode::Blend;
                            m.cull = false;
                        }
                    } else {
                        let digit = name
                            .chars()
                            .rev()
                            .take(3)
                            .collect::<String>()
                            .chars()
                            .rev()
                            .collect::<String>()
                            .parse::<u32>()
                            .ok();
                        if let Some(d) = digit.filter(|_| name.contains("digit")) {
                            let value = (item.stattrak.unwrap_or(0) / 10_u32.pow(d.min(9))) % 10;
                            let texture = if name.starts_with("cut_") {
                                "models/weapons/stattrack/cut_digit_d"
                            } else {
                                "models/weapons/stattrack/stat_digit"
                            };
                            let key = (texture.to_string(), value as usize);
                            if !state.digits.contains_key(&key) {
                                if let Some(v) = level
                                    .0
                                    .vfs
                                    .read(&vmt::vtf_path(texture))
                                    .and_then(|b| vtf::Vtf::parse(&b).ok())
                                {
                                    if let Some(bytes) = v.rgba8(0, value as usize, 0) {
                                        let image=Image::new(bevy::render::render_resource::Extent3d{width:v.width as u32,height:v.height as u32,depth_or_array_layers:1},bevy::render::render_resource::TextureDimension::D2,bytes,bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,bevy::asset::RenderAssetUsages::RENDER_WORLD);
                                        state.digits.insert(key.clone(), images.add(image));
                                    }
                                }
                            }
                            if let Some(h) = state.digits.get(&key) {
                                m.base = h.clone();
                            }
                            m.params.flags = flags::UNLIT | flags::TRANSLUCENT;
                            m.params.color = Vec4::ONE;
                            m.alpha_mode = AlphaMode::Blend;
                        }
                        if name.contains("error") {
                            m.params.color.w = 0.;
                            m.alpha_mode = AlphaMode::Blend;
                        }
                    }
                    *mat = materials.add(m);
                    own.push(mat.clone());
                }
                let instance = skinning::spawn(
                    &mut commands,
                    &asset,
                    Some(RenderLayers::layer(parent.layer)),
                    None,
                );
                for e in &instance.meshes {commands.entity(*e).insert(Visibility::Hidden);}
                if glove {
                    crate::skin_finish::tag(
                        &mut commands,
                        &instance,
                        crate::skin_finish::Target::Gloves,
                    );
                }
                additions.push(Part {
                    asset,
                    instance,
                    materials: own,
                });
            }
            state.instances.insert(entity, (item, additions));
        }
        if let Some((_, parts)) = state.instances.get(&entity) {
            let light = materials.get(&base.0).map(|m| m.params);
            for part in parts {
                for (bone, joint) in part.asset.model.bones.iter().zip(&part.instance.joints) {
                    if let Some((_, source)) = parent.bones.iter().find(|(n, _)| *n == bone.name) {
                        if let Ok(t) = transforms.get(*source).copied() {
                            if let Ok(mut dest) = transforms.get_mut(*joint) {
                                *dest = t;
                            }
                        }
                    } else if bone.parent>=0 {
                        if let Some(parent_joint)=part.instance.joints.get(bone.parent as usize) {
                            if let Ok(t)=transforms.get(*parent_joint).copied(){
                                let axis=skinning::axis_change();
                                let local=glam::Mat4::from_rotation_translation(bone.quat,bone.pos);
                                let local=Mat4::from_cols_array(&local.to_cols_array());
                                if let Ok(mut dest)=transforms.get_mut(*joint){
                                    *dest=Transform::from_matrix(t.to_matrix()*axis*local*axis.inverse());
                                }
                            }
                        }
                    }
                }
                for e in &part.instance.meshes {
                    if let Ok(mut v) = visibility.get_mut(*e) {
                        *v = *visible;
                    }
                }
                if let Some(light) = light {
                    for h in &part.materials {
                        if let Some(mut m) = materials.get_mut(h) {
                            m.params.ambient_cube = light.ambient_cube;
                            m.params.sun_dir = light.sun_dir;
                            m.params.sun_color = light.sun_color;
                            m.params.exposure = light.exposure;
                        }
                    }
                }
            }
        }
    }
}
