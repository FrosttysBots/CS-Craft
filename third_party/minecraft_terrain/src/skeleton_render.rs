//! Pack-backed pinned 26.3 skeleton body layers.
//! Source: SkeletonModel.createBodyLayer/createDefaultSkeletonMesh and
//! createSingleModelDualBodyLayer (the parched), BoggedModel's mushrooms,
//! HumanoidModel.createMesh/poseRightArm, SkeletonClothingLayer (the stray's
//! and bogged's outer layers, `LayerDefinitions` inflating the humanoid mesh
//! by 0.25 and 0.2), and each renderer's textures.
use crate::{
    client_mobs::ClientMobs,
    cow_render::cube_tinted_pose_mirror,
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
};
use glam::{EulerRot, Quat, Vec3};
use minecraftoss_entities::{skeleton::SkeletonKind, world::SkeletonEntity};

/// A cuboid: corners, texture offset, pivot, mirrored, the size its UVs
/// come from when inflated, and which pose it takes (0 body, 1 head,
/// 2 right arm, 3 left arm, 4 legs).
type Part = ([f32; 3], [f32; 3], [f32; 2], [f32; 3], bool, Option<[f32; 3]>, u8);

/// `createDefaultSkeletonMesh` over `HumanoidModel.createMesh`.
const SKELETON: [Part; 7] = [
    ([-4., -8., -4.], [4., 0., 4.], [0., 0.], [0., 0., 0.], false, None, 1),
    ([-4.5, -8.5, -4.5], [4.5, 0.5, 4.5], [32., 0.], [0., 0., 0.], false, Some([8., 8., 8.]), 1),
    ([-4., 0., -2.], [4., 12., 2.], [16., 16.], [0., 0., 0.], false, None, 0),
    ([-1., -2., -1.], [1., 10., 1.], [40., 16.], [-5., 2., 0.], false, None, 2),
    ([-1., -2., -1.], [1., 10., 1.], [40., 16.], [5., 2., 0.], true, None, 3),
    ([-1., 0., -1.], [1., 12., 1.], [0., 16.], [-2., 12., 0.], false, None, 4),
    ([-1., 0., -1.], [1., 12., 1.], [0., 16.], [2., 12., 0.], true, None, 4),
];

/// `createSingleModelDualBodyLayer`: each part with its outer box, on one
/// 64×64 sheet.
const PARCHED: [Part; 13] = [
    ([-4., 0., -2.], [4., 12., 2.], [16., 16.], [0., 0., 0.], false, None, 0),
    ([-4., 10., -2.], [4., 11., 2.], [28., 0.], [0., 0., 0.], false, None, 0),
    ([-4.025, -0.025, -2.025], [4.025, 12.025, 2.025], [16., 48.], [0., 0., 0.], false, Some([8., 12., 4.]), 0),
    ([-4., -8., -4.], [4., 0., 4.], [0., 0.], [0., 0., 0.], false, None, 1),
    ([-4.2, -8.2, -4.2], [4.2, 0.2, 4.2], [0., 32.], [0., 0., 0.], false, Some([8., 8., 8.]), 1),
    ([-1., -2., -1.], [1., 10., 1.], [40., 16.], [-5.5, 2., 0.], false, None, 2),
    ([-1.55, -2.025, -1.5], [1.45, 9.975, 1.5], [42., 33.], [-5.5, 2., 0.], false, None, 2),
    ([-1., -2., -1.], [1., 10., 1.], [56., 16.], [5.5, 2., 0.], false, None, 3),
    ([-1.45, -2.025, -1.5], [1.55, 9.975, 1.5], [40., 48.], [5.5, 2., 0.], false, None, 3),
    ([-1., 0., -1.], [1., 12., 1.], [0., 16.], [-2., 12., 0.], false, None, 4),
    ([-1.5, 0., -1.5], [1.5, 12., 1.5], [0., 49.], [-2., 12., 0.], false, None, 4),
    ([-1., 0., -1.], [1., 12., 1.], [0., 16.], [2., 12., 0.], false, None, 4),
    ([-1.5, 0., -1.5], [1.5, 12., 1.5], [4., 49.], [2., 12., 0.], false, None, 4),
];

/// `HumanoidModel.createMesh` inflated by `grow`: the clothing layer's
/// parts (the right leg only; the left one follows mirrored).
fn clothing(grow: f32) -> [Part; 7] {
    let box_ = |from: [f32; 3], size: [f32; 3], extra: f32| -> ([f32; 3], [f32; 3]) {
        let g = grow + extra;
        ([from[0] - g, from[1] - g, from[2] - g], [from[0] + size[0] + g, from[1] + size[1] + g, from[2] + size[2] + g])
    };
    let (head_from, head_to) = box_([-4., -8., -4.], [8., 8., 8.], 0.0);
    let (hat_from, hat_to) = box_([-4., -8., -4.], [8., 8., 8.], 0.5);
    let (body_from, body_to) = box_([-4., 0., -2.], [8., 12., 4.], 0.0);
    let (right_arm_from, right_arm_to) = box_([-3., -2., -2.], [4., 12., 4.], 0.0);
    let (left_arm_from, left_arm_to) = box_([-1., -2., -2.], [4., 12., 4.], 0.0);
    let (leg_from, leg_to) = box_([-2., 0., -2.], [4., 12., 4.], 0.0);
    [
        (head_from, head_to, [0., 0.], [0., 0., 0.], false, Some([8., 8., 8.]), 1),
        (hat_from, hat_to, [32., 0.], [0., 0., 0.], false, Some([8., 8., 8.]), 1),
        (body_from, body_to, [16., 16.], [0., 0., 0.], false, Some([8., 12., 4.]), 0),
        (right_arm_from, right_arm_to, [40., 16.], [-5., 2., 0.], false, Some([4., 12., 4.]), 2),
        (left_arm_from, left_arm_to, [40., 16.], [5., 2., 0.], true, Some([4., 12., 4.]), 3),
        (leg_from, leg_to, [0., 16.], [-1.9, 12., 0.], false, Some([4., 12., 4.]), 4),
        (leg_from, leg_to, [0., 16.], [1.9, 12., 0.], true, Some([4., 12., 4.]), 4),
    ]
}

/// `BoggedModel`'s mushrooms, children of the head: box, texture offset,
/// offset and rotation (x, y, z).
const MUSHROOMS: [([f32; 3], [f32; 3], [f32; 2], [f32; 3], [f32; 3]); 6] = [
    ([-3., -3., 0.], [3., 1., 0.], [50., 16.], [3., -8., 3.], [0., std::f32::consts::FRAC_PI_4, 0.]),
    ([-3., -3., 0.], [3., 1., 0.], [50., 16.], [3., -8., 3.], [0., 3.0 * std::f32::consts::FRAC_PI_4, 0.]),
    ([-3., -3., 0.], [3., 1., 0.], [50., 22.], [-3., -8., -3.], [0., std::f32::consts::FRAC_PI_4, 0.]),
    ([-3., -3., 0.], [3., 1., 0.], [50., 22.], [-3., -8., -3.], [0., 3.0 * std::f32::consts::FRAC_PI_4, 0.]),
    ([-3., -4., 0.], [3., 0., 0.], [50., 28.], [-2., -1., 4.], [-std::f32::consts::FRAC_PI_2, 0., std::f32::consts::FRAC_PI_4]),
    ([-3., -4., 0.], [3., 0., 0.], [50., 28.], [-2., -1., 4.], [-std::f32::consts::FRAC_PI_2, 0., 3.0 * std::f32::consts::FRAC_PI_4]),
];

fn texture(path: &str) -> ResourceId {
    ResourceId::parse(&format!("minecraft:entity/skeleton/{path}")).unwrap()
}

/// The right arm, left arm, right leg and left leg (x, y, z rotations):
/// `HumanoidModel.setupAnim` with a skeleton's arm poses (`BOW_AND_ARROW`
/// while aggressive with a bow, else `EMPTY`), the arm bob, then
/// `SkeletonModel.setupAnim`'s raised arms for an aggressive skeleton
/// without a bow (its attack swing not yet tracked).
pub fn limb_rotations(walk_position: f32, walk_speed: f32, age: f32, head_yaw: f32, head_pitch: f32, aggressive: bool, holding_bow: bool) -> [Vec3; 4] {
    use crate::client_mobs::mth_cos;
    let pi = std::f32::consts::PI;
    let swing = walk_position * 0.6662;
    let mut right_arm = Vec3::new(mth_cos(swing + pi) * 2.0 * walk_speed * 0.5, 0.0, 0.0);
    let mut left_arm = Vec3::new(mth_cos(swing) * 2.0 * walk_speed * 0.5, 0.0, 0.0);
    let right_leg = Vec3::new(mth_cos(swing) * 1.4 * walk_speed, 0.005, 0.005);
    let left_leg = Vec3::new(mth_cos(swing + pi) * 1.4 * walk_speed, -0.005, -0.005);
    if aggressive && holding_bow {
        right_arm.y = -0.1 + head_yaw;
        left_arm.y = 0.1 + head_yaw + 0.4;
        right_arm.x = -pi / 2.0 + head_pitch;
        left_arm.x = -pi / 2.0 + head_pitch;
    }
    let bob = |arm: &mut Vec3, scale: f32| {
        arm.z += scale * (mth_cos(age * 0.09) * 0.05 + 0.05);
        arm.x += scale * (minecraftoss_player::mth::sin(f64::from(age * 0.067)) * 0.05);
    };
    bob(&mut right_arm, 1.0);
    bob(&mut left_arm, -1.0);
    if aggressive && !holding_bow {
        right_arm = Vec3::new(-pi / 2.0, -0.1, 0.0);
        left_arm = Vec3::new(-pi / 2.0, 0.1, 0.0);
        // `bobArms` bobs each arm twice.
        for _ in 0..2 {
            bob(&mut right_arm, 1.0);
            bob(&mut left_arm, -1.0);
        }
    }
    [right_arm, left_arm, right_leg, left_leg]
}

/// Appends the skeletons and returns their held bows: each hand's pose,
/// where it is lit, and the item.
pub fn append_skeletons<'a>(
    mesh: &mut ChunkMesh,
    skeletons: impl IntoIterator<Item = &'a SkeletonEntity>,
    poses: &ClientMobs,
    atlas: &Atlas,
    light: &SkyLight,
    partial: f32,
) -> Vec<crate::mesh::HeldItem> {
    let mut held = Vec::new();
    // Each mob's first vertex and overlay (`getOverlayCoords`).
    let mut marks = Vec::new();
    for entity in skeletons {
        let kind = entity.skeleton.kind;
        let Some(mob) = poses.pose(entity.id, partial) else { continue };
        marks.push((mesh.vertices.len(), mob.overlay(0.0)));
        let feet = mob.feet;
        let sample = mob.light_block();
        let sky = light.get(sample) as f32;
        let block = light.get_block(sample) as f32;
        let rotation = mob.body_rotation(90.0);
        // `state.yRot * (PI / 180)` as a float.
        let head_yaw = mob.head_yaw * (std::f32::consts::PI / 180.0);
        let head_pitch = mob.head_pitch * (std::f32::consts::PI / 180.0);
        let head = Quat::from_euler(EulerRot::ZYX, 0.0, head_yaw, head_pitch);
        let holding_bow = entity.skeleton.holds_bow;
        let limbs = limb_rotations(mob.walk_position, mob.walk_speed, mob.age_in_ticks, head_yaw, head_pitch, entity.bow.aggressive, holding_bow);
        let part = |r: Vec3| Quat::from_euler(EulerRot::ZYX, r.z, r.y, r.x);
        // Legs share a pose code; the pivot tells right from left.
        let pose_of = |pose: u8, pivot_x: f32| match pose {
            1 => head,
            2 => part(limbs[0]),
            3 => part(limbs[1]),
            4 if pivot_x < 0.0 => part(limbs[2]),
            4 => part(limbs[3]),
            _ => Quat::IDENTITY,
        };
        let mut draw = |parts: &[Part], id: &ResourceId, size: [f32; 2]| {
            let region = atlas.entity_region(id);
            for (from, to, uv, pivot, mirror, uv_size, pose) in parts {
                cube_tinted_pose_mirror(mesh, feet, rotation, 1.0, region, sky, block, *from, *to, *uv, *pivot, pose_of(*pose, pivot[0]), [1.0; 3], size, *uv_size, *mirror);
            }
        };
        // `SkeletonModel.translateToHand`: the arm a pixel further out.
        if holding_bow {
            let pivot = if kind == SkeletonKind::Parched { -5.5 } else { -5.0 } + 1.0;
            let hand = crate::cow_render::right_hand_pose(feet, rotation, Vec3::new(pivot, 2.0, 0.0), part(limbs[0]), false);
            held.push(crate::mesh::HeldItem { pose: hand, light: mob.light_probe.as_vec3(), id: "minecraft:bow".to_owned(), display: crate::mesh::HeldDisplay::RightHand, first_tint: None });
        }
        match kind {
            SkeletonKind::Parched => draw(&PARCHED, &texture("parched"), [64., 64.]),
            SkeletonKind::Skeleton => draw(&SKELETON, &texture("skeleton"), [64., 32.]),
            SkeletonKind::Stray => {
                draw(&SKELETON, &texture("stray"), [64., 32.]);
                draw(&clothing(0.25), &texture("stray_overlay"), [64., 32.]);
            }
            SkeletonKind::Bogged => {
                draw(&SKELETON, &texture("bogged"), [64., 32.]);
                draw(&clothing(0.2), &texture("bogged_overlay"), [64., 32.]);
            }
        }
        // `HumanoidArmorLayer` on the humanoid armour meshes (legs at
        // ±1.9; armour takes no hurt overlay).
        if entity.skeleton.armor.iter().any(Option::is_some) {
            marks.push((mesh.vertices.len(), 1.0));
            let arm_x = if kind == SkeletonKind::Parched { 5.5 } else { 5.0 };
            let armor_parts = crate::armor_render::HumanoidParts {
                head: (Vec3::ZERO, head),
                body: (Vec3::ZERO, Quat::IDENTITY),
                arms: [(Vec3::new(-arm_x, 2.0, 0.0), part(limbs[0])), (Vec3::new(arm_x, 2.0, 0.0), part(limbs[1]))],
                legs: [(Vec3::new(-1.9, 12.0, 0.0), part(limbs[2])), (Vec3::new(1.9, 12.0, 0.0), part(limbs[3]))],
            };
            crate::armor_render::append_humanoid_armor(mesh, atlas, feet, rotation, 1.0, &armor_parts, &entity.skeleton.armor, sky, block);
            marks.push((mesh.vertices.len(), mob.overlay(0.0)));
        }
        // `BoggedModel.setupAnim`: the mushrooms show until sheared.
        if kind == SkeletonKind::Bogged && !entity.skeleton.sheared {
            let region = atlas.entity_region(&texture("bogged"));
            for (from, to, uv, offset, [x, y, z]) in MUSHROOMS {
                let pivot = head * Vec3::from_array(offset);
                let pose = head * Quat::from_euler(EulerRot::ZYX, z, y, x);
                cube_tinted_pose_mirror(mesh, feet, rotation, 1.0, region, sky, block, from, to, uv, pivot.to_array(), pose, [1.0; 3], [64., 32.], None, false);
            }
        }
    }
    crate::cow_render::apply_overlays(mesh, &marks);
    held
}
