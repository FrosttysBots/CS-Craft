//! Rust replay of scenarios/redstone-wall-torch-basic.json.
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
    let support_removal = std::env::args().any(|arg| arg == "--support-removal");
    let support = (0, 81, 0);
    let torch = (1, 81, 0);
    let east_lamp = (2, 81, 0);
    let north_lamp = (1, 81, -1);
    let positions = if support_removal {
        vec![support, torch, east_lamp]
    } else {
        vec![support, torch, east_lamp, north_lamp]
    };
    let mut world = Circuit::default();
    world.set_block(
        support,
        Some(Block::new(if support_removal {
            "minecraft:stone"
        } else {
            "minecraft:redstone_block"
        })),
    );
    world.set_block(
        torch,
        Some(
            Block::new("minecraft:redstone_wall_torch")
                .with("facing", "east")
                .with("lit", if support_removal { "true" } else { "false" }),
        ),
    );
    for &pos in positions.iter().skip(2) {
        world.set_block(
            pos,
            Some(
                Block::new("minecraft:redstone_lamp")
                    .with("lit", if support_removal { "true" } else { "false" }),
            ),
        );
    }
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=if support_removal { 10u64 } else { 16u64 } {
        if tick == 2 || (!support_removal && tick == 8) {
            let block = if support_removal {
                None
            } else {
                Some(Block::new(if tick == 2 {
                    "minecraft:stone"
                } else {
                    "minecraft:redstone_block"
                }))
            };
            world.set_block(support, block);
            engine.changed(&mut world, &[support], tick - 1);
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
        println!("{}", serde_json::json!({"tick": tick, "blocks": blocks}));
    }
}
