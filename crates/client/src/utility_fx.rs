//! Shared grenade state drives bounded visuals; no separate client-side hit/visibility rules.
use crate::{
    coords::to_bevy,
    player::{LocalPlayer, SimWorld},
};
use bevy::prelude::*;
use sim::utility::Kind;
use std::collections::HashMap;

#[derive(Component)]
pub struct FlashOverlay;
struct Flying {
    instance: crate::skinning::SkinnedInstance,
    pose: Vec<glam::Mat4>,
    base: Mat4,
}
#[derive(Resource)]
pub struct AssetsFx {
    sphere: Handle<Mesh>,
    smoke: Handle<StandardMaterial>,
    fire: Vec<Handle<StandardMaterial>>,
    fields: HashMap<u32, Vec<Entity>>,
    projectiles: HashMap<u32, Flying>,
}
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    level: Res<crate::LoadedLevel>,
) {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let smoke_texture = level
        .0
        .vfs
        .read("materials/particle/particle_smokegrenade.vtf")
        .and_then(|b| vtf::Vtf::parse(&b).ok())
        .and_then(|v| {
            Some(images.add(Image::new(
                Extent3d {
                    width: v.width as u32,
                    height: v.height as u32,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                v.rgba8(0, 0, 0)?,
                TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::RENDER_WORLD,
            )))
        });
    // This installed fire sheet is a horizontal strip of square sprite frames.
    let fire_frames=level.0.vfs.read("materials/particle/fire_particle_4/fire_particle_4.vtf")
        .and_then(|b|vtf::Vtf::parse(&b).ok()).and_then(|v| {
            let pixels=v.rgba8(0,0,0)?;let size=v.height as usize;let width=v.width as usize;
            if size==0 || width%size!=0 {return None}
            let mut frames=Vec::new();
            for frame in 0..width/size {
                let mut rgba=Vec::with_capacity(size*size*4);
                for row in 0..size {let start=(row*width+frame*size)*4;rgba.extend_from_slice(&pixels[start..start+size*4]);}
                let image=images.add(Image::new(Extent3d{width:size as u32,height:size as u32,depth_or_array_layers:1},TextureDimension::D2,rgba,TextureFormat::Rgba8UnormSrgb,bevy::asset::RenderAssetUsages::RENDER_WORLD));
                frames.push(mats.add(StandardMaterial{base_color:Color::srgba(1.0,0.8,0.6,0.75),base_color_texture:Some(image),unlit:true,alpha_mode:AlphaMode::Add,cull_mode:None,..default()}));
            }
            Some(frames)
        }).unwrap_or_else(|| {warn!("Original fire sprite sheet missing");vec![mats.add(StandardMaterial{base_color:Color::srgb(1.0,0.4,0.05),unlit:true,..default()})]});
    commands.insert_resource(AssetsFx {
        sphere: meshes.add(Rectangle::new(2.0, 2.0)),
        smoke: mats.add(StandardMaterial {
            base_color: Color::srgba(0.62, 0.60, 0.55, 0.95),
            base_color_texture: smoke_texture,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            cull_mode: None,
            ..default()
        }),
        fire: fire_frames,
        fields: HashMap::new(),
        projectiles: HashMap::new(),
    });
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        BackgroundColor(Color::NONE),
        GlobalZIndex(90),
        FlashOverlay,
    ));
}
#[allow(clippy::too_many_arguments)]
pub fn update(
    mut commands: Commands,
    world: Res<SimWorld>,
    local: Res<LocalPlayer>,
    mut fx: ResMut<AssetsFx>,
    characters: Res<crate::characters::CharacterModels>,
    mut mats: ResMut<Assets<crate::material::WorldMaterial>>,
    mut transforms: Query<&mut Transform>,
    mut sprite_materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut overlay: Query<&mut BackgroundColor, With<FlashOverlay>>,
    fixed: Res<Time<Fixed>>,
) {
    let w = &world.0;
    let time = w.time();
    let gone: Vec<_> = fx
        .fields
        .keys()
        .filter(|id| !w.utility_fields.iter().any(|f| f.id == **id))
        .copied()
        .collect();
    for id in gone {
        if let Some(entities) = fx.fields.remove(&id) {
            for e in entities {
                commands.entity(e).despawn();
            }
        }
    }
    for field in w
        .utility_fields
        .iter()
        .filter(|f| f.kind == Kind::Smoke || f.kind.fire())
    {
        if !fx.fields.contains_key(&field.id) {
            let material = if field.kind == Kind::Smoke {
                fx.smoke.clone()
            } else {
                fx.fire[0].clone()
            };
            let entities = (0..18)
                .map(|_| {
                    commands
                        .spawn((
                            Mesh3d(fx.sphere.clone()),
                            MeshMaterial3d(material.clone()),
                            Transform::default(),
                        ))
                        .id()
                })
                .collect();
            fx.fields.insert(field.id, entities);
        }
        let growth = ((time - field.born) / 0.6).clamp(0.02, 1.0)
            * ((field.until - time) / 1.0).clamp(0.02, 1.0);
        for (i, e) in fx.fields[&field.id].iter().enumerate() {
            let a = i as f32 * std::f32::consts::TAU / 8.0;
            let offset = if i % 9 == 8 {
                Vec3::ZERO
            } else {
                Vec3::new(a.cos(), 0.0, a.sin()) * 65.0
            };
            let offset = Vec3::new(offset.x, if i > 8 { 22.0 } else { -15.0 }, offset.z);
            let smoke = field.kind == Kind::Smoke;
            let pos = to_bevy(field.pos)
                + Vec3::new(offset.x, if smoke { offset.y } else { 0.0 }, offset.z) * growth
                + Vec3::Y * if smoke { 55.0 } else { 30.0 };
            if !smoke {if let Ok(mut mat)=sprite_materials.get_mut(*e) {mat.0=fx.fire[((time*18.0) as usize+i*3)%fx.fire.len()].clone();}}
            let pulse = 1.0 + 0.1 * (time * 7.0 + i as f32).sin();
            if let Ok(mut tr) = transforms.get_mut(*e) {
                tr.translation = pos;
                tr.rotation = Transform::from_translation(pos)
                    .looking_at(to_bevy(local.render_eye), Vec3::Y)
                    .rotation;
                tr.scale = if smoke {
                    Vec3::splat(100.0 * growth)
                } else {
                    Vec3::new(38.0, 45.0 * pulse, 1.0) * growth
                };
            }
        }
    }
    let gone: Vec<_> = fx
        .projectiles
        .keys()
        .filter(|id| !w.projectiles.iter().any(|p| p.id == **id))
        .copied()
        .collect();
    for id in gone {
        if let Some(inst) = fx.projectiles.remove(&id) {
            for e in inst.instance.meshes.iter().chain(&inst.instance.joints) {
                commands.entity(*e).despawn();
            }
        }
    }
    for p in &w.projectiles {
        let Some(asset) = characters.weapons.get(p.kind.name()) else {
            continue;
        };
        let pos = p.pos + p.velocity * (fixed.overstep_fraction() * sim::TICK_DT);
        let spin = Mat4::from_rotation_translation(
            Quat::from_rotation_y((time - p.born) * 4.0),
            to_bevy(pos),
        );
        if !fx.projectiles.contains_key(&p.id) {
            let base = crate::items::ground_transform(&asset.model, glam::Vec3::ZERO, 0.0, false);
            let pose = mdl::skeleton::model_space(
                &asset.model.bones,
                &mdl::skeleton::rest_pose(&asset.model.bones),
            );
            let instance =
                crate::skinning::spawn_posed(&mut commands, asset, &mut mats, spin * base);
            fx.projectiles.insert(
                p.id,
                Flying {
                    instance,
                    pose,
                    base,
                },
            );
        } else {
            let flying = &fx.projectiles[&p.id];
            crate::skinning::apply_pose(
                &flying.instance,
                spin * flying.base,
                &flying.pose,
                &mut transforms,
            );
        }
    }
    let flash = w.flash_amount(local.view_target());
    let smoke = w.utility_fields.iter().any(|f| {
        f.kind == Kind::Smoke
            && local.curr_eye.distance(f.pos + glam::Vec3::Z * 55.0) < 130.0
            && time - f.born > 0.3
    });
    for mut color in &mut overlay {
        *color = BackgroundColor(if flash > 0.0 {
            Color::srgba(1.0, 1.0, 1.0, flash)
        } else if smoke {
            Color::srgba(0.46, 0.44, 0.40, 1.0)
        } else {
            Color::NONE
        });
    }
}
