//! First-person item models and depth-tested block selection/mining feedback.
use super::*;
use terrain::{
    mesh::{ChunkMesh, HeldDisplay, HeldItem, SectionVertex, Vertex},
    pack::ResourceId,
};

#[derive(Resource, Default)]
pub(super) struct Feedback {
    held: Option<(Entity, Handle<Mesh>)>,
    outline: Option<(Entity, Handle<Mesh>)>,
    cracks: Option<(Entity, Handle<Mesh>)>,
    held_id: Option<String>,
    target: String,
    stage: i32,
    equip: f32,
    swing: f32,
    was_active: bool,
}
fn retire(
    slot: &mut Option<(Entity, Handle<Mesh>)>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
) {
    if let Some((e, h)) = slot.take() {
        commands.entity(e).despawn();
        meshes.remove(h.id());
    }
}
fn spawn_model(
    data: ChunkMesh,
    origin: [f64; 3],
    material: Handle<VoxelMaterial>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    first_person: bool,
) -> (Entity, Handle<Mesh>) {
    let vertices: Vec<_> = data
        .vertices
        .iter()
        .map(SectionVertex::from_vertex)
        .collect();
    let handle = meshes.add(mesh(&vertices, data.indices, origin));
    let mut entity = commands.spawn((
        Mesh3d(handle.clone()),
        MeshMaterial3d(material),
        Transform::IDENTITY,
    ));
    if first_person {
        entity.insert(crate::viewmodel::layer());
    }
    (entity.id(), handle)
}
fn box_mesh(out: &mut ChunkMesh, min: [f32; 3], max: [f32; 3], uv: [f32; 4], color: [f32; 4]) {
    let [x, y, z] = min;
    let [a, b, c] = max;
    for quad in [
        [[x, y, z], [x, y, c], [x, b, c], [x, b, z]],
        [[a, y, c], [a, y, z], [a, b, z], [a, b, c]],
        [[x, y, c], [x, y, z], [a, y, z], [a, y, c]],
        [[x, b, z], [x, b, c], [a, b, c], [a, b, z]],
        [[a, y, z], [x, y, z], [x, b, z], [a, b, z]],
        [[x, y, c], [a, y, c], [a, b, c], [x, b, c]],
    ] {
        let start = out.vertices.len() as u32;
        for (position, uv) in quad.into_iter().zip([
            [uv[0], uv[3]],
            [uv[2], uv[3]],
            [uv[2], uv[1]],
            [uv[0], uv[1]],
        ]) {
            out.vertices.push(Vertex {
                position,
                uv,
                color,
                sky_light: 15.,
                block_light: 0.,
            });
        }
        out.indices
            .extend([start, start + 1, start + 2, start, start + 2, start + 3]);
        out.faces += 1;
    }
}
fn wire_box(out: &mut ChunkMesh, min: [f32; 3], max: [f32; 3], uv: [f32; 4]) {
    let width = 0.003;
    for axis in 0..3 {
        let a = (axis + 1) % 3;
        let b = (axis + 2) % 3;
        for x in [min[a], max[a]] {
            for y in [min[b], max[b]] {
                let mut lo = min;
                let mut hi = max;
                lo[a] = x - width;
                hi[a] = x + width;
                lo[b] = y - width;
                hi[b] = y + width;
                box_mesh(out, lo, hi, uv, [0., 0., 0., 1.]);
            }
        }
    }
}
fn held_mesh(runtime: &mut overworld::Runtime, id: &str) -> ChunkMesh {
    let mut data = ChunkMesh::default();
    // Pack models include both block meshes and extruded, alpha-cut item sprites.
    if !id.is_empty() {
        let item = HeldItem {
            id: id.into(),
            display: HeldDisplay::FirstPerson,
            first_tint: None,
            light: glam_mc::Vec3::from_array(runtime.save.player.map(|x| x as f32)),
            pose: glam_mc::Mat4::from_translation(glam_mc::Vec3::new(0.34, -0.24, -1.05))
                * glam_mc::Mat4::from_rotation_y(45_f32.to_radians())
                * glam_mc::Mat4::from_scale(glam_mc::Vec3::splat(0.65)),
        };
        if let Err(e) = runtime.item_visuals.append_held_items(
            &mut data,
            &[item],
            &runtime.packs,
            &runtime.atlas,
            &runtime.light,
        ) {
            warn!("Held item {id}: {e}");
        }
    }
    // Native player-skin right arm, reaching from the lower right to the grip.
    let start = data.vertices.len();
    terrain::cow_render::cube_tinted_pose(
        &mut data,
        glam_mc::DVec3::ZERO,
        glam_mc::Quat::IDENTITY,
        1.,
        runtime
            .atlas
            .entity_region(&ResourceId::parse("minecraft:entity/player/wide/steve").unwrap()),
        15.,
        0.,
        [-2., 0., -2.],
        [2., 12., 2.],
        [40., 16.],
        [0., 24., 0.],
        glam_mc::Quat::IDENTITY,
        [1.; 3],
        [64.; 2],
        None,
    );
    let arm = glam_mc::Mat4::from_translation(glam_mc::Vec3::new(0.39, -0.34, -0.95))
        * glam_mc::Mat4::from_rotation_x(-0.80)
        * glam_mc::Mat4::from_rotation_z(0.25);
    for v in &mut data.vertices[start..] {
        v.position = arm
            .transform_point3(
                glam_mc::Vec3::new(-v.position[0], -0.75 - v.position[1], v.position[2]) * 0.8,
            )
            .to_array();
    }
    data
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update(
    mut commands: Commands,
    level: Res<LoadedLevel>,
    local: Res<LocalPlayer>,
    world: Res<SimWorld>,
    building: Res<Building>,
    front: Res<crate::menu::Menu>,
    buy: Res<crate::buymenu::BuyMenu>,
    render: Res<RenderWorld>,
    mut feedback: ResMut<Feedback>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut transforms: Query<(&mut Transform, &mut Visibility)>,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    settings: Res<crate::settings::Settings>,
) {
    let Some(runtime) = &level.0.overworld else {
        return;
    };
    if !render.initialized {
        return;
    }
    let mut runtime = runtime.lock().unwrap();
    let alive = world.0.player(local.id).is_some_and(|p| p.alive);
    let active = building.active && alive && !local.noclip && !front.open();
    let interacting = active && !building.inventory_open && !buy.open && !front.open();
    let id = runtime.inventory.slots[building.selected.min(8)]
        .as_ref()
        .map_or(String::new(), |s| s.id.clone());
    if active && (feedback.held_id.as_ref() != Some(&id) || feedback.held.is_none()) {
        retire(&mut feedback.held, &mut commands, &mut meshes);
        let data = held_mesh(&mut runtime, &id);
        feedback.held = Some(spawn_model(
            data,
            [0.; 3],
            render.solid.clone(),
            &mut commands,
            &mut meshes,
            true,
        ));
        feedback.held_id = Some(id);
        feedback.equip = 1.;
    }
    if active && !feedback.was_active {
        feedback.equip = 1.;
    }
    feedback.was_active = active;
    let dt = time.delta_secs().min(0.1);
    feedback.equip = (feedback.equip - dt * 4.).max(0.);
    let swinging =
        interacting && (mouse.pressed(MouseButton::Left) || mouse.pressed(MouseButton::Right));
    feedback.swing = if swinging {
        (feedback.swing + dt * 2.7) % 1.
    } else {
        0.
    };
    let swing = (feedback.swing * std::f32::consts::PI).sin();
    if let Some((e, _)) = feedback.held.as_ref() {
        if let Ok((mut tr, mut vis)) = transforms.get_mut(*e) {
            *vis = if active {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            let fwd = crate::coords::view_forward(local.render_angles.x, local.render_angles.y);
            let camera = Transform::from_translation(crate::coords::to_bevy(local.render_eye))
                .looking_to(fwd, Vec3::Y);
            let bob = world
                .0
                .player(local.id)
                .map_or(0., |p| p.movement.velocity.length().min(180.) / 180.)
                * (time.elapsed_secs() * 8.).sin()
                * if settings.mc_hand_motion {0.008} else {0.};
            let offset = Vec3::new(
                -swing * 0.10,
                -feedback.equip * 0.5 + bob + swing * 0.08,
                -swing * 0.04,
            ) * overworld::BLOCK;
            *tr = Transform {
                translation: camera.translation + camera.rotation * offset,
                rotation: camera.rotation * Quat::from_rotation_x(-swing * 0.25),
                ..default()
            };
        }
    }
    let target = if interacting {
        let eye = overworld::to_block(runtime.save.origin, local.render_eye);
        let mut player = overworld::mc_player::Player::new(
            glam_mc::DVec3::from_array(eye) - glam_mc::DVec3::Y * 1.62,
        );
        player.yaw = (-local.render_angles.y - 90.) as f64;
        player.pitch = local.render_angles.x as f64;
        let scene = runtime.collision.scene.read().unwrap();
        player.target(&*scene, 5.).map(|hit| {
            (
                hit.pos,
                overworld::mc_player::selection_boxes(&*scene, hit.pos),
            )
        })
    } else {
        None
    };
    let key = format!("{}:{target:?}", runtime.save.adventure.dimension);
    let stage = target
        .as_ref()
        .filter(|(pos, _)| runtime.mining.target() == Some(*pos) && runtime.mining.progress() > 0.)
        .map_or(-1, |_| {
            (runtime.mining.progress() * 10.).floor().clamp(0., 9.) as i32
        });
    if key != feedback.target || stage != feedback.stage {
        retire(&mut feedback.outline, &mut commands, &mut meshes);
        retire(&mut feedback.cracks, &mut commands, &mut meshes);
        feedback.target = key;
        feedback.stage = stage;
        if let Some((pos, shapes)) = target {
            let mut outline = ChunkMesh::default();
            let mut cracks = ChunkMesh::default();
            let stone = runtime
                .atlas
                .region(&ResourceId::parse("minecraft:block/stone").unwrap());
            let crack = runtime.atlas.region(
                &ResourceId::parse(&format!("minecraft:block/destroy_stage_{}", stage.max(0)))
                    .unwrap(),
            );
            for (min, max) in shapes {
                let base = [pos.0 as f32, pos.1 as f32, pos.2 as f32];
                let lo = std::array::from_fn(|i| base[i] + min[i] as f32 - 0.002);
                let hi = std::array::from_fn(|i| base[i] + max[i] as f32 + 0.002);
                wire_box(&mut outline, lo, hi, stone);
                if stage >= 0 {
                    box_mesh(&mut cracks, lo, hi, crack, [1.; 4]);
                }
            }
            feedback.outline = Some(spawn_model(
                outline,
                runtime.save.origin,
                render.solid.clone(),
                &mut commands,
                &mut meshes,
                false,
            ));
            if stage >= 0 {
                feedback.cracks = Some(spawn_model(
                    cracks,
                    runtime.save.origin,
                    render.cracks.clone(),
                    &mut commands,
                    &mut meshes,
                    false,
                ));
            }
        }
    }
}
