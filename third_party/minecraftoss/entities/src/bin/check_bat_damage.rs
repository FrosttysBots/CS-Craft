//! Exact pinned 26.3 Bat wake/fatal generic-damage gate.
use glam::DVec3;
use minecraftoss_entities::{bat::Bat, world::EntityWorld};
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
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap())
}
fn exact(expected: &Value, actual: f64, label: &str, tick: i64) {
    assert_eq!(
        actual.to_bits(),
        number(expected).to_bits(),
        "tick {tick} {label}"
    );
}
fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_bat_damage TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut ids = BTreeMap::<String, u64>::new();
    let mut hits = 0;
    let mut frames = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "bat_damage_wakes_and_kills"
            ),
            "complete" => complete = true,
            "entity_hurt" => {
                let data = &row["data"];
                let tag = data["tag"].as_str().unwrap();
                assert_eq!(data["source"], "minecraft:generic");
                let amount = number(&data["amount"]) as f32;
                let entity = world.bat_mut(ids[tag]).unwrap();
                exact(
                    &data["health_before"],
                    f64::from(entity.bat.health),
                    &format!("{tag} before"),
                    row["tick"].as_i64().unwrap(),
                );
                let result = entity.hurt(amount);
                assert_eq!(data["applied"], result.applied);
                assert_eq!(data["alive"], entity.bat.health > 0.0);
                exact(
                    &data["health_after"],
                    f64::from(entity.bat.health),
                    &format!("{tag} after"),
                    row["tick"].as_i64().unwrap(),
                );
                hits += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                if tick == 0 {
                    for (key, block) in data["blocks"].as_object().unwrap() {
                        let pos: Vec<i32> =
                            key.split(',').map(|part| part.parse().unwrap()).collect();
                        let id = block["id"].as_str().unwrap();
                        if id != "minecraft:air" {
                            scene.set_block((pos[0], pos[1], pos[2]), Some(Block::new(id)));
                        }
                    }
                    for tag in ["wake", "fatal_rest", "fatal_fly"] {
                        let observed = &data["entities"][tag][0];
                        let mut bat = Bat::new(DVec3::new(
                            number(&observed["x"]),
                            number(&observed["y"]),
                            number(&observed["z"]),
                        ));
                        bat.body.velocity = DVec3::new(
                            number(&observed["vx"]),
                            number(&observed["vy"]),
                            number(&observed["vz"]),
                        );
                        bat.body.on_ground = observed["on_ground"].as_bool().unwrap();
                        bat.health = number(&observed["health"]) as f32;
                        bat.resting = observed["bat_resting"].as_bool().unwrap();
                        bat.persistence_required =
                            observed["persistence_required"].as_bool().unwrap();
                        ids.insert(tag.to_owned(), world.spawn_bat(bat, true));
                    }
                } else {
                    world.tick(&mut scene);
                    assert!(data["item_counts"].as_object().unwrap().is_empty());
                    for tag in ["wake", "fatal_rest", "fatal_fly"] {
                        let observed = &data["entities"][tag][0];
                        let entity = world.bats().iter().find(|e| e.id == ids[tag]).unwrap();
                        let bat = &entity.bat;
                        assert_eq!(observed["bat_resting"], bat.resting);
                        assert_eq!(observed["bat_flapping"], bat.is_flapping(entity.tick_count));
                        assert_eq!(observed["alive"], bat.health > 0.0);
                        assert_eq!(observed["no_ai"], entity.no_ai);
                        assert_eq!(observed["on_ground"], bat.body.on_ground);
                        for (field, actual) in [
                            ("x", bat.body.position.x),
                            ("y", bat.body.position.y),
                            ("z", bat.body.position.z),
                            ("vx", bat.body.velocity.x),
                            ("vy", bat.body.velocity.y),
                            ("vz", bat.body.velocity.z),
                            ("health", f64::from(bat.health)),
                        ] {
                            exact(&observed[field], actual, &format!("{tag} {field}"), tick);
                        }
                        frames += 1;
                    }
                }
            }
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!((hits, frames), (3, 18));
    println!("{hits} generic bat hits and {frames} exact entity frames matched");
}
