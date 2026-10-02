//! Pack-backed villager meshes for the pinned Java 26.3 client.
//! Sources: VillagerModel.createBodyModel and setupAnim,
//! BabyVillagerModel.createBodyModel, LayerDefinitions.VILLAGER(_BABY),
//! VillagerRenderer, VillagerProfessionLayer. The body turns to its yaw, the
//! head to its look (turn and pitch), shaking while unhappy, the legs
//! swing with the walk.
//! Villager type/profession state is still pending.
use crate::{
    cow_render::cube_tinted_pose_mirror,
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
    client_mobs::ClientMobs,
};
use glam::{EulerRot, Quat, Vec3};
use minecraftoss_entities::world::VillagerEntity;
use std::f32::consts::PI;

/// A part's rotation and origin in the model (pixels, y down), composed
/// down the part tree (`ModelPart.translateAndRotate`).
#[derive(Clone, Copy)]
struct Pose {
    rotation: Quat,
    origin: Vec3,
}

impl Pose {
    const ROOT: Pose = Pose { rotation: Quat::IDENTITY, origin: Vec3::ZERO };

    fn child(self, offset: [f32; 3], x: f32, y: f32, z: f32) -> Pose {
        Pose { rotation: self.rotation * Quat::from_euler(EulerRot::ZYX, z, y, x), origin: self.origin + self.rotation * Vec3::from_array(offset) }
    }
}

type Part = ([f32; 3], [f32; 3], [f32; 2], [f32; 3]);

// Coordinates remain in the native, downward-positive 24-pixel model space.
const ADULT: [Part; 11] = [
    ([-4., -10., -4.], [4., 0., 4.], [0., 0.], [0., 0., 0.]),
    (
        [-4.51, -10.51, -4.51],
        [4.51, 0.51, 4.51],
        [32., 0.],
        [0., 0., 0.],
    ),
    ([-8., -8., -6.], [8., 8., -5.], [30., 47.], [0., 0., 0.]),
    ([-1., -1., -6.], [1., 3., -4.], [24., 0.], [0., -2., 0.]),
    ([-4., 0., -3.], [4., 12., 3.], [16., 20.], [0., 0., 0.]),
    (
        [-4.5, -0.5, -3.5],
        [4.5, 20.5, 3.5],
        [0., 38.],
        [0., 0., 0.],
    ),
    ([-8., -2., -2.], [-4., 6., 2.], [44., 22.], [0., 3., -1.]),
    ([4., -2., -2.], [8., 6., 2.], [44., 22.], [0., 3., -1.]),
    ([-4., 2., -2.], [4., 6., 2.], [40., 38.], [0., 3., -1.]),
    ([-2., 0., -2.], [2., 12., 2.], [0., 22.], [-2., 12., 0.]),
    ([-2., 0., -2.], [2., 12., 2.], [0., 22.], [2., 12., 0.]),
];

const BABY: [Part; 11] = [
    ([-4., -8., -3.5], [4., 0., 3.5], [0., 0.], [0., 16., 0.]),
    (
        [-4.3, -8.3, -3.8],
        [4.3, 0.3, 3.8],
        [0., 30.],
        [0., 16., 0.],
    ),
    ([-7., -5., -6.], [7., -4., 6.], [0., 45.], [0., 16., 0.]),
    ([-1., 0., -0.5], [1., 2., 0.5], [23., 0.], [0., 14., -4.]),
    (
        [-2., -2.75, -1.5],
        [2., 2.25, 1.5],
        [0., 15.],
        [0., 18.75, 0.],
    ),
    (
        [-2.7, -8.2, -1.7],
        [1.7, -1.8, 1.7],
        [16., 21.],
        [0.5, 24., 0.],
    ),
    (
        [-1., -2.4925, -1.8401],
        [1., 1.5075, 0.1599],
        [36., 15.],
        [-3., 18.9025, -0.9599],
    ),
    (
        [5., -2.4925, -1.8401],
        [7., 1.5075, 0.1599],
        [16., 15.],
        [-3., 18.9025, -0.9599],
    ),
    (
        [-2., -0.9924, -0.9825],
        [2., 1.0076, 1.0175],
        [24., 17.],
        [0., 18.4024, -1.8175],
    ),
    ([-1., -0.5, -1.], [1., 2.5, 1.], [8., 23.], [-1., 21.5, 0.]),
    ([-1., -0.5, -1.], [1., 2.5, 1.], [0., 23.], [1., 21.5, 0.]),
];

/// `VillagerMetadataSection.Hat` of the vanilla pack's villager textures.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hat {
    None,
    Partial,
    Full,
}

fn hat_of_type(kind: &str) -> Hat {
    if matches!(kind, "desert" | "snow") {
        Hat::Full
    } else {
        Hat::None
    }
}

fn hat_of_profession(profession: &str) -> Hat {
    match profession {
        "butcher" => Hat::Partial,
        "farmer" | "fisherman" | "fletcher" | "librarian" | "shepherd" => Hat::Full,
        _ => Hat::None,
    }
}

pub fn append_villagers<'a>(
    mesh: &mut ChunkMesh,
    villagers: impl IntoIterator<Item = &'a VillagerEntity>,
    poses: &ClientMobs,
    bed_facing: &dyn Fn((i32, i32, i32)) -> Option<String>,
    atlas: &Atlas,
    light: &SkyLight,
    partial: f32,
) -> Vec<crate::mesh::HeldItem> {
    let mut held = Vec::new();
    // Each mob's first vertex and overlay (`getOverlayCoords`).
    let mut marks = Vec::new();
    for entity in villagers {
        let villager = &entity.villager;
        let baby = villager.age.baby();
        let Some(mob) = poses.pose(entity.id, partial) else { continue };
        marks.push((mesh.vertices.len(), mob.overlay(0.0)));
        let mut feet = mob.feet;
        let sample = mob.light_block();
        let sky = light.get(sample) as f32;
        let block = light.get_block(sample) as f32;
        // `VillagerModel.setupAnim`: the head's turn from the body and its
        // pitch; each leg `cos(walk * 0.6662 (+ pi)) * 1.4 * speed * 0.5`.
        let (body_yaw, turn, pitch) = (mob.body_rot, mob.head_yaw, mob.head_pitch);
        let mut rotation = mob.body_rotation(90.0);
        // `LivingEntityRenderer`: a sleeper lies along its bed
        // (`sleepDirectionToRotation`, flipped 90 on Z, then 270 on Y), its
        // head a tenth of a block towards the pillow.
        if let Some(bed) = entity.sleeping {
            let facing = bed_facing(bed);
            let (angle, step) = match facing.as_deref() {
                Some("south") => (90.0_f32, (0.0, 1.0)),
                Some("west") => (0.0, (-1.0, 0.0)),
                Some("north") => (270.0, (0.0, -1.0)),
                Some("east") => (180.0, (1.0, 0.0)),
                _ => (body_yaw, (0.0, 0.0)),
            };
            rotation = Quat::from_rotation_y(angle.to_radians()) * Quat::from_rotation_z(90.0_f32.to_radians()) * Quat::from_rotation_y(270.0_f32.to_radians());
            let offset = f64::from(entity.eye_height() - 0.1);
            feet -= glam::DVec3::new(step.0 * offset, 0.0, step.1 * offset);
        }
        let (walk_position, walk_speed) = (mob.walk_position, mob.walk_speed);
        let swing = walk_position * 0.6662;
        let (right, left) = (swing.cos() * 1.4 * walk_speed * 0.5, (swing + PI).cos() * 1.4 * walk_speed * 0.5);
        // An unhappy villager (a refused trade) tips its head and shakes it.
        let (head_pitch, head_roll) = if entity.unhappy > 0 {
            let age = mob.age_in_ticks;
            (0.4, 0.3 * (0.45 * age).sin())
        } else {
            (pitch.to_radians(), 0.0)
        };
        let head = Pose::ROOT.child(if baby { [0.0, 16.0, 0.0] } else { [0.0; 3] }, head_pitch, turn.to_radians(), head_roll);
        let parts = if baby { &BABY } else { &ADULT };
        let base = ResourceId::parse(if baby {
            "minecraft:entity/villager/villager_baby"
        } else {
            "minecraft:entity/villager/villager"
        })
        .unwrap();
        // `VillagerProfessionLayer`: the type (its hat hidden under a
        // profession's full hat, or a partial one over a full type hat),
        // then an adult's profession and its level badge.
        let kind = villager.kind.strip_prefix("minecraft:").unwrap_or(&villager.kind);
        let profession = villager.profession.id().strip_prefix("minecraft:").unwrap_or("none");
        let (type_hat, profession_hat) = (hat_of_type(kind), hat_of_profession(profession));
        let type_hat_visible = profession_hat == Hat::None || (profession_hat == Hat::Partial && type_hat != Hat::Full);
        let mut layers = vec![(base, true)];
        if let Ok(texture) = ResourceId::parse(&format!("minecraft:entity/villager/{}/{kind}", if baby { "baby" } else { "type" })) {
            layers.push((texture, type_hat_visible));
        }
        if !baby && profession != "none" {
            if let Ok(texture) = ResourceId::parse(&format!("minecraft:entity/villager/profession/{profession}")) {
                layers.push((texture, true));
            }
            if profession != "nitwit" {
                let level = ["stone", "iron", "gold", "emerald", "diamond"][(villager.level.clamp(1, 5) - 1) as usize];
                if let Ok(texture) = ResourceId::parse(&format!("minecraft:entity/villager/profession_level/{level}")) {
                    layers.push((texture, true));
                }
            }
        }
        // `CrossedArmsItemLayer`: the held item in the crossed arms (the
        // adult's root carries the villagers' 0.9375 scale), shown as on
        // the ground.
        if let Some(item) = &entity.held_item {
            use glam::Mat4;
            // The baby's `arms` is the group its hands hang from: moved,
            // not turned.
            let arms = if baby { Pose::ROOT.child([0.0, 17.5, 0.0], 0.0, 0.0, 0.0) } else { Pose::ROOT.child([0.0, 3.0, -1.0], -0.75, 0.0, 0.0) };
            let root = if baby { Mat4::IDENTITY } else { Mat4::from_translation(Vec3::new(0.0, 24.016 * (1.0 - 0.9375) / 16.0, 0.0)) * Mat4::from_scale(Vec3::splat(0.9375)) };
            let pose = Mat4::from_translation(feet.as_vec3())
                * Mat4::from_quat(rotation)
                * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
                * Mat4::from_translation(Vec3::new(0.0, -1.501, 0.0))
                * root
                * Mat4::from_translation(arms.origin / 16.0)
                * Mat4::from_quat(arms.rotation)
                * Mat4::from_rotation_x(0.75)
                * Mat4::from_scale(Vec3::splat(1.07))
                * Mat4::from_translation(Vec3::new(0.0, 0.13, -0.34))
                * Mat4::from_rotation_x(PI);
            held.push(crate::mesh::HeldItem { pose, light: mob.light_probe.as_vec3(), id: item.id.clone(), display: crate::mesh::HeldDisplay::Ground, first_tint: None });
        }
        for (texture, hat_visible) in layers {
            let region = atlas.entity_region(&texture);
            for (index, &(from, to, uv, pivot)) in parts.iter().enumerate() {
                // The no-hat model leaves out the hat and its rim.
                if !hat_visible && !baby && matches!(index, 1 | 2) {
                    continue;
                }
                // CubeDeformation expands geometry without expanding its
                // texture layout. The hat and robe would otherwise read
                // neighboring face panels on the 64x64 skin.
                let uv_dimensions = if baby {
                    match index {
                        1 => Some([8., 8., 7.]),
                        5 => Some([4., 6., 3.]),
                        _ => None,
                    }
                } else {
                    match index {
                        1 => Some([8., 10., 8.]),
                        5 => Some([8., 20., 6.]),
                        _ => None,
                    }
                };
                // The head and what hangs on it turn with it; the legs swing.
                let pose = match (baby, index) {
                    (_, 0 | 1) => head,
                    (false, 2) => head.child([0.0; 3], -PI / 2.0, 0.0, 0.0),
                    (true, 2) => head,
                    (false, 3) => head.child([0.0, -2.0, 0.0], 0.0, 0.0, 0.0),
                    (true, 3) => head.child([0.0, -2.0, -4.0], 0.0, 0.0, 0.0),
                    (false, 6..=8) => Pose::ROOT.child(pivot, -0.75, 0.0, 0.0),
                    (true, 6..=8) => Pose::ROOT.child(pivot, -1.0472, 0.0, 0.0),
                    (_, 9) => Pose::ROOT.child(pivot, right, 0.0, 0.0),
                    (_, 10) => Pose::ROOT.child(pivot, left, 0.0, 0.0),
                    _ => Pose::ROOT.child(pivot, 0.0, 0.0, 0.0),
                };
                let (pivot, part_rotation) = (pose.origin.to_array(), pose.rotation);
                // VillagerModel mirrors the adult's left leg.
                cube_tinted_pose_mirror(
                    mesh,
                    feet,
                    rotation,
                    if baby { 1.0 } else { 0.9375 },
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
                    uv_dimensions,
                    !baby && index == 10,
                );
            }
        }
    }
    crate::cow_render::apply_overlays(mesh, &marks);
    held
}
