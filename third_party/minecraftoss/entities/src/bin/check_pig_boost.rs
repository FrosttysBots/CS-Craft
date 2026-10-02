//! Exact seeded Pig.boost / ItemBasedSteering state without a passenger.
use glam::DVec3;
use minecraftoss_entities::{pig::Pig, world::EntityWorld};
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
fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_pig_boost TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut ids = BTreeMap::<String, u64>::new();
    let mut frames = 0;
    let mut seeds = 0;
    let mut boosts = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "pig_boost_unridden_seeded"
            ),
            "complete" => complete = true,
            "entity_set_random_seed" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                let seed = row["data"]["seed"]
                    .as_str()
                    .unwrap()
                    .parse::<i64>()
                    .unwrap();
                world.pig_mut(ids[tag]).unwrap().set_random_seed(seed);
                seeds += 1;
            }
            "entity_boost" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                let entity = world.pig_mut(ids[tag]).unwrap();
                let applied = entity.steering.boost(&mut entity.random);
                assert_eq!(applied, row["data"]["applied"].as_bool().unwrap());
                assert_eq!(
                    entity.steering.boosting,
                    row["data"]["boosting"].as_bool().unwrap()
                );
                assert_eq!(
                    entity.steering.boost_time,
                    row["data"]["boost_time"].as_i64().unwrap() as i32
                );
                assert_eq!(
                    entity.steering.boost_time_total,
                    row["data"]["boost_time_total"].as_i64().unwrap() as i32
                );
                boosts += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                if tick == 0 {
                    for (key, value) in row["data"]["blocks"].as_object().unwrap() {
                        let xyz: Vec<i32> =
                            key.split(',').map(|part| part.parse().unwrap()).collect();
                        let block_id = value["id"].as_str().unwrap();
                        if block_id != "minecraft:air" {
                            scene.set_block((xyz[0], xyz[1], xyz[2]), Some(Block::new(block_id)));
                        }
                    }
                    for (tag, group) in row["data"]["entities"].as_object().unwrap() {
                        let observed = &group[0];
                        let mut pig = Pig::new(DVec3::new(
                            number(&observed["x"]),
                            number(&observed["y"]),
                            number(&observed["z"]),
                        ));
                        pig.body.on_ground = observed["on_ground"].as_bool().unwrap();
                        ids.insert(tag.to_owned(), world.spawn_pig_no_ai(pig));
                    }
                } else {
                    assert_eq!(tick, frames + 1);
                    world.tick(&mut scene);
                    for (tag, group) in row["data"]["entities"].as_object().unwrap() {
                        let observed = &group[0];
                        let entity = world
                            .pigs()
                            .iter()
                            .find(|entity| entity.id == ids[tag])
                            .unwrap();
                        assert_eq!(
                            entity.steering.boosting,
                            observed["boosting"].as_bool().unwrap(),
                            "tick {tick} {tag} boosting"
                        );
                        assert_eq!(
                            entity.steering.boost_time,
                            observed["boost_time"].as_i64().unwrap() as i32,
                            "tick {tick} {tag} boost time"
                        );
                        assert_eq!(
                            entity.steering.boost_time_total,
                            observed["boost_time_total"].as_i64().unwrap() as i32,
                            "tick {tick} {tag} total"
                        );
                        assert_eq!(
                            entity.pig.age.ticks as i64,
                            observed["age"].as_i64().unwrap(),
                            "tick {tick} {tag} age"
                        );
                        assert_eq!(
                            entity.pig.body.position.x.to_bits(),
                            number(&observed["x"]).to_bits(),
                            "tick {tick} {tag} x"
                        );
                        assert_eq!(
                            entity.pig.body.position.y.to_bits(),
                            number(&observed["y"]).to_bits(),
                            "tick {tick} {tag} y"
                        );
                        assert_eq!(
                            entity.pig.body.position.z.to_bits(),
                            number(&observed["z"]).to_bits(),
                            "tick {tick} {tag} z"
                        );
                    }
                    frames += 1;
                }
            }
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!((seeds, boosts, frames), (2, 5, 12));
    println!(
        "{seeds} seeded streams, {boosts} boost calls and {} pig-state frames matched exactly",
        frames * 2
    );
}
