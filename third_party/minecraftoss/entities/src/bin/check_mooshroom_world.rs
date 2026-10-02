//! Shared-world Mooshroom no-AI tick and interaction gate for pinned 26.3.
use glam::DVec3;
use minecraftoss_entities::{
    age::Age,
    cow::InteractionResult,
    mooshroom::{flower_effects, MushroomCow, MushroomVariant},
    world::EntityWorld,
};
use minecraftoss_player::{
    inventory::{Inventory, ItemStack},
    Block, Pos, World,
};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs};

#[derive(Default)]
struct Scene(BTreeMap<Pos, Block>);
impl World for Scene {
    fn block(&self, pos: Pos) -> Option<Block> {
        self.0.get(&pos).cloned()
    }
    fn set_block(&mut self, pos: Pos, block: Option<Block>) {
        if let Some(block) = block {
            self.0.insert(pos, block);
        } else {
            self.0.remove(&pos);
        }
    }
}
fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}
fn exact(expected: &Value, actual: f64, label: &str) {
    assert_eq!(actual.to_bits(), number(expected).to_bits(), "{label}");
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
        .expect("usage: check_mooshroom_world TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut ids = BTreeMap::<String, u64>::new();
    let mut last_tick = 0;
    let mut uses = 0;
    let mut frames = 0;
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
            let mut inventory = Inventory::default();
            inventory.slots[0] = Some(ItemStack::new(
                data["item"].as_str().unwrap(),
                data["count"].as_u64().unwrap() as u8,
            ));
            let entity = world.mooshroom_mut(ids[tag]).unwrap();
            let (outcome, _) = entity.mooshroom.as_mut().unwrap().interact(
                &mut entity.cow,
                &mut inventory,
                0,
                false,
                |item| item == "minecraft:wheat",
                flower_effects,
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
                data["hand_item"].as_str().unwrap()
            );
            assert_eq!(
                hand.map_or(0, |stack| stack.count as u64),
                data["hand_count"].as_u64().unwrap()
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
        let tick = row["tick"].as_i64().unwrap();
        if tick == 0 {
            for (key, block) in row["data"]["blocks"].as_object().unwrap() {
                let coords: Vec<i32> = key.split(',').map(|part| part.parse().unwrap()).collect();
                let id = block["id"].as_str().unwrap();
                if id != "minecraft:air" {
                    scene.set_block((coords[0], coords[1], coords[2]), Some(Block::new(id)));
                }
            }
            for (tag, variant) in [
                ("red", MushroomVariant::Red),
                ("brown", MushroomVariant::Brown),
                ("baby", MushroomVariant::Brown),
            ] {
                let observed = &row["data"]["entities"][tag][0];
                let mut mob = MushroomCow::new(
                    DVec3::new(
                        number(&observed["x"]),
                        number(&observed["y"]),
                        number(&observed["z"]),
                    ),
                    variant,
                );
                mob.cow.body.velocity = DVec3::new(
                    number(&observed["vx"]),
                    number(&observed["vy"]),
                    number(&observed["vz"]),
                );
                mob.cow.body.on_ground = observed["on_ground"].as_bool().unwrap();
                mob.cow.yaw = number(&observed["yaw"]) as f32;
                mob.cow.speed = number(&observed["speed"]) as f32;
                mob.cow.age = Age {
                    ticks: observed["age"].as_i64().unwrap() as i32,
                    forced: observed["forced_age"].as_i64().unwrap() as i32,
                    locked: observed["age_locked"].as_bool().unwrap(),
                    forced_particle_ticks: observed["forced_age_timer"].as_i64().unwrap() as i32,
                    lock_particle_ticks: 0,
                };
                mob.cow.health = number(&observed["health"]) as f32;
                mob.cow.in_love = observed["in_love"].as_i64().unwrap() as i32;
                mob.cow.persistence_required = observed["persistence_required"].as_bool().unwrap();
                ids.insert(tag.to_owned(), world.spawn_mooshroom(mob, true));
            }
        } else {
            assert_eq!(tick, last_tick + 1);
            world.tick(&mut scene);
            for tag in ["red", "brown", "baby"] {
                let observed = &row["data"]["entities"][tag][0];
                let entity = world
                    .cows()
                    .iter()
                    .find(|entity| entity.id == ids[tag])
                    .unwrap();
                let cow = &entity.cow;
                assert_eq!(
                    observed["mooshroom_variant"],
                    if entity.mooshroom.as_ref().unwrap().variant == MushroomVariant::Red {
                        "red"
                    } else {
                        "brown"
                    }
                );
                assert_eq!(
                    entity.id as i64,
                    observed["entity_numeric_id"].as_i64().unwrap()
                );
                assert_eq!(
                    entity.tick_count as i64,
                    observed["entity_tick_count"].as_i64().unwrap()
                );
                assert_eq!(
                    cow.age.ticks as i64,
                    observed["age"].as_i64().unwrap(),
                    "tick {tick} {tag} age"
                );
                assert_eq!(cow.in_love as i64, observed["in_love"].as_i64().unwrap());
                assert_eq!(
                    cow.persistence_required,
                    observed["persistence_required"].as_bool().unwrap()
                );
                assert_eq!(cow.body.on_ground, observed["on_ground"].as_bool().unwrap());
                assert_eq!(entity.running_goals().len(), 0);
                for (field, actual) in [
                    ("x", cow.body.position.x),
                    ("y", cow.body.position.y),
                    ("z", cow.body.position.z),
                    ("vx", cow.body.velocity.x),
                    ("vy", cow.body.velocity.y),
                    ("vz", cow.body.velocity.z),
                    ("yaw", f64::from(cow.yaw)),
                    ("speed", f64::from(cow.speed)),
                    ("head_yaw", f64::from(entity.look_control.head_yaw)),
                    ("body_yaw", f64::from(entity.body_rotation.body_yaw)),
                    ("pitch", f64::from(entity.look_control.pitch)),
                    (
                        "eye_y",
                        cow.body.position.y
                            + if cow.age.baby() {
                                f64::from(0.69_f32)
                            } else {
                                f64::from(1.3_f32)
                            },
                    ),
                ] {
                    exact(
                        &observed[field],
                        actual,
                        &format!("tick {tick} {tag} {field}"),
                    );
                }
                frames += 1;
            }
        }
        last_tick = tick;
    }
    assert!(complete);
    assert_eq!(uses, 8);
    assert_eq!(frames, 24);
    println!("{uses} mooshroom uses and {frames} shared-world state frames matched exactly");
}
