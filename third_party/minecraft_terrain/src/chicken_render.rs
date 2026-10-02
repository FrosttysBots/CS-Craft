//! Pinned 26.3 AdultChickenModel, BabyChickenModel and ColdChickenModel cuboids.
//! Textures resolve through the selected resource-pack atlas.
use crate::{
    client_mobs::ClientMobs,
    cow_render::cube_tinted_pose,
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
};
use glam::Quat;
use minecraftoss_entities::{chicken::ChickenVariant, world::ChickenEntity};

type Part = ([f32; 3], [f32; 3], [f32; 2], [f32; 3], bool);

const ADULT: [Part; 8] = [
    (
        [-2., -6., -2.],
        [2., 0., 1.],
        [0., 0.],
        [0., 15., -4.],
        false,
    ),
    (
        [-2., -4., -4.],
        [2., -2., -2.],
        [14., 0.],
        [0., 15., -4.],
        false,
    ),
    (
        [-1., -2., -3.],
        [1., 0., -1.],
        [14., 4.],
        [0., 15., -4.],
        false,
    ),
    ([-3., -4., -3.], [3., 4., 3.], [0., 9.], [0., 16., 0.], true),
    (
        [-1., 0., -3.],
        [2., 5., 0.],
        [26., 0.],
        [-2., 19., 1.],
        false,
    ),
    (
        [-1., 0., -3.],
        [2., 5., 0.],
        [26., 0.],
        [1., 19., 1.],
        false,
    ),
    (
        [0., 0., -3.],
        [1., 4., 3.],
        [24., 13.],
        [-4., 13., 0.],
        false,
    ),
    (
        [-1., 0., -3.],
        [0., 4., 3.],
        [24., 13.],
        [4., 13., 0.],
        false,
    ),
];

const BABY: [Part; 8] = [
    (
        [-2., -2.25, -0.75],
        [2., 1.75, 3.25],
        [0., 0.],
        [0., 20.25, -1.25],
        false,
    ),
    (
        [-1., -0.25, -1.75],
        [1., 0.75, -0.75],
        [10., 8.],
        [0., 20.25, -1.25],
        false,
    ),
    (
        [-0.5, 0., 0.],
        [0.5, 2., 0.],
        [2., 2.],
        [1., 22., 0.5],
        false,
    ),
    (
        [-0.5, 2., -1.],
        [0.5, 2., 0.],
        [0., 1.],
        [1., 22., 0.5],
        false,
    ),
    (
        [-0.5, 0., 0.],
        [0.5, 2., 0.],
        [0., 2.],
        [-1., 22., 0.5],
        false,
    ),
    (
        [-0.5, 2., -1.],
        [0.5, 2., 0.],
        [0., 0.],
        [-1., 22., 0.5],
        false,
    ),
    ([0., 0., -1.], [1., 0., 1.], [6., 8.], [2., 20., 0.], false),
    (
        [-1., 0., -1.],
        [0., 0., 1.],
        [4., 8.],
        [-2., 20., 0.],
        false,
    ),
];

pub fn texture_id(variant: ChickenVariant, baby: bool) -> ResourceId {
    let kind = match variant {
        ChickenVariant::Temperate => "temperate",
        ChickenVariant::Warm => "warm",
        ChickenVariant::Cold => "cold",
    };
    ResourceId::parse(&format!(
        "minecraft:entity/chicken/chicken_{kind}{}",
        if baby { "_baby" } else { "" }
    ))
    .unwrap()
}

pub fn append_chickens<'a>(
    mesh: &mut ChunkMesh,
    chickens: impl IntoIterator<Item = &'a ChickenEntity>,
    poses: &ClientMobs,
    atlas: &Atlas,
    light: &SkyLight,
    partial: f32,
) {
    // Each mob's first vertex and overlay (`getOverlayCoords`).
    let mut marks = Vec::new();
    for entity in chickens {
        let chicken = &entity.chicken;
        let Some(mob) = poses.pose(entity.id, partial) else { continue };
        marks.push((mesh.vertices.len(), mob.overlay(0.0)));
        let feet = mob.feet;
        let rotation = mob.body_rotation(90.0);
        let baby = chicken.age.baby();
        let region = atlas.entity_region(&texture_id(chicken.variant, baby));
        // ChickenModel.setupAnim (and AdultChickenModel's looking head):
        // legs swing, wings flap by `(sin(flap) + 1) * flapSpeed`.
        let head = Quat::from_euler(glam::EulerRot::ZYX, 0.0, mob.head_yaw.to_radians(), mob.head_pitch.to_radians());
        let flap = (minecraftoss_player::mth::sin(f64::from(mob.flap)) + 1.0) * mob.flap_speed;
        let swing = crate::client_mobs::mth_cos(mob.walk_position * 0.6662) * 1.4 * mob.walk_speed;
        let swing_back = crate::client_mobs::mth_cos(mob.walk_position * 0.6662 + std::f32::consts::PI) * 1.4 * mob.walk_speed;
        let (right_leg, left_leg) = (Quat::from_rotation_x(swing), Quat::from_rotation_x(swing_back));
        let (right_wing, left_wing) = (Quat::from_rotation_z(flap), Quat::from_rotation_z(-flap));
        let part = |index: usize| -> Quat {
            if baby {
                match index {
                    2 | 3 => left_leg,
                    4 | 5 => right_leg,
                    6 => right_wing,
                    7 => left_wing,
                    _ => Quat::IDENTITY,
                }
            } else {
                match index {
                    0..=2 => head,
                    3 => Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                    4 => right_leg,
                    5 => left_leg,
                    6 => right_wing,
                    7 => left_wing,
                    _ => Quat::IDENTITY,
                }
            }
        };
        let pos = mob.light_block();
        let sky = light.get(pos) as f32;
        let block = light.get_block(pos) as f32;
        for (index, &(from, to, uv, pivot, _)) in (if baby { &BABY[..] } else { &ADULT[..] }).iter().enumerate() {
            cube_tinted_pose(
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
                part(index),
                [1.0; 3],
                if baby { [16., 16.] } else { [64., 32.] },
                None,
            );
        }
        if chicken.variant == ChickenVariant::Cold && !baby {
            for (index, (from, to, uv, pivot, _)) in [
                ([0., 3., -1.], [0., 6., 4.], [38., 9.], [0., 16., 0.], true),
                (
                    [-3., -7., -2.015],
                    [3., -4., 1.985],
                    [44., 0.],
                    [0., 15., -4.],
                    false,
                ),
            ]
            .into_iter()
            .enumerate()
            {
                // The cold coat's body piece, then its head piece.
                cube_tinted_pose(
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
                    if index == 0 { part(3) } else { head },
                    [1.0; 3],
                    [64., 32.],
                    None,
                );
            }
        }
    }
    crate::cow_render::apply_overlays(mesh, &marks);
}
