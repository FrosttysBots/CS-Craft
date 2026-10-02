//! Replay of the measured repeater -> candidate conductor -> lamp matrix.
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
    let candidates = [
        (0, "minecraft:stone"),
        (3, "minecraft:cobblestone"),
        (6, "minecraft:oak_planks"),
        (9, "minecraft:glass"),
        (12, "minecraft:oak_slab"),
    ];
    let mut world = Circuit::default();
    let mut observed = Vec::new();
    for (z, id) in candidates {
        let source = (-1, 81, z);
        let repeater = (0, 81, z);
        let conductor = (1, 81, z);
        let lamp = (2, 81, z);
        observed.extend([source, repeater, conductor, lamp]);
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
        let candidate = if id == "minecraft:oak_slab" {
            Block::new(id)
                .with("type", "bottom")
                .with("waterlogged", "false")
        } else {
            Block::new(id)
        };
        world.set_block(conductor, Some(candidate));
        world.set_block(
            lamp,
            Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
        );
    }
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &observed, 0);
    for tick in 0..=12 {
        if tick == 2 || tick == 7 {
            for (z, _) in candidates {
                let source = (-1, 81, z);
                world.set_block(
                    source,
                    if tick == 2 {
                        Some(Block::new("minecraft:redstone_block"))
                    } else {
                        None
                    },
                );
                engine.changed(&mut world, &[source], tick - 1);
            }
        }
        engine.tick(&mut world, tick);
        let blocks: Vec<_> = observed
            .iter()
            .map(|&pos| {
                if let Some(block) = world.block(pos) {
                    serde_json::json!({"id":block.id,"properties":block.properties})
                } else {
                    serde_json::json!({"id":"minecraft:air","properties":{}})
                }
            })
            .collect();
        println!("{}", serde_json::json!({"tick":tick,"blocks":blocks}));
    }
}
