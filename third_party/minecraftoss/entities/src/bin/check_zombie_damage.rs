//! Exact pinned 26.3 Zombie generic damage, cooldown and death cleanup gate.
use glam::DVec3;
use minecraftoss_entities::{world::EntityWorld, zombie::Zombie};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

struct EmptyWorld;
impl World for EmptyWorld {
    fn block(&self, _: Pos) -> Option<Block> {
        None
    }
    fn set_block(&mut self, _: Pos, _: Option<Block>) {}
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
        .expect("usage: check_zombie_damage TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = EmptyWorld;
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
                "zombie_generic_damage_and_death_cleanup"
            ),
            "complete" => complete = true,
            "entity_set_random_seed" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                world
                    .zombie_mut(ids[tag])
                    .unwrap()
                    .set_random_seed(row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "entity_hurt" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let tag = data["tag"].as_str().unwrap();
                let entity = world.zombie_mut(ids[tag]).unwrap();
                exact(
                    &data["health_before"],
                    f64::from(entity.zombie.health),
                    "health before",
                    tick,
                );
                let result = entity.hurt(number(&data["amount"]) as f32);
                assert_eq!(data["applied"], result.applied, "tick {tick} applied");
                assert_eq!(
                    data["alive"],
                    entity.zombie.health > 0.0,
                    "tick {tick} alive"
                );
                exact(
                    &data["health_after"],
                    f64::from(entity.zombie.health),
                    "health after",
                    tick,
                );
                hits += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                if tick == 0 {
                    for tag in ["adult", "baby"] {
                        let observed = &data["entities"][tag][0];
                        let mut zombie = Zombie::new(DVec3::new(
                            number(&observed["x"]),
                            number(&observed["y"]),
                            number(&observed["z"]),
                        ));
                        zombie.set_baby(observed["zombie_baby"].as_bool().unwrap());
                        zombie.persistence_required =
                            observed["persistence_required"].as_bool().unwrap();
                        ids.insert(tag.to_owned(), world.spawn_zombie(zombie, true));
                    }
                } else {
                    world.tick(&mut scene);
                    assert_eq!(
                        data["item_counts"],
                        serde_json::json!({}),
                        "tick {tick} drops"
                    );
                    assert_eq!(
                        data["entity_type_counts"]["minecraft:zombie"]["total"],
                        world.zombies().len(),
                        "tick {tick} population"
                    );
                    for tag in ["adult", "baby"] {
                        let observed_list = data["entities"][tag].as_array().unwrap();
                        let entity = world.zombies().iter().find(|entity| entity.id == ids[tag]);
                        assert_eq!(
                            observed_list.len(),
                            usize::from(entity.is_some()),
                            "tick {tick} {tag} presence"
                        );
                        let Some(entity) = entity else { continue };
                        let observed = &observed_list[0];
                        assert_eq!(observed["zombie_baby"], entity.zombie.baby);
                        assert_eq!(observed["entity_tick_count"], entity.tick_count);
                        assert_eq!(observed["ambient_sound_time"], entity.ambient_sound_time);
                        assert_eq!(observed["no_action_time"], entity.no_action_time);
                        for (field, actual) in [
                            ("x", entity.zombie.body.position.x),
                            ("y", entity.zombie.body.position.y),
                            ("z", entity.zombie.body.position.z),
                            ("vx", entity.zombie.body.velocity.x),
                            ("vy", entity.zombie.body.velocity.y),
                            ("vz", entity.zombie.body.velocity.z),
                            ("health", f64::from(entity.zombie.health)),
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
    assert_eq!(hits, 4);
    assert_eq!(frames, 43);
    println!("{hits} zombie damage results and {frames} exact entity frames matched");
}
