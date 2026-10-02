//! Rust replay of scenarios/redstone-comparator-basic.json.
use minecraftoss_player::{
    redstone::{RedstoneEngine, UseSound},
    Block, Pos, World,
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
    if std::env::args().any(|arg| arg == "--analog-blocks") {
        run_analog_blocks();
        return;
    }
    if std::env::args().any(|arg| arg == "--cake") {
        run_cake();
        return;
    }
    if std::env::args().any(|arg| arg == "--use") {
        run_use();
        return;
    }
    let analog_wire = std::env::args().any(|arg| arg == "--analog-wire");
    if analog_wire || std::env::args().any(|arg| arg == "--analog") {
        run_analog(analog_wire);
        return;
    }
    let subtract = std::env::args().any(|arg| arg == "--subtract");
    let x = if subtract { 11 } else { 1 };
    let rear = (x - 1, 81, 0);
    let side = (x, 81, -1);
    let comparator = (x, 81, 0);
    let lamp = (x + 1, 81, 0);
    let positions = [rear, side, comparator, lamp];
    let mut world = Circuit::default();
    world.set_block(
        rear,
        Some(
            Block::new("minecraft:lever")
                .with("face", "floor")
                .with("facing", "north")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        comparator,
        Some(
            Block::new("minecraft:comparator")
                .with("facing", "west")
                .with("mode", if subtract { "subtract" } else { "compare" })
                .with("powered", "false"),
        ),
    );
    world.set_block(
        lamp,
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    let last_tick = if subtract { 15 } else { 13 };
    for tick in 0..=last_tick {
        if tick == 2 || tick == (if subtract { 11 } else { 8 }) {
            engine.use_block(&mut world, rear, tick - 1).unwrap();
        }
        if tick == 5 || tick == (if subtract { 8 } else { 11 }) {
            let next = if tick == 5 {
                Some(Block::new("minecraft:redstone_block"))
            } else {
                None
            };
            world.set_block(side, next);
            engine.changed(&mut world, &[side], tick - 1);
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
                "tick": tick, "blocks": blocks,
                "comparator_output": engine.comparator_output(comparator),
            })
        );
    }
}

fn run_analog_blocks() {
    let input = (0, 81, 0);
    let comparator = (1, 81, 0);
    let wire = (2, 81, 0);
    let lamp = (3, 81, 0);
    let positions = [input, comparator, wire, lamp];
    let mut world = Circuit::default();
    world.set_block(
        input,
        Some(Block::new("minecraft:composter").with("level", "6")),
    );
    world.set_block(
        comparator,
        Some(
            Block::new("minecraft:comparator")
                .with("facing", "west")
                .with("mode", "compare")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        wire,
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
        lamp,
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=19 {
        if let Some(replacement) = match tick {
            4 => Some(Some(
                Block::new("minecraft:water_cauldron").with("level", "2"),
            )),
            7 => Some(Some(Block::new("minecraft:lava_cauldron"))),
            10 => Some(Some(Block::new("minecraft:cauldron"))),
            13 => Some(Some(
                Block::new("minecraft:powder_snow_cauldron").with("level", "3"),
            )),
            16 => Some(None),
            _ => None,
        } {
            world.set_block(input, replacement);
            engine.changed(&mut world, &[input], tick - 1);
        }
        emit_tick(&mut engine, &mut world, &positions, comparator, tick);
    }
}

fn emit_tick(
    engine: &mut RedstoneEngine,
    world: &mut Circuit,
    positions: &[Pos; 4],
    comparator: Pos,
    tick: u64,
) {
    engine.tick(world, tick);
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
            "tick": tick, "blocks": blocks,
            "comparator_output": engine.comparator_output(comparator),
        })
    );
}

fn run_cake() {
    let cake = (0, 81, 0);
    let comparator = (1, 81, 0);
    let wire = (2, 81, 0);
    let lamp = (3, 81, 0);
    let positions = [cake, comparator, wire, lamp];
    let mut world = Circuit::default();
    world.set_block(cake, Some(Block::new("minecraft:cake").with("bites", "0")));
    world.set_block(
        comparator,
        Some(
            Block::new("minecraft:comparator")
                .with("facing", "west")
                .with("mode", "compare")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        wire,
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
        lamp,
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=17 {
        if tick == 4 || tick == 8 || tick == 12 {
            let replacement = match tick {
                4 => Some(Block::new("minecraft:cake").with("bites", "3")),
                8 => Some(Block::new("minecraft:cake").with("bites", "6")),
                _ => None,
            };
            world.set_block(cake, replacement);
            engine.changed(&mut world, &[cake], tick - 1);
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
                "tick": tick, "blocks": blocks,
                "comparator_output": engine.comparator_output(comparator),
            })
        );
    }
}

fn run_use() {
    let rear = (0, 81, 0);
    let side = (1, 81, -1);
    let comparator = (1, 81, 0);
    let lamp = (2, 81, 0);
    let positions = [rear, side, comparator, lamp];
    let mut world = Circuit::default();
    world.set_block(
        rear,
        Some(
            Block::new("minecraft:lever")
                .with("face", "floor")
                .with("facing", "north")
                .with("powered", "false"),
        ),
    );
    world.set_block(side, Some(Block::new("minecraft:redstone_block")));
    world.set_block(
        comparator,
        Some(
            Block::new("minecraft:comparator")
                .with("facing", "west")
                .with("mode", "compare")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        lamp,
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=18 {
        let mut sound_events = Vec::new();
        if tick == 2 || tick == 11 {
            let (sound, _) = engine.use_block(&mut world, rear, tick - 1).unwrap();
            sound_events.push(use_sound_json(sound, rear));
        }
        if tick == 5 || tick == 8 {
            let (sound, _) = engine.use_block(&mut world, comparator, tick - 1).unwrap();
            sound_events.push(use_sound_json(sound, comparator));
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
                "tick": tick, "blocks": blocks,
                "comparator_output": engine.comparator_output(comparator),
                "sound_events": sound_events,
            })
        );
    }
}

fn use_sound_json(sound: UseSound, pos: Pos) -> serde_json::Value {
    let (id, pitch) = match sound {
        UseSound::Lever { powered } => (
            "minecraft:block.lever.click",
            if powered { 0.6_f32 } else { 0.5 },
        ),
        UseSound::Comparator { subtract } => (
            "minecraft:block.comparator.click",
            if subtract { 0.55_f32 } else { 0.5 },
        ),
        _ => unreachable!("comparator mode fixture only uses lever and comparator"),
    };
    serde_json::json!({
        "id": id, "category": "block", "position": [pos.0,pos.1,pos.2],
        "volume_bits": format!("{:08x}", 0.3_f32.to_bits()),
        "pitch_bits": format!("{:08x}", pitch.to_bits()),
    })
}

fn run_analog(output_wire: bool) {
    let rear = (0, 81, 0);
    let source = (1, 81, -3);
    let first_wire = (1, 81, -2);
    let side_wire = (1, 81, -1);
    let comparator = (1, 81, 0);
    let wire = (2, 81, 0);
    let lamp = (if output_wire { 3 } else { 2 }, 81, 0);
    let mut positions = vec![rear, source, first_wire, side_wire, comparator];
    if output_wire {
        positions.push(wire);
    }
    positions.push(lamp);
    let mut world = Circuit::default();
    world.set_block(
        rear,
        Some(
            Block::new("minecraft:lever")
                .with("face", "floor")
                .with("facing", "north")
                .with("powered", "false"),
        ),
    );
    world.set_block(source, Some(Block::new("minecraft:redstone_block")));
    for pos in [first_wire, side_wire] {
        world.set_block(
            pos,
            Some(
                Block::new("minecraft:redstone_wire")
                    .with("power", "0")
                    .with("north", "none")
                    .with("east", "none")
                    .with("south", "none")
                    .with("west", "none"),
            ),
        );
    }
    world.set_block(
        comparator,
        Some(
            Block::new("minecraft:comparator")
                .with("facing", "west")
                .with("mode", "subtract")
                .with("powered", "false"),
        ),
    );
    if output_wire {
        world.set_block(
            wire,
            Some(
                Block::new("minecraft:redstone_wire")
                    .with("power", "0")
                    .with("north", "none")
                    .with("east", "none")
                    .with("south", "none")
                    .with("west", "none"),
            ),
        );
    }
    world.set_block(
        lamp,
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=12 {
        if tick == 2 || tick == 7 {
            engine.use_block(&mut world, rear, tick - 1).unwrap();
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
                "tick": tick, "blocks": blocks,
                "comparator_output": engine.comparator_output(comparator),
            })
        );
    }
}
