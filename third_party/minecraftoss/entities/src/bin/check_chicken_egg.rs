//! Exact pinned 26.3 NoAI chicken flap, age and egg-lay lifecycle trace.
use glam::DVec3;
use minecraftoss_entities::{chicken::Chicken, world::EntityWorld};
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
        .expect("usage: check_chicken_egg TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut ids = BTreeMap::<String, u64>::new();
    let mut ticks = 0;
    let mut frames = 0;
    let mut seeds = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "chicken_egg_lifecycle_no_ai"
            ),
            "complete" => complete = true,
            "entity_set_random_seed" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                let seed = row["data"]["seed"]
                    .as_str()
                    .unwrap()
                    .parse::<i64>()
                    .unwrap();
                world.chicken_mut(ids[tag]).unwrap().set_random_seed(seed);
                seeds += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                if tick == 0 {
                    for (key, value) in row["data"]["blocks"].as_object().unwrap() {
                        let xyz: Vec<i32> =
                            key.split(',').map(|part| part.parse().unwrap()).collect();
                        let id = value["id"].as_str().unwrap();
                        if id != "minecraft:air" {
                            scene.set_block((xyz[0], xyz[1], xyz[2]), Some(Block::new(id)));
                        }
                    }
                    for (tag, group) in row["data"]["entities"].as_object().unwrap() {
                        let observed = &group[0];
                        let mut chicken = Chicken::new(DVec3::new(
                            number(&observed["x"]),
                            number(&observed["y"]),
                            number(&observed["z"]),
                        ));
                        chicken.body.on_ground = observed["on_ground"].as_bool().unwrap();
                        chicken.age.ticks = observed["age"].as_i64().unwrap() as i32;
                        chicken.egg_time = observed["egg_time"].as_i64().unwrap() as i32;
                        chicken.is_chicken_jockey = observed["chicken_jockey"].as_bool().unwrap();
                        chicken.persistence_required =
                            observed["persistence_required"].as_bool().unwrap();
                        ids.insert(tag.to_owned(), world.spawn_chicken(chicken, true));
                    }
                } else {
                    assert_eq!(tick, ticks + 1);
                    world.tick(&mut scene);
                    for (tag, group) in row["data"]["entities"].as_object().unwrap() {
                        let observed = &group[0];
                        let entity = world
                            .chickens()
                            .iter()
                            .find(|entity| entity.id == ids[tag])
                            .unwrap();
                        let chicken = &entity.chicken;
                        assert_eq!(
                            entity.no_ai,
                            observed["no_ai"].as_bool().unwrap(),
                            "tick {tick} {tag} NoAI"
                        );
                        assert_eq!(entity.tick_count as i64, tick, "tick {tick} {tag} count");
                        assert_eq!(
                            chicken.age.ticks as i64,
                            observed["age"].as_i64().unwrap(),
                            "tick {tick} {tag} age"
                        );
                        assert_eq!(
                            chicken.egg_time as i64,
                            observed["egg_time"].as_i64().unwrap(),
                            "tick {tick} {tag} egg timer"
                        );
                        assert_eq!(
                            chicken.is_chicken_jockey,
                            observed["chicken_jockey"].as_bool().unwrap(),
                            "tick {tick} {tag} jockey"
                        );
                        exact(
                            &observed["flap"],
                            f64::from(chicken.flap),
                            &format!("{tag} flap"),
                            tick,
                        );
                        exact(
                            &observed["flap_speed"],
                            f64::from(chicken.flap_speed),
                            &format!("{tag} flap speed"),
                            tick,
                        );
                        exact(
                            &observed["flapping"],
                            f64::from(chicken.flapping),
                            &format!("{tag} flapping"),
                            tick,
                        );
                        for (name, actual) in [
                            ("x", chicken.body.position.x),
                            ("y", chicken.body.position.y),
                            ("z", chicken.body.position.z),
                            ("vx", chicken.body.velocity.x),
                            ("vy", chicken.body.velocity.y),
                            ("vz", chicken.body.velocity.z),
                        ] {
                            exact(&observed[name], actual, &format!("{tag} {name}"), tick);
                        }
                        frames += 1;
                    }
                    let eggs = world
                        .chickens()
                        .iter()
                        .map(|entity| entity.eggs_laid)
                        .sum::<usize>();
                    assert_eq!(
                        eggs,
                        row["data"]["item_counts"]
                            .get("minecraft:egg")
                            .and_then(Value::as_u64)
                            .unwrap_or(0) as usize,
                        "tick {tick} eggs"
                    );
                    ticks += 1;
                }
            }
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!((ticks, seeds, frames), (20, 3, 60));
    println!("{seeds} seeds and {frames} chicken lifecycle frames matched exactly");
}
