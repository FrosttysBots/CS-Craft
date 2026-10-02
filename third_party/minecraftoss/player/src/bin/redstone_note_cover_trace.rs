//! Replay of scenarios/redstone-note-block-cover.json.
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
    let note = (1, 81, 0);
    let above = (1, 82, 0);
    let mut world = Circuit::default();
    world.set_block((1, 80, 0), Some(Block::new("minecraft:stone")));
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
    engine.changed(&mut world, &[note], 0);
    for tick in 0..=10 {
        if tick == 3 {
            world.set_block(above, Some(Block::new("minecraft:stone")));
        }
        if tick == 5 {
            world.set_block(above, None);
        }
        if tick == 7 {
            world.set_block(
                above,
                Some(
                    Block::new("minecraft:zombie_head")
                        .with("powered", "false")
                        .with("rotation", "0"),
                ),
            );
        }
        if [3, 5, 7].contains(&tick) {
            engine.changed(&mut world, &[above], tick - 1);
        }
        if [2, 4, 6, 8].contains(&tick) {
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
        let blocks: Vec<_> = [note, above]
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
                "tick": tick, "blocks": blocks, "sound_events": [], "block_events": block_events,
            })
        );
    }
}
