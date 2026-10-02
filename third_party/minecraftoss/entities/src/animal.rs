//! Shared pinned 26.3 Animal/AgeableMob item interactions for breeding species.
use crate::{age::Age, cow::InteractionResult};
use minecraftoss_player::inventory::Inventory;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnimalEvent {
    Hearts,
    AgeLock,
    AgeUnlock,
}

pub fn interact(
    age: &mut Age,
    in_love: &mut i32,
    persistence_required: &mut bool,
    inventory: &mut Inventory,
    hand: usize,
    infinite_materials: bool,
    is_food: impl Fn(&str) -> bool,
    cannot_age_lock: bool,
) -> (InteractionResult, Vec<AnimalEvent>) {
    let Some(held) = inventory.slots.get(hand).and_then(Option::as_ref) else {
        return (InteractionResult::Pass, Vec::new());
    };
    let item = held.id.clone();
    if is_food(&item) {
        if age.ticks == 0 && *in_love <= 0 {
            if !infinite_materials {
                consume_one(inventory, hand);
            }
            *in_love = 600;
            return (InteractionResult::SuccessServer, vec![AnimalEvent::Hearts]);
        }
        if age.can_grow() {
            if !infinite_materials {
                consume_one(inventory, hand);
            }
            age.feed();
            return (InteractionResult::SuccessPredicted, Vec::new());
        }
    }
    if item == "minecraft:golden_dandelion" && !cannot_age_lock && age.toggle_lock(Age::BABY_START)
    {
        if !infinite_materials {
            consume_one(inventory, hand);
        }
        if age.locked {
            *persistence_required = true;
        }
        return (
            InteractionResult::SuccessPredicted,
            vec![if age.locked {
                AnimalEvent::AgeLock
            } else {
                AnimalEvent::AgeUnlock
            }],
        );
    }
    (InteractionResult::Pass, Vec::new())
}

pub fn consume_one(inventory: &mut Inventory, hand: usize) {
    let held = inventory.slots[hand].as_mut().expect("held stack");
    held.count -= 1;
    if held.count == 0 {
        inventory.slots[hand] = None;
    }
}
