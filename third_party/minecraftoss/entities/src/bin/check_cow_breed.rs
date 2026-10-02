//! Exact cow Breed, FollowParent and Tempt goal traces against pinned 26.3.
use glam::DVec3;
use minecraftoss_entities::{cow::Cow, tempt::PlayerCandidate, world::EntityWorld};
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
fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_cow_breed TRACE.jsonl");
    let trace = fs::read_to_string(path).unwrap();
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids = BTreeMap::<String, u64>::new();
    let mut current_tick = 0;
    let mut frames = 0;
    let mut actions = 0;
    let mut overlap = false;
    let mut temptation = false;
    let mut player = None;
    let mut player_actions = 0;
    let mut complete = false;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        if row["type"] == "manifest" {
            overlap =
                row["data"]["suite"]["scenarios"][0]["id"] == "cow_breed_follow_parent_overlap";
            temptation = row["data"]["suite"]["scenarios"][0]["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("cow_tempt_"));
        }
        if row["type"] == "complete" {
            complete = true;
        }
        if row["type"] == "entity_keep_goal" {
            assert_eq!(
                row["data"]["goal_class"],
                if temptation { "TemptGoal" } else { "BreedGoal" }
            );
            assert_eq!(row["data"]["removed"], 7);
            let tag = row["data"]["tag"].as_str().unwrap();
            let goal = row["data"]["goal_class"].as_str().unwrap();
            world.cow_mut(ids[tag]).unwrap().retain_goals(&[goal]);
            actions += 1;
        }
        if row["type"] == "player_probe" {
            assert_eq!(row["data"]["tag"], "feeder");
            let pos = &row["data"]["pos"];
            let item = row["data"]["item"].as_str().unwrap();
            assert!(item == "minecraft:wheat" || item == "minecraft:air");
            player = Some(PlayerCandidate {
                id: 1,
                position: DVec3::new(
                    pos[0].as_f64().unwrap(),
                    pos[1].as_f64().unwrap(),
                    pos[2].as_f64().unwrap(),
                ),
                eye_height: 1.62,
                main_hand_cow_food: item == "minecraft:wheat",
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
            player_actions += 1;
        }
        if row["type"] == "entity_keep_goals" {
            assert_eq!(row["data"]["tag"], "child");
            assert_eq!(
                row["data"]["goal_classes"],
                serde_json::json!(["BreedGoal", "FollowParentGoal"])
            );
            assert_eq!(row["data"]["removed"], 6);
            let names = row["data"]["goal_classes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap())
                .collect::<Vec<_>>();
            world.cow_mut(ids["child"]).unwrap().retain_goals(&names);
            actions += 1;
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
                let e = &group.as_array().unwrap()[0];
                let mut cow = Cow::new(DVec3::new(
                    number(&e["x"]),
                    number(&e["y"]),
                    number(&e["z"]),
                ));
                cow.body.velocity =
                    DVec3::new(number(&e["vx"]), number(&e["vy"]), number(&e["vz"]));
                cow.body.on_ground = e["on_ground"].as_bool().unwrap();
                cow.yaw = number(&e["yaw"]) as f32;
                cow.speed = number(&e["speed"]) as f32;
                cow.age.ticks = e["age"].as_i64().unwrap() as i32;
                cow.in_love = e["in_love"].as_i64().unwrap() as i32;
                cow.persistence_required = e["persistence_required"].as_bool().unwrap();
                ids.insert(
                    tag.to_owned(),
                    world.spawn_cow(cow, e["no_ai"].as_bool().unwrap()),
                );
            }
        } else {
            assert_eq!(tick, current_tick + 1);
            world.tick_with_players(&mut scene, &player.into_iter().collect::<Vec<_>>());
            for (tag, group) in row["data"]["entities"].as_object().unwrap() {
                let e = &group.as_array().unwrap()[0];
                let entity = world.cows().iter().find(|c| c.id == ids[tag]).unwrap();
                let cow = &entity.cow;
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
                    cow.age.ticks as i64,
                    e["age"].as_i64().unwrap(),
                    "tick {tick} {tag} age"
                );
                assert_eq!(
                    cow.in_love as i64,
                    e["in_love"].as_i64().unwrap(),
                    "tick {tick} {tag} love"
                );
                assert_eq!(
                    cow.persistence_required,
                    e["persistence_required"].as_bool().unwrap(),
                    "tick {tick} {tag} persistence"
                );
                exact(
                    &e["health"],
                    f64::from(cow.health),
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
                    entity.follow_parent.running,
                    expected_goals.iter().any(|goal| goal == "FollowParentGoal"),
                    "tick {tick} {tag} follow parent goal"
                );
                assert_eq!(
                    entity.tempt.running,
                    expected_goals.iter().any(|goal| goal == "TemptGoal"),
                    "tick {tick} {tag} tempt goal"
                );
                assert_eq!(
                    expected_goals.len(),
                    usize::from(entity.breed.running)
                        + usize::from(entity.tempt.running)
                        + usize::from(entity.follow_parent.running),
                    "tick {tick} {tag} other goal"
                );
                for (name, actual) in [
                    ("x", cow.body.position.x),
                    ("y", cow.body.position.y),
                    ("z", cow.body.position.z),
                    ("vx", cow.body.velocity.x),
                    ("vy", cow.body.velocity.y),
                    ("vz", cow.body.velocity.z),
                    ("yaw", cow.yaw as f64),
                    ("speed", cow.speed as f64),
                    ("move_control_x", cow.move_control.wanted.x),
                    ("move_control_y", cow.move_control.wanted.y),
                    ("move_control_z", cow.move_control.wanted.z),
                ] {
                    exact(&e[name], actual, &format!("tick {tick} {tag} {name}"));
                }
                if e.get("head_yaw").is_some() {
                    let eye_height: f32 = if cow.age.baby() { 0.665 } else { 1.3 };
                    exact(
                        &e["eye_y"],
                        cow.body.position.y + f64::from(eye_height),
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
                    cow.body.on_ground,
                    e["on_ground"].as_bool().unwrap(),
                    "tick {tick} {tag} ground"
                );
                assert_eq!(
                    cow.move_control.has_wanted(),
                    e["move_control_wanted"].as_bool().unwrap(),
                    "tick {tick} {tag} wanted"
                );
                assert_eq!(
                    cow.navigation.is_done(),
                    e["navigation_done"].as_bool().unwrap(),
                    "tick {tick} {tag} navigation done"
                );
                assert_eq!(
                    cow.navigation.nodes.len(),
                    e["path_node_count"].as_u64().unwrap() as usize,
                    "tick {tick} {tag} node count"
                );
                assert_eq!(
                    cow.navigation.observed_next(),
                    e["path_next_node"].as_i64().unwrap() as i32,
                    "tick {tick} {tag} next node"
                );
                assert_eq!(
                    cow.navigation.reached && !cow.navigation.nodes.is_empty(),
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
                    cow.navigation.nodes, expected_nodes,
                    "tick {tick} {tag} path nodes"
                );
                frames += 1;
            }
            let count = &row["data"]["entity_type_counts"]["minecraft:cow"];
            assert_eq!(
                world.cows().len(),
                count["total"].as_u64().unwrap() as usize,
                "tick {tick} cow count"
            );
            assert_eq!(
                world.cows().iter().filter(|e| e.cow.age.baby()).count(),
                count["babies"].as_u64().unwrap() as usize,
                "tick {tick} baby count"
            );
        }
        current_tick = tick;
    }
    assert!(complete && actions == if overlap || temptation { 1 } else { 2 });
    assert_eq!(player_actions, if temptation { 3 } else { 0 });
    assert_eq!(
        frames,
        if overlap {
            50
        } else if temptation {
            65
        } else {
            150
        }
    );
    println!(
        "{actions} goal-filter actions and {frames} cow AI/navigation/motion frames matched exactly"
    );
}
