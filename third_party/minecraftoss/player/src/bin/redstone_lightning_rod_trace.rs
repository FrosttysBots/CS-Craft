//! Replay of scenarios/redstone-lightning-rod.json after a paired vanilla gate.
use minecraftoss_player::{
    lightning::clean_struck_copper, redstone::RedstoneEngine, Block, Pos, World,
};
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
    let direction_case = std::env::args().any(|arg| arg == "--direction");
    let weathered_case = std::env::args().any(|arg| arg == "--weathered");
    let rod = (0, 81, 0);
    let wire = (1, 81, 0);
    let lamp = (2, 81, 0);
    let mut world = Circuit::default();
    world.set_block(
        rod,
        Some(
            Block::new(if weathered_case {
                "minecraft:exposed_lightning_rod"
            } else {
                "minecraft:lightning_rod"
            })
            .with("facing", if direction_case { "east" } else { "up" })
            .with("powered", "false")
            .with("waterlogged", "false"),
        ),
    );
    let positions = if direction_case {
        for x in [-2, 2] {
            world.set_block(
                (x, 81, 0),
                Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
            );
        }
        for x in [-1, 1] {
            world.set_block((x, 81, 0), Some(Block::new("minecraft:stone")));
        }
        vec![(-2, 81, 0), (-1, 81, 0), rod, (1, 81, 0), lamp]
    } else {
        world.set_block(
            wire,
            Some(Block::new("minecraft:redstone_wire").with("power", "0")),
        );
        world.set_block(
            lamp,
            Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
        );
        vec![rod, wire, lamp]
    };
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=15 {
        if tick == 2 {
            // Summon runs at START_SERVER_TICK, then the bolt's first entity
            // tick powers the rod after the scheduled-tick phase.
            engine.lightning_strike(&mut world, rod, tick);
            if weathered_case {
                clean_struck_copper(&mut world, rod);
                engine.copper_replaced_lightning_rod(&world, rod, tick);
            }
        }
        engine.tick(&mut world, tick);
        let blocks: Vec<_> = positions
            .iter()
            .copied()
            .map(|pos| {
                let b = world.block(pos).unwrap();
                serde_json::json!({"id": b.id, "properties": b.properties})
            })
            .collect();
        let events: Vec<_> = engine
            .take_executed_ticks()
            .into_iter()
            .map(|(pos, id)| serde_json::json!({"block": id, "position": [pos.0, pos.1, pos.2]}))
            .collect();
        println!(
            "{}",
            serde_json::json!({"tick": tick, "blocks": blocks, "scheduled_tick_events": events})
        );
    }
}
