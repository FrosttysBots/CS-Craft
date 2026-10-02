//! Exact pinned 26.3 gate for mobs pushing one another
//! (`scenarios/mobs/pushing.json`): fifteen animals crowded on a chunk
//! border, so that the entity sections they part into decide who pushes
//! whom first; eleven monsters crowded in a pen; twenty-six cows crammed
//! into a one-block well; a pig pushing still (NoAI) mobs, whose motion
//! only grows; and cows sliding across a chunk border and dropping into the
//! section below, where they join the end of the section's list. Each mob
//! runs its full AI (or none, as summoned); every tick
//! compares position, motion, yaw, health, the mob random, its tick and
//! ambient clocks, whether it is still there, and every sound in the pens.
#[path = "support/sounds.rs"]
mod sounds;

use glam::DVec3;
use minecraftoss_entities::{
    age::Age,
    chicken::Chicken,
    cow::{Cow, CowSoundVariant},
    creeper::Creeper,
    movement::Body,
    pig::{Pig, PigSoundVariant},
    sheep::{Sheep, Wool},
    skeleton::Skeleton,
    slime::Slime,
    spider::Spider,
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

fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn exact(observed: &Value, actual: f64, label: &str) {
    assert_eq!(actual.to_bits(), number(observed).to_bits(), "{label}: {actual} vs {}", number(observed));
}

/// What the gate compares of any mob.
struct Seen<'a> {
    body: &'a Body,
    health: f32,
    yaw: f32,
    random: &'a LegacyRandom,
    tick_count: i32,
    ambient: i32,
}

fn seen(world: &EntityWorld, id: u64) -> Option<Seen<'_>> {
    macro_rules! find {
        ($list:expr, |$e:ident| $mob:expr, $yaw:expr) => {
            if let Some($e) = $list.iter().find(|e| e.id == id) {
                return Some(Seen { body: &$mob.body, health: $mob.health, yaw: $yaw, random: &$e.random, tick_count: $e.tick_count, ambient: $e.ambient_sound_time });
            }
        };
    }
    find!(world.cows(), |e| e.cow, e.cow.yaw);
    find!(world.pigs(), |e| e.pig, e.pig.yaw);
    find!(world.chickens(), |e| e.chicken, e.chicken.yaw);
    find!(world.zombies(), |e| e.zombie, e.ai.as_ref().map_or(e.yaw, |ai| ai.yaw));
    find!(world.skeletons(), |e| e.skeleton, e.ai.as_ref().map_or(e.yaw, |ai| ai.yaw));
    find!(world.creepers(), |e| e.creeper, e.ai.yaw);
    find!(world.spiders(), |e| e.spider, e.ai.yaw);
    find!(world.slimes(), |e| e.slime, e.ai.yaw);
    find!(world.witches(), |e| e.witch, e.ai.yaw);
    world
        .sheep()
        .iter()
        .find(|e| e.id == id)
        .map(|e| Seen { body: &e.body, health: e.health, yaw: e.yaw, random: &e.random, tick_count: e.tick_count, ambient: e.ambient_sound_time })
}

/// Spawns an observed mob of its type as the harness found it, with or
/// without AI.
fn spawn(world: &mut EntityWorld, e: &Value) -> u64 {
    let position = DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"]));
    let velocity = DVec3::new(number(&e["vx"]), number(&e["vy"]), number(&e["vz"]));
    let on_ground = e["on_ground"].as_bool().unwrap();
    let no_ai = e["no_ai"].as_bool().unwrap();
    let yaw = number(&e["yaw"]) as f32;
    let age = || e["age"].as_i64().unwrap_or(0) as i32;
    world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
    let id = match e["type"].as_str().unwrap() {
        "minecraft:cow" => {
            let mut cow = Cow::new(position);
            cow.yaw = yaw;
            cow.age.ticks = age();
            cow.persistence_required = true;
            cow.sound_variant = CowSoundVariant::Classic;
            world.spawn_cow(cow, no_ai)
        }
        "minecraft:pig" => {
            let mut pig = Pig::new(position);
            pig.yaw = yaw;
            pig.age.ticks = age();
            pig.persistence_required = true;
            pig.sound_variant = PigSoundVariant::Classic;
            world.spawn_pig(pig, no_ai)
        }
        "minecraft:chicken" => {
            let mut chicken = Chicken::new(position);
            chicken.yaw = yaw;
            chicken.age.ticks = age();
            chicken.persistence_required = true;
            world.spawn_chicken(chicken, no_ai)
        }
        "minecraft:sheep" => {
            let sheep = Sheep { age: Age { ticks: age(), ..Age::default() }, in_love: 0, wool: Wool::default(), persistence_required: true };
            let id = world.spawn_sheep(sheep, position, no_ai);
            let entity = world.sheep_mut(id).unwrap();
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
        "minecraft:skeleton" => {
            // Summoned with data: no finalized equipment, so no bow.
            let mut skeleton = Skeleton::new(position);
            skeleton.persistence_required = true;
            skeleton.holds_bow = false;
            world.spawn_skeleton_active(skeleton, yaw)
        }
        "minecraft:creeper" => {
            let mut creeper = Creeper::new(position);
            creeper.persistence_required = true;
            creeper.yaw = yaw;
            world.spawn_creeper(creeper, no_ai)
        }
        "minecraft:spider" => {
            let mut spider = Spider::new(position);
            spider.persistence_required = true;
            spider.yaw = yaw;
            world.spawn_spider(spider, no_ai)
        }
        "minecraft:slime" => {
            let mut slime = Slime::new(position, e["cube_size"].as_i64().unwrap() as i32);
            slime.persistence_required = true;
            slime.yaw = yaw;
            slime.was_on_ground = e["cube_was_on_ground"].as_bool().unwrap();
            let id = world.spawn_slime(slime, no_ai);
            // The move control's first heading comes from the constructor's
            // unseeded yaw.
            let entity = world.slime_mut(id).unwrap();
            entity.ai.state.cube.y_rot = number(&e["cube_move_y_rot"]) as f32;
            entity.ai.state.cube.jump_delay = e["cube_jump_delay"].as_i64().unwrap() as i32;
            id
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
    body.velocity = velocity;
    body.on_ground = on_ground;
    id
}

fn set_seed(world: &mut EntityWorld, id: u64, seed: u64) {
    let random = LegacyRandom::new(seed);
    if let Some(e) = world.cow_mut(id) {
        e.random = random;
    } else if let Some(e) = world.pig_mut(id) {
        e.random = random;
    } else if let Some(e) = world.chicken_mut(id) {
        e.random = random;
    } else if let Some(e) = world.sheep_mut(id) {
        e.random = random;
    } else if let Some(e) = world.zombie_mut(id) {
        e.random = random;
    } else if let Some(e) = world.skeleton_mut(id) {
        e.random = random;
    } else if let Some(e) = world.creeper_mut(id) {
        e.random = random;
    } else if let Some(e) = world.spider_mut(id) {
        e.random = random;
    } else if let Some(e) = world.slime_mut(id) {
        e.random = random;
    } else if let Some(e) = world.witch_mut(id) {
        e.random = random;
    } else {
        panic!("no mob {id} to seed");
    }
}

fn main() {
    let path = env::args().nth(1).expect("usage: check_pushing TRACE.jsonl");
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut scenario = String::new();
    let (mut frames, mut heard, mut deaths, mut pushed, mut crammed) = (0, 0, 0, 0, 0);
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
                        let block = Block::new(parts[7]);
                        for x in n(1)..=n(4) {
                            for y in n(2)..=n(5) {
                                for z in n(3)..=n(6) {
                                    scene.set_block((x, y, z), Some(block.clone()));
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
            "snapshot" if scenario == "push_warmup" => {}
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
                let before: BTreeMap<u64, (f32, DVec3)> =
                    ids.values().filter_map(|&id| seen(&world, id).map(|m| (id, (m.health, m.body.velocity)))).collect();
                world.tick_with_players(&mut scene, &[]);
                assert_eq!(world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                let (compared, ours) = sounds::check_heard(&mut world, data, &format!("{scenario} tick {tick}"), &sounds::pens(&suite, &scenario));
                heard += compared;
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
                        ("yaw", f64::from(mob.yaw)),
                        ("health", f64::from(mob.health)),
                        ("fall_distance", mob.body.fall_distance),
                    ] {
                        exact(&e[field], actual, &format!("{at} {field}"));
                    }
                    assert_eq!(e["on_ground"], mob.body.on_ground, "{at} on ground");
                    assert_eq!(e["entity_tick_count"], mob.tick_count, "{at} tick count");
                    assert_eq!(e["ambient_sound_time"], mob.ambient, "{at} ambient time");
                    assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), mob.random.raw_state(), "{at} random");
                    if let Some(&(health, _)) = before.get(&id) {
                        // Six at once: cramming.
                        crammed += usize::from(health - mob.health == 6.0 && scenario == "push_crowd");
                    }
                    pushed += usize::from(mob.body.velocity.x.abs() > 0.0 && scenario == "push_still" && mob.body.position.x == 200.5);
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert!(crammed > 0 && pushed > 0, "mobs cram and still mobs are pushed");
    println!("{frames} exact pushing frames matched ({heard} sounds, {deaths} deaths, {crammed} cramming hurts, {pushed} still-zombie pushed ticks)");
}
