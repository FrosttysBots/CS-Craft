//! Controlled open-air item ejection replay of redstone-dropper-entities.json.
use minecraftoss_player::{
    dropper::Dropper, inventory::ItemStack, items::WorldItems, Block, Pos, World,
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

fn bits(value: f64) -> String {
    format!("{:016x}", value.to_bits())
}

fn main() {
    let mut world = Circuit::default();
    for x in -8..=8 {
        for z in -8..=8 {
            world.set_block((x, 80, z), Some(Block::new("minecraft:stone")));
        }
    }
    world.set_block(
        (0, 81, 0),
        Some(Block::new("minecraft:dropper").with("facing", "east")),
    );
    let mut dropper = Dropper::default();
    dropper.slots[0] = Some(ItemStack::new("minecraft:stone", 1));
    let mut items = WorldItems::with_seeds(9234567, 1, 2);
    for tick in 0..=40 {
        if tick == 5 {
            assert_eq!(
                items.dispense_dropper_item(&mut dropper, (0, 81, 0), "east"),
                Some(ItemStack::new("minecraft:stone", 1))
            );
            // The isolated server assigns this entity ID after its own
            // preexisting entities. ItemEntity uses it for resting motion.
            items.entities.last_mut().unwrap().entity_id = 11;
        }
        items.tick(&world);
        let entities: Vec<_> = items
            .entities
            .iter()
            .map(|entity| {
                serde_json::json!({
                    "id": entity.stack.id,
                    "count": entity.stack.count,
                    "x": bits(entity.position.x),
                    "y": bits(entity.position.y),
                    "z": bits(entity.position.z),
                    "vx": bits(entity.velocity.x),
                    "vy": bits(entity.velocity.y),
                    "vz": bits(entity.velocity.z),
                    "on_ground": entity.on_ground,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({"tick": tick, "item_entities": entities})
        );
    }
}
