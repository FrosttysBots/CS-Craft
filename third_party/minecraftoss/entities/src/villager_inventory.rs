//! A villager's inventory and appetite (26.3 `Villager`, `SimpleContainer`,
//! `InventoryCarrier`): eight slots it fills with what it wants
//! (`wantsToPickUp`: `#minecraft:villager_picks_up`, any villager food, and
//! its profession's requested items, while there is room), the food points
//! those hold (`DataComponents.VILLAGER_FOOD`: bread 4, carrots, potatoes
//! and beetroots 1, from `Items`' registrations) and its own food level,
//! which breeding needs twelve of (`canBreed`, `eatAndDigestFood`).
use crate::villager::Profession;
use minecraftoss_player::inventory::ItemStack;
use serde_json::Value;

/// `Villager`'s `SimpleContainer(8)`.
pub const SLOTS: usize = 8;

/// `Container.getMaxStackSize`.
const CONTAINER_MAX: u8 = 99;

/// `#minecraft:villager_plantable_seeds` (26.3 data).
pub fn plantable_seed(item: &str) -> bool {
    matches!(item, "minecraft:wheat_seeds" | "minecraft:potato" | "minecraft:carrot" | "minecraft:beetroot_seeds" | "minecraft:torchflower_seeds" | "minecraft:pitcher_pod")
}

/// `#minecraft:villager_picks_up` (26.3 data): the plantable seeds, bread,
/// wheat and beetroot.
pub fn picks_up(item: &str) -> bool {
    plantable_seed(item) || matches!(item, "minecraft:bread" | "minecraft:wheat" | "minecraft:beetroot")
}

/// `DataComponents.VILLAGER_FOOD`'s nutrition: the stack's own component
/// (or its removal), else the item's default.
pub fn villager_food(item: &str, components: Option<&Value>) -> Option<i32> {
    if let Some(food) = components.and_then(|c| c.get("minecraft:villager_food")) {
        return food.get("nutrition").and_then(Value::as_i64).map(|n| n as i32);
    }
    if components.is_some_and(|c| c.get("!minecraft:villager_food").is_some()) {
        return None;
    }
    match item {
        "minecraft:bread" => Some(4),
        "minecraft:carrot" | "minecraft:potato" | "minecraft:beetroot" => Some(1),
        _ => None,
    }
}

impl Profession {
    /// `VillagerProfession.requestedItems`: only farmers ask for any.
    pub fn requested_items(self) -> &'static [&'static str] {
        match self {
            Profession::Farmer => &["minecraft:wheat", "minecraft:wheat_seeds", "minecraft:beetroot_seeds", "minecraft:bone_meal"],
            _ => &[],
        }
    }
}

/// `ItemStack.isSameItemSameComponents`.
fn same(a: &ItemStack, item: &str, components: Option<&Value>) -> bool {
    a.id == item && a.components.as_ref() == components
}

/// A villager's eight slots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VillagerInventory {
    pub slots: [Option<ItemStack>; SLOTS],
}

impl VillagerInventory {
    /// `SimpleContainer.canAddItem`: an empty slot, or a stack of the same
    /// item and components with room.
    pub fn can_add(&self, item: &str, components: Option<&Value>) -> bool {
        self.slots.iter().any(|slot| match slot {
            None => true,
            Some(stack) => same(stack, item, components) && stack.count < stack.max,
        })
    }

    /// `SimpleContainer.addItem`: onto stacks of the same item first, then
    /// all that is left into the first empty slot (trimmed to a stack);
    /// what did not go in.
    pub fn add(&mut self, mut stack: ItemStack) -> Option<ItemStack> {
        for slot in self.slots.iter_mut().flatten() {
            if same(slot, &stack.id, stack.components.as_ref()) {
                let max = CONTAINER_MAX.min(slot.max);
                let moved = stack.count.min(max.saturating_sub(slot.count));
                slot.count += moved;
                stack.count -= moved;
                if stack.count == 0 {
                    return None;
                }
            }
        }
        if let Some(empty) = self.slots.iter_mut().find(|s| s.is_none()) {
            let max = CONTAINER_MAX.min(stack.max);
            stack.count = stack.count.min(max);
            *empty = Some(stack);
            return None;
        }
        Some(stack)
    }

    /// `countFoodPointsInInventory`.
    pub fn food_points(&self) -> i32 {
        self.slots.iter().flatten().filter_map(|s| villager_food(&s.id, s.components.as_ref()).map(|n| n * i32::from(s.count))).sum()
    }

    /// `countItem`.
    pub fn count(&self, item: &str) -> i32 {
        self.slots.iter().flatten().filter(|s| s.id == item).map(|s| i32::from(s.count)).sum()
    }

    /// `throwHalfStack`'s take: from the first slot matching with more than
    /// half a stack, half of it; or with more than 24, all past 24.
    pub fn take_half_stack(&mut self, matches: impl Fn(&ItemStack) -> bool) -> Option<ItemStack> {
        for slot in self.slots.iter_mut() {
            let Some(stack) = slot.as_mut().filter(|s| s.count > 0 && matches(s)) else { continue };
            let count = if stack.count > stack.max / 2 {
                stack.count / 2
            } else if stack.count > 24 {
                stack.count - 24
            } else {
                continue;
            };
            let mut taken = stack.clone();
            taken.count = count;
            stack.count -= count;
            if stack.count == 0 {
                *slot = None;
            }
            return Some(taken);
        }
        None
    }

    /// `removeItemType`: up to `count` of the item, from the last slot back.
    pub fn remove_type(&mut self, item: &str, count: i32) {
        let mut removed = 0;
        for slot in (0..SLOTS).rev() {
            if self.slots[slot].as_ref().is_some_and(|s| s.id == item) {
                let take = (count - removed).clamp(0, 255) as u8;
                let have = self.slots[slot].as_ref().map_or(0, |s| s.count);
                let taken = take.min(have);
                self.remove(slot, taken);
                removed += i32::from(taken);
                if removed == count {
                    break;
                }
            }
        }
    }

    /// `hasFarmSeeds`.
    pub fn has_farm_seeds(&self) -> bool {
        self.slots.iter().flatten().any(|s| plantable_seed(&s.id))
    }

    /// `removeItem(slot, count)`.
    pub fn remove(&mut self, slot: usize, count: u8) {
        if let Some(stack) = self.slots[slot].as_mut() {
            stack.count = stack.count.saturating_sub(count);
            if stack.count == 0 {
                self.slots[slot] = None;
            }
        }
    }
}

/// `Villager.wantsToPickUp`.
pub fn wants_to_pick_up(inventory: &VillagerInventory, profession: Profession, item: &str, components: Option<&Value>) -> bool {
    (picks_up(item) || villager_food(item, components).is_some() || profession.requested_items().contains(&item)) && inventory.can_add(item, components)
}

/// A villager's food level (`foodLevel`) with its inventory: `hungry`,
/// `eatUntilFull`, `digestFood` and `canBreed`'s food half.
pub fn eat_until_full(food_level: &mut i32, inventory: &mut VillagerInventory) {
    if *food_level >= 12 {
        return;
    }
    for slot in 0..SLOTS {
        let Some(stack) = inventory.slots[slot].clone() else { continue };
        let Some(nutrition) = villager_food(&stack.id, stack.components.as_ref()) else { continue };
        let mut eaten = 0u8;
        for _ in 0..stack.count {
            *food_level += nutrition;
            eaten += 1;
            if *food_level >= 12 {
                inventory.remove(slot, eaten);
                return;
            }
        }
        inventory.remove(slot, eaten);
    }
}

/// `canBreed`'s food half: twelve points between its level and its
/// inventory.
pub fn has_food_to_breed(food_level: i32, inventory: &VillagerInventory) -> bool {
    food_level + inventory.food_points() >= 12
}

/// `Villager.canBreed`: food enough, awake, and neither a baby nor
/// resting from breeding (an age of zero).
pub fn can_breed(food_level: i32, inventory: &VillagerInventory, sleeping: bool, age: i32) -> bool {
    has_food_to_breed(food_level, inventory) && !sleeping && age == 0
}

/// `eatAndDigestFood`: it eats until full, then twelve points go.
pub fn eat_and_digest(food_level: &mut i32, inventory: &mut VillagerInventory) {
    eat_until_full(food_level, inventory);
    *food_level -= 12;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(id: &str, count: u8) -> ItemStack {
        ItemStack::new(id, count)
    }

    #[test]
    fn fills_matching_stacks_then_a_free_slot() {
        let mut inventory = VillagerInventory::default();
        assert!(inventory.add(stack("minecraft:bread", 60)).is_none());
        assert!(inventory.add(stack("minecraft:bread", 10)).is_none());
        assert_eq!(inventory.slots[0].as_ref().map(|s| s.count), Some(64));
        assert_eq!(inventory.slots[1].as_ref().map(|s| s.count), Some(6));
        assert_eq!(inventory.food_points(), 70 * 4);
    }

    #[test]
    fn wants_food_seeds_and_its_profession_items_while_there_is_room() {
        let inventory = VillagerInventory::default();
        assert!(wants_to_pick_up(&inventory, Profession::None, "minecraft:bread", None));
        assert!(wants_to_pick_up(&inventory, Profession::None, "minecraft:potato", None));
        assert!(!wants_to_pick_up(&inventory, Profession::None, "minecraft:bone_meal", None));
        assert!(wants_to_pick_up(&inventory, Profession::Farmer, "minecraft:bone_meal", None));
        let full = VillagerInventory { slots: std::array::from_fn(|_| Some(stack("minecraft:stone", 64))) };
        assert!(!wants_to_pick_up(&full, Profession::None, "minecraft:bread", None));
    }

    #[test]
    fn eats_until_full() {
        let mut inventory = VillagerInventory::default();
        inventory.add(stack("minecraft:carrot", 5));
        inventory.add(stack("minecraft:bread", 3));
        let mut food = 0;
        eat_until_full(&mut food, &mut inventory);
        // Five carrots (5), then two loaves (13).
        assert_eq!(food, 13);
        assert!(inventory.slots[0].is_none());
        assert_eq!(inventory.slots[1].as_ref().map(|s| s.count), Some(1));
    }

    #[test]
    fn breeding_takes_twelve_food_points_awake_at_age_zero() {
        let mut inventory = VillagerInventory::default();
        inventory.add(stack("minecraft:bread", 3));
        assert!(can_breed(0, &inventory, false, 0));
        assert!(!can_breed(0, &inventory, true, 0), "asleep");
        assert!(!can_breed(0, &inventory, false, 6000), "resting from breeding");
        let mut food = 0;
        eat_and_digest(&mut food, &mut inventory);
        assert_eq!((food, inventory.food_points()), (0, 0));
        assert!(!can_breed(food, &inventory, false, 0));
    }
}
