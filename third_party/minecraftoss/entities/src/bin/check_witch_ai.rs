//! Exact pinned 26.3 witch gate (`scenarios/mobs/witch-ai.json`): witches
//! in walled stone pens of an empty void world close in on probes, drink
//! swiftness, water breathing and healing, and throw slowness, poison,
//! weakness and harming; the potions fly through the probes (not in the
//! entity lookup) and splash a cow, a pig and a still sheep. Every tick
//! compares each witch's position, motion, rotations, controls, path, goals
//! and target goals, target, drinking, the potion in hand, effects and
//! random; each animal's position, motion, health, effects and random;
//! each potion in flight (position, motion, rotation, age, thrower); and
//! every sound in the pen.
#[path = "support/sounds.rs"]
mod sounds;

use glam::DVec3;
use minecraftoss_entities::{
    cow::Cow,
    effects::{EffectInstance, MobEffect, MobEffects},
    monster_ai::PlayerVitals,
    pig::Pig,
    sheep::Sheep,
    tempt::PlayerCandidate,
    witch::Witch,
    world::{EntityWorld, PlayerAttack},
};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

/// The pens' blocks, lit as at midnight.
#[derive(Default)]
struct Scene {
    blocks: BTreeMap<Pos, Block>,
}
impl World for Scene {
    fn block(&self, pos: Pos) -> Option<Block> {
        self.blocks.get(&pos).cloned()
    }
    fn set_block(&mut self, pos: Pos, block: Option<Block>) {
        match block {
            Some(block) if block.id != "minecraft:air" => {
                self.blocks.insert(pos, block);
            }
            _ => {
                self.blocks.remove(&pos);
            }
        }
    }
    fn step_sound(&self, pos: Pos) -> Option<(String, f32, f32)> {
        sounds::step_sound(&self.blocks.get(&pos)?.id)
    }
    fn can_see_sky(&self, (x, y, z): Pos) -> bool {
        !self.blocks.keys().any(|&(bx, by, bz)| bx == x && bz == z && by >= y)
    }
    fn light_path_cost(&self, _pos: Pos) -> f32 {
        // The midnight sky's level 4 (`LightTexture.getBrightness` less 0.5).
        (4.0_f32 / 15.0) / (4.0 - 3.0 * (4.0_f32 / 15.0)) - 0.5
    }
}

fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn exact(observed: &Value, actual: f64, label: &str) {
    assert_eq!(actual.to_bits(), number(observed).to_bits(), "{label}: {actual} vs {}", number(observed));
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

/// An effect as the codec wrote it (defaults left out), with the hidden
/// effects beneath it.
fn observed_effect(value: &Value) -> EffectInstance {
    let effect = MobEffect::from_id(value["id"].as_str().unwrap()).unwrap_or_else(|| panic!("unknown effect {value}"));
    let visible = value["show_particles"].as_bool().unwrap_or(true);
    EffectInstance {
        effect,
        duration: value["duration"].as_i64().unwrap_or(0) as i32,
        amplifier: value["amplifier"].as_i64().unwrap_or(0) as i32,
        ambient: value["ambient"].as_bool().unwrap_or(false),
        visible,
        show_icon: value["show_icon"].as_bool().unwrap_or(visible),
        hidden: value.get("hidden_effect").map(|hidden| {
            let mut hidden = hidden.clone();
            hidden["id"] = value["id"].clone();
            Box::new(observed_effect(&hidden))
        }),
    }
}

fn check_effects(observed: &Value, ours: &MobEffects, at: &str) {
    let mut vanilla: Vec<EffectInstance> = observed.as_array().unwrap().iter().map(observed_effect).collect();
    let mut ours: Vec<EffectInstance> = ours.iter().cloned().collect();
    vanilla.sort_by_key(|e| e.effect.id());
    ours.sort_by_key(|e| e.effect.id());
    assert_eq!(ours, vanilla, "{at} effects");
}

fn main() {
    let path = env::args().nth(1).expect("usage: check_witch_ai TRACE.jsonl");
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut probes: BTreeMap<String, PlayerCandidate> = BTreeMap::new();
    let mut vitals: Vec<(u64, PlayerVitals)> = Vec::new();
    let mut scenario = String::new();
    let (mut frames, mut heard, mut throws, mut drinks, mut potion_frames) = (0, 0, 0, 0, 0);
    let mut thrown: BTreeMap<String, usize> = BTreeMap::new();
    let mut drunk: BTreeMap<String, usize> = BTreeMap::new();
    let mut splashed: BTreeMap<String, usize> = BTreeMap::new();
    let mut goals_seen = std::collections::BTreeSet::new();
    let mut was_drinking: BTreeMap<String, bool> = BTreeMap::new();
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
                    if parts[0] == "fill" {
                        let n = |i: usize| parts[i].parse::<i32>().unwrap();
                        for x in n(1)..=n(4) {
                            for y in n(2)..=n(5) {
                                for z in n(3)..=n(6) {
                                    scene.set_block((x, y, z), Some(Block::new(parts[7])));
                                }
                            }
                        }
                    }
                }
                world = EntityWorld::default();
                ids.clear();
                probes.clear();
                vitals.clear();
                was_drinking.clear();
            }
            "entity_set_random_seed" => {
                let id = ids[row["data"]["tag"].as_str().unwrap()];
                let seed: u64 = row["data"]["seed"].as_str().unwrap().parse::<i64>().unwrap() as u64;
                if let Some(e) = world.witch_mut(id) {
                    e.set_random_seed(seed);
                } else if let Some(e) = world.cow_mut(id) {
                    e.set_random_seed(seed as i64);
                } else if let Some(e) = world.pig_mut(id) {
                    e.set_random_seed(seed as i64);
                } else if let Some(e) = world.sheep_mut(id) {
                    e.set_random_seed(seed as i64);
                } else {
                    panic!("no mob to seed");
                }
            }
            "projectile_shoot_seed" => {
                world.set_arrow_shoot_seed(Some(row["data"]["seed"].as_str().unwrap().parse::<i64>().unwrap() as u64));
            }
            "player_probe" => {
                let data = &row["data"];
                let tag = data["tag"].as_str().unwrap().to_owned();
                let pos = &data["pos"];
                let position = DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap());
                let id = 1_000_000 + probes.len() as u64;
                probes.insert(tag, probe(id, position));
                let effects: Vec<&str> = data["effects"].as_array().map(|all| all.iter().map(|e| e["id"].as_str().unwrap()).collect()).unwrap_or_default();
                vitals.push((
                    id,
                    PlayerVitals {
                        health: data["health"].as_f64().unwrap_or(20.0) as f32,
                        velocity: DVec3::ZERO,
                        slowed: effects.contains(&"minecraft:slowness"),
                        poisoned: effects.contains(&"minecraft:poison"),
                        weakened: effects.contains(&"minecraft:weakness"),
                    },
                ));
                world.set_player_vitals(vitals.clone());
            }
            "player_attack" => {
                // A full-strength bare-handed hit on the ground.
                let data = &row["data"];
                let candidate = probes[data["probe"].as_str().unwrap()];
                let attack = PlayerAttack {
                    player_id: candidate.id,
                    position: candidate.position,
                    yaw: 0.0,
                    attack_damage: 1.0,
                    strength: 1.0,
                    sprinting: false,
                    can_critical: false,
                    can_sweep: false,
                };
                let target = ids[data["target"].as_str().unwrap()];
                let result = world.player_attack(&attack, target);
                assert!(result.hurt, "{scenario} attack lands");
            }
            "snapshot" if scenario == "witch_warmup" => {}
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = data["entities"].as_object().unwrap();
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    let mut order: Vec<(&String, &Value)> = observed.iter().map(|(tag, states)| (tag, &states[0])).collect();
                    order.sort_by_key(|(_, e)| e["entity_numeric_id"].as_u64().unwrap());
                    for (tag, e) in order {
                        let position = DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"]));
                        let yaw = number(&e["yaw"]) as f32;
                        let on_ground = e["on_ground"].as_bool().unwrap();
                        let no_ai = e["no_ai"].as_bool().unwrap();
                        world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
                        let id = match e["type"].as_str().unwrap() {
                            "minecraft:witch" => {
                                let mut witch = Witch::new(position);
                                witch.body.on_ground = on_ground;
                                witch.persistence_required = true;
                                witch.yaw = yaw;
                                world.spawn_witch(witch, no_ai)
                            }
                            "minecraft:cow" => {
                                let mut cow = Cow::new(position);
                                cow.body.on_ground = on_ground;
                                cow.yaw = yaw;
                                cow.persistence_required = true;
                                world.spawn_cow(cow, no_ai)
                            }
                            "minecraft:pig" => {
                                let mut pig = Pig::new(position);
                                pig.body.on_ground = on_ground;
                                pig.yaw = yaw;
                                pig.persistence_required = true;
                                world.spawn_pig(pig, no_ai)
                            }
                            "minecraft:sheep" => {
                                let sheep = Sheep { persistence_required: true, ..Sheep::default() };
                                let id = world.spawn_sheep(sheep, position, no_ai);
                                let entity = world.sheep_mut(id).unwrap();
                                entity.body.on_ground = on_ground;
                                entity.yaw = yaw;
                                id
                            }
                            other => panic!("unexpected {other}"),
                        };
                        assert_eq!(id, e["entity_numeric_id"].as_u64().unwrap(), "{scenario} id");
                        ids.insert(tag.clone(), id);
                    }
                    let _ = world.take_sounds();
                    continue;
                }
                let players: Vec<PlayerCandidate> = probes.values().copied().collect();
                world.tick_with_players(&mut scene, &players);
                assert_eq!(world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                let (compared, ours) = sounds::check_heard(&mut world, data, &format!("{scenario} tick {tick}"), &sounds::pens(&suite, &scenario));
                heard += compared;
                throws += ours.iter().filter(|s| s.0 == "minecraft:entity.witch.throw").count();
                drinks += ours.iter().filter(|s| s.0 == "minecraft:entity.witch.drink").count();
                for (tag, states) in observed {
                    let id = ids[tag];
                    let states = states.as_array().unwrap();
                    let at = format!("{scenario} tick {tick} {tag}");
                    if let Some(entity) = world.witches().iter().find(|e| e.id == id) {
                        assert_eq!(states.len(), 1, "{at} presence");
                        let e = &states[0];
                        let ai = &entity.ai;
                        let (body, state) = (&entity.witch.body, &ai.state);
                        for (field, actual) in [
                            ("x", body.position.x),
                            ("y", body.position.y),
                            ("z", body.position.z),
                            ("vx", body.velocity.x),
                            ("vy", body.velocity.y),
                            ("vz", body.velocity.z),
                            ("health", f64::from(entity.witch.health)),
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
                            ("fall_distance", body.fall_distance),
                        ] {
                            exact(&e[field], actual, &format!("{at} {field}"));
                        }
                        assert_eq!(e["on_ground"], body.on_ground, "{at} on ground");
                        assert_eq!(e["air_supply"], body.air, "{at} air");
                        assert_eq!(e["entity_tick_count"], entity.tick_count, "{at} tick count");
                        assert_eq!(e["ambient_sound_time"], entity.ambient_sound_time, "{at} ambient time");
                        assert_eq!(e["no_action_time"], entity.no_action_time, "{at} idle time");
                        let goals: Vec<&str> = e["running_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                        assert_eq!(goals, ai.running_goals(), "{at} goals");
                        let targets: Vec<&str> = e["running_target_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                        assert_eq!(targets, ai.running_targets(), "{at} target goals");
                        assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random");
                        goals_seen.extend(ai.running_goals());
                        goals_seen.extend(ai.running_targets());
                        assert_eq!(e.get("target_uuid").is_some_and(|t| !t.is_null()), state.target().is_some(), "{at} target");
                        assert_eq!(e["aggressive"], state.melee.aggressive, "{at} aggressive");
                        assert_eq!(e["move_control_wanted"], ai.move_control.has_wanted(), "{at} move wanted");
                        assert_eq!(e["look_control_wanted"], state.look_control.cooldown > 0, "{at} look wanted");
                        assert_eq!(e["navigation_done"], state.navigation.is_done(), "{at} navigation done");
                        let nodes: Vec<(i32, i32, i32)> = e["path_nodes"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|n| (n[0].as_i64().unwrap() as i32, n[1].as_i64().unwrap() as i32, n[2].as_i64().unwrap() as i32))
                            .collect();
                        assert_eq!(nodes, state.navigation.nodes, "{at} path");
                        let witch = &entity.witch;
                        assert_eq!(e["witch_drinking"], witch.drinking.is_some(), "{at} drinking");
                        assert_eq!(e["witch_using_time"], witch.using_time, "{at} using time");
                        assert_eq!(e["witch_heal_cooldown"], witch.heal_cooldown, "{at} heal cooldown");
                        assert_eq!(e["main_hand_potion"].as_str(), witch.drinking.map(|p| p.id()), "{at} potion in hand");
                        check_effects(&e["effects"], &witch.effects, &at);
                        if let Some(potion) = witch.drinking {
                            if !was_drinking.get(tag).copied().unwrap_or(false) {
                                *drunk.entry(potion.id().to_owned()).or_default() += 1;
                            }
                        }
                        was_drinking.insert(tag.clone(), witch.drinking.is_some());
                        continue;
                    }
                    // An animal: gone once removed.
                    let (body, health, effects, random) = if let Some(e) = world.cows().iter().find(|e| e.id == id) {
                        (&e.cow.body, e.cow.health, &e.effects, e.random.raw_state())
                    } else if let Some(e) = world.pigs().iter().find(|e| e.id == id) {
                        (&e.pig.body, e.pig.health, &e.effects, e.random.raw_state())
                    } else if let Some(e) = world.sheep().iter().find(|e| e.id == id) {
                        (&e.body, e.health, &e.effects, e.random.raw_state())
                    } else {
                        assert!(states.is_empty(), "{at} presence");
                        continue;
                    };
                    assert_eq!(states.len(), 1, "{at} presence");
                    let e = &states[0];
                    for (field, actual) in [
                        ("x", body.position.x),
                        ("y", body.position.y),
                        ("z", body.position.z),
                        ("vx", body.velocity.x),
                        ("vy", body.velocity.y),
                        ("vz", body.velocity.z),
                        ("health", f64::from(health)),
                    ] {
                        exact(&e[field], actual, &format!("{at} {field}"));
                    }
                    assert_eq!(e["on_ground"], body.on_ground, "{at} on ground");
                    if let Some(state) = e.get("random_state") {
                        assert_eq!(state.as_str().unwrap().parse::<u64>().unwrap(), random, "{at} random");
                    }
                    check_effects(&e["effects"], effects, &at);
                    for effect in effects.iter() {
                        *splashed.entry(format!("{tag} {}", effect.effect.id())).or_default() += 1;
                    }
                }
                // The potions in flight, in order.
                let vanilla: Vec<&Value> = data["entity_type_states"]["minecraft:splash_potion"].as_array().unwrap().iter().collect();
                let ours = world.potions();
                assert_eq!(ours.len(), vanilla.len(), "{scenario} tick {tick} potions");
                for (p, v) in ours.iter().zip(vanilla) {
                    let at = format!("{scenario} tick {tick} potion {}", v["entity_numeric_id"]);
                    let potion = &p.potion;
                    for (field, actual) in [
                        ("x", potion.position.x),
                        ("y", potion.position.y),
                        ("z", potion.position.z),
                        ("vx", potion.velocity.x),
                        ("vy", potion.velocity.y),
                        ("vz", potion.velocity.z),
                        ("yaw", f64::from(potion.yaw)),
                        ("pitch", f64::from(potion.pitch)),
                    ] {
                        exact(&v[field], actual, &format!("{at} {field}"));
                    }
                    assert_eq!(v["potion"].as_str(), Some(potion.potion.id()), "{at} kind");
                    assert_eq!(v["entity_tick_count"], potion.tick_count, "{at} age");
                    assert_eq!(v["left_owner"], potion.left_owner, "{at} left owner");
                    assert_eq!(v["owner_numeric_id"].as_u64(), potion.owner, "{at} owner");
                    if potion.tick_count == 0 {
                        *thrown.entry(potion.potion.id().to_owned()).or_default() += 1;
                    }
                    potion_frames += 1;
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert!(throws > 0 && drinks > 0, "witches throw and drink");
    println!("{frames} exact witch frames matched ({heard} sounds, {throws} throws {thrown:?}, {drinks} drinks {drunk:?}, {potion_frames} potion frames; effect-ticks {splashed:?}; goals seen: {goals_seen:?})");
}
