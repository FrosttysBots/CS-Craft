//! Exact Sheep/Animal/AgeableMob item transactions against repeated pinned 26.3.
use glam::DVec3;
use minecraftoss_entities::{
    age::Age,
    cow::InteractionResult,
    sheep::{Sheep, Wool, DYE_NAMES},
    world::EntityWorld,
};
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

fn exact(value: &Value, actual: f64, label: &str, tick: i64, tag: &str) {
    assert_eq!(
        number(value).to_bits(),
        actual.to_bits(),
        "tick {tick} {tag} {label}"
    );
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
        .expect("usage: check_sheep_feeding TRACE.jsonl");
    let trace = fs::read_to_string(path).unwrap();
    let mut world = EntityWorld::default();
    let mut ids = HashMap::<String, u64>::new();
    let mut actions = 0;
    let mut frames = 0;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        match row["type"].as_str().unwrap() {
            "entity_use" => {
                let data = &row["data"];
                let tag = data["tag"].as_str().unwrap();
                let mut inventory = Inventory::default();
                let item = data["item"].as_str().unwrap();
                inventory.slots[0] =
                    Some(ItemStack::new(item, data["count"].as_u64().unwrap() as u8));
                let (outcome, _) = world.sheep_mut(ids[tag]).unwrap().sheep.interact_food(
                    &mut inventory,
                    0,
                    false,
                    |id| id == "minecraft:wheat",
                    false,
                );
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
                    "tick {} {tag} held item",
                    row["tick"]
                );
                assert_eq!(
                    hand.map_or(0, |stack| stack.count as u64),
                    data["hand_count"].as_u64().unwrap(),
                    "tick {} {tag} held count",
                    row["tick"]
                );
                assert_eq!(data["inventory_milk_buckets"], 0);
                actions += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let groups = row["data"]["entities"].as_object().unwrap();
                if tick == 0 {
                    let mut ordered: Vec<_> = groups
                        .iter()
                        .map(|(tag, states)| (tag, &states[0]))
                        .collect();
                    ordered.sort_by_key(|(_, state)| {
                        state["entity_numeric_id"].as_i64().unwrap_or_default()
                    });
                    for (tag, state) in ordered {
                        let color = DYE_NAMES
                            .iter()
                            .position(|name| *name == state["sheep_color"].as_str().unwrap())
                            .unwrap() as u8;
                        let mut wool = Wool::default();
                        wool.set_color(color);
                        wool.set_sheared(state["sheared"].as_bool().unwrap());
                        let sheep = Sheep {
                            age: Age {
                                ticks: state["age"].as_i64().unwrap() as i32,
                                forced: state["forced_age"].as_i64().unwrap() as i32,
                                locked: state["age_locked"].as_bool().unwrap(),
                                forced_particle_ticks: state["forced_age_timer"].as_i64().unwrap()
                                    as i32,
                                lock_particle_ticks: 0,
                            },
                            in_love: state["in_love"].as_i64().unwrap() as i32,
                            wool,
                            persistence_required: state["persistence_required"].as_bool().unwrap(),
                        };
                        let id = world.spawn_sheep_no_ai(
                            sheep,
                            DVec3::new(
                                number(&state["x"]),
                                number(&state["y"]),
                                number(&state["z"]),
                            ),
                        );
                        ids.insert(tag.clone(), id);
                    }
                } else {
                    assert_eq!(tick, frames + 1);
                    world.tick(&mut EmptyWorld);
                    frames += 1;
                }
                for (tag, states) in groups {
                    let state = &states[0];
                    let entity = world.sheep_mut(ids[tag]).unwrap();
                    assert_eq!(
                        state["age"], entity.sheep.age.ticks,
                        "tick {tick} {tag} age"
                    );
                    assert_eq!(
                        state["forced_age"], entity.sheep.age.forced,
                        "tick {tick} {tag} forced age"
                    );
                    assert_eq!(
                        state["forced_age_timer"], entity.sheep.age.forced_particle_ticks,
                        "tick {tick} {tag} forced timer"
                    );
                    assert_eq!(
                        state["age_locked"], entity.sheep.age.locked,
                        "tick {tick} {tag} age lock"
                    );
                    assert_eq!(
                        state["in_love"], entity.sheep.in_love,
                        "tick {tick} {tag} love"
                    );
                    assert_eq!(
                        state["persistence_required"], entity.sheep.persistence_required,
                        "tick {tick} {tag} persistence"
                    );
                    assert_eq!(
                        state["sheep_color"],
                        entity.sheep.wool.color(),
                        "tick {tick} {tag} color"
                    );
                    assert_eq!(
                        state["sheared"],
                        entity.sheep.wool.sheared(),
                        "tick {tick} {tag} fleece"
                    );
                    for (name, actual) in [
                        ("x", entity.body.position.x),
                        ("y", entity.body.position.y),
                        ("z", entity.body.position.z),
                        ("vx", entity.body.velocity.x),
                        ("vy", entity.body.velocity.y),
                        ("vz", entity.body.velocity.z),
                        ("health", f64::from(entity.health)),
                    ] {
                        exact(&state[name], actual, name, tick, tag);
                    }
                }
            }
            _ => {}
        }
    }
    assert_eq!(actions, 10);
    assert_eq!(frames, 48);
    println!("{actions} sheep interactions and {frames} shared-world frames matched exactly");
}
