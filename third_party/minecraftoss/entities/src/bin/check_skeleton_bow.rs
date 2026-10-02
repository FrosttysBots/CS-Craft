//! Exact 26.3 bow-use, launch, and open-air arrow gate. Skeleton locomotion
//! is still supplied by the reference trace pending its own world integration.
use glam::DVec3;
use minecraftoss_entities::{
    projectile::Arrow,
    skeleton_bow::{BowMovement, SkeletonBowGoal},
};
use minecraftoss_player::rng::LegacyRandom;
use serde_json::Value;
use std::{env, fs};

fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn position(value: &Value) -> DVec3 {
    DVec3::new(
        number(&value["x"]),
        number(&value["y"]),
        number(&value["z"]),
    )
}

fn exact(value: &Value, actual: f64, tick: i64, field: &str) {
    assert_eq!(
        number(value).to_bits(),
        actual.to_bits(),
        "tick {tick} {field}"
    );
}

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_skeleton_bow TRACE.jsonl");
    let mut bow = SkeletonBowGoal::normal();
    let mut bow_random = LegacyRandom::new(0);
    let mut shot_random_seed = None;
    let mut target = None;
    let mut previous_skeleton = None;
    let mut arrow: Option<Arrow> = None;
    let mut frames = 0;
    let mut expected_frames = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => {
                let case = &row["data"]["suite"]["scenarios"][0];
                assert_eq!(case["id"], "skeleton_player_bow_pursuit");
                expected_frames = case["ticks"].as_u64().unwrap() as usize;
            }
            "entity_set_random_seed" => {
                assert_eq!(row["data"]["tag"], "skeleton");
                bow_random =
                    LegacyRandom::new(row["data"]["seed"].as_str().unwrap().parse().unwrap());
            }
            "projectile_shoot_seed" => {
                shot_random_seed = Some(
                    row["data"]["seed"]
                        .as_str()
                        .unwrap()
                        .parse::<u64>()
                        .unwrap(),
                );
            }
            "player_probe" => {
                let p = &row["data"]["pos"];
                target = Some(DVec3::new(
                    p[0].as_f64().unwrap(),
                    p[1].as_f64().unwrap(),
                    p[2].as_f64().unwrap(),
                ));
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let observed = &row["data"]["entities"]["skeleton"][0];
                if tick == 0 {
                    previous_skeleton = Some(position(observed));
                    continue;
                }
                frames += 1;
                let previous = previous_skeleton.unwrap();
                let target = target.unwrap();
                if let Some(arrow) = &mut arrow {
                    arrow.tick_open_air();
                }
                let result = bow
                    .tick(
                        true,
                        true,
                        false,
                        previous.distance_squared(target),
                        true,
                        &mut bow_random,
                    )
                    .unwrap();
                assert_eq!(observed["using_item"], bow.using_item, "tick {tick} use");
                assert_eq!(
                    observed["ticks_using_item"], bow.ticks_using_item,
                    "tick {tick} use ticks"
                );
                assert_eq!(
                    observed["aggressive"], bow.aggressive,
                    "tick {tick} aggressive"
                );
                assert_eq!(observed["running_goals"][0], "RangedBowAttackGoal");
                if tick == 20 {
                    assert!(matches!(result.movement, BowMovement::Strafe { .. }));
                    assert_eq!(observed["navigation_done"], true);
                }
                if let Some(power) = result.shoot_power {
                    assert!(arrow.is_none(), "fixture has one shot");
                    assert_eq!(power, 1.0);
                    arrow = Some(Arrow::skeleton_shot(
                        previous,
                        1.74,
                        target,
                        1.8,
                        6.0,
                        &mut LegacyRandom::new(shot_random_seed.unwrap()),
                    ));
                }
                let observed_arrows = row["data"]["entity_type_states"]["minecraft:arrow"]
                    .as_array()
                    .unwrap();
                assert_eq!(
                    observed_arrows.len(),
                    usize::from(arrow.is_some()),
                    "tick {tick} arrow count"
                );
                if let Some(arrow) = &arrow {
                    let observed_arrow = &observed_arrows[0];
                    for (field, value) in [
                        ("x", arrow.position.x),
                        ("y", arrow.position.y),
                        ("z", arrow.position.z),
                        ("vx", arrow.velocity.x),
                        ("vy", arrow.velocity.y),
                        ("vz", arrow.velocity.z),
                    ] {
                        exact(&observed_arrow[field], value, tick, field);
                    }
                }
                previous_skeleton = Some(position(observed));
            }
            "complete" => complete = true,
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!(frames, expected_frames);
    println!("exact skeleton bow use and open-air arrow: {frames} ticks");
}
