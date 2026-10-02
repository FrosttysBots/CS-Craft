//! Exact pinned 26.3 unfiltered zombie gate (`scenarios/mobs/zombie-full-ai.json`):
//! zombies with every registered goal at midnight, mobs allowed to grief.
//! One idles alone (stroll, looks, the turtle-egg goal's timer), one hunts
//! and hits a probe player, and a probe's hit on one of three zombies alerts
//! the two behind a wall, which walk round. The zombies run on the monster
//! goal framework; every tick compares position, motion, rotations,
//! controls, path, goals, target, raised arms and the mob random.
#[path = "support/sounds.rs"]
mod sounds;
use glam::DVec3;
use minecraftoss_entities::{
    tempt::PlayerCandidate,
    world::{EntityWorld, PlayerAttack},
    zombie::Zombie,
};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

#[derive(Default)]
struct Scene(BTreeMap<Pos, Block>);
impl World for Scene {
    fn step_sound(&self, pos: Pos) -> Option<(String, f32, f32)> {
        sounds::step_sound(&self.0.get(&pos)?.id)
    }
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
}

fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn exact(observed: &Value, actual: f64, label: &str) {
    assert_eq!(actual.to_bits(), number(observed).to_bits(), "{label}: {actual} vs {}", number(observed));
}

fn probe(id: u64, position: DVec3) -> PlayerCandidate {
    PlayerCandidate {
        id,
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
    }
}

#[derive(Default)]
struct Case {
    world: EntityWorld,
    ids: BTreeMap<String, u64>,
    probes: BTreeMap<String, (PlayerCandidate, f32)>,
}

fn main() {
    let path = env::args().nth(1).expect("usage: check_zombie_full_ai TRACE.jsonl");
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut case = Case::default();
    let mut scenario = String::new();
    let (mut frames, mut hits, mut alerted, mut heard) = (0, 0, 0, 0);
    let mut goals_seen = std::collections::BTreeSet::new();
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
            }
            "entity_set_random_seed" => {
                let id = case.ids[row["data"]["tag"].as_str().unwrap()];
                case.world.zombie_mut(id).unwrap().set_random_seed(row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "player_probe" => {
                let tag = row["data"]["tag"].as_str().unwrap().to_owned();
                let pos = &row["data"]["pos"];
                let position = DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap());
                let definition = suite["scenarios"].as_array().unwrap().iter().find(|c| c["id"] == scenario.as_str()).unwrap();
                let action = definition["actions"].as_array().unwrap().iter().find(|a| a["type"] == "player_probe" && a["tag"] == tag.as_str()).unwrap();
                let id = 1_000_000 + case.probes.len() as u64;
                case.probes.insert(tag, (probe(id, position), action["yaw"].as_f64().unwrap_or(0.0) as f32));
            }
            "player_attack" => {
                // A full-strength bare-handed hit on the ground.
                let data = &row["data"];
                let (candidate, yaw) = case.probes[data["probe"].as_str().unwrap()];
                let attack = PlayerAttack {
                    player_id: candidate.id,
                    position: candidate.position,
                    yaw,
                    attack_damage: 1.0,
                    strength: 1.0,
                    sprinting: false,
                    can_critical: false,
                    can_sweep: false,
                };
                let target = case.ids[data["target"].as_str().unwrap()];
                let result = case.world.player_attack(&attack, target);
                assert!(result.hurt, "{scenario} attack lands");
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = data["entities"].as_object().unwrap();
                if tick == 0 {
                    case.world.set_game_time(data["game_time"].as_i64().unwrap());
                    let mut order: Vec<(&String, &Value)> = observed.iter().map(|(tag, states)| (tag, &states[0])).collect();
                    order.sort_by_key(|(_, e)| e["entity_numeric_id"].as_u64().unwrap());
                    for (tag, e) in order {
                        let mut zombie = Zombie::new(DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"])));
                        zombie.body.on_ground = e["on_ground"].as_bool().unwrap();
                        zombie.persistence_required = true;
                        case.world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
                        let id = case.world.spawn_zombie_active(zombie, number(&e["yaw"]) as f32);
                        case.ids.insert(tag.clone(), id);
                    }
                    continue;
                }
                let players: Vec<PlayerCandidate> = case.probes.values().map(|p| p.0).collect();
                let ticks = observed.iter().find_map(|(tag, states)| {
                    let e = states.as_array().unwrap().first()?;
                    let ours = case.world.zombies().iter().find(|z| z.id == case.ids[tag])?;
                    Some(e["entity_tick_count"].as_i64().unwrap() > i64::from(ours.tick_count))
                });
                if ticks.unwrap_or(true) {
                    case.world.tick_with_players(&mut scene, &players);
                } else {
                    case.world.set_game_time(data["game_time"].as_i64().unwrap());
                }
                assert_eq!(case.world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                heard += sounds::check(&mut case.world, data, &format!("{scenario} tick {tick}"), &sounds::pens(&suite, &scenario));
                hits += case.world.take_player_hits().len();
                for (tag, states) in observed {
                    let id = case.ids[tag];
                    let ours = case.world.zombies().iter().find(|z| z.id == id);
                    let states = states.as_array().unwrap();
                    assert_eq!(states.len(), usize::from(ours.is_some()), "{scenario} tick {tick} {tag} presence");
                    let Some(entity) = ours else { continue };
                    let e = &states[0];
                    let at = format!("{scenario} tick {tick} {tag}");
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
                    goals_seen.extend(ai.running_goals());
                    let targeted = e.get("target_uuid").is_some_and(|t| !t.is_null());
                    assert_eq!(targeted, state.target().is_some(), "{at} target");
                    if targeted && tag != "zombiea" && scenario == "zombie_alert" {
                        alerted += 1;
                    }
                    assert_eq!(e["aggressive"], state.melee.aggressive, "{at} aggressive");
                    assert_eq!(e["jumping"].as_bool().unwrap_or(ai.jumping), ai.jumping, "{at} jumping");
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
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert!(alerted > 0, "the hit alerts the zombies behind the wall");
    println!("{frames} exact unfiltered zombie frames matched ({hits} hits on the probe, {alerted} alerted zombie-ticks, {heard} sounds; goals seen: {goals_seen:?})");
}
