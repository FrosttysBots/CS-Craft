//! Generic-damage, cooldown and death-timer gate against repeated 26.3 traces.
//! Run the Python exact vanilla-to-vanilla comparator before this checker.
use glam::DVec3;
use minecraftoss_entities::{cow::Cow, world::EntityWorld};
use minecraftoss_player::{Block, Pos, World};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

struct EmptyWorld;
impl World for EmptyWorld {
    fn block(&self, _: Pos) -> Option<Block> {
        None
    }
    fn set_block(&mut self, _: Pos, _: Option<Block>) {}
}
fn f64_exact(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}
fn check_health(expected: &Value, actual: f32, label: &str) {
    assert_eq!(
        actual.to_bits(),
        (f64_exact(expected) as f32).to_bits(),
        "{label}"
    );
}

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_cow_damage TRACE.jsonl");
    let trace = fs::read_to_string(path).expect("read trace");
    let mut world = EntityWorld::default();
    let mut ids = BTreeMap::<String, u64>::new();
    let mut current_tick = 0;
    let mut hits = 0;
    let mut frames = 0;
    let mut complete = false;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).expect("JSON record");
        assert_ne!(row["type"], "error", "reference error");
        if row["type"] == "complete" {
            complete = true;
        }
        if row["type"] == "entity_hurt" {
            let data = &row["data"];
            assert_eq!(data["source"], "minecraft:generic");
            let tag = data["tag"].as_str().unwrap();
            let cow = &mut world.cow_mut(ids[tag]).expect("tagged cow").cow;
            check_health(&data["health_before"], cow.health, "health before hit");
            let result = cow.hurt_generic(f64_exact(&data["amount"]) as f32);
            assert_eq!(
                result.applied,
                data["applied"].as_bool().unwrap(),
                "tick {} {tag} applied",
                row["tick"]
            );
            check_health(&data["health_after"], cow.health, "health after hit");
            assert_eq!(
                cow.health > 0.0,
                data["alive"].as_bool().unwrap(),
                "tick {} {tag} alive",
                row["tick"]
            );
            hits += 1;
        }
        if row["type"] != "snapshot" {
            continue;
        }
        let tick = row["tick"].as_i64().unwrap();
        if tick > 0 {
            assert_eq!(tick, current_tick + 1);
            world.tick(&mut EmptyWorld);
        }
        current_tick = tick;
        for (tag, group) in row["data"]["entities"].as_object().unwrap() {
            let entities = group.as_array().unwrap();
            if tick == 0 {
                assert_eq!(entities.len(), 1);
                let e = &entities[0];
                let mut cow = Cow::new(DVec3::new(
                    f64_exact(&e["x"]),
                    f64_exact(&e["y"]),
                    f64_exact(&e["z"]),
                ));
                cow.health = f64_exact(&e["health"]) as f32;
                cow.age.ticks = e["age"].as_i64().unwrap() as i32;
                cow.sync_dimensions();
                let id = world.spawn_cow(cow, e["no_ai"].as_bool().unwrap());
                ids.insert(tag.clone(), id);
            } else {
                let cow = world.cows().iter().find(|cow| cow.id == ids[tag]);
                assert_eq!(
                    cow.is_some(),
                    !entities.is_empty(),
                    "tick {tick} {tag} existence"
                );
                if let Some(cow) = cow {
                    let e = &entities[0];
                    check_health(
                        &e["health"],
                        cow.cow.health,
                        &format!("tick {tick} {tag} health"),
                    );
                    assert_eq!(
                        cow.cow.health > 0.0,
                        e["alive"].as_bool().unwrap(),
                        "tick {tick} {tag} alive"
                    );
                    frames += 1;
                }
            }
        }
    }
    assert!(
        complete && hits > 0 && frames > 0,
        "incomplete or empty gate"
    );
    println!("{hits} generic damage actions and {frames} cow state frames matched exactly; loot and attacker context excluded");
}
