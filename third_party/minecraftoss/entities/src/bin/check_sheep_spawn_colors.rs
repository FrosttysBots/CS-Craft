//! Check all seeded sheep color draws against the pinned Java spawn rule.
use minecraftoss_entities::sheep::{spawn_color, SheepSpawnClimate, DYE_NAMES};
use minecraftoss_player::rng::LegacyRandom;
use serde_json::Value;
use std::{env, fs};

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_sheep_spawn_colors TRACE.jsonl");
    let trace = fs::read_to_string(path).unwrap();
    let mut probes = 0;
    let mut draws = 0;
    let mut complete = false;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        if row["type"] == "complete" {
            complete = true;
        }
        if row["type"] != "sheep_color_probe" {
            continue;
        }
        let data = &row["data"];
        let climate = match data["biome"].as_str().unwrap() {
            "minecraft:plains" => SheepSpawnClimate::Temperate,
            "minecraft:desert" => SheepSpawnClimate::Warm,
            "minecraft:snowy_plains" => SheepSpawnClimate::Cold,
            other => panic!("unsupported probe biome {other}"),
        };
        let seed = data["seed"].as_str().unwrap().parse::<i64>().unwrap();
        let mut random = LegacyRandom::new(seed as u64);
        let colors = data["colors"].as_array().unwrap();
        assert_eq!(colors.len(), data["count"].as_u64().unwrap() as usize);
        for (index, color) in colors.iter().enumerate() {
            let got = DYE_NAMES[spawn_color(climate, &mut random) as usize];
            assert_eq!(
                got,
                color.as_str().unwrap(),
                "{climate:?} seed {seed} draw {index}"
            );
            draws += 1;
        }
        probes += 1;
    }
    assert!(complete && probes == 6 && draws == 3072);
    println!("{probes} probes and {draws} sheep color draws matched exactly");
}
