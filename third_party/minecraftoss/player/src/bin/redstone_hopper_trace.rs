//! Rust replay of scenarios/redstone-hopper-lock.json.
use minecraftoss_player::{
    hopper::Hopper, inventory::ItemStack, redstone::RedstoneEngine, Block, Pos, World,
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
    let transfer = std::env::args().any(|arg| arg == "--transfer");
    let lever = (0, 81, 0);
    let hopper = (1, 81, 0);
    let positions = [lever, hopper];
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
        hopper,
        Some(
            Block::new("minecraft:hopper")
                .with("facing", "down")
                .with("enabled", "true"),
        ),
    );
    let mut engine = RedstoneEngine::default();
    engine.changed(&mut world, &positions, 0);
    let mut hopper_inventory = Hopper::default();
    let mut chest_slots: Vec<Option<ItemStack>> = vec![None; 27];
    if transfer {
        hopper_inventory.slots[0] = Some(ItemStack::new("minecraft:stone", 8));
    }
    for tick in 0..=if transfer { 44u64 } else { 10u64 } {
        if tick == 2 || tick == if transfer { 20 } else { 7 } {
            engine.use_block(&mut world, lever, tick - 1).unwrap();
        }
        engine.tick(&mut world, tick);
        if transfer && tick > 0 {
            hopper_inventory.enabled =
                world.block(hopper).unwrap().property("enabled") == Some("true");
            hopper_inventory.tick_at(tick, Some(&mut chest_slots), None);
        }
        let blocks: Vec<_> = positions
            .iter()
            .map(|&pos| {
                let block = world.block(pos).unwrap();
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
        let inventories = if transfer {
            Some(serde_json::json!({
                "1,80,0": slots(&chest_slots),
                "1,81,0": slots(&hopper_inventory.slots),
            }))
        } else {
            None
        };
        println!(
            "{}",
            serde_json::json!({"tick": tick, "blocks": blocks, "inventories": inventories})
        );
    }
}
