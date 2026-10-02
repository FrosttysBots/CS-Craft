//! Pinned 26.3 skeleton generic damage and death cleanup gate.
use glam::DVec3;
use minecraftoss_entities::{skeleton::Skeleton, world::EntityWorld};
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
fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}
fn exact(value: &Value, actual: f64, tick: i64, field: &str) {
    assert_eq!(
        number(value).to_bits(),
        actual.to_bits(),
        "tick {tick} {field}"
    );
}
fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_skeleton_damage TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = EmptyWorld;
    let mut ids = BTreeMap::<String, u64>::new();
    let mut hits = 0;
    let mut frames = 0;
    let mut expected_frames = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => {
                let case = &row["data"]["suite"]["scenarios"][0];
                assert_eq!(case["id"], "skeleton_generic_damage_and_death_cleanup");
                expected_frames = case["ticks"].as_u64().unwrap() as usize;
            }
            "entity_set_random_seed" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                world
                    .skeleton_mut(ids[tag])
                    .unwrap()
                    .set_random_seed(row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "entity_hurt" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let entity = world
                    .skeleton_mut(ids[data["tag"].as_str().unwrap()])
                    .unwrap();
                exact(
                    &data["health_before"],
                    f64::from(entity.skeleton.health),
                    tick,
                    "health before",
                );
                let result = entity.hurt(number(&data["amount"]) as f32);
                assert_eq!(data["applied"], result.applied, "tick {tick} applied");
                assert_eq!(
                    data["alive"],
                    entity.skeleton.health > 0.0,
                    "tick {tick} alive"
                );
                exact(
                    &data["health_after"],
                    f64::from(entity.skeleton.health),
                    tick,
                    "health after",
                );
                hits += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                if tick == 0 {
                    for tag in ["first", "second"] {
                        let observed = &data["entities"][tag][0];
                        let mut skeleton = Skeleton::new(DVec3::new(
                            number(&observed["x"]),
                            number(&observed["y"]),
                            number(&observed["z"]),
                        ));
                        skeleton.persistence_required =
                            observed["persistence_required"].as_bool().unwrap();
                        ids.insert(tag.to_owned(), world.spawn_skeleton(skeleton, true));
                    }
                } else {
                    world.tick(&mut scene);
                    assert_eq!(
                        data["item_counts"],
                        serde_json::json!({}),
                        "tick {tick} drops"
                    );
                    assert_eq!(
                        data["entity_type_counts"]["minecraft:skeleton"]["total"],
                        world.skeletons().len(),
                        "tick {tick} population"
                    );
                    for tag in ["first", "second"] {
                        let observed_list = data["entities"][tag].as_array().unwrap();
                        let entity = world
                            .skeletons()
                            .iter()
                            .find(|entity| entity.id == ids[tag]);
                        assert_eq!(
                            observed_list.len(),
                            usize::from(entity.is_some()),
                            "tick {tick} {tag} presence"
                        );
                        let Some(entity) = entity else { continue };
                        let observed = &observed_list[0];
                        assert_eq!(
                            observed["entity_tick_count"], entity.tick_count,
                            "tick {tick} {tag} tick count"
                        );
                        assert_eq!(
                            observed["ambient_sound_time"], entity.ambient_sound_time,
                            "tick {tick} {tag} ambient"
                        );
                        assert_eq!(
                            observed["no_action_time"], entity.no_action_time,
                            "tick {tick} {tag} inactivity"
                        );
                        assert_eq!(
                            observed["alive"],
                            entity.skeleton.health > 0.0,
                            "tick {tick} {tag} alive"
                        );
                        assert_eq!(observed["no_ai"], entity.no_ai, "tick {tick} {tag} NoAI");
                        for (field, actual) in [
                            ("x", entity.skeleton.body.position.x),
                            ("y", entity.skeleton.body.position.y),
                            ("z", entity.skeleton.body.position.z),
                            ("vx", entity.skeleton.body.velocity.x),
                            ("vy", entity.skeleton.body.velocity.y),
                            ("vz", entity.skeleton.body.velocity.z),
                            ("health", f64::from(entity.skeleton.health)),
                        ] {
                            exact(&observed[field], actual, tick, &format!("{tag} {field}"));
                        }
                        frames += 1;
                    }
                }
            }
            "complete" => complete = true,
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!(hits, 4);
    assert_eq!(frames, 43);
    assert_eq!(expected_frames, 26);
    println!("{hits} skeleton damage results and {frames} exact state frames");
}
