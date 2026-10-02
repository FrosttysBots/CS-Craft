//! Rust replay of scenarios/redstone-repeater-basic.json.
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
    if std::env::args().any(|arg| arg == "--pending-delay") {
        run_pending_delay();
        return;
    }
    if std::env::args().any(|arg| arg == "--use") {
        run_use();
        return;
    }
    let short = std::env::args().any(|arg| arg == "--short");
    let delay_four = std::env::args().any(|arg| arg == "--delay-four");
    let locking = std::env::args().any(|arg| arg == "--lock");
    let x = if short {
        10
    } else if delay_four {
        20
    } else if locking {
        30
    } else {
        0
    };
    let lever = (x, 81, 0);
    let repeater = (x + 1, 81, 0);
    let lamp = (x + 2, 81, 0);
    let side_lever = (x + 1, 81, -2);
    let side_repeater = (x + 1, 81, -1);
    let positions = if locking {
        vec![lever, repeater, lamp, side_lever, side_repeater]
    } else {
        vec![lever, repeater, lamp]
    };
    let mut world = Circuit::default();
    world.set_block(
        lever,
        Some(
            Block::new("minecraft:lever")
                .with("face", "floor")
                .with("facing", "north")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        repeater,
        Some(
            Block::new("minecraft:repeater")
                .with("facing", "west")
                .with("delay", if delay_four { "4" } else { "1" })
                .with("locked", "false")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        lamp,
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    if locking {
        world.set_block(
            side_lever,
            Some(
                Block::new("minecraft:lever")
                    .with("face", "floor")
                    .with("facing", "north")
                    .with("powered", "false"),
            ),
        );
        world.set_block(
            side_repeater,
            Some(
                Block::new("minecraft:repeater")
                    .with("facing", "north")
                    .with("delay", "1")
                    .with("locked", "false")
                    .with("powered", "false"),
            ),
        );
    }
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    let off_tick = if short {
        3
    } else if delay_four {
        14
    } else {
        7
    };
    let last_tick = if short {
        13
    } else if delay_four {
        28
    } else if locking {
        20
    } else {
        16
    };
    for tick in 0..=last_tick {
        if tick == 2 || tick == off_tick {
            let mut block = world.block(lever).unwrap();
            block
                .properties
                .insert("powered".into(), (tick == 2).to_string());
            world.set_block(lever, Some(block));
            engine.changed(&mut world, &[lever], tick - 1);
        }
        if locking && (tick == 4 || tick == 12) {
            let mut block = world.block(side_lever).unwrap();
            block
                .properties
                .insert("powered".into(), (tick == 4).to_string());
            world.set_block(side_lever, Some(block));
            engine.changed(&mut world, &[side_lever], tick - 1);
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

fn run_pending_delay() {
    let lever = (0, 81, 0);
    let repeater = (1, 81, 0);
    let lamp = (2, 81, 0);
    let positions = [lever, repeater, lamp];
    let mut world = Circuit::default();
    world.set_block(
        lever,
        Some(
            Block::new("minecraft:lever")
                .with("face", "floor")
                .with("facing", "north")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        repeater,
        Some(
            Block::new("minecraft:repeater")
                .with("facing", "west")
                .with("delay", "4")
                .with("locked", "false")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        lamp,
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=17 {
        let mut sounds = Vec::new();
        if tick == 2 || tick == 10 {
            let (sound, _) = engine.use_block(&mut world, lever, tick - 1).unwrap();
            let UseSound::Lever { powered } = sound else {
                unreachable!()
            };
            sounds.push(serde_json::json!({
                "id": "minecraft:block.lever.click", "category": "block",
                "position": [lever.0,lever.1,lever.2],
                "volume_bits": format!("{:08x}", 0.3_f32.to_bits()),
                "pitch_bits": format!("{:08x}", (if powered {0.6_f32} else {0.5}).to_bits()),
            }));
        }
        if tick == 3 {
            let (sound, _) = engine.use_block(&mut world, repeater, tick - 1).unwrap();
            assert!(matches!(sound, UseSound::None));
        }
        engine.tick(&mut world, tick);
        let events: Vec<_> = engine
            .take_executed_ticks()
            .into_iter()
            .map(
                |(pos, block)| serde_json::json!({"block": block, "position": [pos.0,pos.1,pos.2]}),
            )
            .collect();
        let blocks: Vec<_> = positions
            .iter()
            .map(|&pos| {
                let block = world.block(pos).unwrap();
                serde_json::json!({"id": block.id, "properties": block.properties})
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "tick": tick, "blocks": blocks, "sound_events": sounds,
                "scheduled_tick_events": events,
            })
        );
    }
}

fn run_use() {
    let lever = (0, 81, 0);
    let repeater = (1, 81, 0);
    let lamp = (2, 81, 0);
    let positions = [lever, repeater, lamp];
    let mut world = Circuit::default();
    world.set_block(
        lever,
        Some(
            Block::new("minecraft:lever")
                .with("face", "floor")
                .with("facing", "north")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        repeater,
        Some(
            Block::new("minecraft:repeater")
                .with("facing", "west")
                .with("delay", "1")
                .with("locked", "false")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        lamp,
        Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=26 {
        let mut sound_events = Vec::new();
        if tick == 2 || tick == 16 || tick == 20 {
            let (sound, _) = engine.use_block(&mut world, lever, tick - 1).unwrap();
            let UseSound::Lever { powered } = sound else {
                unreachable!()
            };
            sound_events.push(serde_json::json!({
                "id": "minecraft:block.lever.click", "category": "block",
                "position": [lever.0, lever.1, lever.2],
                "volume_bits": format!("{:08x}", 0.3_f32.to_bits()),
                "pitch_bits": format!("{:08x}", (if powered { 0.6_f32 } else { 0.5 }).to_bits()),
            }));
        }
        if [4, 7, 10, 13].contains(&tick) {
            let (sound, _) = engine.use_block(&mut world, repeater, tick - 1).unwrap();
            assert!(matches!(sound, UseSound::None));
        }
        engine.tick(&mut world, tick);
        let blocks: Vec<_> = positions
            .iter()
            .map(|&pos| {
                let block = world.block(pos).unwrap();
                serde_json::json!({"id": block.id, "properties": block.properties})
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "tick": tick, "blocks": blocks, "sound_events": sound_events,
            })
        );
    }
}
