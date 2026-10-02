//! Rust replay of scenarios/redstone-comparator-hopper-transfer.json.
use minecraftoss_player::{
    hopper::{container_signal, Hopper},
    inventory::ItemStack,
    redstone::RedstoneEngine,
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
    let remove = std::env::args().any(|arg| arg == "--remove");
    let hopper_pos = (0, 81, 0);
    let comparator_pos = (1, 81, 0);
    let wire_pos = (2, 81, 0);
    let positions = [hopper_pos, comparator_pos, wire_pos];
    let mut world = Circuit::default();
    world.set_block(
        hopper_pos,
        Some(
            Block::new("minecraft:hopper")
                .with("facing", "down")
                .with("enabled", "true"),
        ),
    );
    world.set_block(
        comparator_pos,
        Some(
            Block::new("minecraft:comparator")
                .with("facing", "west")
                .with("mode", "compare")
                .with("powered", "false"),
        ),
    );
    world.set_block(
        wire_pos,
        Some(
            Block::new("minecraft:redstone_wire")
                .with("power", "0")
                .with("north", "none")
                .with("east", "none")
                .with("south", "none")
                .with("west", "none"),
        ),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    let mut hopper = Hopper::default();
    let mut chest_slots = vec![None; 27];
    chest_slots[0] = Some(ItemStack::new("minecraft:stone", 4));
    for tick in 0..=if remove { 18_u64 } else { 22_u64 } {
        if remove && tick == 12 {
            world.set_block(hopper_pos, None);
            engine.changed(&mut world, &[hopper_pos], tick - 1);
        }
        engine.tick(&mut world, tick);
        if tick > 0 && (!remove || tick < 12) && hopper.tick_at(tick, None, Some(&mut chest_slots))
        {
            engine.container_changed(
                &mut world,
                hopper_pos,
                container_signal(&hopper.slots),
                tick,
            );
        }
        let blocks: Vec<_> = positions
            .iter()
            .map(|&pos| {
                let block = world
                    .block(pos)
                    .unwrap_or_else(|| Block::new("minecraft:air"));
                serde_json::json!({"id": block.id, "properties": block.properties})
            })
            .collect();
        let slots = |inventory: &[Option<ItemStack>]| {
            inventory
                .iter()
                .enumerate()
                .map(|(slot, stack)| {
                    serde_json::json!({
                        "slot": slot,
                        "id": stack.as_ref().map_or("minecraft:air", |s| s.id.as_str()),
                        "count": stack.as_ref().map_or(0, |s| s.count)
                    })
                })
                .collect::<Vec<_>>()
        };
        let inventories = if remove {
            serde_json::json!({"0,82,0": slots(&chest_slots)})
        } else {
            serde_json::json!({
                "0,81,0": slots(&hopper.slots),
                "0,82,0": slots(&chest_slots),
            })
        };
        println!(
            "{}",
            serde_json::json!({
                "tick": tick,
                "blocks": blocks,
                "comparator_output": engine.comparator_output(comparator_pos),
                "inventories": inventories,
            })
        );
    }
}
