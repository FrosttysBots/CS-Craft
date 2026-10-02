//! Exact horse and donkey AI against pinned Minecraft 26.3
//! (`scenarios/mobs/horse-ai.json`): a horse, a donkey and a foal stroll,
//! graze, rear, look about, follow their parent and are tempted by a
//! golden carrot, then the horse and the donkey are hurt. Each tick
//! compares their movement, rotations, goals, navigation, randoms, sounds
//! and `AbstractHorse` state (flags, counters, animations, temper and
//! `RandomStandGoal`'s interval).
#[path = "support/sounds.rs"]
mod sounds;
use glam::DVec3;
use minecraftoss_entities::{
    cow::Cow,
    horse::{HorseKind, HorseState},
    tempt::PlayerCandidate,
    world::{DamageSourceKind, EntityWorld},
};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

#[derive(Default)]
struct Scene(BTreeMap<Pos, Block>);
impl World for Scene {
    fn step_sound(&self, pos: Pos) -> Option<(String, f32, f32)> {
        sounds::step_sound(&self.0.get(&pos)?.id)
    }
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
        !self.0.keys().any(|&(bx, by, bz)| bx == x && bz == z && by >= y)
    }
    fn light_path_cost(&self, _pos: Pos) -> f32 {
        // The pen floor lies open to the noon sky: level 15, whose magic
        // value is 1, less a half.
        0.5
    }
}

fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn exact(expected: &Value, actual: f64, label: &str) {
    assert_eq!(actual.to_bits(), number(expected).to_bits(), "{label}: ours {actual} vanilla {}", number(expected));
}

fn main() {
    let path = env::args().nth(1).expect("usage: check_horse_ai TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut player: Option<PlayerCandidate> = None;
    let (mut frames, mut heard, mut hurts) = (0, 0, 0);
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "complete" => complete = true,
            "entity_set_random_seed" => {
                let seed = row["data"]["seed"].as_str().unwrap().parse::<i64>().unwrap();
                let tag = row["data"]["tag"].as_str().unwrap();
                world.cow_mut(ids[tag]).unwrap().set_random_seed(seed);
            }
            "entity_hurt" => {
                let data = &row["data"];
                assert_eq!(data["source"], "minecraft:generic");
                let tag = data["tag"].as_str().unwrap();
                let entity = world.cow_mut(ids[tag]).unwrap();
                exact(&data["health_before"], f64::from(entity.cow.health), &format!("{tag} health before hurt"));
                let result = entity.hurt(number(&data["amount"]) as f32, DamageSourceKind::Generic);
                assert_eq!(result.applied, data["applied"].as_bool().unwrap(), "{tag} hurt applied");
                exact(&data["health_after"], f64::from(entity.cow.health), &format!("{tag} health after hurt"));
                hurts += 1;
            }
            "player_probe" => {
                let item = row["data"]["item"].as_str().unwrap();
                assert!(matches!(item, "minecraft:air" | "minecraft:golden_carrot"));
                let pos = &row["data"]["pos"];
                player = Some(PlayerCandidate {
                    id: 0,
                    position: DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap()),
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
                    main_hand_horse_tempt: item == "minecraft:golden_carrot",
                    offhand_horse_tempt: false,
                    alive: true,
                    spectator: false,
                    attackable: false,
                });
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let entities = row["data"]["entities"].as_object().unwrap();
                if tick == 0 {
                    for (key, value) in row["data"]["blocks"].as_object().unwrap() {
                        let pos: Vec<i32> = key.split(',').map(|part| part.parse().unwrap()).collect();
                        let id = value["id"].as_str().unwrap();
                        if id != "minecraft:air" {
                            scene.set_block((pos[0], pos[1], pos[2]), Some(Block::new(id)));
                        }
                    }
                    // The mobs as summoned, in the order vanilla made them.
                    let mut order: Vec<(&String, &Value)> = entities.iter().map(|(tag, s)| (tag, &s[0])).collect();
                    order.sort_by_key(|(_, e)| e["entity_numeric_id"].as_i64().unwrap());
                    for (tag, e) in order {
                        let kind = match e["type"].as_str().unwrap() {
                            "minecraft:horse" => HorseKind::Horse,
                            "minecraft:donkey" => HorseKind::Donkey,
                            other => panic!("unexpected {other}"),
                        };
                        let mut cow = Cow::new(DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"])));
                        cow.body.velocity = DVec3::new(number(&e["vx"]), number(&e["vy"]), number(&e["vz"]));
                        cow.body.on_ground = e["on_ground"].as_bool().unwrap();
                        cow.yaw = number(&e["yaw"]) as f32;
                        cow.age.ticks = e["age"].as_i64().unwrap() as i32;
                        cow.health = number(&e["health"]) as f32;
                        cow.persistence_required = true;
                        let mut horse = HorseState::new(kind);
                        horse.variant = e["variant"].as_i64().unwrap_or(0) as i32;
                        let id = world.spawn_horse(cow, horse, false);
                        assert_eq!(id as i64, e["entity_numeric_id"].as_i64().unwrap(), "{tag} numeric id");
                        ids.insert(tag.clone(), id);
                    }
                    continue;
                }
                assert_eq!(tick, frames + 1);
                world.tick_with_players(&mut scene, &player.into_iter().collect::<Vec<_>>());
                heard += sounds::check(&mut world, &row["data"], &format!("tick {tick}"), &[]);
                for (tag, states) in entities {
                    let at = format!("tick {tick} {tag}");
                    let e = &states[0];
                    let entity = world.cows().iter().find(|c| c.id == ids[tag]).unwrap();
                    let cow = &entity.cow;
                    let horse = entity.horse.as_ref().unwrap();
                    assert_eq!(cow.age.ticks as i64, e["age"].as_i64().unwrap(), "{at} age");
                    exact(&e["health"], f64::from(cow.health), &format!("{at} health"));
                    exact(&e["eye_y"], cow.body.position.y + f64::from(horse.kind.eye_height(cow.age.baby())), &format!("{at} eye Y"));
                    let goals: Vec<&str> = e["running_goals"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
                    assert_eq!(entity.running_goals(), goals, "{at} goals");
                    for goal in goals {
                        *seen.entry(format!("{tag} {goal}")).or_default() += 1;
                    }
                    assert_eq!(entity.tick_count as i64, e["entity_tick_count"].as_i64().unwrap(), "{at} entity ticks");
                    assert_eq!(entity.ambient_sound_time as i64, e["ambient_sound_time"].as_i64().unwrap(), "{at} ambient timer");
                    if e.get("no_action_time").is_some() {
                        assert_eq!(entity.no_action_time as i64, e["no_action_time"].as_i64().unwrap(), "{at} no-action timer");
                    }
                    assert_eq!(entity.jumping, e["jumping"].as_bool().unwrap(), "{at} jumping");
                    for (name, actual) in [
                        ("x", cow.body.position.x),
                        ("y", cow.body.position.y),
                        ("z", cow.body.position.z),
                        ("vx", cow.body.velocity.x),
                        ("vy", cow.body.velocity.y),
                        ("vz", cow.body.velocity.z),
                        ("yaw", f64::from(cow.yaw)),
                        ("speed", f64::from(cow.speed)),
                        ("head_yaw", f64::from(entity.look_control.head_yaw)),
                        ("body_yaw", f64::from(entity.body_rotation.body_yaw)),
                        ("pitch", f64::from(entity.look_control.pitch)),
                        ("move_control_x", cow.move_control.wanted.x),
                        ("move_control_y", cow.move_control.wanted.y),
                        ("move_control_z", cow.move_control.wanted.z),
                        ("look_control_x", entity.look_control.wanted.x),
                        ("look_control_y", entity.look_control.wanted.y),
                        ("look_control_z", entity.look_control.wanted.z),
                    ] {
                        exact(&e[name], actual, &format!("{at} {name}"));
                    }
                    assert_eq!(cow.body.on_ground, e["on_ground"].as_bool().unwrap(), "{at} on ground");
                    assert_eq!(cow.move_control.has_wanted(), e["move_control_wanted"].as_bool().unwrap(), "{at} move wanted");
                    assert_eq!(entity.look_control.cooldown > 0, e["look_control_wanted"].as_bool().unwrap(), "{at} look wanted");
                    assert_eq!(cow.navigation.is_done(), e["navigation_done"].as_bool().unwrap(), "{at} navigation done");
                    let nodes: Vec<(i32, i32, i32)> = e["path_nodes"].as_array().unwrap().iter().map(|p| (p[0].as_i64().unwrap() as i32, p[1].as_i64().unwrap() as i32, p[2].as_i64().unwrap() as i32)).collect();
                    assert_eq!(cow.navigation.nodes, nodes, "{at} path nodes");
                    // `AbstractHorse`.
                    for (name, ours) in [
                        ("tame", horse.tame),
                        ("bred", horse.bred),
                        ("eating", horse.eating),
                        ("standing", horse.standing),
                        ("mouth_open", horse.mouth_open),
                    ] {
                        assert_eq!(ours, e[name].as_bool().unwrap(), "{at} {name}");
                    }
                    for (name, ours) in [
                        ("eatingCounter", horse.eating_counter),
                        ("mouthCounter", horse.mouth_counter),
                        ("standCounter", horse.stand_counter),
                        ("tailCounter", horse.tail_counter),
                        ("sprintCounter", horse.sprint_counter),
                        ("temper", horse.temper),
                        ("next_stand", horse.next_stand),
                    ] {
                        assert_eq!(i64::from(ours), e[name].as_i64().unwrap(), "{at} {name}");
                    }
                    for (name, ours) in [
                        ("eatAnim", horse.eat_anim),
                        ("eatAnimO", horse.eat_anim_o),
                        ("standAnim", horse.stand_anim),
                        ("standAnimO", horse.stand_anim_o),
                        ("mouthAnim", horse.mouth_anim),
                        ("mouthAnimO", horse.mouth_anim_o),
                    ] {
                        exact(&e[name], f64::from(ours), &format!("{at} {name}"));
                    }
                    if horse.eating {
                        *seen.entry(format!("{tag} grazing")).or_default() += 1;
                    }
                    if horse.standing {
                        *seen.entry(format!("{tag} rearing")).or_default() += 1;
                    }
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert!(seen.keys().any(|k| k.ends_with("grazing")), "they graze");
    println!("{frames} exact horse frames matched ({hurts} hurts, {heard} sounds; {seen:?})");
}
