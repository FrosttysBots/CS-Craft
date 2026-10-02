//! Exact pinned 26.3 active Bat flight gate in an empty world.
use glam::DVec3;
use minecraftoss_entities::{bat::Bat, world::EntityWorld};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{env, fs};

#[derive(Default)]
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
        "tick {tick} {label}: actual={actual}, expected={}",
        number(expected)
    );
}
fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_bat_active_flight TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = EmptyWorld;
    let mut id = None;
    let mut frames = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "bat_active_flight_without_roost"
            ),
            "complete" => complete = true,
            "entity_set_random_seed" => {
                world
                    .bat_mut(id.unwrap())
                    .unwrap()
                    .set_random_seed(row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let observed = &row["data"]["entities"]["bat"][0];
                if tick == 0 {
                    let mut bat = Bat::new(DVec3::new(
                        number(&observed["x"]),
                        number(&observed["y"]),
                        number(&observed["z"]),
                    ));
                    bat.resting = observed["bat_resting"].as_bool().unwrap();
                    bat.persistence_required = observed["persistence_required"].as_bool().unwrap();
                    id = Some(world.spawn_bat(bat, false));
                } else {
                    world.tick(&mut scene);
                    let entity = world.bats().iter().find(|e| e.id == id.unwrap()).unwrap();
                    assert_eq!(
                        observed["bat_resting"], entity.bat.resting,
                        "tick {tick} resting"
                    );
                    assert_eq!(
                        observed["bat_flapping"],
                        entity.bat.is_flapping(entity.tick_count),
                        "tick {tick} flapping"
                    );
                    assert_eq!(
                        observed["ambient_sound_time"], entity.ambient_sound_time,
                        "tick {tick} ambient"
                    );
                    assert_eq!(
                        observed["no_action_time"], entity.no_action_time,
                        "tick {tick} no action"
                    );
                    assert_eq!(
                        observed["entity_tick_count"], entity.tick_count,
                        "tick {tick} count"
                    );
                    assert_eq!(
                        observed["entity_numeric_id"], entity.id,
                        "tick {tick} entity id"
                    );
                    assert_eq!(observed["no_ai"], entity.no_ai, "tick {tick} NoAI");
                    assert_eq!(
                        observed["persistence_required"], entity.bat.persistence_required,
                        "tick {tick} persistence"
                    );
                    assert_eq!(
                        observed["running_goals"],
                        serde_json::json!([]),
                        "tick {tick} goals"
                    );
                    assert_eq!(
                        observed["move_control_wanted"], false,
                        "tick {tick} move control"
                    );
                    assert_eq!(
                        observed["look_control_wanted"], false,
                        "tick {tick} look control"
                    );
                    assert_eq!(
                        observed["on_ground"], entity.bat.body.on_ground,
                        "tick {tick} on ground"
                    );
                    for (field, actual) in [
                        ("x", entity.bat.body.position.x),
                        ("y", entity.bat.body.position.y),
                        ("z", entity.bat.body.position.z),
                        ("vx", entity.bat.body.velocity.x),
                        ("vy", entity.bat.body.velocity.y),
                        ("vz", entity.bat.body.velocity.z),
                        ("yaw", f64::from(entity.yaw)),
                        ("head_yaw", f64::from(entity.look_control.head_yaw)),
                        ("body_yaw", f64::from(entity.body_rotation.body_yaw)),
                        ("health", f64::from(entity.bat.health)),
                        ("eye_y", entity.bat.body.position.y + f64::from(0.45_f32)),
                        ("speed", 0.0),
                        ("pitch", f64::from(entity.look_control.pitch)),
                        ("move_control_x", 0.0),
                        ("move_control_y", 0.0),
                        ("move_control_z", 0.0),
                        ("look_control_x", 0.0),
                        ("look_control_y", 0.0),
                        ("look_control_z", 0.0),
                    ] {
                        exact(&observed[field], actual, field, tick);
                    }
                    frames += 1;
                }
            }
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!(frames, 20);
    println!("{frames} exact active bat flight frames matched");
}
