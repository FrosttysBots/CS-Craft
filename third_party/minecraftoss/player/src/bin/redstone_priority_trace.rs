//! Ordered tick replay of scenarios/redstone-scheduled-priority.json.
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
    let comparator = (1, 81, 0);
    let repeater = (5, 81, 0);
    let positions: Vec<Pos> = (0..=5).map(|x| (x, 81, 0)).collect();
    let mut world = Circuit::default();
    world.set_block(
        comparator,
        Some(
            Block::new("minecraft:comparator")
                .with("facing", "west")
                .with("mode", "compare")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        repeater,
        Some(
            Block::new("minecraft:repeater")
                .with("facing", "west")
                .with("delay", "1")
                .with("locked", "false")
                .with("powered", "false"),
        ),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=8 {
        if tick == 2 {
            for input in [(0, 81, 0), (4, 81, 0)] {
                world.set_block(input, Some(Block::new("minecraft:redstone_block")));
                engine.changed(&mut world, &[input], tick - 1);
            }
        }
        engine.tick(&mut world, tick);
        let events: Vec<_> = engine.take_executed_ticks().into_iter().map(|(pos, block)|
            serde_json::json!({"block": block, "position": [pos.0, pos.1, pos.2]})).collect();
        let blocks: Vec<_> = positions
            .iter()
            .map(|&pos| {
                let block = world
                    .block(pos)
                    .unwrap_or_else(|| Block::new("minecraft:air"));
                serde_json::json!({"id": block.id, "properties": block.properties})
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "tick": tick, "blocks": blocks,
                "comparator_output": engine.comparator_output(comparator),
                "scheduled_tick_events": events,
            })
        );
    }
}
