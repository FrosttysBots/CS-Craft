//! Exact pinned 26.3 active Bat roost and player wakeup gate.
use glam::DVec3;
use minecraftoss_entities::{bat::Bat, tempt::PlayerCandidate, world::EntityWorld};
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
        "tick {tick} {label}: actual={actual}, expected={}",
        number(expected)
    );
}
fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_bat_roost TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut players = Vec::new();
    let mut id = None;
    let mut frames = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "bat_roost_and_nearby_player_wakeup"
            ),
            "complete" => complete = true,
            "entity_set_random_seed" => world
                .bat_mut(id.unwrap())
                .unwrap()
                .set_random_seed(row["data"]["seed"].as_str().unwrap().parse().unwrap()),
            "player_probe" => {
                let pos = row["data"]["pos"].as_array().unwrap();
                players.push(PlayerCandidate {
                    id: 1,
                    position: DVec3::new(
                        pos[0].as_f64().unwrap(),
                        pos[1].as_f64().unwrap(),
                        pos[2].as_f64().unwrap(),
                    ),
                    eye_height: 1.62,
                    main_hand_cow_food: false,
                    offhand_cow_food: false,
                    main_hand_pig_food: false,
                    offhand_pig_food: false,
                    main_hand_chicken_food: false,
                    offhand_chicken_food: false,
                    main_hand_carrot_on_a_stick: false,
                    offhand_carrot_on_a_stick: false,
                    main_hand_wolf_interest: false,
                    offhand_wolf_interest: false,
                    main_hand_horse_tempt: false,
                    offhand_horse_tempt: false,
                    alive: true,
                    spectator: false,
                    attackable: false,
                });
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = &data["entities"]["bat"][0];
                if tick == 0 {
                    for (key, block) in data["blocks"].as_object().unwrap() {
                        let pos: Vec<i32> =
                            key.split(',').map(|part| part.parse().unwrap()).collect();
                        let block_id = block["id"].as_str().unwrap();
                        if block_id != "minecraft:air" {
                            scene.set_block((pos[0], pos[1], pos[2]), Some(Block::new(block_id)));
                        }
                    }
                    let mut bat = Bat::new(DVec3::new(
                        number(&observed["x"]),
                        number(&observed["y"]),
                        number(&observed["z"]),
                    ));
                    bat.resting = observed["bat_resting"].as_bool().unwrap();
                    bat.persistence_required = observed["persistence_required"].as_bool().unwrap();
                    id = Some(world.spawn_bat(bat, false));
                } else {
                    world.tick_with_players(&mut scene, &players);
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
    assert_eq!(frames, 12);
    println!("{frames} exact active bat roost and wakeup frames matched");
}
