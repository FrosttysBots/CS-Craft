use collision::{DynamicWorld, contents};
use glam::Vec3;
use overworld::{
    Runtime, glam_mc,
    terrain::{
        scene::{Block, Scene},
        sections::CullCamera,
    },
};
use std::time::{Duration, Instant};
fn main() -> Result<(), String> {
    let mut world = Runtime::load()?;
    let spawn = world.spawn_source();
    let origin = world.save.origin;
    let down = world.collision.trace_box(
        spawn,
        spawn - Vec3::Z * 256.,
        Vec3::new(-16., -16., 0.),
        Vec3::new(16., 16., 72.),
        contents::MASK_PLAYERSOLID,
    );
    assert!(
        down.hit() && !down.start_solid,
        "spawn should stand over solid ground: {down:?}"
    );
    let pos = overworld::block_pos(overworld::to_block(
        origin,
        spawn + Vec3::new(144., 0., 144.),
    ));
    let original = {
        let s = world.collision.scene.read().unwrap();
        Scene::block(&*s, pos).cloned()
    };
    assert!(world.set_block(pos, Some(Block::new("minecraft:stone"))));
    let center = overworld::to_source(
        origin,
        [pos.0 as f64 + 0.5, pos.1 as f64 + 0.5, pos.2 as f64 + 0.5],
    );
    let hit = world.collision.trace_box(
        center - Vec3::X * 72.,
        center + Vec3::X * 72.,
        Vec3::ZERO,
        Vec3::ZERO,
        contents::MASK_SHOT,
    );
    assert!(hit.hit(), "placed block should immediately stop bullets");
    assert!(world.mine(pos, 1000.));
    {
        let s = world.collision.scene.read().unwrap();
        assert!(
            Scene::block(&*s, pos).is_none(),
            "mined block should be removed"
        );
    }
    world.set_block(pos, Some(Block::new("minecraft:glass")));
    world.save_now()?;
    let seed = world.save.seed;
    let p = overworld::to_block(origin, spawn + Vec3::Z * 64.);
    let camera = CullCamera {
        position: glam_mc::DVec3::from_array(p),
        forward: glam_mc::Vec3::NEG_Z,
        fov_degrees: 74.,
        aspect: 16. / 9.,
        yaw_degrees: 0.,
        pitch_degrees: 0.,
    };
    let now = Instant::now();
    let (mut uploads, mut vertices) = (0, 0);
    while now.elapsed() < Duration::from_secs(20) {
        world.tick(0.05, spawn, true);
        let out = world.frame(&camera);
        uploads += out.uploads.len();
        vertices += out
            .uploads
            .iter()
            .map(|(_, m)| m.vertices.len())
            .sum::<usize>();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(vertices > 0, "terrain should produce visible meshes");
    println!(
        "seed={seed}, uploads={uploads}, vertices={vertices}, mobs={}",
        world.mobs.len()
    );
    // Reproduce shutdown after the last acknowledgement was already consumed.
    world.server.wait_idle();
    world.tick(0.0,spawn,true);
    world.server.wait_idle();
    drop(world);
    let mut reloaded = Runtime::load()?;
    {
        let scene = reloaded.collision.scene.read().unwrap();
        assert_eq!(
            Scene::block(&*scene, pos).map(|b| b.id.path.as_str()),
            Some("glass"),
            "world edits must survive restart"
        );
    }
    reloaded.set_block(pos, original);
    reloaded.save_now()?;
    println!(
        "Overworld check: collision, mining, placement, meshing, save/reload passed; failures: 0"
    );
    Ok(())
}
