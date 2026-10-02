//! Rust replay of scenarios/redstone-comparator-through-block.json.
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
    let positions: [Pos; 5] = [(0, 81, 0), (1, 81, 0), (2, 81, 0), (3, 81, 0), (4, 81, 0)];
    let mut world = Circuit::default();
    world.set_block(
        positions[0],
        Some(Block::new("minecraft:cake").with("bites", "0")),
    );
    world.set_block(positions[1], Some(Block::new("minecraft:stone")));
    world.set_block(
        positions[2],
        Some(
            Block::new("minecraft:comparator")
                .with("facing", "west")
                .with("mode", "compare")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        positions[3],
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
        positions[4],
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=25 {
        let edit = match tick {
            4 => Some((
                positions[0],
                Some(Block::new("minecraft:cake").with("bites", "3")),
            )),
            8 => Some((positions[1], Some(Block::new("minecraft:glass")))),
            12 => Some((positions[1], Some(Block::new("minecraft:stone")))),
            16 => Some((
                positions[0],
                Some(Block::new("minecraft:cake").with("bites", "6")),
            )),
            20 => Some((positions[0], None)),
            _ => None,
        };
        if let Some((pos, block)) = edit {
            world.set_block(pos, block);
            engine.changed(&mut world, &[pos], tick - 1);
        }
        engine.tick(&mut world, tick);
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
                "tick": tick,
                "blocks": blocks,
                "comparator_output": engine.comparator_output(positions[2]),
            })
        );
    }
}
