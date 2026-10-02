//! Rust replay of scenarios/redstone-trapdoor-basic.json.
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
    let hand_use = std::env::args().any(|arg| arg == "--use");
    let gate = std::env::args().any(|arg| arg == "--gate");
    let gate_use = std::env::args().any(|arg| arg == "--gate-use");
    let gate_wall = std::env::args().any(|arg| arg == "--gate-wall");
    let door = std::env::args().any(|arg| arg == "--door");
    let door_use = std::env::args().any(|arg| arg == "--door-use");
    let door_upper = std::env::args().any(|arg| arg == "--door-upper");
    let iron_door = std::env::args().any(|arg| arg == "--iron-door");
    let door_remove = std::env::args().any(|arg| arg == "--door-remove");
    if door_remove {
        run_door_removal();
        return;
    }
    if gate_wall {
        run_gate_wall();
        return;
    }
    let lever = (0, 81, 0);
    let trapdoor = (1, 81, 0);
    let upper = (1, 82, 0);
    let upper_source = (0, 82, 0);
    let mut positions = if door_use {
        vec![trapdoor, upper]
    } else if gate_use {
        vec![trapdoor]
    } else if door_upper {
        vec![upper_source, trapdoor, upper]
    } else {
        vec![lever, trapdoor]
    };
    if (door || iron_door) && !door_use {
        positions.push(upper);
    }
    let mut world = Circuit::default();
    if door || door_use || door_upper || iron_door {
        world.set_block((1, 80, 0), Some(Block::new("minecraft:stone")));
    }
    if !door_use && !door_upper {
        world.set_block(
            lever,
            Some(
                Block::new("minecraft:lever")
                    .with("face", "floor")
                    .with("facing", "north")
                    .with("powered", "false"),
            ),
        );
    }
    world.set_block(
        trapdoor,
        Some(if door || door_use || door_upper || iron_door {
            Block::new(if iron_door {
                "minecraft:iron_door"
            } else {
                "minecraft:oak_door"
            })
            .with("facing", "north")
            .with("half", "lower")
            .with("hinge", "left")
            .with("open", "false")
            .with("powered", "false")
        } else if gate || gate_use {
            Block::new("minecraft:oak_fence_gate")
                .with("facing", "north")
                .with("in_wall", "false")
                .with("open", "false")
                .with("powered", "false")
        } else {
            Block::new("minecraft:oak_trapdoor")
                .with("facing", "north")
                .with("half", "bottom")
                .with("open", "false")
                .with("powered", "false")
                .with("waterlogged", "false")
        }),
    );
    if door || door_use || door_upper || iron_door {
        world.set_block(
            upper,
            Some(
                Block::new(if iron_door {
                    "minecraft:iron_door"
                } else {
                    "minecraft:oak_door"
                })
                .with("facing", "north")
                .with("half", "upper")
                .with("hinge", "left")
                .with("open", "false")
                .with("powered", "false"),
            ),
        );
    }
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=if hand_use || door_use {
        9
    } else if gate_use {
        11
    } else {
        12
    } {
        if hand_use && (tick == 2 || tick == 5) {
            engine.use_block(&mut world, trapdoor, tick - 1).unwrap();
        }
        if door_use && (tick == 2 || tick == 5) {
            engine
                .use_block(
                    &mut world,
                    if tick == 2 { trapdoor } else { upper },
                    tick - 1,
                )
                .unwrap();
        }
        if gate_use && (tick == 2 || tick == 5 || tick == 8) {
            let facing = if tick == 2 { "south" } else { "north" };
            engine
                .use_block_facing(&mut world, trapdoor, tick - 1, facing)
                .unwrap();
        }
        if door_upper && (tick == 2 || tick == 7) {
            world.set_block(
                upper_source,
                (tick == 2).then(|| Block::new("minecraft:redstone_block")),
            );
            engine.changed(&mut world, &[upper_source], tick - 1);
        }
        if !hand_use && !door_use && !door_upper && !gate_use && (tick == 2 || tick == 7) {
            let mut block = world.block(lever).unwrap();
            block
                .properties
                .insert("powered".into(), (tick == 2).to_string());
            world.set_block(lever, Some(block));
            engine.changed(&mut world, &[lever], tick - 1);
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

fn run_gate_wall() {
    let gate = (1, 81, 0);
    let west = (0, 81, 0);
    let east = (2, 81, 0);
    let mut world = Circuit::default();
    world.set_block(
        gate,
        Some(
            Block::new("minecraft:oak_fence_gate")
                .with("facing", "north")
                .with("in_wall", "false")
                .with("open", "false")
                .with("powered", "false"),
        ),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &[gate], 0);
    for tick in 0..=11 {
        let action = match tick {
            2 => Some((west, true)),
            4 => Some((east, true)),
            6 => Some((west, false)),
            8 => Some((east, false)),
            _ => None,
        };
        if let Some((pos, place)) = action {
            world.set_block(pos, place.then(|| Block::new("minecraft:cobblestone_wall")));
            engine.changed(&mut world, &[pos], tick - 1);
        }
        engine.tick(&mut world, tick);
        let block = world.block(gate).unwrap();
        println!(
            "{}",
            serde_json::json!({"tick": tick, "blocks": [
                {"id": block.id, "properties": block.properties}
            ]})
        );
    }
}

fn run_door_removal() {
    let positions = [
        (1, 81, 0),
        (1, 82, 0),
        (3, 81, 0),
        (3, 82, 0),
        (5, 80, 0),
        (5, 81, 0),
        (5, 82, 0),
    ];
    let mut world = Circuit::default();
    for x in [1, 3, 5] {
        world.set_block((x, 80, 0), Some(Block::new("minecraft:stone")));
        for (y, half) in [(81, "lower"), (82, "upper")] {
            world.set_block(
                (x, y, 0),
                Some(
                    Block::new("minecraft:oak_door")
                        .with("facing", "north")
                        .with("half", half)
                        .with("hinge", "left")
                        .with("open", "false")
                        .with("powered", "false"),
                ),
            );
        }
    }
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=9 {
        if let Some(pos) = match tick {
            2 => Some((1, 81, 0)),
            4 => Some((3, 82, 0)),
            6 => Some((5, 80, 0)),
            _ => None,
        } {
            world.set_block(pos, None);
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
        println!("{}", serde_json::json!({"tick": tick, "blocks": blocks}));
    }
}
