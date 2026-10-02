//! Replay ordinary-rail corner placement and removal against paired 26.3 traces.
use anyhow::{bail, Context, Result};
use minecraftoss_player::{redstone::RedstoneEngine, Block, Pos, World};
use serde_json::{json, Value};
use std::{collections::BTreeMap, env, fs, path::Path};

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

fn ordinary_rail(shape: &str) -> Block {
    Block::new("minecraft:rail")
        .with("shape", shape)
        .with("waterlogged", "false")
}

fn place(world: &mut Circuit, engine: &mut RedstoneEngine, pos: Pos, shape: &str, tick: u64) {
    world.set_block(pos, Some(ordinary_rail(shape)));
    let mut changed = engine.rail_placed(world, pos);
    changed.push(pos);
    engine.changed(world, &changed, tick);
}

fn replay() -> Vec<Value> {
    let mut world = Circuit::default();
    for x in -2..=2 {
        for z in -2..=2 {
            world.set_block((x, 80, z), Some(Block::new("minecraft:stone")));
        }
    }
    let mut engine = RedstoneEngine::default();
    place(&mut world, &mut engine, (0, 81, 0), "north_south", 0);
    place(&mut world, &mut engine, (0, 81, -1), "north_south", 0);
    let mut output = Vec::new();
    for tick in 0..=8 {
        match tick {
            2 => place(&mut world, &mut engine, (1, 81, 0), "east_west", tick),
            4 => {
                world.set_block((1, 81, 0), None);
                engine.changed(&mut world, &[(1, 81, 0)], tick);
            }
            6 => place(&mut world, &mut engine, (0, 81, 1), "north_south", tick),
            _ => {}
        }
        engine.tick(&mut world, tick);
        let mut blocks = serde_json::Map::new();
        for z in -1..=1 {
            for x in -1..=1 {
                let block = world
                    .block((x, 81, z))
                    .unwrap_or_else(|| Block::new("minecraft:air"));
                blocks.insert(
                    format!("{x},81,{z}"),
                    json!({"id": block.id, "properties": block.properties}),
                );
            }
        }
        output.push(json!({"tick": tick, "blocks": blocks, "scheduled_tick_events": []}));
    }
    output
}

fn replay_junction() -> Vec<Value> {
    let mut world = Circuit::default();
    for x in -2..=2 {
        for z in -2..=2 {
            world.set_block((x, 80, z), Some(Block::new("minecraft:stone")));
        }
    }
    let mut engine = RedstoneEngine::default();
    place(&mut world, &mut engine, (0, 81, -1), "north_south", 0);
    place(&mut world, &mut engine, (0, 81, 1), "north_south", 0);
    place(&mut world, &mut engine, (1, 81, 0), "east_west", 0);
    place(&mut world, &mut engine, (0, 81, 0), "north_south", 0);
    let mut output = Vec::new();
    for tick in 0..=8 {
        match tick {
            2 => {
                world.set_block((-1, 81, 0), Some(Block::new("minecraft:redstone_block")));
                engine.changed(&mut world, &[(-1, 81, 0)], tick);
            }
            4 => {
                world.set_block((-1, 81, 0), None);
                engine.changed(&mut world, &[(-1, 81, 0)], tick);
            }
            _ => {}
        }
        engine.tick(&mut world, tick);
        let mut blocks = serde_json::Map::new();
        for z in -1..=1 {
            for x in -1..=1 {
                let block = world
                    .block((x, 81, z))
                    .unwrap_or_else(|| Block::new("minecraft:air"));
                blocks.insert(
                    format!("{x},81,{z}"),
                    json!({"id": block.id, "properties": block.properties}),
                );
            }
        }
        output.push(json!({"tick": tick, "blocks": blocks, "scheduled_tick_events": []}));
    }
    output
}

fn replay_lever_junction() -> Vec<Value> {
    let mut world = Circuit::default();
    for x in -2..=2 {
        for z in -2..=2 {
            world.set_block((x, 80, z), Some(Block::new("minecraft:stone")));
        }
    }
    let mut engine = RedstoneEngine::default();
    place(&mut world, &mut engine, (0, 81, -1), "north_south", 0);
    place(&mut world, &mut engine, (0, 81, 1), "north_south", 0);
    place(&mut world, &mut engine, (1, 81, 0), "east_west", 0);
    place(&mut world, &mut engine, (0, 81, 0), "north_south", 0);
    let lever = (-1, 81, 0);
    world.set_block(
        lever,
        Some(
            Block::new("minecraft:lever")
                .with("face", "floor")
                .with("facing", "north")
                .with("powered", "false"),
        ),
    );
    engine.changed(&mut world, &[lever], 0);
    let mut output = Vec::new();
    for tick in 0..=8 {
        if tick == 2 || tick == 4 {
            engine.use_block(&mut world, lever, tick).unwrap();
        } else if tick == 6 {
            world.set_block(lever, None);
            engine.changed(&mut world, &[lever], tick);
        }
        engine.tick(&mut world, tick);
        let mut blocks = serde_json::Map::new();
        for z in -1..=1 {
            for x in -1..=1 {
                let block = world
                    .block((x, 81, z))
                    .unwrap_or_else(|| Block::new("minecraft:air"));
                blocks.insert(
                    format!("{x},81,{z}"),
                    json!({"id": block.id, "properties": block.properties}),
                );
            }
        }
        output.push(json!({"tick": tick, "blocks": blocks, "scheduled_tick_events": []}));
    }
    output
}

fn replay_button_junction() -> Vec<Value> {
    let mut world = Circuit::default();
    for x in -2..=2 {
        for z in -2..=2 {
            world.set_block((x, 80, z), Some(Block::new("minecraft:stone")));
        }
    }
    let mut engine = RedstoneEngine::default();
    place(&mut world, &mut engine, (0, 81, -1), "north_south", 0);
    place(&mut world, &mut engine, (0, 81, 1), "north_south", 0);
    place(&mut world, &mut engine, (1, 81, 0), "east_west", 0);
    place(&mut world, &mut engine, (0, 81, 0), "north_south", 0);
    let button = (-1, 81, 0);
    world.set_block(
        button,
        Some(
            Block::new("minecraft:stone_button")
                .with("face", "floor")
                .with("facing", "north")
                .with("powered", "false"),
        ),
    );
    engine.changed(&mut world, &[button], 0);
    let mut output = Vec::new();
    for tick in 0..=25 {
        if tick == 2 {
            // START_SERVER_TICK action runs while the world's game time is N-1.
            engine.use_block(&mut world, button, tick - 1).unwrap();
        }
        engine.tick(&mut world, tick);
        let mut blocks = serde_json::Map::new();
        for z in -1..=1 {
            for x in -1..=1 {
                let block = world
                    .block((x, 81, z))
                    .unwrap_or_else(|| Block::new("minecraft:air"));
                blocks.insert(
                    format!("{x},81,{z}"),
                    json!({"id": block.id, "properties": block.properties}),
                );
            }
        }
        let scheduled_tick_events: Vec<_> = engine
            .take_executed_ticks()
            .into_iter()
            .map(|(pos, id)| json!({"block":id,"position":[pos.0,pos.1,pos.2]}))
            .collect();
        output.push(
            json!({"tick":tick,"blocks":blocks,"scheduled_tick_events":scheduled_tick_events}),
        );
    }
    output
}

fn replay_underpower_junction() -> Vec<Value> {
    let mut world = Circuit::default();
    for x in -2..=2 {
        for z in -2..=2 {
            world.set_block((x, 80, z), Some(Block::new("minecraft:stone")));
        }
    }
    let mut engine = RedstoneEngine::default();
    place(&mut world, &mut engine, (0, 81, -1), "north_south", 0);
    place(&mut world, &mut engine, (0, 81, 1), "north_south", 0);
    place(&mut world, &mut engine, (1, 81, 0), "east_west", 0);
    place(&mut world, &mut engine, (0, 81, 0), "north_south", 0);
    let support = (0, 80, 0);
    let mut output = Vec::new();
    for tick in 0..=7 {
        if tick == 2 || tick == 4 {
            world.set_block(
                support,
                Some(Block::new(if tick == 2 {
                    "minecraft:redstone_block"
                } else {
                    "minecraft:stone"
                })),
            );
            engine.changed(&mut world, &[support], tick);
        }
        engine.tick(&mut world, tick);
        let mut blocks = serde_json::Map::new();
        for y in 80..=81 {
            for z in -1..=1 {
                for x in -1..=1 {
                    let block = world
                        .block((x, y, z))
                        .unwrap_or_else(|| Block::new("minecraft:air"));
                    blocks.insert(
                        format!("{x},{y},{z}"),
                        json!({"id": block.id, "properties": block.properties}),
                    );
                }
            }
        }
        output.push(json!({"tick":tick,"blocks":blocks,"scheduled_tick_events":[]}));
    }
    output
}

fn replay_support() -> Vec<Value> {
    let mut world = Circuit::default();
    for x in -1..=6 {
        for z in -1..=1 {
            world.set_block((x, 80, z), Some(Block::new("minecraft:stone")));
        }
    }
    world.set_block((5, 81, 0), Some(Block::new("minecraft:stone")));
    world.set_block((0, 81, 0), Some(ordinary_rail("east_west")));
    world.set_block(
        (2, 81, 0),
        Some(
            Block::new("minecraft:detector_rail")
                .with("shape", "east_west")
                .with("powered", "false")
                .with("waterlogged", "false"),
        ),
    );
    world.set_block((4, 81, 0), Some(ordinary_rail("ascending_east")));
    let mut engine = RedstoneEngine::default();
    let mut output = Vec::new();
    for tick in 0..=8 {
        let removed = match tick {
            2 => Some((0, 80, 0)),
            4 => Some((2, 80, 0)),
            6 => Some((5, 81, 0)),
            _ => None,
        };
        if let Some(pos) = removed {
            world.set_block(pos, None);
            engine.changed(&mut world, &[pos], tick);
        }
        engine.tick(&mut world, tick);
        let mut blocks = serde_json::Map::new();
        for y in 80..=81 {
            for x in 0..=5 {
                let block = world
                    .block((x, y, 0))
                    .unwrap_or_else(|| Block::new("minecraft:air"));
                blocks.insert(
                    format!("{x},{y},0"),
                    json!({"id": block.id, "properties": block.properties}),
                );
            }
        }
        output.push(json!({"tick": tick, "blocks": blocks, "scheduled_tick_events": []}));
    }
    output
}

fn replay_four_way() -> Vec<Value> {
    let mut world = Circuit::default();
    for x in -2..=12 {
        for z in -2..=2 {
            world.set_block((x, 80, z), Some(Block::new("minecraft:stone")));
        }
    }
    world.set_block((10, 80, 0), Some(Block::new("minecraft:redstone_block")));
    let mut engine = RedstoneEngine::default();
    for center in [0, 10] {
        place(&mut world, &mut engine, (center, 81, -1), "north_south", 0);
        place(&mut world, &mut engine, (center, 81, 1), "north_south", 0);
        place(&mut world, &mut engine, (center + 1, 81, 0), "east_west", 0);
        place(&mut world, &mut engine, (center - 1, 81, 0), "east_west", 0);
        place(&mut world, &mut engine, (center, 81, 0), "north_south", 0);
    }
    let mut output = Vec::new();
    for tick in 0..=6 {
        if tick == 2 {
            world.set_block((10, 80, 0), Some(Block::new("minecraft:stone")));
            engine.changed(&mut world, &[(10, 80, 0)], tick);
        } else if tick == 4 {
            world.set_block((0, 80, 0), Some(Block::new("minecraft:redstone_block")));
            engine.changed(&mut world, &[(0, 80, 0)], tick);
        }
        engine.tick(&mut world, tick);
        let mut blocks = serde_json::Map::new();
        for z in -1..=1 {
            for x in -1..=11 {
                let block = world
                    .block((x, 81, z))
                    .unwrap_or_else(|| Block::new("minecraft:air"));
                blocks.insert(
                    format!("{x},81,{z}"),
                    json!({"id": block.id, "properties": block.properties}),
                );
            }
        }
        output.push(json!({"tick": tick, "blocks": blocks, "scheduled_tick_events": []}));
    }
    output
}

fn check(path: &Path, actual: &[Value]) -> Result<()> {
    let trace = fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let reference: Vec<Value> = trace
        .lines()
        .map(serde_json::from_str)
        .collect::<serde_json::Result<_>>()?;
    let snapshots: Vec<_> = reference
        .iter()
        .filter(|record| record["type"] == "snapshot")
        .collect();
    if snapshots.len() != actual.len() {
        bail!(
            "{}: {} snapshots vs {}",
            path.display(),
            snapshots.len(),
            actual.len()
        );
    }
    for (index, (record, candidate)) in snapshots.iter().zip(actual).enumerate() {
        for field in ["blocks", "scheduled_tick_events"] {
            if record["data"][field] != candidate[field] {
                bail!(
                    "{}: tick {index} {field} mismatch\nvanilla: {}\nRust: {}",
                    path.display(),
                    record["data"][field],
                    candidate[field]
                );
            }
        }
    }
    println!("{}: {} exact snapshots", path.display(), actual.len());
    Ok(())
}

fn main() -> Result<()> {
    let mut paths: Vec<_> = env::args().skip(1).collect();
    let junction = paths.first().is_some_and(|arg| arg == "--junction");
    let lever = paths.first().is_some_and(|arg| arg == "--lever");
    let button = paths.first().is_some_and(|arg| arg == "--button");
    let underpower = paths.first().is_some_and(|arg| arg == "--underpower");
    let support = paths.first().is_some_and(|arg| arg == "--support");
    let four_way = paths.first().is_some_and(|arg| arg == "--four-way");
    if junction || lever || button || underpower || support || four_way {
        paths.remove(0);
    }
    if paths.len() != 2 {
        bail!(
            "usage: ordinary_rail_trace [--junction|--lever|--button|--underpower|--support|--four-way] FIRST_TRACE SECOND_TRACE"
        );
    }
    let actual = if junction {
        replay_junction()
    } else if lever {
        replay_lever_junction()
    } else if button {
        replay_button_junction()
    } else if underpower {
        replay_underpower_junction()
    } else if support {
        replay_support()
    } else if four_way {
        replay_four_way()
    } else {
        replay()
    };
    for path in &paths {
        check(Path::new(path), &actual)?;
    }
    Ok(())
}
