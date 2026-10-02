//! Rust replay of scenarios/redstone-observer-basic.json.
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
    if std::env::args().any(|arg| arg == "--chain") {
        run_chain();
        return;
    }
    if std::env::args().any(|arg| arg == "--vertical") {
        run_vertical();
        return;
    }
    let rapid = std::env::args().any(|arg| arg == "--rapid");
    let front = (0, 81, 0);
    let observer = (1, 81, 0);
    let lamp = (2, 81, 0);
    let positions = [front, observer, lamp];
    let mut world = Circuit::default();
    world.set_block(front, Some(Block::new("minecraft:stone")));
    world.set_block(
        observer,
        Some(
            Block::new("minecraft:observer")
                .with("facing", "west")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        lamp,
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=if rapid { 20 } else { 23 } {
        if if rapid {
            (6..=9).contains(&tick)
        } else {
            tick == 6 || tick == 12
        } {
            let dirt = if rapid { tick % 2 == 0 } else { tick == 6 };
            world.set_block(
                front,
                Some(Block::new(if dirt {
                    "minecraft:dirt"
                } else {
                    "minecraft:stone"
                })),
            );
            engine.changed(&mut world, &[front], tick - 1);
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

fn run_chain() {
    let positions = [(0, 81, 0), (1, 81, 0), (2, 81, 0), (3, 81, 0)];
    let mut world = Circuit::default();
    world.set_block(positions[0], Some(Block::new("minecraft:stone")));
    for &pos in &positions[1..3] {
        world.set_block(
            pos,
            Some(
                Block::new("minecraft:observer")
                    .with("facing", "west")
                    .with("powered", "false"),
            ),
        );
    }
    world.set_block(
        positions[3],
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=21 {
        if tick == 8 {
            world.set_block(positions[0], Some(Block::new("minecraft:dirt")));
            engine.changed(&mut world, &positions[..1], tick - 1);
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

fn run_vertical() {
    let watched_down = (0, 81, 0);
    let observer_down = (0, 82, 0);
    let lamp_up = (0, 83, 0);
    let watched_up = (4, 83, 0);
    let observer_up = (4, 82, 0);
    let lamp_down = (4, 81, 0);
    let positions = [
        watched_down,
        observer_down,
        lamp_up,
        lamp_down,
        observer_up,
        watched_up,
    ];
    let mut world = Circuit::default();
    for watched in [watched_down, watched_up] {
        world.set_block(watched, Some(Block::new("minecraft:stone")));
    }
    world.set_block(
        observer_down,
        Some(
            Block::new("minecraft:observer")
                .with("facing", "down")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        observer_up,
        Some(
            Block::new("minecraft:observer")
                .with("facing", "up")
                .with("powered", "false"),
        ),
    );
    for lamp in [lamp_up, lamp_down] {
        world.set_block(
            lamp,
            Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
        );
    }
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=23 {
        if tick == 6 || tick == 12 {
            for watched in [watched_down, watched_up] {
                world.set_block(
                    watched,
                    Some(Block::new(if tick == 6 {
                        "minecraft:dirt"
                    } else {
                        "minecraft:stone"
                    })),
                );
                engine.changed(&mut world, &[watched], tick - 1);
            }
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
