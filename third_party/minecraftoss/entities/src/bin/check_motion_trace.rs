//! Focused dry-motion gate, initialized from a repeated reference's tick zero.
//! Only cow bodies with zero movement input are accepted in this initial gate.
use glam::DVec3;
use minecraftoss_entities::movement::Body;
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

#[derive(Default)]
struct Scene(BTreeMap<Pos, Block>);
impl World for Scene {
    fn block(&self, p: Pos) -> Option<Block> {
        self.0.get(&p).cloned()
    }
    fn set_block(&mut self, p: Pos, block: Option<Block>) {
        if let Some(b) = block {
            self.0.insert(p, b);
        } else {
            self.0.remove(&p);
        }
    }
}
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap())
}
fn main() {
    let trace =
        fs::read_to_string(env::args().nth(1).expect("usage: check_motion_trace TRACE")).unwrap();
    let mut bodies: BTreeMap<String, Body> = BTreeMap::new();
    let mut scene = Scene::default();
    let mut checked = 0;
    let mut complete = false;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        if row["type"] == "complete" {
            complete = true;
        }
        if row["type"] != "snapshot" {
            continue;
        }
        let tick = row["tick"].as_i64().unwrap();
        if tick == 0 {
            bodies.clear();
            scene.0.clear();
            for (key, value) in row["data"]["blocks"]
                .as_object()
                .expect("observed blocks")
                .iter()
            {
                let p: Vec<i32> = key.split(',').map(|v| v.parse().unwrap()).collect();
                let mut block = Block::new(value["id"].as_str().unwrap());
                for (k, v) in value["properties"].as_object().unwrap() {
                    block = block.with(k, v.as_str().unwrap());
                }
                scene.set_block((p[0], p[1], p[2]), Some(block));
            }
        }
        for group in row["data"]["entities"].as_object().unwrap().values() {
            for e in group.as_array().unwrap() {
                assert_eq!(e["type"], "minecraft:cow", "unsupported body dimensions");
                let id = e["uuid"].as_str().unwrap();
                if tick == 0 {
                    let mut body = Body::new(
                        DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"])),
                        0.9,
                        1.4,
                    );
                    body.velocity =
                        DVec3::new(number(&e["vx"]), number(&e["vy"]), number(&e["vz"]));
                    body.on_ground = e["on_ground"].as_bool().unwrap();
                    bodies.insert(id.to_owned(), body);
                } else {
                    let body = bodies.get_mut(id).unwrap();
                    body.trim_small_velocity();
                    body.travel_air(&scene, DVec3::ZERO, 0.0, 0.0);
                    for (key, actual) in [
                        ("x", body.position.x),
                        ("y", body.position.y),
                        ("z", body.position.z),
                        ("vx", body.velocity.x),
                        ("vy", body.velocity.y),
                        ("vz", body.velocity.z),
                    ] {
                        assert_eq!(
                            actual.to_bits(),
                            number(&e[key]).to_bits(),
                            "tick {tick} {id} {key}: Rust {actual:?}, Java {:?}",
                            number(&e[key])
                        );
                    }
                    assert_eq!(
                        body.on_ground,
                        e["on_ground"].as_bool().unwrap(),
                        "tick {tick} {id} on_ground"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(complete && checked > 0);
    println!("{checked} exact motion transitions matched (position/velocity bits and on_ground); AI, health and events outside this gate");
}
