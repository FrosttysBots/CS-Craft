//! Replay the pinned straight powered-rail chain against two server traces.
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

fn rail_pos(index: i32, north_south: bool, slope: bool) -> Pos {
    if north_south {
        (0, 81, index)
    } else {
        (index, if slope && index >= 7 { 82 } else { 81 }, 0)
    }
}

fn replay(north_south: bool, slope: bool, rail_id: &str, mixed: bool) -> Vec<Value> {
    let mut world = Circuit::default();
    for along in -2..=11 {
        for across in -1..=1 {
            let support = if north_south {
                (across, 80, along)
            } else {
                (along, 80, across)
            };
            world.set_block(support, Some(Block::new("minecraft:stone")));
        }
    }
    if slope {
        for x in 7..=11 {
            for z in -1..=1 {
                world.set_block((x, 81, z), Some(Block::new("minecraft:stone")));
            }
        }
    }
    for index in 0..=10 {
        world.set_block(
            rail_pos(index, north_south, slope),
            Some(
                Block::new(if mixed && index == 5 {
                    "minecraft:powered_rail"
                } else {
                    rail_id
                })
                .with(
                    "shape",
                    if slope && index == 6 {
                        "ascending_east"
                    } else if north_south {
                        "north_south"
                    } else {
                        "east_west"
                    },
                )
                .with("powered", "false")
                .with("waterlogged", "false"),
            ),
        );
    }
    world.set_block(
        rail_pos(-1, north_south, slope),
        Some(
            Block::new("minecraft:lever")
                .with("face", "floor")
                .with("facing", "north")
                .with("powered", "false"),
        ),
    );
    let mut engine = RedstoneEngine::default();
    let mut output = Vec::new();
    for tick in 0..=9 {
        if tick == 2 || tick == 5 {
            engine
                .use_block(&mut world, rail_pos(-1, north_south, slope), tick)
                .expect("lever use");
        }
        engine.tick(&mut world, tick);
        let positions: Vec<_> = if slope {
            (-1..=10).flat_map(|x| [(x, 81, 0), (x, 82, 0)]).collect()
        } else {
            (-1..=10)
                .map(|index| rail_pos(index, north_south, slope))
                .collect()
        };
        let blocks: serde_json::Map<_, _> = positions
            .into_iter()
            .map(|pos| {
                let block = world
                    .block(pos)
                    .unwrap_or_else(|| Block::new("minecraft:air"));
                (
                    format!("{},{},{}", pos.0, pos.1, pos.2),
                    json!({"id": block.id, "properties": block.properties}),
                )
            })
            .collect();
        output.push(Value::Object(serde_json::Map::from_iter([
            ("tick".into(), json!(tick)),
            ("blocks".into(), Value::Object(blocks)),
        ])));
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
    for (pos, shape) in [((0, 81, 0), "east_west"), ((4, 81, 0), "ascending_east")] {
        world.set_block(
            pos,
            Some(
                Block::new("minecraft:powered_rail")
                    .with("shape", shape)
                    .with("powered", "false")
                    .with("waterlogged", "false"),
            ),
        );
    }
    let mut engine = RedstoneEngine::default();
    let mut output = Vec::new();
    for tick in 0..=8 {
        if tick == 2 || tick == 4 {
            let pos = if tick == 2 { (0, 80, 0) } else { (5, 81, 0) };
            world.set_block(pos, None);
            engine.changed(&mut world, &[pos], tick);
        }
        engine.tick(&mut world, tick);
        let blocks: serde_json::Map<_, _> = (80..=81)
            .flat_map(|y| (0..=5).map(move |x| (x, y, 0)))
            .map(|pos| {
                let block = world
                    .block(pos)
                    .unwrap_or_else(|| Block::new("minecraft:air"));
                (
                    format!("{},{},{}", pos.0, pos.1, pos.2),
                    json!({"id": block.id, "properties": block.properties}),
                )
            })
            .collect();
        output.push(json!({"tick": tick, "blocks": blocks}));
    }
    output
}

fn replay_oriented_slope(direction: &str) -> Vec<Value> {
    let north_south = matches!(direction, "north" | "south");
    let reverse = matches!(direction, "west" | "north");
    let at = |along: i32, y: i32, across: i32| {
        let along = if reverse { 10 - along } else { along };
        if north_south {
            (across, y, along)
        } else {
            (along, y, across)
        }
    };
    let mut world = Circuit::default();
    for along in -2..=6 {
        for across in -1..=1 {
            world.set_block(at(along, 80, across), Some(Block::new("minecraft:stone")));
        }
    }
    for along in 7..=11 {
        for across in -1..=1 {
            world.set_block(at(along, 81, across), Some(Block::new("minecraft:stone")));
        }
    }
    for along in 0..=10 {
        world.set_block(
            at(along, if along >= 7 { 82 } else { 81 }, 0),
            Some(
                Block::new("minecraft:powered_rail")
                    .with(
                        "shape",
                        if along == 6 {
                            match direction {
                                "west" => "ascending_west",
                                "north" => "ascending_north",
                                "south" => "ascending_south",
                                _ => unreachable!(),
                            }
                        } else if north_south {
                            "north_south"
                        } else {
                            "east_west"
                        },
                    )
                    .with("powered", "false")
                    .with("waterlogged", "false"),
            ),
        );
    }
    let lever = at(-1, 81, 0);
    world.set_block(
        lever,
        Some(
            Block::new("minecraft:lever")
                .with("face", "floor")
                .with("facing", "north")
                .with("powered", "false"),
        ),
    );
    let mut engine = RedstoneEngine::default();
    let mut output = Vec::new();
    for tick in 0..=9 {
        if tick == 2 || tick == 5 {
            engine
                .use_block(&mut world, lever, tick)
                .expect("lever use");
        }
        engine.tick(&mut world, tick);
        let (start, end) = if reverse { (0, 11) } else { (-1, 10) };
        let blocks: serde_json::Map<_, _> = (81..=82)
            .flat_map(|y| {
                (start..=end).map(move |along| {
                    if north_south {
                        (0, y, along)
                    } else {
                        (along, y, 0)
                    }
                })
            })
            .map(|pos| {
                let block = world
                    .block(pos)
                    .unwrap_or_else(|| Block::new("minecraft:air"));
                (
                    format!("{},{},{}", pos.0, pos.1, pos.2),
                    json!({"id": block.id, "properties": block.properties}),
                )
            })
            .collect();
        output.push(json!({"tick": tick, "blocks": blocks}));
    }
    output
}

fn replay_support_shapes() -> Vec<Value> {
    replay_support_matrix([
        (0, Block::new("minecraft:glass")),
        (
            3,
            Block::new("minecraft:oak_slab")
                .with("type", "top")
                .with("waterlogged", "false"),
        ),
        (
            6,
            Block::new("minecraft:oak_slab")
                .with("type", "bottom")
                .with("waterlogged", "false"),
        ),
        (
            9,
            Block::new("minecraft:oak_stairs")
                .with("facing", "north")
                .with("half", "top")
                .with("shape", "straight")
                .with("waterlogged", "false"),
        ),
        (
            12,
            Block::new("minecraft:oak_stairs")
                .with("facing", "north")
                .with("half", "bottom")
                .with("shape", "straight")
                .with("waterlogged", "false"),
        ),
        (
            15,
            Block::new("minecraft:chest")
                .with("facing", "north")
                .with("type", "single")
                .with("waterlogged", "false"),
        ),
    ])
}

fn replay_support_special() -> Vec<Value> {
    replay_support_matrix([
        (
            0,
            Block::new("minecraft:oak_fence")
                .with("east", "true")
                .with("north", "true")
                .with("south", "true")
                .with("west", "true")
                .with("waterlogged", "false"),
        ),
        (
            3,
            Block::new("minecraft:iron_bars")
                .with("east", "true")
                .with("north", "true")
                .with("south", "true")
                .with("west", "true")
                .with("waterlogged", "false"),
        ),
        (
            6,
            Block::new("minecraft:oak_trapdoor")
                .with("facing", "north")
                .with("half", "top")
                .with("open", "false")
                .with("powered", "false")
                .with("waterlogged", "false"),
        ),
        (9, Block::new("minecraft:farmland").with("moisture", "0")),
        (12, Block::new("minecraft:dirt_path")),
        (15, Block::new("minecraft:cauldron")),
    ])
}

fn replay_support_matrix(supports: [(i32, Block); 6]) -> Vec<Value> {
    let mut world = Circuit::default();
    for x in -1..=16 {
        for z in -1..=1 {
            world.set_block((x, 80, z), Some(Block::new("minecraft:stone")));
        }
    }
    for x in [0, 3, 6, 9, 12, 15] {
        world.set_block(
            (x, 81, 0),
            Some(
                Block::new("minecraft:powered_rail")
                    .with("shape", "east_west")
                    .with("powered", "false")
                    .with("waterlogged", "false"),
            ),
        );
    }
    let mut engine = RedstoneEngine::default();
    let mut output = Vec::new();
    for tick in 0..=5 {
        if tick == 2 {
            for (x, block) in &supports {
                let pos = (*x, 80, 0);
                world.set_block(pos, Some(block.clone()));
                engine.changed(&mut world, &[pos], tick);
            }
        }
        engine.tick(&mut world, tick);
        let blocks: serde_json::Map<_, _> = (80..=81)
            .flat_map(|y| (0..=15).map(move |x| (x, y, 0)))
            .map(|pos| {
                let block = world
                    .block(pos)
                    .unwrap_or_else(|| Block::new("minecraft:air"));
                (
                    format!("{},{},{}", pos.0, pos.1, pos.2),
                    json!({"id": block.id, "properties": block.properties}),
                )
            })
            .collect();
        output.push(json!({"tick": tick, "blocks": blocks}));
    }
    output
}

fn check(path: &Path, expected: &[Value], scenario: &str) -> Result<()> {
    let raw = fs::read_to_string(path)?;
    let mut snapshots = Vec::new();
    for line in raw.lines() {
        let record: Value = serde_json::from_str(line)?;
        if record["type"] == "snapshot" && record["scenario"] == scenario {
            snapshots.push(record);
        }
    }
    if snapshots.len() != expected.len() {
        bail!(
            "{}: expected {} snapshots, found {}",
            path.display(),
            expected.len(),
            snapshots.len()
        );
    }
    for (index, (actual, expected)) in snapshots.iter().zip(expected).enumerate() {
        if actual["tick"] != expected["tick"] || actual["data"]["blocks"] != expected["blocks"] {
            if let (Some(reference), Some(rust)) = (
                actual["data"]["blocks"].as_object(),
                expected["blocks"].as_object(),
            ) {
                for (pos, block) in rust {
                    if reference.get(pos) != Some(block) {
                        bail!(
                            "{}: tick {} block {pos}: vanilla {} vs Rust {block}",
                            path.display(),
                            actual["tick"],
                            reference.get(pos).unwrap_or(&Value::Null)
                        );
                    }
                }
            }
            bail!(
                "{}: first mismatch at snapshot {index}, tick {}",
                path.display(),
                actual["tick"]
            );
        }
        if actual["data"]["scheduled_tick_events"]
            .as_array()
            .context("missing tick events")?
            .len()
            != 0
        {
            bail!("{}: unexpected scheduled tick at {index}", path.display());
        }
    }
    println!(
        "{}: {} exact circuit snapshots",
        path.display(),
        snapshots.len()
    );
    Ok(())
}

fn main() -> Result<()> {
    let mut paths: Vec<_> = env::args().skip(1).collect();
    let north_south = paths.first().is_some_and(|arg| arg == "--north-south");
    let slope = paths.first().is_some_and(|arg| arg == "--slope");
    let west_slope = paths.first().is_some_and(|arg| arg == "--west-slope");
    let north_slope = paths.first().is_some_and(|arg| arg == "--north-slope");
    let south_slope = paths.first().is_some_and(|arg| arg == "--south-slope");
    let support = paths.first().is_some_and(|arg| arg == "--support");
    let support_shapes = paths.first().is_some_and(|arg| arg == "--support-shapes");
    let support_special = paths.first().is_some_and(|arg| arg == "--support-special");
    let activator = paths.first().is_some_and(|arg| arg == "--activator");
    let mixed = paths.first().is_some_and(|arg| arg == "--mixed");
    if north_south
        || slope
        || west_slope
        || north_slope
        || south_slope
        || support
        || support_shapes
        || support_special
        || activator
        || mixed
    {
        paths.remove(0);
    }
    if paths.len() != 2 {
        bail!(
            "usage: redstone_powered_rail_trace [--north-south|--slope|--west-slope|--north-slope|--south-slope|--support|--support-shapes|--support-special|--activator|--mixed] FIRST_TRACE SECOND_TRACE"
        );
    }
    let expected = if west_slope {
        replay_oriented_slope("west")
    } else if north_slope {
        replay_oriented_slope("north")
    } else if south_slope {
        replay_oriented_slope("south")
    } else if support_special {
        replay_support_special()
    } else if support_shapes {
        replay_support_shapes()
    } else if support {
        replay_support()
    } else {
        replay(
            north_south,
            slope,
            if activator || mixed {
                "minecraft:activator_rail"
            } else {
                "minecraft:powered_rail"
            },
            mixed,
        )
    };
    let scenario = if west_slope {
        "powered_rail_west_ascending_chain"
    } else if north_slope {
        "powered_rail_north_ascending_chain"
    } else if south_slope {
        "powered_rail_south_ascending_chain"
    } else if support_special {
        "powered_rail_support_special"
    } else if support_shapes {
        "powered_rail_support_shapes"
    } else if support {
        "powered_rail_support_loss"
    } else if activator {
        "activator_rail_straight_chain"
    } else if mixed {
        "activator_rail_mixed_chain"
    } else if slope {
        "powered_rail_east_ascending_chain"
    } else if north_south {
        "powered_rail_north_south_chain"
    } else {
        "powered_rail_straight_chain"
    };
    for path in paths {
        check(Path::new(&path), &expected, scenario)?;
    }
    Ok(())
}
