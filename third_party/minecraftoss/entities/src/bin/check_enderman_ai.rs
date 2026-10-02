//! Exact pinned 26.3 enderman gate (`scenarios/mobs/enderman-ai.json`):
//! endermen in walled stone pens of an empty void world take blocks from a
//! ring of grass, grow angry at a probe that stares at their eyes (and
//! freeze, or teleport away when it is close), hunt a probe that hit them,
//! drown and teleport out of water, and teleport under the noon sun. Every
//! tick compares position, motion, rotations, controls, path, goals and
//! target goals, target, the carried block, the creepy and stared flags,
//! anger, the mob random, the blocks it moved and every sound in the pen.
#[path = "support/sounds.rs"]
mod sounds;

use glam::DVec3;
use minecraftoss_entities::{
    enderman::{Enderman, PlayerView},
    tempt::PlayerCandidate,
    world::{EntityWorld, PlayerAttack},
};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

/// The pens' blocks, lit as at midnight (dim) or noon (full light under
/// open sky).
#[derive(Default)]
struct Scene {
    blocks: BTreeMap<Pos, Block>,
    noon: bool,
}
impl World for Scene {
    fn block(&self, pos: Pos) -> Option<Block> {
        self.blocks.get(&pos).cloned()
    }
    fn set_block(&mut self, pos: Pos, block: Option<Block>) {
        match block {
            Some(block) if block.id != "minecraft:air" => {
                self.blocks.insert(pos, block);
            }
            _ => {
                self.blocks.remove(&pos);
            }
        }
    }
    fn step_sound(&self, pos: Pos) -> Option<(String, f32, f32)> {
        sounds::step_sound(&self.blocks.get(&pos)?.id)
    }
    fn can_see_sky(&self, (x, y, z): Pos) -> bool {
        !self.blocks.keys().any(|&(bx, by, bz)| bx == x && bz == z && by >= y)
    }
    fn light_path_cost(&self, pos: Pos) -> f32 {
        // `LightTexture.getBrightness` less 0.5: full light at noon under
        // open sky, the midnight sky's level 4 otherwise.
        let night = (4.0_f32 / 15.0) / (4.0 - 3.0 * (4.0_f32 / 15.0)) - 0.5;
        if self.noon && self.can_see_sky(pos) {
            0.5
        } else {
            night
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

fn main() {
    let path = env::args().nth(1).expect("usage: check_enderman_ai TRACE.jsonl");
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut probes: BTreeMap<String, (PlayerCandidate, PlayerView)> = BTreeMap::new();
    let mut scenario = String::new();
    let (mut frames, mut heard, mut teleports, mut taken, mut stared, mut hits) = (0, 0, 0, 0, 0, 0);
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
                    match parts[0] {
                        "fill" => {
                            let n = |i: usize| parts[i].parse::<i32>().unwrap();
                            for x in n(1)..=n(4) {
                                for y in n(2)..=n(5) {
                                    for z in n(3)..=n(6) {
                                        scene.set_block((x, y, z), Some(Block::new(parts[7])));
                                    }
                                }
                            }
                        }
                        "time" => scene.noon = parts[2] == "noon",
                        _ => {}
                    }
                }
                world = EntityWorld::default();
                world.set_bright_outside(scene.noon);
                ids.clear();
                probes.clear();
            }
            "entity_set_random_seed" => {
                let id = ids[row["data"]["tag"].as_str().unwrap()];
                world.enderman_mut(id).unwrap().set_random_seed(row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "player_probe" => {
                let data = &row["data"];
                let tag = data["tag"].as_str().unwrap().to_owned();
                let pos = &data["pos"];
                let position = DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap());
                let view = PlayerView {
                    head_yaw: data["head_yaw"].as_f64().unwrap_or(0.0) as f32,
                    pitch: data["pitch"].as_f64().unwrap_or(0.0) as f32,
                    disguised: false,
                };
                let id = 1_000_000 + probes.len() as u64;
                probes.insert(tag, (probe(id, position), view));
                world.set_player_views(probes.values().map(|(p, v)| (p.id, *v)).collect());
            }
            "player_attack" => {
                // A full-strength bare-handed hit on the ground.
                let data = &row["data"];
                let (candidate, view) = probes[data["probe"].as_str().unwrap()];
                let attack = PlayerAttack {
                    player_id: candidate.id,
                    position: candidate.position,
                    yaw: view.head_yaw,
                    attack_damage: 1.0,
                    strength: 1.0,
                    sprinting: false,
                    can_critical: false,
                    can_sweep: false,
                };
                let target = ids[data["target"].as_str().unwrap()];
                let result = world.player_attack(&attack, target);
                assert!(result.hurt, "{scenario} attack lands");
            }
            "snapshot" if scenario == "enderman_warmup" => {}
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = data["entities"].as_object().unwrap();
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    let mut order: Vec<(&String, &Value)> = observed.iter().map(|(tag, states)| (tag, &states[0])).collect();
                    order.sort_by_key(|(_, e)| e["entity_numeric_id"].as_u64().unwrap());
                    for (tag, e) in order {
                        let mut enderman = Enderman::new(DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"])));
                        enderman.body.on_ground = e["on_ground"].as_bool().unwrap();
                        enderman.persistence_required = true;
                        enderman.yaw = number(&e["yaw"]) as f32;
                        world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
                        let id = world.spawn_enderman(enderman, false);
                        ids.insert(tag.clone(), id);
                    }
                    let _ = world.take_sounds();
                    continue;
                }
                let players: Vec<PlayerCandidate> = probes.values().map(|p| p.0).collect();
                world.tick_with_players(&mut scene, &players);
                assert_eq!(world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                let (compared, ours) = sounds::check_heard(&mut world, data, &format!("{scenario} tick {tick}"), &sounds::pens(&suite, &scenario));
                heard += compared;
                // Each teleport plays its sound twice.
                teleports += ours.iter().filter(|s| s.0 == "minecraft:entity.enderman.teleport").count() / 2;
                hits += world.take_player_hits().len();
                for (tag, states) in observed {
                    let id = ids[tag];
                    let states = states.as_array().unwrap();
                    let ours = world.endermen().iter().find(|e| e.id == id);
                    assert_eq!(states.len(), usize::from(ours.is_some()), "{scenario} tick {tick} {tag} presence");
                    let Some(entity) = ours else { continue };
                    let e = &states[0];
                    let at = format!("{scenario} tick {tick} {tag}");
                    let ai = &entity.ai;
                    let (body, state) = (&entity.enderman.body, &ai.state);
                    for (field, actual) in [
                        ("x", body.position.x),
                        ("y", body.position.y),
                        ("z", body.position.z),
                        ("vx", body.velocity.x),
                        ("vy", body.velocity.y),
                        ("vz", body.velocity.z),
                        ("health", f64::from(entity.enderman.health)),
                        ("yaw", f64::from(ai.yaw)),
                        ("speed", f64::from(ai.speed)),
                        ("head_yaw", f64::from(state.look_control.head_yaw)),
                        ("body_yaw", f64::from(ai.body_rotation.body_yaw)),
                        ("pitch", f64::from(state.look_control.pitch)),
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
                    let goals: Vec<&str> = e["running_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                    assert_eq!(goals, ai.running_goals(), "{at} goals");
                    let targets: Vec<&str> = e["running_target_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                    assert_eq!(targets, ai.running_targets(), "{at} target goals");
                    assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random");
                    goals_seen.extend(ai.running_goals());
                    goals_seen.extend(ai.running_targets());
                    assert_eq!(e.get("target_uuid").is_some_and(|t| !t.is_null()), state.target().is_some(), "{at} target");
                    assert_eq!(e["aggressive"], state.melee.aggressive, "{at} aggressive");
                    assert_eq!(e["move_control_wanted"], ai.move_control.has_wanted(), "{at} move wanted");
                    assert_eq!(e["look_control_wanted"], state.look_control.cooldown > 0, "{at} look wanted");
                    assert_eq!(e["navigation_done"], state.navigation.is_done(), "{at} navigation done");
                    let nodes: Vec<(i32, i32, i32)> = e["path_nodes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|n| (n[0].as_i64().unwrap() as i32, n[1].as_i64().unwrap() as i32, n[2].as_i64().unwrap() as i32))
                        .collect();
                    assert_eq!(nodes, state.navigation.nodes, "{at} path");
                    let ender = &state.enderman;
                    assert_eq!(e["carried_block"].as_str(), ender.carried.as_ref().map(|b| b.id.as_str()), "{at} carried");
                    assert_eq!(e["creepy"], ender.creepy, "{at} creepy");
                    assert_eq!(e["stared_at"], ender.stared_at, "{at} stared at");
                    assert_eq!(e["anger_end_time"].as_i64().unwrap(), ender.anger_end_time, "{at} anger end");
                    assert_eq!(e.get("angry_at").is_some_and(|t| !t.is_null()), ender.anger_target.is_some(), "{at} angry at");
                    assert_eq!(e["target_change_time"].as_i64().unwrap(), i64::from(ender.target_change_time), "{at} target change");
                    taken += usize::from(ender.carried.is_some());
                    stared += usize::from(ender.stared_at);
                }
                if let Some(blocks) = data["blocks"].as_object() {
                    for (key, block) in blocks {
                        let p: Vec<i32> = key.split(',').map(|v| v.parse().unwrap()).collect();
                        let vanilla = block["id"].as_str().unwrap();
                        let ours = scene.block((p[0], p[1], p[2])).map_or_else(|| "minecraft:air".to_owned(), |b| b.id);
                        assert_eq!(ours, vanilla, "{scenario} tick {tick} block {key}");
                    }
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert!(taken > 0, "an enderman takes a block");
    assert!(stared > 0, "a stare sets one off");
    assert!(teleports > 0, "endermen teleport");
    println!("{frames} exact enderman frames matched ({heard} sounds, {teleports} teleports, {hits} hits on probes, {taken} carrying enderman-ticks, {stared} stared-at enderman-ticks; goals seen: {goals_seen:?})");
}
