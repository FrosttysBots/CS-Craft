//! Pig fatal damage and raw death loot against repeated pinned server traces.
use glam::DVec3;
use minecraftoss_entities::{
    loot::{EntityLootBook, EntityLootContext},
    pig::Pig,
    world::{DamageSourceKind, EntityWorld},
};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    path::Path,
};

struct EmptyWorld;
impl World for EmptyWorld {
    fn block(&self, _: Pos) -> Option<Block> {
        None
    }
    fn set_block(&mut self, _: Pos, _: Option<Block>) {}
}

fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn main() {
    let mut args = env::args().skip(1);
    let trace = fs::read_to_string(
        args.next()
            .expect("usage: check_pig_loot TRACE.jsonl COMMON.jar"),
    )
    .unwrap();
    let jar = args.next().expect("common data JAR");
    let mut loot = EntityLootBook::from_jar(Path::new(&jar), 0).unwrap();
    let mut world = EntityWorld::default();
    let mut ids = HashMap::<String, u64>::new();
    let mut totals = BTreeMap::<String, u32>::new();
    let mut deaths = 0;
    let mut frames = 0;
    let mut complete = false;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        match row["type"].as_str().unwrap() {
            "error" => panic!("reference error: {row}"),
            "complete" => complete = true,
            "entity_hurt" => {
                let data = &row["data"];
                let tag = data["tag"].as_str().unwrap();
                let entity = world.pig_mut(ids[tag]).unwrap();
                assert_eq!(
                    number(&data["health_before"]).to_bits(),
                    f64::from(entity.pig.health).to_bits()
                );
                let result = entity.hurt(number(&data["amount"]) as f32, DamageSourceKind::Generic);
                assert_eq!(result.applied, data["applied"].as_bool().unwrap());
                assert_eq!(
                    number(&data["health_after"]).to_bits(),
                    f64::from(entity.pig.health).to_bits()
                );
                if result.died {
                    deaths += 1;
                    if !entity.pig.age.baby() {
                        for stack in loot
                            .roll("minecraft:pig", EntityLootContext::default())
                            .expect("supported raw pig loot")
                        {
                            *totals.entry(stack.id).or_default() += stack.count as u32;
                        }
                    }
                }
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let groups = row["data"]["entities"].as_object().unwrap();
                if tick == 0 {
                    for (tag, states) in groups {
                        let state = &states[0];
                        let mut pig = Pig::new(DVec3::new(
                            number(&state["x"]),
                            number(&state["y"]),
                            number(&state["z"]),
                        ));
                        pig.age.ticks = state["age"].as_i64().unwrap() as i32;
                        pig.health = number(&state["health"]) as f32;
                        ids.insert(tag.clone(), world.spawn_pig_no_ai(pig));
                    }
                } else {
                    assert_eq!(tick, frames);
                    world.tick(&mut EmptyWorld);
                }
                let observed = row["data"]["item_counts"].as_object().unwrap();
                assert_eq!(totals.len(), observed.len(), "tick {tick} item kinds");
                for (id, count) in &totals {
                    assert_eq!(
                        *count as u64,
                        observed[id].as_u64().unwrap(),
                        "tick {tick} {id}"
                    );
                }
                for (tag, states) in groups {
                    let state = &states[0];
                    let pig = &world.pig_mut(ids[tag]).unwrap().pig;
                    assert_eq!(
                        number(&state["health"]).to_bits(),
                        f64::from(pig.health).to_bits(),
                        "tick {tick} {tag} health"
                    );
                    assert_eq!(state["alive"], pig.health > 0.0, "tick {tick} {tag} alive");
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete && deaths == 4 && frames == 9);
    println!("{deaths} pig deaths and {frames} health/loot frames matched exactly");
}
