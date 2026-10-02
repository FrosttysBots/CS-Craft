//! Exact pinned 26.3 Bat NoAI resting/flying lifecycle gate.
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
        .expect("usage: check_bat_no_ai TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut ids = BTreeMap::<String, u64>::new();
    let mut frames = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "bat_no_ai_resting_and_flying"
            ),
            "complete" => complete = true,
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
                    for tag in ["resting", "flying"] {
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
                    for tag in ["resting", "flying"] {
                        let observed = &data["entities"][tag][0];
                        let entity = world.bats().iter().find(|e| e.id == ids[tag]).unwrap();
                        let bat = &entity.bat;
                        assert_eq!(observed["type"], "minecraft:bat");
                        assert_eq!(observed["no_ai"], entity.no_ai);
                        assert_eq!(observed["persistence_required"], bat.persistence_required);
                        assert_eq!(observed["bat_resting"], bat.resting);
                        assert_eq!(observed["bat_flapping"], bat.is_flapping(entity.tick_count));
                        assert_eq!(observed["entity_numeric_id"], entity.id);
                        assert_eq!(observed["entity_tick_count"], entity.tick_count);
                        assert_eq!(observed["ambient_sound_time"], entity.ambient_sound_time);
                        assert_eq!(observed["on_ground"], bat.body.on_ground);
                        assert_eq!(observed["running_goals"], serde_json::json!([]));
                        assert_eq!(observed["move_control_wanted"], false);
                        assert_eq!(observed["look_control_wanted"], false);
                        for (field, actual) in [
                            ("x", bat.body.position.x),
                            ("y", bat.body.position.y),
                            ("z", bat.body.position.z),
                            ("vx", bat.body.velocity.x),
                            ("vy", bat.body.velocity.y),
                            ("vz", bat.body.velocity.z),
                            ("health", f64::from(bat.health)),
                            ("eye_y", bat.body.position.y + f64::from(0.45_f32)),
                        ] {
                            exact(&observed[field], actual, &format!("{tag} {field}"), tick);
                        }
                        for field in [
                            "yaw",
                            "speed",
                            "head_yaw",
                            "body_yaw",
                            "pitch",
                            "move_control_x",
                            "move_control_y",
                            "move_control_z",
                            "look_control_x",
                            "look_control_y",
                            "look_control_z",
                        ] {
                            exact(&observed[field], 0.0, &format!("{tag} {field}"), tick);
                        }
                        frames += 1;
                    }
                }
            }
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!(frames, 32);
    println!("{frames} exact NoAI bat state and motion frames matched");
}
