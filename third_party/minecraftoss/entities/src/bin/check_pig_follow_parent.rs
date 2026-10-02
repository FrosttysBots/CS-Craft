//! Isolated FollowParentGoal through shared navigation/control against pinned 26.3.
use glam::DVec3;
use minecraftoss_entities::{pig::Pig, world::EntityWorld};
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
        .expect("usage: check_pig_follow_parent TRACE.jsonl");
    let trace = fs::read_to_string(path).unwrap();
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids = BTreeMap::<String, u64>::new();
    let mut current_tick = 0;
    let mut frames = 0;
    let mut actions = 0;
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
            assert_eq!(data["removed"], 8);
            world
                .pig_mut(ids["child"])
                .unwrap()
                .retain_goals(&["FollowParentGoal"]);
            actions += 1;
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
                let mut pig = Pig::new(DVec3::new(
                    number(&e["x"]),
                    number(&e["y"]),
                    number(&e["z"]),
                ));
                pig.body.velocity =
                    DVec3::new(number(&e["vx"]), number(&e["vy"]), number(&e["vz"]));
                pig.body.on_ground = e["on_ground"].as_bool().unwrap();
                pig.yaw = number(&e["yaw"]) as f32;
                pig.speed = number(&e["speed"]) as f32;
                pig.age.ticks = e["age"].as_i64().unwrap() as i32;
                ids.insert(
                    tag.to_owned(),
                    world.spawn_pig(pig, e["no_ai"].as_bool().unwrap()),
                );
            }
        } else {
            assert_eq!(tick, current_tick + 1);
            world.tick(&mut scene);
            for (tag, group) in row["data"]["entities"].as_object().unwrap() {
                let e = &group.as_array().unwrap()[0];
                let pig = &world.pigs().iter().find(|c| c.id == ids[tag]).unwrap().pig;
                let entity = world.pigs().iter().find(|c| c.id == ids[tag]).unwrap();
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
                    pig.age.ticks as i64,
                    e["age"].as_i64().unwrap(),
                    "tick {tick} {tag} age"
                );
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
                    ("x", pig.body.position.x),
                    ("y", pig.body.position.y),
                    ("z", pig.body.position.z),
                    ("vx", pig.body.velocity.x),
                    ("vy", pig.body.velocity.y),
                    ("vz", pig.body.velocity.z),
                    ("yaw", pig.yaw as f64),
                    ("speed", pig.speed as f64),
                ] {
                    exact(&e[name], actual, &format!("tick {tick} {tag} {name}"));
                }
                assert_eq!(
                    pig.body.on_ground,
                    e["on_ground"].as_bool().unwrap(),
                    "tick {tick} {tag} on_ground"
                );
                assert_eq!(
                    pig.move_control.has_wanted(),
                    e["move_control_wanted"].as_bool().unwrap(),
                    "tick {tick} {tag} wanted"
                );
                assert_eq!(
                    pig.navigation.is_done(),
                    e["navigation_done"].as_bool().unwrap(),
                    "tick {tick} navigation done"
                );
                assert_eq!(
                    pig.navigation.nodes.len(),
                    e["path_node_count"].as_u64().unwrap() as usize,
                    "tick {tick} node count"
                );
                assert_eq!(
                    pig.navigation.observed_next(),
                    e["path_next_node"].as_i64().unwrap() as i32,
                    "tick {tick} next node"
                );
                assert_eq!(
                    pig.navigation.reached && !pig.navigation.nodes.is_empty(),
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
                    pig.navigation.nodes, expected_nodes,
                    "tick {tick} {tag} path nodes"
                );
                frames += 1;
            }
        }
        current_tick = tick;
    }
    assert!(complete && actions > 0 && frames > 0);
    println!("{actions} retained goals and {frames} pig follow-parent/navigation/control/motion frames matched exactly");
}
