//! Direct MoveControl plus dry travel against a repeated pinned 26.3 trace.
//! Goals were explicitly removed in the source; navigation is outside this gate.
use glam::DVec3;
use minecraftoss_entities::{cow::Cow, world::EntityWorld};
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
        .expect("usage: check_cow_control TRACE.jsonl");
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
        if row["type"] == "entity_move_to" {
            let data = &row["data"];
            let tag = data["tag"].as_str().unwrap();
            let cow = &mut world.cow_mut(ids[tag]).unwrap().cow;
            let p = &data["target"];
            cow.move_control.set_wanted_position(
                DVec3::new(
                    p[0].as_f64().unwrap(),
                    p[1].as_f64().unwrap(),
                    p[2].as_f64().unwrap(),
                ),
                number(&data["speed"]),
            );
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
                let mut cow = Cow::new(DVec3::new(
                    number(&e["x"]),
                    number(&e["y"]),
                    number(&e["z"]),
                ));
                cow.body.velocity =
                    DVec3::new(number(&e["vx"]), number(&e["vy"]), number(&e["vz"]));
                cow.body.on_ground = e["on_ground"].as_bool().unwrap();
                cow.yaw = number(&e["yaw"]) as f32;
                cow.speed = number(&e["speed"]) as f32;
                ids.insert(
                    tag.to_owned(),
                    world.spawn_cow(cow, e["no_ai"].as_bool().unwrap()),
                );
            }
        } else {
            assert_eq!(tick, current_tick + 1);
            world.tick(&mut scene);
            for (tag, group) in row["data"]["entities"].as_object().unwrap() {
                let e = &group.as_array().unwrap()[0];
                let cow = &world.cows().iter().find(|c| c.id == ids[tag]).unwrap().cow;
                for (name, actual) in [
                    ("x", cow.body.position.x),
                    ("y", cow.body.position.y),
                    ("z", cow.body.position.z),
                    ("vx", cow.body.velocity.x),
                    ("vy", cow.body.velocity.y),
                    ("vz", cow.body.velocity.z),
                    ("yaw", cow.yaw as f64),
                    ("speed", cow.speed as f64),
                ] {
                    exact(&e[name], actual, &format!("tick {tick} {tag} {name}"));
                }
                assert_eq!(
                    cow.body.on_ground,
                    e["on_ground"].as_bool().unwrap(),
                    "tick {tick} {tag} on_ground"
                );
                assert_eq!(
                    cow.move_control.has_wanted(),
                    e["move_control_wanted"].as_bool().unwrap(),
                    "tick {tick} {tag} wanted"
                );
                frames += 1;
            }
        }
        current_tick = tick;
    }
    assert!(complete && actions > 0 && frames > 0);
    println!("{actions} direct move commands and {frames} cow control/motion frames matched exactly; goals and paths excluded");
}
