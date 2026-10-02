//! Compare cow item transactions and age/love state against a repeated 26.3
//! server trace. The vanilla-to-vanilla comparator runs before this gate.
use glam::DVec3;
use minecraftoss_entities::{
    age::Age,
    cow::{Cow, InteractionResult},
};
use minecraftoss_player::inventory::{Inventory, ItemStack};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap())
}
fn initial(e: &Value) -> Cow {
    let mut cow = Cow::new(DVec3::new(
        number(&e["x"]),
        number(&e["y"]),
        number(&e["z"]),
    ));
    cow.age = Age {
        ticks: e["age"].as_i64().unwrap() as i32,
        forced: e["forced_age"].as_i64().unwrap() as i32,
        locked: e["age_locked"].as_bool().unwrap(),
        forced_particle_ticks: e["forced_age_timer"].as_i64().unwrap() as i32,
        lock_particle_ticks: 0,
    };
    cow.in_love = e["in_love"].as_i64().unwrap() as i32;
    cow.persistence_required = e["persistence_required"].as_bool().unwrap();
    cow.health = number(&e["health"]) as f32;
    cow.sync_dimensions();
    cow
}
fn result(v: InteractionResult) -> &'static str {
    match v {
        InteractionResult::Pass => "Pass[]",
        InteractionResult::SuccessServer => "Success[swingSource=SERVER_ONLY, itemContext=ItemContext[wasItemInteraction=true, heldItemTransformedTo=null]]",
        InteractionResult::SuccessPredicted => "Success[swingSource=PREDICTED, itemContext=ItemContext[wasItemInteraction=true, heldItemTransformedTo=null]]",
    }
}
fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_cow_interactions TRACE.jsonl");
    let trace = fs::read_to_string(path).unwrap();
    let mut cows = BTreeMap::<String, Cow>::new();
    let mut current_tick = 0;
    let mut actions = 0;
    let mut frames = 0;
    let mut completed = false;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        if row["type"] == "complete" {
            completed = true;
        }
        if row["type"] == "entity_use" {
            let d = &row["data"];
            let tag = d["tag"].as_str().unwrap();
            let cow = cows.get_mut(tag).expect("tagged initial cow");
            let mut inventory = Inventory::default();
            let id = d["item"].as_str().unwrap();
            inventory.slots[0] = Some(ItemStack::new(id, d["count"].as_u64().unwrap() as u8));
            let (outcome, _) = cow.interact(
                &mut inventory,
                0,
                false,
                |item| item == "minecraft:wheat",
                false,
            );
            assert_eq!(
                result(outcome),
                d["result"].as_str().unwrap(),
                "tick {} {tag} result",
                row["tick"]
            );
            let hand = inventory.slots[0].as_ref();
            assert_eq!(
                hand.map_or("minecraft:air", |s| s.id.as_str()),
                d["hand_item"].as_str().unwrap(),
                "tick {} {tag} hand item",
                row["tick"]
            );
            assert_eq!(
                hand.map_or(0, |s| s.count as u64),
                d["hand_count"].as_u64().unwrap(),
                "tick {} {tag} hand count",
                row["tick"]
            );
            let milk: u64 = inventory
                .slots
                .iter()
                .flatten()
                .filter(|s| s.id == "minecraft:milk_bucket")
                .map(|s| s.count as u64)
                .sum();
            assert_eq!(
                milk,
                d["inventory_milk_buckets"].as_u64().unwrap(),
                "tick {} {tag} milk",
                row["tick"]
            );
            actions += 1;
        }
        if row["type"] != "snapshot" {
            continue;
        }
        let tick = row["tick"].as_i64().unwrap();
        if tick > 0 {
            assert_eq!(tick, current_tick + 1);
            for cow in cows.values_mut() {
                cow.tick_age_and_love();
            }
        }
        current_tick = tick;
        for (tag, group) in row["data"]["entities"].as_object().unwrap() {
            let entities = group.as_array().unwrap();
            assert_eq!(entities.len(), 1, "one cow per tag");
            let e = &entities[0];
            assert_eq!(e["type"], "minecraft:cow");
            if tick == 0 {
                cows.insert(tag.to_owned(), initial(e));
                continue;
            }
            let cow = &cows[tag];
            for (field, actual) in [
                ("age", cow.age.ticks),
                ("forced_age", cow.age.forced),
                ("forced_age_timer", cow.age.forced_particle_ticks),
                ("in_love", cow.in_love),
            ] {
                assert_eq!(
                    actual as i64,
                    e[field].as_i64().unwrap(),
                    "tick {tick} {tag} {field}"
                );
            }
            assert_eq!(
                cow.age.locked,
                e["age_locked"].as_bool().unwrap(),
                "tick {tick} {tag} age lock"
            );
            assert_eq!(
                cow.persistence_required,
                e["persistence_required"].as_bool().unwrap(),
                "tick {tick} {tag} persistence"
            );
            frames += 1;
        }
    }
    assert!(
        completed && actions > 0 && frames > 0,
        "incomplete or empty gate"
    );
    println!("{actions} interaction transactions and {frames} cow state frames matched exactly; AI/movement excluded");
}
