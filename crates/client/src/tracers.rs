//! Cosmetic tracers follow the rendered muzzle; the simulation owns hit rays.
use crate::{
    coords::{to_bevy, to_source},
    player::LocalPlayer,
};
use bevy::prelude::*;
use std::collections::HashMap;

#[derive(Resource, Default)]
pub struct InspectRequest(pub u64);
#[derive(Resource, Default)]
pub struct Muzzles {
    pub first: Option<(sim::PlayerId, Vec3)>,
    pub world: HashMap<sim::PlayerId, Vec3>,
}
struct Trail {
    start: Vec3,
    end: Vec3,
    born: f32,
}
#[derive(Resource, Default)]
pub struct Tracers {
    pub pending: Vec<(sim::PlayerId, glam::Vec3, glam::Vec3)>,
    live: Vec<Trail>,
}
/// Prefer the authored attachment, including its offset from the parent bone.
pub fn muzzle_attachment(model: &mdl::Model, left: bool) -> Option<(usize, glam::Vec3)> {
    let name = if left { "2" } else { "1" };
    model
        .attachments
        .iter()
        .find(|a| a.name == name)
        .or_else(|| model.attachments.iter().find(|a| a.name == "muzzle_flash"))
        .map(|a| (a.bone, a.position))
        .or_else(|| {
            model
                .bone_index("weapon_muzzle")
                .or_else(|| {
                    model.bones.iter().position(|b| {
                        let name = b.name.to_ascii_lowercase();
                        name.contains("flash") || name.ends_with("muzzle")
                    })
                })
                .map(|i| (i, glam::Vec3::ZERO))
        })
}
pub fn muzzle_position(model: &mdl::Model, pose: &[glam::Mat4], left: bool) -> Option<glam::Vec3> {
    let (bone, offset) = muzzle_attachment(model, left)?;
    Some(pose.get(bone)?.transform_point3(offset))
}
/// Compensate for the separate viewmodel FOV so the world trail starts at the
/// same screen pixel as the visible barrel, including scoped-rifle ADS.
pub fn project_viewmodel_muzzle(
    muzzle: Vec3,
    eye: Vec3,
    forward: Vec3,
    world_fov: f32,
    view_fov: f32,
) -> Vec3 {
    let delta = muzzle - eye;
    let axial = forward * delta.dot(forward);
    eye + axial + (delta - axial) * ((world_fov * 0.5).tan() / (view_fov * 0.5).tan())
}
pub fn segment(start: Vec3, end: Vec3, age: f32) -> Option<(Vec3, Vec3, f32)> {
    let distance = start.distance(end);
    if distance < 1.0 || age < 0.0 || age > 0.12 {
        return None;
    }
    let tail = age * 7000.0;
    if tail >= distance {
        return None;
    }
    let direction = (end - start) / distance;
    Some((
        start + direction * tail,
        start + direction * (tail + 85.0).min(distance),
        0.25 * (1.0 - age / 0.12),
    ))
}
pub fn draw(
    mut trails: ResMut<Tracers>,
    muzzles: Res<Muzzles>,
    local: Res<LocalPlayer>,
    level: Res<crate::LoadedLevel>,
    time: Res<Time>,
    mut gizmos: Gizmos,
) {
    let now = time.elapsed_secs();
    let pending = std::mem::take(&mut trails.pending);
    for (owner, eye, end) in pending {
        let muzzle = if owner == local.view_target() && !local.noclip {
            muzzles.first.filter(|m| m.0 == owner).map(|m| m.1)
        } else {
            muzzles.world.get(&owner).copied()
        };
        let Some(mut start) = muzzle else { continue };
        // Do not let a cosmetic muzzle origin poke through nearby cover.
        let trace =
            level
                .0
                .collision
                .trace_ray(eye, to_source(start), collision::contents::MASK_SHOT);
        if trace.hit() {
            start = to_bevy(trace.end + trace.normal * 0.5);
        }
        let end = to_bevy(end);
        if (end - start).dot(end - to_bevy(eye)) <= 0.0 {
            continue;
        }
        if trails.live.len() < 256 {
            trails.live.push(Trail {
                start,
                end,
                born: now,
            });
        }
    }
    trails
        .live
        .retain(|t| segment(t.start, t.end, now - t.born).is_some());
    for trail in &trails.live {
        if let Some((a, b, alpha)) = segment(trail.start, trail.end, now - trail.born) {
            gizmos.line(a, b, Color::srgba(0.48, 0.44, 0.34, alpha));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn muzzle_uses_attachment_offset_in_animated_bone_space() {
        let model=mdl::Model {attachments:vec![mdl::Attachment{name:"1".into(),bone:0,position:glam::Vec3::new(0.0,3.0,9.0)}],..Default::default()};
        let pose=[glam::Mat4::from_translation(glam::Vec3::new(10.0,20.0,30.0))];
        assert_eq!(muzzle_position(&model,&pose,false),Some(glam::Vec3::new(10.0,23.0,39.0)));
        assert!(muzzle_position(&model,&[],false).is_none());
    }
    #[test]
    fn trail_starts_at_muzzle_and_never_overshoots_hit() {
        let start = Vec3::new(20.0, -3.0, 2.0);
        let end = start + Vec3::X * 40.0;
        let (a, b, alpha) = segment(start, end, 0.0).unwrap();
        assert_eq!(a, start);
        assert_eq!(b, end);
        assert!(alpha <= 0.25);
        assert!(segment(start, end, 0.01).is_none());
    }
    #[test]
    fn corrected_muzzle_has_same_screen_projection() {
        let p = Vec3::new(4.0, 2.0, -25.0);
        let wf = 1.28;
        let vf = 0.82;
        let q = project_viewmodel_muzzle(p, Vec3::ZERO, Vec3::NEG_Z, wf, vf);
        assert!((p.x / (-p.z * (vf / 2.0).tan()) - q.x / (-q.z * (wf / 2.0).tan())).abs() < 0.0001);
    }
}
