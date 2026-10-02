//! Reconstruct the measured BlockPlaceContext slab target and state cases.
use minecraftoss_player::{slab_placement_state, slab_placement_target, Block, Face, Pos, World};
use std::collections::BTreeMap;

#[derive(Default)]
struct Scene(BTreeMap<Pos, Block>);
impl World for Scene {
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

fn slab(kind: &str, waterlogged: &str) -> Block {
    Block::new("minecraft:oak_slab")
        .with("type", kind)
        .with("waterlogged", waterlogged)
}

fn key(pos: Pos) -> String {
    format!("{},{},{}", pos.0, pos.1, pos.2)
}

fn main() {
    let mut world = Scene::default();
    for x in [0, 8, 16] {
        world.set_block((x, 81, 0), Some(slab("bottom", "false")));
    }
    for x in [4, 12] {
        world.set_block((x, 81, 0), Some(slab("top", "false")));
    }
    world.set_block((32, 81, 0), Some(slab("bottom", "true")));
    for x in [20, 24, 28] {
        world.set_block((x, 81, 0), Some(Block::new("minecraft:stone")));
    }
    world.set_block(
        (20, 81, -1),
        Some(Block::new("minecraft:water").with("level", "0")),
    );
    world.set_block(
        (24, 81, -1),
        Some(Block::new("minecraft:water").with("level", "1")),
    );
    world.set_block((28, 81, -1), Some(Block::new("minecraft:lava")));
    let cases = [
        ((0, 81, 0), Face::Up, 81.5),
        ((4, 81, 0), Face::Down, 81.5),
        ((8, 81, 0), Face::North, 81.75),
        ((12, 81, 0), Face::North, 81.25),
        ((16, 81, 0), Face::North, 81.25),
        ((20, 81, 0), Face::North, 81.25),
        ((24, 81, 0), Face::North, 81.25),
        ((28, 81, 0), Face::North, 81.25),
        ((32, 81, 0), Face::Up, 81.5),
    ];
    let mut placed = Vec::new();
    for (clicked_pos, face, click_y) in cases {
        let pos = slab_placement_target(&world, clicked_pos, face, click_y, "minecraft:oak_slab");
        let block = slab_placement_state(&world, pos, Block::new("minecraft:oak_slab"), face, click_y);
        world.set_block(pos, Some(block));
        placed.push(key(pos));
    }
    let positions = [
        (0, 81, 0),
        (4, 81, 0),
        (8, 81, 0),
        (12, 81, 0),
        (16, 81, 0),
        (16, 81, -1),
        (20, 81, -1),
        (24, 81, -1),
        (28, 81, -1),
        (32, 81, 0),
    ];
    let states: BTreeMap<_, _> = positions
        .into_iter()
        .map(|pos| {
            let block = world.block(pos).unwrap();
            (key(pos), serde_json::json!({"id":block.id,"properties":block.properties}))
        })
        .collect();
    println!("{}", serde_json::json!({"placed":placed,"states":states}));
}
