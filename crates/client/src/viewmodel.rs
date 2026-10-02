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
    left_muzzle: bool,
}

struct WeaponView {
    scope_center: Option<glam::Vec3>,
    scope_rear: Option<(glam::Vec3, f32)>,
    asset: SkinnedAsset,
    /// Activity → sequences (decoded once).
    clips: HashMap<String, Vec<SeqClip>>,
}

#[derive(Resource, Default)]
pub struct ViewModels {
    weapons: HashMap<String, WeaponView>,
    decorations:HashMap<String,SkinnedAsset>,
    gloves:HashMap<String,SkinnedAsset>,
    arms_t: Option<SkinnedAsset>,
    arms_ct: Option<SkinnedAsset>,
}

impl ViewModels {
    pub fn preview(&self,name:&str)->Option<&SkinnedAsset>{self.weapons.get(name).map(|w|&w.asset)}
    pub fn decoration(&self,path:&str)->Option<&SkinnedAsset>{self.decorations.get(path)}
}
#[derive(Resource, Default)]
pub struct ViewModelState {
    weapon: Option<(String, SkinnedInstance, Option<SkinnedInstance>, Vec<Handle<WorldMaterial>>)>,
    glove:String,
    team: Option<sim::Team>,
    owner: Option<sim::PlayerId>,
    activity: String,
    clip_index: usize,
    started: f32,
    last_attack_seen: u64,
    inspect_seen: u64,
    reloading: bool,
    /// Last shell insert seen (for the debug trace).
    shell_seen: Option<i32>,
    planting: bool,
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
    catalog:&crate::cosmetics::Catalog,
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
                if seq.activity.is_empty() && seq.name != "lookat01" && seq.name != "inspect" {
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
                        clips.entry(if seq.name == "lookat01" || seq.name == "inspect" {"INSPECT".into()}else{seq.activity.to_ascii_uppercase()}).or_default().push(SeqClip {
                            clip: Arc::new(clip),
                            weights: Arc::new(weights),
                            left_muzzle: seq.name.to_ascii_lowercase().contains("left"),
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
        // Align the optic using its original lens geometry in the idle pose.
        let mut scope_rear = None;
        let scope_center = if def.attr_f32("zoom levels").unwrap_or(0.0) > 0.0 && def.class == weapons::WeaponClass::Rifle {
            let mut idle = mdl::skeleton::rest_pose(&model.bones);
            if let Some(seq) = clips.get("ACT_VM_IDLE").and_then(|c| c.first()) {
                seq.clip.sample_layered(0.0, &mut idle, |b| seq.weights.get(b).copied().unwrap_or(1.0));
            }
            let pose = mdl::skeleton::model_space(&model.bones, &idle);
            let mut center = glam::Vec3::ZERO;
            let mut count = 0;
            for mesh in &model.meshes {
                let material = model.skins.first().and_then(|s| s.get(mesh.material)).copied().unwrap_or(mesh.material);
                let name = model.textures.get(material).map(String::as_str).unwrap_or("");
                let rear = name.contains("scope_glass");
                if !rear && !name.contains("scope_lens_dirt") { continue; }
                let mut min = glam::Vec3::splat(f32::INFINITY);
                let mut max = -min;
                for vertex in &mesh.vertices {
                    let mut position = glam::Vec3::ZERO;
                    for i in 0..vertex.bone_count as usize {
                        let bone = vertex.bones[i] as usize;
                        position += (pose[bone] * model.bones[bone].pose_to_bone).transform_point3(vertex.position) * vertex.weights[i];
                    }
                    min = min.min(position); max = max.max(position);
                    if !rear { center += position; count += 1; }
                }
                if rear && !mesh.vertices.is_empty() {
                    let size = max - min;
                    scope_rear = Some(((min + max) * 0.5, size.y.min(size.z) * 0.5));
                }
            }
            (count > 0).then(|| center / count as f32)
        } else { None };
        out.weapons.insert(def.class_name.clone(), WeaponView { asset: build(model), clips, scope_center, scope_rear });
    }
    for (name,path,_) in crate::cosmetic_catalog::model_items(fs) {
        if let Some(model)=load_model(fs,&path){for (side,arms) in [("Terrorist",T_ARMS),("CounterTerrorist",CT_ARMS)]{
            if let Some(arms)=load_model(fs,arms){out.gloves.insert(format!("{name}:{side}"),build(crate::cosmetic_geometry::sleeves(model.clone(),&arms)));}
        }}
    }
    {
        for path in catalog.models.values().flat_map(|m|m.stickers.iter().cloned()).chain(["models/weapons/stattrack.mdl".into(),"models/weapons/stattrack_cut.mdl".into()]){
            if !out.decorations.contains_key(&path){if let Some(model)=load_model(fs,&path){out.decorations.insert(path,build(model));}}
        }
    }
    {
        for (name,info) in &catalog.models{
            if info.stickers.is_empty(){continue;}
            let Some(view)=out.weapons.get(name).map(|v|v.asset.model.clone())else{continue};
            let Some(world)=load_model(fs,&info.world)else{continue};
            for path in &info.stickers {
                if let Some(decal)=out.decorations.get(path).map(|a|a.model.clone()){
                    if let Some(model)=crate::cosmetic_geometry::world_decal(&decal,&view,&world){out.decorations.insert(format!("world:{path}"),build(model));}
                    else{warn!("Could not map world sticker {}",path);}
                }
            }
        }
    }
    for (name,_,path) in crate::cosmetic_catalog::model_items(fs){if let Some(model)=load_model(fs,&path){out.decorations.insert(format!("glove:{name}"),build(model));}}
    for (name,view) in &out.weapons {
        if !view.asset.model.bones.iter().any(|b|b.name=="v_weapon.stattrack"){continue;}
        let path=if crate::cosmetics::is_rare_knife(name){"models/weapons/stattrack_cut.mdl"}else{"models/weapons/stattrack.mdl"};
        let (Some(counter),Some(info))=(out.decorations.get(path),catalog.models.get(name))else{continue};
        let Some(world)=load_model(fs,&info.world)else{continue};
        let mut counter=(*counter.model).clone();
        let pose:Vec<_>=view.asset.model.bones.iter().map(|b|b.pose_to_bone.inverse()).collect();
        let merged=skinning::bone_merge(&counter.bones,&view.asset.model.bones,&pose);
        for mesh in &mut counter.meshes {for vertex in &mut mesh.vertices{
            let bone=vertex.bones[0] as usize;
            let transform=merged[bone]*counter.bones[bone].pose_to_bone;
            vertex.position=transform.transform_point3(vertex.position);
            vertex.normal=transform.transform_vector3(vertex.normal).normalize_or_zero();
        }}
        if let Some(model)=crate::cosmetic_geometry::world_decal(&counter,&view.asset.model,&world){
            out.decorations.insert(format!("world:{name}:{path}"),build(model));
        }
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
    (request,inventory,building): (Res<crate::tracers::InspectRequest>,Res<crate::cosmetics::Inventory>,Res<crate::overworld_client::Building>),
    mut muzzles: ResMut<crate::tracers::Muzzles>,
    mut materials: ResMut<Assets<WorldMaterial>>,
    effects: Res<crate::effects::Effects>,
    level: Res<LoadedLevel>,
    characters: Res<crate::characters::CharacterModels>,
    (time, fixed): (Res<Time>, Res<Time<Fixed>>),
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    mat_query: Query<&MeshMaterial3d<WorldMaterial>>,
    mut transforms: Query<&mut Transform>,
    mut visibility: Query<&mut Visibility>,
) {
    muzzles.first=None;
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let Some(p) = world.0.player(local.view_target()) else { return };
    let show = !building.active && p.alive && !local.noclip && !p.active_weapon().is_some_and(|w| w.hides_viewmodel());
    let active = p.active_weapon().map(|w| w.def.class_name.clone()).unwrap_or_default();

    let glove=if p.id==local.id{inventory.active("gloves").map(|i|i.weapon.as_str()).unwrap_or("")}else{""};
    // Respawn the viewmodel when the weapon or team changes.
    let current = state.weapon.as_ref().map(|w| w.0.clone());
    if current.as_deref() != Some(active.as_str()) || state.team != Some(p.team) || state.owner != Some(p.id) || state.glove!=glove {
        if let Some((_, gun, arms, _)) = state.weapon.take() {
            despawn(&mut commands, &gun);
            if let Some(a) = arms {
                despawn(&mut commands, &a);
            }
        }
        if let Some(view) = models.weapons.get(&active) {
            let gun = skinning::spawn(&mut commands, &view.asset, Some(layer()), Some(&mut *materials));
            crate::skin_finish::tag(&mut commands,&gun,crate::skin_finish::Target::View);
            crate::decorations::tag(&mut commands,&view.asset,&gun,crate::skin_finish::Target::View,VIEWMODEL_LAYER);
            let arms_asset = models.gloves.get(&format!("{}:{:?}",glove,p.team)).or_else(||match p.team {
                sim::Team::Terrorist => models.arms_t.as_ref(),
                _ => models.arms_ct.as_ref(),
            });
            // Knives and some weapons carry their own hands; others get arms.
            let arms = arms_asset.map(|a| skinning::spawn(&mut commands, a, Some(layer()), Some(&mut *materials)));
            if !glove.is_empty(){if let Some(a)=&arms{crate::skin_finish::tag(&mut commands,a,crate::skin_finish::Target::Gloves);}}
            state.glove=glove.into();
            state.weapon = Some((active.clone(), gun, arms, Vec::new()));
            state.team = Some(p.team);
            state.owner = Some(p.id);
            state.sway = glam::Vec2::ZERO;
            state.last_angles = local.render_angles.truncate();
            state.inspect_seen = request.0;
            state.reloading = false;
            state.planting = false;
            state.last_attack_seen = effects.attack_serial;
            state.activity = "ACT_VM_DRAW".into();
            state.clip_index = 0;
            state.started = now;
            state.light_timer = 0.0;
            state.pose = mdl::skeleton::rest_pose(&view.asset.model.bones);
            if std::env::var_os("CSR_WEAPON_TRACE").is_some() { info!("weapon activity: {} ACT_VM_DRAW", active); }
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
    for (index, e) in entities.iter().enumerate() {
        let clear_lens = p.active_weapon().is_some_and(|w| w.zoom_level > 0 && w.def.class == weapons::WeaponClass::Rifle)
            && models.weapons.get(&name).and_then(|view| view.asset.model.meshes.get(index).map(|mesh| {
                let model = &view.asset.model;
                let material = model.skins.first().and_then(|s| s.get(mesh.material)).copied().unwrap_or(mesh.material);
                model.textures.get(material).is_some_and(|m| m.starts_with("scope_") || m.contains("scope_glass"))
            })).unwrap_or(false);
        if let Ok(mut v) = visibility.get_mut(*e) {
            *v = if show && !clear_lens { Visibility::Inherited } else { Visibility::Hidden };
        }
    }
    if !p.alive || local.noclip { return; }
    let Some(view) = models.weapons.get(&name) else { return };

    // Pick the activity from the weapon state.
    let w = p.active_weapon();
    let inspect_requested=state.inspect_seen!=request.0;state.inspect_seen=request.0;
    let reloading = w.is_some_and(|w| w.reload_end.is_some());
    let attack = effects.consume_attack(p.id, &name, &mut state.last_attack_seen);
    // Shotguns that load shell by shell follow the simulation's stage:
    // the start, then one insert per shell, then (below) the finish.
    let sim_now = world.0.time() + fixed.overstep_fraction() * sim::TICK_DT;
    let shell = w.and_then(|w| w.shell_stage(sim_now));
    let shell_gun = w.is_some_and(|w| w.shell_reload().is_some());
    if reloading && !state.reloading {
        state.activity = if shell_gun { "ACT_SHOTGUN_RELOAD_START" } else { "ACT_VM_RELOAD" }.into();
        state.started = now;
    } else if !reloading && state.reloading && shell_gun && attack.is_none() && view.clips.contains_key("ACT_SHOTGUN_RELOAD_FINISH") {
        state.activity = "ACT_SHOTGUN_RELOAD_FINISH".into();
        state.started = now;
    } else if p.plant_start.is_some() && !state.planting {
        // C4: the keypad sequence while planting.
        state.activity = "ACT_VM_PRIMARYATTACK".into();
        state.started = now;
    } else if let Some(activity) = attack {
        state.activity = activity.into();
        state.clip_index = (state.clip_index + 1) % 3;
        state.started = now;
    }
    if attack.is_none() && !reloading && p.plant_start.is_none() && w.is_some_and(|w|w.zoom_level==0 && w.next_attack<=world.0.time()) && inspect_requested && view.clips.contains_key("INSPECT") {
        state.activity="INSPECT".into();state.started=now;state.clip_index=0;
    }
    if state.activity=="INSPECT" && w.is_some_and(|w|w.zoom_level>0) {state.activity="ACT_VM_IDLE".into();state.started=now;}
    state.reloading = reloading;
    state.planting = p.plant_start.is_some();
    if let Some((stage, _)) = shell {
        if stage != state.shell_seen && stage.is_some() && std::env::var_os("CSR_WEAPON_TRACE").is_some() {
            info!("weapon activity: {} shell insert {}", name, stage.unwrap_or(0) + 1);
        }
        state.shell_seen = stage;
        if matches!(state.activity.as_str(), "ACT_SHOTGUN_RELOAD_START" | "ACT_VM_RELOAD") {
            let start = stage.is_none() && view.clips.contains_key("ACT_SHOTGUN_RELOAD_START");
            state.activity = if start { "ACT_SHOTGUN_RELOAD_START" } else { "ACT_VM_RELOAD" }.into();
        }
    }
    let clips = view
        .clips
        .get(&state.activity)
        .filter(|c| !c.is_empty())
        .or_else(|| {
            // Some knife variants name only the miss activities.
            match state.activity.as_str() {
                "ACT_VM_HITCENTER2" => view.clips.get("ACT_VM_MISSCENTER2"),
                "ACT_VM_HITCENTER" => view.clips.get("ACT_VM_MISSCENTER"),
                _ => None,
            }
        })
        .or_else(|| view.clips.get("ACT_VM_IDLE"));
    let Some(clips) = clips else { return };
    let mut seq = clips[state.clip_index % clips.len()].clone();
    let mut clip = seq.clip.clone();
    let mut t = now - state.started;
    if let (Some((_, frac)), "ACT_SHOTGUN_RELOAD_START" | "ACT_VM_RELOAD") = (shell, state.activity.as_str()) {
        // One pass of the start or insert clip per simulated stage.
        t = frac * clip.duration();
    } else if state.activity == "ACT_VM_RELOAD" {
        // Stretch the reload animation over the simulated reload time.
        if let Some(frac) = w.and_then(|w| w.reload_fraction(sim_now)) {
            t = frac * clip.duration();
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

    if state.started == now && std::env::var_os("CSR_WEAPON_TRACE").is_some() {
        info!("weapon activity: {} {}", name, state.activity);
    }
    let model = &view.asset.model;
    if state.pose.len() != model.bones.len() {
        state.pose = mdl::skeleton::rest_pose(&model.bones);
    }
    let weights = seq.weights.clone();
    clip.sample_layered(t, &mut state.pose, |b| weights.get(b).copied().unwrap_or(1.0));
    let pose = mdl::skeleton::model_space(&model.bones, &state.pose);

    // Sway: the gun lags slightly behind fast turns.
    let angles = local.render_angles.truncate();
    let delta = glam::Vec2::new(angles.x - state.last_angles.x, (angles.y - state.last_angles.y + 180.0).rem_euclid(360.0) - 180.0);
    state.last_angles = angles;
    state.sway = (state.sway + delta * 0.15).clamp(glam::Vec2::splat(-2.0), glam::Vec2::splat(2.0)) * (-dt * 8.0).exp();

    let aiming = w.is_some_and(|w| w.zoom_level > 0);
    let sway = if aiming { glam::Vec3::ZERO } else { glam::Vec3::new(state.sway.x, state.sway.y, 0.0) };
    let view_angles = local.render_angles - sway;
    let rot = mdl::angles_to_mat3(view_angles);
    let eye = local.render_eye;
    let (f, r, u) = movement::angle_vectors(view_angles);
    let mut offset = glam::Vec3::new(VIEWMODEL_OFFSET.y, -VIEWMODEL_OFFSET.x, VIEWMODEL_OFFSET.z);
    if let (Some(center), Some(w)) = (view.scope_center, w) {
        let scoped_fov = w.def.attr_f32("zoom fov 1").unwrap_or(45.0);
        let blend = ((90.0 - w.rendered_fov(world.0.time())) / (90.0 - scoped_fov)).clamp(0.0, 1.0);
        let rear = view.scope_rear.map_or(center, |(position, _)| position);
        offset = offset.lerp(glam::Vec3::new(8.0 - center.x, -rear.y, -rear.z), blend);
    }
    let origin = eye + f * offset.x - r * offset.y + u * offset.z;
    let world_from_model = Mat4::from_rotation_translation(rotation_to_bevy(rot), to_bevy(origin));
    skinning::apply_pose(&gun, world_from_model, &pose, &mut transforms);
    if show {if let Some(point)=crate::tracers::muzzle_position(model,&pose,seq.left_muzzle) {
        let muzzle=world_from_model.transform_point3(to_bevy(point));
        let world_fov=vertical_fov(w.map_or(90.0,|w|w.rendered_fov(world.0.time())));
        muzzles.first=Some((p.id,crate::tracers::project_viewmodel_muzzle(muzzle,to_bevy(eye),to_bevy(movement::angle_vectors(local.render_angles).0),world_fov,vertical_fov(VIEWMODEL_FOV))));
    }}
    if let Some(arms) = &arms {
        let arms_model = models.gloves.get(&format!("{}:{:?}",glove,p.team)).or_else(||match p.team {
            sim::Team::Terrorist => models.arms_t.as_ref(),
            _ => models.arms_ct.as_ref(),
        });
        if let Some(a) = arms_model {
            let arms_pose = skinning::bone_merge(&a.model.bones, &model.bones, &pose);
            skinning::apply_pose(arms, world_from_model, &arms_pose, &mut transforms);
        }
    }

    let aperture = if w.is_some_and(|w| w.zoom_level > 0 && w.def.class == weapons::WeaponClass::Rifle) {
        view.scope_rear.map_or(Vec4::ZERO, |(rear, radius)| {
            let lens = rear + offset;
            let focal = window.physical_height() as f32 * 0.5 / (vertical_fov(VIEWMODEL_FOV) * 0.5).tan();
            let distance = lens.x.max(1.0);
            Vec4::new(window.physical_width() as f32 * 0.5 - lens.y * focal / distance,
                window.physical_height() as f32 * 0.5 - lens.z * focal / distance,
                radius * focal / distance * 0.97, 0.0)
        })
    } else { Vec4::ZERO };
    if let Some((_, _, _, mats)) = &state.weapon {
        for handle in mats { if let Some(mut material) = materials.get_mut(handle) { material.params.scope_aperture = aperture; } }
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
