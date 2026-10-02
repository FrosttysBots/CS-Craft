//! Exact pinned 26.3 mob sound gate (`scenarios/mobs/mob-sounds.json`): NoAI
//! mobs of each kind (a baby zombie and a moody cow among them) at midnight,
//! and a probe player's sword hits on some. Every sound a mob plays
//! (`Entity.playSound`: its ambient, hurt and death sounds) is compared
//! each tick: event, position, volume and the pitch drawn from the mob's
//! random, with the ambient timers and randoms. The probe's own attack
//! sounds are the client's, so they are left out.
use glam::DVec3;
use minecraftoss_entities::{
    chicken::Chicken,
    cow::{Cow, CowSoundVariant},
    creeper::Creeper,
    pig::Pig,
    sheep::Sheep,
    skeleton::Skeleton,
    villager::Villager,
    world::{EntityWorld, PlayerAttack},
    zombie::Zombie,
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

/// A sound as the comparison sees it: event, position bits, volume bits, pitch bits.
type Heard = (String, [u64; 3], u32, u32);

fn main() {
    let path = env::args().nth(1).expect("usage: check_mob_sounds TRACE.jsonl");
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut probe = (DVec3::ZERO, 0.0_f32);
    let mut heard: Vec<Heard> = Vec::new();
    let (mut frames, mut sounds) = (0, 0);
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => suite = row["data"]["suite"].clone(),
            "complete" => complete = true,
            "scenario_start" => {
                let definition = &suite["scenarios"][0];
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
            }
            "entity_set_random_seed" => {
                let id = ids[row["data"]["tag"].as_str().unwrap()];
                let seed: u64 = row["data"]["seed"].as_str().unwrap().parse().unwrap();
                if let Some(e) = world.zombie_mut(id) {
                    e.set_random_seed(seed);
                } else if let Some(e) = world.skeleton_mut(id) {
                    e.set_random_seed(seed);
                } else if let Some(e) = world.creeper_mut(id) {
                    e.set_random_seed(seed);
                } else if let Some(e) = world.cow_mut(id) {
                    e.set_random_seed(seed as i64);
                } else if let Some(e) = world.pig_mut(id) {
                    e.set_random_seed(seed as i64);
                } else if let Some(e) = world.chicken_mut(id) {
                    e.set_random_seed(seed as i64);
                } else if let Some(e) = world.sheep_mut(id) {
                    e.set_random_seed(seed as i64);
                } else {
                    world.villager_mut(id).unwrap().set_random_seed(seed);
                }
            }
            "player_probe" => {
                let pos = &row["data"]["pos"];
                probe.0 = DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap());
                probe.1 = suite["scenarios"][0]["actions"].as_array().unwrap().iter().find(|a| a["type"] == "player_probe").unwrap()["yaw"].as_f64().unwrap() as f32;
            }
            "player_attack" => {
                let tick = row["tick"].as_i64().unwrap();
                let action = suite["scenarios"][0]["actions"].as_array().unwrap().iter().find(|a| a["type"] == "player_attack" && a["tick"] == tick).unwrap();
                // A diamond sword: 7 damage, 1.6 attacks a second.
                let delay = (1.0 / (4.0 - 2.4000000953674316) * 20.0) as f32;
                let strength = ((action["attack_ticker"].as_i64().unwrap() as f32 + 0.5) / delay).clamp(0.0, 1.0);
                let attack = PlayerAttack { player_id: 1, position: probe.0, yaw: probe.1, attack_damage: 7.0, strength, sprinting: false, can_critical: false, can_sweep: true };
                let target = ids[row["data"]["target"].as_str().unwrap()];
                let result = world.player_attack(&attack, target);
                assert!(result.hurt, "tick {tick} attack lands");
                heard.extend(world.take_sounds().into_iter().map(|s| (format!("minecraft:{}", s.event), [s.position.x.to_bits(), s.position.y.to_bits(), s.position.z.to_bits()], s.volume.to_bits(), s.pitch.to_bits())));
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = data["entities"].as_object().unwrap();
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    let mut order: Vec<(&String, &Value)> = observed.iter().map(|(tag, states)| (tag, &states[0])).collect();
                    order.sort_by_key(|(_, e)| e["entity_numeric_id"].as_u64().unwrap());
                    for (tag, e) in order {
                        let at = DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"]));
                        world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
                        let id = match e["type"].as_str().unwrap() {
                            "minecraft:zombie" => {
                                let mut zombie = Zombie::new(at);
                                zombie.set_baby(tag == "babyzombie");
                                world.spawn_zombie(zombie, true)
                            }
                            "minecraft:skeleton" => world.spawn_skeleton(Skeleton::new(at), true),
                            "minecraft:creeper" => world.spawn_creeper(Creeper::new(at), true),
                            "minecraft:cow" => {
                                let mut cow = Cow::new(at);
                                if tag == "moody" {
                                    cow.sound_variant = CowSoundVariant::Moody;
                                }
                                world.spawn_cow(cow, true)
                            }
                            "minecraft:pig" => world.spawn_pig(Pig::new(at), true),
                            "minecraft:chicken" => world.spawn_chicken(Chicken::new(at), true),
                            "minecraft:sheep" => world.spawn_sheep(Sheep::default(), at, true),
                            "minecraft:villager" => world.spawn_villager(Villager::new(at), true),
                            other => panic!("unexpected {other}"),
                        };
                        ids.insert(tag.clone(), id);
                    }
                    continue;
                }
                world.tick(&mut scene);
                heard.extend(world.take_sounds().into_iter().map(|s| (format!("minecraft:{}", s.event), [s.position.x.to_bits(), s.position.y.to_bits(), s.position.z.to_bits()], s.volume.to_bits(), s.pitch.to_bits())));
                let mut vanilla: Vec<Heard> = data["entity_sound_events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|e| !e["id"].as_str().unwrap().starts_with("minecraft:entity.player."))
                    .map(|e| {
                        let p = e["position"].as_array().unwrap();
                        let bits = |v: &Value| u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap();
                        (e["id"].as_str().unwrap().to_owned(), [bits(&p[0]), bits(&p[1]), bits(&p[2])], u32::from_str_radix(e["volume_bits"].as_str().unwrap(), 16).unwrap(), u32::from_str_radix(e["pitch_bits"].as_str().unwrap(), 16).unwrap())
                    })
                    .collect();
                let mut ours = std::mem::take(&mut heard);
                vanilla.sort();
                ours.sort();
                assert_eq!(ours, vanilla, "tick {tick} sounds");
                sounds += vanilla.len();
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert!(sounds > 20, "mobs are heard");
    println!("{frames} exact mob sound frames matched ({sounds} sounds)");
}
