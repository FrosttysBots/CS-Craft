//! Focused age-component gate. The Python harness must first validate complete
//! traces and prove reference repeatability. This checks only the named fields.
use minecraftoss_entities::age::Age;
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_age_trace TRACE.jsonl");
    let trace = fs::read_to_string(path).expect("read trace");
    let mut states: BTreeMap<String, Age> = BTreeMap::new();
    let mut checked = 0;
    let mut complete = false;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).expect("JSON record");
        assert_ne!(row["type"], "error", "reference error");
        if row["type"] == "complete" {
            complete = true;
        }
        if row["type"] != "snapshot" {
            continue;
        }
        let tick = row["tick"].as_i64().unwrap();
        if tick == 0 {
            states.clear();
        }
        for group in row["data"]["entities"]
            .as_object()
            .expect("entity groups")
            .values()
        {
            for entity in group.as_array().unwrap() {
                if entity.get("age").is_none() {
                    continue;
                }
                let id = entity["uuid"].as_str().unwrap().to_owned();
                let expected = Age {
                    ticks: entity["age"].as_i64().unwrap() as i32,
                    forced: entity["forced_age"].as_i64().unwrap() as i32,
                    locked: entity["age_locked"].as_bool().unwrap(),
                    forced_particle_ticks: entity["forced_age_timer"].as_i64().unwrap() as i32,
                    lock_particle_ticks: 0,
                };
                if tick == 0 {
                    states.insert(id, expected);
                } else {
                    let state = states.get_mut(&id).expect("existing initial entity");
                    state.tick(entity["alive"].as_bool().unwrap());
                    assert_eq!(
                        *state, expected,
                        "scenario {} tick {tick} UUID {id}",
                        row["scenario"]
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(complete && checked > 0, "incomplete or empty gate");
    println!("{checked} exact age-component transitions matched; movement/AI/interactions are outside this gate");
}
