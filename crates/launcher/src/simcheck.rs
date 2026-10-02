//! `csr sim <map>`: headless checks against the real map.
//!
//! 1. Movement: players spawn on the ground, run in eight directions without
//!    falling out of the world or sticking in solids, and jump ~57 units.
//! 2. Combat: a 5v5 bot deathmatch runs for a minute of game time; bots
//!    must move, fire, hit and kill with the real weapon stats.
//! 3. Rounds: a 5v5 bot competitive match with the game's own
//!    `gamemode_competitive.cfg` rules; rounds must start and end, bots
//!    must buy, plant the bomb, and the economy must stay within bounds.
//!
//! Exits non-zero on failure.

use glam::Vec3;
use movement::buttons;
use sim::{bots::Bots, GameEvent, GameMode, PlayerId, RoundEndReason, RulesConfig, Team, TickInput, World, TICK_DT, TICK_RATE};
use std::collections::BTreeMap;
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
    println!("weapon catalog: {} weapons", level.catalog.weapons.len());
    let mut failures = movement_checks(&level);
    failures += combat_check(&level);
    failures += round_check(&level);
    println!("failures: {failures}");
    if failures == 0 { 0 } else { 1 }
}

fn movement_checks(level: &level::Level) -> i32 {
    let floor_z = level.collision.mins.z;
    let mut world = World::with_catalog(level.collision.clone(), level.catalog.clone());
    world.load_spawns(&level.bsp);
    println!("spawns: {}", world.spawns.len());
    if world.spawns.is_empty() {
        eprintln!("error: map has no spawn points");
        return 1;
    }
    let t_id = world.add_player("terrorist", Team::Terrorist);
    let ct_id = world.add_player("counter-terrorist", Team::CounterTerrorist);

    let mut failures = 0;
    let step = |world: &mut World, inputs: &[(PlayerId, TickInput)], ticks: usize| {
        for _ in 0..ticks {
            world.step(inputs);
        }
    };

    step(&mut world, &[], TICK_RATE as usize / 2);
    for id in [t_id, ct_id] {
        let p = world.player(id).unwrap();
        println!(
            "{}: spawned at {:.1?} on_ground={} weapon={:?}",
            p.name,
            p.movement.origin,
            p.movement.on_ground,
            p.active_weapon().map(|w| w.def.class_name.clone())
        );
        if !p.movement.on_ground {
            println!("  FAIL: not standing on ground after settling");
            failures += 1;
        }
    }

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
                weapon_slot: 3, // knife: 250 u/s
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

    world.respawn(t_id);
    step(&mut world, &[], 8);
    let base_z = world.player(t_id).unwrap().movement.origin.z;
    let jump = TickInput {
        buttons: buttons::JUMP,
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
    println!("movement: {ticks} player-ticks at {per_tick:.1} µs/tick");
    failures
}

fn combat_check(level: &level::Level) -> i32 {
    if level.catalog.weapons.is_empty() {
        println!("combat: FAIL no weapon catalog");
        return 1;
    }
    let mut world = World::with_catalog(level.collision.clone(), level.catalog.clone());
    world.load_spawns(&level.bsp);
    for i in 0..5 {
        world.add_bot(&format!("T bot {i}"), Team::Terrorist);
        world.add_bot(&format!("CT bot {i}"), Team::CounterTerrorist);
    }
    let mut bots = Bots::with_skill(0.7).with_nav(level.nav.clone());
    println!(
        "nav mesh: {}",
        level.nav.as_ref().map_or("none".to_string(), |n| format!("{} areas, {} places", n.areas.len(), n.places.len()))
    );
    let seconds = 60;
    let t = Instant::now();
    let (mut shots, mut hits, mut kills, mut headshots, mut impacts) = (0, 0, 0, 0, 0);
    let mut distance = 0.0f32;
    let mut last: Vec<Vec3> = world.players.iter().map(|p| p.movement.origin).collect();
    let mut stall = vec![0u32; world.players.len()];
    let mut peak_stall = 0u32;
    for _ in 0..(TICK_RATE * seconds) {
        let inputs = bots.think(&world);
        world.step(&inputs);
        for (i, p) in world.players.iter().enumerate() {
            if p.alive {
                let d = (p.movement.origin - last[i]).length();
                if d < 64.0 {
                    distance += d;
                }
                let commanded = inputs.iter().find(|(id,_)| *id==p.id).is_some_and(|(_,v)| v.forward_move.abs()+v.side_move.abs()>100.0);
                stall[i] = if commanded && d < 0.3 && p.movement.on_ground { stall[i]+1 } else { 0 };
                peak_stall = peak_stall.max(stall[i]);
            }
            last[i] = p.movement.origin;
        }
        for e in &world.events {
            match e {
                GameEvent::Fire { .. } => shots += 1,
                GameEvent::Damage { .. } => hits += 1,
                GameEvent::Impact { .. } => impacts += 1,
                GameEvent::Kill { headshot, .. } => {
                    kills += 1;
                    if *headshot {
                        headshots += 1;
                    }
                }
                _ => {}
            }
        }
    }
    let elapsed = t.elapsed().as_secs_f32();
    let t_kills: u32 = world.players.iter().filter(|p| p.team == Team::Terrorist).map(|p| p.kills).sum();
    let ct_kills: u32 = world.players.iter().filter(|p| p.team == Team::CounterTerrorist).map(|p| p.kills).sum();
    println!(
        "combat: 5v5 bots for {seconds}s game time in {elapsed:.2}s ({:.0}x real time)",
        seconds as f32 / elapsed
    );
    println!(
        "  bots walked {:.0}u total; {shots} shots, {impacts} wall impacts, {hits} hits, {kills} kills ({headshots} headshots); score T {t_kills} - CT {ct_kills}",
        distance
    );
    println!("  longest continuous blocked movement: {:.2}s", peak_stall as f32 * TICK_DT);
    let mut failures = 0;
    if distance < 1000.0 {
        println!("  FAIL: bots barely moved");
        failures += 1;
    }
    if shots == 0 || hits == 0 || kills == 0 {
        println!("  FAIL: expected shots, hits and kills");
        failures += 1;
    }
    let _ = TICK_DT;
    failures
}

fn round_check(level: &level::Level) -> i32 {
    let mut world = World::with_catalog(level.collision.clone(), level.catalog.clone());
    world.load_map(&level.bsp, level.overview().as_deref());
    let rules: RulesConfig = client::competitive_rules(level);
    println!(
        "rounds: freeze {}s, round {:.0}s, buy {}s, bomb {}s, start ${}, {} rounds; {} buy zones, sites {:?}",
        rules.freeze_time,
        rules.round_time,
        rules.buy_time,
        rules.c4_timer,
        rules.start_money,
        rules.max_rounds,
        world.objectives.buy_zones.len(),
        world.objectives.bomb_sites.iter().map(|s| s.name).collect::<String>()
    );
    let mut failures = 0;
    if world.objectives.bomb_sites.is_empty() || world.objectives.buy_zones.len() < 2 {
        println!("  FAIL: expected bomb sites and a buy zone per team");
        failures += 1;
    }
    for i in 0..5 {
        world.add_bot(&format!("T bot {i}"), Team::Terrorist);
        world.add_bot(&format!("CT bot {i}"), Team::CounterTerrorist);
    }
    world.set_mode(GameMode::Competitive, rules.clone());
    if world.players.iter().any(|p| !p.in_buy_zone && p.alive) {
        // Buy zone flags update on the first tick.
        world.step(&[]);
    }
    let not_in_zone = world.players.iter().filter(|p| !p.in_buy_zone).count();
    if not_in_zone > 0 {
        println!("  FAIL: {not_in_zone} players spawned outside their buy zone");
        failures += 1;
    }
    let mut bots = Bots::with_skill(0.7).with_nav(level.nav.clone());
    let target_rounds = 10;
    let t = Instant::now();
    let mut reasons: BTreeMap<String, u32> = BTreeMap::new();
    let (mut purchases, mut plants, mut defuses, mut explosions, mut kills, mut rounds) = (0, 0, 0, 0, 0, 0);
    let mut bought: BTreeMap<String, u32> = BTreeMap::new();
    let mut utility: BTreeMap<String,u32> = BTreeMap::new();
    let mut detonations=0;
    let mut money_ok = true;
    let mut ticks = 0u64;
    let max_ticks = (TICK_RATE as f32 * (rules.freeze_time + rules.round_time + rules.c4_timer + rules.restart_delay + 5.0) * target_rounds as f32) as u64;
    while rounds < target_rounds && ticks < max_ticks {
        let inputs = bots.think(&world);
        world.step(&inputs);
        ticks += 1;
        for e in &world.events {
            match e {
                GameEvent::UtilityThrown {kind,..} => {*utility.entry(format!("{kind:?}")).or_default()+=1;}
                GameEvent::UtilityDetonated {..} => {detonations+=1;}
                GameEvent::Purchase { item, .. } => {
                    purchases += 1;
                    let name = match item {
                        sim::Purchase::Weapon(i) => world.catalog.weapons.iter().find(|w| w.def_index == *i).map_or("?".into(), |w| w.class_name.trim_start_matches("weapon_").to_string()),
                        other => format!("{other:?}"),
                    };
                    *bought.entry(name).or_default() += 1;
                }
                GameEvent::BombPlanted { site, .. } => {
                    plants += 1;
                    println!("  round {}: bomb planted at {site} ({:.0}s into the round)", world.round.round, world.time() - world.round.live_start);
                }
                GameEvent::BombDefused { .. } => defuses += 1,
                GameEvent::BombExploded { .. } => explosions += 1,
                GameEvent::Kill { .. } => kills += 1,
                GameEvent::RoundEnd { reason } => {
                    rounds += 1;
                    *reasons.entry(format!("{reason:?}")).or_default() += 1;
                    let money: Vec<i32> = world.players.iter().map(|p| p.money).collect();
                    println!(
                        "  round {} -> {} (T {} - CT {}), money {}..{}",
                        world.round.round,
                        reason.message(),
                        world.round.score_t,
                        world.round.score_ct,
                        money.iter().min().unwrap_or(&0),
                        money.iter().max().unwrap_or(&0)
                    );
                }
                _ => {}
            }
        }
        if world.players.iter().any(|p| p.money < 0 || p.money > rules.max_money) {
            money_ok = false;
        }
    }
    let elapsed = t.elapsed().as_secs_f32();
    let game = ticks as f32 * TICK_DT;
    println!(
        "  {rounds} rounds in {game:.0}s game time ({elapsed:.1}s, {:.0}x real time): {reasons:?}",
        game / elapsed.max(1e-3)
    );
    println!("  {purchases} purchases {bought:?}; {kills} kills, {plants} plants, {defuses} defuses, {explosions} explosions");
    println!("  utility thrown {utility:?}; {detonations} detonations");
    if rounds < target_rounds {
        println!("  FAIL: only {rounds} of {target_rounds} rounds finished");
        failures += 1;
    }
    if purchases == 0 {
        println!("  FAIL: bots never bought anything");
        failures += 1;
    }
    if plants == 0 {
        println!("  FAIL: the bomb was never planted");
        failures += 1;
    }
    if !money_ok {
        println!("  FAIL: money went outside 0..max");
        failures += 1;
    }
    let _ = RoundEndReason::Draw;
    failures
}
