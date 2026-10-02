//! Replay of scenarios/redstone-note-block.json, including block-event calls.
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
    let lever = (0, 81, 0);
    let note = (1, 81, 0);
    let positions = [lever, note];
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
        note,
        Some(
            Block::new("minecraft:note_block")
                .with("instrument", "basedrum")
                .with("note", "0")
                .with("powered", "false"),
        ),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    for tick in 0..=18 {
        let mut sounds = Vec::new();
        if [4, 7, 9, 13, 15].contains(&tick) {
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
        if tick == 2 || tick == 11 {
            engine.use_block(&mut world, note, tick - 1).unwrap();
        }
        engine.tick(&mut world, tick);
        let block_events: Vec<_> = engine
            .take_note_events()
            .into_iter()
            .map(|event| {
                serde_json::json!({"block": "minecraft:note_block",
                "position": [event.pos.0,event.pos.1,event.pos.2], "id": 0, "param": 0})
            })
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
                "block_events": block_events,
            })
        );
    }
}
