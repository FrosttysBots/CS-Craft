//! Compare the authored-world stair placement state with the pinned context fixture.
use minecraftoss_player::{stair_placement_state, Block, Face, Pos, World};
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

fn main() {
    let mut world = Scene::default();
    for pos in [(8, 81, -1), (12, 81, -1)] {
        world.set_block(pos, Some(Block::new("minecraft:water").with("level", "0")));
    }
    world.set_block(
        (16, 81, -1),
        Some(Block::new("minecraft:water").with("level", "1")),
    );
    world.set_block((20, 81, -1), Some(Block::new("minecraft:lava")));
    let cases = [
        ((0, 81, 0), 0.0, Face::Up, 81.0),
        ((4, 81, 0), 90.0, Face::Down, 82.0),
        ((8, 81, -1), 180.0, Face::North, 81.25),
        ((12, 81, -1), 270.0, Face::North, 81.75),
        ((16, 81, -1), 0.0, Face::North, 81.25),
        ((20, 81, -1), 0.0, Face::North, 81.25),
    ];
    let mut states = BTreeMap::new();
    for (pos, yaw, face, click_y) in cases {
        let state = stair_placement_state(
            &world,
            pos,
            Block::new("minecraft:oak_stairs"),
            yaw,
            face,
            click_y,
        );
        states.insert(
            format!("{},{},{}", pos.0, pos.1, pos.2),
            serde_json::json!({"id":state.id,"properties":state.properties}),
        );
        world.set_block(pos, Some(state));
    }
    println!("{}", serde_json::to_string(&states).unwrap());
}
