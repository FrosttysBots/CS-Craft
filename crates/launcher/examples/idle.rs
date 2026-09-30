fn main() {
    let level = level::Level::load("de_dust2", None).unwrap();
    let mut world = sim::World::new(level.collision.clone());
    world.load_spawns(&level.bsp);
    let id = world.add_player("p", sim::Team::CounterTerrorist);
    for t in 0..200 {
        let p = world.player(id).unwrap();
        if t % 20 == 0 || t < 5 {
            println!("tick {t}: origin {:.2?} vel {:.2?} ground {} ducked {}", p.movement.origin, p.movement.velocity, p.movement.on_ground, p.movement.ducked);
        }
        let input = sim::TickInput { view_angles: glam::Vec3::new(0.0, 112.0, 0.0), weapon_max_speed: 250.0, ..Default::default() };
        world.step(&[(id, input)]);
    }
}
