//! Rust replay of scenarios/redstone-copper-bulb.json.
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

fn sound_json(sound: UseSound, pos: Pos) -> serde_json::Value {
    let (event, volume, pitch) = match sound {
        UseSound::CopperBulb { lit } => (
            if lit {
                "block.copper_bulb.turn_on"
            } else {
                "block.copper_bulb.turn_off"
            },
            1.0_f32,
            1.0_f32,
        ),
        UseSound::Lever { powered } => (
            "block.lever.click",
            0.3_f32,
            if powered { 0.6_f32 } else { 0.5_f32 },
        ),
        _ => unreachable!("copper bulb fixture only uses bulb and lever sounds"),
    };
    serde_json::json!({
        "id": format!("minecraft:{event}"),
        "category": "block",
        "position": [pos.0, pos.1, pos.2],
        "volume_bits": format!("{:08x}", volume.to_bits()),
        "pitch_bits": format!("{:08x}", pitch.to_bits())
    })
}

fn main() {
    let placement = std::env::args().any(|arg| arg == "--placement");
    let positions: Vec<Pos> = (0..=if placement { 3 } else { 4 })
        .map(|x| (x, 81, 0))
        .collect();
    let [lever, bulb, comparator, wire, lamp] =
        [(0, 81, 0), (1, 81, 0), (2, 81, 0), (3, 81, 0), (4, 81, 0)];
    let mut world = Circuit::default();
    world.set_block(
        lever,
        Some(
            Block::new("minecraft:lever")
                .with("face", "floor")
                .with("facing", "north")
                .with("powered", if placement { "true" } else { "false" }),
        ),
    );
    if !placement {
        world.set_block(
            bulb,
            Some(
                Block::new("minecraft:copper_bulb")
                    .with("lit", "false")
                    .with("powered", "false"),
            ),
        );
    }
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
        Some(Block::new("minecraft:redstone_wire").with("power", "0")),
    );
    if !placement {
        world.set_block(
            lamp,
            Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
        );
    }
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    engine.take_sounds();
    for tick in 0..=if placement { 12 } else { 18 } {
        let mut sound_events = Vec::new();
        if placement && tick == 2 {
            world.set_block(
                bulb,
                Some(
                    Block::new("minecraft:copper_bulb")
                        .with("lit", "false")
                        .with("powered", "false"),
                ),
            );
            engine.changed(&mut world, &[bulb], tick - 1);
            sound_events.extend(
                engine
                    .take_sounds()
                    .into_iter()
                    .map(|(pos, sound)| sound_json(sound, pos)),
            );
        }
        if (if placement {
            &[5, 8][..]
        } else {
            &[2, 5, 8, 11][..]
        })
        .contains(&tick)
        {
            let (lever_sound, _) = engine.use_block(&mut world, lever, tick - 1).unwrap();
            sound_events.extend(
                engine
                    .take_sounds()
                    .into_iter()
                    .map(|(pos, sound)| sound_json(sound, pos)),
            );
            sound_events.push(sound_json(lever_sound, lever));
        }
        engine.tick(&mut world, tick);
        sound_events.extend(
            engine
                .take_sounds()
                .into_iter()
                .map(|(pos, sound)| sound_json(sound, pos)),
        );
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
                "tick": tick,
                "blocks": blocks,
                "comparator_output": engine.comparator_output(comparator),
                "sound_events": sound_events
            })
        );
    }
}
