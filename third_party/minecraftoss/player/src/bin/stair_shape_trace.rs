//! Replay the measured neighbor-driven oak stair shape transitions.
use minecraftoss_player::{update_stair_shapes, Block, Pos, World};
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

fn stair(facing: &str) -> Block {
    Block::new("minecraft:oak_stairs")
        .with("facing", facing)
        .with("half", "bottom")
        .with("shape", "straight")
        .with("waterlogged", "false")
}

fn main() {
    let guard = std::env::args().any(|arg| arg == "--guard");
    let mut world = Circuit::default();
    world.set_block((0, 81, 0), Some(stair("north")));
    if guard {
        world.set_block((1, 81, 0), Some(stair("north")));
    }
    let initial = if guard {
        vec![(0, 81, 0), (1, 81, 0)]
    } else {
        vec![(0, 81, 0)]
    };
    update_stair_shapes(&mut world, &initial);
    for tick in 0..=if guard { 18 } else { 12 } {
        let changed = match (guard, tick) {
            (_, 2) => {
                world.set_block((0, 81, -1), Some(stair("west")));
                Some((0, 81, -1))
            }
            (true, 4) => {
                world.set_block((1, 81, 0), None);
                Some((1, 81, 0))
            }
            (false, 4) | (true, 6) => {
                world.set_block((0, 81, -1), None);
                Some((0, 81, -1))
            }
            (true, 8) => {
                world.set_block((-1, 81, 0), Some(stair("north")));
                Some((-1, 81, 0))
            }
            (false, 6) | (true, 10) => {
                world.set_block((0, 81, 1), Some(stair("west")));
                Some((0, 81, 1))
            }
            (true, 12) => {
                world.set_block((-1, 81, 0), None);
                Some((-1, 81, 0))
            }
            (false, 8) | (true, 14) => {
                world.set_block((0, 81, 1), None);
                Some((0, 81, 1))
            }
            _ => None,
        };
        if let Some(pos) = changed {
            update_stair_shapes(&mut world, &[pos]);
        }
        let positions: Vec<_> = if guard {
            (-1..=1)
                .flat_map(|x| (-1..=1).map(move |z| (x, 81, z)))
                .collect()
        } else {
            (-1..=1).map(|z| (0, 81, z)).collect()
        };
        let blocks: BTreeMap<_, _> = positions
            .into_iter()
            .map(|pos| {
                let state = world.block(pos).map_or_else(
                    || serde_json::json!({"id":"minecraft:air","properties":{}}),
                    |block| serde_json::json!({"id":block.id,"properties":block.properties}),
                );
                (format!("{},{},{}", pos.0, pos.1, pos.2), state)
            })
            .collect();
        println!("{}", serde_json::json!({"tick":tick,"blocks":blocks}));
    }
}
