//! Exact pinned 26.3 zombie-hits-player gate: a zombie with only
//! `ZombieAttackGoal` reaches a survival probe player and hits it once on
//! each difficulty (`scenarios/mobs/zombie-hits-player.json`). The zombie
//! runs in the entity world; its hit goes through `Player::hurt_by`
//! (`Player.hurtServer`: difficulty scaling, armor, the damage cooldown,
//! knockback, the hurt direction and food exhaustion). The probe never
//! ticks, so neither does the Rust player.
use glam::DVec3;
use minecraftoss_entities::{
    tempt::PlayerCandidate,
    world::{EntityWorld, PlayerHitKind},
    zombie::Zombie,
};
use minecraftoss_player::survival::{Armor, Difficulty};
use minecraftoss_player::{Block, HitFrom, IncomingHit, Player, Pos, World};
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
fn exact(observed: &Value, actual: f64, label: &str, scenario: &str, tick: i64) {
    assert_eq!(actual.to_bits(), number(observed).to_bits(), "{scenario} tick {tick} {label}: {actual} vs {}", number(observed));
}

/// One scenario's replay state.
struct Case {
    world: EntityWorld,
    scene: Scene,
    difficulty: Difficulty,
    probe: Option<(PlayerCandidate, Player, Armor)>,
    zombie_id: Option<u64>,
}

fn main() {
    let path = env::args().nth(1).expect("usage: check_zombie_player_hits TRACE.jsonl");
    let mut suite = Value::Null;
    let mut case: Option<Case> = None;
    let mut scenario = String::new();
    let (mut frames, mut hits) = (0, 0);
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
                let mut scene = Scene::default();
                let mut difficulty = Difficulty::Normal;
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
                        "difficulty" => difficulty = Difficulty::parse(parts[1]).unwrap(),
                        _ => {}
                    }
                }
                case = Some(Case { world: EntityWorld::default(), scene, difficulty, probe: None, zombie_id: None });
            }
            "entity_set_random_seed" => {
                let case = case.as_mut().unwrap();
                case.world.zombie_mut(case.zombie_id.unwrap()).unwrap().set_random_seed(row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "player_probe" => {
                let case = case.as_mut().unwrap();
                let pos = &row["data"]["pos"];
                let position = DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap());
                let definition = suite["scenarios"].as_array().unwrap().iter().find(|c| c["id"] == scenario.as_str()).unwrap();
                let action = definition["actions"].as_array().unwrap().iter().find(|a| a["type"] == "player_probe").unwrap();
                let armor = Armor {
                    value: Some((action["armor"].as_f64().unwrap_or(0.0) as f32, action["armor_toughness"].as_f64().unwrap_or(0.0) as f32)),
                };
                let candidate = PlayerCandidate {
                    id: 1,
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
                };
                let mut player = Player::new(position);
                player.yaw = action["yaw"].as_f64().unwrap_or(0.0);
                player.on_ground = false;
                case.probe = Some((candidate, player, armor));
            }
            "snapshot" => {
                let case = case.as_mut().unwrap();
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observed = data["entities"].as_object().unwrap().values().next().unwrap()[0].clone();
                if tick == 0 {
                    case.world.set_game_time(data["game_time"].as_i64().unwrap());
                    let mut zombie = Zombie::new(DVec3::new(number(&observed["x"]), number(&observed["y"]), number(&observed["z"])));
                    zombie.body.velocity = DVec3::new(number(&observed["vx"]), number(&observed["vy"]), number(&observed["vz"]));
                    zombie.body.on_ground = observed["on_ground"].as_bool().unwrap();
                    zombie.persistence_required = observed["persistence_required"].as_bool().unwrap();
                    case.world.set_next_entity_id(observed["entity_numeric_id"].as_u64().unwrap());
                    case.zombie_id = Some(case.world.spawn_zombie_pursuit(zombie));
                    continue;
                }
                let players: Vec<PlayerCandidate> = case.probe.iter().map(|p| p.0).collect();
                // A zombie whose chunk is not entity-ticking yet skips the
                // tick (its tick count holds); the world clock still runs.
                let ticked = case.world.zombies().iter().find(|e| Some(e.id) == case.zombie_id).unwrap().tick_count;
                if observed["entity_tick_count"].as_i64().unwrap() > i64::from(ticked) {
                    case.world.tick_with_players(&mut case.scene, &players);
                } else {
                    case.world.set_game_time(data["game_time"].as_i64().unwrap());
                }
                assert_eq!(case.world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                let entity = case.world.zombies().iter().find(|e| Some(e.id) == case.zombie_id).unwrap();
                for (field, actual) in [("x", entity.zombie.body.position.x), ("z", entity.zombie.body.position.z), ("vx", entity.zombie.body.velocity.x)] {
                    exact(&observed[field], actual, field, &scenario, tick);
                }
                let difficulty = case.difficulty;
                for hit in case.world.take_player_hits() {
                    let (_, player, armor) = case.probe.as_mut().unwrap();
                    let from = match hit.kind {
                        PlayerHitKind::Melee { attacker, .. } => HitFrom::Position(attacker),
                        PlayerHitKind::Arrow { velocity, .. } => HitFrom::Projectile(velocity),
                        PlayerHitKind::Explosion { .. } => HitFrom::Explosion,
                    };
                    let incoming = IncomingHit { damage: hit.damage, from, scales_with_difficulty: true, exhaustion: 0.1 };
                    if player.hurt_by(&incoming, difficulty, *armor, 0.0) {
                        hits += 1;
                    }
                }
                if let Some((_, player, _)) = &case.probe {
                    let probe = data["player_probes"].as_object().unwrap().values().next().unwrap();
                    exact(&probe["health"], f64::from(player.survival.health), "health", &scenario, tick);
                    exact(&probe["motion_x"], player.velocity.x, "motion x", &scenario, tick);
                    exact(&probe["motion_y"], player.velocity.y, "motion y", &scenario, tick);
                    exact(&probe["motion_z"], player.velocity.z, "motion z", &scenario, tick);
                    exact(&probe["hurt_dir"], f64::from(player.hurt_dir), "hurt dir", &scenario, tick);
                    exact(&probe["exhaustion"], f64::from(player.survival.food.exhaustion), "exhaustion", &scenario, tick);
                    assert_eq!(probe["hurt_time"], player.hurt_time, "{scenario} tick {tick} hurt time");
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert_eq!(hits, 3, "one hit per difficulty");
    println!("{frames} exact zombie-hits-player frames matched ({hits} hits)");
}
