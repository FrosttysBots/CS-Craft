//! Exact selected state for pinned 26.3 zombie-to-drowned replacement.
use glam::DVec3;
use minecraftoss_entities::{
    world::EntityWorld,
    zombie::Zombie,
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

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_zombie_conversion TRACE.jsonl");
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
    let mut original_id = None;
    let mut frames = 0;
    let mut interventions = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "zombie_to_drowned_terminal_conversion"
            ),
            "complete" => complete = true,
            "zombie_set_in_water_time" => {
                world
                    .zombie_mut(original_id.unwrap())
                    .unwrap()
                    .zombie
                    .in_water_time = row["data"]["time"].as_i64().unwrap() as i32;
                interventions += 1;
            }
            "zombie_set_conversion_time" => {
                world
                    .zombie_mut(original_id.unwrap())
                    .unwrap()
                    .zombie
                    .conversion_time = row["data"]["time"].as_i64().unwrap() as i32;
                interventions += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let observed = &row["data"]["entities"]["convert"][0];
                if tick == 0 {
                    let mut zombie = Zombie::new(DVec3::new(
                        number(&observed["x"]),
                        number(&observed["y"]),
                        number(&observed["z"]),
                    ));
                    zombie.health = number(&observed["health"]) as f32;
                    zombie.set_baby(observed["zombie_baby"].as_bool().unwrap());
                    zombie.persistence_required =
                        observed["persistence_required"].as_bool().unwrap();
                    original_id = Some(world.spawn_zombie_drowning(zombie));
                } else {
                    world.tick(&mut scene);
                }
                assert_eq!(world.zombies().len(), 1, "tick {tick} undead count");
                let entity = &world.zombies()[0];
                let expected_type = entity.zombie.kind.type_id();
                assert_eq!(observed["type"], expected_type, "tick {tick} type");
                assert_eq!(
                    observed["zombie_baby"], entity.zombie.baby,
                    "tick {tick} baby"
                );
                assert_eq!(
                    observed["persistence_required"], entity.zombie.persistence_required,
                    "tick {tick} persistence"
                );
                assert_eq!(
                    observed["zombie_underwater_converting"], entity.zombie.underwater_converting,
                    "tick {tick} conversion flag"
                );
                assert_eq!(
                    number(&observed["health"]).to_bits(),
                    f64::from(entity.zombie.health).to_bits(),
                    "tick {tick} health"
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
                        "tick {tick} {field}"
                    );
                }
                assert_eq!(
                    observed["on_ground"], entity.zombie.body.on_ground,
                    "tick {tick} ground"
                );
                for (kind, actual) in [
                    ("minecraft:zombie", expected_type == "minecraft:zombie"),
                    ("minecraft:drowned", expected_type == "minecraft:drowned"),
                ] {
                    assert_eq!(
                        row["data"]["entity_type_counts"][kind]["total"],
                        i32::from(actual),
                        "tick {tick} {kind} count"
                    );
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!(interventions, 2);
    assert_eq!(frames, 4);
    println!("{frames} exact zombie-to-drowned conversion and motion frames matched");
}
