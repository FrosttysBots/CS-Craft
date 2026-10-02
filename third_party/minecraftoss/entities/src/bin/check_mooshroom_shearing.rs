//! Exact no-AI mooshroom shearing conversion against pinned Minecraft 26.3.
use glam::DVec3;
use minecraftoss_entities::{
    age::Age,
    mooshroom::{MushroomCow, MushroomVariant},
    world::EntityWorld,
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
        .expect("usage: check_mooshroom_shearing TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut ids = BTreeMap::<String, u64>::new();
    let mut drops = BTreeMap::<String, u64>::new();
    let mut uses = 0;
    let mut frames = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "mooshroom_shearing_conversion"
            ),
            "complete" => complete = true,
            "entity_use" => {
                let data = &row["data"];
                assert_eq!(data["item"], "minecraft:shears");
                let tag = data["tag"].as_str().unwrap();
                let outcome = world.shear_mooshroom(ids[tag]);
                assert_eq!(
                    data["result"],
                    if outcome.is_some() {
                        "Success[swingSource=PREDICTED, itemContext=ItemContext[wasItemInteraction=true, heldItemTransformedTo=null]]"
                    } else {
                        "Pass[]"
                    }
                );
                assert_eq!(data["hand_item"], "minecraft:shears");
                assert_eq!(data["hand_count"], 1);
                assert_eq!(data["hand_stew_effects"], serde_json::json!([]));
                assert_eq!(data["hand_max_damage"], 238);
                assert_eq!(data["hand_damage"], outcome.map_or(0, |s| s.tool_damage));
                if let Some(shearing) = outcome {
                    *drops.entry(shearing.drop_item.to_owned()).or_default() +=
                        u64::from(shearing.drop_count);
                }
                uses += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                if tick == 0 {
                    for (key, block) in data["blocks"].as_object().unwrap() {
                        let coords: Vec<i32> =
                            key.split(',').map(|part| part.parse().unwrap()).collect();
                        let id = block["id"].as_str().unwrap();
                        if id != "minecraft:air" {
                            scene
                                .set_block((coords[0], coords[1], coords[2]), Some(Block::new(id)));
                        }
                    }
                    for (tag, variant) in [
                        ("red", MushroomVariant::Red),
                        ("brown", MushroomVariant::Brown),
                        ("baby", MushroomVariant::Brown),
                    ] {
                        let observed = &data["entities"][tag][0];
                        let mut mob = MushroomCow::new(
                            DVec3::new(
                                number(&observed["x"]),
                                number(&observed["y"]),
                                number(&observed["z"]),
                            ),
                            variant,
                        );
                        mob.cow.body.velocity = DVec3::new(
                            number(&observed["vx"]),
                            number(&observed["vy"]),
                            number(&observed["vz"]),
                        );
                        mob.cow.body.on_ground = observed["on_ground"].as_bool().unwrap();
                        mob.cow.age = Age {
                            ticks: observed["age"].as_i64().unwrap() as i32,
                            forced: observed["forced_age"].as_i64().unwrap() as i32,
                            locked: observed["age_locked"].as_bool().unwrap(),
                            forced_particle_ticks: observed["forced_age_timer"].as_i64().unwrap()
                                as i32,
                            lock_particle_ticks: 0,
                        };
                        mob.cow.health = number(&observed["health"]) as f32;
                        mob.cow.persistence_required =
                            observed["persistence_required"].as_bool().unwrap();
                        ids.insert(tag.to_owned(), world.spawn_mooshroom(mob, true));
                    }
                } else {
                    world.tick(&mut scene);
                    let cow_count = world
                        .cows()
                        .iter()
                        .filter(|e| e.mooshroom.is_none())
                        .count() as u64;
                    let mooshroom_count = world.cows().len() as u64 - cow_count;
                    assert_eq!(
                        data["entity_type_counts"]["minecraft:cow"]["total"],
                        cow_count
                    );
                    assert_eq!(
                        data["entity_type_counts"]["minecraft:mooshroom"]["total"],
                        mooshroom_count
                    );
                    assert_eq!(data["entity_type_counts"]["minecraft:cow"]["babies"], 0);
                    assert_eq!(
                        data["entity_type_counts"]["minecraft:mooshroom"]["babies"],
                        1
                    );
                    let observed_drops: BTreeMap<String, u64> = data["item_counts"]
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(id, count)| (id.clone(), count.as_u64().unwrap()))
                        .collect();
                    assert_eq!(observed_drops, drops, "tick {tick} drops");
                    for tag in ["red", "brown", "baby"] {
                        let observed = &data["entities"][tag][0];
                        let entity = world.cows().iter().find(|e| e.id == ids[tag]).unwrap();
                        let cow = &entity.cow;
                        assert_eq!(
                            observed["type"],
                            if entity.mooshroom.is_some() {
                                "minecraft:mooshroom"
                            } else {
                                "minecraft:cow"
                            }
                        );
                        if let Some(state) = &entity.mooshroom {
                            assert_eq!(
                                observed["mooshroom_variant"],
                                if state.variant == MushroomVariant::Red {
                                    "red"
                                } else {
                                    "brown"
                                }
                            );
                        } else {
                            assert!(observed.get("mooshroom_variant").is_none());
                        }
                        assert_eq!(observed["age"], cow.age.ticks);
                        assert_eq!(observed["no_ai"], entity.no_ai);
                        assert_eq!(observed["persistence_required"], cow.persistence_required);
                        assert_eq!(observed["on_ground"], cow.body.on_ground);
                        assert_eq!(observed["alive"], cow.health > 0.0);
                        exact(
                            &observed["health"],
                            f64::from(cow.health),
                            &format!("{tag} health"),
                            tick,
                        );
                        for (name, actual) in [
                            ("x", cow.body.position.x),
                            ("y", cow.body.position.y),
                            ("z", cow.body.position.z),
                            ("vx", cow.body.velocity.x),
                            ("vy", cow.body.velocity.y),
                            ("vz", cow.body.velocity.z),
                        ] {
                            exact(&observed[name], actual, &format!("{tag} {name}"), tick);
                        }
                        frames += 1;
                    }
                }
            }
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!((uses, frames), (3, 15));
    println!("{uses} shears uses, {frames} exact entity frames, and variant drops matched");
}
