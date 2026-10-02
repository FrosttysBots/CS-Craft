//! Exact pinned 26.3 monster death loot gate
//! (`scenarios/mobs/monster-loot.json`, `mob_drops` on): zombies, a baby
//! zombie and drowned killed by a probe player's axe, by generic damage, by
//! burning and by generic damage 99 and 100 ticks after the player's hit
//! (the `lastHurtByPlayerMemoryTime` boundary); bow skeletons killed by the
//! player (the 8.5% bow roll, a guaranteed bow with random wear), by
//! generic damage (a preserved bow drops unworn) and by burning; creepers
//! killed by the player, by generic damage and by a skeleton's arrow (a
//! music disc); and a charged creeper's blast (the first victim with a head
//! drops it). The entity world reports each death
//! ([`EntityWorld::take_deaths`]) and the loot book rolls it with the
//! world seed's named sequences; every tick compares the level's item
//! counts, the dropped bows' wear, the experience left, and each mob's
//! health, fire, player memory, tick count and random. Item motion comes from the level's
//! unseeded random and is not compared.
use glam::DVec3;
use minecraftoss_entities::{
    creeper::Creeper,
    loot::EntityLootBook,
    pig::Pig,
    projectile::Arrow,
    skeleton::Skeleton,
    world::{EntityWorld, PlayerAttack},
    zombie::{Zombie, ZombieKind},
};
use minecraftoss_player::{rng::LegacyRandom, Block, Pos, World};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    path::Path,
};

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

fn exact(observed: &Value, actual: f64, label: &str) {
    assert_eq!(actual.to_bits(), number(observed).to_bits(), "{label}: {actual} vs {}", number(observed));
}

fn position(e: &Value) -> DVec3 {
    DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"]))
}

/// A setup summon's main-hand drop chance (`drop_chances:{mainhand:…f}`).
fn main_hand_drop_chance(definition: &Value, tag: &str) -> Option<f32> {
    let wanted = format!("Tags:[\"{tag}\"]");
    definition["setup"].as_array()?.iter().filter_map(Value::as_str).find(|c| c.starts_with("summon") && c.contains(&wanted)).and_then(|c| {
        let rest = &c[c.find("drop_chances:{mainhand:")? + "drop_chances:{mainhand:".len()..];
        rest[..rest.find('f')?].parse().ok()
    })
}

/// What the check follows of one mob.
struct Mob {
    kind: &'static str,
    id: u64,
}

/// The world's view of a mob: health, fire, player memory, tick count and
/// random, or `None` once it is gone.
fn state(world: &EntityWorld, mob: &Mob) -> Option<(f32, i32, i32, i32, u64)> {
    match mob.kind {
        "zombie" => world.zombies().iter().find(|e| e.id == mob.id).map(|e| (e.zombie.health, e.zombie.body.fire_ticks, e.zombie.damage.player_memory, e.tick_count, e.random.raw_state())),
        "skeleton" => world.skeletons().iter().find(|e| e.id == mob.id).map(|e| (e.skeleton.health, e.skeleton.body.fire_ticks, e.skeleton.damage.player_memory, e.tick_count, e.random.raw_state())),
        "creeper" => world.creepers().iter().find(|e| e.id == mob.id).map(|e| (e.creeper.health, 0, e.creeper.damage.player_memory, e.tick_count, e.random.raw_state())),
        "pig" => world.pigs().iter().find(|e| e.id == mob.id).map(|e| (e.pig.health, 0, e.pig.damage.player_memory, e.tick_count, e.random.raw_state())),
        kind => panic!("unexpected {kind}"),
    }
}

fn set_seed(world: &mut EntityWorld, mob: &Mob, seed: u64) {
    let random = LegacyRandom::new(seed);
    match mob.kind {
        "zombie" => world.zombie_mut(mob.id).unwrap().random = random,
        "skeleton" => world.skeleton_mut(mob.id).unwrap().random = random,
        "creeper" => world.creeper_mut(mob.id).unwrap().random = random,
        "pig" => world.pig_mut(mob.id).unwrap().random = random,
        kind => panic!("unexpected {kind}"),
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let trace = fs::read_to_string(args.next().expect("usage: check_monster_loot TRACE.jsonl COMMON.jar")).unwrap();
    let jar = args.next().expect("common data JAR");
    // One level: the named loot sequences and the items persist across
    // the scenarios.
    let mut book = EntityLootBook::from_jar(Path::new(&jar), 0).unwrap();
    let mut totals = BTreeMap::<String, u64>::new();
    let mut experience: i64 = 0;
    let mut bows: Vec<u32> = Vec::new();
    let mut suite = Value::Null;
    let mut scenario = String::new();
    let mut definition = Value::Null;
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut mobs: BTreeMap<String, Mob> = BTreeMap::new();
    let mut probes: HashMap<String, (DVec3, f32)> = HashMap::new();
    let (mut frames, mut deaths, mut player_kills) = (0, 0, 0);
    let mut complete = false;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error", "{row}");
        match row["type"].as_str().unwrap() {
            "manifest" => suite = row["data"]["suite"].clone(),
            "complete" => complete = true,
            "scenario_start" => {
                scenario = row["scenario"].as_str().unwrap().to_owned();
                definition = suite["scenarios"].as_array().unwrap().iter().find(|c| c["id"] == scenario.as_str()).unwrap().clone();
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
                world.set_mob_griefing(false);
                mobs.clear();
                probes.clear();
            }
            "entity_set_random_seed" => {
                let mob = &mobs[row["data"]["tag"].as_str().unwrap()];
                set_seed(&mut world, mob, row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "player_probe" => {
                let data = &row["data"];
                let pos = &data["pos"];
                let at = DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap());
                probes.insert(data["tag"].as_str().unwrap().to_owned(), (at, data["yaw"].as_f64().unwrap_or(0.0) as f32));
            }
            "player_attack" => {
                let data = &row["data"];
                let (position, yaw) = probes[data["probe"].as_str().unwrap()];
                let attack = PlayerAttack {
                    player_id: 1_000_000,
                    position,
                    yaw,
                    attack_damage: number(&data["attack_damage"]),
                    strength: number(&data["strength"]) as f32,
                    sprinting: false,
                    can_critical: false,
                    can_sweep: false,
                };
                let mob = &mobs[data["target"].as_str().unwrap()];
                let at = format!("{scenario} tick {} attack {}", row["tick"], data["target"]);
                exact(&data["health_before"], f64::from(state(&world, mob).unwrap().0), &format!("{at} health before"));
                let result = world.player_attack(&attack, mob.id);
                assert!(result.hurt, "{at} hurts");
                exact(&data["health_after"], f64::from(state(&world, mob).unwrap().0), &format!("{at} health after"));
                player_kills += usize::from(result.died);
            }
            "entity_hurt" => {
                let data = &row["data"];
                let mob = &mobs[data["tag"].as_str().unwrap()];
                let amount = number(&data["amount"]) as f32;
                let at = format!("{scenario} tick {} hurt {}", row["tick"], data["tag"]);
                exact(&data["health_before"], f64::from(state(&world, mob).unwrap().0), &format!("{at} health before"));
                // Generic damage bypasses armor.
                let result = match mob.kind {
                    "zombie" => world.zombie_mut(mob.id).unwrap().hurt(amount),
                    "skeleton" => world.skeleton_mut(mob.id).unwrap().hurt(amount),
                    "creeper" => world.creeper_mut(mob.id).unwrap().hurt(amount),
                    kind => panic!("unexpected {kind}"),
                };
                assert_eq!(result.applied, data["applied"].as_bool().unwrap(), "{at} applied");
                exact(&data["health_after"], f64::from(state(&world, mob).unwrap().0), &format!("{at} health after"));
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = data["entities"].as_object().unwrap();
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    // Summon order: the observed tags follow it.
                    for tag in definition["observe"]["entities"].as_array().unwrap() {
                        let tag = tag.as_str().unwrap();
                        let e = &observed[tag][0];
                        let health = number(&e["health"]) as f32;
                        let fire = e["remaining_fire_ticks"].as_i64().unwrap() as i32;
                        let mob = match e["type"].as_str().unwrap() {
                            kind @ ("minecraft:zombie" | "minecraft:drowned") => {
                                let mut zombie = Zombie::new(position(e));
                                if kind == "minecraft:drowned" {
                                    zombie.kind = ZombieKind::Drowned;
                                }
                                zombie.set_baby(e["zombie_baby"].as_bool().unwrap_or(false));
                                zombie.body.on_ground = e["on_ground"].as_bool().unwrap();
                                zombie.health = health;
                                zombie.body.fire_ticks = fire;
                                zombie.persistence_required = true;
                                Mob { kind: "zombie", id: world.spawn_zombie(zombie, true) }
                            }
                            "minecraft:skeleton" => {
                                let mut skeleton = Skeleton::new(position(e));
                                skeleton.body.on_ground = e["on_ground"].as_bool().unwrap();
                                skeleton.health = health;
                                skeleton.body.fire_ticks = fire;
                                skeleton.persistence_required = true;
                                // Only the archer goes without its bow.
                                skeleton.holds_bow = definition["setup"].as_array().unwrap().iter().any(|c| c.as_str().unwrap().contains(&format!("tag={tag},")) && c.as_str().unwrap().contains("minecraft:bow"));
                                if let Some(chance) = main_hand_drop_chance(&definition, tag) {
                                    skeleton.bow_drop_chance = chance;
                                }
                                Mob { kind: "skeleton", id: world.spawn_skeleton(skeleton, true) }
                            }
                            "minecraft:creeper" => {
                                let mut creeper = Creeper::new(position(e));
                                creeper.body.on_ground = e["on_ground"].as_bool().unwrap();
                                creeper.health = health;
                                creeper.persistence_required = true;
                                if e.get("creeper_swell").is_some() {
                                    creeper.old_swell = e["creeper_old_swell"].as_i64().unwrap() as i32;
                                    creeper.swell = e["creeper_swell"].as_i64().unwrap() as i32;
                                    creeper.max_swell = e["creeper_max_swell"].as_i64().unwrap() as i32;
                                    creeper.explosion_radius = e["creeper_explosion_radius"].as_i64().unwrap() as i32;
                                    creeper.swell_dir = e["creeper_swell_dir"].as_i64().unwrap() as i32;
                                    creeper.ignited = e["creeper_ignited"].as_bool().unwrap();
                                    creeper.powered = e["creeper_powered"].as_bool().unwrap();
                                }
                                Mob { kind: "creeper", id: world.spawn_creeper(creeper, true) }
                            }
                            "minecraft:pig" => {
                                let mut pig = Pig::new(position(e));
                                pig.body.on_ground = e["on_ground"].as_bool().unwrap();
                                pig.health = health;
                                pig.persistence_required = true;
                                Mob { kind: "pig", id: world.spawn_pig_no_ai(pig) }
                            }
                            kind => panic!("unexpected {kind}"),
                        };
                        mobs.insert(tag.to_owned(), mob);
                    }
                    // The summoned arrow, owned by the archer.
                    for arrow in data.get("entity_type_states").and_then(|s| s["minecraft:arrow"].as_array()).into_iter().flatten() {
                        let velocity = DVec3::new(number(&arrow["vx"]), number(&arrow["vy"]), number(&arrow["vz"]));
                        world.spawn_owned_arrow(mobs["archer"].id, Arrow::in_flight(position(arrow), velocity, LegacyRandom::new(0)));
                    }
                } else {
                    world.tick(&mut scene);
                    assert_eq!(world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                }
                let reported = world.take_deaths();
                deaths += reported.len();
                experience += reported.iter().map(|d| i64::from(d.experience)).sum::<i64>();
                for (stack, _) in book.death_drops(reported) {
                    if stack.id == "minecraft:bow" {
                        bows.push(stack.components.as_ref().and_then(|c| c["minecraft:damage"].as_u64()).unwrap_or(0) as u32);
                    }
                    *totals.entry(stack.id).or_default() += u64::from(stack.count);
                }
                let at = format!("{scenario} tick {tick}");
                let counts: BTreeMap<String, u64> = data["item_counts"].as_object().unwrap().iter().map(|(id, n)| (id.clone(), n.as_u64().unwrap())).collect();
                assert_eq!(counts, totals, "{at} item counts");
                assert_eq!(data["experience_total"].as_i64().unwrap(), experience, "{at} experience");
                let mut observed_bows: Vec<u32> = data["item_entities"].as_array().unwrap().iter().filter(|i| i["id"] == "minecraft:bow").map(|i| i["damage"].as_u64().unwrap() as u32).collect();
                observed_bows.sort_unstable();
                let mut ours = bows.clone();
                ours.sort_unstable();
                assert_eq!(observed_bows, ours, "{at} bows' wear");
                for (tag, mob) in &mobs {
                    let states = observed[tag].as_array().unwrap();
                    let ours = state(&world, mob);
                    assert_eq!(states.len(), usize::from(ours.is_some()), "{at} {tag} presence");
                    let Some((health, fire, memory, tick_count, random)) = ours else { continue };
                    let e = &states[0];
                    exact(&e["health"], f64::from(health), &format!("{at} {tag} health"));
                    assert_eq!(e["alive"], health > 0.0, "{at} {tag} alive");
                    assert_eq!(e["remaining_fire_ticks"], fire, "{at} {tag} fire");
                    assert_eq!(e["player_memory"], memory, "{at} {tag} player memory");
                    assert_eq!(e["entity_tick_count"], tick_count, "{at} {tag} tick count");
                    if let Some(observed) = e.get("random_state") {
                        assert_eq!(observed.as_str().unwrap().parse::<u64>().unwrap(), random, "{at} {tag} random");
                    }
                }
                let arrows: Vec<&Value> = data.get("entity_type_states").and_then(|s| s["minecraft:arrow"].as_array()).into_iter().flatten().collect();
                let ours: Vec<_> = world.arrows().iter().filter(|a| a.arrow.alive).collect();
                assert_eq!(arrows.len(), ours.len(), "{at} arrows");
                for (observed, ours) in arrows.iter().zip(&ours) {
                    for (field, actual) in [
                        ("x", ours.arrow.position.x),
                        ("y", ours.arrow.position.y),
                        ("z", ours.arrow.position.z),
                        ("vx", ours.arrow.velocity.x),
                        ("vy", ours.arrow.velocity.y),
                        ("vz", ours.arrow.velocity.z),
                    ] {
                        exact(&observed[field], actual, &format!("{at} arrow {field}"));
                    }
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert!(totals.contains_key("minecraft:copper_ingot"), "a drowned's player kill drops copper");
    assert!(totals.keys().any(|id| id.starts_with("minecraft:music_disc_")), "the skeleton's arrow earns a disc");
    assert!(totals.contains_key("minecraft:zombie_head"), "the charged blast knocks off a head");
    println!(
        "{frames} exact monster loot frames matched: {deaths} deaths ({player_kills} by the player's hits), {} items dropped, bows' wear {bows:?}, {experience} experience",
        totals.values().sum::<u64>()
    );
}
