//! Exact pinned 26.3 gate for the world's hazards on mobs
//! (`scenarios/mobs/hazards.json`): lava, fire (with a pool to run to),
//! cactus, magma, a lit campfire, suffocation, water putting fire out,
//! sweet berry bushes, cobwebs, and a witch drinking fire resistance in
//! lava. Each mob runs its full AI (or none, as summoned); every tick
//! compares position, motion, health, fire ticks, air, fall distance, the
//! mob random, its tick and ambient clocks, whether it is still there, the
//! witch's drinking and effects, and every sound in the pens.
#[path = "support/sounds.rs"]
mod sounds;

use glam::DVec3;
use minecraftoss_entities::{
    age::Age,
    cow::{Cow, CowSoundVariant},
    effects::MobEffect,
    movement::Body,
    pig::{Pig, PigSoundVariant},
    sheep::{Sheep, Wool},
    tempt::PlayerCandidate,
    witch::Witch,
    world::EntityWorld,
    zombie::Zombie,
};
use minecraftoss_player::{rng::LegacyRandom, Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

#[derive(Default)]
struct Scene(BTreeMap<Pos, Block>);
impl World for Scene {
    fn block(&self, pos: Pos) -> Option<Block> {
        self.0.get(&pos).cloned()
    }
    fn set_block(&mut self, pos: Pos, block: Option<Block>) {
        match block {
            Some(block) if block.id != "minecraft:air" => {
                self.0.insert(pos, block);
            }
            _ => {
                self.0.remove(&pos);
            }
        }
    }
    fn step_sound(&self, pos: Pos) -> Option<(String, f32, f32)> {
        sounds::step_sound(&self.0.get(&pos)?.id)
    }
    fn light_path_cost(&self, _pos: Pos) -> f32 {
        // Midnight: `LightTexture.getBrightness` at level 4.
        (4.0_f32 / 15.0) / (4.0 - 3.0 * (4.0_f32 / 15.0)) - 0.5
    }
}

/// A block from a command's state (`id[key=value,...]`).
fn parse_block(spec: &str) -> Block {
    let Some((id, rest)) = spec.split_once('[') else { return Block::new(spec) };
    let mut block = Block::new(id);
    for pair in rest.trim_end_matches(']').split(',').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap();
        block = block.with(key, value);
    }
    block
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

/// What the gate compares of any mob.
struct Seen<'a> {
    body: &'a Body,
    health: f32,
    random: &'a LegacyRandom,
    tick_count: i32,
    ambient: i32,
}

fn seen(world: &EntityWorld, id: u64) -> Option<Seen<'_>> {
    macro_rules! find {
        ($list:expr, $mob:ident) => {
            if let Some(e) = $list.iter().find(|e| e.id == id) {
                return Some(Seen { body: &e.$mob.body, health: e.$mob.health, random: &e.random, tick_count: e.tick_count, ambient: e.ambient_sound_time });
            }
        };
    }
    find!(world.cows(), cow);
    find!(world.pigs(), pig);
    find!(world.zombies(), zombie);
    find!(world.witches(), witch);
    world
        .sheep()
        .iter()
        .find(|e| e.id == id)
        .map(|e| Seen { body: &e.body, health: e.health, random: &e.random, tick_count: e.tick_count, ambient: e.ambient_sound_time })
}

/// Spawns an observed mob of its type, with or without AI, as the harness
/// found it (burning ones with their fire).
/// The yaw a scenario summons the tagged mob with (`Rotation:[yaw f, ..]`;
/// the mob state records no rotation).
fn summoned_yaw(suite: &Value, scenario: &str, tag: &str) -> f32 {
    let definition = suite["scenarios"].as_array().unwrap().iter().find(|c| c["id"] == scenario).unwrap();
    let marker = format!("Tags:[\"{tag}\"]");
    definition["setup"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .find(|c| c.contains(&marker))
        .and_then(|c| c.split("Rotation:[").nth(1))
        .and_then(|r| r.split('f').next())
        .map_or(0.0, |yaw| yaw.parse().unwrap())
}

fn spawn(world: &mut EntityWorld, e: &Value, yaw: f32) -> u64 {
    let position = DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"]));
    let velocity = DVec3::new(number(&e["vx"]), number(&e["vy"]), number(&e["vz"]));
    let on_ground = e["on_ground"].as_bool().unwrap();
    let no_ai = e["no_ai"].as_bool().unwrap();
    let fire = e["remaining_fire_ticks"].as_i64().unwrap() as i32;
    let age = || e["age"].as_i64().unwrap_or(0) as i32;
    world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
    let id = match e["type"].as_str().unwrap() {
        "minecraft:cow" => {
            let mut cow = Cow::new(position);
            cow.body.velocity = velocity;
            cow.yaw = yaw;
            cow.age.ticks = age();
            cow.persistence_required = true;
            cow.sound_variant = CowSoundVariant::Classic;
            world.spawn_cow(cow, no_ai)
        }
        "minecraft:pig" => {
            let mut pig = Pig::new(position);
            pig.body.velocity = velocity;
            pig.yaw = yaw;
            pig.age.ticks = age();
            pig.persistence_required = true;
            pig.sound_variant = PigSoundVariant::Classic;
            world.spawn_pig(pig, no_ai)
        }
        "minecraft:sheep" => {
            let sheep = Sheep { age: Age { ticks: age(), ..Age::default() }, in_love: 0, wool: Wool::default(), persistence_required: true };
            let id = world.spawn_sheep(sheep, position, no_ai);
            let entity = world.sheep_mut(id).unwrap();
            entity.body.velocity = velocity;
            entity.yaw = yaw;
            entity.previous_yaw = yaw;
            id
        }
        "minecraft:zombie" => {
            let mut zombie = Zombie::new(position);
            zombie.persistence_required = true;
            if no_ai {
                world.spawn_zombie(zombie, true)
            } else {
                world.spawn_zombie_active(zombie, yaw)
            }
        }
        "minecraft:witch" => {
            let mut witch = Witch::new(position);
            witch.persistence_required = true;
            witch.yaw = yaw;
            world.spawn_witch(witch, no_ai)
        }
        other => panic!("unsupported mob {other}"),
    };
    let body = world.body_mut(id).unwrap();
    body.on_ground = on_ground;
    body.fire_ticks = fire;
    id
}

fn set_seed(world: &mut EntityWorld, id: u64, seed: u64) {
    if let Some(e) = world.cow_mut(id) {
        e.random = LegacyRandom::new(seed);
    } else if let Some(e) = world.pig_mut(id) {
        e.random = LegacyRandom::new(seed);
    } else if let Some(e) = world.sheep_mut(id) {
        e.random = LegacyRandom::new(seed);
    } else if let Some(e) = world.zombie_mut(id) {
        e.random = LegacyRandom::new(seed);
    } else if let Some(e) = world.witch_mut(id) {
        e.random = LegacyRandom::new(seed);
    } else {
        panic!("no mob {id} to seed");
    }
}

fn main() {
    let path = env::args().nth(1).expect("usage: check_hazards TRACE.jsonl");
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut probes: Vec<PlayerCandidate> = Vec::new();
    let mut scenario = String::new();
    let (mut frames, mut heard, mut burning, mut deaths, mut breathless) = (0, 0, 0, 0, 0);
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
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
                    let n = |i: usize| parts[i].parse::<i32>().unwrap();
                    match parts[0] {
                        "fill" => {
                            let block = parse_block(parts[7]);
                            for x in n(1)..=n(4) {
                                for y in n(2)..=n(5) {
                                    for z in n(3)..=n(6) {
                                        scene.set_block((x, y, z), Some(block.clone()));
                                    }
                                }
                            }
                        }
                        "setblock" => scene.set_block((n(1), n(2), n(3)), Some(parse_block(parts[4]))),
                        _ => {}
                    }
                }
                world = EntityWorld::default();
                ids.clear();
                probes.clear();
            }
            "entity_set_random_seed" => {
                let id = ids[row["data"]["tag"].as_str().unwrap()];
                set_seed(&mut world, id, row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "player_probe" => {
                let pos = &row["data"]["pos"];
                let position = DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap());
                probes.push(probe(1_000_000 + probes.len() as u64, position));
            }
            "snapshot" if scenario == "hazard_warmup" => {}
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = data["entities"].as_object().unwrap();
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    let mut order: Vec<(&String, &Value)> = observed.iter().map(|(tag, states)| (tag, &states[0])).collect();
                    order.sort_by_key(|(_, e)| e["entity_numeric_id"].as_u64().unwrap());
                    for (tag, e) in order {
                        let id = spawn(&mut world, e, summoned_yaw(&suite, &scenario, tag));
                        ids.insert(tag.clone(), id);
                    }
                    let _ = world.take_sounds();
                    continue;
                }
                world.tick_with_players(&mut scene, &probes);
                assert_eq!(world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                let (compared, ours) = sounds::check_heard(&mut world, data, &format!("{scenario} tick {tick}"), &sounds::pens(&suite, &scenario));
                heard += compared;
                deaths += ours.iter().filter(|s| s.0.ends_with(".death")).count();
                for s in &ours {
                    if s.0.starts_with("minecraft:entity.generic.") {
                        *kinds.entry(s.0.clone()).or_default() += 1;
                    }
                }
                for (tag, states) in observed {
                    let id = ids[tag];
                    let states = states.as_array().unwrap();
                    let ours = seen(&world, id);
                    assert_eq!(states.len(), usize::from(ours.is_some()), "{scenario} tick {tick} {tag} presence");
                    let Some(mob) = ours else { continue };
                    let e = &states[0];
                    let at = format!("{scenario} tick {tick} {tag}");
                    for (field, actual) in [
                        ("x", mob.body.position.x),
                        ("y", mob.body.position.y),
                        ("z", mob.body.position.z),
                        ("vx", mob.body.velocity.x),
                        ("vy", mob.body.velocity.y),
                        ("vz", mob.body.velocity.z),
                        ("health", f64::from(mob.health)),
                        ("fall_distance", mob.body.fall_distance),
                    ] {
                        exact(&e[field], actual, &format!("{at} {field}"));
                    }
                    assert_eq!(e["on_ground"], mob.body.on_ground, "{at} on ground");
                    assert_eq!(e["remaining_fire_ticks"].as_i64().unwrap(), i64::from(mob.body.fire_ticks), "{at} fire ticks");
                    assert_eq!(e["air_supply"].as_i64().unwrap(), i64::from(mob.body.air), "{at} air");
                    assert_eq!(e["entity_tick_count"], mob.tick_count, "{at} tick count");
                    assert_eq!(e["ambient_sound_time"], mob.ambient, "{at} ambient time");
                    assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), mob.random.raw_state(), "{at} random");
                    burning += usize::from(mob.body.fire_ticks > 0);
                    breathless += usize::from(mob.body.air < 300);
                    if let Some(witch) = world.witches().iter().find(|w| w.id == id) {
                        assert_eq!(e["witch_drinking"], witch.witch.drinking.is_some(), "{at} drinking");
                        let effects: Vec<&str> = e["effects"].as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap()).collect();
                        let ours: Vec<&str> = witch.witch.effects.iter().map(|x| x.effect.id()).collect();
                        assert_eq!(ours, effects, "{at} effects");
                        if witch.witch.effects.has(MobEffect::FireResistance) {
                            *kinds.entry("fire resistance ticks".to_owned()).or_default() += 1;
                        }
                    }
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert!(burning > 0 && deaths > 0, "mobs burn and die");
    println!("{frames} exact hazard frames matched ({heard} sounds, {deaths} deaths, {burning} burning mob-ticks, {breathless} breathless mob-ticks; {kinds:?})");
}
