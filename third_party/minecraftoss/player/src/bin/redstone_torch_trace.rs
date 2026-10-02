//! Rust replay of scenarios/redstone-torch-basic.json.
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
    let burnout = std::env::args().any(|arg| arg == "--burnout");
    let support = (0, 81, 0);
    let torch = (0, 82, 0);
    let lamp = (1, 82, 0);
    let positions = [support, torch, lamp];
    let mut world = Circuit::default();
    world.set_block(
        support,
        Some(Block::new(if burnout {
            "minecraft:stone"
        } else {
            "minecraft:redstone_block"
        })),
    );
    world.set_block(
        torch,
        Some(
            Block::new("minecraft:redstone_torch")
                .with("lit", if burnout { "true" } else { "false" }),
        ),
    );
    world.set_block(
        lamp,
        Some(
            Block::new("minecraft:redstone_lamp")
                .with("lit", if burnout { "true" } else { "false" }),
        ),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=if burnout { 210_u64 } else { 16_u64 } {
        let burnout_switch = burnout && tick >= 2 && tick <= 47 && (tick % 6 == 2 || tick % 6 == 5);
        if burnout_switch || !burnout && (tick == 2 || tick == 8) {
            let support_id = if burnout {
                if tick % 6 == 2 {
                    "minecraft:redstone_block"
                } else {
                    "minecraft:stone"
                }
            } else if tick == 2 {
                "minecraft:stone"
            } else {
                "minecraft:redstone_block"
            };
            world.set_block(support, Some(Block::new(support_id)));
            // Fabric START_SERVER_TICK actions precede the scheduled world tick;
            // snapshot N's pre-phase game time is N-1.
            engine.changed(&mut world, &[support], tick - 1);
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
