//! Exact pinned 26.3 gate for falls and drowning
//! (`scenarios/mobs/falls-and-drowning.json`): cows, a pig, a sheep, a
//! chicken, a zombie, a skeleton, a creeper, a spider and a slime drop down
//! walled shafts (one cow onto hay), and a pig, creeper, spider, zombie,
//! cow and chicken are sealed in water. Each mob runs its full AI; every
//! tick compares position, motion, health, air, fall distance, the mob
//! random, its tick and ambient clocks, whether it is still there, and
//! every sound in the pens.
#[path = "support/sounds.rs"]
mod sounds;

use glam::DVec3;
use minecraftoss_entities::{
    age::Age,
    chicken::{Chicken, ChickenSoundVariant},
    cow::{Cow, CowSoundVariant},
    creeper::Creeper,
    movement::Body,
    pig::{Pig, PigSoundVariant},
    sheep::{Sheep, Wool},
    skeleton::{Skeleton, SkeletonKind},
    slime::Slime,
    spider::Spider,
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

fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn exact(observed: &Value, actual: f64, label: &str) {
    assert_eq!(actual.to_bits(), number(observed).to_bits(), "{label}: {actual} vs {}", number(observed));
}

/// What the gate compares of any mob: its body, health, random, and tick
/// and ambient clocks.
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
    find!(world.chickens(), chicken);
    find!(world.zombies(), zombie);
    find!(world.skeletons(), skeleton);
    find!(world.creepers(), creeper);
    find!(world.spiders(), spider);
    find!(world.slimes(), slime);
    world
        .sheep()
        .iter()
        .find(|e| e.id == id)
        .map(|e| Seen { body: &e.body, health: e.health, random: &e.random, tick_count: e.tick_count, ambient: e.ambient_sound_time })
}

/// Spawns an observed mob of its type with its AI, as the harness found it.
fn spawn(world: &mut EntityWorld, e: &Value) -> u64 {
    let position = DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"]));
    let velocity = DVec3::new(number(&e["vx"]), number(&e["vy"]), number(&e["vz"]));
    let on_ground = e["on_ground"].as_bool().unwrap();
    // Summoned facing 0 (the look observation is not recorded).
    let yaw = e.get("yaw").map_or(0.0, number) as f32;
    let age = || e["age"].as_i64().unwrap_or(0) as i32;
    world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
    match e["type"].as_str().unwrap() {
        "minecraft:cow" => {
            let mut cow = Cow::new(position);
            cow.body.velocity = velocity;
            cow.body.on_ground = on_ground;
            cow.yaw = yaw;
            cow.age.ticks = age();
            cow.persistence_required = true;
            cow.sound_variant = CowSoundVariant::Classic;
            world.spawn_cow(cow, false)
        }
        "minecraft:pig" => {
            let mut pig = Pig::new(position);
            pig.body.velocity = velocity;
            pig.body.on_ground = on_ground;
            pig.yaw = yaw;
            pig.age.ticks = age();
            pig.persistence_required = true;
            pig.sound_variant = PigSoundVariant::Classic;
            world.spawn_pig(pig, false)
        }
        "minecraft:chicken" => {
            let mut chicken = Chicken::new(position);
            chicken.body.velocity = velocity;
            chicken.body.on_ground = on_ground;
            chicken.yaw = yaw;
            chicken.age.ticks = age();
            chicken.persistence_required = true;
            chicken.sound_variant = ChickenSoundVariant::Classic;
            chicken.egg_time = e["egg_time"].as_i64().unwrap() as i32;
            world.spawn_chicken(chicken, false)
        }
        "minecraft:sheep" => {
            let sheep = Sheep { age: Age { ticks: age(), ..Age::default() }, in_love: 0, wool: Wool::default(), persistence_required: true };
            let id = world.spawn_sheep(sheep, position, false);
            let entity = world.sheep_mut(id).unwrap();
            entity.body.velocity = velocity;
            entity.body.on_ground = on_ground;
            entity.yaw = yaw;
            entity.previous_yaw = yaw;
            id
        }
        "minecraft:zombie" => {
            let mut zombie = Zombie::new(position);
            zombie.body.on_ground = on_ground;
            zombie.persistence_required = true;
            world.spawn_zombie_active(zombie, yaw)
        }
        "minecraft:skeleton" => {
            let mut skeleton = Skeleton::of_kind(SkeletonKind::Skeleton, position);
            skeleton.body.on_ground = on_ground;
            skeleton.persistence_required = true;
            world.spawn_skeleton_active(skeleton, yaw)
        }
        "minecraft:creeper" => {
            let mut creeper = Creeper::new(position);
            creeper.body.on_ground = on_ground;
            creeper.yaw = yaw;
            creeper.persistence_required = true;
            world.spawn_creeper(creeper, false)
        }
        "minecraft:spider" => {
            let mut spider = Spider::new(position);
            spider.body.on_ground = on_ground;
            spider.yaw = yaw;
            spider.persistence_required = true;
            world.spawn_spider(spider, false)
        }
        "minecraft:slime" => {
            let mut slime = Slime::new(position, e["cube_size"].as_i64().unwrap() as i32);
            slime.body.on_ground = on_ground;
            slime.persistence_required = true;
            slime.yaw = yaw;
            slime.was_on_ground = e["cube_was_on_ground"].as_bool().unwrap();
            let id = world.spawn_slime(slime, false);
            let entity = world.slime_mut(id).unwrap();
            // The move control's first heading comes from the
            // constructor's unseeded yaw.
            entity.ai.state.cube.y_rot = number(&e["cube_move_y_rot"]) as f32;
            entity.ai.state.cube.jump_delay = e["cube_jump_delay"].as_i64().unwrap() as i32;
            id
        }
        other => panic!("unsupported mob {other}"),
    }
}

fn set_seed(world: &mut EntityWorld, id: u64, seed: u64) {
    if let Some(e) = world.cow_mut(id) {
        e.random = LegacyRandom::new(seed);
    } else if let Some(e) = world.pig_mut(id) {
        e.random = LegacyRandom::new(seed);
    } else if let Some(e) = world.chicken_mut(id) {
        e.random = LegacyRandom::new(seed);
    } else if let Some(e) = world.sheep_mut(id) {
        e.random = LegacyRandom::new(seed);
    } else if let Some(e) = world.zombie_mut(id) {
        e.random = LegacyRandom::new(seed);
    } else if let Some(e) = world.skeleton_mut(id) {
        e.random = LegacyRandom::new(seed);
    } else if let Some(e) = world.creeper_mut(id) {
        e.random = LegacyRandom::new(seed);
    } else if let Some(e) = world.spider_mut(id) {
        e.random = LegacyRandom::new(seed);
    } else if let Some(e) = world.slime_mut(id) {
        e.random = LegacyRandom::new(seed);
    } else {
        panic!("no mob {id} to seed");
    }
}

fn main() {
    let path = env::args().nth(1).expect("usage: check_falls_and_drowning TRACE.jsonl");
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut scenario = String::new();
    let (mut frames, mut heard, mut falls, mut drowned, mut deaths) = (0, 0, 0, 0, 0);
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
            }
            "entity_set_random_seed" => {
                let id = ids[row["data"]["tag"].as_str().unwrap()];
                set_seed(&mut world, id, row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "snapshot" if scenario == "falls_warmup" => {}
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = data["entities"].as_object().unwrap();
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    let mut order: Vec<(&String, &Value)> = observed.iter().map(|(tag, states)| (tag, &states[0])).collect();
                    order.sort_by_key(|(_, e)| e["entity_numeric_id"].as_u64().unwrap());
                    for (tag, e) in order {
                        let id = spawn(&mut world, e);
                        ids.insert(tag.clone(), id);
                    }
                    let _ = world.take_sounds();
                    continue;
                }
                world.tick_with_players(&mut scene, &[]);
                assert_eq!(world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                let (compared, ours) = sounds::check_heard(&mut world, data, &format!("{scenario} tick {tick}"), &sounds::pens(&suite, &scenario));
                heard += compared;
                falls += ours.iter().filter(|s| s.0.ends_with("_fall")).count();
                deaths += ours.iter().filter(|s| s.0.ends_with(".death")).count();
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
                    assert_eq!(e["air_supply"].as_i64().unwrap(), i64::from(mob.body.air), "{at} air");
                    assert_eq!(e["entity_tick_count"], mob.tick_count, "{at} tick count");
                    assert_eq!(e["ambient_sound_time"], mob.ambient, "{at} ambient time");
                    assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), mob.random.raw_state(), "{at} random");
                    drowned += usize::from(mob.body.air <= 0);
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert!(falls > 0 && drowned > 0, "mobs fall and drown");
    println!("{frames} exact fall and drowning frames matched ({heard} sounds, {falls} fall sounds, {deaths} deaths, {drowned} breathless mob-ticks)");
}
