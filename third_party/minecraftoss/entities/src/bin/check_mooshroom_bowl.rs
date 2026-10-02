//! Exact isolated MushroomCow bowl and brown-flower interactions against 26.3.
use glam::DVec3;
use minecraftoss_entities::{
    cow::InteractionResult,
    mooshroom::{flower_effects, MushroomCow, MushroomVariant},
};
use minecraftoss_player::inventory::{Inventory, ItemStack};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_mooshroom_bowl TRACE.jsonl");
    let mut mobs = BTreeMap::<String, MushroomCow>::new();
    let mut uses = 0;
    let mut frames = 0;
    let mut tick = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        if row["type"] == "manifest" {
            assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "mooshroom_bowl_and_brown_flower"
            );
        }
        if row["type"] == "complete" {
            complete = true;
        }
        if row["type"] == "entity_use" {
            let data = &row["data"];
            let tag = data["tag"].as_str().unwrap();
            let item = data["item"].as_str().unwrap();
            let count = data["count"].as_u64().unwrap() as u8;
            let mut inventory = Inventory::default();
            inventory.slots[0] = Some(ItemStack::new(item, count));
            let (result, _) = mobs.get_mut(tag).unwrap().interact(
                &mut inventory,
                0,
                false,
                |held| held == "minecraft:wheat",
                flower_effects,
            );
            let observed_success = data["result"].as_str().unwrap().starts_with("Success");
            assert_eq!(
                result != InteractionResult::Pass,
                observed_success,
                "tick {} {tag} result",
                row["tick"]
            );
            let hand = inventory.slots[0].as_ref();
            assert_eq!(
                hand.map_or("minecraft:air", |stack| stack.id.as_str()),
                data["hand_item"].as_str().unwrap(),
                "tick {} {tag} hand item",
                row["tick"]
            );
            assert_eq!(
                hand.map_or(0, |stack| stack.count as i64),
                data["hand_count"].as_i64().unwrap(),
                "tick {} {tag} hand count",
                row["tick"]
            );
            let effects = hand
                .and_then(|stack| stack.components.as_ref())
                .and_then(|components| components.get("minecraft:suspicious_stew_effects"))
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()));
            assert_eq!(
                effects, data["hand_stew_effects"],
                "tick {} {tag} stew effects",
                row["tick"]
            );
            uses += 1;
        }
        if row["type"] != "snapshot" {
            continue;
        }
        let next_tick = row["tick"].as_i64().unwrap();
        if next_tick == 0 {
            for (tag, variant) in [
                ("red", MushroomVariant::Red),
                ("brown", MushroomVariant::Brown),
                ("baby", MushroomVariant::Brown),
            ] {
                let entity = &row["data"]["entities"][tag][0];
                let mut mob = MushroomCow::new(
                    DVec3::new(
                        number(&entity["x"]),
                        number(&entity["y"]),
                        number(&entity["z"]),
                    ),
                    variant,
                );
                mob.cow.age.ticks = entity["age"].as_i64().unwrap() as i32;
                mobs.insert(tag.to_owned(), mob);
            }
        } else {
            assert_eq!(next_tick, tick + 1);
            for (tag, mob) in &mut mobs {
                mob.cow.tick_age_and_love();
                let observed = &row["data"]["entities"][tag][0];
                assert_eq!(
                    mob.cow.age.ticks as i64,
                    observed["age"].as_i64().unwrap(),
                    "tick {next_tick} {tag} age"
                );
                assert_eq!(
                    mob.cow.in_love as i64,
                    observed["in_love"].as_i64().unwrap(),
                    "tick {next_tick} {tag} love"
                );
                frames += 1;
            }
        }
        tick = next_tick;
    }
    assert!(complete);
    assert_eq!(uses, 8);
    assert_eq!(frames, 24);
    println!("{uses} mooshroom bowl/flower uses and {frames} age/love frames matched");
}
