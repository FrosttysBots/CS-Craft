//! Exact pinned 26.3 unfiltered spider gate
//! (`scenarios/mobs/spider-full-ai.json`): spiders with every registered
//! goal. One hunts a probe player at midnight (approach, `LeapAtTargetGoal`,
//! bites), one climbs the wall below a probe on a ledge it cannot path up
//! (`WallClimberNavigation` heads for the probe's block), one hit by a probe
//! at noon gives up its attack in the light while its hurt-by goal hands the
//! target back, beside one that never targets a probe in the light, and one
//! idles near a probe hidden behind a wall. The spiders run on the monster
//! goal framework; every tick compares position, motion, rotations,
//! controls, path, goals, target, the climbing flag, health and the mob
//! random, and each bite (a swing starting that tick) with a hit on a probe.
#[path = "support/sounds.rs"]
mod sounds;
use glam::DVec3;
use minecraftoss_entities::{
    spider::Spider,
    tempt::PlayerCandidate,
    world::{EntityWorld, PlayerAttack},
};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

/// The scene's blocks, lit as at midnight (sky light 15 less 11: every
/// spot too dark for a spider) or noon (full light under open sky).
#[derive(Default)]
struct Scene {
    blocks: BTreeMap<Pos, Block>,
    noon: bool,
}
impl World for Scene {
    fn step_sound(&self, pos: Pos) -> Option<(String, f32, f32)> {
        sounds::step_sound(&self.blocks.get(&pos)?.id)
    }
    fn block(&self, pos: Pos) -> Option<Block> {
        self.blocks.get(&pos).cloned()
    }
    fn set_block(&mut self, pos: Pos, block: Option<Block>) {
        if let Some(block) = block {
            self.blocks.insert(pos, block);
        } else {
            self.blocks.remove(&pos);
        }
    }
    fn can_see_sky(&self, (x, y, z): Pos) -> bool {
        !self.blocks.keys().any(|&(bx, by, bz)| bx == x && bz == z && by > y)
    }
    fn light_path_cost(&self, pos: Pos) -> f32 {
        // `LightTexture.getBrightness` at level 4: 4/15 / (4 - 3 * 4/15).
        let night = (4.0_f32 / 15.0) / (4.0 - 3.0 * (4.0_f32 / 15.0)) - 0.5;
        match (self.noon, self.can_see_sky(pos)) {
            (true, true) => 0.5,
            (true, false) => -0.5,
            (false, _) => night,
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
    let mut heard = 0;
    let path = env::args().nth(1).expect("usage: check_spider_full_ai TRACE.jsonl");
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut probes: Vec<PlayerCandidate> = Vec::new();
    let mut probe_ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut scenario = String::new();
    let (mut frames, mut bites, mut climbing, mut leaps) = (0, 0, 0, 0);
    let mut goals_seen = std::collections::BTreeSet::new();
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error", "{row}");
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
                probe_ids.clear();
            }
            "entity_set_random_seed" => {
                let id = ids[row["data"]["tag"].as_str().unwrap()];
                world.spider_mut(id).unwrap().set_random_seed(row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "player_probe" => {
                let data = &row["data"];
                let pos = &data["pos"];
                let id = 1_000_000 + probes.len() as u64;
                probe_ids.insert(data["tag"].as_str().unwrap().to_owned(), id);
                probes.push(probe(id, DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap())));
            }
            "player_attack" => {
                let data = &row["data"];
                let player = probes.iter().find(|p| p.id == probe_ids[data["probe"].as_str().unwrap()]).copied().unwrap();
                let attack = PlayerAttack {
                    player_id: player.id,
                    position: player.position,
                    yaw: 0.0,
                    attack_damage: number(&data["attack_damage"]),
                    strength: number(&data["strength"]) as f32,
                    sprinting: false,
                    can_critical: false,
                    can_sweep: false,
                };
                let target = ids[data["target"].as_str().unwrap()];
                assert!(world.player_attack(&attack, target).hurt, "{scenario} the probe's hit lands");
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = data["entities"].as_object().unwrap();
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    for (tag, states) in observed {
                        let e = &states[0];
                        let mut spider = Spider::new(DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"])));
                        spider.body.on_ground = e["on_ground"].as_bool().unwrap();
                        spider.persistence_required = true;
                        spider.yaw = number(&e["yaw"]) as f32;
                        world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
                        let id = world.spawn_spider(spider, false);
                        assert_eq!(id, e["entity_numeric_id"].as_u64().unwrap());
                        ids.insert(tag.clone(), id);
                    }
                    continue;
                }
                let ticks = observed.iter().find_map(|(tag, states)| {
                    let e = states.as_array().unwrap().first()?;
                    let ours = world.spiders().iter().find(|s| s.id == ids[tag])?;
                    Some(e["entity_tick_count"].as_i64().unwrap() > i64::from(ours.tick_count))
                });
                if ticks.unwrap_or(true) {
                    world.tick_with_players(&mut scene, &probes);
                } else {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                }
                assert_eq!(world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                heard += sounds::check(&mut world, data, &format!("{scenario} tick {tick}"), &sounds::pens(&suite, &scenario));
                let hits = world.take_player_hits().len();
                let mut swings = 0;
                for (tag, states) in observed {
                    let id = ids[tag];
                    let ours = world.spiders().iter().find(|s| s.id == id);
                    let states = states.as_array().unwrap();
                    assert_eq!(states.len(), usize::from(ours.is_some()), "{scenario} tick {tick} {tag} presence");
                    let Some(entity) = ours else { continue };
                    let e = &states[0];
                    let at = format!("{scenario} tick {tick} {tag}");
                    let ai = &entity.ai;
                    let (body, state) = (&entity.spider.body, &ai.state);
                    for (field, actual) in [
                        ("x", body.position.x),
                        ("y", body.position.y),
                        ("z", body.position.z),
                        ("vx", body.velocity.x),
                        ("vy", body.velocity.y),
                        ("vz", body.velocity.z),
                        ("health", f64::from(entity.spider.health)),
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
                    assert_eq!(e["spider_climbing"], body.climbing, "{at} climbing");
                    assert_eq!(e["entity_tick_count"], entity.tick_count, "{at} tick count");
                    assert_eq!(e["ambient_sound_time"], entity.ambient_sound_time, "{at} ambient time");
                    assert_eq!(e["no_action_time"], entity.no_action_time, "{at} idle time");
                    assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random");
                    let goals: Vec<&str> = e["running_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                    assert_eq!(goals, ai.running_goals(), "{at} goals");
                    goals_seen.extend(ai.running_goals());
                    assert_eq!(e.get("target_uuid").is_some_and(|t| !t.is_null()), state.target().is_some(), "{at} target");
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
                    if e["swinging"] == true && e["swing_ticks"] == 0 {
                        swings += 1;
                    }
                    climbing += usize::from(body.climbing);
                    leaps += usize::from(ai.running_goals().contains(&"LeapAtTargetGoal"));
                }
                assert_eq!(swings, hits, "{scenario} tick {tick} bites");
                bites += hits;
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert!(bites >= 3 && climbing > 0 && leaps > 0, "the spiders bite, climb and leap");
    println!("{frames} exact unfiltered spider frames matched ({bites} bites, {climbing} climbing spider-ticks, {leaps} leaping spider-ticks, {heard} sounds; goals seen: {goals_seen:?})");
}
