//! Isolated FollowParentGoal through shared navigation/control against pinned 26.3.
use glam::DVec3;
use minecraftoss_entities::{chicken::Chicken, world::EntityWorld};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

#[derive(Default)]
struct Scene(BTreeMap<Pos, Block>);
impl World for Scene {
    fn block(&self, p: Pos) -> Option<Block> {
        self.0.get(&p).cloned()
    }
    fn set_block(&mut self, p: Pos, b: Option<Block>) {
        if let Some(b) = b {
            self.0.insert(p, b);
        } else {
            self.0.remove(&p);
        }
    }
}
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap())
}
fn exact(expected: &Value, actual: f64, context: &str) {
    assert_eq!(
        actual.to_bits(),
        number(expected).to_bits(),
        "{context}: Rust {actual:?}, Java {:?}",
        number(expected)
    );
}
fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_chicken_follow_parent TRACE.jsonl");
    let trace = fs::read_to_string(path).unwrap();
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids = BTreeMap::<String, u64>::new();
    let mut current_tick = 0;
    let mut frames = 0;
    let mut actions = 0;
    let mut seeds = 0;
    let mut complete = false;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        if row["type"] == "complete" {
            complete = true;
        }
        if row["type"] == "entity_keep_goal" {
            let data = &row["data"];
            assert_eq!(data["tag"], "child");
            assert_eq!(data["goal_class"], "FollowParentGoal");
            assert_eq!(data["removed"], 7);
            world
                .chicken_mut(ids["child"])
                .unwrap()
                .retain_goals(&["FollowParentGoal"]);
            actions += 1;
        }
        if row["type"] == "entity_set_random_seed" {
            let tag = row["data"]["tag"].as_str().unwrap();
            let seed = row["data"]["seed"]
                .as_str()
                .unwrap()
                .parse::<i64>()
                .unwrap();
            world.chicken_mut(ids[tag]).unwrap().set_random_seed(seed);
            seeds += 1;
        }
        if row["type"] != "snapshot" {
            continue;
        }
        let tick = row["tick"].as_i64().unwrap();
        if tick == 0 {
            for (key, value) in row["data"]["blocks"].as_object().unwrap() {
                let pos: Vec<i32> = key.split(',').map(|v| v.parse().unwrap()).collect();
                let id = value["id"].as_str().unwrap();
                if id != "minecraft:air" {
                    scene.set_block((pos[0], pos[1], pos[2]), Some(Block::new(id)));
                }
            }
            for (tag, group) in row["data"]["entities"].as_object().unwrap() {
                let e = &group.as_array().unwrap()[0];
                let mut chicken = Chicken::new(DVec3::new(
                    number(&e["x"]),
                    number(&e["y"]),
                    number(&e["z"]),
                ));
                chicken.body.velocity =
                    DVec3::new(number(&e["vx"]), number(&e["vy"]), number(&e["vz"]));
                chicken.body.on_ground = e["on_ground"].as_bool().unwrap();
                chicken.yaw = number(&e["yaw"]) as f32;
                chicken.speed = number(&e["speed"]) as f32;
                chicken.age.ticks = e["age"].as_i64().unwrap() as i32;
                chicken.egg_time = e["egg_time"].as_i64().unwrap() as i32;
                ids.insert(
                    tag.to_owned(),
                    world.spawn_chicken(chicken, e["no_ai"].as_bool().unwrap()),
                );
            }
        } else {
            assert_eq!(tick, current_tick + 1);
            world.tick(&mut scene);
            for (tag, group) in row["data"]["entities"].as_object().unwrap() {
                let e = &group.as_array().unwrap()[0];
                let chicken = &world
                    .chickens()
                    .iter()
                    .find(|c| c.id == ids[tag])
                    .unwrap()
                    .chicken;
                let entity = world.chickens().iter().find(|c| c.id == ids[tag]).unwrap();
                assert_eq!(
                    entity.id as i64,
                    e["entity_numeric_id"].as_i64().unwrap(),
                    "tick {tick} {tag} numeric id"
                );
                assert_eq!(
                    entity.tick_count as i64,
                    e["entity_tick_count"].as_i64().unwrap(),
                    "tick {tick} {tag} tick count"
                );
                assert_eq!(
                    chicken.age.ticks as i64,
                    e["age"].as_i64().unwrap(),
                    "tick {tick} {tag} age"
                );
                assert_eq!(
                    chicken.egg_time as i64,
                    e["egg_time"].as_i64().unwrap(),
                    "tick {tick} {tag} egg timer"
                );
                for (name, actual) in [
                    ("flap", f64::from(chicken.flap)),
                    ("flap_speed", f64::from(chicken.flap_speed)),
                    ("flapping", f64::from(chicken.flapping)),
                ] {
                    exact(&e[name], actual, &format!("tick {tick} {tag} {name}"));
                }
                let eye_height: f32 = if chicken.age.baby() { 0.28125 } else { 0.644 };
                exact(
                    &e["eye_y"],
                    chicken.body.position.y + f64::from(eye_height),
                    &format!("tick {tick} {tag} eye y"),
                );
                for (name, actual) in [
                    ("head_yaw", f64::from(entity.look_control.head_yaw)),
                    ("body_yaw", f64::from(entity.body_rotation.body_yaw)),
                    ("pitch", f64::from(entity.look_control.pitch)),
                    ("look_control_x", entity.look_control.wanted.x),
                    ("look_control_y", entity.look_control.wanted.y),
                    ("look_control_z", entity.look_control.wanted.z),
                ] {
                    exact(&e[name], actual, &format!("tick {tick} {tag} {name}"));
                }
                assert_eq!(
                    entity.look_control.cooldown > 0,
                    e["look_control_wanted"].as_bool().unwrap(),
                    "tick {tick} {tag} look wanted"
                );
                assert_eq!(
                    entity.ambient_sound_time as i64,
                    e["ambient_sound_time"].as_i64().unwrap(),
                    "tick {tick} {tag} ambient timer"
                );
                if let Some(no_action) = e.get("no_action_time") {
                    assert_eq!(
                        entity.no_action_time as i64,
                        no_action.as_i64().unwrap(),
                        "tick {tick} {tag} inactivity"
                    );
                }
                let expected_goals = e["running_goals"].as_array().unwrap();
                assert_eq!(
                    entity.follow_parent.running,
                    expected_goals.iter().any(|goal| goal == "FollowParentGoal"),
                    "tick {tick} {tag} running goal"
                );
                assert_eq!(
                    expected_goals.len(),
                    usize::from(entity.follow_parent.running),
                    "tick {tick} {tag} other goals"
                );
                for (name, actual) in [
                    ("x", chicken.body.position.x),
                    ("y", chicken.body.position.y),
                    ("z", chicken.body.position.z),
                    ("vx", chicken.body.velocity.x),
                    ("vy", chicken.body.velocity.y),
                    ("vz", chicken.body.velocity.z),
                    ("yaw", chicken.yaw as f64),
                    ("speed", chicken.speed as f64),
                ] {
                    exact(&e[name], actual, &format!("tick {tick} {tag} {name}"));
                }
                assert_eq!(
                    chicken.body.on_ground,
                    e["on_ground"].as_bool().unwrap(),
                    "tick {tick} {tag} on_ground"
                );
                assert_eq!(
                    chicken.move_control.has_wanted(),
                    e["move_control_wanted"].as_bool().unwrap(),
                    "tick {tick} {tag} wanted"
                );
                assert_eq!(
                    chicken.navigation.is_done(),
                    e["navigation_done"].as_bool().unwrap(),
                    "tick {tick} navigation done"
                );
                assert_eq!(
                    chicken.navigation.nodes.len(),
                    e["path_node_count"].as_u64().unwrap() as usize,
                    "tick {tick} node count"
                );
                assert_eq!(
                    chicken.navigation.observed_next(),
                    e["path_next_node"].as_i64().unwrap() as i32,
                    "tick {tick} next node"
                );
                assert_eq!(
                    chicken.navigation.reached && !chicken.navigation.nodes.is_empty(),
                    e["path_reached"].as_bool().unwrap(),
                    "tick {tick} {tag} path reached"
                );
                let expected_nodes: Vec<(i32, i32, i32)> = e["path_nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| {
                        (
                            p[0].as_i64().unwrap() as i32,
                            p[1].as_i64().unwrap() as i32,
                            p[2].as_i64().unwrap() as i32,
                        )
                    })
                    .collect();
                assert_eq!(
                    chicken.navigation.nodes, expected_nodes,
                    "tick {tick} {tag} path nodes"
                );
                frames += 1;
            }
        }
        current_tick = tick;
    }
    assert!(complete && actions == 1 && seeds == 2 && frames == 180);
    println!("{actions} retained goals and {frames} chicken follow-parent/navigation/control/motion frames matched exactly");
}
