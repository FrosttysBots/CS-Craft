//! Pack-backed pinned 26.3 spider: `SpiderModel.createSpiderBodyLayer` (a
//! head, a small and a large body segment, and eight legs splayed out
//! sideways), its `setupAnim` (the head's turn and pitch; the legs sweep
//! back and forth and lift in four phases with the walk), and
//! `SpiderEyesLayer`'s eyes: the same model with `spider_eyes`, drawn at
//! full brightness where vanilla adds it unlit. The death flip is not drawn.
use crate::{
    cow_render::cube_scaled,
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
    client_mobs::ClientMobs,
};
use glam::{Quat, Vec3};
use minecraftoss_entities::world::SpiderEntity;
use std::f32::consts::PI;

type Part = ([f32; 3], [f32; 3], [f32; 2], [f32; 3]);

const HEAD: Part = ([-4., -4., -8.], [4., 4., 0.], [32., 4.], [0., 15., -3.]);
const BODY_0: Part = ([-3., -3., -3.], [3., 3., 3.], [0., 0.], [0., 15., 0.]);
const BODY_1: Part = ([-5., -4., -6.], [5., 4., 6.], [0., 12.], [0., 15., 9.]);

/// A leg's pivot, resting yaw and roll, and which of the four walk phases
/// it follows (hind, middle hind, middle front, front). Right legs reach
/// out along -X; left legs are their mirror.
const LEGS: [([f32; 3], f32, f32, usize); 8] = [
    ([-4., 15., 2.], 0.785_398_2, -0.785_398_2, 0),
    ([4., 15., 2.], -0.785_398_2, 0.785_398_2, 0),
    ([-4., 15., 1.], 0.392_699_1, -0.581_194_64, 1),
    ([4., 15., 1.], -0.392_699_1, 0.581_194_64, 1),
    ([-4., 15., 0.], -0.392_699_1, -0.581_194_64, 2),
    ([4., 15., 0.], 0.392_699_1, 0.581_194_64, 2),
    ([-4., 15., -1.], -0.785_398_2, -0.785_398_2, 3),
    ([4., 15., -1.], 0.785_398_2, 0.785_398_2, 3),
];

/// `SpiderModel.setupAnim`'s leg sweep (yaw) and lift (roll) for each of
/// the four phases at a walk position and speed.
pub fn leg_motion(position: f32, speed: f32) -> [(f32, f32); 4] {
    let p = position * 0.6662;
    [0.0, PI, PI / 2.0, 4.712_389].map(|phase| {
        let swing = -((p * 2.0 + phase).cos() * 0.4) * speed;
        let step = ((p + phase).sin() * 0.4).abs() * speed;
        (swing, step)
    })
}

pub fn append_spiders<'a>(
    mesh: &mut ChunkMesh,
    spiders: impl IntoIterator<Item = &'a SpiderEntity>,
    poses: &ClientMobs,
    atlas: &Atlas,
    light: &SkyLight,
    partial: f32,
) {
    let skin = atlas.entity_region(&ResourceId::parse("minecraft:entity/spider/spider").unwrap());
    let eyes_id = ResourceId::parse("minecraft:entity/spider/spider_eyes").unwrap();
    let eyes = atlas.contains(&eyes_id).then(|| atlas.entity_region(&eyes_id));
    let partial = partial.clamp(0.0, 1.0);
    // Each mob's first vertex and overlay (`getOverlayCoords`).
    let mut marks = Vec::new();
    for entity in spiders {
        let Some(mob) = poses.pose(entity.id, partial) else { continue };
        marks.push((mesh.vertices.len(), mob.overlay(0.0)));
        let feet = mob.feet;
        let sample = mob.light_block();
        let (sky, block) = (light.get(sample) as f32, light.get_block(sample) as f32);
        let rotation = mob.body_rotation(180.0);
        let head = Quat::from_euler(glam::EulerRot::ZYX, 0.0, mob.head_yaw.to_radians(), mob.head_pitch.to_radians());
        let motion = leg_motion(mob.walk_position, mob.walk_speed);
        let mut parts: Vec<(Part, Quat, bool)> = vec![(HEAD, head, false), (BODY_0, Quat::IDENTITY, false), (BODY_1, Quat::IDENTITY, false)];
        for (pivot, yaw, roll, phase) in LEGS {
            let left = pivot[0] > 0.0;
            let (swing, step) = motion[phase];
            let (yaw, roll) = if left { (yaw - swing, roll - step) } else { (yaw + swing, roll + step) };
            let (from, to) = if left { ([-1., -1., -1.], [15., 1., 1.]) } else { ([-15., -1., -1.], [1., 1., 1.]) };
            parts.push(((from, to, [18., 0.], pivot), Quat::from_euler(glam::EulerRot::ZYX, roll, yaw, 0.0), left));
        }
        for &((from, to, uv, pivot), pose, mirror) in &parts {
            cube_scaled(mesh, feet, rotation, Vec3::ONE, skin, sky, block, from, to, uv, pivot, pose, [1.0; 3], [64., 32.], None, mirror);
        }
        if let Some(eyes) = eyes {
            let ((from, to, uv, pivot), pose, _) = parts[0];
            cube_scaled(mesh, feet, rotation, Vec3::ONE, eyes, 15.0, 15.0, from, to, uv, pivot, pose, [1.0; 3], [64., 32.], None, false);
        }
    }
    crate::cow_render::apply_overlays(mesh, &marks);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn still_legs_rest_and_walking_legs_move_in_four_phases() {
        assert!(leg_motion(3.0, 0.0).iter().all(|&(swing, step)| swing == 0.0 && step == 0.0));
        let walking = leg_motion(1.0, 1.0);
        assert!(walking.iter().all(|&(_, step)| step >= 0.0));
        assert_ne!(walking[0], walking[1]);
    }
}
