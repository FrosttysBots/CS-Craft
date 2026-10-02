//! Exact mixed-variant mooshroom BreedGoal gate against pinned 26.3.
use glam::DVec3;
use minecraftoss_entities::{
    mooshroom::{MushroomCow, MushroomVariant},
    world::EntityWorld,
};
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
        .expect("usage: check_mooshroom_breeding TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut ids = BTreeMap::<String, u64>::new();
    let mut actions = 0;
    let mut seeds = 0;
    let mut frames = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "mooshroom_breed_mixed_variants"
            ),
            "complete" => complete = true,
            "entity_keep_goal" => {
                assert_eq!(row["data"]["goal_class"], "BreedGoal");
                assert_eq!(row["data"]["removed"], 7);
                let tag = row["data"]["tag"].as_str().unwrap();
                world
                    .cow_mut(ids[tag])
                    .unwrap()
                    .retain_goals(&["BreedGoal"]);
                actions += 1;
            }
            "entity_set_random_seed" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                let seed = row["data"]["seed"]
                    .as_str()
                    .unwrap()
                    .parse::<i64>()
                    .unwrap();
                assert_eq!(seed, if tag == "mate_a" { 59 } else { 2 });
                world.cow_mut(ids[tag]).unwrap().set_random_seed(seed);
                seeds += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                if tick == 0 {
                    for (key, block) in data["blocks"].as_object().unwrap() {
                        let coords: Vec<i32> =
                            key.split(',').map(|part| part.parse().unwrap()).collect();
                        let id = block["id"].as_str().unwrap();
                        if id != "minecraft:air" {
                            scene
                                .set_block((coords[0], coords[1], coords[2]), Some(Block::new(id)));
                        }
                    }
                    for (tag, variant) in [
                        ("mate_a", MushroomVariant::Red),
                        ("mate_b", MushroomVariant::Brown),
                    ] {
                        let observed = &data["entities"][tag][0];
                        let mut mob = MushroomCow::new(
                            DVec3::new(
                                number(&observed["x"]),
                                number(&observed["y"]),
                                number(&observed["z"]),
                            ),
                            variant,
                        );
                        mob.cow.body.velocity = DVec3::new(
                            number(&observed["vx"]),
                            number(&observed["vy"]),
                            number(&observed["vz"]),
                        );
                        mob.cow.body.on_ground = observed["on_ground"].as_bool().unwrap();
                        mob.cow.yaw = number(&observed["yaw"]) as f32;
                        mob.cow.speed = number(&observed["speed"]) as f32;
                        mob.cow.age.ticks = observed["age"].as_i64().unwrap() as i32;
                        mob.cow.in_love = observed["in_love"].as_i64().unwrap() as i32;
                        mob.cow.persistence_required =
                            observed["persistence_required"].as_bool().unwrap();
                        ids.insert(tag.to_owned(), world.spawn_mooshroom(mob, false));
                    }
                } else {
                    world.tick(&mut scene);
                    for tag in ["mate_a", "mate_b"] {
                        let observed = &data["entities"][tag][0];
                        let entity = world
                            .cows()
                            .iter()
                            .find(|entity| entity.id == ids[tag])
                            .unwrap();
                        let cow = &entity.cow;
                        assert_eq!(observed["type"], "minecraft:mooshroom");
                        assert_eq!(
                            observed["mooshroom_variant"],
                            if entity.mooshroom.as_ref().unwrap().variant == MushroomVariant::Red {
                                "red"
                            } else {
                                "brown"
                            }
                        );
                        assert_eq!(observed["entity_numeric_id"], entity.id);
                        assert_eq!(observed["entity_tick_count"], entity.tick_count);
                        assert_eq!(observed["age"], cow.age.ticks);
                        assert_eq!(observed["in_love"], cow.in_love);
                        assert_eq!(observed["persistence_required"], cow.persistence_required);
                        assert_eq!(observed["on_ground"], cow.body.on_ground);
                        assert_eq!(
                            observed["move_control_wanted"],
                            cow.move_control.has_wanted()
                        );
                        assert_eq!(observed["navigation_done"], cow.navigation.is_done());
                        assert_eq!(observed["path_next_node"], cow.navigation.observed_next());
                        assert_eq!(observed["path_node_count"], cow.navigation.nodes.len());
                        assert_eq!(
                            observed["running_goals"],
                            serde_json::json!(entity.running_goals())
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
                        assert_eq!(cow.navigation.nodes, expected_nodes);
                        for (name, actual) in [
                            ("x", cow.body.position.x),
                            ("y", cow.body.position.y),
                            ("z", cow.body.position.z),
                            ("vx", cow.body.velocity.x),
                            ("vy", cow.body.velocity.y),
                            ("vz", cow.body.velocity.z),
                            ("yaw", f64::from(cow.yaw)),
                            ("speed", f64::from(cow.speed)),
                            ("move_control_x", cow.move_control.wanted.x),
                            ("move_control_y", cow.move_control.wanted.y),
                            ("move_control_z", cow.move_control.wanted.z),
                        ] {
                            exact(&observed[name], actual, &format!("{tag} {name}"), tick);
                        }
                        frames += 1;
                    }
                    let mobs: Vec<_> = world
                        .cows()
                        .iter()
                        .filter(|e| e.mooshroom.is_some())
                        .collect();
                    assert_eq!(
                        data["entity_type_counts"]["minecraft:mooshroom"]["total"],
                        mobs.len()
                    );
                    assert_eq!(
                        data["entity_type_counts"]["minecraft:mooshroom"]["babies"],
                        mobs.iter().filter(|e| e.cow.age.baby()).count()
                    );
                    for (name, variant) in [
                        ("red", MushroomVariant::Red),
                        ("brown", MushroomVariant::Brown),
                    ] {
                        let matching: Vec<_> = mobs
                            .iter()
                            .filter(|e| e.mooshroom.as_ref().unwrap().variant == variant)
                            .collect();
                        assert_eq!(
                            data["mooshroom_variant_counts"][name]["total"],
                            matching.len(),
                            "tick {tick} {name} total"
                        );
                        assert_eq!(
                            data["mooshroom_variant_counts"][name]["babies"],
                            matching.iter().filter(|e| e.cow.age.baby()).count(),
                            "tick {tick} {name} babies"
                        );
                    }
                }
            }
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!((actions, seeds, frames), (2, 2, 150));
    println!("{actions} retained BreedGoals, {seeds} RNG seeds, and {frames} exact mooshroom frames matched");
}
