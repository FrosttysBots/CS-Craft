//! Exact pinned 26.3 unfiltered skeleton gate
//! (`scenarios/mobs/skeleton-full-ai.json`): bow skeletons with every
//! registered goal. One hunts a probe player at midnight (approach,
//! strafing, draws and shots with a seeded shoot random), one idles near a
//! probe hidden behind a wall, and one burns under the noon sun with no
//! shade (`RestrictSunGoal`, `FleeSunGoal`'s tries). The skeletons run on
//! the monster goal framework; every tick compares position, motion,
//! rotations, controls, path, goals, target, the bow's draw, fire, health
//! and the mob random.
#[path = "support/sounds.rs"]
mod sounds;
use glam::DVec3;
use minecraftoss_entities::{skeleton::Skeleton, tempt::PlayerCandidate, world::EntityWorld};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

/// The scene's blocks, lit as at midnight (uniformly dim) or noon (full
/// sky light under open sky, none beneath blocks).
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
        match (self.noon, self.can_see_sky(pos)) {
            (true, true) => 0.5,
            (true, false) => -0.5,
            (false, _) => 0.0,
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
    let path = env::args().nth(1).expect("usage: check_skeleton_full_ai TRACE.jsonl");
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut probes: Vec<PlayerCandidate> = Vec::new();
    let mut scenario = String::new();
    let (mut frames, mut shots, mut burning) = (0, 0, 0);
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
                world.set_monsters_burn(scene.noon);
                ids.clear();
                probes.clear();
            }
            "entity_set_random_seed" => {
                let id = ids[row["data"]["tag"].as_str().unwrap()];
                world.skeleton_mut(id).unwrap().set_random_seed(row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "projectile_shoot_seed" => world.set_arrow_shoot_seed(Some(row["data"]["seed"].as_str().unwrap().parse().unwrap())),
            "player_probe" => {
                let pos = &row["data"]["pos"];
                probes.push(probe(1_000_000, DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap())));
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = data["entities"].as_object().unwrap();
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    for (tag, states) in observed {
                        let e = &states[0];
                        let mut skeleton = Skeleton::new(DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"])));
                        skeleton.body.on_ground = e["on_ground"].as_bool().unwrap();
                        skeleton.persistence_required = true;
                        world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
                        let id = world.spawn_skeleton_active(skeleton, number(&e["yaw"]) as f32);
                        ids.insert(tag.clone(), id);
                    }
                    continue;
                }
                let ticks = observed.iter().find_map(|(tag, states)| {
                    let e = states.as_array().unwrap().first()?;
                    let ours = world.skeletons().iter().find(|s| s.id == ids[tag])?;
                    Some(e["entity_tick_count"].as_i64().unwrap() > i64::from(ours.tick_count))
                });
                if ticks.unwrap_or(true) {
                    world.tick_with_players(&mut scene, &probes);
                } else {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                }
                assert_eq!(world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                let (compared, ours) = sounds::check_heard(&mut world, data, &format!("{scenario} tick {tick}"), &sounds::pens(&suite, &scenario));
                heard += compared;
                shots += ours.iter().filter(|s| s.0 == "minecraft:entity.skeleton.shoot").count();
                for (tag, states) in observed {
                    let id = ids[tag];
                    let ours = world.skeletons().iter().find(|s| s.id == id);
                    let states = states.as_array().unwrap();
                    assert_eq!(states.len(), usize::from(ours.is_some()), "{scenario} tick {tick} {tag} presence");
                    let Some(entity) = ours else { continue };
                    let e = &states[0];
                    let at = format!("{scenario} tick {tick} {tag}");
                    let ai = entity.ai.as_deref().unwrap();
                    let (body, state) = (&entity.skeleton.body, &ai.state);
                    for (field, actual) in [
                        ("x", body.position.x),
                        ("y", body.position.y),
                        ("z", body.position.z),
                        ("vx", body.velocity.x),
                        ("vy", body.velocity.y),
                        ("vz", body.velocity.z),
                        ("health", f64::from(entity.skeleton.health)),
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
                    assert_eq!(e["remaining_fire_ticks"], entity.skeleton.body.fire_ticks, "{at} fire");
                    assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random");
                    let goals: Vec<&str> = e["running_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                    assert_eq!(goals, ai.running_goals(), "{at} goals");
                    goals_seen.extend(ai.running_goals());
                    assert_eq!(e.get("target_uuid").is_some_and(|t| !t.is_null()), state.target().is_some(), "{at} target");
                    assert_eq!(e["aggressive"], state.melee.aggressive, "{at} aggressive");
                    assert_eq!(e["using_item"], state.bow.using_item, "{at} using bow");
                    assert_eq!(e["ticks_using_item"], if state.bow.using_item { state.bow.ticks_using_item } else { 0 }, "{at} draw");
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
                    if entity.skeleton.body.fire_ticks > 0 {
                        burning += 1;
                    }
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert!(shots >= 3, "the hunter shoots");
    println!("{frames} exact unfiltered skeleton frames matched ({shots} shots, {burning} burning skeleton-ticks, {heard} sounds; goals seen: {goals_seen:?})");
}
