//! Exact pinned 26.3 player melee gate (`scenarios/mobs/player-attacks.json`):
//! a probe player hits mobs through `Player.attack` with a diamond sword
//! (sweep with no one near), a sprinting stone sword (knockback attack,
//! zombie armor), a falling bare hand (a critical; the creeper then turns on
//! the probe), a weak iron axe, an iron sword's sweep over a group, and two
//! quick fist hits (the second inside the damage cooldown). The attack's
//! player half comes from the action (held item attributes from the item
//! catalog, attack strength, sprinting, ground contact, fall distance); the
//! mobs run in the entity world. Every tick compares the mobs' motion,
//! health, goals, target and random.
use glam::DVec3;
use minecraftoss_entities::{
    cow::Cow,
    creeper::Creeper,
    pig::Pig,
    sheep::Sheep,
    tempt::PlayerCandidate,
    world::{EntityWorld, PlayerAttack},
    zombie::Zombie,
};
use minecraftoss_player::item_catalog::{attribute_value, ItemCatalog};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs, path::Path};

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
    fn can_see_sky(&self, (x, y, z): Pos) -> bool {
        !self.0.keys().any(|&(bx, by, bz)| bx == x && bz == z && by > y)
    }
    /// Noon: full sky light under open sky, none below blocks.
    fn light_path_cost(&self, pos: Pos) -> f32 {
        if self.can_see_sky(pos) { 0.5 } else { -0.5 }
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

/// A probe as placed: where, facing and holding what.
struct Probe {
    candidate: PlayerCandidate,
    yaw: f32,
    item: String,
}

#[derive(Default)]
struct Case {
    world: EntityWorld,
    ids: BTreeMap<String, u64>,
    probes: BTreeMap<String, Probe>,
}

fn main() {
    let path = env::args().nth(1).expect("usage: check_player_attacks TRACE.jsonl [ITEM_CATALOG]");
    let catalog_path = env::args().nth(2).unwrap_or_else(|| "artifacts/item-catalog/26.3.json".to_owned());
    let catalog = ItemCatalog::from_path(Path::new(&catalog_path)).expect("item catalog");
    let attributes = |item: &str| {
        let modifiers = catalog.get(item).map(|p| p.attribute_modifiers.clone()).unwrap_or_default();
        let of = |attribute: &str| modifiers.iter().filter(|m| m.in_main_hand() && m.attribute == attribute).cloned().collect::<Vec<_>>();
        (attribute_value(1.0, &of("minecraft:attack_damage"), (0.0, 2048.0)), attribute_value(4.0, &of("minecraft:attack_speed"), (0.0, 1024.0)))
    };
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut case = Case::default();
    let mut scenario = String::new();
    let (mut frames, mut attacks, mut swept, mut crits) = (0, 0, 0, 0);
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
                case = Case::default();
                // The suite runs at noon, when undead burn.
                case.world.set_monsters_burn(true);
            }
            "entity_set_random_seed" => {
                let id = case.ids[row["data"]["tag"].as_str().unwrap()];
                let seed: u64 = row["data"]["seed"].as_str().unwrap().parse().unwrap();
                let world = &mut case.world;
                if let Some(e) = world.cow_mut(id) {
                    e.set_random_seed(seed as i64);
                } else if let Some(e) = world.zombie_mut(id) {
                    e.set_random_seed(seed);
                } else if let Some(e) = world.creeper_mut(id) {
                    e.set_random_seed(seed);
                } else if let Some(e) = world.sheep_mut(id) {
                    e.set_random_seed(seed as i64);
                } else {
                    world.pig_mut(id).unwrap().set_random_seed(seed as i64);
                }
            }
            "player_probe" => {
                let tag = row["data"]["tag"].as_str().unwrap().to_owned();
                let pos = &row["data"]["pos"];
                let position = DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap());
                let definition = suite["scenarios"].as_array().unwrap().iter().find(|c| c["id"] == scenario.as_str()).unwrap();
                let action = definition["actions"].as_array().unwrap().iter().find(|a| a["type"] == "player_probe" && a["tag"] == tag.as_str()).unwrap();
                let id = 1_000_000 + case.probes.len() as u64;
                let item = row["data"]["item"].as_str().unwrap().to_owned();
                case.probes.insert(tag, Probe { candidate: probe(id, position), yaw: action["yaw"].as_f64().unwrap_or(0.0) as f32, item });
            }
            "player_attack" => {
                let data = &row["data"];
                let tick = row["tick"].as_i64().unwrap();
                let definition = suite["scenarios"].as_array().unwrap().iter().find(|c| c["id"] == scenario.as_str()).unwrap();
                let action = definition["actions"].as_array().unwrap().iter().find(|a| a["type"] == "player_attack" && a["tick"] == tick).unwrap();
                let probe = &case.probes[data["probe"].as_str().unwrap()];
                let (damage, speed) = attributes(&probe.item);
                let at = format!("{scenario} tick {tick} attack");
                exact(&data["attack_damage"], damage, &format!("{at} attack damage"));
                exact(&data["attack_speed"], speed, &format!("{at} attack speed"));
                // `getCurrentItemAttackStrengthDelay` and `getAttackStrengthScale(0.5)`.
                let delay = (1.0 / speed * 20.0) as f32;
                let ticker = action["attack_ticker"].as_i64().unwrap_or(100) as f32;
                let strength = ((ticker + 0.5) / delay).clamp(0.0, 1.0);
                exact(&data["strength"], f64::from(strength), &format!("{at} strength"));
                let sprinting = action["sprinting"].as_bool().unwrap_or(false);
                let on_ground = action["on_ground"].as_bool().unwrap_or(true);
                let fall_distance = action["fall_distance"].as_f64().unwrap_or(0.0);
                let player_speed = action["speed"].as_f64().unwrap_or(0.1) as f32;
                // The probe's known movement is zero.
                let sweep_speed = f64::from(player_speed) * 2.5;
                let attack = PlayerAttack {
                    player_id: probe.candidate.id,
                    position: probe.candidate.position,
                    yaw: probe.yaw,
                    attack_damage: damage,
                    strength,
                    sprinting,
                    can_critical: fall_distance > 0.0 && !on_ground && !sprinting,
                    can_sweep: on_ground && 0.0 < sweep_speed * sweep_speed && probe.item.ends_with("_sword"),
                };
                let target = case.ids[data["target"].as_str().unwrap()];
                let result = case.world.player_attack(&attack, target);
                assert!(result.hurt, "{at} hurts");
                attacks += 1;
                swept += result.swept.len();
                crits += usize::from(result.critical);
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = data["entities"].as_object().unwrap();
                if tick == 0 {
                    case.world.set_game_time(data["game_time"].as_i64().unwrap());
                    let mut order: Vec<(&String, &Value)> = observed.iter().map(|(tag, states)| (tag, &states[0])).collect();
                    order.sort_by_key(|(_, e)| e["entity_numeric_id"].as_u64().unwrap());
                    for (tag, e) in order {
                        let position = DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"]));
                        let no_ai = e["no_ai"].as_bool().unwrap();
                        case.world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
                        let id = match e["type"].as_str().unwrap() {
                            "minecraft:cow" => {
                                let mut cow = Cow::new(position);
                                cow.persistence_required = true;
                                cow.body.on_ground = e["on_ground"].as_bool().unwrap();
                                case.world.spawn_cow(cow, no_ai)
                            }
                            "minecraft:pig" => {
                                let mut pig = Pig::new(position);
                                pig.persistence_required = true;
                                pig.body.on_ground = e["on_ground"].as_bool().unwrap();
                                case.world.spawn_pig(pig, no_ai)
                            }
                            "minecraft:sheep" => {
                                let mut sheep = Sheep::default();
                                sheep.persistence_required = true;
                                let id = case.world.spawn_sheep(sheep, position, no_ai);
                                case.world.sheep_mut(id).unwrap().body.on_ground = e["on_ground"].as_bool().unwrap();
                                id
                            }
                            "minecraft:zombie" => {
                                let mut zombie = Zombie::new(position);
                                zombie.persistence_required = true;
                                zombie.body.on_ground = e["on_ground"].as_bool().unwrap();
                                assert!(no_ai);
                                case.world.spawn_zombie(zombie, true)
                            }
                            "minecraft:creeper" => {
                                let mut creeper = Creeper::new(position);
                                creeper.persistence_required = true;
                                creeper.body.on_ground = e["on_ground"].as_bool().unwrap();
                                case.world.spawn_creeper(creeper, no_ai)
                            }
                            other => panic!("unexpected {other}"),
                        };
                        case.ids.insert(tag.clone(), id);
                    }
                    continue;
                }
                let players: Vec<PlayerCandidate> = case.probes.values().map(|p| p.candidate).collect();
                let ticks = observed.iter().find_map(|(tag, states)| {
                    let e = states.as_array().unwrap().first()?;
                    Some(e["entity_tick_count"].as_i64().unwrap() > i64::from(tick_count(&case.world, case.ids[tag])?))
                });
                if ticks.unwrap_or(true) {
                    case.world.tick_with_players(&mut scene, &players);
                } else {
                    case.world.set_game_time(data["game_time"].as_i64().unwrap());
                }
                assert_eq!(case.world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                for (tag, states) in observed {
                    let id = case.ids[tag];
                    let states = states.as_array().unwrap();
                    let Some(state) = mob_state(&case.world, id) else {
                        assert!(states.is_empty(), "{scenario} tick {tick} {tag} presence");
                        continue;
                    };
                    let e = &states[0];
                    let at = format!("{scenario} tick {tick} {tag}");
                    for (field, actual) in [("x", state.position.x), ("y", state.position.y), ("z", state.position.z), ("vx", state.velocity.x), ("vy", state.velocity.y), ("vz", state.velocity.z), ("health", f64::from(state.health))] {
                        exact(&e[field], actual, &format!("{at} {field}"));
                    }
                    assert_eq!(e["entity_tick_count"], state.tick_count, "{at} tick count");
                    assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), state.random, "{at} random");
                    let goals: Vec<&str> = e["running_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                    assert_eq!(goals, state.goals, "{at} goals");
                    if let Some(target) = state.targeted {
                        assert_eq!(e.get("target_uuid").is_some_and(|t| !t.is_null()), target, "{at} target");
                    }
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert_eq!(attacks, 7, "seven attacks");
    println!("{frames} exact player-attack frames matched ({attacks} attacks, {crits} critical, {swept} mobs swept)");
}

struct MobState {
    position: DVec3,
    velocity: DVec3,
    health: f32,
    tick_count: i32,
    random: u64,
    goals: Vec<&'static str>,
    targeted: Option<bool>,
}

fn tick_count(world: &EntityWorld, id: u64) -> Option<i32> {
    mob_state(world, id).map(|s| s.tick_count)
}

fn mob_state(world: &EntityWorld, id: u64) -> Option<MobState> {
    if let Some(e) = world.cows().iter().find(|e| e.id == id) {
        return Some(MobState { position: e.cow.body.position, velocity: e.cow.body.velocity, health: e.cow.health, tick_count: e.tick_count, random: e.random.raw_state(), goals: e.running_goals(), targeted: None });
    }
    if let Some(e) = world.pigs().iter().find(|e| e.id == id) {
        return Some(MobState { position: e.pig.body.position, velocity: e.pig.body.velocity, health: e.pig.health, tick_count: e.tick_count, random: e.random.raw_state(), goals: e.running_goals(), targeted: None });
    }
    if let Some(e) = world.sheep().iter().find(|e| e.id == id) {
        return Some(MobState { position: e.body.position, velocity: e.body.velocity, health: e.health, tick_count: e.tick_count, random: e.random.raw_state(), goals: e.running_goals(), targeted: None });
    }
    if let Some(e) = world.zombies().iter().find(|e| e.id == id) {
        return Some(MobState { position: e.zombie.body.position, velocity: e.zombie.body.velocity, health: e.zombie.health, tick_count: e.tick_count, random: e.random.raw_state(), goals: Vec::new(), targeted: None });
    }
    if let Some(e) = world.creepers().iter().find(|e| e.id == id) {
        return Some(MobState {
            position: e.creeper.body.position,
            velocity: e.creeper.body.velocity,
            health: e.creeper.health,
            tick_count: e.tick_count,
            random: e.random.raw_state(),
            goals: e.ai.running_goals(),
            targeted: Some(e.ai.state.target().is_some()),
        });
    }
    None
}
