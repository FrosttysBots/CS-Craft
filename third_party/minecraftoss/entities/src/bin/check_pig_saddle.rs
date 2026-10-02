//! Exact saddle equipment and feeding states for controlled NoAI pigs.
use glam::DVec3;
use minecraftoss_entities::{cow::InteractionResult, pig::Pig, world::EntityWorld};
use minecraftoss_player::{
    inventory::{Inventory, ItemStack},
    Block, Pos, World,
};
use serde_json::Value;
use std::{collections::HashMap, env, fs};

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

fn result(value: InteractionResult) -> &'static str {
    match value {
        InteractionResult::Pass => "Pass[]",
        InteractionResult::SuccessServer => "Success[swingSource=SERVER_ONLY, itemContext=ItemContext[wasItemInteraction=true, heldItemTransformedTo=null]]",
        InteractionResult::SuccessPredicted => "Success[swingSource=PREDICTED, itemContext=ItemContext[wasItemInteraction=true, heldItemTransformedTo=null]]",
    }
}

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_pig_saddle TRACE.jsonl");
    let trace = fs::read_to_string(path).unwrap();
    let mut world = EntityWorld::default();
    let mut ids = HashMap::<String, u64>::new();
    let mut actions = 0;
    let mut frames = 0;
    let mut complete = false;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        match row["type"].as_str().unwrap() {
            "error" => panic!("reference error: {row}"),
            "complete" => complete = true,
            "entity_use" => {
                let data = &row["data"];
                let tag = data["tag"].as_str().unwrap();
                let item = data["item"].as_str().unwrap();
                let mut inventory = Inventory::default();
                inventory.slots[0] =
                    Some(ItemStack::new(item, data["count"].as_u64().unwrap() as u8));
                let pig = &mut world.pig_mut(ids[tag]).unwrap().pig;
                let outcome = if item == "minecraft:saddle" {
                    pig.interact_saddle(&mut inventory, 0, false)
                } else {
                    pig.interact_food(
                        &mut inventory,
                        0,
                        false,
                        |id| id == "minecraft:carrot",
                        false,
                    )
                    .0
                };
                assert_eq!(
                    result(outcome),
                    data["result"].as_str().unwrap(),
                    "tick {} {tag} result",
                    row["tick"]
                );
                let hand = inventory.slots[0].as_ref();
                assert_eq!(
                    hand.map_or("minecraft:air", |stack| stack.id.as_str()),
                    data["hand_item"].as_str().unwrap(),
                    "tick {} {tag} item",
                    row["tick"]
                );
                assert_eq!(
                    hand.map_or(0, |stack| stack.count as u64),
                    data["hand_count"].as_u64().unwrap(),
                    "tick {} {tag} count",
                    row["tick"]
                );
                actions += 1;
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
                        pig.saddled = state["saddled"].as_bool().unwrap();
                        pig.persistence_required = state["persistence_required"].as_bool().unwrap();
                        let id = world.spawn_pig_no_ai(pig);
                        ids.insert(tag.clone(), id);
                    }
                } else {
                    assert_eq!(tick, frames + 1);
                    world.tick(&mut EmptyWorld);
                    frames += 1;
                }
                for (tag, states) in groups {
                    let state = &states[0];
                    let pig = &world.pig_mut(ids[tag]).unwrap().pig;
                    assert_eq!(state["saddled"], pig.saddled, "tick {tick} {tag} saddle");
                    assert_eq!(
                        state["saddle_item"],
                        if pig.saddled {
                            "minecraft:saddle"
                        } else {
                            "minecraft:air"
                        },
                        "tick {tick} {tag} saddle item"
                    );
                    assert_eq!(state["passengers"], 0, "tick {tick} {tag} passengers");
                    assert_eq!(state["age"], pig.age.ticks, "tick {tick} {tag} age");
                    assert_eq!(state["in_love"], pig.in_love, "tick {tick} {tag} love");
                    for (name, actual) in [
                        ("x", pig.body.position.x),
                        ("y", pig.body.position.y),
                        ("z", pig.body.position.z),
                        ("health", f64::from(pig.health)),
                    ] {
                        assert_eq!(
                            number(&state[name]).to_bits(),
                            actual.to_bits(),
                            "tick {tick} {tag} {name}"
                        );
                    }
                }
            }
            _ => {}
        }
    }
    assert!(complete && actions == 4 && frames == 3);
    println!("{actions} pig interactions and {frames} saddle frames matched exactly");
}
