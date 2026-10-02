//! Pack-backed pinned 26.3 wolf: `AdultWolfModel.createBodyLayer` (a head
//! with its real head, ears and snout, body, mane, legs and a tail on a
//! real tail; 64 by 32) and `BabyWolfModel.createBodyLayer` (32 by 32),
//! `WolfModel.setupAnim` (legs and tail swing with the walk, an angry
//! wolf's tail still; the head turns and pitches; the tail's angle from
//! health; the sitting pose), `shakeOffWater` (the body, mane, head and
//! tail roll as it shakes dry, the head tilting as it begs),
//! `WolfRenderer` (its variant's wild, angry or tame texture, darker while
//! wet) and `WolfCollarLayer` (a tame wolf's collar in its dye colour).
use crate::{
    cow_render::cube_tinted_pose_mirror,
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
    client_mobs::ClientMobs,
};
use glam::{EulerRot, Quat, Vec3};
use minecraftoss_entities::world::WolfEntity;
use std::f32::consts::PI;

/// `DyeColor.getTextureDiffuseColor`, by dye ID.
const DYE_DIFFUSE: [u32; 16] = [
    16383998, 16351261, 13061821, 3847130, 16701501, 8439583, 15961002, 4673362, 10329495, 1481884, 8991416, 3949738, 8606770, 6192150, 11546150, 1908001,
];

#[derive(Clone, Copy, PartialEq)]
enum Part {
    /// The head and what hangs from it (a baby's ears), turned with it.
    Head,
    /// The adult's real head, rolled within the head.
    RealHead,
    Body,
    UpperBody,
    RightHindLeg,
    LeftHindLeg,
    RightFrontLeg,
    LeftFrontLeg,
    /// The adult's real tail, or the baby's tail cube, within the tail.
    Tail,
}

/// A cuboid: corners, texture offset, mirrored, inflated UV size, the part,
/// and (for a baby's ears and tail cube) its own offset and turn within it.
type Cube = ([f32; 3], [f32; 3], [f32; 2], bool, Option<[f32; 3]>, Part, [f32; 3], f32);

const fn cube(from: [f32; 3], size: [f32; 3], uv: [f32; 2], part: Part) -> Cube {
    ([from[0], from[1], from[2]], [from[0] + size[0], from[1] + size[1], from[2] + size[2]], uv, false, None, part, [0.0; 3], 0.0)
}

const ADULT: [Cube; 11] = [
    cube([-2., -3., -2.], [6., 6., 4.], [0., 0.], Part::RealHead),
    cube([-2., -5., 0.], [2., 2., 1.], [16., 14.], Part::RealHead),
    cube([2., -5., 0.], [2., 2., 1.], [16., 14.], Part::RealHead),
    cube([-0.5, -0.001, -5.], [3., 3., 4.], [0., 10.], Part::RealHead),
    cube([-3., -2., -3.], [6., 9., 6.], [18., 14.], Part::Body),
    cube([-3., -3., -3.], [8., 6., 7.], [21., 0.], Part::UpperBody),
    // The right legs are mirrored.
    ([0., 0., -1.], [2., 8., 1.], [0., 18.], true, None, Part::RightHindLeg, [0.; 3], 0.),
    cube([0., 0., -1.], [2., 8., 2.], [0., 18.], Part::LeftHindLeg),
    ([0., 0., -1.], [2., 8., 1.], [0., 18.], true, None, Part::RightFrontLeg, [0.; 3], 0.),
    cube([0., 0., -1.], [2., 8., 2.], [0., 18.], Part::LeftFrontLeg),
    cube([0., 0., -1.], [2., 8., 2.], [9., 18.], Part::Tail),
];

const BABY: [Cube; 10] = [
    // `CubeDeformation(0.025F)`.
    ([-3.015, -3.275, -3.025], [3.035, 1.775, 2.025], [0., 12.], false, Some([6., 5., 5.]), Part::Head, [0.; 3], 0.),
    cube([-1.5, -0.24, -5.], [3., 2., 2.], [17., 12.], Part::Head),
    ([-1., -1., -0.5], [1., 1., 0.5], [0., 5.], false, None, Part::Head, [-2., -4.25, -0.5], 0.),
    ([-1., -1., -0.5], [1., 1., 0.5], [20., 5.], false, None, Part::Head, [2., -4.25, -0.5], 0.),
    cube([-3., -2., -4.], [6., 4., 8.], [0., 0.], Part::Body),
    cube([-1., 0., -1.], [2., 3., 2.], [0., 22.], Part::RightHindLeg),
    cube([-1., 0., -1.], [2., 3., 2.], [8., 22.], Part::LeftHindLeg),
    cube([-1., 0., -1.], [2., 3., 2.], [0., 0.], Part::RightFrontLeg),
    cube([-1., 0., -1.], [2., 3., 2.], [20., 0.], Part::LeftFrontLeg),
    ([-1., -5.7, -1.], [1., 0.3, 1.], [22., 16.], false, None, Part::Tail, [0., -0.6, 0.2], -3.1),
];

/// `WolfRenderState.getBodyRollAngle`: the roll as the shake passes `offset`.
pub fn body_roll(shake: f32, offset: f32) -> f32 {
    let progress = ((shake + offset) / 1.8).clamp(0.0, 1.0);
    (progress * PI).sin() * (progress * PI * 11.0).sin() * 0.15 * PI
}

/// A part's pose: its offset (pixels) and `rotationZYX` angles.
#[derive(Clone, Copy)]
struct Pose {
    offset: [f32; 3],
    x: f32,
    y: f32,
    z: f32,
}

impl Pose {
    const fn at(offset: [f32; 3], x: f32) -> Self {
        Self { offset, x, y: 0.0, z: 0.0 }
    }
    fn rotation(&self) -> Quat {
        Quat::from_euler(EulerRot::ZYX, self.z, self.y, self.x)
    }
}

/// `WolfModel.setupAnim` and its model's overrides: each part's pose.
#[allow(clippy::too_many_arguments)]
fn part_poses(baby: bool, walk_position: f32, walk_speed: f32, angry: bool, sitting: bool, shake: f32, head_roll: f32, head: (f32, f32), tail_angle: f32) -> [Pose; 9] {
    let mut p = if baby {
        [
            Pose::at([0., 18.25, -4.], 0.),
            Pose::at([0.; 3], 0.),
            Pose::at([0., 19., 0.], 0.),
            Pose::at([0.; 3], 0.),
            Pose::at([-1.5, 21., 3.], 0.),
            Pose::at([1.5, 21., 3.], 0.),
            Pose::at([-1.5, 21., -3.], 0.),
            Pose::at([1.5, 21., -3.], 0.),
            Pose::at([0., 19., 3.], -0.5236),
        ]
    } else {
        [
            Pose::at([-1., 13.5, -7.], 0.),
            Pose::at([0.; 3], 0.),
            Pose::at([0., 14., 2.], PI / 2.),
            Pose::at([-1., 14., -3.], PI / 2.),
            Pose::at([-2.5, 16., 7.], 0.),
            Pose::at([0.5, 16., 7.], 0.),
            Pose::at([-2.5, 16., -4.], 0.),
            Pose::at([0.5, 16., -4.], 0.),
            Pose::at([-1., 12., 8.], PI / 5.),
        ]
    };
    let [head_part, real_head, body, upper_body, right_hind, left_hind, right_front, left_front, tail] = &mut p;
    let swing = (walk_position * 0.6662).cos() * 1.4 * walk_speed;
    let counter = (walk_position * 0.6662 + PI).cos() * 1.4 * walk_speed;
    tail.y = if angry { 0.0 } else { swing };
    if sitting {
        // `setSittingPose`, its offsets scaled by `ageScale`.
        let age_scale = if baby { 0.5 } else { 1.0 };
        body.offset[1] += 4.0 * age_scale;
        body.offset[2] -= 2.0 * age_scale;
        body.x = PI / 4.0;
        tail.offset[1] += 9.0 * age_scale;
        tail.offset[2] -= 2.0 * age_scale;
        for leg in [&mut *right_hind, &mut *left_hind] {
            leg.offset[1] += 6.7 * age_scale;
            leg.offset[2] -= 5.0 * age_scale;
            leg.x = PI * 3.0 / 2.0;
        }
        right_front.x = 5.811_947;
        right_front.offset[0] += 0.01 * age_scale;
        right_front.offset[1] += age_scale;
        left_front.x = 5.811_947;
        left_front.offset[0] -= 0.01 * age_scale;
        left_front.offset[1] += age_scale;
        if baby {
            body.x -= PI / 2.0;
        } else {
            upper_body.offset[1] += 2.0;
            upper_body.x = PI * 2.0 / 5.0;
            upper_body.y = 0.0;
        }
    } else {
        right_hind.x = swing;
        left_hind.x = counter;
        right_front.x = counter;
        left_front.x = swing;
    }
    // `shakeOffWater`.
    body.z = body_roll(shake, -0.16);
    if baby {
        head_part.z = head_roll + body_roll(shake, 0.0);
        tail.z = body_roll(shake, -0.2);
    } else {
        real_head.z = head_roll + body_roll(shake, 0.0);
        upper_body.z = body_roll(shake, -0.08);
    }
    head_part.x = head.0;
    head_part.y = head.1;
    tail.x = tail_angle;
    p
}

/// Appends the living wolves.
pub fn append_wolves<'a>(mesh: &mut ChunkMesh, wolves: impl IntoIterator<Item = &'a WolfEntity>, poses: &ClientMobs, atlas: &Atlas, light: &SkyLight, partial: f32, game_time: i64) {
    let partial = partial.clamp(0.0, 1.0);
    let collar_ids = (ResourceId::parse("minecraft:entity/wolf/wolf_collar"), ResourceId::parse("minecraft:entity/wolf/wolf_collar_baby"));
    // Each mob's first vertex and overlay (`getOverlayCoords`).
    let mut marks = Vec::new();
    for entity in wolves {
        let wolf = &entity.wolf;
        let baby = wolf.baby();
        let angry = entity.angry(game_time);
        let Ok(skin_id) = ResourceId::parse(&wolf.texture(angry)) else { continue };
        if !atlas.contains(&skin_id) {
            continue;
        }
        let Some(mob) = poses.pose(entity.id, partial) else { continue };
        marks.push((mesh.vertices.len(), mob.overlay(0.0)));
        let feet = mob.feet;
        let eye = mob.light_probe;
        let sample = (eye.x.floor() as i32, eye.y.floor() as i32, eye.z.floor() as i32);
        let (sky, block) = (light.get(sample) as f32, light.get_block(sample) as f32);
        let rotation = mob.body_rotation(90.0);
        let shake = wolf.shake_anim_o + (wolf.shake_anim - wolf.shake_anim_o) * partial;
        let head_roll = (wolf.interested_angle_o + (wolf.interested_angle - wolf.interested_angle_o) * partial) * 0.15 * PI;
        // `getWetShade` darkens the whole model while it is wet.
        let wet = if wolf.wet { (0.75 + shake / 2.0 * 0.25).min(1.0) } else { 1.0 };
        let pose = part_poses(
            baby,
            mob.walk_position,
            mob.walk_speed,
            angry,
            wolf.sitting,
            shake,
            head_roll,
            (mob.head_pitch.to_radians(), mob.head_yaw.to_radians()),
            wolf.tail_angle(angry),
        );
        let (cubes, texture_size): (&[Cube], [f32; 2]) = if baby { (&BABY, [32., 32.]) } else { (&ADULT, [64., 32.]) };
        let mut layers = vec![(atlas.entity_region(&skin_id), [wet; 3])];
        // `WolfCollarLayer`: a tame wolf's collar, in its dye's colour.
        if wolf.tame {
            if let Ok(collar) = if baby { &collar_ids.1 } else { &collar_ids.0 } {
                if atlas.contains(collar) {
                    let rgb = DYE_DIFFUSE[usize::from(wolf.collar & 15)];
                    let channel = |shift: u32| ((rgb >> shift) & 255) as f32 / 255.0;
                    layers.push((atlas.entity_region(collar), [channel(16), channel(8), channel(0)]));
                }
            }
        }
        for (region, tint) in layers {
            for &(from, to, uv, mirror, uv_size, part, own_offset, own_x) in cubes {
                let parent = match part {
                    Part::Head => pose[0],
                    Part::RealHead => pose[0],
                    Part::Body => pose[2],
                    Part::UpperBody => pose[3],
                    Part::RightHindLeg => pose[4],
                    Part::LeftHindLeg => pose[5],
                    Part::RightFrontLeg => pose[6],
                    Part::LeftFrontLeg => pose[7],
                    Part::Tail => pose[8],
                };
                let parent_rotation = parent.rotation();
                // Children: the real head's roll, the adult's real tail
                // rolling as it shakes, the baby's ears and tail cube with
                // their own offsets and turns.
                let child = match part {
                    Part::RealHead => pose[1].rotation(),
                    Part::Tail if !baby => Quat::from_rotation_z(body_roll(shake, -0.2)),
                    _ => Quat::from_rotation_x(own_x),
                };
                let pivot = Vec3::from_array(parent.offset) + parent_rotation * Vec3::from_array(own_offset);
                cube_tinted_pose_mirror(mesh, feet, rotation, 1.0, region, sky, block, from, to, uv, pivot.to_array(), parent_rotation * child, tint, texture_size, uv_size, mirror);
            }
        }
    }
    crate::cow_render::apply_overlays(mesh, &marks);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shake_rolls_the_body_and_the_tail_follows_its_angle() {
        assert_eq!(body_roll(0.0, -0.16), 0.0);
        assert!(body_roll(0.9, 0.0).abs() <= 0.15 * PI + 1e-6);
        assert!(body_roll(2.0, 0.0).abs() < 1e-5);
        // Standing still, an adult's legs hang straight and its tail takes
        // the angle given.
        let p = part_poses(false, 0.0, 0.0, false, false, 0.0, 0.0, (0.0, 0.0), 0.7);
        assert_eq!(p[4].x, 0.0);
        assert_eq!(p[8].x, 0.7);
        // Sitting, the hind legs fold under it.
        let p = part_poses(false, 0.0, 0.0, false, true, 0.0, 0.0, (0.0, 0.0), 0.7);
        assert_eq!(p[4].x, PI * 3.0 / 2.0);
        assert_eq!(p[2].offset, [0., 18., 0.]);
    }
}
