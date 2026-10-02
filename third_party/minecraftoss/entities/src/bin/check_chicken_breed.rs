//! Exact isolated chicken breeding goal and newborn count against pinned 26.3.
use glam::DVec3;
use minecraftoss_entities::{
    chicken::{Chicken, ChickenVariant},
    world::EntityWorld,
};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

#[derive(Default)]
struct Scene(BTreeMap<Pos, Block>);
impl World for Scene {
    fn block(&self, p: Pos) -> Option<Block> {
        self.0.get(&p).cloned()
    }
    fn set_block(&mut self, p: Pos, b: Option<Block>) {
        if let Some(b) = b {
            self.0.insert(p, b);
        } else {
            self.0.remove(&p);
        }
    }
}
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap())
}
fn exact(expected: &Value, actual: f64, context: &str) {
    assert_eq!(actual.to_bits(), number(expected).to_bits(), "{context}");
}
fn variant(name: &Value) -> ChickenVariant {
    match name.as_str().unwrap() {
        "minecraft:temperate" => ChickenVariant::Temperate,
        "minecraft:warm" => ChickenVariant::Warm,
        "minecraft:cold" => ChickenVariant::Cold,
        other => panic!("unknown chicken variant {other}"),
    }
}
fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_chicken_breed TRACE.jsonl");
    let trace = fs::read_to_string(path).unwrap();
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids = BTreeMap::<String, u64>::new();
    let mut current_tick = 0;
    let mut frames = 0;
    let mut actions = 0;
    let mut seeds = 0;
    let mut complete = false;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        if row["type"] == "manifest" {
            assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "chicken_breed_isolated"
            );
        }
        if row["type"] == "complete" {
            complete = true;
        }
        if row["type"] == "entity_keep_goal" {
            assert_eq!(row["data"]["goal_class"], "BreedGoal");
            assert_eq!(row["data"]["removed"], 7);
            let tag = row["data"]["tag"].as_str().unwrap();
            let goal = row["data"]["goal_class"].as_str().unwrap();
            world.chicken_mut(ids[tag]).unwrap().retain_goals(&[goal]);
            actions += 1;
        }
        if row["type"] == "entity_set_random_seed" {
            let tag = row["data"]["tag"].as_str().unwrap();
            let seed = row["data"]["seed"]
                .as_str()
                .unwrap()
                .parse::<i64>()
                .unwrap();
            if tag == "child" {
                let child_id = world.chickens().last().unwrap().id;
                ids.insert(tag.to_owned(), child_id);
            }
            world.chicken_mut(ids[tag]).unwrap().set_random_seed(seed);
            seeds += 1;
        }
        if row["type"] == "command"
            && row["tick"] == 59
            && row["data"]["command"]
                .as_str()
                .unwrap()
                .starts_with("data merge entity")
        {
            let child_id = world.chickens().last().unwrap().id;
            let child = world.chicken_mut(child_id).unwrap();
            child.no_ai = true;
            child.chicken.egg_time = 10000;
        }
        if row["type"] != "snapshot" {
            continue;
        }
        let tick = row["tick"].as_i64().unwrap();
        if tick == 0 {
            for (key, value) in row["data"]["blocks"].as_object().unwrap() {
                let pos: Vec<i32> = key.split(',').map(|v| v.parse().unwrap()).collect();
                let id = value["id"].as_str().unwrap();
                if id != "minecraft:air" {
                    scene.set_block((pos[0], pos[1], pos[2]), Some(Block::new(id)));
                }
            }
            for (tag, group) in row["data"]["entities"].as_object().unwrap() {
                if group.as_array().unwrap().is_empty() {
                    continue;
                }
                let e = &group.as_array().unwrap()[0];
                assert_ne!(tag, "child");
                let mut chicken = Chicken::new(DVec3::new(
                    number(&e["x"]),
                    number(&e["y"]),
                    number(&e["z"]),
                ));
                chicken.body.velocity =
                    DVec3::new(number(&e["vx"]), number(&e["vy"]), number(&e["vz"]));
                chicken.body.on_ground = e["on_ground"].as_bool().unwrap();
                chicken.yaw = number(&e["yaw"]) as f32;
                chicken.speed = number(&e["speed"]) as f32;
                chicken.age.ticks = e["age"].as_i64().unwrap() as i32;
                chicken.in_love = e["in_love"].as_i64().unwrap() as i32;
                chicken.persistence_required = e["persistence_required"].as_bool().unwrap();
                chicken.variant = variant(&e["chicken_variant"]);
                chicken.egg_time = e["egg_time"].as_i64().unwrap() as i32;
                ids.insert(
                    tag.to_owned(),
                    world.spawn_chicken(chicken, e["no_ai"].as_bool().unwrap()),
                );
            }
        } else {
            assert_eq!(tick, current_tick + 1);
            world.tick(&mut scene);
            for (tag, group) in row["data"]["entities"].as_object().unwrap() {
                if group.as_array().unwrap().is_empty() {
                    continue;
                }
                let e = &group.as_array().unwrap()[0];
                if tag == "child" {
                    assert!(tick >= 59);
                    assert_eq!(world.chickens().last().unwrap().id, ids[tag]);
                    assert_eq!(
                        world.chickens().last().unwrap().chicken.variant,
                        variant(&e["chicken_variant"])
                    );
                }
                let entity = world.chickens().iter().find(|c| c.id == ids[tag]).unwrap();
                let chicken = &entity.chicken;
                assert_eq!(chicken.variant, variant(&e["chicken_variant"]));
                assert_eq!(
                    entity.id as i64,
                    e["entity_numeric_id"].as_i64().unwrap(),
                    "tick {tick} {tag} entity id"
                );
                assert_eq!(
                    entity.tick_count as i64,
                    e["entity_tick_count"].as_i64().unwrap(),
                    "tick {tick} {tag} tick count"
                );
                assert_eq!(
                    chicken.age.ticks as i64,
                    e["age"].as_i64().unwrap(),
                    "tick {tick} {tag} age"
                );
                assert_eq!(
                    chicken.in_love as i64,
                    e["in_love"].as_i64().unwrap(),
                    "tick {tick} {tag} love"
                );
                assert_eq!(
                    chicken.persistence_required,
                    e["persistence_required"].as_bool().unwrap(),
                    "tick {tick} {tag} persistence"
                );
                assert_eq!(
                    chicken.egg_time as i64,
                    e["egg_time"].as_i64().unwrap(),
                    "tick {tick} {tag} egg timer"
                );
                assert_eq!(
                    chicken.is_chicken_jockey,
                    e["chicken_jockey"].as_bool().unwrap(),
                    "tick {tick} {tag} jockey"
                );
                for (name, actual) in [
                    ("flap", f64::from(chicken.flap)),
                    ("flap_speed", f64::from(chicken.flap_speed)),
                    ("flapping", f64::from(chicken.flapping)),
                ] {
                    exact(&e[name], actual, &format!("tick {tick} {tag} {name}"));
                }
                exact(
                    &e["health"],
                    f64::from(chicken.health),
                    &format!("tick {tick} {tag} health"),
                );
                let expected_goals = e["running_goals"].as_array().unwrap();
                let actual_goals = entity.running_goals();
                assert_eq!(
                    actual_goals,
                    expected_goals
                        .iter()
                        .map(|goal| goal.as_str().unwrap())
                        .collect::<Vec<_>>(),
                    "tick {tick} {tag} ordered running goals"
                );
                assert_eq!(
                    entity.breed.running,
                    expected_goals.iter().any(|goal| goal == "BreedGoal"),
                    "tick {tick} {tag} breed goal"
                );
                assert_eq!(
                    expected_goals.len(),
                    usize::from(entity.breed.running),
                    "tick {tick} {tag} other goal"
                );
                for (name, actual) in [
                    ("x", chicken.body.position.x),
                    ("y", chicken.body.position.y),
                    ("z", chicken.body.position.z),
                    ("vx", chicken.body.velocity.x),
                    ("vy", chicken.body.velocity.y),
                    ("vz", chicken.body.velocity.z),
                    ("yaw", chicken.yaw as f64),
                    ("speed", chicken.speed as f64),
                    ("move_control_x", chicken.move_control.wanted.x),
                    ("move_control_y", chicken.move_control.wanted.y),
                    ("move_control_z", chicken.move_control.wanted.z),
                ] {
                    exact(&e[name], actual, &format!("tick {tick} {tag} {name}"));
                }
                if e.get("head_yaw").is_some() {
                    let eye_height: f32 = if chicken.age.baby() { 0.28125 } else { 0.644 };
                    exact(
                        &e["eye_y"],
                        chicken.body.position.y + f64::from(eye_height),
                        &format!("tick {tick} {tag} eye y"),
                    );
                    for (name, actual) in [
                        ("head_yaw", f64::from(entity.look_control.head_yaw)),
                        ("body_yaw", f64::from(entity.body_rotation.body_yaw)),
                        ("pitch", f64::from(entity.look_control.pitch)),
                        ("look_control_x", entity.look_control.wanted.x),
                        ("look_control_y", entity.look_control.wanted.y),
                        ("look_control_z", entity.look_control.wanted.z),
                    ] {
                        exact(&e[name], actual, &format!("tick {tick} {tag} {name}"));
                    }
                    assert_eq!(
                        entity.look_control.cooldown > 0,
                        e["look_control_wanted"].as_bool().unwrap(),
                        "tick {tick} {tag} look wanted"
                    );
                }
                assert_eq!(
                    chicken.body.on_ground,
                    e["on_ground"].as_bool().unwrap(),
                    "tick {tick} {tag} ground"
                );
                assert_eq!(
                    chicken.move_control.has_wanted(),
                    e["move_control_wanted"].as_bool().unwrap(),
                    "tick {tick} {tag} wanted"
                );
                assert_eq!(
                    chicken.navigation.is_done(),
                    e["navigation_done"].as_bool().unwrap(),
                    "tick {tick} {tag} navigation done"
                );
                assert_eq!(
                    chicken.navigation.nodes.len(),
                    e["path_node_count"].as_u64().unwrap() as usize,
                    "tick {tick} {tag} node count"
                );
                assert_eq!(
                    chicken.navigation.observed_next(),
                    e["path_next_node"].as_i64().unwrap() as i32,
                    "tick {tick} {tag} next node"
                );
                assert_eq!(
                    chicken.navigation.reached && !chicken.navigation.nodes.is_empty(),
                    e["path_reached"].as_bool().unwrap(),
                    "tick {tick} {tag} path reached"
                );
                let expected_nodes: Vec<(i32, i32, i32)> = e["path_nodes"]
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
                    chicken.navigation.nodes, expected_nodes,
                    "tick {tick} {tag} path nodes"
                );
                frames += 1;
            }
            let count = &row["data"]["entity_type_counts"]["minecraft:chicken"];
            assert_eq!(
                world.chickens().len(),
                count["total"].as_u64().unwrap() as usize,
                "tick {tick} chicken count"
            );
            assert_eq!(
                world
                    .chickens()
                    .iter()
                    .filter(|e| e.chicken.age.baby())
                    .count(),
                count["babies"].as_u64().unwrap() as usize,
                "tick {tick} baby count"
            );
        }
        current_tick = tick;
    }
    assert!(complete);
    assert_eq!(actions, 2);
    assert_eq!(seeds, 3);
    assert_eq!(frames, 167);
    println!(
        "{actions} goal-filter actions and {frames} chicken AI/navigation/motion frames matched exactly"
    );
}
