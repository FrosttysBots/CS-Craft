//! Exact mooshroom lightning UUID guard against pinned Minecraft 26.3.
use glam::DVec3;
use minecraftoss_entities::{
    mooshroom::{MushroomCow, MushroomVariant},
    world::EntityWorld,
};
use minecraftoss_player::{Block, Pos, World};
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
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap())
}
fn exact(expected: &Value, actual: f64, label: &str, tick: i64) {
    assert_eq!(
        actual.to_bits(),
        number(expected).to_bits(),
        "tick {tick} {label}"
    );
}
fn name(variant: MushroomVariant) -> &'static str {
    match variant {
        MushroomVariant::Red => "red",
        MushroomVariant::Brown => "brown",
    }
}
fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: check_mooshroom_lightning TRACE.jsonl");
    let mut world = EntityWorld::default();
    let mut scene = Scene::default();
    let mut ids = BTreeMap::<String, u64>::new();
    let mut probes = 0;
    let mut frames = 0;
    let mut complete = false;
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => assert_eq!(
                row["data"]["suite"]["scenarios"][0]["id"],
                "mooshroom_lightning_uuid_guard"
            ),
            "complete" => complete = true,
            "mooshroom_lightning_probe" => {
                let data = &row["data"];
                let tag = data["tag"].as_str().unwrap();
                let bolt = data["bolt_uuid"].as_str().unwrap();
                let entity = world.mooshroom_mut(ids[tag]).unwrap();
                let state = entity.mooshroom.as_mut().unwrap();
                assert_eq!(data["before"], name(state.variant));
                let changed = state.lightning_hit(bolt);
                assert_eq!(data["after"], name(state.variant));
                assert_eq!(changed, data["before"] != data["after"]);
                probes += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                if tick == 0 {
                    for (key, block) in data["blocks"].as_object().unwrap() {
                        let pos: Vec<i32> = key.split(',').map(|s| s.parse().unwrap()).collect();
                        let id = block["id"].as_str().unwrap();
                        if id != "minecraft:air" {
                            scene.set_block((pos[0], pos[1], pos[2]), Some(Block::new(id)));
                        }
                    }
                    for (tag, variant) in [
                        ("red", MushroomVariant::Red),
                        ("brown", MushroomVariant::Brown),
                        ("baby", MushroomVariant::Brown),
                    ] {
                        let observed = &data["entities"][tag][0];
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
                        mob.cow.age.ticks = observed["age"].as_i64().unwrap() as i32;
                        mob.cow.health = number(&observed["health"]) as f32;
                        mob.cow.persistence_required =
                            observed["persistence_required"].as_bool().unwrap();
                        ids.insert(tag.to_owned(), world.spawn_mooshroom(mob, true));
                    }
                } else {
                    world.tick(&mut scene);
                    for tag in ["red", "brown", "baby"] {
                        let observed = &data["entities"][tag][0];
                        let entity = world.mooshroom_mut(ids[tag]).unwrap();
                        let cow = &entity.cow;
                        assert_eq!(observed["type"], "minecraft:mooshroom");
                        assert_eq!(
                            observed["mooshroom_variant"],
                            name(entity.mooshroom.as_ref().unwrap().variant)
                        );
                        assert_eq!(observed["age"], cow.age.ticks);
                        assert_eq!(observed["no_ai"], entity.no_ai);
                        assert_eq!(observed["persistence_required"], cow.persistence_required);
                        assert_eq!(observed["on_ground"], cow.body.on_ground);
                        for (field, actual) in [
                            ("x", cow.body.position.x),
                            ("y", cow.body.position.y),
                            ("z", cow.body.position.z),
                            ("vx", cow.body.velocity.x),
                            ("vy", cow.body.velocity.y),
                            ("vz", cow.body.velocity.z),
                            ("health", f64::from(cow.health)),
                        ] {
                            exact(&observed[field], actual, &format!("{tag} {field}"), tick);
                        }
                        frames += 1;
                    }
                }
            }
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!((probes, frames), (9, 12));
    println!("{probes} lightning probes and {frames} exact mooshroom frames matched");
}
