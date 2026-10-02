//! Exact seeded pig FloatGoal and water travel gate against pinned 26.3.
use glam::DVec3;
use minecraftoss_entities::{pig::Pig, world::EntityWorld};
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
        .expect("usage: check_pig_float TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut pig_id = None;
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
                lava = id == "pig_float_lava_seeded";
                assert!(id == "pig_float_deep_water_seeded" || lava);
            }
            "complete" => complete = true,
            "entity_keep_goal" => {
                assert_eq!(row["data"]["goal_class"], "FloatGoal");
                assert_eq!(row["data"]["removed"], 8);
                world
                    .pig_mut(pig_id.unwrap())
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
                    .pig_mut(pig_id.unwrap())
                    .unwrap()
                    .set_random_seed(seed);
                seeds += 1;
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
                            let mut block = Block::new(id);
                            for (name, property) in value["properties"].as_object().unwrap() {
                                block = block.with(name, property.as_str().unwrap());
                            }
                            scene.set_block((pos[0], pos[1], pos[2]), Some(block));
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
                world.tick(&mut scene);
                let entity = world
                    .pigs()
                    .iter()
                    .find(|entity| Some(entity.id) == pig_id)
                    .unwrap();
                let pig = &entity.pig;
                exact(&observed["health"], f64::from(pig.health), "health", tick);
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
                ] {
                    exact(&observed[name], actual, name, tick);
                }
                assert_eq!(
                    pig.body.on_ground,
                    observed["on_ground"].as_bool().unwrap(),
                    "tick {tick} ground"
                );
                assert_eq!(
                    pig.navigation.is_done(),
                    observed["navigation_done"].as_bool().unwrap(),
                    "tick {tick} navigation"
                );
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete && pig_id.is_some());
    assert_eq!((filters, seeds, frames), (1, 1, if lava { 35 } else { 90 }));
    println!(
        "{filters} goal filter, {seeds} seed intervention, {frames} exact pig fluid frames matched"
    );
}
