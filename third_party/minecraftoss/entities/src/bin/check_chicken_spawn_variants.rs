//! Exact direct spawn-variant draws against pinned Minecraft 26.3 registries.
use glam::DVec3;
use minecraftoss_entities::chicken::{Chicken, ChickenSoundVariant, ChickenVariant};
use minecraftoss_player::rng::LegacyRandom;
use serde_json::Value;
use std::{env, fs};

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_chicken_spawn_variants TRACE.jsonl");
    let mut probes = 0;
    let mut draws = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        if row["type"] == "manifest" {
            assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "chicken_spawn_variant_rules"
            );
        }
        if row["type"] == "complete" {
            complete = true;
        }
        if row["type"] != "chicken_spawn_probe" {
            continue;
        }
        let data = &row["data"];
        let biome = data["biome"].as_str().unwrap();
        let seed = data["seed"].as_str().unwrap().parse::<i64>().unwrap();
        let count = data["count"].as_u64().unwrap() as usize;
        let variants = data["variants"].as_array().unwrap();
        let sounds = data["sounds"].as_array().unwrap();
        assert_eq!(variants.len(), count);
        assert_eq!(sounds.len(), count);
        let mut random = LegacyRandom::new(seed as u64);
        for i in 0..count {
            let mut chicken = Chicken::new(DVec3::ZERO);
            chicken.select_spawn_variants(
                biome == "minecraft:desert",
                biome == "minecraft:snowy_plains",
                &mut random,
            );
            let expected_variant = match variants[i].as_str().unwrap() {
                "minecraft:temperate" => ChickenVariant::Temperate,
                "minecraft:warm" => ChickenVariant::Warm,
                "minecraft:cold" => ChickenVariant::Cold,
                other => panic!("unknown appearance variant {other}"),
            };
            let expected_sound = match sounds[i].as_str().unwrap() {
                "minecraft:classic" => ChickenSoundVariant::Classic,
                "minecraft:picky" => ChickenSoundVariant::Picky,
                other => panic!("unknown sound variant {other}"),
            };
            assert_eq!(
                chicken.variant, expected_variant,
                "{biome} seed {seed} draw {i}"
            );
            assert_eq!(
                chicken.sound_variant, expected_sound,
                "{biome} seed {seed} draw {i}"
            );
            draws += 1;
        }
        probes += 1;
    }
    assert!(complete);
    assert_eq!(probes, 6);
    assert_eq!(draws, 384);
    println!(
        "{probes} biome/seed probes and {draws} ordered chicken appearance/sound draws matched"
    );
}
