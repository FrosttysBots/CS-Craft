//! Things on the ground: dropped weapons, defuse kits and the planted bomb,
//! drawn with the game's world models and lit like characters.

use crate::LoadedLevel;
use crate::characters::{CharacterModels, lighting_at};
use crate::coords::{rotation_to_bevy, to_bevy};
use crate::material::WorldMaterial;
use crate::player::SimWorld;
use crate::skinning::{self, SkinnedAsset, SkinnedInstance};
use crate::textures::TextureCache;
use crate::world_mesh::SharedHandles;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

pub const PLANTED_C4_MODEL: &str = "models/weapons/w_c4_planted.mdl";
pub const DEFUSER_MODEL: &str = "models/weapons/w_defuser.mdl";

#[derive(Resource)]
pub struct GroundModels {
    pub planted: Option<SkinnedAsset>,
    pub kit: Option<SkinnedAsset>,
}

struct Placed {
    instance: SkinnedInstance,
    materials: Vec<Handle<WorldMaterial>>,
    pos: glam::Vec3,
}

#[derive(Resource, Default)]
pub struct GroundInstances {
    items: HashMap<u32, Placed>,
    bomb: Option<(glam::Vec3, Placed)>,
}

#[allow(clippy::too_many_arguments)]
pub fn load(
    level: &level::Level,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WorldMaterial>,
    images: &mut Assets<Image>,
    textures: &mut TextureCache,
    shared: &SharedHandles,
    bindposes: &mut Assets<SkinnedMeshInverseBindposes>,
    exposure: f32,
) -> GroundModels {
    let fs = &level.vfs;
    let mut asset = |path: &str| -> Option<SkinnedAsset> {
        let base = path.trim_end_matches(".mdl");
        let model = mdl::Model::load(
            &fs.read(path)?,
            &fs.read(&format!("{base}.vvd"))?,
            &fs.read(&format!("{base}.dx90.vtx"))?,
        )
        .ok()?;
        Some(skinning::build_asset(
            Arc::new(model),
            fs,
            meshes,
            materials,
            images,
            textures,
            shared,
            bindposes,
            exposure,
            true,
        ))
    };
    GroundModels {
        planted: asset(PLANTED_C4_MODEL),
        kit: asset(DEFUSER_MODEL),
    }
}

fn despawn(commands: &mut Commands, p: &Placed) {
    for e in p.instance.meshes.iter().chain(&p.instance.joints) {
        commands.entity(*e).despawn();
    }
}

/// Weapons lie on their side; the bomb and kits sit upright.
pub(crate) fn ground_transform(model: &mdl::Model, pos: glam::Vec3, yaw: f32, on_side: bool) -> Mat4 {
    let roll = if on_side { 90.0 } else { 0.0 };
    // World weapons include the character's reference skeleton. Undo its
    // authored hand rotation before laying the weapon's thin axis down.
    let hand = model
        .bones
        .iter()
        .find(|b| b.name == "weapon_hand_R")
        .map_or(glam::Mat3::IDENTITY, |b| {
            glam::Mat3::from_mat4(b.pose_to_bone)
        });
    let rot = mdl::angles_to_mat3(glam::Vec3::new(0.0, yaw, roll)) * hand;
    // World models are authored relative to the held skeleton, not a floor
    // pivot. Place the lowest rotated vertex on the surface and centre XY.
    let offset = resting_offset(
        model
            .meshes
            .iter()
            .flat_map(|m| m.vertices.iter().map(|v| rot * v.position)),
    );
    Mat4::from_rotation_translation(rotation_to_bevy(rot), to_bevy(pos + offset))
}

fn resting_offset(points: impl Iterator<Item = glam::Vec3>) -> glam::Vec3 {
    let (mut lo, mut hi) = (
        glam::Vec3::splat(f32::INFINITY),
        glam::Vec3::splat(f32::NEG_INFINITY),
    );
    for p in points {
        lo = lo.min(p);
        hi = hi.max(p);
    }
    if !lo.is_finite() {
        return glam::Vec3::ZERO;
    }
    glam::Vec3::new(-(lo.x + hi.x) * 0.5, -(lo.y + hi.y) * 0.5, 0.25 - lo.z)
}

#[allow(clippy::too_many_arguments)]
pub fn sync(
    mut commands: Commands,
    world: Res<SimWorld>,
    characters: Res<CharacterModels>,
    ground: Res<GroundModels>,
    mut placed: ResMut<GroundInstances>,
    mut materials: ResMut<Assets<WorldMaterial>>,
    level: Res<LoadedLevel>,
    mat_query: Query<&MeshMaterial3d<WorldMaterial>>,
) {
    let world = &world.0;
    let sun = characters.sun;

    // Items that were picked up or cleared.
    let gone: Vec<u32> = placed
        .items
        .keys()
        .copied()
        .filter(|id| !world.items.iter().any(|i| i.id == *id))
        .collect();
    for id in gone {
        if let Some(p) = placed.items.remove(&id) {
            despawn(&mut commands, &p);
        }
    }
    for item in &world.items {
        if placed.items.contains_key(&item.id) {
            continue;
        }
        let (asset, on_side) = match &item.kind {
            sim::GroundKind::Weapon(w) => (
                characters.weapons.get(&w.def.class_name),
                w.def.class_name != sim::weapon::C4,
            ),
            sim::GroundKind::DefuseKit => (ground.kit.as_ref(), false),
        };
        let Some(asset) = asset else { continue };
        let instance = skinning::spawn_posed(
            &mut commands,
            asset,
            &mut materials,
            ground_transform(&asset.model, item.pos, item.yaw, on_side),
        );
        crate::skin_finish::tag(&mut commands,&instance,crate::skin_finish::Target::Ground(item.id));
        crate::decorations::tag(&mut commands,asset,&instance,crate::skin_finish::Target::Ground(item.id),0);
        placed.items.insert(
            item.id,
            Placed {
                instance,
                materials: Vec::new(),
                pos: item.pos,
            },
        );
    }

    // The planted bomb.
    let bomb_pos = world.bomb.map(|b| b.pos);
    if placed.bomb.as_ref().map(|b| b.0) != bomb_pos {
        if let Some((_, p)) = placed.bomb.take() {
            despawn(&mut commands, &p);
        }
        if let (Some(pos), Some(asset)) = (bomb_pos, ground.planted.as_ref()) {
            let instance = skinning::spawn_posed(
                &mut commands,
                asset,
                &mut materials,
                ground_transform(&asset.model, pos, 0.0, false),
            );
            placed.bomb = Some((
                pos,
                Placed {
                    instance,
                    materials: Vec::new(),
                    pos,
                },
            ));
        }
    }

    // Light new instances once their materials exist.
    let GroundInstances { items, bomb } = &mut *placed;
    for p in items.values_mut().chain(bomb.iter_mut().map(|b| &mut b.1)) {
        if !p.materials.is_empty() {
            continue;
        }
        p.materials = p
            .instance
            .meshes
            .iter()
            .filter_map(|e| mat_query.get(*e).ok().map(|m| m.0.clone()))
            .collect();
        if p.materials.is_empty() {
            continue;
        }
        let (cube, sun_dir, sun_color) = lighting_at(&level.0, sun, p.pos + glam::Vec3::Z * 8.0);
        for h in &p.materials {
            if let Some(mut m) = materials.get_mut(h) {
                m.params.ambient_cube = cube;
                m.params.sun_dir = sun_dir;
                m.params.sun_color = sun_color;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires the user's installed CS:GO Legacy assets"]
    fn installed_world_weapons_rest_flat_on_the_surface() {
        let (_, fs) = level::mount_game(None).unwrap();
        let catalog = weapons::Catalog::from_items_game(
            &fs.read_string("scripts/items/items_game.txt").unwrap(),
        )
        .unwrap();
        for name in [
            "weapon_ak47",
            "weapon_m4a1_silencer",
            "weapon_glock",
            "weapon_knife",
        ] {
            let def = catalog.get(name).unwrap();
            let base = def.world_model.trim_end_matches(".mdl");
            let model = mdl::Model::load(
                &fs.read(&def.world_model).unwrap(),
                &fs.read(&format!("{base}.vvd")).unwrap(),
                &fs.read(&format!("{base}.dx90.vtx")).unwrap(),
            )
            .unwrap();
            for yaw in [0.0, 35.0, 180.0] {
                let transform =
                    ground_transform(&model, glam::Vec3::new(100.0, 200.0, 30.0), yaw, true);
                let mut min = f32::INFINITY;
                let mut max = f32::NEG_INFINITY;
                for vertex in model.meshes.iter().flat_map(|m| &m.vertices) {
                    let z = transform.transform_point3(to_bevy(vertex.position)).y;
                    min = min.min(z);
                    max = max.max(z);
                }
                assert!((min - 30.25).abs() < 0.01, "{name} misses the floor: {min}");
                assert!(
                    max - min < 4.0,
                    "{name} is standing upright: height {}",
                    max - min
                );
            }
        }
    }
    #[test]
    fn held_model_pivot_is_removed_for_ground_placement() {
        for roll in [0.0, 90.0] {
            for yaw in [0.0, 35.0, 180.0] {
                let rot = mdl::angles_to_mat3(glam::Vec3::new(0.0, yaw, roll));
                let points = [
                    glam::Vec3::new(15.0, -8.0, 35.0),
                    glam::Vec3::new(50.0, 4.0, 60.0),
                    glam::Vec3::new(20.0, 6.0, 42.0),
                ]
                .map(|p| rot * p);
                let offset = resting_offset(points.into_iter());
                let lo = points
                    .into_iter()
                    .map(|p| p + offset)
                    .fold(glam::Vec3::splat(f32::INFINITY), glam::Vec3::min);
                let hi = points
                    .into_iter()
                    .map(|p| p + offset)
                    .fold(glam::Vec3::splat(f32::NEG_INFINITY), glam::Vec3::max);
                assert!((lo.z - 0.25).abs() < 0.001);
                assert!((lo.x + hi.x).abs() < 0.001 && (lo.y + hi.y).abs() < 0.001);
            }
        }
    }
}
