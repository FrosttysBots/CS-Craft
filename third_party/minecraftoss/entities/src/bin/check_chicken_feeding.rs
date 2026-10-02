//! Exact Chicken/Animal/AgeableMob transactions and NoAI world ticks.
use glam::DVec3;
use minecraftoss_entities::{
    age::Age, chicken::Chicken, cow::InteractionResult, world::EntityWorld,
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
        .expect("usage: check_chicken_feeding TRACE.jsonl");
    let trace = fs::read_to_string(path).unwrap();
    let mut world = EntityWorld::default();
    let mut ids = HashMap::<String, u64>::new();
    let mut actions = 0;
    let mut seeds = 0;
    let mut frames = 0;
    let mut complete = false;
    for line in trace.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        match row["type"].as_str().unwrap() {
            "error" => panic!("reference error: {row}"),
            "complete" => complete = true,
            "entity_set_random_seed" => {
                let tag = row["data"]["tag"].as_str().unwrap();
                let seed = row["data"]["seed"]
                    .as_str()
                    .unwrap()
                    .parse::<i64>()
                    .unwrap();
                world.chicken_mut(ids[tag]).unwrap().set_random_seed(seed);
                seeds += 1;
            }
            "entity_use" => {
                let data = &row["data"];
                let tag = data["tag"].as_str().unwrap();
                let item = data["item"].as_str().unwrap();
                let mut inventory = Inventory::default();
                inventory.slots[0] =
                    Some(ItemStack::new(item, data["count"].as_u64().unwrap() as u8));
                let (outcome, _) = world.chicken_mut(ids[tag]).unwrap().chicken.interact_food(
                    &mut inventory,
                    0,
                    false,
                    |id| {
                        matches!(
                            id,
                            "minecraft:wheat_seeds"
                                | "minecraft:melon_seeds"
                                | "minecraft:pumpkin_seeds"
                                | "minecraft:beetroot_seeds"
                                | "minecraft:torchflower_seeds"
                                | "minecraft:pitcher_pod"
                        )
                    },
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
                        let mut chicken = Chicken::new(DVec3::new(
                            number(&state["x"]),
                            number(&state["y"]),
                            number(&state["z"]),
                        ));
                        chicken.age = Age {
                            ticks: state["age"].as_i64().unwrap() as i32,
                            forced: state["forced_age"].as_i64().unwrap() as i32,
                            locked: state["age_locked"].as_bool().unwrap(),
                            forced_particle_ticks: state["forced_age_timer"].as_i64().unwrap()
                                as i32,
                            lock_particle_ticks: 0,
                        };
                        chicken.in_love = state["in_love"].as_i64().unwrap() as i32;
                        chicken.egg_time = state["egg_time"].as_i64().unwrap() as i32;
                        chicken.persistence_required =
                            state["persistence_required"].as_bool().unwrap();
                        let id = world.spawn_chicken(chicken, true);
                        ids.insert(tag.clone(), id);
                    }
                } else {
                    assert_eq!(tick, frames + 1);
                    world.tick(&mut EmptyWorld);
                    frames += 1;
                }
                for (tag, states) in groups {
                    let state = &states[0];
                    let chicken = &world.chicken_mut(ids[tag]).unwrap().chicken;
                    for (name, actual) in [
                        ("age", chicken.age.ticks),
                        ("forced_age", chicken.age.forced),
                        ("forced_age_timer", chicken.age.forced_particle_ticks),
                        ("in_love", chicken.in_love),
                    ] {
                        assert_eq!(state[name], actual, "tick {tick} {tag} {name}");
                    }
                    assert_eq!(
                        state["age_locked"], chicken.age.locked,
                        "tick {tick} {tag} age lock"
                    );
                    assert_eq!(
                        state["persistence_required"], chicken.persistence_required,
                        "tick {tick} {tag} persistence"
                    );
                    assert_eq!(
                        state["egg_time"], chicken.egg_time,
                        "tick {tick} {tag} egg timer"
                    );
                    for (name, actual) in [
                        ("flap", f64::from(chicken.flap)),
                        ("flap_speed", f64::from(chicken.flap_speed)),
                        ("flapping", f64::from(chicken.flapping)),
                    ] {
                        assert_eq!(
                            number(&state[name]).to_bits(),
                            actual.to_bits(),
                            "tick {tick} {tag} {name}"
                        );
                    }
                    for (name, actual) in [
                        ("x", chicken.body.position.x),
                        ("y", chicken.body.position.y),
                        ("z", chicken.body.position.z),
                        ("vx", chicken.body.velocity.x),
                        ("vy", chicken.body.velocity.y),
                        ("vz", chicken.body.velocity.z),
                        ("health", f64::from(chicken.health)),
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
    assert!(complete && actions == 9 && seeds == 2 && frames == 48);
    println!("{actions} chicken interactions and {frames} shared-world frames matched exactly");
}
