//! Exact pinned 26.3 skeleton variant gate
//! (`scenarios/mobs/skeleton-variants.json`): strays, bogged and parched
//! beside the skeleton. NoAI ones stand at noon (all but the parched burn;
//! the bogged and parched have 16 health), then a stray, a bogged and a
//! parched with bows each hunt a probe player on hard (the stray drawing
//! every 20 ticks, the bogged and parched every 50). Every tick compares
//! the hunters' position, motion, rotations, controls, path, goals, target,
//! the bow's draw and the mob random, and every arrow's flight, base damage
//! and effect (slowness 600, poison 100, weakness 600). Probe players stay
//! out of the entity lookup, so the arrows fly past them.
use glam::DVec3;
use minecraftoss_entities::{
    skeleton::{Skeleton, SkeletonKind},
    tempt::PlayerCandidate,
    world::EntityWorld,
};
use minecraftoss_player::survival::EffectKind;
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

/// The scene's blocks, lit as at midnight (uniformly dim) or noon (full
/// sky light under open sky, none beneath blocks).
#[derive(Default)]
struct Scene {
    blocks: BTreeMap<Pos, Block>,
    noon: bool,
}
impl World for Scene {
    fn block(&self, pos: Pos) -> Option<Block> {
        self.blocks.get(&pos).cloned()
    }
    fn set_block(&mut self, pos: Pos, block: Option<Block>) {
        if let Some(block) = block {
            self.blocks.insert(pos, block);
        } else {
            self.blocks.remove(&pos);
        }
    }
    fn can_see_sky(&self, (x, y, z): Pos) -> bool {
        !self.blocks.keys().any(|&(bx, by, bz)| bx == x && bz == z && by > y)
    }
    fn light_path_cost(&self, pos: Pos) -> f32 {
        match (self.noon, self.can_see_sky(pos)) {
            (true, true) => 0.5,
            (true, false) => -0.5,
            (false, _) => 0.0,
        }
    }
}

fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn exact(observed: &Value, actual: f64, label: &str) {
    assert_eq!(actual.to_bits(), number(observed).to_bits(), "{label}: {actual} vs {}", number(observed));
}

fn kind_of(type_id: &str) -> SkeletonKind {
    match type_id {
        "minecraft:skeleton" => SkeletonKind::Skeleton,
        "minecraft:stray" => SkeletonKind::Stray,
        "minecraft:bogged" => SkeletonKind::Bogged,
        "minecraft:parched" => SkeletonKind::Parched,
        other => panic!("unexpected {other}"),
    }
}

fn effect_id(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::Slowness => "minecraft:slowness",
        EffectKind::Poison => "minecraft:poison",
        EffectKind::Weakness => "minecraft:weakness",
        other => panic!("unexpected arrow effect {other:?}"),
    }
}

fn probe(id: u64, position: DVec3) -> PlayerCandidate {
    PlayerCandidate {
        id,
        position,
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
    }
}

fn main() {
    let path = env::args().nth(1).expect("usage: check_skeleton_variants TRACE.jsonl");
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut probes: Vec<PlayerCandidate> = Vec::new();
    let mut scenario = String::new();
    let mut difficulty = 2;
    let (mut frames, mut shots) = (0, 0);
    let mut burning: BTreeMap<String, usize> = BTreeMap::new();
    let mut effects_seen: BTreeMap<String, String> = BTreeMap::new();
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => suite = row["data"]["suite"].clone(),
            "complete" => complete = true,
            "scenario_start" => {
                scenario = row["scenario"].as_str().unwrap().to_owned();
                let definition = suite["scenarios"].as_array().unwrap().iter().find(|c| c["id"] == scenario.as_str()).unwrap();
                for command in definition["prepare"].as_array().unwrap() {
                    let parts: Vec<&str> = command.as_str().unwrap().split_whitespace().collect();
                    match parts[0] {
                        "fill" => {
                            let n = |i: usize| parts[i].parse::<i32>().unwrap();
                            for x in n(1)..=n(4) {
                                for y in n(2)..=n(5) {
                                    for z in n(3)..=n(6) {
                                        scene.set_block((x, y, z), Some(Block::new(parts[7])));
                                    }
                                }
                            }
                        }
                        "time" => scene.noon = parts[2] == "noon",
                        "difficulty" => difficulty = ["peaceful", "easy", "normal", "hard"].iter().position(|d| *d == parts[1]).unwrap() as i32,
                        _ => {}
                    }
                }
                world = EntityWorld::default();
                world.set_difficulty(difficulty);
                world.set_bright_outside(scene.noon);
                world.set_monsters_burn(scene.noon);
                ids.clear();
                probes.clear();
            }
            "entity_set_random_seed" => {
                let id = ids[row["data"]["tag"].as_str().unwrap()];
                world.skeleton_mut(id).unwrap().set_random_seed(row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "projectile_shoot_seed" => world.set_arrow_shoot_seed(Some(row["data"]["seed"].as_str().unwrap().parse().unwrap())),
            "projectile_damage_seed" => world.set_arrow_damage_seed(Some(row["data"]["seed"].as_str().unwrap().parse().unwrap())),
            "player_probe" => {
                let pos = &row["data"]["pos"];
                probes.push(probe(1_000_000, DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap())));
            }
            "snapshot" if scenario == "variants_warmup" => {}
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = data["entities"].as_object().unwrap();
                let hunt = scenario.ends_with("_hunt");
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    let mut order: Vec<(&String, &Value)> = observed.iter().map(|(tag, states)| (tag, &states[0])).collect();
                    order.sort_by_key(|(_, e)| e["entity_numeric_id"].as_u64().unwrap());
                    for (tag, e) in order {
                        let mut skeleton = Skeleton::of_kind(kind_of(e["type"].as_str().unwrap()), DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"])));
                        skeleton.body.on_ground = e["on_ground"].as_bool().unwrap();
                        skeleton.persistence_required = true;
                        world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
                        let id = if hunt { world.spawn_skeleton_active(skeleton, number(&e["yaw"]) as f32) } else { world.spawn_skeleton(skeleton, true) };
                        ids.insert(tag.clone(), id);
                    }
                    continue;
                }
                // Only the forceloaded row of chunks ticks entities (an
                // arrow that flies out stops).
                world.tick_with_players_where(&mut scene, &probes, &|p| (p.z.floor() as i32) >> 4 == 0 && (0..=12).contains(&((p.x.floor() as i32) >> 4)));
                assert_eq!(world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                shots += world.take_sounds().iter().filter(|s| s.event == "entity.skeleton.shoot").count();
                for (tag, states) in observed {
                    let entity = world.skeletons().iter().find(|s| s.id == ids[tag]).unwrap();
                    let e = &states[0];
                    let at = format!("{scenario} tick {tick} {tag}");
                    assert_eq!(e["type"], entity.skeleton.kind.type_id(), "{at} type");
                    exact(&e["health"], f64::from(entity.skeleton.health), &format!("{at} health"));
                    assert_eq!(e["entity_tick_count"], entity.tick_count, "{at} tick count");
                    assert_eq!(e["ambient_sound_time"], entity.ambient_sound_time, "{at} ambient time");
                    assert_eq!(e["no_action_time"], entity.no_action_time, "{at} idle time");
                    assert_eq!(e["remaining_fire_ticks"], entity.skeleton.body.fire_ticks, "{at} fire");
                    assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random");
                    if entity.skeleton.body.fire_ticks > 0 {
                        *burning.entry(tag.clone()).or_default() += 1;
                    }
                    if !hunt {
                        continue;
                    }
                    let ai = entity.ai.as_deref().unwrap();
                    let (body, state) = (&entity.skeleton.body, &ai.state);
                    for (field, actual) in [
                        ("x", body.position.x),
                        ("y", body.position.y),
                        ("z", body.position.z),
                        ("vx", body.velocity.x),
                        ("vy", body.velocity.y),
                        ("vz", body.velocity.z),
                        ("yaw", f64::from(ai.yaw)),
                        ("speed", f64::from(ai.speed)),
                        ("head_yaw", f64::from(state.look_control.head_yaw)),
                        ("body_yaw", f64::from(ai.body_rotation.body_yaw)),
                        ("pitch", f64::from(state.look_control.pitch)),
                        ("move_control_x", ai.move_control.wanted.x),
                        ("move_control_y", ai.move_control.wanted.y),
                        ("move_control_z", ai.move_control.wanted.z),
                        ("look_control_x", state.look_control.wanted.x),
                        ("look_control_y", state.look_control.wanted.y),
                        ("look_control_z", state.look_control.wanted.z),
                    ] {
                        exact(&e[field], actual, &format!("{at} {field}"));
                    }
                    assert_eq!(e["on_ground"], body.on_ground, "{at} on ground");
                    let goals: Vec<&str> = e["running_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                    assert_eq!(goals, ai.running_goals(), "{at} goals");
                    assert_eq!(e.get("target_uuid").is_some_and(|t| !t.is_null()), state.target().is_some(), "{at} target");
                    assert_eq!(e["aggressive"], state.melee.aggressive, "{at} aggressive");
                    assert_eq!(e["using_item"], state.bow.using_item, "{at} using bow");
                    assert_eq!(e["ticks_using_item"], if state.bow.using_item { state.bow.ticks_using_item } else { 0 }, "{at} draw");
                    assert_eq!(e["navigation_done"], state.navigation.is_done(), "{at} navigation done");
                    let nodes: Vec<(i32, i32, i32)> = e["path_nodes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|n| (n[0].as_i64().unwrap() as i32, n[1].as_i64().unwrap() as i32, n[2].as_i64().unwrap() as i32))
                        .collect();
                    assert_eq!(nodes, state.navigation.nodes, "{at} path");
                }
                if hunt {
                    // Every arrow in the level, oldest first.
                    let vanilla: Vec<&Value> = data["entity_type_states"]["minecraft:arrow"].as_array().unwrap().iter().collect();
                    let ours: Vec<_> = world.arrows().iter().filter(|a| a.arrow.alive).collect();
                    assert_eq!(vanilla.len(), ours.len(), "{scenario} tick {tick} arrows");
                    for (i, (v, ours)) in vanilla.iter().zip(&ours).enumerate() {
                        let at = format!("{scenario} tick {tick} arrow {i}");
                        let arrow = &ours.arrow;
                        for (field, actual) in [("x", arrow.position.x), ("y", arrow.position.y), ("z", arrow.position.z), ("vx", arrow.velocity.x), ("vy", arrow.velocity.y), ("vz", arrow.velocity.z)] {
                            exact(&v[field], actual, &format!("{at} {field}"));
                        }
                        exact(&v["arrow_base_damage"], arrow.base_damage, &format!("{at} base damage"));
                        assert_eq!(v["arrow_in_ground"], arrow.in_ground, "{at} in ground");
                        assert_eq!(v["arrow_life"], arrow.life, "{at} life");
                        let effects: Vec<(String, u32, u8)> = v["arrow_effects"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|e| (e["id"].as_str().unwrap().to_owned(), e["duration"].as_u64().unwrap() as u32, e["amplifier"].as_u64().unwrap() as u8))
                            .collect();
                        let expected: Vec<(String, u32, u8)> = ours.effect.map(|(kind, duration)| (effect_id(kind).to_owned(), duration, 0)).into_iter().collect();
                        assert_eq!(effects, expected, "{at} effects");
                        if let Some((id, duration, _)) = effects.first() {
                            effects_seen.insert(scenario.clone(), format!("{id} {duration}"));
                        }
                    }
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert!(!burning.contains_key("parched") && ["skeleton", "stray", "bogged"].iter().all(|k| burning.contains_key(*k)), "all but the parched burn: {burning:?}");
    assert_eq!(effects_seen.len(), 3, "each hunter's arrows carry its effect: {effects_seen:?}");
    println!("{frames} exact skeleton variant frames matched ({shots} shots; burning mob-ticks {burning:?}; arrow effects {effects_seen:?})");
}
