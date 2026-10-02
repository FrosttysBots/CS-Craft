//! Exact shared-world sheep goal traces against pinned 26.3.
#[path = "support/sounds.rs"]
mod sounds;
use glam::DVec3;
use minecraftoss_entities::{
    age::Age,
    sheep::{Sheep, Wool, DYE_NAMES},
    tempt::PlayerCandidate,
    world::{DamageSourceKind, EntityWorld},
};
use minecraftoss_player::{crafting::RecipeBook, Block, Pos, World};
use serde_json::Value;
use std::{collections::HashMap, env, fs, path::Path, sync::Arc};

struct FlatWorld {
    floor: &'static str,
    extent: i32,
    fluid: Option<&'static str>,
}
impl World for FlatWorld {
    fn block(&self, p: Pos) -> Option<Block> {
        if let Some(fluid) = self.fluid {
            if !(0..16).contains(&p.0) || !(0..16).contains(&p.2) || !(0..=4).contains(&p.1) {
                return None;
            }
            if p.0 == 0 || p.0 == 15 || p.2 == 0 || p.2 == 15 || p.1 == 0 {
                return Some(Block::new("minecraft:stone"));
            }
            return (p.1 <= 3).then(|| Block::new(fluid));
        }
        (p.1 == 0 && (0..self.extent).contains(&p.0) && (0..self.extent).contains(&p.2))
            .then(|| Block::new(self.floor))
    }
    fn set_block(&mut self, _: Pos, _: Option<Block>) {}
    fn step_sound(&self, p: Pos) -> Option<(String, f32, f32)> {
        sounds::step_sound(&self.block(p)?.id)
    }
}
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap())
}
fn exact(v: &Value, actual: f64, field: &str, tick: i64, tag: &str) {
    assert_eq!(
        number(v).to_bits(),
        actual.to_bits(),
        "tick {tick} {tag} {field}: expected {}, actual {actual}",
        number(v)
    );
}
fn main() {
    let mut heard = 0;
    let trace = fs::read_to_string(
        env::args()
            .nth(1)
            .expect("usage: check_sheep_breeding TRACE.jsonl"),
    )
    .unwrap();
    let mut world = EntityWorld::default();
    let jar = env::args()
        .nth(2)
        .expect("second argument: pinned common JAR");
    world.set_recipe_book(Arc::new(RecipeBook::from_jar(Path::new(&jar)).unwrap()));
    let mut ids = HashMap::<String, u64>::new();
    let mut frames = 0;
    let mut expected_frames = None;
    let mut player = None;
    let mut floor = FlatWorld {
        floor: "minecraft:stone",
        extent: 16,
        fluid: None,
    };
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let tick = row["tick"].as_i64().unwrap();
        match row["type"].as_str().unwrap() {
            "manifest" => {
                let scenario = &row["data"]["suite"]["scenarios"][0];
                expected_frames = scenario["ticks"].as_i64();
                if scenario["id"] == "sheep_random_stroll_seeded"
                    || scenario["id"] == "sheep_panic_magic_seeded"
                    || scenario["id"] == "sheep_panic_source_change"
                    || scenario["id"] == "sheep_ai_unfiltered_seeded"
                    || scenario["id"] == "sheep_ai_unfiltered_idle_seeded"
                {
                    floor = FlatWorld {
                        floor: "minecraft:grass_block",
                        extent: 26,
                        fluid: None,
                    };
                }
                if scenario["id"] == "sheep_float_deep_water_seeded" {
                    floor.fluid = Some("minecraft:water");
                }
                if scenario["id"] == "sheep_float_lava_seeded" {
                    floor.fluid = Some("minecraft:lava");
                }
            }
            "entity_keep_goal" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                let goal = row["data"]["goal_class"].as_str().unwrap();
                world.sheep_mut(ids[tag]).unwrap().retain_goals(&[goal]);
            }
            "entity_keep_goals" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                let goals = row["data"]["goal_classes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap())
                    .collect::<Vec<_>>();
                world.sheep_mut(ids[tag]).unwrap().retain_goals(&goals);
            }
            "entity_set_random_seed" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                let seed = row["data"]["seed"].as_str().unwrap().parse().unwrap();
                world.sheep_mut(ids[tag]).unwrap().set_random_seed(seed);
            }
            "entity_hurt" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                let source = match row["data"]["source"].as_str().unwrap() {
                    "minecraft:magic" => DamageSourceKind::Magic,
                    "minecraft:generic" => DamageSourceKind::Generic,
                    other => panic!("unexpected damage source {other}"),
                };
                let entity = world.sheep_mut(ids[tag]).unwrap();
                exact(
                    &row["data"]["health_before"],
                    f64::from(entity.health),
                    "health before",
                    tick,
                    tag,
                );
                let result = entity.hurt(number(&row["data"]["amount"]) as f32, source);
                assert_eq!(
                    row["data"]["applied"], result.applied,
                    "tick {tick} {tag} hit"
                );
                exact(
                    &row["data"]["health_after"],
                    f64::from(entity.health),
                    "health after",
                    tick,
                    tag,
                );
            }
            "player_probe" => {
                let pos = &row["data"]["pos"];
                player = Some(PlayerCandidate {
                    id: 1,
                    position: DVec3::new(
                        pos[0].as_f64().unwrap(),
                        pos[1].as_f64().unwrap(),
                        pos[2].as_f64().unwrap(),
                    ),
                    eye_height: 1.62,
                    main_hand_cow_food: row["data"]["item"] == "minecraft:wheat",
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
            "command" if tick == 59 => {
                if row["data"]["command"].as_str().unwrap().contains("NoAI:1b") {
                    let child = world
                        .sheep()
                        .iter()
                        .find(|e| !ids.values().any(|id| *id == e.id))
                        .unwrap()
                        .id;
                    world.sheep_mut(child).unwrap().no_ai = true;
                } else if row["data"]["command"]
                    .as_str()
                    .unwrap()
                    .contains("add child")
                {
                    let child = world
                        .sheep()
                        .iter()
                        .find(|e| !ids.values().any(|id| *id == e.id))
                        .unwrap()
                        .id;
                    ids.insert("child".into(), child);
                }
            }
            "snapshot" => {
                let groups = row["data"]["entities"].as_object().unwrap();
                if tick == 0 {
                    let mut ordered: Vec<_> = groups
                        .iter()
                        .filter_map(|(tag, states)| {
                            states[0].as_object().map(|_| (tag, &states[0]))
                        })
                        .collect();
                    ordered.sort_by_key(|(_, state)| state["entity_numeric_id"].as_i64().unwrap());
                    for (tag, state) in ordered {
                        let color = DYE_NAMES
                            .iter()
                            .position(|name| *name == state["sheep_color"].as_str().unwrap())
                            .unwrap() as u8;
                        let mut wool = Wool::default();
                        wool.set_color(color);
                        let sheep = Sheep {
                            age: Age {
                                ticks: state["age"].as_i64().unwrap() as i32,
                                forced: state["forced_age"].as_i64().unwrap() as i32,
                                forced_particle_ticks: state["forced_age_timer"].as_i64().unwrap()
                                    as i32,
                                locked: state["age_locked"].as_bool().unwrap(),
                                lock_particle_ticks: 0,
                            },
                            in_love: state["in_love"].as_i64().unwrap() as i32,
                            wool,
                            persistence_required: state["persistence_required"].as_bool().unwrap(),
                        };
                        let id = world.spawn_sheep(
                            sheep,
                            DVec3::new(
                                number(&state["x"]),
                                number(&state["y"]),
                                number(&state["z"]),
                            ),
                            state["no_ai"].as_bool().unwrap(),
                        );
                        ids.insert(tag.clone(), id);
                    }
                } else {
                    world
                        .tick_with_players(&mut floor, &player.iter().copied().collect::<Vec<_>>());
                    heard += sounds::check(&mut world, &row["data"], &format!("tick {}", row["tick"]), &[]);
                    frames += 1;
                }
                if let Some(counts) =
                    row["data"]["entity_type_counts"]["minecraft:sheep"].as_object()
                {
                    assert_eq!(
                        counts["total"],
                        world.sheep().len(),
                        "tick {tick} sheep count"
                    );
                    assert_eq!(
                        counts["babies"],
                        world
                            .sheep()
                            .iter()
                            .filter(|entity| entity.sheep.age.baby())
                            .count(),
                        "tick {tick} baby count"
                    );
                }
                for (tag, states) in groups {
                    if states.as_array().unwrap().is_empty() {
                        continue;
                    }
                    let state = &states[0];
                    let entity = world.sheep_mut(ids[tag]).unwrap();
                    assert_eq!(
                        state["age"], entity.sheep.age.ticks,
                        "tick {tick} {tag} age"
                    );
                    assert_eq!(
                        state["in_love"], entity.sheep.in_love,
                        "tick {tick} {tag} love"
                    );
                    assert_eq!(
                        state["sheep_color"],
                        entity.sheep.wool.color(),
                        "tick {tick} {tag} color"
                    );
                    assert_eq!(
                        state["entity_numeric_id"], entity.id,
                        "tick {tick} {tag} id"
                    );
                    assert_eq!(
                        state["running_goals"],
                        serde_json::json!(entity.running_goals()),
                        "tick {tick} {tag} goals"
                    );
                    for (name, actual) in [
                        ("on_ground", entity.body.on_ground),
                        ("alive", entity.health > 0.0),
                        ("no_ai", entity.no_ai),
                        ("persistence_required", entity.sheep.persistence_required),
                        ("age_locked", entity.sheep.age.locked),
                        ("sheared", entity.sheep.wool.sheared()),
                        ("move_control_wanted", entity.move_control.has_wanted()),
                        ("path_reached", entity.navigation.reached),
                    ] {
                        assert_eq!(state[name], actual, "tick {tick} {tag} {name}");
                    }
                    for (name, actual) in [
                        ("forced_age", entity.sheep.age.forced),
                        ("forced_age_timer", entity.sheep.age.forced_particle_ticks),
                        ("entity_tick_count", entity.tick_count),
                        ("ambient_sound_time", entity.ambient_sound_time),
                        ("no_action_time", entity.no_action_time),
                    ] {
                        assert_eq!(state[name], actual, "tick {tick} {tag} {name}");
                    }
                    for (name, actual) in [
                        ("x", entity.body.position.x),
                        ("y", entity.body.position.y),
                        ("z", entity.body.position.z),
                        ("vx", entity.body.velocity.x),
                        ("vy", entity.body.velocity.y),
                        ("vz", entity.body.velocity.z),
                        ("yaw", f64::from(entity.yaw)),
                        ("speed", f64::from(entity.speed)),
                        ("health", f64::from(entity.health)),
                        ("move_control_x", entity.move_control.wanted.x),
                        ("move_control_y", entity.move_control.wanted.y),
                        ("move_control_z", entity.move_control.wanted.z),
                    ] {
                        exact(&state[name], actual, name, tick, tag);
                    }
                    if state.get("head_yaw").is_some() {
                        let eye_height = if entity.sheep.age.baby() {
                            0.6175_f32
                        } else {
                            1.235_f32
                        };
                        for (name, actual) in [
                            ("eye_y", entity.body.position.y + f64::from(eye_height)),
                            ("head_yaw", f64::from(entity.look_control.head_yaw)),
                            ("body_yaw", f64::from(entity.body_rotation.body_yaw)),
                            ("pitch", f64::from(entity.look_control.pitch)),
                            ("look_control_x", entity.look_control.wanted.x),
                            ("look_control_y", entity.look_control.wanted.y),
                            ("look_control_z", entity.look_control.wanted.z),
                        ] {
                            exact(&state[name], actual, name, tick, tag);
                        }
                        assert_eq!(
                            state["look_control_wanted"],
                            entity.look_control.cooldown > 0,
                            "tick {tick} {tag} look wanted"
                        );
                    }
                    if state.get("in_water").is_some() {
                        for (name, actual) in [
                            ("in_water", entity.fluid.in_water()),
                            ("in_lava", entity.fluid.in_lava()),
                            ("jumping", entity.jumping),
                        ] {
                            assert_eq!(state[name], actual, "tick {tick} {tag} {name}");
                        }
                        for (name, actual) in [
                            ("water_height", entity.fluid.water_height),
                            ("lava_height", entity.fluid.lava_height),
                            ("floatable_fluid_height", entity.fluid.floatable_height()),
                        ] {
                            exact(&state[name], actual, name, tick, tag);
                        }
                    }
                    assert_eq!(
                        state["navigation_done"],
                        entity.navigation.is_done(),
                        "tick {tick} {tag} nav done"
                    );
                    assert_eq!(
                        state["path_node_count"],
                        entity.navigation.nodes.len(),
                        "tick {tick} {tag} path count"
                    );
                    assert_eq!(
                        state["path_next_node"],
                        entity.navigation.observed_next(),
                        "tick {tick} {tag} path next"
                    );
                    assert_eq!(
                        state["path_nodes"],
                        serde_json::json!(entity.navigation.nodes),
                        "tick {tick} {tag} path nodes"
                    );
                }
            }
            _ => {}
        }
    }
    assert_eq!(Some(frames), expected_frames);
    println!("{frames} sheep goal frames matched exactly ({heard} sounds)");
}
