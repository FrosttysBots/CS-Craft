//! Replay of scenarios/redstone-piston-basic.json.
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
    let args: Vec<_> = std::env::args().collect();
    let short = args.iter().any(|arg| arg == "--short");
    let two = args.iter().any(|arg| arg == "--two");
    let sticky = args.iter().any(|arg| arg == "--sticky");
    let count = args
        .windows(2)
        .find(|pair| pair[0] == "--count")
        .map(|pair| pair[1].parse::<usize>().expect("valid --count"))
        .unwrap_or(if two { 2 } else { 1 });
    let lever = (0, 81, 0);
    let piston = (1, 81, 0);
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
        piston,
        Some(
            Block::new(if sticky {
                "minecraft:sticky_piston"
            } else {
                "minecraft:piston"
            })
            .with("extended", "false")
            .with("facing", "east"),
        ),
    );
    for x in 2..2 + count as i32 {
        world.set_block((x, 81, 0), Some(Block::new("minecraft:stone")));
    }
    let mut engine = RedstoneEngine::default();
    let mut initial = vec![lever, piston];
    initial.extend((2..2 + count as i32).map(|x| (x, 81, 0)));
    engine.changed(&mut world, &initial, 0);
    for tick in 0..=if count >= 12 {
        6
    } else if short {
        8
    } else {
        12
    } {
        if tick == 2 || count < 12 && tick == if short { 3 } else { 7 } {
            engine.use_block(&mut world, lever, tick - 1).unwrap();
        }
        engine.tick(&mut world, tick);
        let block_events: Vec<_> = engine
            .take_piston_events()
            .into_iter()
            .map(|event| {
                serde_json::json!({"block": if sticky { "minecraft:sticky_piston" } else { "minecraft:piston" },
                "position": [event.pos.0,event.pos.1,event.pos.2],
                "id": event.id, "param": event.param})
            })
            .collect();
        let blocks: Vec<_> = (0..=if count >= 12 {
            15
        } else if two {
            5
        } else {
            4
        })
            .map(|x| {
                let block = world
                    .block((x, 81, 0))
                    .unwrap_or_else(|| Block::new("minecraft:air"));
                serde_json::json!({"id": block.id, "properties": block.properties})
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "tick": tick, "blocks": blocks, "block_events": block_events,
            })
        );
    }
}
