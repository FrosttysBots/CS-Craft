//! Replay same-chunk and cross-chunk equal-priority repeater tick orders.
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
    for (name, z, first_x, second_x, repeater_x) in [
        ("repeater_same_priority_low_x_first", 0, 0, 4, [1, 5]),
        ("repeater_same_priority_high_x_first", 4, 4, 0, [1, 5]),
        ("cross_chunk_repeater_low_x_first", 0, 14, 18, [15, 19]),
        ("cross_chunk_repeater_high_x_first", 4, 18, 14, [15, 19]),
    ] {
        let positions: Vec<Pos> = (repeater_x[0] - 1..=repeater_x[1])
            .map(|x| (x, 81, z))
            .collect();
        let mut world = Circuit::default();
        for x in repeater_x {
            world.set_block(
                (x, 81, z),
                Some(
                    Block::new("minecraft:repeater")
                        .with("facing", "west")
                        .with("delay", "1")
                        .with("locked", "false")
                        .with("powered", "false"),
                ),
            );
        }
        let mut engine = RedstoneEngine::default();
        engine.changed(&mut world, &positions, 0);
        for tick in 0..=8 {
            if tick == 2 {
                for x in [first_x, second_x] {
                    let input = (x, 81, z);
                    world.set_block(input, Some(Block::new("minecraft:redstone_block")));
                    engine.changed(&mut world, &[input], tick - 1);
                }
            }
            engine.tick(&mut world, tick);
            let events: Vec<_> = engine
                .take_executed_ticks()
                .into_iter()
                .map(|(pos, block)| {
                    serde_json::json!({"block": block, "position": [pos.0, pos.1, pos.2]})
                })
                .collect();
            let blocks: BTreeMap<_, _> = positions
                .iter()
                .map(|&pos| {
                    let block = world
                        .block(pos)
                        .unwrap_or_else(|| Block::new("minecraft:air"));
                    (
                        format!("{},{},{}", pos.0, pos.1, pos.2),
                        serde_json::json!({"id": block.id, "properties": block.properties}),
                    )
                })
                .collect();
            println!(
                "{}",
                serde_json::json!({
                    "scenario": name,
                    "tick": tick,
                    "blocks": blocks,
                    "scheduled_tick_events": events,
                })
            );
        }
    }
}
