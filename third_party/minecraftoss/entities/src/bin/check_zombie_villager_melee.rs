//! Exact pinned 26.3 zombie versus stationary villager combat gate.
use glam::DVec3;
use minecraftoss_entities::{
    villager::Villager,
    world::EntityWorld,
    zombie::{Zombie, ZombieKind},
};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

#[derive(Default)]
struct Scene(BTreeMap<Pos, Block>);
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
}
fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}
fn exact(observed: &Value, actual: f64, label: &str, tick: i64) {
    assert_eq!(
        actual.to_bits(),
        number(observed).to_bits(),
        "tick {tick} {label}"
    );
}
fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_zombie_villager_melee TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut zombie_id = None;
    let mut victim_id = None;
    let mut frames = 0;
    let mut expected_frames = 0;
    let mut complete = false;
    let mut kind = ZombieKind::Zombie;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => {
                expected_frames = row["data"]["suite"]["scenarios"][0]["ticks"]
                    .as_u64()
                    .unwrap() as usize;
                let (entity_kind, bright_outside) = match row["data"]["suite"]["scenarios"][0]["id"]
                    .as_str()
                {
                    Some("zombie_melee_against_stationary_villager") => (ZombieKind::Zombie, false),
                    Some("drowned_melee_against_stationary_villager") => {
                        (ZombieKind::Drowned, false)
                    }
                    Some("drowned_daylight_target_gate") => (ZombieKind::Drowned, true),
                    Some("drowned_underwater_villager_melee_daylight") => {
                        (ZombieKind::Drowned, true)
                    }
                    Some("drowned_underwater_elevated_villager_daylight") => {
                        (ZombieKind::Drowned, true)
                    }
                    other => panic!("unexpected melee scenario {other:?}"),
                };
                kind = entity_kind;
                world.set_bright_outside(bright_outside);
            }
            "complete" => complete = true,
            "entity_set_random_seed" => {
                let seed = row["data"]["seed"].as_str().unwrap().parse().unwrap();
                let tag = row["data"]["tag"].as_str().unwrap();
                if tag
                    == if kind == ZombieKind::Drowned {
                        "drowned"
                    } else {
                        "zombie"
                    }
                {
                    world
                        .zombie_mut(zombie_id.unwrap())
                        .unwrap()
                        .set_random_seed(seed);
                } else if tag == "victim" {
                    world
                        .villager_mut(victim_id.unwrap())
                        .unwrap()
                        .set_random_seed(seed);
                } else {
                    panic!("unexpected tag {tag}");
                }
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let z = &data["entities"][if kind == ZombieKind::Drowned {
                    "drowned"
                } else {
                    "zombie"
                }][0];
                let v = &data["entities"]["victim"][0];
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    for (key, block) in data["blocks"].as_object().unwrap() {
                        let pos: Vec<i32> =
                            key.split(',').map(|part| part.parse().unwrap()).collect();
                        if block["id"] != "minecraft:air" {
                            scene.set_block(
                                (pos[0], pos[1], pos[2]),
                                Some(Block::new(block["id"].as_str().unwrap())),
                            );
                        }
                    }
                    let mut zombie = Zombie::new(DVec3::new(
                        number(&z["x"]),
                        number(&z["y"]),
                        number(&z["z"]),
                    ));
                    zombie.body.velocity =
                        DVec3::new(number(&z["vx"]), number(&z["vy"]), number(&z["vz"]));
                    zombie.body.on_ground = z["on_ground"].as_bool().unwrap();
                    zombie.persistence_required = z["persistence_required"].as_bool().unwrap();
                    zombie_id = Some(if kind == ZombieKind::Drowned {
                        world.spawn_drowned_pursuit(zombie)
                    } else {
                        world.spawn_zombie_pursuit(zombie)
                    });
                    let mut villager = Villager::new(DVec3::new(
                        number(&v["x"]),
                        number(&v["y"]),
                        number(&v["z"]),
                    ));
                    villager.body.velocity =
                        DVec3::new(number(&v["vx"]), number(&v["vy"]), number(&v["vz"]));
                    villager.body.on_ground = v["on_ground"].as_bool().unwrap();
                    villager.persistence_required = v["persistence_required"].as_bool().unwrap();
                    villager.set_age(v["age"].as_i64().unwrap() as i32);
                    victim_id = Some(world.spawn_villager(villager, true));
                } else {
                    world.tick(&mut scene);
                    assert_eq!(
                        world.game_time(),
                        data["game_time"].as_i64().unwrap(),
                        "tick {tick} game time"
                    );
                    let zombie = world
                        .zombies()
                        .iter()
                        .find(|entity| Some(entity.id) == zombie_id)
                        .unwrap();
                    let villager = world
                        .villagers()
                        .iter()
                        .find(|entity| Some(entity.id) == victim_id)
                        .unwrap();
                    assert_eq!(zombie.zombie.kind, kind, "tick {tick} kind");
                    assert_eq!(
                        z["type"],
                        if kind == ZombieKind::Drowned {
                            "minecraft:drowned"
                        } else {
                            "minecraft:zombie"
                        },
                        "tick {tick} zombie type"
                    );
                    assert_eq!(v["type"], "minecraft:villager", "tick {tick} villager type");
                    assert_eq!(z["entity_numeric_id"], zombie.id, "tick {tick} zombie id");
                    assert_eq!(
                        v["entity_numeric_id"], villager.id,
                        "tick {tick} villager id"
                    );
                    assert_eq!(
                        z["entity_tick_count"], zombie.tick_count,
                        "tick {tick} zombie tick count"
                    );
                    assert_eq!(
                        v["entity_tick_count"], villager.tick_count,
                        "tick {tick} villager tick count"
                    );
                    assert_eq!(z["no_ai"], zombie.no_ai, "tick {tick} zombie NoAI");
                    assert_eq!(v["no_ai"], villager.no_ai, "tick {tick} villager NoAI");
                    assert_eq!(
                        z["persistence_required"], zombie.zombie.persistence_required,
                        "tick {tick} zombie persistence"
                    );
                    assert_eq!(
                        v["persistence_required"], villager.villager.persistence_required,
                        "tick {tick} villager persistence"
                    );
                    assert_eq!(
                        z["alive"],
                        zombie.zombie.health > 0.0,
                        "tick {tick} zombie alive"
                    );
                    assert_eq!(
                        v["alive"],
                        villager.villager.health > 0.0,
                        "tick {tick} villager alive"
                    );
                    assert_eq!(
                        z["zombie_baby"], zombie.zombie.baby,
                        "tick {tick} zombie baby"
                    );
                    assert_eq!(
                        z["zombie_can_break_doors"], zombie.zombie.can_break_doors,
                        "tick {tick} zombie doors"
                    );
                    assert_eq!(
                        z["zombie_underwater_converting"], zombie.zombie.underwater_converting,
                        "tick {tick} zombie conversion"
                    );
                    assert_eq!(
                        z["random_state"].as_str().unwrap().parse::<u64>().unwrap(),
                        zombie.random.raw_state(),
                        "tick {tick} zombie RNG"
                    );
                    assert_eq!(
                        v["random_state"].as_str().unwrap().parse::<u64>().unwrap(),
                        villager.random.raw_state(),
                        "tick {tick} villager RNG"
                    );
                    assert_eq!(
                        z["target_uuid"].is_string(),
                        zombie.target_villager_id.is_some(),
                        "tick {tick} target"
                    );
                    if zombie.target_villager_id.is_some() {
                        assert_eq!(z["target_uuid"], v["uuid"], "tick {tick} target UUID");
                    }
                    assert_eq!(
                        z["running_goals"].as_array().unwrap().len() == 1,
                        zombie.attack_goal_running,
                        "tick {tick} goal"
                    );
                    if zombie.attack_goal_running {
                        assert_eq!(
                            z["running_goals"],
                            serde_json::json!([if kind == ZombieKind::Drowned {
                                "DrownedAttackGoal"
                            } else {
                                "ZombieAttackGoal"
                            }]),
                            "tick {tick} goal identity"
                        );
                    }
                    assert_eq!(z["aggressive"], zombie.aggressive, "tick {tick} aggressive");
                    assert_eq!(
                        z["navigation_done"],
                        zombie.navigation.is_done(),
                        "tick {tick} navigation"
                    );
                    assert_eq!(
                        z["path_next_node"],
                        zombie.navigation.observed_next(),
                        "tick {tick} path next"
                    );
                    assert_eq!(
                        z["path_node_count"],
                        zombie.navigation.nodes.len(),
                        "tick {tick} path count"
                    );
                    assert_eq!(
                        z["path_reached"], zombie.navigation.reached,
                        "tick {tick} path reached"
                    );
                    assert_eq!(
                        z["move_control_wanted"],
                        zombie.move_control.has_wanted(),
                        "tick {tick} move wanted"
                    );
                    assert_eq!(
                        z["look_control_wanted"],
                        zombie.look_control.cooldown > 0,
                        "tick {tick} look wanted"
                    );
                    let nodes: Vec<_> = z["path_nodes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|node| {
                            (
                                node[0].as_i64().unwrap() as i32,
                                node[1].as_i64().unwrap() as i32,
                                node[2].as_i64().unwrap() as i32,
                            )
                        })
                        .collect();
                    assert_eq!(nodes, zombie.navigation.nodes, "tick {tick} path");
                    assert_eq!(
                        z["on_ground"], zombie.zombie.body.on_ground,
                        "tick {tick} zombie ground"
                    );
                    assert_eq!(
                        v["on_ground"], villager.villager.body.on_ground,
                        "tick {tick} villager ground"
                    );
                    assert_eq!(
                        z["ambient_sound_time"], zombie.ambient_sound_time,
                        "tick {tick} zombie ambient"
                    );
                    assert_eq!(
                        v["ambient_sound_time"], villager.ambient_sound_time,
                        "tick {tick} villager ambient"
                    );
                    assert_eq!(
                        z["no_action_time"], zombie.no_action_time,
                        "tick {tick} zombie inactivity"
                    );
                    assert_eq!(
                        v["no_action_time"], villager.no_action_time,
                        "tick {tick} villager inactivity"
                    );
                    assert_eq!(
                        v["age"], villager.villager.age.ticks,
                        "tick {tick} villager age"
                    );
                    assert_eq!(
                        v["forced_age"], villager.villager.age.forced,
                        "tick {tick} forced age"
                    );
                    assert_eq!(
                        v["forced_age_timer"], villager.villager.age.forced_particle_ticks,
                        "tick {tick} forced age timer"
                    );
                    assert_eq!(
                        v["age_locked"], villager.villager.age.locked,
                        "tick {tick} age lock"
                    );
                    for key in ["aggressive", "move_control_wanted", "look_control_wanted"] {
                        assert_eq!(v[key], false, "tick {tick} villager {key}");
                    }
                    assert_eq!(
                        v["running_goals"],
                        serde_json::json!([]),
                        "tick {tick} villager goals"
                    );
                    assert_eq!(
                        v["navigation_done"], true,
                        "tick {tick} villager navigation"
                    );
                    assert_eq!(v["path_node_count"], 0, "tick {tick} villager path count");
                    assert_eq!(v["path_next_node"], -1, "tick {tick} villager path next");
                    assert_eq!(
                        v["path_reached"], false,
                        "tick {tick} villager path reached"
                    );
                    assert_eq!(
                        v["path_nodes"],
                        serde_json::json!([]),
                        "tick {tick} villager path"
                    );
                    for (label, state, position, velocity, health, eye_height) in [
                        (
                            "zombie",
                            z,
                            zombie.zombie.body.position,
                            zombie.zombie.body.velocity,
                            zombie.zombie.health,
                            zombie.zombie.eye_height(),
                        ),
                        (
                            "victim",
                            v,
                            villager.villager.body.position,
                            villager.villager.body.velocity,
                            villager.villager.health,
                            villager.villager.eye_height(),
                        ),
                    ] {
                        for (field, actual) in [
                            ("x", position.x),
                            ("y", position.y),
                            ("z", position.z),
                            ("vx", velocity.x),
                            ("vy", velocity.y),
                            ("vz", velocity.z),
                            ("health", f64::from(health)),
                            ("eye_y", position.y + f64::from(eye_height)),
                        ] {
                            exact(&state[field], actual, &format!("{label} {field}"), tick);
                        }
                    }
                    for (field, actual) in [
                        ("yaw", f64::from(zombie.yaw)),
                        ("speed", f64::from(zombie.speed)),
                        ("head_yaw", f64::from(zombie.look_control.head_yaw)),
                        ("body_yaw", f64::from(zombie.body_rotation.body_yaw)),
                        ("pitch", f64::from(zombie.look_control.pitch)),
                        ("move_control_x", zombie.move_control.wanted.x),
                        ("move_control_y", zombie.move_control.wanted.y),
                        ("move_control_z", zombie.move_control.wanted.z),
                        ("look_control_x", zombie.look_control.wanted.x),
                        ("look_control_y", zombie.look_control.wanted.y),
                        ("look_control_z", zombie.look_control.wanted.z),
                    ] {
                        exact(&z[field], actual, field, tick);
                    }
                    for field in [
                        "yaw",
                        "speed",
                        "head_yaw",
                        "body_yaw",
                        "pitch",
                        "move_control_x",
                        "move_control_y",
                        "move_control_z",
                        "look_control_x",
                        "look_control_y",
                        "look_control_z",
                    ] {
                        exact(&v[field], 0.0, &format!("victim {field}"), tick);
                    }
                    frames += 1;
                }
            }
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!(frames, expected_frames);
    println!("{frames} exact {kind:?}-villager interaction frames matched");
}
