//! Real-asset solo integration check; use CSR_WORLD_DIR for an isolated save.
use sim::{GameEvent, GameMode, SpawnPoint, Team, TickInput, World};
fn main() {
    let level = level::Level::load("overworld", None).expect("Overworld assets");
    assert!(
        level.nav.is_none(),
        "Special mode does not need bot navigation"
    );
    let runtime = level.overworld.as_ref().unwrap();
    for event in [
        "minecraft:block.grass.step",
        "minecraft:block.stone.break",
        "minecraft:entity.cow.ambient",
    ] {
        let (_, bytes) = runtime
            .lock()
            .unwrap()
            .sound_bytes(event, 0)
            .expect("Minecraft sound asset");
        let decoder = rodio::Decoder::builder()
            .with_byte_len(bytes.len() as u64)
            .with_data(std::io::Cursor::new(bytes))
            .build()
            .expect("Minecraft Vorbis decoder");
        assert!(decoder.take(1024).count() > 0);
    }
    {
        let mut r=runtime.lock().unwrap();
        let before=serde_json::to_value(&r.save).unwrap();
        let inventory=serde_json::to_value(&r.inventory).unwrap();
        let old=r.stream.view_distance();
        let epoch=r.epoch;
        for distance in [3,8,old] {r.set_render_distance(distance);assert_eq!(r.stream.view_distance(),distance);}
        assert!(r.epoch>epoch);
        assert_eq!(serde_json::to_value(&r.save).unwrap(),before);
        assert_eq!(serde_json::to_value(&r.inventory).unwrap(),inventory);
        println!("Live render distance preserves inventory, player and adventure state.");
    }
    let spawn = runtime.lock().unwrap().spawn_source();
    let mut world = World::with_catalog(level.collision.clone(), level.catalog.clone());
    world.spawns.push(SpawnPoint {
        origin: spawn,
        yaw: 0.,
        team: Team::CounterTerrorist,
    });
    world.set_mode(GameMode::Special, sim::RulesConfig::default());
    let id = world.add_player("You", Team::CounterTerrorist);
    let mut shots = 0;
    for tick in 0..64 * 20 {
        let input = TickInput {
            view_angles: glam::Vec3::new(0., (tick as f32 / 64.) * 20., 0.),
            forward_move: 30.,
            buttons: if tick % 8 == 0 {
                movement::buttons::ATTACK
            } else {
                0
            },
            ..Default::default()
        };
        world.step(&[(id, input)]);
        shots += world
            .events
            .iter()
            .filter(|e| matches!(e, GameEvent::Fire { .. }))
            .count();
        let p = world.player(id).unwrap();
        assert!(p.movement.origin.is_finite());
        assert!(p.movement.origin.z > spawn.z - 4000.);
        runtime
            .lock()
            .unwrap()
            .tick(sim::TICK_DT, p.movement.origin, p.alive);
    }
    assert!(shots > 0);
    assert_eq!(world.players.len(), 1);
    assert!(!world.players[0].is_bot);
    world.tick = 64 * 60 * 120;
    world.step(&[]);
    assert_eq!(world.mode, GameMode::Special);
    assert!(!world.competitive());
    world.damage_environment(id, 1000, "minecraft");
    assert!(!world.player(id).unwrap().alive);
    world.tick += 64 * 10;
    world.step(&[]);
    assert!(world.player(id).unwrap().alive);
    println!(
        "Overworld Special: one player, no CS bots, unlimited time, {shots} shots, creature damage/respawn and Minecraft audio passed; failures: 0"
    );
}
