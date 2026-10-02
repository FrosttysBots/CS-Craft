//! Exact pinned 26.3 NoAI creeper fuse and removal gate.
use glam::DVec3;
use minecraftoss_entities::{creeper::Creeper, world::EntityWorld};
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
        .expect("usage: check_creeper_ignited TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut ignited_id = None;
    let mut idle_id = None;
    let mut initial_blocks = None;
    let mut frames = 0;
    let mut explosions = 0;
    let mut expected_frames = 0;
    let mut expected_explosion_tick = 0;
    let mut expected_radius = 0.0;
    let mut expected_powered = false;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => {
                let case = &row["data"]["suite"]["scenarios"][0];
                assert!(
                    case["id"] == "creeper_ignited_no_ai" || case["id"] == "creeper_powered_no_ai"
                );
                expected_frames = case["ticks"].as_u64().unwrap() as usize;
            }
            "entity_set_random_seed" => {
                let seed = row["data"]["seed"].as_str().unwrap().parse().unwrap();
                let id = match row["data"]["tag"].as_str().unwrap() {
                    "ignited" => ignited_id.unwrap(),
                    "idle" => idle_id.unwrap(),
                    tag => panic!("unexpected seed target {tag}"),
                };
                world.creeper_mut(id).unwrap().set_random_seed(seed);
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    initial_blocks = Some(data["blocks"].clone());
                    for (tag, slot) in [("ignited", &mut ignited_id), ("idle", &mut idle_id)] {
                        let observed = &data["entities"][tag][0];
                        let mut creeper = Creeper::new(DVec3::new(
                            number(&observed["x"]),
                            number(&observed["y"]),
                            number(&observed["z"]),
                        ));
                        creeper.body.on_ground = observed["on_ground"].as_bool().unwrap();
                        creeper.persistence_required =
                            observed["persistence_required"].as_bool().unwrap();
                        creeper.old_swell = observed["creeper_old_swell"].as_i64().unwrap() as i32;
                        creeper.swell = observed["creeper_swell"].as_i64().unwrap() as i32;
                        creeper.max_swell = observed["creeper_max_swell"].as_i64().unwrap() as i32;
                        creeper.explosion_radius =
                            observed["creeper_explosion_radius"].as_i64().unwrap() as i32;
                        creeper.swell_dir = observed["creeper_swell_dir"].as_i64().unwrap() as i32;
                        creeper.ignited = observed["creeper_ignited"].as_bool().unwrap();
                        creeper.powered = observed["creeper_powered"].as_bool().unwrap();
                        if tag == "ignited" {
                            expected_explosion_tick = i64::from(creeper.max_swell);
                            expected_radius = creeper.explosion_radius as f32
                                * if creeper.powered { 2.0 } else { 1.0 };
                            expected_powered = creeper.powered;
                        }
                        *slot = Some(world.spawn_creeper(creeper, true));
                    }
                    continue;
                }
                frames += 1;
                world.tick(&mut scene);
                assert_eq!(
                    data["game_time"],
                    world.game_time(),
                    "tick {tick} game time"
                );
                assert_eq!(
                    data["blocks"],
                    *initial_blocks.as_ref().unwrap(),
                    "tick {tick} no griefing"
                );
                let events = world.take_creeper_explosions();
                if tick == expected_explosion_tick {
                    assert_eq!(events.len(), 1);
                    let event = events[0];
                    assert_eq!(event.position, DVec3::new(4.5, 1.0, 4.5));
                    assert_eq!(event.radius, expected_radius);
                    assert_eq!(event.powered, expected_powered);
                    explosions += 1;
                } else {
                    assert!(events.is_empty(), "tick {tick} unexpected explosion");
                }
                assert_eq!(
                    data["entity_type_counts"]["minecraft:creeper"]["total"],
                    world.creepers().len(),
                    "tick {tick} creeper count"
                );
                for (tag, id) in [("ignited", ignited_id.unwrap()), ("idle", idle_id.unwrap())] {
                    let references = data["entities"][tag].as_array().unwrap();
                    let current = world.creepers().iter().find(|entity| entity.id == id);
                    assert_eq!(
                        references.len(),
                        usize::from(current.is_some()),
                        "tick {tick} {tag} presence"
                    );
                    if let Some(entity) = current {
                        let observed = &references[0];
                        let body = &entity.creeper.body;
                        for (field, actual) in [
                            ("x", body.position.x),
                            ("y", body.position.y),
                            ("z", body.position.z),
                            ("vx", body.velocity.x),
                            ("vy", body.velocity.y),
                            ("vz", body.velocity.z),
                            ("health", f64::from(entity.creeper.health)),
                        ] {
                            exact(&observed[field], actual, tick, &format!("{tag} {field}"));
                        }
                        assert_eq!(observed["on_ground"], body.on_ground);
                        assert_eq!(observed["no_ai"], entity.no_ai);
                        assert_eq!(
                            observed["persistence_required"],
                            entity.creeper.persistence_required
                        );
                        assert_eq!(observed["creeper_old_swell"], entity.creeper.old_swell);
                        assert_eq!(observed["creeper_swell"], entity.creeper.swell);
                        assert_eq!(observed["creeper_max_swell"], entity.creeper.max_swell);
                        assert_eq!(
                            observed["creeper_explosion_radius"],
                            entity.creeper.explosion_radius
                        );
                        assert_eq!(observed["creeper_swell_dir"], entity.creeper.swell_dir);
                        assert_eq!(observed["creeper_ignited"], entity.creeper.ignited);
                        assert_eq!(observed["creeper_powered"], entity.creeper.powered);
                        assert_eq!(observed["entity_tick_count"], entity.tick_count);
                        assert_eq!(observed["ambient_sound_time"], entity.ambient_sound_time);
                        assert_eq!(observed["no_action_time"], entity.no_action_time);
                        assert_eq!(observed["running_goals"], serde_json::json!([]));
                        assert_eq!(
                            observed["random_state"]
                                .as_str()
                                .unwrap()
                                .parse::<u64>()
                                .unwrap(),
                            entity.random.raw_state()
                        );
                    }
                }
            }
            "complete" => complete = true,
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!(frames, expected_frames);
    assert_eq!(explosions, 1);
    println!("exact creeper fuse trace: {frames} ticks");
}
