//! Exact pinned 26.3 zombie drowning timer start/cancel gate.
use glam::DVec3;
use minecraftoss_entities::{world::EntityWorld, zombie::Zombie};
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
        .expect("usage: check_zombie_drowning TRACE.jsonl");
    let mut scene = Scene::default();
    for x in 0..16 {
        for z in 0..16 {
            scene.set_block((x, 0, z), Some(Block::new("minecraft:stone")));
            scene.set_block((x, 6, z), Some(Block::new("minecraft:stone")));
            for y in 1..=5 {
                scene.set_block((x, y, z), Some(Block::new("minecraft:water")));
            }
        }
    }
    let mut world = EntityWorld::default();
    let mut ids = BTreeMap::new();
    let mut frames = 0;
    let mut actions = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "zombie_drowning_start_cancel_and_no_ai"
            ),
            "complete" => complete = true,
            "zombie_set_in_water_time" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                world.zombie_mut(ids[tag]).unwrap().zombie.in_water_time =
                    row["data"]["time"].as_i64().unwrap() as i32;
                actions += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                if tick == 0 {
                    for tag in ["active", "no_ai"] {
                        let observed = &row["data"]["entities"][tag][0];
                        let zombie = Zombie::new(DVec3::new(
                            number(&observed["x"]),
                            number(&observed["y"]),
                            number(&observed["z"]),
                        ));
                        let id = if tag == "active" {
                            world.spawn_zombie_drowning(zombie)
                        } else {
                            world.spawn_zombie(zombie, true)
                        };
                        ids.insert(tag, id);
                    }
                } else {
                    if tick == 4 || tick == 6 {
                        let body = &mut world.zombie_mut(ids["active"]).unwrap().zombie.body;
                        body.position.y = if tick == 4 { 7.0 } else { 1.0 };
                        body.velocity = DVec3::ZERO;
                        body.on_ground = false;
                    }
                    world.tick(&mut scene);
                    for tag in ["active", "no_ai"] {
                        let observed = &row["data"]["entities"][tag][0];
                        let entity = world.zombies().iter().find(|z| z.id == ids[tag]).unwrap();
                        assert_eq!(
                            observed["zombie_underwater_converting"],
                            entity.zombie.underwater_converting,
                            "tick {tick} {tag} conversion"
                        );
                        for (field, actual) in [
                            ("x", entity.zombie.body.position.x),
                            ("y", entity.zombie.body.position.y),
                            ("z", entity.zombie.body.position.z),
                            ("vx", entity.zombie.body.velocity.x),
                            ("vy", entity.zombie.body.velocity.y),
                            ("vz", entity.zombie.body.velocity.z),
                        ] {
                            assert_eq!(
                                number(&observed[field]).to_bits(),
                                actual.to_bits(),
                                "tick {tick} {tag} {field}"
                            );
                        }
                        assert_eq!(
                            observed["on_ground"], entity.zombie.body.on_ground,
                            "tick {tick} {tag} ground"
                        );
                        frames += 1;
                    }
                }
            }
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!(actions, 3);
    assert_eq!(frames, 20);
    println!("{frames} exact zombie conversion and motion frames matched");
}
