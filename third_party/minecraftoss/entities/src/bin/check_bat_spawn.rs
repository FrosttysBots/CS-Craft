//! Exact pinned 26.3 Bat.checkBatSpawnRules sampled with seeded Java RNG.
use minecraftoss_entities::bat::{Bat, BatSpawnContext};
use minecraftoss_player::rng::LegacyRandom;
use serde_json::Value;
use std::{env, fs};

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_bat_spawn TRACE.jsonl");
    let mut probes = 0;
    let mut samples = 0;
    let mut accepted = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "bat_spawn_rule_dark_cave_and_rejections"
            ),
            "complete" => complete = true,
            "bat_spawn_probe" => {
                let data = &row["data"];
                let context = BatSpawnContext {
                    y: data["pos"][1].as_i64().unwrap() as i32,
                    world_surface_y: data["surface_y"].as_i64().unwrap() as i32,
                    max_local_raw_brightness: data["brightness"].as_i64().unwrap() as i32,
                    below_bats_spawnable_on: data["below_bat_tag"].as_bool().unwrap(),
                    below_valid_spawn: data["below_valid_spawn"].as_bool().unwrap(),
                };
                let mut random = LegacyRandom::new(data["seed"].as_str().unwrap().parse().unwrap());
                let results = data["results"].as_array().unwrap();
                assert_eq!(results.len(), data["count"].as_u64().unwrap() as usize);
                for (index, result) in results.iter().enumerate() {
                    let actual = Bat::can_naturally_spawn(context, &mut random);
                    let expected = result.as_bool().unwrap();
                    assert_eq!(actual, expected, "probe {} sample {index}", data["pos"]);
                    samples += 1;
                    accepted += usize::from(actual);
                }
                probes += 1;
            }
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!(probes, 4);
    assert_eq!(samples, 1280);
    assert_eq!(accepted, 506);
    println!(
        "{samples} exact bat spawn decisions matched across {probes} probes ({accepted} accepted)"
    );
}
