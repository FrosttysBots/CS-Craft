//! Pack-backed pinned 26.3 enderman: `EndermanModel.createBodyLayer` (the
//! humanoid mesh raised by 14, with a hat inside the head and arms and legs
//! 30 long), its `setupAnim` over `HumanoidModel`'s (the head's turn and
//! pitch, the limbs' walk swing and the arms' bob, halved and held within
//! 0.4; arms out for a carried block; a creepy enderman's head lifted off
//! its jaw), `EnderEyesLayer` (drawn at full brightness where vanilla adds
//! it unlit), `CarriedBlockLayer`'s pose for the block in its hands, and
//! `EndermanRenderer.getRenderOffset`'s shaking while creepy.
use crate::{
    cow_render::cube_tinted_pose_mirror,
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
    client_mobs::ClientMobs,
};
use glam::{DVec3, EulerRot, Mat4, Quat, Vec3};
use minecraftoss_entities::world::EndermanEntity;
use std::f32::consts::PI;

/// A cuboid: corners, texture offset, pivot, mirrored, the size its UVs
/// come from when inflated, and its pose (0 body, 1 head, 2 right arm,
/// 3 left arm, 4 right leg, 5 left leg, 6 the hat inside the head).
type Part = ([f32; 3], [f32; 3], [f32; 2], [f32; 3], bool, Option<[f32; 3]>, u8);

const PARTS: [Part; 7] = [
    ([-4., -8., -4.], [4., 0., 4.], [0., 0.], [0., -13., 0.], false, None, 1),
    // `CubeDeformation(-0.5)`.
    ([-3.5, -7.5, -3.5], [3.5, -0.5, 3.5], [0., 16.], [0., -13., 0.], false, Some([8., 8., 8.]), 6),
    ([-4., 0., -2.], [4., 12., 2.], [32., 16.], [0., -14., 0.], false, None, 0),
    ([-1., -2., -1.], [1., 28., 1.], [56., 0.], [-5., -12., 0.], false, None, 2),
    ([-1., -2., -1.], [1., 28., 1.], [56., 0.], [5., -12., 0.], true, None, 3),
    ([-1., 0., -1.], [1., 30., 1.], [56., 0.], [-2., -5., 0.], false, None, 4),
    ([-1., 0., -1.], [1., 30., 1.], [56., 0.], [2., -5., 0.], true, None, 5),
];

/// An enderman's pose: the body's twist, the arms' pivots (x, z in
/// pixels, which an attack moves) and the right arm, left arm, right leg
/// and left leg rotations (x, y, z).
pub struct EndermanLimbs {
    pub body_yaw: f32,
    pub arm_pivots: [(f32, f32); 2],
    pub limbs: [Vec3; 4],
}

/// `HumanoidModel.setupAnim` for an enderman: the walk swing (legs with
/// their touch of yaw and roll), the `EMPTY` arm poses, `setupAttackAnimation`
/// (a right-handed `WHACK`: the body twists, the arms' pivots follow, the
/// right arm strikes), the arm bob, then `EndermanModel`'s halving and
/// ±0.4 clamp, or the arms held out for a block.
pub fn limb_angles(walk_position: f32, walk_speed: f32, age: f32, carrying: bool, swing: Option<f32>, head_pitch: f32) -> EndermanLimbs {
    use crate::client_mobs::mth_cos;
    let sin = |x: f32| minecraftoss_player::mth::sin(f64::from(x));
    let p = walk_position * 0.6662;
    let mut right_arm = Vec3::new(mth_cos(p + PI) * 2.0 * walk_speed * 0.5, 0.0, 0.0);
    let mut left_arm = Vec3::new(mth_cos(p) * 2.0 * walk_speed * 0.5, 0.0, 0.0);
    let mut right_leg = Vec3::new(mth_cos(p) * 1.4 * walk_speed, 0.005, 0.005);
    let mut left_leg = Vec3::new(mth_cos(p + PI) * 1.4 * walk_speed, -0.005, -0.005);
    let (mut body_yaw, mut arm_pivots) = (0.0, [(-5.0, 0.0), (5.0, 0.0)]);
    if let Some(s) = swing.filter(|&s| s > 0.0) {
        body_yaw = sin(s.sqrt() * (PI * 2.0)) * 0.2;
        let (bs, bc) = (sin(body_yaw), mth_cos(body_yaw));
        arm_pivots = [(-bc * 5.0, bs * 5.0), (bc * 5.0, -bs * 5.0)];
        right_arm.y += body_yaw;
        left_arm.y += body_yaw;
        left_arm.x += body_yaw;
        // `Ease.outQuart`.
        let eased = 1.0 - ((1.0 - s) * (1.0 - s)) * ((1.0 - s) * (1.0 - s));
        let aa = sin(eased * PI);
        let bb = sin(s * PI) * -(head_pitch - 0.7) * 0.75;
        right_arm.x -= aa * 1.2 + bb;
        right_arm.y += body_yaw * 2.0;
        right_arm.z += sin(s * PI) * -0.4;
    }
    let bob_z = mth_cos(age * 0.09) * 0.05 + 0.05;
    let bob_x = sin(age * 0.067) * 0.05;
    right_arm.z += bob_z;
    right_arm.x += bob_x;
    left_arm.z -= bob_z;
    left_arm.x -= bob_x;
    for limb in [&mut right_arm, &mut left_arm, &mut right_leg, &mut left_leg] {
        limb.x = (limb.x * 0.5).clamp(-0.4, 0.4);
    }
    if carrying {
        right_arm.x = -0.5;
        left_arm.x = -0.5;
        right_arm.z = 0.05;
        left_arm.z = -0.05;
    }
    EndermanLimbs { body_yaw, arm_pivots, limbs: [right_arm, left_arm, right_leg, left_leg] }
}

/// `CarriedBlockLayer`'s pose for a block model in `[0, 1]³`, in the
/// model's block units (y down from the model's origin, 1.5 above the
/// feet).
fn carried_block_pose() -> Mat4 {
    Mat4::from_translation(Vec3::new(0.0, 0.6875, -0.75))
        * Mat4::from_rotation_x(20f32.to_radians())
        * Mat4::from_rotation_y(45f32.to_radians())
        * Mat4::from_translation(Vec3::new(0.25, 0.1875, 0.25))
        * Mat4::from_scale(Vec3::new(-0.5, -0.5, 0.5))
        * Mat4::from_rotation_y(90f32.to_radians())
}

/// Appends the endermen and returns each carried block's model pose (world
/// space, for a block model in `[0, 1]³`), where it is lit, and its block.
pub fn append_endermen<'a>(
    mesh: &mut ChunkMesh,
    endermen: impl IntoIterator<Item = &'a EndermanEntity>,
    poses: &ClientMobs,
    atlas: &Atlas,
    light: &SkyLight,
    partial: f32,
    frame: u64,
) -> Vec<(Mat4, Vec3, String)> {
    let skin = atlas.entity_region(&ResourceId::parse("minecraft:entity/enderman/enderman").unwrap());
    let eyes_id = ResourceId::parse("minecraft:entity/enderman/enderman_eyes").unwrap();
    let eyes = atlas.contains(&eyes_id).then(|| atlas.entity_region(&eyes_id));
    let partial = partial.clamp(0.0, 1.0);
    let mut carried = Vec::new();
    // Each mob's first vertex and overlay (`getOverlayCoords`).
    let mut marks = Vec::new();
    for entity in endermen {
        let Some(mob) = poses.pose(entity.id, partial) else { continue };
        marks.push((mesh.vertices.len(), mob.overlay(0.0)));
        let mut feet = mob.feet;
        let creepy = entity.creepy();
        if creepy {
            // `getRenderOffset`: a gaussian shake of 0.02 a side.
            let (dx, dz) = shake(entity.id, frame);
            feet += DVec3::new(dx * 0.02, 0.0, dz * 0.02);
        }
        let eye = mob.light_probe;
        let sample = (eye.x.floor() as i32, eye.y.floor() as i32, eye.z.floor() as i32);
        let (sky, block) = (light.get(sample) as f32, light.get_block(sample) as f32);
        let rotation = mob.body_rotation(90.0);
        let head = Quat::from_euler(EulerRot::ZYX, 0.0, mob.head_yaw.to_radians(), mob.head_pitch.to_radians());
        // `ageInTicks`: the client entity's own tick count.
        let age = mob.age_in_ticks;
        let head_pitch = mob.head_pitch * (PI / 180.0);
        let pose_limbs = limb_angles(mob.walk_position, mob.walk_speed, age, entity.carried().is_some(), mob.swing, head_pitch);
        // Creepy: the head rises five pixels, the hat (its jaw) stays.
        let head_pivot = Vec3::new(0.0, if creepy { -18.0 } else { -13.0 }, 0.0);
        let hat_pivot = head_pivot + head * Vec3::new(0.0, if creepy { 5.0 } else { 0.0 }, 0.0);
        let pose_of = |pose: u8| -> (Quat, Option<Vec3>) {
            match pose {
                1 => (head, Some(head_pivot)),
                6 => (head, Some(hat_pivot)),
                // The arms' pivots follow an attack's twist.
                2 | 3 => {
                    let r = pose_limbs.limbs[usize::from(pose - 2)];
                    let (x, z) = pose_limbs.arm_pivots[usize::from(pose - 2)];
                    (Quat::from_euler(EulerRot::ZYX, r.z, r.y, r.x), Some(Vec3::new(x, -12.0, z)))
                }
                4 | 5 => {
                    let r = pose_limbs.limbs[usize::from(pose - 2)];
                    (Quat::from_euler(EulerRot::ZYX, r.z, r.y, r.x), None)
                }
                0 => (Quat::from_rotation_y(pose_limbs.body_yaw), None),
                _ => (Quat::IDENTITY, None),
            }
        };
        let mut draw = |region: [f32; 4], sky: f32, block: f32| {
            for (from, to, uv, pivot, mirror, uv_size, pose) in PARTS {
                let (part, moved) = pose_of(pose);
                let pivot = moved.map_or(pivot, |p| p.to_array());
                cube_tinted_pose_mirror(mesh, feet, rotation, 1.0, region, sky, block, from, to, uv, pivot, part, [1.0; 3], [64., 32.], uv_size, mirror);
            }
        };
        draw(skin, sky, block);
        if let Some(eyes) = eyes {
            draw(eyes, 15.0, 15.0);
        }
        if let Some(held) = entity.carried() {
            // `LivingEntityRenderer`'s model space: `scale(-1, -1, 1)` and
            // the 1.501 lift, turned with the body.
            let flip = Mat4::from_translation(Vec3::new(0.0, 1.501, 0.0)) * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
            let world = Mat4::from_translation(feet.as_vec3()) * Mat4::from_quat(rotation) * flip * carried_block_pose();
            carried.push((world, eye.as_vec3(), held.id.clone()));
        }
    }
    crate::cow_render::apply_overlays(mesh, &marks);
    carried
}

/// Two standard-normal-ish offsets for a creepy enderman's shake, fresh
/// each frame (vanilla draws them from the renderer's own random).
fn shake(id: u64, frame: u64) -> (f64, f64) {
    let mut state = id.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ frame.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let mut uniform = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    // Box–Muller.
    let (u, v) = (uniform().max(1e-12), uniform());
    let r = (-2.0 * u.ln()).sqrt();
    (r * (std::f64::consts::TAU * v).cos(), r * (std::f64::consts::TAU * v).sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limbs_swing_within_a_small_arc_and_hold_out_a_block() {
        for step in 0..40 {
            for swing in [None, Some(step as f32 / 40.0)] {
                for limb in limb_angles(step as f32 * 0.37, 1.0, step as f32, false, swing, 0.0).limbs {
                    assert!(limb.x.abs() <= 0.4);
                }
            }
        }
        let held = limb_angles(1.0, 1.0, 5.0, true, None, 0.0).limbs;
        assert_eq!((held[0].x, held[0].z, held[1].x, held[1].z), (-0.5, 0.05, -0.5, -0.05));
    }
}
