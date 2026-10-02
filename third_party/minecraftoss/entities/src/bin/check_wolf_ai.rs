//! Exact pinned 26.3 wild wolf gate (`scenarios/mobs/wolf-ai.json`): wild
//! wolves in walled stone pens of an empty void world hunt two sheep
//! (`NonTameRandomTargetGoal`, `MeleeAttackGoal`, `LeapAtTargetGoal`), turn
//! as a pack on a probe player that hits one of them
//! (`HurtByTargetGoal.alertOthers`, anger and the angry-player target), and
//! beg at a probe holding a bone (`BegGoal`); a wolf wet from a water hole
//! shakes itself dry once the hole is drained. Every tick compares each
//! wolf's position, motion, rotations, controls, path, goals and target
//! goals, target, anger, begging and head tilt, wetness, health and random;
//! each sheep's position, motion, health, goals and random; the probe's
//! health, motion and hurt state after the bites; and every sound in the
//! pens. `scenarios/mobs/wolf-tame.json` tames one: a probe uses bones
//! until it takes (each later use orders it to sit or stand), walks off so
//! that it teleports along, feeds it into love and back to health and dyes
//! its collar; a tame wolf whose owner is absent sits by itself. Its
//! taming, owner, sitting, love and collar are compared each tick too, and
//! each use's result and what is left in the probe's hand.
//! `scenarios/mobs/wolf-guard.json` has a tamed wolf fight the zombie that
//! bit its owner and then the cow its owner hit (the owner's hurts and hits
//! stamped with the tick count its probe carries); the zombie and the cow
//! are compared each tick as well. `scenarios/mobs/wolf-breed.json` has two
//! tamed wolves, fed into love, breed a pup (its random pinned) whose
//! variant, voice, owner, health and crafted collar colour are compared as
//! it grows. With the data JAR as the second argument the recipes mix the
//! collars. `scenarios/mobs/wolf-skeleton.json` has a wild wolf hunt a bow
//! skeleton that flees wolves within six blocks (`AvoidEntityGoal`) and
//! shoots back; the skeleton is compared each tick too.
#[path = "support/sounds.rs"]
mod sounds;

use glam::DVec3;
use minecraftoss_entities::{
    cow::{Cow, CowSoundVariant},
    monster_ai::Target,
    sheep::Sheep,
    skeleton::Skeleton,
    tempt::PlayerCandidate,
    wolf::{self, Wolf},
    world::{EntityWorld, PlayerAttack, PlayerHitKind},
    zombie::Zombie,
};
use minecraftoss_player::inventory::{Inventory, ItemStack};
use minecraftoss_player::survival::{Armor, Difficulty};
use minecraftoss_player::{Block, HitFrom, IncomingHit, Player, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

/// The pens' blocks at day time 4000, or at midnight.
#[derive(Default)]
struct Scene {
    blocks: BTreeMap<Pos, Block>,
    night: bool,
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
    fn light_path_cost(&self, pos: Pos) -> f32 {
        // Every place a stroll can pick (the pen floors and wall tops) is
        // open to the sky: by day level 15, whose magic value is 1, less a
        // half; at midnight the sky's level 4 (sky light 0 under cover).
        if !self.night {
            0.5
        } else if self.can_see_sky(pos) {
            (4.0_f32 / 15.0) / (4.0 - 3.0 * (4.0_f32 / 15.0)) - 0.5
        } else {
            -0.5
        }
    }
}

fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn exact(observed: &Value, actual: f64, label: &str) {
    assert_eq!(actual.to_bits(), number(observed).to_bits(), "{label}: {actual} vs {}", number(observed));
}

/// A mob's health, whichever kind it is.
fn health_of(world: &EntityWorld, id: u64) -> f32 {
    world
        .wolves()
        .iter()
        .find(|e| e.id == id)
        .map(|e| e.wolf.health)
        .or_else(|| world.cows().iter().find(|e| e.id == id).map(|e| e.cow.health))
        .or_else(|| world.zombies().iter().find(|e| e.id == id).map(|e| e.zombie.health))
        .or_else(|| world.sheep().iter().find(|e| e.id == id).map(|e| e.health))
        .or_else(|| world.skeletons().iter().find(|e| e.id == id).map(|e| e.skeleton.health))
        .expect("a mob")
}

fn candidate(id: u64, position: DVec3, item: &str) -> PlayerCandidate {
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
        main_hand_wolf_interest: wolf::interests(item),
        offhand_wolf_interest: false,
        main_hand_horse_tempt: false,
        offhand_horse_tempt: false,
        alive: true,
        spectator: false,
        attackable: true,
    }
}

/// A probe: what the mobs see, the player the bites land on, its UUID and
/// what it carries.
struct Probe {
    candidate: PlayerCandidate,
    player: Player,
    uuid: String,
    inventory: Inventory,
}

impl Probe {
    /// What wolves see it hold.
    fn update_interest(&mut self) {
        let held = self.inventory.slots[0].as_ref().map_or("minecraft:air", |s| s.id.as_str());
        self.candidate.main_hand_wolf_interest = wolf::interests(held);
    }
}

fn main() {
    let path = env::args().nth(1).expect("usage: check_wolf_ai TRACE.jsonl [DATA.jar]");
    let recipes = env::args().nth(2).map(|jar| std::sync::Arc::new(minecraftoss_player::crafting::RecipeBook::from_jar(std::path::Path::new(&jar)).expect("recipes")));
    // Pups' pinned tags, in turn.
    let mut born_pins: std::collections::VecDeque<String> = Default::default();
    let mut births = 0;
    // The `mob_drops` game rule, which outlives scenarios.
    let mut mob_drops = true;
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut uuids: BTreeMap<u64, String> = BTreeMap::new();
    let mut probes: BTreeMap<String, Probe> = BTreeMap::new();
    let mut scenario = String::new();
    let (mut frames, mut sheep_frames, mut probe_frames, mut heard, mut bites, mut kills) = (0, 0, 0, 0, 0, 0);
    let mut goals_seen: BTreeMap<String, usize> = BTreeMap::new();
    let (mut begging, mut shaking) = (0, 0);
    let (mut uses, mut tame_frames, mut sitting, mut in_love, mut teleports) = (0, 0, 0, 0, 0);
    let (mut other_frames, mut skeleton_frames) = (0, 0);
    // Where each wolf stood at the last frame (a teleport jumps).
    let mut last_positions: BTreeMap<String, DVec3> = BTreeMap::new();
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => suite = row["data"]["suite"].clone(),
            "complete" => complete = true,
            "scenario_start" => {
                scenario = row["scenario"].as_str().unwrap().to_owned();
                world = EntityWorld::default();
                world.set_mob_drops(mob_drops);
                if let Some(book) = &recipes {
                    world.set_recipe_book(book.clone());
                }
                born_pins.clear();
                ids.clear();
                uuids.clear();
                probes.clear();
                last_positions.clear();
            }
            // The pens' fills as they run, and the drain in the middle of a
            // scenario (the actions run before the tick's entities).
            "command" => {
                let parts: Vec<&str> = row["data"]["command"].as_str().unwrap().split_whitespace().collect();
                if parts[..2] == ["gamerule", "minecraft:mob_drops"] {
                    mob_drops = parts[2] == "true";
                    world.set_mob_drops(mob_drops);
                }
                if parts[..2] == ["time", "set"] {
                    scene.night = match parts[2] {
                        "4000" | "6000" | "noon" | "day" => false,
                        "18000" | "midnight" => true,
                        other => panic!("{scenario}: no light model for time {other}"),
                    };
                }
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
            "projectile_shoot_seed" => world.set_arrow_shoot_seed(Some(row["data"]["seed"].as_str().unwrap().parse().unwrap())),
            "projectile_damage_seed" => world.set_arrow_damage_seed(Some(row["data"]["seed"].as_str().unwrap().parse().unwrap())),
            "born_mob_seed" => {
                let seed: u64 = row["data"]["seed"].as_str().unwrap().parse::<i64>().unwrap() as u64;
                world.push_born_seed(seed);
                born_pins.push_back(row["data"]["tag"].as_str().unwrap().to_owned());
            }
            "entity_set_random_seed" => {
                let id = ids[row["data"]["tag"].as_str().unwrap()];
                let seed: u64 = row["data"]["seed"].as_str().unwrap().parse::<i64>().unwrap() as u64;
                if let Some(e) = world.wolf_mut(id) {
                    e.set_random_seed(seed);
                } else if let Some(e) = world.sheep_mut(id) {
                    e.set_random_seed(seed as i64);
                } else if let Some(e) = world.cow_mut(id) {
                    e.set_random_seed(seed as i64);
                } else if let Some(e) = world.zombie_mut(id) {
                    e.random = minecraftoss_player::rng::LegacyRandom::new(seed);
                } else if let Some(e) = world.skeleton_mut(id) {
                    e.random = minecraftoss_player::rng::LegacyRandom::new(seed);
                } else {
                    panic!("no mob to seed");
                }
            }
            "player_probe" => {
                // Placed, or moved (the same player): its main hand is set
                // anew, then the listed slots.
                let data = &row["data"];
                let tag = data["tag"].as_str().unwrap().to_owned();
                let pos = &data["pos"];
                let position = DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap());
                let id = probes.get(&tag).map_or(1_000_000 + probes.len() as u64, |p| p.candidate.id);
                // A new probe is an entity in vanilla: it takes the next ID.
                if !probes.contains_key(&tag) {
                    let next = world.next_entity_id();
                    world.set_next_entity_id(next + 1);
                }
                let uuid = data["uuid"].as_str().unwrap().to_owned();
                world.set_player_uuid(id, minecraftoss_entities::gossip::parse_uuid(&uuid).unwrap());
                let item = data["item"].as_str().unwrap();
                let probe = probes.entry(tag.clone()).or_insert_with(|| {
                    let mut player = Player::new(position);
                    // A probe never ticks: it never lands.
                    player.on_ground = false;
                    Probe { candidate: candidate(id, position, item), player, uuid, inventory: Inventory::default() }
                });
                probe.candidate.position = position;
                probe.player.pos = position;
                probe.player.yaw = data["head_yaw"].as_f64().unwrap_or(0.0);
                probe.inventory.slots[0] = (item != "minecraft:air").then(|| ItemStack::new(item, 1));
                let definition = suite["scenarios"].as_array().unwrap().iter().find(|c| c["id"] == scenario.as_str()).unwrap();
                let action = definition["actions"].as_array().unwrap().iter().find(|a| a["type"] == "player_probe" && a["tag"] == tag.as_str() && a["tick"] == row["tick"]).unwrap();
                for s in action["inventory"].as_array().into_iter().flatten() {
                    probe.inventory.slots[s["slot"].as_u64().unwrap() as usize] = Some(ItemStack::new(s["item"].as_str().unwrap(), s["count"].as_u64().unwrap() as u8));
                }
                probe.update_interest();
                // The tick count its tick would have (it never ticks).
                if let Some(ticks) = action["tick_count"].as_i64() {
                    world.set_player_tick_count(id, ticks as i32);
                }
            }
            "probe_interact" => {
                // `Player.interactOn` with the main hand.
                let data = &row["data"];
                let at = format!("{scenario} tick {} use", row["tick"]);
                let probe = probes.get_mut(data["probe"].as_str().unwrap()).unwrap();
                let result = world.wolf_interact(ids[data["tag"].as_str().unwrap()], probe.candidate.id, &mut probe.inventory, 0, false);
                let name = match result {
                    minecraftoss_entities::cow::InteractionResult::Pass => "pass",
                    minecraftoss_entities::cow::InteractionResult::SuccessServer => "success_server_only",
                    minecraftoss_entities::cow::InteractionResult::SuccessPredicted => "success_predicted",
                };
                assert_eq!(data["result"].as_str().unwrap(), name, "{at} result");
                let hand = probe.inventory.slots[0].as_ref();
                assert_eq!(data["hand_item"].as_str().unwrap(), hand.map_or("minecraft:air", |s| s.id.as_str()), "{at} hand");
                assert_eq!(data["hand_count"].as_u64().unwrap(), hand.map_or(0, |s| u64::from(s.count)), "{at} hand count");
                probe.update_interest();
                uses += 1;
            }
            "entity_hurt" => {
                // `hurtServer` before the tick, with no attacker.
                let data = &row["data"];
                let at = format!("{scenario} tick {} hurt", row["tick"]);
                assert!(data.get("attacker").is_none_or(Value::is_null), "{at}: no attacker");
                let time = world.game_time();
                let entity = world.wolf_mut(ids[data["tag"].as_str().unwrap()]).unwrap();
                exact(&data["health_before"], f64::from(entity.wolf.health), &format!("{at} health before"));
                let result = entity.hurt_from(number(&data["amount"]) as f32, "minecraft:generic", None, time);
                assert_eq!(result.applied, data["applied"].as_bool().unwrap(), "{at} applied");
                exact(&data["health_after"], f64::from(entity.wolf.health), &format!("{at} health after"));
            }
            "player_attack" => {
                // A full-strength bare-handed hit on the ground, along the
                // probe's facing.
                let data = &row["data"];
                let at = format!("{scenario} tick {} attack", row["tick"]);
                let probe = probes.get_mut(data["probe"].as_str().unwrap()).unwrap();
                let attack = PlayerAttack {
                    player_id: probe.candidate.id,
                    position: probe.candidate.position,
                    yaw: probe.player.yaw as f32,
                    attack_damage: 1.0,
                    strength: 1.0,
                    sprinting: false,
                    can_critical: false,
                    can_sweep: false,
                };
                // `Player.attack` tires the attacker; the harness puts the
                // attacking probe on the ground.
                probe.player.survival.food.exhaustion = number(&data["exhaustion"]) as f32;
                probe.player.on_ground = true;
                let target = ids[data["target"].as_str().unwrap()];
                exact(&data["health_before"], f64::from(health_of(&world, target)), &format!("{at} health before"));
                let result = world.player_attack(&attack, target);
                assert!(result.hurt, "{at} lands");
                exact(&data["health_after"], f64::from(health_of(&world, target)), &format!("{at} health after"));
            }
            "snapshot" if scenario.ends_with("_warmup") => {}
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = data["entities"].as_object().unwrap();
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    // Tags of mobs yet to be born have no state.
                    let mut order: Vec<(&String, &Value)> = observed.iter().filter_map(|(tag, states)| Some((tag, states.as_array()?.first()?))).collect();
                    order.sort_by_key(|(_, e)| e["entity_numeric_id"].as_u64().unwrap());
                    for (tag, e) in order {
                        let position = DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"]));
                        let yaw = number(&e["yaw"]) as f32;
                        let on_ground = e["on_ground"].as_bool().unwrap();
                        let no_ai = e["no_ai"].as_bool().unwrap();
                        world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
                        let id = match e["type"].as_str().unwrap() {
                            "minecraft:wolf" => {
                                let mut wolf = Wolf::new(position);
                                wolf.body.on_ground = on_ground;
                                wolf.persistence_required = true;
                                wolf.yaw = yaw;
                                wolf.variant = e["wolf_variant"].as_str().unwrap().trim_start_matches("minecraft:").to_owned();
                                wolf.sound_variant = e["wolf_sound_variant"].as_str().unwrap().trim_start_matches("minecraft:").to_owned();
                                wolf.collar = e["collar"].as_u64().unwrap() as u8;
                                wolf.health = number(&e["health"]) as f32;
                                // `TamableAnimal.readAdditionalSaveData`: tame
                                // with an owner, no taming side effects.
                                wolf.owner = e["owner"].as_str().map(|u| minecraftoss_entities::gossip::parse_uuid(u).unwrap());
                                wolf.tame = e["tame"].as_bool().unwrap();
                                wolf.ordered_to_sit = e["ordered_to_sit"].as_bool().unwrap();
                                wolf.sitting = e["sitting"].as_bool().unwrap();
                                world.spawn_wolf(wolf, no_ai)
                            }
                            "minecraft:sheep" => {
                                let sheep = Sheep { persistence_required: true, ..Sheep::default() };
                                let id = world.spawn_sheep(sheep, position, no_ai);
                                let entity = world.sheep_mut(id).unwrap();
                                entity.body.on_ground = on_ground;
                                entity.yaw = yaw;
                                assert_eq!(e["sheep_color"], entity.sheep.wool.color(), "{scenario} {tag} color");
                                id
                            }
                            "minecraft:zombie" => {
                                let mut zombie = Zombie::new(position);
                                zombie.persistence_required = true;
                                zombie.body.on_ground = on_ground;
                                world.spawn_zombie_active(zombie, yaw)
                            }
                            "minecraft:skeleton" => {
                                let mut skeleton = Skeleton::new(position);
                                skeleton.body.on_ground = on_ground;
                                skeleton.persistence_required = true;
                                world.spawn_skeleton_active(skeleton, yaw)
                            }
                            "minecraft:cow" => {
                                let mut cow = Cow::new(position);
                                cow.body.on_ground = on_ground;
                                cow.yaw = yaw;
                                cow.persistence_required = true;
                                cow.sound_variant = CowSoundVariant::Classic;
                                world.spawn_cow(cow, no_ai)
                            }
                            other => panic!("unexpected {other}"),
                        };
                        assert_eq!(id, e["entity_numeric_id"].as_u64().unwrap(), "{scenario} id");
                        ids.insert(tag.clone(), id);
                        uuids.insert(id, e["uuid"].as_str().unwrap().to_owned());
                    }
                    let _ = world.take_sounds();
                    continue;
                }
                // The level random as this tick's entities began.
                if let Some(state) = data["level_random_before_entities"].as_u64() {
                    *world.level_random_mut() = minecraftoss_player::rng::LegacyRandom::from_raw_state(state);
                }
                // `Mob.checkDespawn` before each mob ticks: a probe within 32
                // blocks restarts the idle clocks.
                let players: Vec<PlayerCandidate> = probes.values().map(|p| p.candidate).collect();
                let feet: Vec<DVec3> = players.iter().map(|p| p.position).collect();
                world.check_despawn(&feet, false, &|_| true);
                world.tick_with_players(&mut scene, &players);
                assert_eq!(world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                // A pup takes the next pinned tag.
                for pup in world.take_born_wolves() {
                    let tag = born_pins.pop_front().expect("each pup has a pinned seed and tag");
                    ids.insert(tag, pup);
                    births += 1;
                }
                heard += sounds::check(&mut world, data, &format!("{scenario} tick {tick}"), &sounds::pens(&suite, &scenario));
                // The bites go through `Player.hurtServer` (the probe never
                // ticks, so its damage cooldown never runs out).
                for hit in world.take_player_hits() {
                    let PlayerHitKind::Melee { attacker, .. } = hit.kind else { panic!("{scenario} tick {tick}: only bites") };
                    let probe = probes.values_mut().find(|p| p.candidate.id == hit.player_id).unwrap();
                    let incoming = IncomingHit { damage: hit.damage, from: HitFrom::Position(attacker), scales_with_difficulty: true, exhaustion: 0.1 };
                    if probe.player.hurt_by(&incoming, Difficulty::Normal, Armor { value: Some((0.0, 0.0)) }, 0.0) {
                        // The biter is what it remembers hurt it.
                        world.player_hurt(hit.player_id, hit.source, "minecraft:mob_attack");
                        bites += 1;
                    }
                }
                for (tag, p) in &probes {
                    let at = format!("{scenario} tick {tick} probe {tag}");
                    let theirs = &data["player_probes"][tag.as_str()];
                    let player = &p.player;
                    exact(&theirs["health"], f64::from(player.survival.health), &format!("{at} health"));
                    exact(&theirs["motion_x"], player.velocity.x, &format!("{at} motion x"));
                    exact(&theirs["motion_y"], player.velocity.y, &format!("{at} motion y"));
                    exact(&theirs["motion_z"], player.velocity.z, &format!("{at} motion z"));
                    exact(&theirs["hurt_dir"], f64::from(player.hurt_dir), &format!("{at} hurt dir"));
                    exact(&theirs["exhaustion"], f64::from(player.survival.food.exhaustion), &format!("{at} exhaustion"));
                    assert_eq!(theirs["hurt_time"], player.hurt_time, "{at} hurt time");
                    probe_frames += 1;
                }
                // A pup's UUID (drawn before its pinned seed), once it is here.
                for (tag, states) in observed {
                    if let (Some(&id), Some(uuid)) = (ids.get(tag), states.as_array().and_then(|s| s.first()).and_then(|e| e["uuid"].as_str())) {
                        uuids.entry(id).or_insert_with(|| uuid.to_owned());
                    }
                }
                // `Mob.getTarget` as the harness reads it after the tick:
                // through `asValidTarget`, so a target killed since reads as
                // none.
                let alive = |id: u64| {
                    world.wolves().iter().any(|e| e.id == id && e.wolf.health > 0.0)
                        || world.sheep().iter().any(|e| e.id == id && e.health > 0.0)
                        || world.zombies().iter().any(|e| e.id == id && e.zombie.health > 0.0)
                        || world.cows().iter().any(|e| e.id == id && e.cow.health > 0.0)
                        || world.skeletons().iter().any(|e| e.id == id && e.skeleton.health > 0.0)
                };
                let target_uuid = |target: Option<Target>| {
                    target.and_then(|t| match t {
                        Target::Player(id) => Some(probes.values().find(|p| p.candidate.id == id).unwrap().uuid.clone()),
                        Target::Villager(id) | Target::Mob(id) => alive(id).then(|| uuids[&id].clone()),
                    })
                };
                for (tag, states) in observed {
                    let states = states.as_array().unwrap();
                    // A pup not born yet has neither ID nor state.
                    let Some(&id) = ids.get(tag) else {
                        assert!(states.is_empty(), "{scenario} tick {tick} {tag}: vanilla made it, we did not");
                        continue;
                    };
                    let at = format!("{scenario} tick {tick} {tag}");
                    if let Some(entity) = world.wolves().iter().find(|e| e.id == id) {
                        assert_eq!(states.len(), 1, "{at} presence");
                        let e = &states[0];
                        let ai = &entity.ai;
                        let (body, state, w) = (&entity.wolf.body, &ai.state, &entity.wolf);
                        for (field, actual) in [
                            ("x", body.position.x),
                            ("y", body.position.y),
                            ("z", body.position.z),
                            ("vx", body.velocity.x),
                            ("vy", body.velocity.y),
                            ("vz", body.velocity.z),
                            ("health", f64::from(w.health)),
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
                            ("interested_angle", f64::from(w.interested_angle)),
                            ("shake_anim", f64::from(w.shake_anim)),
                        ] {
                            exact(&e[field], actual, &format!("{at} {field}"));
                        }
                        assert_eq!(e["on_ground"], body.on_ground, "{at} on ground");
                        assert_eq!(e["air_supply"], body.air, "{at} air");
                        assert_eq!(e["entity_tick_count"], entity.tick_count, "{at} tick count");
                        assert_eq!(e["ambient_sound_time"], entity.ambient_sound_time, "{at} ambient time");
                        assert_eq!(e["no_action_time"], entity.no_action_time, "{at} idle time");
                        // The harness records a mob's random once it has ticked
                        // (a pup's not in the tick it is born).
                        match e["random_state"].as_str() {
                            Some(random) => assert_eq!(random.parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random"),
                            None => assert_eq!(entity.tick_count, 0, "{at} random before its first tick"),
                        }
                        let goals: Vec<&str> = e["running_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                        assert_eq!(goals, ai.running_goals(), "{at} goals");
                        let targets: Vec<&str> = e["running_target_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                        assert_eq!(targets, ai.running_targets(), "{at} target goals");
                        for goal in ai.running_goals().into_iter().chain(ai.running_targets()) {
                            *goals_seen.entry(goal.to_owned()).or_default() += 1;
                        }
                        assert_eq!(e["target_uuid"].as_str().map(str::to_owned), target_uuid(state.target), "{at} target");
                        assert_eq!(e["anger_end_time"].as_i64().unwrap(), state.enderman.anger_end_time, "{at} anger end");
                        // The persistent anger target is a reference: it names
                        // a dead target too.
                        let angry_at = state.enderman.anger_target.map(|t| match t {
                            Target::Player(id) => probes.values().find(|p| p.candidate.id == id).unwrap().uuid.clone(),
                            Target::Villager(id) | Target::Mob(id) => uuids[&id].clone(),
                        });
                        assert_eq!(e["angry_at"].as_str().map(str::to_owned), angry_at, "{at} angry at");
                        assert_eq!(e["aggressive"], state.melee.aggressive, "{at} aggressive");
                        assert_eq!(e["move_control_wanted"], ai.move_control.has_wanted(), "{at} move wanted");
                        assert_eq!(e["look_control_wanted"], state.look_control.cooldown > 0, "{at} look wanted");
                        assert_eq!(e["navigation_done"], state.navigation.is_done(), "{at} navigation done");
                        assert_eq!(e["path_next_node"], state.navigation.observed_next(), "{at} path next");
                        let nodes: Vec<(i32, i32, i32)> = e["path_nodes"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|n| (n[0].as_i64().unwrap() as i32, n[1].as_i64().unwrap() as i32, n[2].as_i64().unwrap() as i32))
                            .collect();
                        assert_eq!(nodes, state.navigation.nodes, "{at} path");
                        for (field, actual) in [("interested", w.interested), ("wet", w.wet), ("shaking", w.shaking), ("tame", w.tame), ("sitting", w.sitting), ("ordered_to_sit", w.ordered_to_sit)] {
                            assert_eq!(e[field], actual, "{at} {field}");
                        }
                        assert_eq!(e["collar"], w.collar, "{at} collar");
                        assert_eq!(e["in_love"], w.in_love, "{at} in love");
                        assert_eq!(e["age"], w.age.ticks, "{at} age");
                        // Recorded by later captures.
                        if e.get("max_health").is_some() {
                            exact(&e["max_health"], f64::from(w.max_health()), &format!("{at} max health"));
                        }
                        let owner = w.owner.map(|o| minecraftoss_entities::gossip::uuid_string(o));
                        assert_eq!(e["owner"].as_str().map(str::to_owned), owner, "{at} owner");
                        if w.tame {
                            tame_frames += 1;
                            sitting += usize::from(w.sitting);
                            in_love += usize::from(w.in_love > 0);
                        }
                        if let Some(last) = last_positions.insert(tag.clone(), body.position) {
                            teleports += usize::from(body.position.distance(last) > 1.5);
                        }
                        assert_eq!(e["wolf_variant"].as_str().unwrap(), format!("minecraft:{}", w.variant), "{at} variant");
                        assert_eq!(e["wolf_sound_variant"].as_str().unwrap(), format!("minecraft:{}", w.sound_variant), "{at} sound variant");
                        begging += usize::from(w.interested);
                        shaking += usize::from(w.shaking);
                        frames += 1;
                        continue;
                    }
                    // A zombie: its motion, health, random, goals and target.
                    if let Some(entity) = world.zombies().iter().find(|e| e.id == id) {
                        assert_eq!(states.len(), 1, "{at} presence");
                        let e = &states[0];
                        let ai = entity.ai.as_deref().unwrap();
                        let body = &entity.zombie.body;
                        for (field, actual) in [
                            ("x", body.position.x),
                            ("y", body.position.y),
                            ("z", body.position.z),
                            ("vx", body.velocity.x),
                            ("vy", body.velocity.y),
                            ("vz", body.velocity.z),
                            ("health", f64::from(entity.zombie.health)),
                            ("yaw", f64::from(ai.yaw)),
                        ] {
                            exact(&e[field], actual, &format!("{at} {field}"));
                        }
                        assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random");
                        let goals: Vec<&str> = e["running_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                        assert_eq!(goals, ai.running_goals(), "{at} goals");
                        assert_eq!(e["target_uuid"].as_str().map(str::to_owned), target_uuid(ai.state.target), "{at} target");
                        other_frames += 1;
                        continue;
                    }
                    // A skeleton: its motion, rotations, health, random, goals and
                    // target.
                    if let Some(entity) = world.skeletons().iter().find(|e| e.id == id) {
                        assert_eq!(states.len(), 1, "{at} presence");
                        let e = &states[0];
                        let ai = entity.ai.as_deref().unwrap();
                        let body = &entity.skeleton.body;
                        for (field, actual) in [
                            ("x", body.position.x),
                            ("y", body.position.y),
                            ("z", body.position.z),
                            ("vx", body.velocity.x),
                            ("vy", body.velocity.y),
                            ("vz", body.velocity.z),
                            ("health", f64::from(entity.skeleton.health)),
                            ("yaw", f64::from(ai.yaw)),
                            ("head_yaw", f64::from(ai.state.look_control.head_yaw)),
                            ("pitch", f64::from(ai.state.look_control.pitch)),
                        ] {
                            exact(&e[field], actual, &format!("{at} {field}"));
                        }
                        assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random");
                        let goals: Vec<&str> = e["running_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                        assert_eq!(goals, ai.running_goals(), "{at} goals");
                        for goal in ai.running_goals() {
                            *goals_seen.entry(format!("skeleton {goal}")).or_default() += 1;
                        }
                        skeleton_frames += 1;
                        assert_eq!(e["target_uuid"].as_str().map(str::to_owned), target_uuid(ai.state.target), "{at} target");
                        other_frames += 1;
                        continue;
                    }
                    // A cow: its motion, health, random and goals.
                    if let Some(entity) = world.cows().iter().find(|e| e.id == id) {
                        assert_eq!(states.len(), 1, "{at} presence");
                        let e = &states[0];
                        let body = &entity.cow.body;
                        for (field, actual) in [
                            ("x", body.position.x),
                            ("y", body.position.y),
                            ("z", body.position.z),
                            ("vx", body.velocity.x),
                            ("vy", body.velocity.y),
                            ("vz", body.velocity.z),
                            ("health", f64::from(entity.cow.health)),
                        ] {
                            exact(&e[field], actual, &format!("{at} {field}"));
                        }
                        assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random");
                        assert_eq!(e["running_goals"], serde_json::json!(entity.running_goals()), "{at} goals");
                        other_frames += 1;
                        continue;
                    }
                    // A sheep (or any mob once removed, twenty ticks after it died).
                    let Some(entity) = world.sheep().iter().find(|e| e.id == id) else {
                        assert!(states.is_empty(), "{at} presence");
                        continue;
                    };
                    assert_eq!(states.len(), 1, "{at} presence");
                    let e = &states[0];
                    for (field, actual) in [
                        ("x", entity.body.position.x),
                        ("y", entity.body.position.y),
                        ("z", entity.body.position.z),
                        ("vx", entity.body.velocity.x),
                        ("vy", entity.body.velocity.y),
                        ("vz", entity.body.velocity.z),
                        ("yaw", f64::from(entity.yaw)),
                        ("health", f64::from(entity.health)),
                    ] {
                        exact(&e[field], actual, &format!("{at} {field}"));
                    }
                    assert_eq!(e["on_ground"], entity.body.on_ground, "{at} on ground");
                    assert_eq!(e["alive"], entity.health > 0.0, "{at} alive");
                    assert_eq!(e["running_goals"], serde_json::json!(entity.running_goals()), "{at} goals");
                    assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random");
                    if entity.health <= 0.0 && entity.damage.death_ticks == 1 {
                        kills += 1;
                    }
                    sheep_frames += 1;
                }
                assert!(data["random_state"]["seed"].as_u64().is_none_or(|seed| seed == world.level_random_mut().raw_state()), "{scenario} tick {tick} level random");
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    if skeleton_frames > 0 {
        // The skeleton fixture: the wolf hunts, the skeleton flees and shoots.
        assert!(goals_seen.contains_key("skeleton AvoidEntityGoal") && goals_seen.contains_key("skeleton RangedBowAttackGoal"), "the skeleton flees and shoots");
        println!("{frames} exact wolf frames and {skeleton_frames} exact skeleton frames matched ({heard} sounds; goal-ticks {goals_seen:?})");
        return;
    }
    if tame_frames > 0 {
        assert!(uses > 0 && sitting > 0, "wolves are tamed and sit");
        println!("{tame_frames} exact tame wolf frames ({uses} uses, {sitting} sitting and {in_love} in-love wolf-ticks; {other_frames} zombie and cow frames; {births} pups born)");
    } else {
        assert!(bites > 0 && kills > 0 && begging > 0 && shaking > 0, "the wolves bite, kill, beg and shake");
    }
    println!(
        "{frames} exact wolf frames matched ({sheep_frames} sheep frames, {probe_frames} probe frames, {heard} sounds, {bites} bites on probes, {kills} sheep killed, {begging} begging and {shaking} shaking wolf-ticks, {teleports} teleports; goal-ticks {goals_seen:?})"
    );
}
