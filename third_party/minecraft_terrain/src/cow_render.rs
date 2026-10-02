//! Pack-backed cow body mesh. Geometry/UVs are from pinned 26.3 CowModel;
//! the image is always resolved through the active resource-pack atlas.
use crate::{
    client_mobs::ClientMobs,
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh, Vertex},
    pack::ResourceId,
};
use glam::{DVec3, Quat, Vec3};
use minecraftoss_entities::{cow::CowVariant, mooshroom::MushroomVariant, world::CowEntity};

pub fn texture_id(variant: CowVariant, baby: bool) -> ResourceId {
    let kind = match variant {
        CowVariant::Temperate => "temperate",
        CowVariant::Warm => "warm",
        CowVariant::Cold => "cold",
    };
    ResourceId::parse(&format!(
        "minecraft:entity/cow/cow_{kind}{}",
        if baby { "_baby" } else { "" }
    ))
    .unwrap()
}

pub fn append_cows<'a>(
    mesh: &mut ChunkMesh,
    cows: impl IntoIterator<Item = &'a CowEntity>,
    poses: &ClientMobs,
    atlas: &Atlas,
    light: &SkyLight,
    partial: f32,
) {
    // Each mob's first vertex and overlay (`getOverlayCoords`).
    let mut marks = Vec::new();
    for entity in cows {
        let cow = &entity.cow;
        // Horses and donkeys have their own renderer.
        if entity.horse.is_some() {
            continue;
        }
        let Some(mob) = poses.pose(entity.id, partial) else { continue };
        marks.push((mesh.vertices.len(), mob.overlay(0.0)));
        let feet = mob.feet;
        let rotation = mob.body_rotation(90.0);
        // QuadrupedModel.setupAnim: the head (and all it carries) looks, the
        // legs swing.
        let head_turn = Quat::from_euler(glam::EulerRot::ZYX, 0.0, mob.head_yaw.to_radians(), mob.head_pitch.to_radians());
        let [right_hind, left_hind, right_front, left_front] = crate::client_mobs::quadruped_legs(mob.walk_position, mob.walk_speed);
        let leg = Quat::from_rotation_x;
        let body_turn = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        // A part of the head, given with its pivot and rotation flattened
        // into the model: moved and turned with the head.
        let on_head = |head: [f32; 3], pivot: [f32; 3], part: Quat| {
            let offset = head_turn * (Vec3::from_array(pivot) - Vec3::from_array(head));
            ((Vec3::from_array(head) + offset).to_array(), head_turn * part)
        };
        let sprite = if let Some(state) = &entity.mooshroom {
            let kind = match state.variant {
                MushroomVariant::Red => "red",
                MushroomVariant::Brown => "brown",
            };
            ResourceId::parse(&format!(
                "minecraft:entity/cow/mooshroom_{kind}{}",
                if cow.age.baby() { "_baby" } else { "" }
            ))
            .unwrap()
        } else {
            texture_id(cow.variant, cow.age.baby())
        };
        let region = atlas.entity_region(&sprite);
        let pos = mob.light_block();
        let sky = light.get(pos) as f32;
        let block = light.get_block(pos) as f32;
        let scale = 1.0;
        // `mirror()` cubes swap their side faces and flip their texture.
        let emit_mirrored = |mesh: &mut ChunkMesh, from: [f32; 3], to: [f32; 3], uv: [f32; 2], pivot: [f32; 3], part: Quat, mirror: bool| {
            cube_tinted_pose_mirror(mesh, feet, rotation, scale, region, sky, block, from, to, uv, pivot, part, [1.0; 3], [64.0; 2], None, mirror);
        };
        let emit = |mesh: &mut ChunkMesh, from: [f32; 3], to: [f32; 3], uv: [f32; 2], pivot: [f32; 3], part: Quat| {
            emit_mirrored(mesh, from, to, uv, pivot, part, false);
        };
        if cow.age.baby() {
            // LayerDefinitions maps all three COW_*_BABY layers to
            // BabyCowModel.createBodyLayer; only the texture varies.
            let baby_boxes = [
                (
                    [-3., -4.569, -4.8333],
                    [3., 1.431, 0.1667],
                    [0., 18.],
                    [0., 13.569, -5.1667],
                ),
                (
                    [3., -5.569, -3.8333],
                    [4., -3.569, -2.8333],
                    [8., 29.],
                    [0., 13.569, -5.1667],
                ),
                (
                    [-4., -5.569, -3.8333],
                    [-3., -3.569, -2.8333],
                    [4., 29.],
                    [0., 13.569, -5.1667],
                ),
                (
                    [-2., -1.569, -5.8333],
                    [2., 1.431, -4.8333],
                    [12., 29.],
                    [0., 13.569, -5.1667],
                ),
                ([-7., -7., -1.], [1., -1., 11.], [0., 0.], [3., 19., -5.]),
                (
                    [-1.5, 0., -1.5],
                    [1.5, 6., 1.5],
                    [22., 18.],
                    [-2.5, 18., -3.5],
                ),
                (
                    [-1.5, 0., -1.5],
                    [1.5, 6., 1.5],
                    [34., 18.],
                    [2.5, 18., -3.5],
                ),
                (
                    [-1.5, 0., -1.5],
                    [1.5, 6., 1.5],
                    [22., 27.],
                    [-2.5, 18., 3.5],
                ),
                (
                    [-1.5, 0., -1.5],
                    [1.5, 6., 1.5],
                    [34., 27.],
                    [2.5, 18., 3.5],
                ),
            ];
            // BabyCowModel: the head group, the body, then the right front,
            // left front, right hind and left hind legs.
            for (index, (from, to, uv, pivot)) in baby_boxes.into_iter().enumerate() {
                let part = match index {
                    0..=3 => head_turn,
                    5 => leg(right_front),
                    6 => leg(left_front),
                    7 => leg(right_hind),
                    8 => leg(left_hind),
                    _ => Quat::IDENTITY,
                };
                // Its left horn is the one mirrored cube.
                emit_mirrored(mesh, from, to, uv, pivot, part, index == 2);
            }
            continue;
        }
        // Native model coordinates are downward-positive with its feet at Y=24.
        // The adult layer is CowModel.createBaseCowModel (26.3).
        let boxes: [([f32; 3], [f32; 3], [f32; 2], [f32; 3], bool); 7] = [
            (
                [-4., -4., -6.],
                [4., 4., 0.],
                [0., 0.],
                [0., 4., -8.],
                false,
            ),
            (
                [-3., 1., -7.],
                [3., 4., -6.],
                [1., 33.],
                [0., 4., -8.],
                false,
            ),
            (
                [-6., -10., -7.],
                [6., 8., 3.],
                [18., 4.],
                [0., 5., 2.],
                true,
            ),
            ([-2., 2., -8.], [2., 8., -7.], [52., 0.], [0., 5., 2.], true),
            (
                [-2., 0., -2.],
                [2., 12., 2.],
                [0., 16.],
                [-4., 12., 7.],
                false,
            ),
            (
                [-2., 0., -2.],
                [2., 12., 2.],
                [0., 16.],
                [4., 12., 7.],
                false,
            ),
            (
                [-2., 0., -2.],
                [2., 12., 2.],
                [0., 16.],
                [-4., 12., -5.],
                false,
            ),
        ];
        // CowModel: head, nose, body, udder, then the right hind, left hind
        // and right front legs; the left front leg follows.
        for (index, (from, to, uv, pivot, _)) in boxes.into_iter().enumerate() {
            let part = match index {
                0 | 1 => head_turn,
                2 | 3 => body_turn,
                4 => leg(right_hind),
                5 => leg(left_hind),
                6 => leg(right_front),
                _ => Quat::IDENTITY,
            };
            // The left legs are mirrored.
            emit_mirrored(mesh, from, to, uv, pivot, part, index == 5);
        }
        emit_mirrored(mesh, [-2., 0., -2.], [2., 12., 2.], [0., 16.], [4., 12., -5.], leg(left_front), true);
        let head = [0., 4., -8.];
        // WarmCowModel mirrors its two left horn pieces.
        let horns: &[([f32; 3], [f32; 3], [f32; 2], bool)] = match cow.variant {
            CowVariant::Temperate => &[
                ([-5., -5., -5.], [-4., -2., -4.], [22., 0.], false),
                ([4., -5., -5.], [5., -2., -4.], [22., 0.], false),
            ],
            CowVariant::Warm => &[
                ([-8., -3., -5.], [-4., -1., -3.], [27., 0.], false),
                ([-8., -5., -5.], [-6., -3., -3.], [39., 0.], false),
                ([4., -3., -5.], [8., -1., -3.], [27., 0.], true),
                ([6., -5., -5.], [8., -3., -3.], [39., 0.], true),
            ],
            CowVariant::Cold => &[],
        };
        for &(from, to, uv, mirror) in horns {
            emit_mirrored(mesh, from, to, uv, head, head_turn, mirror);
        }
        if cow.variant == CowVariant::Cold {
            // ColdCowModel adds a half-pixel wool shell and two curved horns.
            cube_tinted_pose(
                mesh,
                feet,
                rotation,
                scale,
                region,
                sky,
                block,
                [-6.5, -10.5, -7.5],
                [6.5, 8.5, 3.5],
                [20., 32.],
                [0., 5., 2.],
                body_turn,
                [1.; 3],
                [64.; 2],
                Some([12., 18., 10.]),
            );
            for &(from, to, uv, pivot) in &[
                (
                    [-1.5, -4.5, -0.5],
                    [0.5, 1.5, 1.5],
                    [0., 40.],
                    [-4.5, 1.5, -11.5],
                ),
                (
                    [-1.5, -3., -0.5],
                    [0.5, 3., 1.5],
                    [0., 32.],
                    [5.5, 1.5, -13.],
                ),
            ] {
                let (pivot, part) = on_head(head, pivot, body_turn);
                emit(mesh, from, to, uv, pivot, part);
            }
        }
        if let Some(state) = &entity.mooshroom {
            // MushroomCowMushroomLayer renders three small cross-model
            // mushrooms from the variant's block state.
            let block_texture = ResourceId::parse(match state.variant {
                MushroomVariant::Red => "minecraft:block/red_mushroom",
                MushroomVariant::Brown => "minecraft:block/brown_mushroom",
            })
            .unwrap();
            let mushroom_region = atlas.region(&block_texture);
            // The third mushroom sits on the head and moves with it.
            for (index, pivot) in [[-4., 3., 4.], [4., 3., 4.], [0., 2., -9.]].into_iter().enumerate() {
                let pivot = if index == 2 { on_head(head, pivot, Quat::IDENTITY).0 } else { pivot };
                mushroom_cross(
                    mesh,
                    feet,
                    rotation,
                    1.0,
                    mushroom_region,
                    sky,
                    block,
                    pivot,
                );
            }
        }
    }
    crate::cow_render::apply_overlays(mesh, &marks);
}

/// A model point (pixels, y down) in the entity's frame before its body
/// turn: `LivingEntityRenderer`'s `scale(-1, -1, 1)`, the renderer's own
/// scale, then `translate(0, -1.501, 0)`.
/// `ItemInHandLayer.submitArmWithItem`'s pose for a right hand: the mob's
/// frame (turned, flipped by `scale(-1, -1, 1)` and lowered 1.501), the arm
/// (`translateToHand`: its pivot in pixels, then its rotation), a turn
/// about X by -90 and Y by 180 degrees, then out to the hand (a baby's
/// offset is shorter). The item's display transform follows.
pub fn right_hand_pose(feet: DVec3, rotation: Quat, arm_pivot: Vec3, arm: Quat, baby: bool) -> glam::Mat4 {
    use glam::Mat4;
    let offset = if baby { Vec3::new(0.0, 1.0, -4.5) } else { Vec3::new(1.0, 2.0, -10.0) } / 16.0;
    Mat4::from_translation(feet.as_vec3())
        * Mat4::from_quat(rotation)
        * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
        * Mat4::from_translation(Vec3::new(0.0, -1.501, 0.0))
        * Mat4::from_translation(arm_pivot / 16.0)
        * Mat4::from_quat(arm)
        * Mat4::from_rotation_x((-90.0f32).to_radians())
        * Mat4::from_rotation_y(180.0f32.to_radians())
        * Mat4::from_translation(offset)
}

/// Marks each mob's vertices, from its first to the next mob's, with its
/// overlay ([`crate::client_mobs::MobPose::overlay`]), which `fs_entity`
/// reads from their alpha.
pub fn apply_overlays(mesh: &mut ChunkMesh, marks: &[(usize, f32)]) {
    for (i, &(start, overlay)) in marks.iter().enumerate() {
        let end = marks.get(i + 1).map_or(mesh.vertices.len(), |&(next, _)| next);
        for vertex in &mut mesh.vertices[start..end] {
            vertex.color[3] = overlay;
        }
    }
}

/// Whether entities are lit by the Nether's second light (from below).
static NETHER_LIGHTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `Lighting.setupLevel`/`setupNetherLevel`, by dimension.
pub fn set_nether_entity_lighting(nether: bool) {
    NETHER_LIGHTING.store(nether, std::sync::atomic::Ordering::Relaxed);
}

/// The entity shader's `minecraft_mix_light` for a world-space normal:
/// two fixed lights, 0.6 of their sum plus 0.4 ambient, at most 1.
pub fn entity_shade(normal: Vec3) -> f32 {
    let light0 = Vec3::new(0.2, 1.0, -0.7).normalize();
    let light1 = if NETHER_LIGHTING.load(std::sync::atomic::Ordering::Relaxed) {
        Vec3::new(-0.2, -1.0, 0.7).normalize()
    } else {
        Vec3::new(-0.2, 1.0, 0.7).normalize()
    };
    let light = light0.dot(normal).max(0.0) + light1.dot(normal).max(0.0);
    (light * 0.6 + 0.4).min(1.0)
}

fn model_to_entity(p: Vec3, scale: Vec3) -> Vec3 {
    Vec3::new(-p.x, 24.016 - p.y, p.z) * scale / 16.0
}

#[allow(clippy::too_many_arguments)]
fn mushroom_cross(
    mesh: &mut ChunkMesh,
    feet: DVec3,
    rotation: Quat,
    scale: f32,
    region: [f32; 4],
    sky: f32,
    block: f32,
    pivot: [f32; 3],
) {
    let half = 4.0;
    for (dx, dz) in [(1.0, 1.0), (1.0, -1.0)] {
        let corners = [
            [-half * dx, half, -half * dz],
            [half * dx, half, half * dz],
            [half * dx, -half, half * dz],
            [-half * dx, -half, -half * dz],
        ];
        let start = mesh.vertices.len() as u32;
        for (corner, uv) in corners.into_iter().zip([
            [region[0], region[3]],
            [region[2], region[3]],
            [region[2], region[1]],
            [region[0], region[1]],
        ]) {
            let p = Vec3::from_array(corner) + Vec3::from_array(pivot);
            let local = model_to_entity(p, Vec3::splat(scale));
            let world = feet.as_vec3() + rotation * local;
            mesh.vertices.push(Vertex {
                position: world.to_array(),
                uv,
                color: [1.0, 1.0, 1.0, 1.0],
                sky_light: sky,
                block_light: block,
            });
        }
        mesh.indices.extend_from_slice(&[
            start,
            start + 1,
            start + 2,
            start,
            start + 2,
            start + 3,
            start + 2,
            start + 1,
            start,
            start + 3,
            start + 2,
            start,
        ]);
        mesh.faces += 2;
    }
}


/// A cuboid with a flag for its mirror, for the cuboid tests.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn cube_tinted(
    mesh: &mut ChunkMesh,
    feet: DVec3,
    rotation: Quat,
    scale: f32,
    region: [f32; 4],
    sky: f32,
    block: f32,
    from: [f32; 3],
    to: [f32; 3],
    uv: [f32; 2],
    pivot: [f32; 3],
    rotate_body: bool,
    tint: [f32; 3],
    texture_size: [f32; 2],
    uv_dimensions: Option<[f32; 3]>,
) {
    cube_tinted_pose(
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
        if rotate_body {
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)
        } else {
            Quat::IDENTITY
        },
        tint,
        texture_size,
        uv_dimensions,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn cube_tinted_pose(
    mesh: &mut ChunkMesh,
    feet: DVec3,
    rotation: Quat,
    scale: f32,
    region: [f32; 4],
    sky: f32,
    block: f32,
    from: [f32; 3],
    to: [f32; 3],
    uv: [f32; 2],
    pivot: [f32; 3],
    part_rotation: Quat,
    tint: [f32; 3],
    texture_size: [f32; 2],
    uv_dimensions: Option<[f32; 3]>,
) {
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
        tint,
        texture_size,
        uv_dimensions,
        false,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cube_tinted_pose_mirror(
    mesh: &mut ChunkMesh,
    feet: DVec3,
    rotation: Quat,
    scale: f32,
    region: [f32; 4],
    sky: f32,
    block: f32,
    from: [f32; 3],
    to: [f32; 3],
    uv: [f32; 2],
    pivot: [f32; 3],
    part_rotation: Quat,
    tint: [f32; 3],
    texture_size: [f32; 2],
    uv_dimensions: Option<[f32; 3]>,
    mirror: bool,
) {
    cube_scaled(mesh, feet, rotation, Vec3::splat(scale), region, sky, block, from, to, uv, pivot, part_rotation, tint, texture_size, uv_dimensions, mirror);
}

/// A model cuboid with a per-axis model scale about the feet (a renderer's
/// `scale` step, as `CreeperRenderer` swells).
#[allow(clippy::too_many_arguments)]
pub(crate) fn cube_scaled(
    mesh: &mut ChunkMesh,
    feet: DVec3,
    rotation: Quat,
    scale: Vec3,
    region: [f32; 4],
    sky: f32,
    block: f32,
    from: [f32; 3],
    to: [f32; 3],
    uv: [f32; 2],
    pivot: [f32; 3],
    part_rotation: Quat,
    tint: [f32; 3],
    texture_size: [f32; 2],
    uv_dimensions: Option<[f32; 3]>,
    mirror: bool,
) {
    let [x0, y0, z0] = from;
    let [x1, y1, z1] = to;
    let (x_lo, x_hi) = if mirror { (x1, x0) } else { (x0, x1) };
    let (w, h, d) = (x1 - x0, y1 - y0, z1 - z0);
    let [uw, uh, ud] = uv_dimensions.unwrap_or([w, h, d]);
    let (u, v) = (uv[0], uv[1]);
    // Pinned 26.3 ModelPart.Cube polygon order: DOWN, UP, WEST, NORTH,
    // EAST, SOUTH. Its Polygon maps vertex UVs as (u1,v0), (u0,v0),
    // (u0,v1), (u1,v1).
    let u0 = u;
    let u1 = u + ud;
    let u2 = u1 + uw;
    let u22 = u2 + uw;
    let u3 = u2 + ud;
    let u4 = u3 + uw;
    let v0 = v;
    let v1 = v + ud;
    let v2 = v1 + uh;
    let faces = [
        (
            [
                [x_hi, y0, z1],
                [x_lo, y0, z1],
                [x_lo, y0, z0],
                [x_hi, y0, z0],
            ],
            [u1, v0, u2, v1],
            [0.0, -1.0, 0.0],
        ),
        (
            [
                [x_hi, y1, z0],
                [x_lo, y1, z0],
                [x_lo, y1, z1],
                [x_hi, y1, z1],
            ],
            [u2, v1, u22, v0],
            [0.0, 1.0, 0.0],
        ),
        (
            [
                [x_lo, y0, z0],
                [x_lo, y0, z1],
                [x_lo, y1, z1],
                [x_lo, y1, z0],
            ],
            [u0, v1, u1, v2],
            [if mirror { 1.0 } else { -1.0 }, 0.0, 0.0],
        ),
        (
            [
                [x_hi, y0, z0],
                [x_lo, y0, z0],
                [x_lo, y1, z0],
                [x_hi, y1, z0],
            ],
            [u1, v1, u2, v2],
            [0.0, 0.0, -1.0],
        ),
        (
            [
                [x_hi, y0, z1],
                [x_hi, y0, z0],
                [x_hi, y1, z0],
                [x_hi, y1, z1],
            ],
            [u2, v1, u3, v2],
            [if mirror { -1.0 } else { 1.0 }, 0.0, 0.0],
        ),
        (
            [
                [x_lo, y0, z1],
                [x_hi, y0, z1],
                [x_hi, y1, z1],
                [x_lo, y1, z1],
            ],
            [u3, v1, u4, v2],
            [0.0, 0.0, 1.0],
        ),
    ];
    for (corners, tex, normal) in faces {
        // The face's normal as the vertices move: part pose, the model's
        // (-1, -1, 1) flip and scale, then the body's rotation.
        let n = part_rotation * Vec3::from_array(normal);
        let n = rotation * (Vec3::new(-n.x, -n.y, n.z) / scale).normalize();
        let shade = entity_shade(n);
        let start = mesh.vertices.len() as u32;
        let mut vertices = std::array::from_fn::<_, 4, _>(|i| {
            (
                corners[i],
                [
                    [tex[2], tex[1]],
                    [tex[0], tex[1]],
                    [tex[0], tex[3]],
                    [tex[2], tex[3]],
                ][i],
            )
        });
        // ModelPart.Polygon reverses the already UV-mapped vertices after
        // Cube mirrors its X coordinates. This preserves outward normals and
        // swaps each face's skin orientation exactly as the pinned client.
        if mirror {
            vertices.reverse();
        }
        for (corner, coord) in vertices {
            let p = part_rotation * Vec3::from_array(corner) + Vec3::from_array(pivot);
            let local = model_to_entity(p, scale);
            let world = feet.as_vec3() + rotation * local;
            mesh.vertices.push(Vertex {
                position: world.to_array(),
                uv: [
                    region[0] + (region[2] - region[0]) * coord[0] / texture_size[0],
                    region[1] + (region[3] - region[1]) * coord[1] / texture_size[1],
                ],
                color: [shade * tint[0], shade * tint[1], shade * tint[2], 1.0],
                sky_light: sky,
                block_light: block,
            });
        }
        mesh.indices
            .extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
        mesh.faces += 1;
    }
}

#[cfg(test)]
mod cuboid_tests {
    use super::*;

    #[test]
    fn every_face_points_outward_and_uses_pinned_modelpart_uvs() {
        for rotation in [Quat::IDENTITY, Quat::from_rotation_y(std::f32::consts::PI)] {
            let mut mesh = ChunkMesh::default();
            cube_tinted(
                &mut mesh,
                DVec3::ZERO,
                rotation,
                1.0,
                [0.0, 0.0, 1.0, 1.0],
                15.0,
                0.0,
                [-4.0, 0.0, -3.0],
                [4.0, 12.0, 3.0],
                [16.0, 20.0],
                [0.0; 3],
                false,
                [1.0; 3],
                [64.0; 2],
                None,
            );
            assert_eq!(mesh.faces, 6);
            let center = rotation * Vec3::new(0.0, 18.0 / 16.0, 0.0);
            for face in 0..6 {
                let i = face * 6;
                let a = Vec3::from_array(mesh.vertices[mesh.indices[i] as usize].position);
                let b = Vec3::from_array(mesh.vertices[mesh.indices[i + 1] as usize].position);
                let c = Vec3::from_array(mesh.vertices[mesh.indices[i + 2] as usize].position);
                let face_center = (0..4)
                    .map(|corner| Vec3::from_array(mesh.vertices[face * 4 + corner].position))
                    .sum::<Vec3>()
                    / 4.0;
                assert!(
                    (b - a).cross(c - a).dot(face_center - center) > 0.0,
                    "face {face}"
                );
            }
            // NORTH's first vertex is ModelPart.Cube's t1, mapped to (u2,v1).
            assert_eq!(mesh.vertices[12].uv, [30.0 / 64.0, 26.0 / 64.0]);
            // SOUTH's first vertex is l0, mapped to (u4,v1).
            assert_eq!(mesh.vertices[20].uv, [44.0 / 64.0, 26.0 / 64.0]);
        }
    }

    #[test]
    fn inflated_cube_keeps_the_original_texture_panels() {
        let mut base = ChunkMesh::default();
        let mut inflated = ChunkMesh::default();
        for (mesh, from, to, uv_dimensions) in [
            (&mut base, [-4., -10., -4.], [4., 0., 4.], None),
            (
                &mut inflated,
                [-4.51, -10.51, -4.51],
                [4.51, 0.51, 4.51],
                Some([8., 10., 8.]),
            ),
        ] {
            cube_tinted(
                mesh,
                DVec3::ZERO,
                Quat::IDENTITY,
                1.0,
                [0., 0., 1., 1.],
                15.,
                0.,
                from,
                to,
                [32., 0.],
                [0.; 3],
                false,
                [1.; 3],
                [64.; 2],
                uv_dimensions,
            );
        }
        assert_eq!(
            base.vertices.iter().map(|v| v.uv).collect::<Vec<_>>(),
            inflated.vertices.iter().map(|v| v.uv).collect::<Vec<_>>()
        );
        assert_ne!(base.vertices[0].position, inflated.vertices[0].position);
    }

    #[test]
    fn posed_cube_keeps_pivot_and_texture_coordinates() {
        let mut unposed = ChunkMesh::default();
        let mut posed = ChunkMesh::default();
        let from = [-2., -2., -1.];
        let to = [2., 6., 1.];
        let pivot = [0., 3., -1.];
        cube_tinted(
            &mut unposed,
            DVec3::ZERO,
            Quat::IDENTITY,
            1.,
            [0., 0., 1., 1.],
            15.,
            0.,
            from,
            to,
            [40., 38.],
            pivot,
            false,
            [1.; 3],
            [64.; 2],
            None,
        );
        cube_tinted_pose(
            &mut posed,
            DVec3::ZERO,
            Quat::IDENTITY,
            1.,
            [0., 0., 1., 1.],
            15.,
            0.,
            from,
            to,
            [40., 38.],
            pivot,
            Quat::from_rotation_x(-0.75),
            [1.; 3],
            [64.; 2],
            None,
        );
        assert_eq!(unposed.indices, posed.indices);
        assert_eq!(
            unposed.vertices.iter().map(|v| v.uv).collect::<Vec<_>>(),
            posed.vertices.iter().map(|v| v.uv).collect::<Vec<_>>()
        );
        assert_ne!(unposed.vertices[0].position, posed.vertices[0].position);
        let source_center = (Vec3::from_array(from) + Vec3::from_array(to)) * 0.5;
        let expected_center =
            Quat::from_rotation_x(-0.75) * source_center + Vec3::from_array(pivot);
        // `scale(-1, -1, 1)` and the 1.501 lift.
        let expected_world = Vec3::new(
            -expected_center.x,
            24.016 - expected_center.y,
            expected_center.z,
        ) / 16.;
        let actual_center = posed.vertices[0..24]
            .iter()
            .map(|vertex| Vec3::from_array(vertex.position))
            .sum::<Vec3>()
            / 24.;
        assert!((actual_center - expected_world).length() < 1e-5);
    }

    #[test]
    fn mirrored_cube_reverses_mapped_vertices_and_keeps_outward_faces() {
        let mut mesh = ChunkMesh::default();
        cube_tinted_pose_mirror(
            &mut mesh,
            DVec3::ZERO,
            Quat::IDENTITY,
            1.0,
            [0., 0., 1., 1.],
            15.,
            0.,
            [-1., -2., -1.],
            [1., 10., 1.],
            [40., 16.],
            [0.; 3],
            Quat::IDENTITY,
            [1.; 3],
            [64., 32.],
            None,
            true,
        );
        // NORTH's first vertex is the source polygon's reversed t2:
        // left-bottom model corner and (u2,v2) after the X mirror.
        assert_eq!(mesh.vertices[12].uv, [44. / 64., 30. / 32.]);
        // The renderer's `scale(-1, -1, 1)` puts the model's -x corner at +x.
        assert_eq!(
            mesh.vertices[12].position,
            [1. / 16., (24.016 - 10.) / 16., -1. / 16.]
        );
        let center = Vec3::new(0., 20. / 16., 0.);
        for face in 0..6 {
            let index = face * 6;
            let a = Vec3::from_array(mesh.vertices[mesh.indices[index] as usize].position);
            let b = Vec3::from_array(mesh.vertices[mesh.indices[index + 1] as usize].position);
            let c = Vec3::from_array(mesh.vertices[mesh.indices[index + 2] as usize].position);
            let face_center = (0..4)
                .map(|corner| Vec3::from_array(mesh.vertices[face * 4 + corner].position))
                .sum::<Vec3>()
                / 4.0;
            assert!(
                (b - a).cross(c - a).dot(face_center - center) > 0.0,
                "face {face}"
            );
        }
    }
}
