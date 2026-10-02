//! Exact pinned 26.3 isolated zombie player-pursuit gate.
use glam::DVec3;
use minecraftoss_entities::{tempt::PlayerCandidate, world::EntityWorld, zombie::Zombie};
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
fn exact(observed: &Value, actual: f64, label: &str, tick: i64) {
    assert_eq!(
        actual.to_bits(),
        number(observed).to_bits(),
        "tick {tick} {label}"
    );
}
fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_zombie_pursuit TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut players = Vec::new();
    let mut zombie_id = None;
    let mut frames = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "zombie_player_pursuit_isolated_attack_goal"
            ),
            "complete" => complete = true,
            "entity_set_random_seed" => world
                .zombie_mut(zombie_id.unwrap())
                .unwrap()
                .set_random_seed(row["data"]["seed"].as_str().unwrap().parse().unwrap()),
            "player_probe" => {
                assert_eq!(row["data"]["game_mode"], "survival");
                let pos = &row["data"]["pos"];
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
                    attackable: true,
                });
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = &data["entities"]["zombie"][0];
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    for (key, block) in data["blocks"].as_object().unwrap() {
                        let pos: Vec<i32> =
                            key.split(',').map(|part| part.parse().unwrap()).collect();
                        if block["id"] != "minecraft:air" {
                            scene.set_block(
                                (pos[0], pos[1], pos[2]),
                                Some(Block::new(block["id"].as_str().unwrap())),
                            );
                        }
                    }
                    let mut zombie = Zombie::new(DVec3::new(
                        number(&observed["x"]),
                        number(&observed["y"]),
                        number(&observed["z"]),
                    ));
                    zombie.body.velocity = DVec3::new(
                        number(&observed["vx"]),
                        number(&observed["vy"]),
                        number(&observed["vz"]),
                    );
                    zombie.body.on_ground = observed["on_ground"].as_bool().unwrap();
                    zombie.persistence_required =
                        observed["persistence_required"].as_bool().unwrap();
                    zombie_id = Some(world.spawn_zombie_pursuit(zombie));
                } else {
                    world.tick_with_players(&mut scene, &players);
                    assert_eq!(
                        world.game_time(),
                        data["game_time"].as_i64().unwrap(),
                        "tick {tick} game time"
                    );
                    let entity = world
                        .zombies()
                        .iter()
                        .find(|entity| Some(entity.id) == zombie_id)
                        .unwrap();
                    let zombie = &entity.zombie;
                    assert_eq!(data["game_time"], tick + 1, "tick {tick} game time");
                    assert_eq!(
                        observed["entity_tick_count"], entity.tick_count,
                        "tick {tick} count"
                    );
                    assert_eq!(
                        observed["ambient_sound_time"], entity.ambient_sound_time,
                        "tick {tick} ambient"
                    );
                    assert_eq!(
                        observed["no_action_time"], entity.no_action_time,
                        "tick {tick} inactivity"
                    );
                    assert_eq!(
                        observed["target_uuid"].is_string(),
                        entity.target_player_id.is_some(),
                        "tick {tick} target"
                    );
                    assert_eq!(
                        observed["aggressive"], entity.aggressive,
                        "tick {tick} aggressive"
                    );
                    assert_eq!(
                        observed["running_goals"].as_array().unwrap().len() == 1,
                        entity.attack_goal_running,
                        "tick {tick} goal"
                    );
                    assert_eq!(
                        observed["on_ground"], zombie.body.on_ground,
                        "tick {tick} ground"
                    );
                    assert_eq!(
                        observed["navigation_done"],
                        entity.navigation.is_done(),
                        "tick {tick} navigation"
                    );
                    assert_eq!(
                        observed["path_next_node"],
                        entity.navigation.observed_next(),
                        "tick {tick} path next"
                    );
                    let nodes: Vec<_> = observed["path_nodes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|node| {
                            (
                                node[0].as_i64().unwrap() as i32,
                                node[1].as_i64().unwrap() as i32,
                                node[2].as_i64().unwrap() as i32,
                            )
                        })
                        .collect();
                    assert_eq!(nodes, entity.navigation.nodes, "tick {tick} path");
                    assert_eq!(
                        observed["move_control_wanted"],
                        entity.move_control.has_wanted(),
                        "tick {tick} move wanted"
                    );
                    assert_eq!(
                        observed["look_control_wanted"],
                        entity.look_control.cooldown > 0,
                        "tick {tick} look wanted"
                    );
                    for (field, actual) in [
                        ("x", zombie.body.position.x),
                        ("y", zombie.body.position.y),
                        ("z", zombie.body.position.z),
                        ("vx", zombie.body.velocity.x),
                        ("vy", zombie.body.velocity.y),
                        ("vz", zombie.body.velocity.z),
                        ("yaw", f64::from(entity.yaw)),
                        ("speed", f64::from(entity.speed)),
                        ("head_yaw", f64::from(entity.look_control.head_yaw)),
                        ("body_yaw", f64::from(entity.body_rotation.body_yaw)),
                        ("pitch", f64::from(entity.look_control.pitch)),
                        (
                            "eye_y",
                            zombie.body.position.y + f64::from(zombie.eye_height()),
                        ),
                        ("move_control_x", entity.move_control.wanted.x),
                        ("move_control_y", entity.move_control.wanted.y),
                        ("move_control_z", entity.move_control.wanted.z),
                        ("look_control_x", entity.look_control.wanted.x),
                        ("look_control_y", entity.look_control.wanted.y),
                        ("look_control_z", entity.look_control.wanted.z),
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
    assert_eq!(frames, 50);
    println!("{frames} exact zombie pursuit frames matched");
}
