//! Deterministic server sound-event replay for authored gate/door circuits.
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

fn event(pos: Pos, sound: UseSound) -> Option<serde_json::Value> {
    let (id, volume, pitch) = match sound {
        UseSound::Lever { powered } => (
            "block.lever.click",
            0.3f32,
            Some(if powered { 0.6f32 } else { 0.5f32 }),
        ),
        UseSound::OakFenceGate { open } => (
            if open {
                "block.fence_gate.open"
            } else {
                "block.fence_gate.close"
            },
            1.0,
            None,
        ),
        UseSound::OakDoor { open } => (
            if open {
                "block.wooden_door.open"
            } else {
                "block.wooden_door.close"
            },
            1.0,
            None,
        ),
        UseSound::IronDoor { open } => (
            if open {
                "block.iron_door.open"
            } else {
                "block.iron_door.close"
            },
            1.0,
            None,
        ),
        _ => return None,
    };
    Some(serde_json::json!({
        "id": format!("minecraft:{id}"), "category": "block",
        "position": [pos.0, pos.1, pos.2],
        "volume_bits": format!("{:08x}", volume.to_bits()),
        "pitch_bits": pitch.map_or("<source-random-0.9f..1.0f>".into(),
            |value| format!("{:08x}", value.to_bits())),
    }))
}

fn main() {
    let kind = std::env::args().nth(1).unwrap_or_default();
    assert!(matches!(kind.as_str(), "gate" | "oak-door" | "iron-door"));
    let lever = (0, 81, 0);
    let target = (1, 81, 0);
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
    if kind == "gate" {
        world.set_block(
            target,
            Some(
                Block::new("minecraft:oak_fence_gate")
                    .with("facing", "north")
                    .with("in_wall", "false")
                    .with("open", "false")
                    .with("powered", "false"),
            ),
        );
    } else {
        let id = if kind == "oak-door" {
            "minecraft:oak_door"
        } else {
            "minecraft:iron_door"
        };
        world.set_block((1, 80, 0), Some(Block::new("minecraft:stone")));
        for (y, half) in [(81, "lower"), (82, "upper")] {
            world.set_block(
                (1, y, 0),
                Some(
                    Block::new(id)
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
    engine.changed(&mut world, &[lever, target], 0);
    for tick in 0..=9 {
        let mut sounds = Vec::new();
        if tick == 2 || tick == 7 {
            let (click, _) = engine.use_block(&mut world, lever, tick - 1).unwrap();
            sounds.extend(engine.take_sounds());
            sounds.push((lever, click));
        }
        engine.tick(&mut world, tick);
        sounds.extend(engine.take_sounds());
        let events: Vec<_> = sounds
            .into_iter()
            .filter_map(|(pos, sound)| event(pos, sound))
            .collect();
        println!(
            "{}",
            serde_json::json!({"tick": tick, "sound_events": events})
        );
    }
}
