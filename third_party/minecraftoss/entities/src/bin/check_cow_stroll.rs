//! Exact seeded dry-ground cow strolling against pinned Minecraft 26.3.
use glam::DVec3;
use minecraftoss_entities::{cow::Cow, tempt::PlayerCandidate, world::EntityWorld};
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
        .expect("usage: check_cow_stroll TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut cow_id = None;
    let mut filters = 0;
    let mut seeds = 0;
    let mut frames = 0;
    let mut complete = false;
    let mut near_player = false;
    let mut player = None;
    let mut probes = 0;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => {
                let scenario = row["data"]["suite"]["scenarios"][0]["id"].as_str().unwrap();
                near_player = scenario == "cow_random_stroll_near_player";
                assert!(scenario == "cow_random_stroll_seeded" || near_player);
            }
            "complete" => complete = true,
            "entity_keep_goal" => {
                assert_eq!(row["data"]["goal_class"], "WaterAvoidingRandomStrollGoal");
                assert_eq!(row["data"]["removed"], 7);
                world
                    .cow_mut(cow_id.unwrap())
                    .unwrap()
                    .retain_goals(&["WaterAvoidingRandomStrollGoal"]);
                filters += 1;
            }
            "entity_set_random_seed" => {
                let seed = row["data"]["seed"]
                    .as_str()
                    .unwrap()
                    .parse::<i64>()
                    .unwrap();
                assert_eq!(seed, 17);
                world
                    .cow_mut(cow_id.unwrap())
                    .unwrap()
                    .set_random_seed(seed);
                seeds += 1;
            }
            "player_probe" => {
                assert!(near_player);
                assert_eq!(row["data"]["item"], "minecraft:air");
                let pos = &row["data"]["pos"];
                player = Some(PlayerCandidate {
                    id: 0,
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
                probes += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let observed = &row["data"]["entities"]["cow"][0];
                if tick == 0 {
                    for (key, value) in row["data"]["blocks"].as_object().unwrap() {
                        let pos: Vec<i32> =
                            key.split(',').map(|part| part.parse().unwrap()).collect();
                        let id = value["id"].as_str().unwrap();
                        if id != "minecraft:air" {
                            scene.set_block((pos[0], pos[1], pos[2]), Some(Block::new(id)));
                        }
                    }
                    let mut cow = Cow::new(DVec3::new(
                        number(&observed["x"]),
                        number(&observed["y"]),
                        number(&observed["z"]),
                    ));
                    cow.body.velocity = DVec3::new(
                        number(&observed["vx"]),
                        number(&observed["vy"]),
                        number(&observed["vz"]),
                    );
                    cow.body.on_ground = observed["on_ground"].as_bool().unwrap();
                    cow.yaw = number(&observed["yaw"]) as f32;
                    cow.age.ticks = observed["age"].as_i64().unwrap() as i32;
                    cow_id = Some(world.spawn_cow(cow, false));
                    continue;
                }
                assert_eq!(tick, frames + 1);
                world.tick_with_players(&mut scene, &player.into_iter().collect::<Vec<_>>());
                let entity = world
                    .cows()
                    .iter()
                    .find(|entity| Some(entity.id) == cow_id)
                    .unwrap();
                let cow = &entity.cow;
                assert_eq!(
                    entity.running_goals(),
                    observed["running_goals"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap())
                        .collect::<Vec<_>>(),
                    "tick {tick} goals"
                );
                assert_eq!(
                    entity.tick_count as i64,
                    observed["entity_tick_count"].as_i64().unwrap(),
                    "tick {tick} entity ticks"
                );
                assert_eq!(
                    entity.ambient_sound_time as i64,
                    observed["ambient_sound_time"].as_i64().unwrap(),
                    "tick {tick} ambient timer"
                );
                if observed.get("no_action_time").is_some() {
                    assert_eq!(
                        entity.no_action_time as i64,
                        observed["no_action_time"].as_i64().unwrap(),
                        "tick {tick} no-action timer"
                    );
                }
                for (name, actual) in [
                    ("x", cow.body.position.x),
                    ("y", cow.body.position.y),
                    ("z", cow.body.position.z),
                    ("vx", cow.body.velocity.x),
                    ("vy", cow.body.velocity.y),
                    ("vz", cow.body.velocity.z),
                    ("yaw", f64::from(cow.yaw)),
                    ("speed", f64::from(cow.speed)),
                    ("head_yaw", f64::from(entity.look_control.head_yaw)),
                    ("body_yaw", f64::from(entity.body_rotation.body_yaw)),
                    ("pitch", f64::from(entity.look_control.pitch)),
                    ("move_control_x", cow.move_control.wanted.x),
                    ("move_control_y", cow.move_control.wanted.y),
                    ("move_control_z", cow.move_control.wanted.z),
                    ("look_control_x", entity.look_control.wanted.x),
                    ("look_control_y", entity.look_control.wanted.y),
                    ("look_control_z", entity.look_control.wanted.z),
                ] {
                    exact(&observed[name], actual, name, tick);
                }
                assert_eq!(
                    cow.body.on_ground,
                    observed["on_ground"].as_bool().unwrap(),
                    "tick {tick} on ground"
                );
                assert_eq!(
                    cow.move_control.has_wanted(),
                    observed["move_control_wanted"].as_bool().unwrap(),
                    "tick {tick} move wanted"
                );
                assert_eq!(
                    entity.look_control.cooldown > 0,
                    observed["look_control_wanted"].as_bool().unwrap(),
                    "tick {tick} look wanted"
                );
                assert_eq!(
                    cow.navigation.is_done(),
                    observed["navigation_done"].as_bool().unwrap(),
                    "tick {tick} navigation done"
                );
                assert_eq!(
                    cow.navigation.observed_next() as i64,
                    observed["path_next_node"].as_i64().unwrap(),
                    "tick {tick} path next"
                );
                assert_eq!(
                    cow.navigation.reached && !cow.navigation.nodes.is_empty(),
                    observed["path_reached"].as_bool().unwrap(),
                    "tick {tick} path reached"
                );
                let expected_nodes: Vec<(i32, i32, i32)> = observed["path_nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| {
                        (
                            p[0].as_i64().unwrap() as i32,
                            p[1].as_i64().unwrap() as i32,
                            p[2].as_i64().unwrap() as i32,
                        )
                    })
                    .collect();
                assert_eq!(
                    cow.navigation.nodes, expected_nodes,
                    "tick {tick} path nodes"
                );
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete && cow_id.is_some());
    assert_eq!(
        (filters, seeds, probes, frames),
        if near_player {
            (1, 1, 1, 120)
        } else {
            (1, 1, 0, 90)
        }
    );
    println!("{filters} goal filter, {seeds} seed intervention, {probes} player probes, {frames} exact cow strolling frames matched");
}
