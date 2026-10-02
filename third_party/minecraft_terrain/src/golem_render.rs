//! Pack-backed pinned 26.3 iron golem: `IronGolemModel.createBodyLayer`
//! (head with its nose, the chest and waist, long arms, legs; 128 by
//! 128), `IronGolemModel.setupAnim` (the arms swing up for ten ticks after
//! a hit, one holds out a poppy while it offers one, otherwise arms and
//! legs swing with the walk as triangle waves; the head turns and pitches),
//! `IronGolemRenderer.setupRotations` (the body rocks side to side with the
//! walk), `IronGolemCrackinessLayer` (the crack overlay for its health) and
//! `IronGolemFlowerLayer` (the poppy block in the right hand), from
//! `entity/iron_golem/iron_golem`.
use crate::{
    cow_render::cube_tinted_pose_mirror,
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
    client_mobs::ClientMobs,
};
use glam::{EulerRot, Mat4, Quat, Vec3};
use minecraftoss_entities::world::IronGolemEntity;
use std::f32::consts::PI;

/// A cuboid: corners, texture offset, mirrored, the size its UVs come from
/// when inflated, and the part it belongs to.
type Cube = ([f32; 3], [f32; 3], [f32; 2], bool, Option<[f32; 3]>, Part);

#[derive(Clone, Copy, PartialEq)]
enum Part {
    Head,
    Body,
    RightArm,
    LeftArm,
    RightLeg,
    LeftLeg,
}

const CUBES: [Cube; 8] = [
    ([-4., -12., -5.5], [4., -2., 2.5], [0., 0.], false, None, Part::Head),
    ([-1., -5., -7.5], [1., -1., -5.5], [24., 0.], false, None, Part::Head),
    ([-9., -2., -6.], [9., 10., 5.], [0., 40.], false, None, Part::Body),
    // `CubeDeformation(0.5F)`.
    ([-5., 9.5, -3.5], [5., 15.5, 3.5], [0., 70.], false, Some([9., 5., 6.]), Part::Body),
    ([-13., -2.5, -3.], [-9., 27.5, 3.], [60., 21.], false, None, Part::RightArm),
    ([9., -2.5, -3.], [13., 27.5, 3.], [60., 58.], false, None, Part::LeftArm),
    ([-3.5, -3., -3.], [2.5, 13., 2.], [37., 0.], false, None, Part::RightLeg),
    ([-3.5, -3., -3.], [2.5, 13., 2.], [60., 0.], true, None, Part::LeftLeg),
];

/// `Mth.triangleWave`: -1 to 1 and back over `period`.
fn triangle_wave(index: f32, period: f32) -> f32 {
    ((index % period - period * 0.5).abs() - period * 0.25) / (period * 0.25)
}

/// `IronGolemModel.setupAnim`'s arm pitches: the swing after a hit (its
/// ticks left, less the partial tick), the poppy held out, or the walk.
pub fn arm_pitches(attack_ticks: f32, offer_flower_tick: i32, walk_position: f32, walk_speed: f32) -> (f32, f32) {
    if attack_ticks > 0.0 {
        let arm = -2.0 + 1.5 * triangle_wave(attack_ticks, 10.0);
        (arm, arm)
    } else if offer_flower_tick > 0 {
        (-0.8 + 0.025 * triangle_wave(offer_flower_tick as f32, 70.0), 0.0)
    } else {
        let wave = triangle_wave(walk_position, 13.0);
        ((-0.2 + 1.5 * wave) * walk_speed, (-0.2 - 1.5 * wave) * walk_speed)
    }
}

/// `IronGolemRenderer.setupRotations`: the body's rock about its facing, in
/// degrees, while it walks.
pub fn body_rock(walk_position: f32, walk_speed: f32) -> f32 {
    if walk_speed < 0.01 {
        return 0.0;
    }
    let position = walk_position + 6.0;
    6.5 * (((position % 13.0) - 6.5).abs() - 3.25) / 3.25
}

/// A part's pose: its offset (pixels) and `rotationZYX` turn.
fn pose(offset: [f32; 3], x: f32, y: f32, z: f32) -> ([f32; 3], Quat) {
    (offset, Quat::from_euler(EulerRot::ZYX, z, y, x))
}

/// Appends the living iron golems and returns the poppies held out, as
/// posed block models (world space, for a block model in `[0, 1]³`), where
/// each is lit, and the block.
pub fn append_iron_golems<'a>(mesh: &mut ChunkMesh, golems: impl IntoIterator<Item = &'a IronGolemEntity>, poses: &ClientMobs, atlas: &Atlas, light: &SkyLight, partial: f32) -> Vec<(Mat4, Vec3, String)> {
    let mut poppies = Vec::new();
    let Ok(skin_id) = ResourceId::parse("minecraft:entity/iron_golem/iron_golem") else { return poppies };
    if !atlas.contains(&skin_id) {
        return poppies;
    }
    let skin = atlas.entity_region(&skin_id);
    let partial = partial.clamp(0.0, 1.0);
    // Each mob's first vertex and overlay (`getOverlayCoords`).
    let mut marks = Vec::new();
    for entity in golems {
        let Some(mob) = poses.pose(entity.id, partial) else { continue };
        marks.push((mesh.vertices.len(), mob.overlay(0.0)));
        let feet = mob.feet;
        let eye = mob.light_probe;
        let sample = (eye.x.floor() as i32, eye.y.floor() as i32, eye.z.floor() as i32);
        let (sky, block) = (light.get(sample) as f32, light.get_block(sample) as f32);
        let (walk_position, walk_speed) = (mob.walk_position, mob.walk_speed);
        let rotation = mob.body_rotation(90.0) * Quat::from_rotation_z(body_rock(walk_position, walk_speed).to_radians());
        let attack = entity.golem.attack_animation_tick;
        let attack_ticks = if attack > 0 { attack as f32 - partial } else { 0.0 };
        let offer = entity.offer_flower_tick();
        let (right_arm_x, left_arm_x) = arm_pitches(attack_ticks, offer, walk_position, walk_speed);
        let leg = 1.5 * triangle_wave(walk_position, 13.0) * walk_speed;
        let head = pose([0.0, -7.0, -2.0], mob.head_pitch.to_radians(), mob.head_yaw.to_radians(), 0.0);
        let body = pose([0.0, -7.0, 0.0], 0.0, 0.0, 0.0);
        let right_arm = pose([0.0, -7.0, 0.0], right_arm_x, 0.0, 0.0);
        let left_arm = pose([0.0, -7.0, 0.0], left_arm_x, 0.0, 0.0);
        let right_leg = pose([-4.0, 11.0, 0.0], -leg, 0.0, 0.0);
        let left_leg = pose([5.0, 11.0, 0.0], leg, 0.0, 0.0);
        // The skin, then the cracks for its health (`renderColoredCutoutModel`).
        let mut regions = vec![skin];
        if let Some(cracks) = entity.golem.crackiness().texture() {
            if let Ok(id) = ResourceId::parse(cracks) {
                if atlas.contains(&id) {
                    regions.push(atlas.entity_region(&id));
                }
            }
        }
        for region in regions {
            for (from, to, uv, mirror, uv_size, part) in CUBES {
                let (pivot, part_rotation) = match part {
                    Part::Head => head,
                    Part::Body => body,
                    Part::RightArm => right_arm,
                    Part::LeftArm => left_arm,
                    Part::RightLeg => right_leg,
                    Part::LeftLeg => left_leg,
                };
                cube_tinted_pose_mirror(mesh, feet, rotation, 1.0, region, sky, block, from, to, uv, pivot, part_rotation, [1.0; 3], [128., 128.], uv_size, mirror);
            }
        }
        if offer > 0 {
            // `IronGolemFlowerLayer`: the right arm's pose, then the poppy
            // half size, tipped back, at the hand.
            let (arm_offset, arm_rotation) = right_arm;
            let model = Mat4::from_translation(Vec3::new(0.0, 1.501, 0.0)) * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
            let hand = Mat4::from_translation(Vec3::from_array(arm_offset) / 16.0)
                * Mat4::from_quat(arm_rotation)
                * Mat4::from_translation(Vec3::new(-1.1875, 1.0625, -0.9375))
                * Mat4::from_translation(Vec3::splat(0.5))
                * Mat4::from_scale(Vec3::splat(0.5))
                * Mat4::from_rotation_x(-PI / 2.0)
                * Mat4::from_translation(Vec3::splat(-0.5));
            let world = Mat4::from_translation(feet.as_vec3()) * Mat4::from_quat(rotation) * model * hand;
            poppies.push((world, eye.as_vec3(), "minecraft:poppy".to_owned()));
        }
    }
    crate::cow_render::apply_overlays(mesh, &marks);
    poppies
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arms_swing_up_after_a_hit_and_hold_out_a_poppy() {
        // Ten ticks after a hit both arms are raised high.
        let (right, left) = arm_pitches(10.0, 0, 0.0, 0.0);
        assert_eq!(right, left);
        assert!((right - (-2.0 + 1.5 * triangle_wave(10.0, 10.0))).abs() < 1e-6);
        // Offering, the right arm reaches forward, the left hangs.
        let (right, left) = arm_pitches(0.0, 400, 0.0, 0.0);
        assert!(right < -0.7 && left == 0.0);
        // Standing still, the arms hang straight.
        assert_eq!(arm_pitches(0.0, 0, 3.0, 0.0), (0.0, 0.0));
        assert_eq!(body_rock(3.0, 0.0), 0.0);
        assert!(body_rock(0.0, 1.0).abs() <= 6.5);
    }
}
