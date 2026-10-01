//! First-person weapon ("viewmodel").
//!
//! CS:GO draws the held weapon from a `v_*.mdl` with the team's arms model
//! bone-merged onto it, using the weapon's own animations (`ACT_VM_DRAW`,
//! `ACT_VM_IDLE`, `ACT_VM_PRIMARYATTACK`, `ACT_VM_RELOAD`). It is rendered by
//! a dedicated camera on its own render layer with a fresh depth buffer and
//! its own field of view, so the gun never clips into walls.

use crate::coords::{rotation_to_bevy, to_bevy};
use crate::material::WorldMaterial;
use crate::player::{LocalPlayer, SimWorld};
use crate::skinning::{self, SkinnedAsset, SkinnedInstance};
use crate::textures::TextureCache;
use crate::world_mesh::SharedHandles;
use crate::LoadedLevel;
use bevy::camera::visibility::RenderLayers;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use mdl::anim::Clip;
use std::collections::HashMap;
use std::sync::Arc;

pub const VIEWMODEL_LAYER: usize = 2;
/// CS:GO `viewmodel_fov` default (horizontal degrees at 4:3).
pub const VIEWMODEL_FOV: f32 = 60.0;
/// CS:GO `viewmodel_offset_x/y/z` defaults: right, forward, up.
pub const VIEWMODEL_OFFSET: glam::Vec3 = glam::Vec3::new(1.0, 1.0, -1.0);

pub const T_ARMS: &str = "models/weapons/t_arms_phoenix.mdl";
pub const CT_ARMS: &str = "models/weapons/ct_arms_sas.mdl";

pub fn layer() -> RenderLayers {
    RenderLayers::layer(VIEWMODEL_LAYER)
}

/// Vertical FOV (radians) for a horizontal 4:3 FOV in degrees, as Source scales it.
pub fn vertical_fov(horizontal_4x3_deg: f32) -> f32 {
    2.0 * ((horizontal_4x3_deg.to_radians() * 0.5).tan() * 0.75).atan()
}

/// A decoded sequence: its clip and per-bone weights.
#[derive(Clone)]
struct SeqClip {
    clip: Arc<Clip>,
    weights: Arc<Vec<f32>>,
}

struct WeaponView {
    asset: SkinnedAsset,
    /// Activity → sequences (decoded once).
    clips: HashMap<String, Vec<SeqClip>>,
}

#[derive(Resource, Default)]
pub struct ViewModels {
    weapons: HashMap<String, WeaponView>,
    arms_t: Option<SkinnedAsset>,
    arms_ct: Option<SkinnedAsset>,
}

#[derive(Resource, Default)]
pub struct ViewModelState {
    weapon: Option<(String, SkinnedInstance, Option<SkinnedInstance>, Vec<Handle<WorldMaterial>>)>,
    team: Option<sim::Team>,
    activity: String,
    clip_index: usize,
    started: f32,
    last_fire_seen: f32,
    reloading: bool,
    light_timer: f32,
    sway: glam::Vec2,
    last_angles: glam::Vec2,
    /// Persistent bone-local pose: sequences only overwrite the bones they
    /// animate (weighted), as the game layers masked viewmodel sequences.
    pose: Vec<(glam::Vec3, glam::Quat)>,
}

fn load_model(fs: &vfs::Vfs, path: &str) -> Option<mdl::Model> {
    let base = path.trim_end_matches(".mdl");
    mdl::Model::load(&fs.read(path)?, &fs.read(&format!("{base}.vvd"))?, &fs.read(&format!("{base}.dx90.vtx"))?).ok()
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
) -> ViewModels {
    let fs = &level.vfs;
    let mut out = ViewModels::default();
    let mut build = |model: mdl::Model| {
        skinning::build_asset(Arc::new(model), fs, meshes, materials, images, textures, shared, bindposes, exposure, true)
    };
    for def in &level.catalog.weapons {
        if def.view_model.is_empty() || out.weapons.contains_key(&def.class_name) {
            continue;
        }
        let Some(model) = load_model(fs, &def.view_model) else { continue };
        let mut clips: HashMap<String, Vec<SeqClip>> = HashMap::new();
        // Sequences live in the model itself or in its include models
        // (knives), in which case they are retargeted by bone name.
        let mut sources: Vec<mdl::Model> = Vec::new();
        for inc in &model.include_models {
            if let Some(m) = fs.read(inc).and_then(|b| mdl::Model::load_header(&b).ok()) {
                sources.push(m);
            }
        }
        let own = std::iter::once(&model).chain(sources.iter());
        for src in own {
            let ani = (!src.anim_block_file.is_empty()).then(|| fs.read(&src.anim_block_file)).flatten();
            for seq in &src.sequences {
                if seq.activity.is_empty() {
                    continue;
                }
                let Some(&first) = seq.anims.first() else { continue };
                match src.decode_animation(first as usize, ani.as_deref()) {
                    Ok(c) => {
                        let same = std::ptr::eq(src, &model);
                        let (clip, weights) = if same {
                            (c, (0..model.bones.len()).map(|b| seq.weight(b)).collect())
                        } else {
                            (
                                mdl::anim::retarget(&c, &src.bones, &model.bones),
                                mdl::anim::retarget_weights(seq, &src.bones, &model.bones),
                            )
                        };
                        clips.entry(seq.activity.to_ascii_uppercase()).or_default().push(SeqClip {
                            clip: Arc::new(clip),
                            weights: Arc::new(weights),
                        });
                    }
                    Err(e) => warn!("{}: {}: {e}", def.view_model, seq.name),
                }
            }
        }
        // Knives attack with "miss/hit centre" activities.
        if !clips.contains_key("ACT_VM_PRIMARYATTACK") {
            if let Some(c) = clips.get("ACT_VM_MISSCENTER").cloned() {
                clips.insert("ACT_VM_PRIMARYATTACK".into(), c);
            }
        }
        out.weapons.insert(def.class_name.clone(), WeaponView { asset: build(model), clips });
    }
    out.arms_t = load_model(fs, T_ARMS).map(&mut build);
    out.arms_ct = load_model(fs, CT_ARMS).map(&mut build);
    out
}

fn despawn(commands: &mut Commands, i: &SkinnedInstance) {
    for e in i.meshes.iter().chain(&i.joints) {
        commands.entity(*e).despawn();
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update(
    mut commands: Commands,
    world: Res<SimWorld>,
    local: Res<LocalPlayer>,
    models: Res<ViewModels>,
    mut state: ResMut<ViewModelState>,
    mut materials: ResMut<Assets<WorldMaterial>>,
    effects: Res<crate::effects::Effects>,
    level: Res<LoadedLevel>,
    characters: Res<crate::characters::CharacterModels>,
    time: Res<Time>,
    mat_query: Query<&MeshMaterial3d<WorldMaterial>>,
    mut transforms: Query<&mut Transform>,
    mut visibility: Query<&mut Visibility>,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let Some(p) = world.0.player(local.id) else { return };
    let show = p.alive && !local.noclip;
    let active = p.active_weapon().map(|w| w.def.class_name.clone()).unwrap_or_default();

    // Respawn the viewmodel when the weapon or team changes.
    let current = state.weapon.as_ref().map(|w| w.0.clone());
    if current.as_deref() != Some(active.as_str()) || state.team != Some(p.team) {
        if let Some((_, gun, arms, _)) = state.weapon.take() {
            despawn(&mut commands, &gun);
            if let Some(a) = arms {
                despawn(&mut commands, &a);
            }
        }
        if let Some(view) = models.weapons.get(&active) {
            let gun = skinning::spawn(&mut commands, &view.asset, Some(layer()), Some(&mut *materials));
            let arms_asset = match p.team {
                sim::Team::Terrorist => models.arms_t.as_ref(),
                _ => models.arms_ct.as_ref(),
            };
            // Knives and some weapons carry their own hands; others get arms.
            let arms = arms_asset.map(|a| skinning::spawn(&mut commands, a, Some(layer()), Some(&mut *materials)));
            state.weapon = Some((active.clone(), gun, arms, Vec::new()));
            state.team = Some(p.team);
            state.activity = "ACT_VM_DRAW".into();
            state.clip_index = 0;
            state.started = now;
            state.light_timer = 0.0;
            state.pose = mdl::skeleton::rest_pose(&view.asset.model.bones);
        }
        return;
    }
    let Some((name, gun, arms, mats)) = &mut state.weapon else { return };
    if mats.is_empty() {
        *mats = gun
            .meshes
            .iter()
            .chain(arms.iter().flat_map(|a| a.meshes.iter()))
            .filter_map(|e| mat_query.get(*e).ok().map(|m| m.0.clone()))
            .collect();
    }
    let (name, gun, arms) = (name.clone(), gun.clone(), arms.clone());
    let entities: Vec<Entity> = gun.meshes.iter().chain(arms.iter().flat_map(|a| a.meshes.iter())).copied().collect();
    for e in &entities {
        if let Ok(mut v) = visibility.get_mut(*e) {
            *v = if show { Visibility::Inherited } else { Visibility::Hidden };
        }
    }
    if !show {
        return;
    }
    let Some(view) = models.weapons.get(&name) else { return };

    // Pick the activity from the weapon state.
    let w = p.active_weapon();
    let reloading = w.is_some_and(|w| w.reload_end.is_some());
    let last_fire = effects.last_fire.get(&local.id).copied().unwrap_or(-1.0);
    if reloading && !state.reloading {
        state.activity = "ACT_VM_RELOAD".into();
        state.started = now;
    } else if last_fire > state.last_fire_seen {
        state.activity = "ACT_VM_PRIMARYATTACK".into();
        state.clip_index = (state.clip_index + 1) % 3;
        state.started = now;
    }
    state.reloading = reloading;
    state.last_fire_seen = last_fire;
    let clips = view
        .clips
        .get(&state.activity)
        .filter(|c| !c.is_empty())
        .or_else(|| view.clips.get("ACT_VM_IDLE"));
    let Some(clips) = clips else { return };
    let mut seq = clips[state.clip_index % clips.len()].clone();
    let mut clip = seq.clip.clone();
    let mut t = now - state.started;
    if state.activity == "ACT_VM_RELOAD" {
        // Stretch the reload animation over the simulated reload time.
        if let Some(w) = w {
            let total = sim::weapon::reload_time(&w.def);
            t = t / total.max(0.1) * clip.duration();
        }
    }
    if state.activity != "ACT_VM_IDLE" && t > clip.duration() && !reloading {
        state.activity = "ACT_VM_IDLE".into();
        state.started = now;
        if let Some(idle) = view.clips.get("ACT_VM_IDLE").and_then(|c| c.first()) {
            seq = idle.clone();
            clip = idle.clip.clone();
            t = 0.0;
        }
    }

    let model = &view.asset.model;
    if state.pose.len() != model.bones.len() {
        state.pose = mdl::skeleton::rest_pose(&model.bones);
    }
    let weights = seq.weights.clone();
    clip.sample_layered(t, &mut state.pose, |b| weights.get(b).copied().unwrap_or(1.0));
    let pose = mdl::skeleton::model_space(&model.bones, &state.pose);

    // Sway: the gun lags slightly behind fast turns.
    let angles = glam::Vec2::new(local.pitch, local.yaw);
    let delta = glam::Vec2::new(angles.x - state.last_angles.x, (angles.y - state.last_angles.y + 180.0).rem_euclid(360.0) - 180.0);
    state.last_angles = angles;
    state.sway = (state.sway + delta * 0.15).clamp(glam::Vec2::splat(-2.0), glam::Vec2::splat(2.0)) * (1.0 - (dt * 8.0).min(1.0));

    let punch = p.view_punch();
    let view_angles = glam::Vec3::new(local.pitch + punch[0] - state.sway.x, local.yaw + punch[1] - state.sway.y, 0.0);
    let rot = mdl::angles_to_mat3(view_angles);
    let eye = local.curr_eye;
    let (f, r, u) = movement::angle_vectors(view_angles);
    let origin = eye + r * VIEWMODEL_OFFSET.x + f * VIEWMODEL_OFFSET.y + u * VIEWMODEL_OFFSET.z;
    let world_from_model = Mat4::from_rotation_translation(rotation_to_bevy(rot), to_bevy(origin));
    skinning::apply_pose(&gun, world_from_model, &pose, &mut transforms);
    if let Some(arms) = &arms {
        let arms_model = match p.team {
            sim::Team::Terrorist => models.arms_t.as_ref(),
            _ => models.arms_ct.as_ref(),
        };
        if let Some(a) = arms_model {
            let arms_pose = skinning::bone_merge(&a.model.bones, &model.bones, &pose);
            skinning::apply_pose(arms, world_from_model, &arms_pose, &mut transforms);
        }
    }

    state.light_timer -= dt;
    if state.light_timer <= 0.0 {
        state.light_timer = 0.15;
        let (cube, sun_dir, sun_color) = crate::characters::lighting_at(&level.0, characters.sun, eye);
        if let Some((_, _, _, mats)) = &state.weapon {
            for h in mats {
                if let Some(mut m) = materials.get_mut(h) {
                    m.params.ambient_cube = cube;
                    m.params.sun_dir = sun_dir;
                    m.params.sun_color = sun_color;
                }
            }
        }
    }
}
