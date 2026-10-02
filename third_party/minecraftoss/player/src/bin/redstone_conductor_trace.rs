//! Rust replay of scenarios/redstone-conductor-basic.json.
use minecraftoss_player::{redstone::RedstoneEngine, Block, Pos, World};
use std::collections::BTreeMap;

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
    let wire_mode = std::env::args().any(|arg| arg == "--wire");
    let stone = (1, 81, 0);
    let lever = (1, 82, 0);
    let wire = (2, 81, 0);
    let lamp = (if wire_mode { 3 } else { 2 }, 81, 0);
    let positions = if wire_mode {
        vec![stone, lever, wire, lamp]
    } else {
        vec![stone, lever, lamp]
    };
    let mut world = Circuit::default();
    world.set_block(stone, Some(Block::new("minecraft:stone")));
    world.set_block(
        lever,
        Some(
            Block::new("minecraft:lever")
                .with("face", "floor")
                .with("facing", "north")
                .with("powered", "false"),
        ),
    );
    if wire_mode {
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
    }
    world.set_block(
        lamp,
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=14 {
        if tick == 2 || tick == 7 {
            engine.use_block(&mut world, lever, tick - 1).unwrap();
        }
        engine.tick(&mut world, tick);
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
