//! Source-informed pinned 26.3 sheep model layers with pack-resolved textures.
//! The authored NoAI sheep uses a fixed pose until its active animation lands.
use crate::{
    client_mobs::ClientMobs,
    cow_render::cube_tinted_pose_mirror,
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
};
use glam::Quat;
use minecraftoss_entities::world::SheepEntity;

const DYE_DIFFUSE: [u32; 16] = [
    16383998, 16351261, 13061821, 3847130, 16701501, 8439583, 15961002, 4673362, 10329495, 1481884,
    8991416, 3949738, 8606770, 6192150, 11546150, 1908001,
];

fn wool_tint(id: usize) -> [f32; 3] {
    // ColorLerper.Type.SHEEP uses a special white and floors 0.75 times
    // DyeColor.getTextureDiffuseColor's channels for every other dye.
    let rgb = if id == 0 { 0xe6e6e6 } else { DYE_DIFFUSE[id] };
    let channel = |shift| {
        let value = ((rgb >> shift) & 255_u32) as f32;
        if id == 0 {
            value / 255.0
        } else {
            (value * 0.75).floor() / 255.0
        }
    };
    [channel(16), channel(8), channel(0)]
}

type BoxPart = ([f32; 3], [f32; 3], [f32; 2], [f32; 3], bool);

const ADULT_BODY: [BoxPart; 6] = [
    (
        [-3., -4., -6.],
        [3., 2., 2.],
        [0., 0.],
        [0., 6., -8.],
        false,
    ),
    (
        [-4., -10., -7.],
        [4., 6., -1.],
        [28., 8.],
        [0., 5., 2.],
        true,
    ),
    (
        [-2., 0., -2.],
        [2., 12., 2.],
        [0., 16.],
        [-3., 12., 7.],
        false,
    ),
    (
        [-2., 0., -2.],
        [2., 12., 2.],
        [0., 16.],
        [3., 12., 7.],
        false,
    ),
    (
        [-2., 0., -2.],
        [2., 12., 2.],
        [0., 16.],
        [-3., 12., -5.],
        false,
    ),
    (
        [-2., 0., -2.],
        [2., 12., 2.],
        [0., 16.],
        [3., 12., -5.],
        false,
    ),
];

const ADULT_FUR: [BoxPart; 6] = [
    (
        [-3.6, -4.6, -4.6],
        [3.6, 2.6, 2.6],
        [0., 0.],
        [0., 6., -8.],
        false,
    ),
    (
        [-5.75, -11.75, -8.75],
        [5.75, 7.75, 0.75],
        [28., 8.],
        [0., 5., 2.],
        true,
    ),
    (
        [-2.5, -0.5, -2.5],
        [2.5, 6.5, 2.5],
        [0., 16.],
        [-3., 12., 7.],
        false,
    ),
    (
        [-2.5, -0.5, -2.5],
        [2.5, 6.5, 2.5],
        [0., 16.],
        [3., 12., 7.],
        false,
    ),
    (
        [-2.5, -0.5, -2.5],
        [2.5, 6.5, 2.5],
        [0., 16.],
        [-3., 12., -5.],
        false,
    ),
    (
        [-2.5, -0.5, -2.5],
        [2.5, 6.5, 2.5],
        [0., 16.],
        [3., 12., -5.],
        false,
    ),
];

const BABY_BODY: [BoxPart; 6] = [
    (
        [-3., -2., -4.5],
        [3., 2., 4.5],
        [0., 10.],
        [0., 17., 0.5],
        false,
    ),
    (
        [-2.5, -4.5, -3.5],
        [2.5, 0.5, 1.5],
        [0., 0.],
        [0., 15.5, -2.5],
        false,
    ),
    (
        [-1., 0., -1.],
        [1., 5., 1.],
        [0., 23.],
        [-2., 19., 3.],
        false,
    ),
    (
        [-1., 0., -1.],
        [1., 5., 1.],
        [24., 12.],
        [2., 19., 3.],
        false,
    ),
    (
        [-1., 0., -1.],
        [1., 5., 1.],
        [8., 23.],
        [-2., 19., -2.],
        false,
    ),
    (
        [-1., 0., -1.],
        [1., 5., 1.],
        [24., 5.],
        [2., 19., -2.],
        false,
    ),
];

pub fn append_sheep<'a>(mesh: &mut ChunkMesh, sheep: impl IntoIterator<Item = &'a SheepEntity>, poses: &ClientMobs, atlas: &Atlas, light: &SkyLight, partial: f32) {
    // Each mob's first vertex and overlay (`getOverlayCoords`).
    let mut marks = Vec::new();
    for entity in sheep {
        let baby = entity.sheep.age.baby();
        let Some(mob) = poses.pose(entity.id, partial) else { continue };
        marks.push((mesh.vertices.len(), mob.overlay(0.0)));
        let feet = mob.feet;
        let rotation = mob.body_rotation(90.0);
        // SheepModel.setupAnim: QuadrupedModel's head and legs, then the
        // grazing head drop and pitch (the look pitch when not grazing).
        let t = partial.clamp(0.0, 1.0);
        let head_pitch = if entity.eat_animation_ticks > 0 { entity.eat_head_angle_scale(t) } else { mob.head_pitch.to_radians() };
        let head_drop = entity.eat_head_position_scale(t) * 9.0 * if baby { 0.5 } else { 1.0 };
        let [right_hind, left_hind, right_front, left_front] = crate::client_mobs::quadruped_legs(mob.walk_position, mob.walk_speed);
        let head = Quat::from_euler(glam::EulerRot::ZYX, 0.0, mob.head_yaw.to_radians(), head_pitch);
        let leg = Quat::from_rotation_x;
        let body = if baby { Quat::IDENTITY } else { Quat::from_rotation_x(std::f32::consts::FRAC_PI_2) };
        let pose = Pose {
            parts: if baby {
                [body, head, leg(right_hind), leg(left_hind), leg(right_front), leg(left_front)]
            } else {
                [head, body, leg(right_hind), leg(left_hind), leg(right_front), leg(left_front)]
            },
            head: if baby { 1 } else { 0 },
            head_drop,
        };
        let pos = mob.light_block();
        let sky = light.get(pos) as f32;
        let block = light.get_block(pos) as f32;
        let base = if baby {
            "minecraft:entity/sheep/sheep_baby"
        } else {
            "minecraft:entity/sheep/sheep"
        };
        append_layer(
            mesh,
            atlas,
            base,
            feet,
            rotation,
            sky,
            block,
            if baby { &BABY_BODY } else { &ADULT_BODY },
            [1.0; 3],
            false,
            &pose,
            // SheepModel's body mesh mirrors the right legs.
            !baby,
        );
        let color = (entity.sheep.wool.data() & 15) as usize;
        let tint = wool_tint(color);
        if !baby && color != 0 {
            append_layer(
                mesh,
                atlas,
                "minecraft:entity/sheep/sheep_wool_undercoat",
                feet,
                rotation,
                sky,
                block,
                // `LayerDefinitions`: the undercoat is the body model.
                &ADULT_BODY,
                tint,
                false,
                &pose,
                true,
            );
        }
        if !entity.sheep.wool.sheared() {
            let texture = if baby {
                "minecraft:entity/sheep/sheep_wool_baby"
            } else {
                "minecraft:entity/sheep/sheep_wool"
            };
            append_layer(
                mesh,
                atlas,
                texture,
                feet,
                rotation,
                sky,
                block,
                if baby { &BABY_BODY } else { &ADULT_FUR },
                tint,
                !baby,
                &pose,
                false,
            );
        }
    }
    crate::cow_render::apply_overlays(mesh, &marks);
}

#[allow(clippy::too_many_arguments)]
/// The six parts' rotations, which part is the head, and how far the head
/// drops while grazing.
struct Pose {
    parts: [Quat; 6],
    head: usize,
    head_drop: f32,
}

#[allow(clippy::too_many_arguments)]
fn append_layer(
    mesh: &mut ChunkMesh,
    atlas: &Atlas,
    texture: &str,
    feet: glam::DVec3,
    rotation: Quat,
    sky: f32,
    block: f32,
    parts: &[BoxPart],
    tint: [f32; 3],
    fur: bool,
    pose: &Pose,
    mirror_right_legs: bool,
) {
    let region = atlas.entity_region(&ResourceId::parse(texture).unwrap());
    for (index, &(from, to, uv, mut pivot, _)) in parts.iter().enumerate() {
        if index == pose.head {
            pivot[1] += pose.head_drop;
        }
        let uv_dimensions = if fur {
            Some(if index == 0 {
                [6.0, 6.0, 6.0]
            } else if index == 1 {
                [8.0, 16.0, 6.0]
            } else {
                [4.0, 6.0, 4.0]
            })
        } else {
            None
        };
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
            pose.parts[index],
            tint,
            [64.0, 32.0],
            uv_dimensions,
            mirror_right_legs && matches!(index, 2 | 4),
        );
    }
}
