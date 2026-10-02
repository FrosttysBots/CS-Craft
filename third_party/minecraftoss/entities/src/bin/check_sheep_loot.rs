//! Compare sheep death loot against a repeatable pinned server trace.
use minecraftoss_entities::{
    loot::{EntityLootBook, EntityLootContext},
    sheep::DYE_NAMES,
};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs, path::Path};

fn exact(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn main() {
    let mut args = env::args().skip(1);
    let trace = fs::read_to_string(
        args.next()
            .expect("usage: check_sheep_loot TRACE.jsonl COMMON.jar"),
    )
    .unwrap();
    let jar = args.next().expect("common data JAR");
    let mut book = EntityLootBook::from_jar(Path::new(&jar), 0).unwrap();
    let mut contexts = BTreeMap::<String, (bool, u8, bool)>::new();
    let mut totals = BTreeMap::<String, u32>::new();
    let mut deaths = 0;
    let mut frames = 0;
    let mut complete = false;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        if row["type"] == "complete" {
            complete = true;
        }
        if row["type"] == "entity_hurt" {
            let data = &row["data"];
            let tag = data["tag"].as_str().unwrap();
            if exact(&data["health_before"]) > 0.0 && !data["alive"].as_bool().unwrap() {
                deaths += 1;
                let (baby, color, sheared) = contexts[tag];
                if !baby {
                    let drops = book
                        .roll(
                            "minecraft:sheep",
                            EntityLootContext {
                                sheep_color: Some(color),
                                sheep_sheared: sheared,
                                ..Default::default()
                            },
                        )
                        .expect("supported sheep loot context");
                    for stack in drops {
                        *totals.entry(stack.id).or_default() += stack.count as u32;
                    }
                }
            }
        }
        if row["type"] != "snapshot" {
            continue;
        }
        let tick = row["tick"].as_i64().unwrap();
        if tick == 0 {
            for (tag, group) in row["data"]["entities"].as_object().unwrap() {
                let sheep = &group.as_array().unwrap()[0];
                let color = DYE_NAMES
                    .iter()
                    .position(|name| Some(*name) == sheep["sheep_color"].as_str())
                    .unwrap() as u8;
                contexts.insert(
                    tag.to_owned(),
                    (
                        sheep["age"].as_i64().unwrap() < 0,
                        color,
                        sheep["sheared"].as_bool().unwrap(),
                    ),
                );
            }
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
        frames += 1;
    }
    assert!(complete && deaths == 4 && frames == 9);
    println!("{deaths} sheep deaths and {frames} item-count frames matched exactly; item trajectories, fire and looting excluded");
}
