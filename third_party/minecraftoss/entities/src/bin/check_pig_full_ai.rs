//! Exact seeded unfiltered pig AI against pinned Minecraft 26.3.
#[path = "support/sounds.rs"]
mod sounds;
use glam::DVec3;
use minecraftoss_entities::{pig::Pig, tempt::PlayerCandidate, world::EntityWorld};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

#[derive(Default)]
struct Scene(BTreeMap<Pos, Block>);
impl World for Scene {
    fn step_sound(&self, pos: Pos) -> Option<(String, f32, f32)> {
        sounds::step_sound(&self.0.get(&pos)?.id)
    }
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
    let mut heard = 0;
    let path = env::args()
        .nth(1)
        .expect("usage: check_pig_full_ai TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut pig_id = None;
    let mut seeds = 0;
    let mut frames = 0;
    let mut complete = false;
    let mut idle_seed = false;
    let mut player = None;
    let mut probes = 0;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => {
                let scenario = row["data"]["suite"]["scenarios"][0]["id"].as_str().unwrap();
                idle_seed = scenario == "pig_ai_unfiltered_idle_seeded";
                assert!(scenario == "pig_ai_unfiltered_seeded" || idle_seed);
            }
            "complete" => complete = true,
            "entity_keep_goal" | "entity_keep_goals" => {
                panic!("unfiltered scenario cannot remove goals")
            }
            "entity_set_random_seed" => {
                let seed = row["data"]["seed"]
                    .as_str()
                    .unwrap()
                    .parse::<i64>()
                    .unwrap();
                assert_eq!(seed, if idle_seed { 2 } else { 59 });
                world
                    .pig_mut(pig_id.unwrap())
                    .unwrap()
                    .set_random_seed(seed);
                seeds += 1;
            }
            "player_probe" => {
                let item = row["data"]["item"].as_str().unwrap();
                assert!(matches!(item, "minecraft:air" | "minecraft:carrot"));
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
                    main_hand_pig_food: item == "minecraft:carrot",
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
                let observed = &row["data"]["entities"]["pig"][0];
                if tick == 0 {
                    for (key, value) in row["data"]["blocks"].as_object().unwrap() {
                        let pos: Vec<i32> =
                            key.split(',').map(|part| part.parse().unwrap()).collect();
                        let id = value["id"].as_str().unwrap();
                        if id != "minecraft:air" {
                            scene.set_block((pos[0], pos[1], pos[2]), Some(Block::new(id)));
                        }
                    }
                    let mut pig = Pig::new(DVec3::new(
                        number(&observed["x"]),
                        number(&observed["y"]),
                        number(&observed["z"]),
                    ));
                    pig.body.velocity = DVec3::new(
                        number(&observed["vx"]),
                        number(&observed["vy"]),
                        number(&observed["vz"]),
                    );
                    pig.body.on_ground = observed["on_ground"].as_bool().unwrap();
                    pig.yaw = number(&observed["yaw"]) as f32;
                    pig.age.ticks = observed["age"].as_i64().unwrap() as i32;
                    pig_id = Some(world.spawn_pig(pig, false));
                    continue;
                }
                assert_eq!(tick, frames + 1);
                world.tick_with_players(&mut scene, &player.into_iter().collect::<Vec<_>>());
                heard += sounds::check(&mut world, &row["data"], &format!("tick {tick}"), &[]);
                let entity = world
                    .pigs()
                    .iter()
                    .find(|entity| Some(entity.id) == pig_id)
                    .unwrap();
                let pig = &entity.pig;
                assert_eq!(
                    entity.id as i64,
                    observed["entity_numeric_id"].as_i64().unwrap(),
                    "tick {tick} numeric id"
                );
                assert_eq!(
                    pig.age.ticks as i64,
                    observed["age"].as_i64().unwrap(),
                    "tick {tick} age"
                );
                assert_eq!(
                    pig.in_love as i64,
                    observed["in_love"].as_i64().unwrap(),
                    "tick {tick} love timer"
                );
                assert_eq!(
                    entity.no_ai,
                    observed["no_ai"].as_bool().unwrap(),
                    "tick {tick} no AI"
                );
                exact(&observed["health"], f64::from(pig.health), "health", tick);
                exact(
                    &observed["eye_y"],
                    pig.body.position.y + f64::from(0.765_f32),
                    "eye Y",
                    tick,
                );
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
                assert_eq!(
                    entity.fluid.in_water(),
                    observed["in_water"].as_bool().unwrap(),
                    "tick {tick} water"
                );
                assert_eq!(
                    entity.fluid.in_lava(),
                    observed["in_lava"].as_bool().unwrap(),
                    "tick {tick} lava"
                );
                assert_eq!(
                    entity.jumping,
                    observed["jumping"].as_bool().unwrap(),
                    "tick {tick} jumping"
                );
                exact(
                    &observed["water_height"],
                    entity.fluid.water_height,
                    "water height",
                    tick,
                );
                exact(
                    &observed["lava_height"],
                    entity.fluid.lava_height,
                    "lava height",
                    tick,
                );
                exact(
                    &observed["floatable_fluid_height"],
                    entity.fluid.floatable_height(),
                    "floatable height",
                    tick,
                );
                for (name, actual) in [
                    ("x", pig.body.position.x),
                    ("y", pig.body.position.y),
                    ("z", pig.body.position.z),
                    ("vx", pig.body.velocity.x),
                    ("vy", pig.body.velocity.y),
                    ("vz", pig.body.velocity.z),
                    ("yaw", f64::from(pig.yaw)),
                    ("speed", f64::from(pig.speed)),
                    ("head_yaw", f64::from(entity.look_control.head_yaw)),
                    ("body_yaw", f64::from(entity.body_rotation.body_yaw)),
                    ("pitch", f64::from(entity.look_control.pitch)),
                    ("move_control_x", pig.move_control.wanted.x),
                    ("move_control_y", pig.move_control.wanted.y),
                    ("move_control_z", pig.move_control.wanted.z),
                    ("look_control_x", entity.look_control.wanted.x),
                    ("look_control_y", entity.look_control.wanted.y),
                    ("look_control_z", entity.look_control.wanted.z),
                ] {
                    exact(&observed[name], actual, name, tick);
                }
                assert_eq!(
                    pig.body.on_ground,
                    observed["on_ground"].as_bool().unwrap(),
                    "tick {tick} on ground"
                );
                assert_eq!(
                    pig.move_control.has_wanted(),
                    observed["move_control_wanted"].as_bool().unwrap(),
                    "tick {tick} move wanted"
                );
                assert_eq!(
                    entity.look_control.cooldown > 0,
                    observed["look_control_wanted"].as_bool().unwrap(),
                    "tick {tick} look wanted"
                );
                assert_eq!(
                    pig.navigation.is_done(),
                    observed["navigation_done"].as_bool().unwrap(),
                    "tick {tick} navigation done"
                );
                assert_eq!(
                    pig.navigation.observed_next() as i64,
                    observed["path_next_node"].as_i64().unwrap(),
                    "tick {tick} path next"
                );
                assert_eq!(
                    pig.navigation.reached && !pig.navigation.nodes.is_empty(),
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
                    pig.navigation.nodes.len() as i64,
                    observed["path_node_count"].as_i64().unwrap(),
                    "tick {tick} path node count"
                );
                assert_eq!(
                    pig.navigation.nodes, expected_nodes,
                    "tick {tick} path nodes"
                );
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete && pig_id.is_some());
    assert_eq!(
        (seeds, probes, frames),
        if idle_seed { (1, 3, 120) } else { (1, 4, 140) }
    );
    println!("{seeds} seed intervention, {probes} player probes, {frames} exact unfiltered pig frames matched ({heard} sounds)");
}
