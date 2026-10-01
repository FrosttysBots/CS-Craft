//! Other players: skinned CS:GO character models animated from the game's
//! animation set, holding their current weapon's world model, lit from the
//! map's ambient light and the sun every few frames.

use crate::coords::{rotation_to_bevy, to_bevy};
use crate::material::WorldMaterial;
use crate::player::{LocalPlayer, SimWorld};
use crate::player_anim::{AnimInput, AnimState, PlayerAnimSet};
use crate::props::Sun;
use crate::skinning::{self, SkinnedAsset, SkinnedInstance};
use crate::textures::TextureCache;
use crate::world_mesh::SharedHandles;
use crate::LoadedLevel;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use collision::contents;
use std::collections::HashMap;
use std::sync::Arc;

pub const T_MODEL: &str = "models/player/custom_player/legacy/tm_phoenix.mdl";
pub const CT_MODEL: &str = "models/player/custom_player/legacy/ctm_sas.mdl";

pub struct TeamModel {
    pub asset: SkinnedAsset,
    pub anims: PlayerAnimSet,
}

/// Everything needed to draw players.
#[derive(Resource)]
pub struct CharacterModels {
    pub terrorist: Option<TeamModel>,
    pub counter_terrorist: Option<TeamModel>,
    /// Third-person weapon models by weapon class name.
    pub weapons: HashMap<String, SkinnedAsset>,
    pub sun: Option<Sun>,
}

struct Character {
    team: sim::Team,
    body: SkinnedInstance,
    materials: Vec<Handle<WorldMaterial>>,
    anim: AnimState,
    weapon: Option<(String, SkinnedInstance, Vec<Handle<WorldMaterial>>)>,
    air_time: f32,
    light_timer: f32,
}

#[derive(Resource, Default)]
pub struct Characters(HashMap<sim::PlayerId, Character>);

fn load_model(fs: &vfs::Vfs, path: &str) -> Option<mdl::Model> {
    let base = path.trim_end_matches(".mdl");
    let m = fs.read(path)?;
    let v = fs.read(&format!("{base}.vvd"))?;
    let x = fs.read(&format!("{base}.dx90.vtx"))?;
    match mdl::Model::load(&m, &v, &x) {
        Ok(model) => Some(model),
        Err(e) => {
            warn!("{path}: {e}");
            None
        }
    }
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
) -> CharacterModels {
    let fs = &level.vfs;
    let mut team = |path: &str| -> Option<TeamModel> {
        let model = Arc::new(load_model(fs, path)?);
        let anims = PlayerAnimSet::load(&model, fs)?;
        let asset = skinning::build_asset(model, fs, meshes, materials, images, textures, shared, bindposes, exposure, true);
        Some(TeamModel { asset, anims })
    };
    let terrorist = team(T_MODEL);
    let counter_terrorist = team(CT_MODEL);
    let mut weapons = HashMap::new();
    for def in &level.catalog.weapons {
        if def.world_model.is_empty() || weapons.contains_key(&def.class_name) {
            continue;
        }
        if let Some(model) = load_model(fs, &def.world_model) {
            let asset = skinning::build_asset(Arc::new(model), fs, meshes, materials, images, textures, shared, bindposes, exposure, true);
            weapons.insert(def.class_name.clone(), asset);
        }
    }
    CharacterModels {
        terrorist,
        counter_terrorist,
        weapons,
        sun: Sun::from_bsp(&level.bsp),
    }
}

fn own_materials(instance: &SkinnedInstance, query: &Query<&MeshMaterial3d<WorldMaterial>>) -> Vec<Handle<WorldMaterial>> {
    instance.meshes.iter().filter_map(|e| query.get(*e).ok().map(|m| m.0.clone())).collect()
}

fn despawn_instance(commands: &mut Commands, i: &SkinnedInstance) {
    for e in i.meshes.iter().chain(&i.joints) {
        commands.entity(*e).despawn();
    }
}

fn angle_diff(a: f32, b: f32) -> f32 {
    (a - b + 180.0).rem_euclid(360.0) - 180.0
}

/// Ambient cube + sun visibility at a point, in material form.
pub fn lighting_at(level: &level::Level, sun: Option<Sun>, chest: glam::Vec3) -> ([Vec4; 6], Vec4, Vec4) {
    let mut cube = level
        .bsp
        .ambient_cube(chest)
        .unwrap_or([[0.35; 3]; 6]);
    // Like the engine's model ambient boost: keep characters readable in
    // dark spots by lifting a dim ambient up to a minimum brightness.
    const AMBIENT_MIN: f32 = 0.12;
    const AMBIENT_MAX_BOOST: f32 = 5.0;
    let lum = cube.iter().map(|c| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).sum::<f32>() / 6.0;
    if lum < AMBIENT_MIN {
        let k = (AMBIENT_MIN / lum.max(1e-3)).min(AMBIENT_MAX_BOOST);
        for c in &mut cube {
            for v in c.iter_mut() {
                *v *= k;
            }
        }
    }
    let bevy_cube = [cube[0], cube[1], cube[4], cube[5], cube[3], cube[2]].map(|c| Vec4::new(c[0], c[1], c[2], 1.0));
    match sun {
        Some(s) => {
            let tr = level.collision.trace_ray(chest, chest + s.to_sun * 16384.0, contents::MASK_SOLID);
            let lit = !tr.hit() || level.collision.is_sky(tr.surface);
            (
                bevy_cube,
                to_bevy(s.to_sun).extend(if lit { 1.0 } else { 0.0 }),
                Vec4::new(s.color.x, s.color.y, s.color.z, 1.0),
            )
        }
        None => (bevy_cube, Vec4::new(0.0, 1.0, 0.0, 0.0), Vec4::ZERO),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn sync(
    mut commands: Commands,
    world: Res<SimWorld>,
    local: Res<LocalPlayer>,
    mut models: ResMut<CharacterModels>,
    mut chars: ResMut<Characters>,
    mut materials: ResMut<Assets<WorldMaterial>>,
    level: Res<LoadedLevel>,
    effects: Res<crate::effects::Effects>,
    time: Res<Time>,
    mat_query: Query<&MeshMaterial3d<WorldMaterial>>,
    mut transforms: Query<&mut Transform>,
    mut visibility: Query<&mut Visibility>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    let sim_now = world.0.time();
    let sun = models.sun;

    // Drop characters whose players left.
    let gone: Vec<sim::PlayerId> = chars.0.keys().copied().filter(|id| world.0.player(*id).is_none()).collect();
    for id in gone {
        if let Some(c) = chars.0.remove(&id) {
            despawn_instance(&mut commands, &c.body);
            if let Some((_, w, _)) = &c.weapon {
                despawn_instance(&mut commands, w);
            }
        }
    }

    for p in &world.0.players {
        let team_model = match p.team {
            sim::Team::Terrorist => models.terrorist.as_ref(),
            _ => models.counter_terrorist.as_ref(),
        };
        let Some(team_model) = team_model else { continue };
        if !chars.0.contains_key(&p.id) || chars.0[&p.id].team != p.team {
            if let Some(old) = chars.0.remove(&p.id) {
                despawn_instance(&mut commands, &old.body);
            }
            let body = skinning::spawn(&mut commands, &team_model.asset, None, Some(&mut materials));
            chars.0.insert(
                p.id,
                Character {
                    team: p.team,
                    body,
                    materials: Vec::new(),
                    anim: AnimState::default(),
                    weapon: None,
                    air_time: 0.0,
                    light_timer: 0.0,
                },
            );
            // Materials become queryable next frame.
            continue;
        }
        let c = chars.0.get_mut(&p.id).unwrap();
        if c.materials.is_empty() {
            c.materials = own_materials(&c.body, &mat_query);
        }

        let visible = p.alive && (p.id != local.id || local.noclip);
        for e in c.body.meshes.iter().chain(c.weapon.iter().flat_map(|w| w.1.meshes.iter())) {
            if let Ok(mut v) = visibility.get_mut(*e) {
                *v = if visible { Visibility::Inherited } else { Visibility::Hidden };
            }
        }
        if !visible {
            continue;
        }

        // Weapon in hand follows the active weapon.
        let active = p.active_weapon().map(|w| w.def.class_name.clone()).unwrap_or_default();
        if c.weapon.as_ref().map(|w| w.0.as_str()) != Some(active.as_str()) {
            if let Some((_, w, _)) = c.weapon.take() {
                despawn_instance(&mut commands, &w);
            }
            if let Some(asset) = models.weapons.get(&active) {
                let inst = skinning::spawn(&mut commands, asset, None, Some(&mut materials));
                c.weapon = Some((active.clone(), inst, Vec::new()));
            }
        }
        if let Some((_, inst, mats)) = &mut c.weapon {
            if mats.is_empty() {
                *mats = own_materials(inst, &mat_query);
            }
        }

        // Animation inputs from the simulation.
        let v = p.movement.velocity;
        let speed = v.truncate().length();
        let vel_yaw = v.y.atan2(v.x).to_degrees();
        c.air_time = if p.movement.on_ground { 0.0 } else { c.air_time + dt };
        let weapon = p.active_weapon();
        let reload_progress = weapon.and_then(|w| {
            w.reload_end.map(|end| {
                let total = sim::weapon::reload_time(&w.def).max(0.1);
                (1.0 - (end - sim_now) / total).clamp(0.0, 1.0)
            })
        });
        let input = AnimInput {
            speed,
            move_yaw: if speed > 1.0 { angle_diff(vel_yaw, p.view_angles.y) } else { 0.0 },
            on_ground: p.movement.on_ground,
            ducked: p.movement.ducked,
            pitch: p.view_angles.x,
            class: weapon.map_or(weapons::WeaponClass::Knife, |w| w.def.class),
            time_since_fire: effects.last_fire.get(&p.id).map_or(10.0, |t| now - t),
            reload_progress,
            air_time: c.air_time,
        };
        let CharacterModels { terrorist, counter_terrorist, weapons: weapon_assets, .. } = &mut *models;
        let anims = match p.team {
            sim::Team::Terrorist => terrorist.as_mut().map(|m| &mut m.anims),
            _ => counter_terrorist.as_mut().map(|m| &mut m.anims),
        };
        let Some(anims) = anims else { continue };
        let pose = anims.evaluate(&mut c.anim, &input, dt);
        let world_from_model = Mat4::from_rotation_translation(
            rotation_to_bevy(mdl::angles_to_mat3(glam::Vec3::new(0.0, p.view_angles.y, 0.0))),
            to_bevy(p.movement.origin),
        );
        skinning::apply_pose(&c.body, world_from_model, &pose, &mut transforms);
        if let Some((name, inst, _)) = &c.weapon {
            if let Some(asset) = weapon_assets.get(name) {
                let wpose = skinning::bone_merge(&asset.model.bones, anims.bones(), &pose);
                skinning::apply_pose(inst, world_from_model, &wpose, &mut transforms);
            }
        }

        // Lighting, ~6 times a second.
        c.light_timer -= dt;
        if c.light_timer <= 0.0 {
            c.light_timer = 0.15;
            let chest = p.movement.origin + glam::Vec3::Z * 40.0;
            let (cube, sun_dir, sun_color) = lighting_at(&level.0, sun, chest);
            let weapon_mats = c.weapon.iter().flat_map(|w| w.2.iter());
            for h in c.materials.iter().chain(weapon_mats) {
                if let Some(mut m) = materials.get_mut(h) {
                    m.params.ambient_cube = cube;
                    m.params.sun_dir = sun_dir;
                    m.params.sun_color = sun_color;
                }
            }
        }
    }
}
