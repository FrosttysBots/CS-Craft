//! Pinned 26.3 PigModel/BabyPigModel cuboids with pack-resolved textures.
//! Active gait and view-dependent pose remain separate.
use crate::{
    client_mobs::ClientMobs,
    cow_render::{cube_tinted_pose, cube_tinted_pose_mirror},
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
};
use glam::Quat;
use minecraftoss_entities::{pig::PigVariant, world::PigEntity};

type BoxPart = (
    [f32; 3],
    [f32; 3],
    [f32; 2],
    [f32; 3],
    bool,
    Option<[f32; 3]>,
);

const ADULT: [BoxPart; 7] = [
    (
        [-4., -4., -8.],
        [4., 4., 0.],
        [0., 0.],
        [0., 12., -6.],
        false,
        None,
    ),
    (
        [-2., 0., -9.],
        [2., 3., -8.],
        [16., 16.],
        [0., 12., -6.],
        false,
        None,
    ),
    (
        [-5., -10., -7.],
        [5., 6., 1.],
        [28., 8.],
        [0., 11., 2.],
        true,
        None,
    ),
    (
        [-2., 0., -2.],
        [2., 6., 2.],
        [0., 16.],
        [-3., 18., 7.],
        false,
        None,
    ),
    (
        [-2., 0., -2.],
        [2., 6., 2.],
        [0., 16.],
        [3., 18., 7.],
        false,
        None,
    ),
    (
        [-2., 0., -2.],
        [2., 6., 2.],
        [0., 16.],
        [-3., 18., -5.],
        false,
        None,
    ),
    (
        [-2., 0., -2.],
        [2., 6., 2.],
        [0., 16.],
        [3., 18., -5.],
        false,
        None,
    ),
];

const BABY: [BoxPart; 7] = [
    (
        [-3.5, -3., -4.5],
        [3.5, 3., 4.5],
        [0., 0.],
        [0., 19., 0.5],
        false,
        None,
    ),
    (
        [-3.525, -5.025, -5.025],
        [3.525, 1.025, 1.025],
        [0., 15.],
        [0., 19., -2.],
        false,
        Some([7., 6., 6.]),
    ),
    (
        [-1.515, -1.99, -6.015],
        [1.515, 0.04, -4.985],
        [6., 27.],
        [0., 19., -2.],
        false,
        Some([3., 2., 1.]),
    ),
    (
        [-1., 0., -1.],
        [1., 2., 1.],
        [0., 0.],
        [2.5, 22., -3.],
        false,
        None,
    ),
    (
        [-1., 0., -1.],
        [1., 2., 1.],
        [23., 0.],
        [-2.5, 22., -3.],
        false,
        None,
    ),
    (
        [-1., 0., -1.],
        [1., 2., 1.],
        [0., 4.],
        [2.5, 22., 4.],
        false,
        None,
    ),
    (
        [-1., 0., -1.],
        [1., 2., 1.],
        [23., 4.],
        [-2.5, 22., 4.],
        false,
        None,
    ),
];

pub fn texture_id(variant: PigVariant, baby: bool) -> ResourceId {
    let kind = match variant {
        PigVariant::Temperate => "temperate",
        PigVariant::Warm => "warm",
        PigVariant::Cold => "cold",
    };
    ResourceId::parse(&format!(
        "minecraft:entity/pig/pig_{kind}{}",
        if baby { "_baby" } else { "" }
    ))
    .unwrap()
}

/// Each part's pose (`QuadrupedModel.setupAnim`): the head (and snout)
/// looks, the adult body lies along the pig, and the legs swing.
fn part_pose(index: usize, baby: bool, head: Quat, legs: [f32; 4]) -> Quat {
    let [right_hind, left_hind, right_front, left_front] = legs;
    let leg = |angle: f32| Quat::from_rotation_x(angle);
    if baby {
        match index {
            1 | 2 => head,
            3 => leg(left_front),
            4 => leg(right_front),
            5 => leg(left_hind),
            6 => leg(right_hind),
            _ => Quat::IDENTITY,
        }
    } else {
        match index {
            0 | 1 => head,
            2 => Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            3 => leg(right_hind),
            4 => leg(left_hind),
            5 => leg(right_front),
            6 => leg(left_front),
            _ => Quat::IDENTITY,
        }
    }
}

pub fn append_pigs<'a>(mesh: &mut ChunkMesh, pigs: impl IntoIterator<Item = &'a PigEntity>, poses: &ClientMobs, atlas: &Atlas, light: &SkyLight, partial: f32) {
    // Each mob's first vertex and overlay (`getOverlayCoords`).
    let mut marks = Vec::new();
    for entity in pigs {
        let pig = &entity.pig;
        let Some(mob) = poses.pose(entity.id, partial) else { continue };
        marks.push((mesh.vertices.len(), mob.overlay(0.0)));
        let feet = mob.feet;
        let baby = pig.age.baby();
        let region = atlas.entity_region(&texture_id(pig.variant, baby));
        let position = mob.light_block();
        let sky = light.get(position) as f32;
        let block = light.get_block(position) as f32;
        let rotation = mob.body_rotation(90.0);
        let head = Quat::from_euler(glam::EulerRot::ZYX, 0.0, mob.head_yaw.to_radians(), mob.head_pitch.to_radians());
        let legs = crate::client_mobs::quadruped_legs(mob.walk_position, mob.walk_speed);
        for (index, &(from, to, uv, pivot, _, dimensions)) in
            (if baby { &BABY[..] } else { &ADULT[..] }).iter().enumerate()
        {
            // PigModel mirrors the adult's left legs.
            cube_tinted_pose_mirror(
                mesh,
                feet,
                rotation,
                1.0,
                region,
                sky,
                block,
                from,
                to,
                uv,
                pivot,
                part_pose(index, baby, head, legs),
                [1.0; 3],
                if baby { [32., 32.] } else { [64., 64.] },
                dimensions,
                !baby && matches!(index, 4 | 6),
            );
        }
        if pig.variant == PigVariant::Cold && !baby {
            // ColdPigModel adds a half-pixel body coat over the base body.
            cube_tinted_pose(
                mesh,
                feet,
                rotation,
                1.0,
                region,
                sky,
                block,
                [-5.5, -10.5, -7.5],
                [5.5, 6.5, 1.5],
                [28., 32.],
                [0., 11., 2.],
                part_pose(2, false, head, legs),
                [1.0; 3],
                [64., 64.],
                Some([10., 16., 8.]),
            );
        }
        if pig.saddled && !baby {
            // ModelLayers.PIG_SADDLE bakes PigModel with CubeDeformation(0.5).
            let saddle = atlas.entity_region(
                &ResourceId::parse("minecraft:entity/equipment/pig_saddle/saddle").unwrap(),
            );
            for (index, &(from, to, uv, pivot, _, _)) in ADULT.iter().enumerate() {
                let dimensions = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
                let inflated_from = [from[0] - 0.5, from[1] - 0.5, from[2] - 0.5];
                let inflated_to = [to[0] + 0.5, to[1] + 0.5, to[2] + 0.5];
                cube_tinted_pose_mirror(
                    mesh,
                    feet,
                    rotation,
                    1.0,
                    saddle,
                    sky,
                    block,
                    inflated_from,
                    inflated_to,
                    uv,
                    pivot,
                    part_pose(index, false, head, legs),
                    [1.0; 3],
                    [64., 64.],
                    Some(dimensions),
                    matches!(index, 4 | 6),
                );
            }
        }
    }
    crate::cow_render::apply_overlays(mesh, &marks);
}
