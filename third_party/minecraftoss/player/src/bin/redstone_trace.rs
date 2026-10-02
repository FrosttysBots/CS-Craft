//! Rust replay of scenarios/redstone-lever-lamp.json for the exact harness gate.
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
    let args: Vec<String> = std::env::args().collect();
    let button_case = args.iter().any(|arg| arg == "--button");
    let line_case = args.iter().any(|arg| arg == "--line");
    let branch_case = args.iter().any(|arg| arg == "--branch");
    let wire_count = if line_case {
        4
    } else if branch_case {
        2
    } else {
        1
    };
    let x = if button_case { 10 } else { 0 };
    let lever = (x, 81, 0);
    let wires: Vec<Pos> = (1..=wire_count).map(|n| (x + n, 81, 0)).collect();
    let lamp = (x + wire_count + 1, 81, 0);
    let mut world = Circuit::default();
    world.set_block(
        lever,
        Some(
            Block::new(if button_case {
                "minecraft:stone_button"
            } else {
                "minecraft:lever"
            })
            .with("face", "floor")
            .with("facing", "north")
            .with("powered", "false"),
        ),
    );
    for &wire in &wires {
        world.set_block(
            wire,
            Some(Block::new("minecraft:redstone_wire").with("power", "0")),
        );
    }
    let branch = (x + 2, 81, 1);
    if branch_case {
        world.set_block(
            branch,
            Some(Block::new("minecraft:redstone_wire").with("power", "0")),
        );
    }
    world.set_block(
        lamp,
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    let mut engine = RedstoneEngine::default();
    let mut positions = vec![lever];
    positions.extend(&wires);
    if branch_case {
        positions.push(branch);
    }
    positions.push(lamp);
    engine.changed(&mut world, &positions, 0);

    for tick in 0..=if button_case {
        28u64
    } else if branch_case {
        10u64
    } else {
        12u64
    } {
        if tick == 2 || (!button_case && tick == 5) {
            // Fabric START_SERVER_TICK actions precede the world's scheduled
            // tick phase. At snapshot N the pre-phase game time is N-1.
            engine.use_block(&mut world, lever, tick - 1).unwrap();
        }
        engine.tick(&mut world, tick);
        let states: Vec<_> = positions
            .iter()
            .map(|&pos| {
                let block = world.block(pos).unwrap();
                serde_json::json!({"id": block.id, "properties": block.properties})
            })
            .collect();
        println!("{}", serde_json::json!({"tick": tick, "blocks": states}));
    }
}
