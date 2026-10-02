//! Exact pinned 26.3 slime gate (`scenarios/mobs/slime-ai.json`): a NoAI
//! size-4 slime killed at once splits into NoAI slimes of half its size
//! when removed; slimes of size 1, 2 and 4 hop about walled pens; a size-2
//! slime hunts a probe player; a size-1 slime floats up through water.
//! Every tick compares position, motion, yaw, the goals, target, the cube
//! move control's heading, hop countdown and aggression, the landing state,
//! the mob random and every sound (jump and squish volumes and pitches).
use glam::DVec3;
use minecraftoss_entities::{slime::Slime, tempt::PlayerCandidate, world::EntityWorld};
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
    /// The pens are stone (`SoundType.STONE`: volume and pitch 1).
    fn step_sound(&self, pos: Pos) -> Option<(String, f32, f32)> {
        (self.0.get(&pos)?.id == "minecraft:stone").then(|| ("block.stone.step".to_owned(), 1.0, 1.0))
    }
}

fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn exact(observed: &Value, actual: f64, label: &str) {
    assert_eq!(actual.to_bits(), number(observed).to_bits(), "{label}: {actual} vs {}", number(observed));
}

/// A sound as the comparison sees it: event, position bits, volume bits, pitch bits.
type Heard = (String, [u64; 3], u32, u32);

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

fn main() {
    let path = env::args().nth(1).expect("usage: check_slime_ai TRACE.jsonl");
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut probes: Vec<PlayerCandidate> = Vec::new();
    let mut heard: Vec<Heard> = Vec::new();
    let mut scenario = String::new();
    // The x spans of this scenario's pens: slimes left over from earlier
    // scenarios keep hopping (and squishing) in theirs.
    let mut pens: Vec<(f64, f64)> = Vec::new();
    let (mut frames, mut sounds, mut hops, mut split_children) = (0, 0, 0, 0);
    let mut goals_seen = std::collections::BTreeSet::new();
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
                pens.clear();
                for command in definition["prepare"].as_array().unwrap() {
                    let parts: Vec<&str> = command.as_str().unwrap().split_whitespace().collect();
                    if parts[0] == "fill" {
                        let n = |i: usize| parts[i].parse::<i32>().unwrap();
                        pens.push((f64::from(n(1)), f64::from(n(4) + 1)));
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
                heard.clear();
            }
            "entity_set_random_seed" => {
                let id = ids[row["data"]["tag"].as_str().unwrap()];
                world.slime_mut(id).unwrap().set_random_seed(row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "entity_hurt" => {
                let data = &row["data"];
                let entity = world.slime_mut(ids[data["tag"].as_str().unwrap()]).unwrap();
                exact(&data["health_before"], f64::from(entity.slime.health), &format!("{scenario} health before"));
                let result = entity.hurt(number(&data["amount"]) as f32);
                assert_eq!(data["applied"], result.applied, "{scenario} hurt applied");
                heard.extend(world.take_sounds().into_iter().map(|s| (format!("minecraft:{}", s.event), [s.position.x.to_bits(), s.position.y.to_bits(), s.position.z.to_bits()], s.volume.to_bits(), s.pitch.to_bits())));
            }
            "player_probe" => {
                let pos = &row["data"]["pos"];
                probes.push(probe(1_000_000, DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap())));
            }
            "snapshot" if scenario == "slime_warmup" => {}
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    if scenario == "slime_split" {
                        // The summoned slime, from its type's states and the
                        // summon's size.
                        let definition = suite["scenarios"].as_array().unwrap().iter().find(|c| c["id"] == scenario.as_str()).unwrap();
                        let summon = definition["setup"][0].as_str().unwrap();
                        let size = summon.split("Size:").nth(1).unwrap().split(',').next().unwrap().parse::<i32>().unwrap() + 1;
                        let e = &data["entity_type_states"]["minecraft:slime"][0];
                        let mut slime = Slime::new(DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"])), size);
                        slime.persistence_required = true;
                        slime.body.on_ground = e["on_ground"].as_bool().unwrap();
                        world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
                        let id = world.spawn_slime(slime, true);
                        ids.insert("big".to_owned(), id);
                        continue;
                    }
                    let observed = data["entities"].as_object().unwrap();
                    let mut order: Vec<(&String, &Value)> = observed.iter().map(|(tag, states)| (tag, &states[0])).collect();
                    order.sort_by_key(|(_, e)| e["entity_numeric_id"].as_u64().unwrap());
                    for (tag, e) in order {
                        let mut slime = Slime::new(DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"])), e["cube_size"].as_i64().unwrap() as i32);
                        slime.persistence_required = true;
                        slime.body.on_ground = e["on_ground"].as_bool().unwrap();
                        slime.yaw = number(&e["yaw"]) as f32;
                        slime.was_on_ground = e["cube_was_on_ground"].as_bool().unwrap();
                        world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
                        let id = world.spawn_slime(slime, false);
                        // The move control's first heading comes from the
                        // constructor's unseeded yaw.
                        let entity = world.slime_mut(id).unwrap();
                        entity.ai.state.cube.y_rot = number(&e["cube_move_y_rot"]) as f32;
                        entity.ai.state.cube.jump_delay = e["cube_jump_delay"].as_i64().unwrap() as i32;
                        ids.insert(tag.clone(), id);
                    }
                    continue;
                }
                world.tick_with_players(&mut scene, &probes);
                assert_eq!(world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                heard.extend(world.take_sounds().into_iter().map(|s| (format!("minecraft:{}", s.event), [s.position.x.to_bits(), s.position.y.to_bits(), s.position.z.to_bits()], s.volume.to_bits(), s.pitch.to_bits())));
                if scenario == "slime_split" {
                    let vanilla = data["entity_type_states"]["minecraft:slime"].as_array().unwrap();
                    let ours = world.slimes();
                    assert_eq!(data["entity_type_counts"]["minecraft:slime"]["total"], ours.len(), "{scenario} tick {tick} slimes");
                    assert_eq!(vanilla.len(), ours.len(), "{scenario} tick {tick} slime states");
                    for (i, (v, e)) in vanilla.iter().zip(ours).enumerate() {
                        let at = format!("{scenario} tick {tick} slime {i}");
                        for (field, actual) in [("x", e.slime.body.position.x), ("y", e.slime.body.position.y), ("z", e.slime.body.position.z), ("health", f64::from(e.slime.health))] {
                            exact(&v[field], actual, &format!("{at} {field}"));
                        }
                    }
                    split_children = split_children.max(ours.iter().filter(|e| e.id != ids["big"]).count());
                } else {
                    for (tag, states) in data["entities"].as_object().unwrap() {
                        let e = &states[0];
                        let entity = world.slimes().iter().find(|s| s.id == ids[tag]).unwrap();
                        let at = format!("{scenario} tick {tick} {tag}");
                        let ai = &entity.ai;
                        let body = &entity.slime.body;
                        for (field, actual) in [
                            ("x", body.position.x),
                            ("y", body.position.y),
                            ("z", body.position.z),
                            ("vx", body.velocity.x),
                            ("vy", body.velocity.y),
                            ("vz", body.velocity.z),
                            ("health", f64::from(entity.slime.health)),
                            ("yaw", f64::from(ai.yaw)),
                            ("speed", f64::from(ai.speed)),
                            ("head_yaw", f64::from(ai.state.look_control.head_yaw)),
                            ("body_yaw", f64::from(ai.body_rotation.body_yaw)),
                            ("cube_move_y_rot", f64::from(ai.state.cube.y_rot)),
                            ("cube_target_squish", f64::from(entity.slime.target_squish)),
                            ("cube_squish", f64::from(entity.slime.squish)),
                        ] {
                            exact(&e[field], actual, &format!("{at} {field}"));
                        }
                        assert_eq!(e["on_ground"], body.on_ground, "{at} on ground");
                        assert_eq!(e["cube_size"], entity.slime.size, "{at} size");
                        assert_eq!(e["cube_was_on_ground"], entity.slime.was_on_ground, "{at} was on ground");
                        assert_eq!(e["cube_jump_delay"], ai.state.cube.jump_delay, "{at} jump delay");
                        assert_eq!(e["cube_aggressive"], ai.state.cube.aggressive, "{at} aggressive");
                        assert_eq!(e["entity_tick_count"], entity.tick_count, "{at} tick count");
                        assert_eq!(e["ambient_sound_time"], entity.ambient_sound_time, "{at} ambient time");
                        assert_eq!(e["no_action_time"], entity.no_action_time, "{at} idle time");
                        let goals: Vec<&str> = e["running_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                        assert_eq!(goals, ai.running_goals(), "{at} goals");
                        assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random");
                        goals_seen.extend(ai.running_goals());
                        assert_eq!(e.get("target_uuid").is_some_and(|t| !t.is_null()), ai.state.target().is_some(), "{at} target");
                    }
                }
                if let Some(events) = data["entity_sound_events"].as_array() {
                    let bits = |v: &Value| u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap();
                    let mut vanilla: Vec<Heard> = events
                        .iter()
                        .map(|e| {
                            let p = e["position"].as_array().unwrap();
                            let hex = |key: &str| u32::from_str_radix(e[key].as_str().unwrap(), 16).unwrap();
                            (e["id"].as_str().unwrap().to_owned(), [bits(&p[0]), bits(&p[1]), bits(&p[2])], hex("volume_bits"), hex("pitch_bits"))
                        })
                        .collect();
                    vanilla.retain(|s| {
                        let x = f64::from_bits(s.1[0]);
                        pens.iter().any(|&(from, to)| x >= from && x < to)
                    });
                    let mut ours = std::mem::take(&mut heard);
                    vanilla.sort();
                    ours.sort();
                    assert_eq!(ours, vanilla, "{scenario} tick {tick} sounds");
                    hops += vanilla.iter().filter(|s| s.0.contains(".jump")).count();
                    sounds += vanilla.len();
                } else {
                    heard.clear();
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    assert!(split_children >= 2, "the big slime splits");
    assert!(hops > 10, "the slimes hop");
    println!("{frames} exact slime frames matched ({hops} hops, {sounds} sounds, {split_children} split slimes; goals seen: {goals_seen:?})");
}
