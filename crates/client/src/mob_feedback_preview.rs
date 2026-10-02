//! Deterministic screenshot-only fixture: healthy and hurt cows side by side.
use super::*;
pub(super) fn append(
    runtime: &mut overworld::Runtime,
    mesh: &mut terrain::mesh::ChunkMesh,
    camera: &terrain::sections::CullCamera,
    frame: u32,
) {
    use overworld::mc_entities::{
        cow::Cow,
        world::{EntityWorld, MobHit, PlayerAttack},
    };
    use terrain::{
        client_mobs::{server_mobs, ClientMobs},
        mob_actions::{attack, Actor},
    };
    let forward = camera.forward.as_dvec3().normalize_or_zero();
    let right = forward.cross(glam_mc::DVec3::Y).normalize_or_zero();
    let base = camera.position + forward * 2.8 - glam_mc::DVec3::Y * 1.2;
    let mut world = EntityWorld::default();
    world.spawn_cow(Cow::new(base - right * 0.8), true);
    let id = world.spawn_cow(Cow::new(base + right * 0.8), true);
    let mut poses = ClientMobs::default();
    poses.receive(server_mobs(&world));
    if (235..245).contains(&frame) {
        let mut inventory = overworld::mc_player::inventory::Inventory::default();
        let mut actor = Actor {
            inventory: &mut inventory,
            selected: 0,
            infinite: false,
            entity_loot: None,
            shearing_loot: None,
        };
        let hit = PlayerAttack {
            player_id: 1,
            position: camera.position,
            yaw: 0.,
            attack_damage: 1.,
            strength: 1.,
            sprinting: false,
            can_critical: false,
            can_sweep: false,
        };
        assert!(attack(&mut world, MobHit::Cow(id), &mut actor, &hit).hurt);
        poses.receive(server_mobs(&world));
        if frame == 235 {
            let point = base + right * 0.8 + glam_mc::DVec3::Y * 0.7 - forward * 0.45;
            runtime.mob_hits.push(overworld::MobHitFx {
                point: overworld::to_source(runtime.save.origin, point.to_array()),
                from: overworld::to_source(runtime.save.origin, camera.position.to_array()),
            });
        }
    }
    terrain::cow_render::append_cows(
        mesh,
        world.cows(),
        &poses,
        &runtime.atlas,
        &runtime.light,
        0.,
    );
}
