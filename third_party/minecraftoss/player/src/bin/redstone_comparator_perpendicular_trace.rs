//! Replay axial and perpendicular comparator-to-wire shapes from the 26.3 suite.
use minecraftoss_player::{redstone::RedstoneEngine, Block, Pos, World};
use serde_json::json;
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
    let repeater = std::env::args().any(|arg| arg == "--repeater");
    let powered = std::env::args().any(|arg| arg == "--powered");
    let cases = if powered {
        vec![
            ("repeater_powered_side_wire", 0, "north", true),
            ("comparator_powered_side_wire", 16, "north", false),
        ]
    } else if repeater {
        vec![("repeater_perpendicular_branch_wire", 0, "north", true)]
    } else {
        vec![
            ("comparator_axial_branch_wire", 0, "west", false),
            ("comparator_perpendicular_branch_wire", 16, "north", false),
        ]
    };
    for (scenario, x, facing, is_repeater) in cases {
        let mut world = Circuit::default();
        for px in x - 2..=x + 4 {
            for pz in -2..=3 {
                world.set_block((px, 80, pz), Some(Block::new("minecraft:stone")));
            }
        }
        let source = if is_repeater {
            Block::new("minecraft:repeater")
                .with("facing", facing)
                .with("delay", "1")
                .with("locked", "false")
                .with("powered", "false")
        } else {
            Block::new("minecraft:comparator")
                .with("facing", facing)
                .with("mode", "compare")
                .with("powered", "false")
        };
        world.set_block((x, 81, 0), Some(source));
        for pos in [(x + 1, 81, 0), (x + 2, 81, 0), (x + 1, 81, 1)] {
            world.set_block(
                pos,
                Some(Block::new("minecraft:redstone_wire").with("power", "0")),
            );
        }
        let mut engine = RedstoneEngine::default();
        let initial: Vec<_> = world.0.keys().copied().collect();
        engine.changed(&mut world, &initial, 0);
        for tick in 0..=if powered { 10 } else { 2 } {
            if powered && (tick == 2 || tick == 6) {
                for z in [-1, 1] {
                    let pos = (x, 81, z);
                    world.set_block(
                        pos,
                        (tick == 2).then(|| Block::new("minecraft:redstone_block")),
                    );
                    engine.changed(&mut world, &[pos], tick - 1);
                }
            }
            engine.tick(&mut world, tick);
            let blocks: BTreeMap<_, _> = (x..=x + 2)
                .flat_map(|px| ((if powered { -1 } else { 0 })..=1).map(move |pz| (px, 81, pz)))
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
            let mut record = json!({
                "scenario": scenario,
                "tick": tick,
                "blocks": blocks,
            });
            if !is_repeater {
                record["comparator_outputs"] = json!({
                    format!("{},81,0", x): engine.comparator_output((x, 81, 0))
                });
            }
            println!("{record}");
        }
    }
}
