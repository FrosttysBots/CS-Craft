//! Exact selected sheep shearing state against the pinned 26.3 server trace.
use glam::DVec3;
use minecraftoss_entities::{
    loot::ShearingLootBook,
    sheep::{ShearsResult, Sheep, Wool, DYE_NAMES},
    world::EntityWorld,
};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs, path::Path};

struct EmptyWorld;
impl World for EmptyWorld {
    fn block(&self, _: Pos) -> Option<Block> {
        None
    }
    fn set_block(&mut self, _: Pos, _: Option<Block>) {}
}

fn exact(expected: &Value, actual: f64, label: &str, tick: i64) {
    let bits = u64::from_str_radix(expected["bits"].as_str().unwrap(), 16).unwrap();
    assert_eq!(actual.to_bits(), bits, "tick {tick} {label}");
}

fn main() {
    let mut args = env::args().skip(1);
    let trace = args
        .next()
        .expect("usage: check_sheep_shearing TRACE COMMON_JAR");
    let jar = args
        .next()
        .expect("usage: check_sheep_shearing TRACE COMMON_JAR");
    let mut loot = ShearingLootBook::from_jar(Path::new(&jar), 0).unwrap();
    let mut world = EntityWorld::default();
    let mut ids = BTreeMap::<String, u64>::new();
    let mut item_counts = BTreeMap::<String, i64>::new();
    let mut frames = 0;
    let mut uses = 0;
    let mut complete = false;
    for line in fs::read_to_string(trace).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "sheep_shearing_colors_and_eligibility"
            ),
            "complete" => complete = true,
            "entity_use" => {
                assert_eq!(row["data"]["item"], "minecraft:shears");
                let tag = row["data"]["tag"].as_str().unwrap();
                let state = world.sheep_mut(ids[tag]).unwrap();
                let result = state.sheep.use_shears(&mut loot).unwrap();
                let (damage, expected_result) = match result {
                    ShearsResult::Sheared { drops, tool_damage } => {
                        for drop in drops {
                            *item_counts.entry(drop.id).or_default() += i64::from(drop.count);
                        }
                        (tool_damage as i64, "Success[swingSource=SERVER_ONLY, itemContext=ItemContext[wasItemInteraction=true, heldItemTransformedTo=null]]")
                    }
                    ShearsResult::Consumed => (0, "Success[swingSource=NONE, itemContext=ItemContext[wasItemInteraction=true, heldItemTransformedTo=null]]"),
                };
                assert_eq!(row["data"]["result"], expected_result);
                assert_eq!(row["data"]["hand_item"], "minecraft:shears");
                assert_eq!(row["data"]["hand_count"], 1);
                assert_eq!(row["data"]["hand_damage"], damage);
                assert_eq!(row["data"]["hand_max_damage"], 238);
                uses += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let groups = row["data"]["entities"].as_object().unwrap();
                assert_eq!(groups.len(), 4);
                if tick == 0 {
                    for (tag, entities) in groups {
                        let observed = &entities[0];
                        assert_eq!(observed["type"], "minecraft:sheep");
                        let color = observed["sheep_color"].as_str().unwrap();
                        let id = DYE_NAMES.iter().position(|name| *name == color).unwrap() as u8;
                        let mut wool = Wool::default();
                        wool.set_color(id);
                        wool.set_sheared(observed["sheared"].as_bool().unwrap());
                        let id = world.spawn_sheep_no_ai(
                            Sheep {
                                age: minecraftoss_entities::age::Age {
                                    ticks: observed["age"].as_i64().unwrap() as i32,
                                    ..Default::default()
                                },
                                wool,
                                ..Default::default()
                            },
                            DVec3::new(
                                f64::from_bits(
                                    u64::from_str_radix(
                                        observed["x"]["bits"].as_str().unwrap(),
                                        16,
                                    )
                                    .unwrap(),
                                ),
                                f64::from_bits(
                                    u64::from_str_radix(
                                        observed["y"]["bits"].as_str().unwrap(),
                                        16,
                                    )
                                    .unwrap(),
                                ),
                                f64::from_bits(
                                    u64::from_str_radix(
                                        observed["z"]["bits"].as_str().unwrap(),
                                        16,
                                    )
                                    .unwrap(),
                                ),
                            ),
                        );
                        ids.insert(tag.clone(), id);
                    }
                    assert!(row["data"]["item_counts"].as_object().unwrap().is_empty());
                } else {
                    assert_eq!(tick, frames + 1);
                    world.tick(&mut EmptyWorld);
                    for (tag, entities) in groups {
                        let state = world.sheep_mut(ids[tag]).unwrap();
                        let observed = &entities[0];
                        assert_eq!(
                            observed["sheep_color"],
                            state.sheep.wool.color(),
                            "tick {tick} {tag} color"
                        );
                        assert_eq!(
                            observed["sheared"],
                            state.sheep.wool.sheared(),
                            "tick {tick} {tag} sheared"
                        );
                        assert_eq!(
                            observed["age"], state.sheep.age.ticks,
                            "tick {tick} {tag} age"
                        );
                        exact(&observed["health"], f64::from(state.health), "health", tick);
                        assert_eq!(observed["no_ai"], state.no_ai);
                        for (name, actual) in [
                            ("x", state.body.position.x),
                            ("y", state.body.position.y),
                            ("z", state.body.position.z),
                            ("vx", state.body.velocity.x),
                            ("vy", state.body.velocity.y),
                            ("vz", state.body.velocity.z),
                        ] {
                            exact(&observed[name], actual, name, tick);
                        }
                        assert_eq!(observed["on_ground"], state.body.on_ground);
                    }
                    let observed_items: BTreeMap<String, i64> = row["data"]["item_counts"]
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(key, value)| (key.clone(), value.as_i64().unwrap()))
                        .collect();
                    assert_eq!(observed_items, item_counts, "tick {tick} item counts");
                    frames += 1;
                }
            }
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!((world.sheep().len(), uses, frames), (4, 5, 8));
    println!("4 shared-world sheep, 5 shears uses, 8 selected exact state/drop frames matched");
}
