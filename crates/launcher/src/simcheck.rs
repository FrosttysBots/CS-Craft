//! `csr sim <map>`: headless movement run against the real map collision.
//!
//! Spawns one player per team, then drives scripted inputs and checks that
//! players stay on the map (never fall through the world or get stuck in
//! solids). Exits non-zero on failure.

use glam::Vec3;
use movement::buttons;
use sim::{PlayerId, Team, TickInput, World, TICK_DT, TICK_RATE};
use std::time::Instant;

pub fn run(map: &str) -> i32 {
    let t = Instant::now();
    let level = match level::Level::load(map, None) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    println!("loaded {map} in {:.2}s {:?}", t.elapsed().as_secs_f32(), level.timings);
    let floor_z = level.collision.mins.z;

    let mut world = World::new(level.collision.clone());
    world.load_spawns(&level.bsp);
    println!("spawns: {}", world.spawns.len());
    if world.spawns.is_empty() {
        eprintln!("error: map has no spawn points");
        return 1;
    }
    let t_id = world.add_player("terrorist", Team::Terrorist);
    let ct_id = world.add_player("counter-terrorist", Team::CounterTerrorist);

    let mut failures = 0;
    let mut step = |world: &mut World, inputs: &[(PlayerId, TickInput)], ticks: usize| {
        for _ in 0..ticks {
            world.step(inputs);
        }
    };

    // Settle.
    step(&mut world, &[], TICK_RATE as usize / 2);
    for id in [t_id, ct_id] {
        let p = world.player(id).unwrap();
        println!(
            "{}: spawned at {:.1?} on_ground={} yaw={:.0}",
            p.name, p.movement.origin, p.movement.on_ground, p.view_angles.y
        );
        if !p.movement.on_ground {
            println!("  FAIL: not standing on ground after settling");
            failures += 1;
        }
    }

    // Run forward in eight directions for two seconds each, starting from spawn.
    let t_sim = Instant::now();
    let mut ticks = 0usize;
    for id in [t_id, ct_id] {
        for dir in 0..8 {
            world.respawn(id);
            step(&mut world, &[], 8);
            let start = world.player(id).unwrap().movement.origin;
            let yaw = dir as f32 * 45.0;
            let input = TickInput {
                view_angles: Vec3::new(0.0, yaw, 0.0),
                forward_move: 450.0,
                weapon_max_speed: 250.0,
                ..Default::default()
            };
            let mut max_speed: f32 = 0.0;
            let mut stuck_solid = 0;
            for _ in 0..TICK_RATE * 2 {
                world.step(&[(id, input)]);
                ticks += 1;
                let p = world.player(id).unwrap();
                max_speed = max_speed.max(p.movement.speed_2d());
                let (mins, maxs) = p.movement.hull();
                let tr = world.collision.trace_box(
                    p.movement.origin,
                    p.movement.origin,
                    mins,
                    maxs,
                    collision::contents::MASK_PLAYERSOLID,
                );
                if tr.start_solid {
                    stuck_solid += 1;
                }
            }
            let p = world.player(id).unwrap();
            let moved = (p.movement.origin - start).truncate().length();
            let fell = p.movement.origin.z < floor_z;
            let status = if fell || stuck_solid > 0 { "FAIL" } else { "ok" };
            if status == "FAIL" {
                failures += 1;
            }
            println!(
                "  {status} {} yaw {yaw:>5.0}: moved {moved:>6.1}u, top speed {max_speed:>5.1}, end z {:>7.1}, stuck ticks {stuck_solid}{}",
                p.name,
                p.movement.origin.z,
                if fell { " (fell out of the world)" } else { "" }
            );
        }
    }

    // Jump in place.
    world.respawn(t_id);
    step(&mut world, &[], 8);
    let base_z = world.player(t_id).unwrap().movement.origin.z;
    let jump = TickInput {
        buttons: buttons::JUMP,
        weapon_max_speed: 250.0,
        ..Default::default()
    };
    world.step(&[(t_id, jump)]);
    let mut peak = base_z;
    for _ in 0..TICK_RATE {
        world.step(&[(t_id, TickInput::default())]);
        ticks += 1;
        peak = peak.max(world.player(t_id).unwrap().movement.origin.z);
    }
    let jump_height = peak - base_z;
    let landed = world.player(t_id).unwrap().movement.on_ground;
    println!("jump: height {jump_height:.1}u, landed={landed}");
    if !(50.0..60.0).contains(&jump_height) || !landed {
        println!("  FAIL: expected ~57u jump and a landing");
        failures += 1;
    }

    let per_tick = t_sim.elapsed().as_secs_f64() / ticks.max(1) as f64 * 1e6;
    println!(
        "simulated {ticks} player-ticks ({:.1}s of game time) at {per_tick:.1} µs/tick",
        ticks as f32 * TICK_DT
    );
    println!("failures: {failures}");
    if failures == 0 { 0 } else { 1 }
}
