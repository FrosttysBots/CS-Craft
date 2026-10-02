//! Exact seeded cow FloatGoal and water travel gate against pinned 26.3.
use glam::DVec3;
use minecraftoss_entities::{cow::Cow, world::EntityWorld};
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
        .expect("usage: check_cow_float TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut cow_id = None;
    let mut filters = 0;
    let mut seeds = 0;
    let mut frames = 0;
    let mut complete = false;
    let mut lava = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => {
                let id = row["data"]["suite"]["scenarios"][0]["id"].as_str().unwrap();
                lava = id == "cow_float_lava_seeded";
                assert!(id == "cow_float_deep_water_seeded" || lava);
            }
            "complete" => complete = true,
            "entity_keep_goal" => {
                assert_eq!(row["data"]["goal_class"], "FloatGoal");
                assert_eq!(row["data"]["removed"], 7);
                world
                    .cow_mut(cow_id.unwrap())
                    .unwrap()
                    .retain_goals(&["FloatGoal"]);
                filters += 1;
            }
            "entity_set_random_seed" => {
                let seed = row["data"]["seed"]
                    .as_str()
                    .unwrap()
                    .parse::<i64>()
                    .unwrap();
                assert_eq!(seed, 59);
                world
                    .cow_mut(cow_id.unwrap())
                    .unwrap()
                    .set_random_seed(seed);
                seeds += 1;
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
                            let mut block = Block::new(id);
                            for (name, property) in value["properties"].as_object().unwrap() {
                                block = block.with(name, property.as_str().unwrap());
                            }
                            scene.set_block((pos[0], pos[1], pos[2]), Some(block));
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
                world.tick(&mut scene);
                let entity = world
                    .cows()
                    .iter()
                    .find(|entity| Some(entity.id) == cow_id)
                    .unwrap();
                let cow = &entity.cow;
                exact(&observed["health"], f64::from(cow.health), "health", tick);
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
                    observed["entity_tick_count"].as_i64().unwrap()
                );
                assert_eq!(
                    entity.ambient_sound_time as i64,
                    observed["ambient_sound_time"].as_i64().unwrap(),
                    "tick {tick} ambient timer"
                );
                assert_eq!(
                    entity.no_action_time as i64,
                    observed["no_action_time"].as_i64().unwrap(),
                    "tick {tick} no-action timer"
                );
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
                ] {
                    exact(&observed[name], actual, name, tick);
                }
                assert_eq!(
                    cow.body.on_ground,
                    observed["on_ground"].as_bool().unwrap(),
                    "tick {tick} ground"
                );
                assert_eq!(
                    cow.navigation.is_done(),
                    observed["navigation_done"].as_bool().unwrap(),
                    "tick {tick} navigation"
                );
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete && cow_id.is_some());
    assert_eq!((filters, seeds, frames), (1, 1, if lava { 35 } else { 90 }));
    println!(
        "{filters} goal filter, {seeds} seed intervention, {frames} exact cow fluid frames matched"
    );
}
