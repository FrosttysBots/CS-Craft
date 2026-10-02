//! Replays rising, descending and ceiling-covered one-block wire circuits.
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
    let down = std::env::args().any(|arg| arg == "--down");
    let covered = std::env::args().any(|arg| arg == "--covered");
    let mut world = Circuit::default();
    for x in 0..=4 {
        world.set_block((x, 80, 0), Some(Block::new("minecraft:stone")));
    }
    for x in if down { 0..=1 } else { 2..=4 } {
        world.set_block((x, 81, 0), Some(Block::new("minecraft:stone")));
    }
    if covered {
        world.set_block((1, 82, 0), Some(Block::new("minecraft:stone")));
    }
    let observed = if down {
        [(0, 82, 0), (1, 82, 0), (2, 81, 0), (3, 81, 0), (4, 81, 0)]
    } else {
        [(0, 81, 0), (1, 81, 0), (2, 82, 0), (3, 82, 0), (4, 82, 0)]
    };
    world.set_block(
        observed[0],
        Some(
            Block::new("minecraft:lever")
                .with("face", "floor")
                .with("facing", "north")
                .with("powered", "false"),
        ),
    );
    for &pos in &observed[1..4] {
        world.set_block(
            pos,
            Some(Block::new("minecraft:redstone_wire").with("power", "0")),
        );
    }
    world.set_block(
        observed[4],
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    let mut engine = RedstoneEngine::default();
    let initial: Vec<_> = world.0.keys().copied().collect();
    engine.changed(&mut world, &initial, 0);
    for tick in 0..=12u64 {
        if tick == 2 || tick == 6 {
            engine.use_block(&mut world, observed[0], tick - 1).unwrap();
        }
        engine.tick(&mut world, tick);
        let blocks: Vec<_> = observed
            .iter()
            .map(|&pos| {
                let block = world.block(pos).unwrap();
                serde_json::json!({"id":block.id,"properties":block.properties})
            })
            .collect();
        println!("{}", serde_json::json!({"tick":tick,"blocks":blocks}));
    }
}
