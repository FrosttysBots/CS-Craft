//! Exact isolated sheep EatBlockGoal, grazing, regrowth and animation gate.
use glam::DVec3;
use minecraftoss_entities::{
    age::Age,
    sheep::{Sheep, Wool, DYE_NAMES},
    world::EntityWorld,
};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
};

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

fn exact(value: &Value, actual: f64, label: &str, tick: i64, tag: &str) {
    assert_eq!(
        actual.to_bits(),
        number(value).to_bits(),
        "tick {tick} {tag} {label}"
    );
}

fn block_position(key: &str) -> Pos {
    let parts: Vec<_> = key
        .split(',')
        .map(|part| part.parse::<i32>().unwrap())
        .collect();
    (parts[0], parts[1], parts[2])
}

fn block_from(value: &Value) -> Option<Block> {
    let id = value["id"].as_str().unwrap();
    if id == "minecraft:air" {
        return None;
    }
    let mut block = Block::new(id);
    for (name, value) in value["properties"].as_object().unwrap() {
        block
            .properties
            .insert(name.clone(), value.as_str().unwrap().to_owned());
    }
    Some(block)
}

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_sheep_grazing TRACE.jsonl");
    let trace = fs::read_to_string(path).unwrap();
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut ids = HashMap::<String, u64>::new();
    let mut frames = 0;
    let mut actions = 0;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        match row["type"].as_str().unwrap() {
            "scenario_start" => {
                let no_grief = row["data"]["prepare"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|command| command == "gamerule minecraft:mob_griefing false");
                world.set_mob_griefing(!no_grief);
            }
            "entity_keep_goal" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                assert_eq!(row["data"]["goal_class"], "EatBlockGoal");
                world
                    .sheep_mut(ids[tag])
                    .unwrap()
                    .retain_goals(&["EatBlockGoal"]);
                actions += 1;
            }
            "entity_set_random_seed" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                let seed = row["data"]["seed"]
                    .as_str()
                    .unwrap()
                    .parse::<i64>()
                    .unwrap();
                world.sheep_mut(ids[tag]).unwrap().set_random_seed(seed);
                actions += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let groups = row["data"]["entities"].as_object().unwrap();
                if tick == 0 {
                    for (key, value) in row["data"]["blocks"].as_object().unwrap() {
                        if let Some(block) = block_from(value) {
                            scene.set_block(block_position(key), Some(block));
                        }
                    }
                    let mut ordered: Vec<_> = groups
                        .iter()
                        .map(|(tag, states)| (tag, &states[0]))
                        .collect();
                    ordered.sort_by_key(|(_, state)| state["entity_numeric_id"].as_i64().unwrap());
                    for (tag, state) in ordered {
                        let color = DYE_NAMES
                            .iter()
                            .position(|name| *name == state["sheep_color"].as_str().unwrap())
                            .unwrap() as u8;
                        let mut wool = Wool::default();
                        wool.set_color(color);
                        wool.set_sheared(state["sheared"].as_bool().unwrap());
                        let id = world.spawn_sheep(
                            Sheep {
                                age: Age {
                                    ticks: state["age"].as_i64().unwrap() as i32,
                                    ..Default::default()
                                },
                                wool,
                                ..Default::default()
                            },
                            DVec3::new(
                                number(&state["x"]),
                                number(&state["y"]),
                                number(&state["z"]),
                            ),
                            false,
                        );
                        assert_eq!(id as i64, state["entity_numeric_id"].as_i64().unwrap());
                        ids.insert(tag.clone(), id);
                    }
                } else {
                    assert_eq!(tick, frames + 1);
                    world.tick(&mut scene);
                }
                for (tag, states) in groups {
                    let state = &states[0];
                    let entity = world.sheep_mut(ids[tag]).unwrap();
                    assert_eq!(state["type"], "minecraft:sheep");
                    assert_eq!(
                        state["entity_tick_count"], entity.tick_count,
                        "tick {tick} {tag} tick count"
                    );
                    assert_eq!(state["no_ai"], entity.no_ai, "tick {tick} {tag} NoAI");
                    assert_eq!(
                        state["on_ground"], entity.body.on_ground,
                        "tick {tick} {tag} ground"
                    );
                    assert_eq!(
                        state["age"], entity.sheep.age.ticks,
                        "tick {tick} {tag} age"
                    );
                    assert_eq!(
                        state["forced_age"], entity.sheep.age.forced,
                        "tick {tick} {tag} forced age"
                    );
                    assert_eq!(
                        state["sheared"],
                        entity.sheep.wool.sheared(),
                        "tick {tick} {tag} sheared"
                    );
                    assert_eq!(
                        state["sheep_color"],
                        entity.sheep.wool.color(),
                        "tick {tick} {tag} color"
                    );
                    assert_eq!(
                        state["ambient_sound_time"], entity.ambient_sound_time,
                        "tick {tick} {tag} ambient"
                    );
                    assert_eq!(
                        state["no_action_time"], entity.no_action_time,
                        "tick {tick} {tag} inactivity"
                    );
                    let goals: Vec<_> = state["running_goals"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|value| value.as_str().unwrap())
                        .collect();
                    assert_eq!(goals, entity.running_goals(), "tick {tick} {tag} goals");
                    for (name, actual) in [
                        ("x", entity.body.position.x),
                        ("y", entity.body.position.y),
                        ("z", entity.body.position.z),
                        ("vx", entity.body.velocity.x),
                        ("vy", entity.body.velocity.y),
                        ("vz", entity.body.velocity.z),
                        ("health", f64::from(entity.health)),
                        (
                            "eat_head_position_scale",
                            f64::from(entity.eat_head_position_scale(0.0)),
                        ),
                        (
                            "eat_head_angle_scale",
                            f64::from(entity.eat_head_angle_scale(0.0)),
                        ),
                    ] {
                        exact(&state[name], actual, name, tick, tag);
                    }
                }
                for (key, expected) in row["data"]["blocks"].as_object().unwrap() {
                    let actual = scene.block(block_position(key));
                    assert_eq!(actual, block_from(expected), "tick {tick} block {key}");
                }
                if tick > 0 {
                    frames += 1;
                }
            }
            _ => {}
        }
    }
    assert_eq!(actions, 4);
    assert_eq!(frames, 50);
    println!("{actions} goal/seed interventions and {frames} exact sheep world frames matched");
}
