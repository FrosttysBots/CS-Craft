//! Tick/state replay of scenarios/redstone-dropper-stone.json.
use minecraftoss_player::{
    dropper::Dropper, inventory::ItemStack, items::WorldItems, redstone::RedstoneEngine, wax,
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
    let args: Vec<String> = std::env::args().collect();
    let honeycomb = args.iter().any(|arg| arg == "--honeycomb");
    let item = args.windows(2).find(|pair| pair[0] == "--item").map_or(
        if honeycomb {
            "minecraft:honeycomb"
        } else {
            "minecraft:stone"
        },
        |pair| pair[1].as_str(),
    );
    let replace = args.iter().any(|arg| arg == "--replace");
    let chest = args.iter().any(|arg| arg == "--chest");
    let chest_full = args.iter().any(|arg| arg == "--chest-full");
    let empty = args.iter().any(|arg| arg == "--empty");
    let dispenser = args.iter().any(|arg| arg == "--dispenser");
    let multislot = args.iter().any(|arg| arg == "--multislot");
    let dropper = (0, 81, 0);
    let source = (-1, 81, 0);
    let target = (1, 81, 0);
    let mut world = Circuit::default();
    let mut redstone = RedstoneEngine::default();
    world.set_block(
        dropper,
        Some(
            Block::new(if dispenser {
                "minecraft:dispenser"
            } else {
                "minecraft:dropper"
            })
            .with("facing", "east")
            .with("triggered", "false"),
        ),
    );
    let mut inventory = Dropper::default();
    if !empty {
        inventory.slots[0] = Some(ItemStack::new(item, if multislot { 2 } else { 3 }));
        if multislot {
            inventory.slots[5] = Some(ItemStack::new("minecraft:dirt", 2));
        }
    }
    let mut world_items = WorldItems::with_seeds(9234567, 1, 2);
    let mut chest_slots = if chest_full {
        vec![Some(ItemStack::new("minecraft:stone", 64)); 27]
    } else {
        vec![None; 27]
    };
    for tick in 0_u64..=if replace {
        9
    } else if multislot {
        17
    } else {
        12
    } {
        if tick == 2 || (!replace && tick == 7) || (multislot && tick == 12) {
            world.set_block(source, Some(Block::new("minecraft:redstone_block")));
            redstone.changed(&mut world, &[source], tick - 1);
        } else if (!replace && tick == if honeycomb { 6 } else { 4 }) || (multislot && tick == 9) {
            world.set_block(source, None);
            redstone.changed(&mut world, &[source], tick - 1);
        } else if replace && tick == 3 {
            world.set_block(
                dropper,
                Some(
                    Block::new("minecraft:dispenser")
                        .with("facing", "east")
                        .with("triggered", "false"),
                ),
            );
            inventory = Dropper::default();
            inventory.slots[0] = Some(ItemStack::new(item, 3));
        }
        if honeycomb && tick == 4 {
            world.set_block(target, Some(Block::new("minecraft:copper_block")));
        }
        redstone.tick(&mut world, tick);
        let scheduled = redstone.take_executed_ticks();
        let activations = redstone.take_dispense_events();
        let mut level_events = Vec::new();
        let mut sound_events = Vec::new();
        for event in &activations {
            assert_eq!(
                event.block_id,
                if dispenser {
                    "minecraft:dispenser"
                } else {
                    "minecraft:dropper"
                }
            );
            assert_eq!(event.facing, "east");
            if empty {
                assert!(inventory.take_one(|_| unreachable!()).is_none());
                level_events.push(serde_json::json!({"id":1001,"position":[0,81,0],"param":0}));
            } else if !dispenser && (chest || chest_full) {
                let expected = !chest_full;
                assert!(
                    inventory.insert_one_into(&mut chest_slots, |bound| {
                        assert_eq!(bound, 1);
                        0
                    }) == expected
                );
            } else {
                if honeycomb {
                    if let Some(waxed_id) = world
                        .block(target)
                        .and_then(|block| wax::waxed_block_id(&block.id))
                    {
                        let mut replacement = world.block(target).expect("waxable target");
                        replacement.id = waxed_id;
                        world.set_block(target, Some(replacement));
                        inventory.take_one_from(0).expect("honeycomb stack");
                        level_events
                            .push(serde_json::json!({"id":3003,"position":[1,81,0],"param":0}));
                        sound_events.push(serde_json::json!({
                            "id":"minecraft:item.honeycomb.wax_on", "category":"block",
                            "position":[1,81,0], "volume_bits":"3f800000", "pitch_bits":"3f800000"
                        }));
                    } else {
                        let stack = inventory.take_one_from(0).expect("honeycomb stack");
                        world_items
                            .dispense_one_item(stack, dropper, "east")
                            .expect("honeycomb fallback ejection");
                    }
                } else if multislot {
                    let slot = inventory
                        .random_slot(|bound| world_items.next_world_int(bound))
                        .expect("occupied dispenser");
                    let stack = inventory.take_one_from(slot).expect("selected stack");
                    world_items
                        .dispense_one_item(stack, dropper, "east")
                        .expect("ordinary item ejection");
                } else {
                    assert_eq!(
                        inventory.take_one(|bound| {
                            assert_eq!(bound, 1);
                            0
                        }),
                        Some(ItemStack::new(item, 1))
                    );
                }
                level_events.push(serde_json::json!({"id":1000,"position":[0,81,0],"param":0}));
                level_events.push(serde_json::json!({"id":2000,"position":[0,81,0],"param":5}));
            }
        }
        println!(
            "{}",
            serde_json::json!({
                "tick": tick,
                "block_id": world.block(dropper).unwrap().id,
                "triggered": world.block(dropper).unwrap().property("triggered"),
                "stone_count": inventory.slots[0].as_ref().map_or(0, |stack| stack.count),
                "dirt_count": inventory.slots[5].as_ref().map_or(0, |stack| stack.count),
                "chest_count": chest_slots[0].as_ref().map_or(0, |stack: &ItemStack| stack.count),
                "chest_total": chest_slots.iter().flatten().map(|stack| u32::from(stack.count)).sum::<u32>(),
                "scheduled": scheduled.iter().map(|(_, id)| id).collect::<Vec<_>>(),
                "activation_count": activations.len(),
                "level_events": level_events,
                "sound_events": sound_events,
                "target": world.block(target).map_or_else(|| serde_json::json!({"id":"minecraft:air","properties":{}}), |block| serde_json::json!({"id":block.id,"properties":block.properties})),
            })
        );
    }
}
