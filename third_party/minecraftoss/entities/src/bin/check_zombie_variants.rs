//! Exact pinned 26.3 zombie variant gate (`scenarios/mobs/zombie-variants.json`):
//! husks and zombie villagers beside zombies. A zombie, a husk and a zombie
//! villager drown with their water timers pushed on (the zombie becomes a
//! drowned, the husk a zombie; a zombie villager never converts); NoAI ones
//! stand at noon (the husk never burns); a husk with every goal hunts and
//! bites a survival probe (the bite's hunger lasts 140 ticks per whole step
//! of the regional difficulty); and NoAI baby and adult zombie villagers and
//! a baby zombie are hurt once (a baby zombie villager's voice centres on
//! 2.0). Every tick compares type, position, motion, health, fire, the
//! timers and the mob random, the husk's goals, path and controls, the
//! probe's health, knockback and effects, and every sound the mobs play.
use glam::DVec3;
use minecraftoss_entities::{
    tempt::PlayerCandidate,
    world::{EntityWorld, PlayerHitKind},
    zombie::{Zombie, ZombieKind},
};
use minecraftoss_player::survival::{Armor, Difficulty, EffectKind};
use minecraftoss_player::{Block, HitFrom, IncomingHit, Player, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

/// The scenarios' blocks, and whether it is noon (full sky light under
/// open sky); at midnight every position is dim alike.
#[derive(Default)]
struct Scene(BTreeMap<Pos, Block>, bool);
impl World for Scene {
    fn block(&self, pos: Pos) -> Option<Block> {
        self.0.get(&pos).cloned()
    }
    fn set_block(&mut self, pos: Pos, block: Option<Block>) {
        if let Some(block) = block {
            self.0.insert(pos, block);
        } else {
            self.0.remove(&pos);
        }
    }
    fn can_see_sky(&self, (x, y, z): Pos) -> bool {
        !self.0.keys().any(|&(bx, by, bz)| bx == x && bz == z && by > y)
    }
    fn light_path_cost(&self, pos: Pos) -> f32 {
        if self.1 && self.can_see_sky(pos) { 0.5 } else { 0.0 }
    }
}

fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn exact(observed: &Value, actual: f64, label: &str) {
    assert_eq!(actual.to_bits(), number(observed).to_bits(), "{label}: {actual} vs {}", number(observed));
}

fn kind_of(type_id: &str) -> ZombieKind {
    match type_id {
        "minecraft:zombie" => ZombieKind::Zombie,
        "minecraft:husk" => ZombieKind::Husk,
        "minecraft:zombie_villager" => ZombieKind::ZombieVillager,
        "minecraft:drowned" => ZombieKind::Drowned,
        other => panic!("unexpected {other}"),
    }
}

/// The effects a player has, as the probe observation lists them.
fn effects(player: &Player) -> Vec<(String, u32, u8)> {
    [
        (EffectKind::Hunger, "minecraft:hunger"),
        (EffectKind::Poison, "minecraft:poison"),
        (EffectKind::Regeneration, "minecraft:regeneration"),
        (EffectKind::Absorption, "minecraft:absorption"),
        (EffectKind::Resistance, "minecraft:resistance"),
        (EffectKind::FireResistance, "minecraft:fire_resistance"),
        (EffectKind::Nausea, "minecraft:nausea"),
    ]
    .into_iter()
    .filter_map(|(kind, id)| player.survival.effect(kind).map(|e| (id.to_owned(), e.duration, e.amplifier)))
    .collect()
}

/// A sound as the comparison sees it: event, position bits, volume bits, pitch bits.
type Heard = (String, [u64; 3], u32, u32);

#[derive(Default)]
struct Case {
    world: EntityWorld,
    ids: BTreeMap<String, u64>,
    probe: Option<(PlayerCandidate, Player)>,
    heard: Vec<Heard>,
}

#[derive(Default)]
struct Tally {
    frames: usize,
    converted: BTreeMap<String, String>,
    burning: BTreeMap<String, usize>,
    bites: usize,
    hunger: Vec<u32>,
    sounds: usize,
    baby_villager_pitches: Vec<f32>,
}

fn main() {
    let path = env::args().nth(1).expect("usage: check_zombie_variants TRACE.jsonl");
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut case = Case::default();
    let mut scenario = String::new();
    let mut tally = Tally::default();
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => suite = row["data"]["suite"].clone(),
            "complete" => complete = true,
            "scenario_start" => {
                scenario = row["scenario"].as_str().unwrap().to_owned();
                let definition = suite["scenarios"].as_array().unwrap().iter().find(|c| c["id"] == scenario.as_str()).unwrap();
                for command in definition["prepare"].as_array().unwrap() {
                    let parts: Vec<&str> = command.as_str().unwrap().split_whitespace().collect();
                    if parts[0] == "fill" {
                        let n = |i: usize| parts[i].parse::<i32>().unwrap();
                        for x in n(1)..=n(4) {
                            for y in n(2)..=n(5) {
                                for z in n(3)..=n(6) {
                                    scene.set_block((x, y, z), Some(Block::new(parts[7])));
                                }
                            }
                        }
                    }
                }
                case = Case::default();
                // Noon burns the undead (`gameplay/monsters_burn`); the other
                // scenarios run at midnight.
                case.world.set_monsters_burn(scenario == "variants_noon");
                scene.1 = scenario == "variants_noon";
            }
            "entity_set_random_seed" => {
                let id = case.ids[row["data"]["tag"].as_str().unwrap()];
                case.world.zombie_mut(id).unwrap().set_random_seed(row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "zombie_set_in_water_time" => {
                let id = case.ids[row["data"]["tag"].as_str().unwrap()];
                case.world.zombie_mut(id).unwrap().zombie.in_water_time = row["data"]["time"].as_i64().unwrap() as i32;
            }
            "zombie_set_conversion_time" => {
                let id = case.ids[row["data"]["tag"].as_str().unwrap()];
                case.world.zombie_mut(id).unwrap().zombie.conversion_time = row["data"]["time"].as_i64().unwrap() as i32;
            }
            // The drowning model runs no goals, so keeping only the look goal
            // changes nothing.
            "entity_keep_goal" => assert_eq!(scenario, "variants_water"),
            "player_probe" => {
                let pos = &row["data"]["pos"];
                let position = DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap());
                let definition = suite["scenarios"].as_array().unwrap().iter().find(|c| c["id"] == scenario.as_str()).unwrap();
                let action = definition["actions"].as_array().unwrap().iter().find(|a| a["type"] == "player_probe").unwrap();
                let candidate = PlayerCandidate {
                    id: 1_000_000,
                    position,
                    eye_height: 1.62,
                    main_hand_cow_food: false,
                    offhand_cow_food: false,
                    main_hand_pig_food: false,
                    offhand_pig_food: false,
                    main_hand_chicken_food: false,
                    offhand_chicken_food: false,
                    main_hand_carrot_on_a_stick: false,
                    offhand_carrot_on_a_stick: false,
                    main_hand_wolf_interest: false,
                    offhand_wolf_interest: false,
                    main_hand_horse_tempt: false,
                    offhand_horse_tempt: false,
                    alive: true,
                    spectator: false,
                    attackable: true,
                };
                let mut player = Player::new(position);
                player.yaw = action["yaw"].as_f64().unwrap_or(0.0);
                player.on_ground = false;
                case.probe = Some((candidate, player));
            }
            "entity_hurt" => {
                let data = &row["data"];
                let entity = case.world.zombie_mut(case.ids[data["tag"].as_str().unwrap()]).unwrap();
                exact(&data["health_before"], f64::from(entity.zombie.health), &format!("{scenario} health before"));
                let result = entity.hurt(number(&data["amount"]) as f32);
                assert_eq!(data["applied"], result.applied, "{scenario} hurt applied");
            }
            "snapshot" if scenario == "variants_warmup" => {}
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = data["entities"].as_object().unwrap();
                if tick == 0 {
                    spawn(&mut case, &scenario, data, observed);
                    continue;
                }
                let players: Vec<PlayerCandidate> = case.probe.iter().map(|p| p.0).collect();
                case.world.tick_with_players(&mut scene, &players);
                assert_eq!(case.world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                let heard: Vec<Heard> = case
                    .world
                    .take_sounds()
                    .into_iter()
                    .map(|s| (format!("minecraft:{}", s.event), [s.position.x.to_bits(), s.position.y.to_bits(), s.position.z.to_bits()], s.volume.to_bits(), s.pitch.to_bits()))
                    .collect();
                case.heard.extend(heard);
                match scenario.as_str() {
                    "variants_water" => check_water(&case, tick, data, observed, &mut tally),
                    "variants_noon" => check_still(&case, &scenario, tick, observed, &mut tally),
                    "husk_bite" => check_bite(&mut case, tick, data, observed, &mut tally),
                    "villager_voice" => {
                        check_still(&case, &scenario, tick, observed, &mut tally);
                        check_sounds(&mut case, tick, data, &mut tally);
                    }
                    other => panic!("unexpected scenario {other}"),
                }
                tally.frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    let converted: Vec<(&str, &str)> = tally.converted.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    assert_eq!(converted, [("minecraft:husk", "minecraft:zombie"), ("minecraft:zombie", "minecraft:drowned")], "conversions");
    assert_eq!(tally.burning.get("husk"), None, "the husk never burns");
    assert!(tally.burning.get("zombie").is_some_and(|&n| n > 0) && tally.burning.get("villager").is_some_and(|&n| n > 0), "the zombie and the zombie villager burn");
    assert_eq!(tally.bites, 1, "one bite lands");
    assert_eq!(tally.hunger, [140], "the bite starves for 140 ticks");
    assert!(!tally.baby_villager_pitches.is_empty() && tally.baby_villager_pitches.iter().all(|p| (1.9..=2.1).contains(p)), "a baby zombie villager's voice centres on 2.0");
    println!(
        "{} exact zombie variant frames matched (conversions {:?}; burning mob-ticks {:?}; {} husk bite, hunger {:?}; {} sounds, baby villager pitches {:?})",
        tally.frames, tally.converted, tally.burning, tally.bites, tally.hunger, tally.sounds, tally.baby_villager_pitches
    );
}

/// Tick 0: the scenario's mobs join the entity world in entity ID order.
fn spawn(case: &mut Case, scenario: &str, data: &Value, observed: &serde_json::Map<String, Value>) {
    case.world.set_game_time(data["game_time"].as_i64().unwrap());
    let mut order: Vec<(&String, &Value)> = observed.iter().map(|(tag, states)| (tag, &states[0])).collect();
    order.sort_by(|a, b| {
        let key = |e: &Value| (e["entity_numeric_id"].as_u64().unwrap_or(0), number(&e["x"]).to_bits());
        key(a.1).cmp(&key(b.1))
    });
    for (tag, e) in order {
        let mut zombie = Zombie::new(DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"])));
        zombie.kind = kind_of(e["type"].as_str().unwrap());
        zombie.set_baby(e["zombie_baby"].as_bool().unwrap());
        zombie.body.velocity = DVec3::new(number(&e["vx"]), number(&e["vy"]), number(&e["vz"]));
        zombie.body.on_ground = e["on_ground"].as_bool().unwrap();
        zombie.persistence_required = e["persistence_required"].as_bool().unwrap();
        if let Some(id) = e["entity_numeric_id"].as_u64() {
            case.world.set_next_entity_id(id);
        }
        let id = match scenario {
            "variants_water" => case.world.spawn_zombie_drowning(zombie),
            "husk_bite" => case.world.spawn_zombie_active(zombie, number(&e["yaw"]) as f32),
            _ => case.world.spawn_zombie(zombie, true),
        };
        case.ids.insert(tag.clone(), id);
    }
}

/// The drowning mobs: each found by its column (conversion replaces the
/// entity in place), and the level's count of each type.
fn check_water(case: &Case, tick: i64, data: &Value, observed: &serde_json::Map<String, Value>, tally: &mut Tally) {
    for (tag, states) in observed {
        let e = &states[0];
        let at = format!("variants_water tick {tick} {tag}");
        let x = number(&e["x"]).to_bits();
        let entity = case.world.zombies().iter().find(|z| z.zombie.body.position.x.to_bits() == x).unwrap_or_else(|| panic!("{at} present"));
        assert_eq!(e["type"], entity.zombie.kind.type_id(), "{at} type");
        let original = case.world.zombies().iter().any(|z| case.ids[tag] == z.id);
        if !original {
            let from = if tag == "wzombie" { "minecraft:zombie" } else { "minecraft:husk" };
            tally.converted.insert(from.to_owned(), entity.zombie.kind.type_id().to_owned());
        }
        assert_eq!(e["zombie_underwater_converting"], entity.zombie.underwater_converting, "{at} converting");
        assert_eq!(e["zombie_baby"], entity.zombie.baby, "{at} baby");
        assert_eq!(e["persistence_required"], entity.zombie.persistence_required, "{at} persistence");
        exact(&e["health"], f64::from(entity.zombie.health), &format!("{at} health"));
        let body = &entity.zombie.body;
        for (field, actual) in [("x", body.position.x), ("y", body.position.y), ("z", body.position.z), ("vx", body.velocity.x), ("vy", body.velocity.y), ("vz", body.velocity.z)] {
            exact(&e[field], actual, &format!("{at} {field}"));
        }
        assert_eq!(e["on_ground"], body.on_ground, "{at} ground");
    }
    for (type_id, counts) in data["entity_type_counts"].as_object().unwrap() {
        let ours: Vec<&Zombie> = case.world.zombies().iter().map(|z| &z.zombie).filter(|z| z.kind.type_id() == type_id).collect();
        assert_eq!(counts["total"], ours.len(), "variants_water tick {tick} {type_id} count");
        assert_eq!(counts["babies"], ours.iter().filter(|z| z.baby).count(), "variants_water tick {tick} {type_id} babies");
    }
}

/// NoAI mobs: tick count, fire, health, the ambient and idle timers and the
/// random.
fn check_still(case: &Case, scenario: &str, tick: i64, observed: &serde_json::Map<String, Value>, tally: &mut Tally) {
    for (tag, states) in observed {
        let e = &states[0];
        let at = format!("{scenario} tick {tick} {tag}");
        let entity = case.world.zombies().iter().find(|z| z.id == case.ids[tag]).unwrap();
        assert_eq!(e["type"], entity.zombie.kind.type_id(), "{at} type");
        assert_eq!(e["entity_tick_count"], entity.tick_count, "{at} tick count");
        assert_eq!(e["remaining_fire_ticks"], entity.zombie.body.fire_ticks, "{at} fire ticks");
        exact(&e["health"], f64::from(entity.zombie.health), &format!("{at} health"));
        assert_eq!(e["ambient_sound_time"], entity.ambient_sound_time, "{at} ambient time");
        assert_eq!(e["no_action_time"], entity.no_action_time, "{at} idle time");
        assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random");
        if entity.zombie.body.fire_ticks > 0 {
            *tally.burning.entry(tag.clone()).or_default() += 1;
        }
    }
}

/// The husk on its goals, and the probe it bites.
fn check_bite(case: &mut Case, tick: i64, data: &Value, observed: &serde_json::Map<String, Value>, tally: &mut Tally) {
    for (tag, states) in observed {
        let e = &states[0];
        let at = format!("husk_bite tick {tick} {tag}");
        let entity = case.world.zombies().iter().find(|z| z.id == case.ids[tag]).unwrap();
        assert_eq!(e["type"], entity.zombie.kind.type_id(), "{at} type");
        let ai = entity.ai.as_deref().unwrap();
        let (body, state) = (&entity.zombie.body, &ai.state);
        for (field, actual) in [
            ("x", body.position.x),
            ("y", body.position.y),
            ("z", body.position.z),
            ("vx", body.velocity.x),
            ("vy", body.velocity.y),
            ("vz", body.velocity.z),
            ("health", f64::from(entity.zombie.health)),
            ("yaw", f64::from(ai.yaw)),
            ("speed", f64::from(ai.speed)),
            ("head_yaw", f64::from(state.look_control.head_yaw)),
            ("body_yaw", f64::from(ai.body_rotation.body_yaw)),
            ("pitch", f64::from(state.look_control.pitch)),
            ("eye_y", body.position.y + f64::from(state.eye_height)),
            ("move_control_x", ai.move_control.wanted.x),
            ("move_control_y", ai.move_control.wanted.y),
            ("move_control_z", ai.move_control.wanted.z),
            ("look_control_x", state.look_control.wanted.x),
            ("look_control_y", state.look_control.wanted.y),
            ("look_control_z", state.look_control.wanted.z),
        ] {
            exact(&e[field], actual, &format!("{at} {field}"));
        }
        assert_eq!(e["on_ground"], body.on_ground, "{at} on ground");
        assert_eq!(e["entity_tick_count"], entity.tick_count, "{at} tick count");
        assert_eq!(e["ambient_sound_time"], entity.ambient_sound_time, "{at} ambient time");
        assert_eq!(e["no_action_time"], entity.no_action_time, "{at} idle time");
        assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random");
        let goals: Vec<&str> = e["running_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
        assert_eq!(goals, ai.running_goals(), "{at} goals");
        let targeted = e.get("target_uuid").is_some_and(|t| !t.is_null());
        assert_eq!(targeted, state.target().is_some(), "{at} target");
        assert_eq!(e["aggressive"], state.melee.aggressive, "{at} aggressive");
        assert_eq!(e["move_control_wanted"], ai.move_control.has_wanted(), "{at} move wanted");
        assert_eq!(e["look_control_wanted"], state.look_control.cooldown > 0, "{at} look wanted");
        assert_eq!(e["navigation_done"], state.navigation.is_done(), "{at} navigation done");
        assert_eq!(e["path_next_node"], state.navigation.observed_next(), "{at} path next");
        let nodes: Vec<(i32, i32, i32)> = e["path_nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| (n[0].as_i64().unwrap() as i32, n[1].as_i64().unwrap() as i32, n[2].as_i64().unwrap() as i32))
            .collect();
        assert_eq!(nodes, state.navigation.nodes, "{at} path");
    }
    // The bite goes through `Player.hurtServer`; one that lands starves
    // (`Husk.doHurtTarget`). The probe never ticks, so the hunger holds.
    for hit in case.world.take_player_hits() {
        let (_, player) = case.probe.as_mut().unwrap();
        let PlayerHitKind::Melee { attacker, hunger_ticks, .. } = hit.kind else { panic!("husk_bite tick {tick}: only bites") };
        let incoming = IncomingHit { damage: hit.damage, from: HitFrom::Position(attacker), scales_with_difficulty: true, exhaustion: 0.1 };
        if player.hurt_by(&incoming, Difficulty::Normal, Armor { value: Some((0.0, 0.0)) }, 0.0) {
            tally.bites += 1;
            tally.hunger.push(hunger_ticks as u32);
            if hunger_ticks > 0 {
                player.survival.add_effect(EffectKind::Hunger, hunger_ticks as u32, 0);
            }
        }
    }
    if let Some((_, player)) = &case.probe {
        let at = format!("husk_bite tick {tick} probe");
        let probe = &data["player_probes"]["p0"];
        exact(&probe["health"], f64::from(player.survival.health), &format!("{at} health"));
        exact(&probe["motion_x"], player.velocity.x, &format!("{at} motion x"));
        exact(&probe["motion_y"], player.velocity.y, &format!("{at} motion y"));
        exact(&probe["motion_z"], player.velocity.z, &format!("{at} motion z"));
        exact(&probe["hurt_dir"], f64::from(player.hurt_dir), &format!("{at} hurt dir"));
        exact(&probe["exhaustion"], f64::from(player.survival.food.exhaustion), &format!("{at} exhaustion"));
        assert_eq!(probe["hurt_time"], player.hurt_time, "{at} hurt time");
        let vanilla: Vec<(String, u32, u8)> = probe["effects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["id"].as_str().unwrap().to_owned(), e["duration"].as_u64().unwrap() as u32, e["amplifier"].as_u64().unwrap() as u8))
            .collect();
        assert_eq!(effects(player), vanilla, "{at} effects");
    }
}

/// Every sound the mobs played this tick: event, position, volume and pitch.
fn check_sounds(case: &mut Case, tick: i64, data: &Value, tally: &mut Tally) {
    let bits = |v: &Value| u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap();
    let mut vanilla: Vec<Heard> = data["entity_sound_events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let p = e["position"].as_array().unwrap();
            let hex = |key: &str| u32::from_str_radix(e[key].as_str().unwrap(), 16).unwrap();
            (e["id"].as_str().unwrap().to_owned(), [bits(&p[0]), bits(&p[1]), bits(&p[2])], hex("volume_bits"), hex("pitch_bits"))
        })
        .collect();
    let mut ours = std::mem::take(&mut case.heard);
    vanilla.sort();
    ours.sort();
    assert_eq!(ours, vanilla, "villager_voice tick {tick} sounds");
    let baby = case.world.zombies().iter().find(|z| z.id == case.ids["baby_villager"]).unwrap().zombie.body.position.x.to_bits();
    tally.baby_villager_pitches.extend(vanilla.iter().filter(|s| s.1[0] == baby).map(|s| f32::from_bits(s.3)));
    tally.sounds += vanilla.len();
}
