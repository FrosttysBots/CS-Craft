//! Pack-backed pinned 26.3 zombie adult/baby cuboids.
//! Source: HumanoidModel.createMesh, BabyZombieModel.createBodyLayer,
//! LayerDefinitions.ZOMBIE and AbstractZombieRenderer textures; husks use
//! the zombie's model (`HuskRenderer`), zombie villagers
//! `ZombieVillagerModel`/`BabyZombieVillagerModel` with the villager type
//! and profession overlays (`VillagerProfessionLayer`).
//! The body faces its body yaw and the head its look. Walk/swing
//! animation, mirrored UVs and equipment overlays remain open.
use crate::{
    client_mobs::{ClientMobs, MobPose},
    cow_render::cube_tinted_pose_mirror,
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
};
use glam::{EulerRot, Quat};
use minecraftoss_entities::world::ZombieEntity;
use minecraftoss_entities::zombie::ZombieKind;

type Part = ([f32; 3], [f32; 3], [f32; 2], [f32; 3]);

const ADULT: [Part; 7] = [
    ([-4., -8., -4.], [4., 0., 4.], [0., 0.], [0., 0., 0.]),
    ([-4.5, -8.5, -4.5], [4.5, 0.5, 4.5], [32., 0.], [0., 0., 0.]),
    ([-4., 0., -2.], [4., 12., 2.], [16., 16.], [0., 0., 0.]),
    ([-3., -2., -2.], [1., 10., 2.], [40., 16.], [-5., 2., 0.]),
    ([-1., -2., -2.], [3., 10., 2.], [40., 16.], [5., 2., 0.]),
    ([-2., 0., -2.], [2., 12., 2.], [0., 16.], [-1.9, 12., 0.]),
    ([-2., 0., -2.], [2., 12., 2.], [0., 16.], [1.9, 12., 0.]),
];

const BABY: [Part; 7] = [
    (
        [-3., -6.25, -3.],
        [3., -0.25, 3.],
        [3., 3.],
        [0., 15.25, 0.],
    ),
    (
        [-3.25, -6.4, -3.25],
        [3.25, 0.1, 3.25],
        [35., 3.],
        [0., 15.25, 0.],
    ),
    ([-2., -2.5, -1.], [2., 2.5, 1.], [16., 16.], [0., 17.5, 0.]),
    ([-1., -0.5, -1.], [1., 4.5, 1.], [36., 16.], [-3., 15.5, 0.]),
    ([-1., -0.5, -1.], [1., 4.5, 1.], [28., 16.], [3., 15.5, 0.]),
    ([-1., 0., -1.], [1., 4., 1.], [8., 16.], [-1., 20., 0.]),
    ([-1., 0., -1.], [1., 4., 1.], [0., 16.], [1., 20., 0.]),
];

pub fn append_zombies<'a>(
    mesh: &mut ChunkMesh,
    zombies: impl IntoIterator<Item = &'a ZombieEntity>,
    poses: &ClientMobs,
    atlas: &Atlas,
    light: &SkyLight,
    partial: f32,
) -> Vec<crate::mesh::HeldItem> {
    let mut held = Vec::new();
    // Each mob's first vertex and overlay (`getOverlayCoords`).
    let mut marks = Vec::new();
    for entity in zombies {
        let zombie = &entity.zombie;
        let Some(pose) = poses.pose(entity.id, partial) else { continue };
        marks.push((mesh.vertices.len(), pose.overlay(0.0)));
        let feet = pose.feet;
        if zombie.kind == ZombieKind::ZombieVillager {
            append_zombie_villager(mesh, entity, &pose, atlas, light);
            continue;
        }
        let id = ResourceId::parse(match (zombie.kind, zombie.baby) {
            (ZombieKind::Zombie, false) => "minecraft:entity/zombie/zombie",
            (ZombieKind::Zombie, true) => "minecraft:entity/zombie/zombie_baby",
            (ZombieKind::Drowned, false) => "minecraft:entity/zombie/drowned",
            (ZombieKind::Drowned, true) => "minecraft:entity/zombie/drowned_baby",
            (ZombieKind::Husk, false) => "minecraft:entity/zombie/husk",
            (ZombieKind::Husk, true) => "minecraft:entity/zombie/husk_baby",
            (ZombieKind::ZombieVillager, _) => unreachable!("drawn by append_zombie_villager"),
        })
        .unwrap();
        let region = atlas.entity_region(&id);
        let sample = pose.light_block();
        let sky = light.get(sample) as f32;
        let block = light.get_block(sample) as f32;
        let rotation = pose.body_rotation(90.0);
        let head = Quat::from_euler(EulerRot::ZYX, 0.0, pose.head_yaw.to_radians(), pose.head_pitch.to_radians());
        let parts = if zombie.baby { &BABY } else { &ADULT };
        let rest = [(parts[3].3[0], parts[3].3[2]), (parts[4].3[0], parts[4].3[2])];
        let pose_limbs = limb_rotations(pose.walk_position, pose.walk_speed, pose.age_in_ticks, entity.aggressive, pose.swing, if zombie.baby { 0.5 } else { 1.0 }, rest);
        let limbs = pose_limbs.limbs;
        let arm_pivot = |arm: usize| {
            let (x, z) = pose_limbs.arm_pivots[arm];
            [x, parts[3 + arm].3[1], z]
        };
        // `ItemInHandLayer`: the main hand's item in the right hand.
        if let Some(item) = &zombie.main_hand {
            let pivot = glam::Vec3::from_array(arm_pivot(0));
            held.push(crate::mesh::HeldItem { pose: crate::cow_render::right_hand_pose(feet, pose.body_rotation(90.0), pivot, limbs[0], zombie.baby), light: pose.light_probe.as_vec3(), id: item.clone(), display: crate::mesh::HeldDisplay::RightHand, first_tint: None });
        }
        // `LayerDefinitions`: the adult husk is scaled up by 1.0625.
        let scale = if zombie.kind == ZombieKind::Husk && !zombie.baby { 1.0625 } else { 1.0 };
        for (index, &(from, to, uv, pivot)) in (if zombie.baby { &BABY } else { &ADULT }).iter().enumerate() {
            let mut uv = uv;
            // DrownedModel gives its left limbs their own skin, unmirrored;
            // HumanoidModel mirrors the adult's left arm and leg.
            let drowned = zombie.kind == ZombieKind::Drowned && !zombie.baby;
            if drowned && index == 4 {
                uv = [32., 48.];
            } else if drowned && index == 6 {
                uv = [16., 48.];
            }
            let mirror = !zombie.baby && !drowned && matches!(index, 4 | 6);
            let part_rotation = match index {
                0 | 1 => head,
                2 => Quat::from_rotation_y(pose_limbs.body_yaw),
                3..=6 => limbs[index - 3],
                _ => Quat::IDENTITY,
            };
            let pivot = match index {
                3 | 4 => arm_pivot(index - 3),
                _ => pivot,
            };
            cube_tinted_pose_mirror(
                mesh,
                feet,
                rotation,
                scale,
                region,
                sky,
                block,
                from,
                to,
                uv,
                pivot,
                part_rotation,
                [1.0; 3],
                [64., 64.],
                // The hat is inflated, but ModelPart.Cube builds its UVs
                // from the original size before CubeDeformation is applied.
                if index == 1 {
                    Some(if zombie.baby {
                        [6., 6., 6.]
                    } else {
                        [8., 8., 8.]
                    })
                } else {
                    None
                },
                mirror,
            );
        }
        // `HumanoidArmorLayer` (adults; armour takes no hurt overlay).
        if !zombie.baby && zombie.armor.iter().any(Option::is_some) {
            marks.push((mesh.vertices.len(), 1.0));
            let at = |i: usize| glam::Vec3::from_array(parts[i].3);
            let armor_parts = crate::armor_render::HumanoidParts {
                head: (glam::Vec3::ZERO, head),
                body: (glam::Vec3::ZERO, Quat::from_rotation_y(pose_limbs.body_yaw)),
                arms: [(glam::Vec3::from_array(arm_pivot(0)), limbs[0]), (glam::Vec3::from_array(arm_pivot(1)), limbs[1])],
                legs: [(at(5), limbs[2]), (at(6), limbs[3])],
            };
            crate::armor_render::append_humanoid_armor(mesh, atlas, feet, rotation, scale, &armor_parts, &zombie.armor, sky, block);
        }
    }
    crate::cow_render::apply_overlays(mesh, &marks);
    held
}

/// A humanoid zombie's pose: the body's twist, the arms' pivots (x, z in
/// pixels, where an attack moves them) and the right arm, left arm, right
/// leg and left leg rotations.
pub struct ZombieLimbs {
    pub body_yaw: f32,
    pub arm_pivots: [(f32, f32); 2],
    pub limbs: [Quat; 4],
}

/// `HumanoidModel.setupAnim` for a zombie:
/// - the leg swing, with a touch of yaw and roll each way;
/// - `setupAttackAnimation`, whose swing (a right-handed one) twists the
///   body and moves the arms' pivots with it, scaled by the age;
/// - the arms `AnimationUtils.animateZombieArms` raises there, with the
///   swing's reach;
/// - `bobModelPart` on each arm.
pub fn limb_rotations(walk_position: f32, walk_speed: f32, age: f32, aggressive: bool, swing: Option<f32>, age_scale: f32, rest_pivots: [(f32, f32); 2]) -> ZombieLimbs {
    use minecraftoss_player::mth;
    let pi = std::f32::consts::PI;
    let walk = walk_position * 0.6662;
    let right_leg = mth::cos(f64::from(walk)) * 1.4 * walk_speed;
    let left_leg = mth::cos(f64::from(walk + pi)) * 1.4 * walk_speed;
    let (mut body_yaw, mut arm_pivots) = (0.0, rest_pivots);
    let attack = swing.filter(|&s| s > 0.0);
    if let Some(s) = attack {
        body_yaw = mth::sin(f64::from(s.sqrt() * (pi * 2.0))) * 0.2;
        let (sin, cos) = (mth::sin(f64::from(body_yaw)), mth::cos(f64::from(body_yaw)));
        arm_pivots = [(-cos * 5.0 * age_scale, sin * 5.0 * age_scale), (cos * 5.0 * age_scale, -sin * 5.0 * age_scale)];
    }
    let s = attack.unwrap_or(0.0);
    let arm_drop = -pi / if aggressive { 1.5 } else { 2.25 };
    let attack_y = mth::sin(f64::from(s * pi));
    let attack_x = mth::sin(f64::from((1.0 - (1.0 - s) * (1.0 - s)) * pi));
    let arm_x = arm_drop + attack_y * 1.2 - attack_x * 0.4;
    let arm_y = 0.1 - attack_y * 0.6;
    let bob_z = mth::cos(f64::from(age * 0.09)) * 0.05 + 0.05;
    let bob_x = mth::sin(f64::from(age * 0.067)) * 0.05;
    let part = |x: f32, y: f32, z: f32| Quat::from_euler(EulerRot::ZYX, z, y, x);
    ZombieLimbs {
        body_yaw,
        arm_pivots,
        limbs: [
            part(arm_x + bob_x, -arm_y, bob_z),
            part(arm_x - bob_x, arm_y, -bob_z),
            part(right_leg, 0.005, 0.005),
            part(left_leg, -0.005, -0.005),
        ],
    }
}

/// A zombie villager part: box corners, texture offset, pivot, which pose
/// it takes (0 body, 1 head, 2 right arm, 3 left arm, 4 the head's hat
/// rim), mirrored texture and the size its UVs come from when inflated.
type VillagerPart = ([f32; 3], [f32; 3], [f32; 2], [f32; 3], u8, bool, Option<[f32; 3]>);

/// `ZombieVillagerModel.createBodyLayer`: the head with its nose, hat and
/// rim, the body with its coat, zombie arms and legs.
const VILLAGER_ADULT: [VillagerPart; 10] = [
    ([-4., -10., -4.], [4., 0., 4.], [0., 0.], [0., 0., 0.], 1, false, None),
    ([-1., -3., -6.], [1., 1., -4.], [24., 0.], [0., 0., 0.], 1, false, None),
    ([-4.5, -10.5, -4.5], [4.5, 0.5, 4.5], [32., 0.], [0., 0., 0.], 1, false, Some([8., 10., 8.])),
    ([-8., -8., -6.], [8., 8., -5.], [30., 47.], [0., 0., 0.], 4, false, None),
    ([-4., 0., -3.], [4., 12., 3.], [16., 20.], [0., 0., 0.], 0, false, None),
    ([-4.05, -0.05, -3.05], [4.05, 20.05, 3.05], [0., 38.], [0., 0., 0.], 0, false, Some([8., 20., 6.])),
    ([-3., -2., -2.], [1., 10., 2.], [44., 22.], [-5., 2., 0.], 2, false, None),
    ([-1., -2., -2.], [3., 10., 2.], [44., 22.], [5., 2., 0.], 3, true, None),
    ([-2., 0., -2.], [2., 12., 2.], [0., 22.], [-2., 12., 0.], 0, false, None),
    ([-2., 0., -2.], [2., 12., 2.], [0., 22.], [2., 12., 0.], 0, true, None),
];

/// `BabyZombieVillagerModel.createBodyLayer`, children folded into the
/// head's space.
const VILLAGER_BABY: [VillagerPart; 10] = [
    ([-2., -2.75, -1.5], [2., 2.25, 1.5], [0., 15.], [0., 18.75, 0.], 0, false, None),
    ([-2.1, -2.85, -1.6], [2.1, 3.35, 1.6], [16., 22.], [0., 18.75, 0.], 0, false, Some([4., 6., 3.])),
    ([-4., -8., -3.5], [4., 0., 3.5], [0., 0.], [0., 16., 0.], 1, false, None),
    ([-4.3, -8.3, -3.8], [4.3, 0.3, 3.8], [0., 31.], [0., 16., 0.], 1, false, Some([8., 8., 7.])),
    ([-7., -5., -6.], [7., -4., 6.], [0., 46.], [0., 16., 0.], 1, false, None),
    ([-1., -2., -4.5], [1., 0., -3.5], [23., 0.], [0., 16., 0.], 1, false, None),
    ([-1., -0.5, -1.], [1., 4.5, 1.], [24., 15.], [-3., 15.5, 0.], 2, false, None),
    ([-1., -0.5, -1.], [1., 4.5, 1.], [16., 15.], [3., 15.5, 0.], 3, false, None),
    ([-1., -0.5, -1.], [1., 2.5, 1.], [8., 23.], [-1., 21.5, 0.], 0, false, None),
    ([-1., -0.5, -1.], [1., 2.5, 1.], [0., 23.], [1., 21.5, 0.], 0, false, None),
];

/// The part of a namespaced ID after the namespace.
fn path_of(id: &str) -> &str {
    id.split_once(':').map_or(id, |(_, path)| path)
}

/// A zombie villager: its base skin, then its villager type's overlay and,
/// for adults, its profession's, on the same model.
fn append_zombie_villager(mesh: &mut ChunkMesh, entity: &ZombieEntity, pose: &MobPose, atlas: &Atlas, light: &SkyLight) {
    let zombie = &entity.zombie;
    let feet = pose.feet;
    let sample = pose.light_block();
    let (sky, block) = (light.get(sample) as f32, light.get_block(sample) as f32);
    let rotation = pose.body_rotation(90.0);
    let head = Quat::from_euler(EulerRot::ZYX, 0.0, pose.head_yaw.to_radians(), pose.head_pitch.to_radians());
    // `AnimationUtils.animateZombieArms` without a swing: raised arms,
    // higher when aggressive.
    let arm_x = -std::f32::consts::PI / if entity.aggressive { 1.5 } else { 2.25 };
    let right_arm = Quat::from_euler(EulerRot::ZYX, 0.0, -0.1, arm_x);
    let left_arm = Quat::from_euler(EulerRot::ZYX, 0.0, 0.1, arm_x);
    let rim = head * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    let (kind, profession) = zombie.villager.as_ref().map_or(("plains", "none"), |(kind, profession)| (path_of(kind), path_of(profession)));
    let mut skins = vec![if zombie.baby { "minecraft:entity/zombie_villager/zombie_villager_baby".to_owned() } else { "minecraft:entity/zombie_villager/zombie_villager".to_owned() }];
    skins.push(format!("minecraft:entity/zombie_villager/{}/{kind}", if zombie.baby { "baby" } else { "type" }));
    if !zombie.baby && profession != "none" {
        skins.push(format!("minecraft:entity/zombie_villager/profession/{profession}"));
    }
    let parts = if zombie.baby { &VILLAGER_BABY } else { &VILLAGER_ADULT };
    for skin in skins {
        let Ok(id) = ResourceId::parse(&skin) else { continue };
        if !atlas.contains(&id) {
            continue;
        }
        let region = atlas.entity_region(&id);
        for &(from, to, uv, pivot, pose, mirror, dims) in parts {
            let part_rotation = match pose {
                1 => head,
                2 => right_arm,
                3 => left_arm,
                4 => rim,
                _ => Quat::IDENTITY,
            };
            crate::cow_render::cube_scaled(mesh, feet, rotation, glam::Vec3::ONE, region, sky, block, from, to, uv, pivot, part_rotation, [1.0; 3], [64., 64.], dims, mirror);
        }
    }
}
