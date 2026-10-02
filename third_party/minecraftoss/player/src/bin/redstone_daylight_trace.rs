//! Replays daylight-detector block logic using environment observations from
//! a repeatable vanilla trace. The environment provider is checked separately.
use minecraftoss_player::{redstone::RedstoneEngine, Block, Pos, World};
use std::{collections::BTreeMap, fs};

#[derive(Default)]
struct Circuit(BTreeMap<Pos, Block>);
impl World for Circuit {
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

fn main() {
    let trace = std::env::args().nth(1).expect("vanilla trace path");
    let detector = (0, 81, 0);
    let wire = (1, 81, 0);
    let lamp = (2, 81, 0);
    let positions = [detector, wire, lamp];
    let mut world = Circuit::default();
    world.set_block(
        detector,
        Some(
            Block::new("minecraft:daylight_detector")
                .with("inverted", "false")
                .with("power", "0"),
        ),
    );
    world.set_block(
        wire,
        Some(
            Block::new("minecraft:redstone_wire")
                .with("power", "0")
                .with("north", "none")
                .with("east", "none")
                .with("south", "none")
                .with("west", "none"),
        ),
    );
    world.set_block(
        lamp,
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for line in fs::read_to_string(trace).unwrap().lines() {
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        if row["type"] != "snapshot" || row["scenario"] != "daylight_detector_time_weather_invert" {
            continue;
        }
        let tick = row["tick"].as_u64().unwrap();
        let sample = &row["data"]["daylight_samples"]["0,81,0"];
        let sky = sample["effective_sky"].as_u64().unwrap() as u8;
        let angle = f32::from_bits(
            u32::from_str_radix(sample["sun_angle_bits"].as_str().unwrap(), 16).unwrap(),
        );
        if tick == 95 {
            engine.use_daylight_detector(&mut world, detector, sky, angle, tick - 1);
        }
        engine.tick(&mut world, tick);
        if tick > 0 && tick % 20 == 0 {
            engine.update_daylight_detector(&mut world, detector, sky, angle, tick);
        }
        let blocks: Vec<_> = positions
            .iter()
            .map(|&pos| {
                let block = world.block(pos).unwrap();
                serde_json::json!({"id": block.id, "properties": block.properties})
            })
            .collect();
        println!("{}", serde_json::json!({"tick": tick, "blocks": blocks}));
    }
}
