//! Nine-slot dropper inventory and the pinned dispenser-family slot lottery.
use crate::{hopper::absorb_stack, inventory::ItemStack};
use std::{collections::HashSet, sync::OnceLock};

#[derive(Clone, Debug)]
pub struct Dropper {
    pub slots: [Option<ItemStack>; 9],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BucketPickupOutcome {
    Stored,
    EjectRemainder(ItemStack),
}

/// Default stacks with a registered, equippable, or spawn-egg dispense route.
/// The measured 26.3 catalog is repeatable across two isolated server runs.
/// Sulfur-cube-swallowable items are absent here because that behavior calls
/// ordinary ejection when there is no sulfur cube in the target cell; mobs are
/// intentionally outside the authored-world build.
pub fn needs_special_dispenser_behavior(id: &str) -> bool {
    if !id.starts_with("minecraft:") {
        return true;
    }
    static NONDEFAULT: OnceLock<HashSet<&'static str>> = OnceLock::new();
    NONDEFAULT
        .get_or_init(|| {
            include_str!("../data/dispenser_nondefault_26_3.txt")
                .lines()
                .filter(|line| line.starts_with("minecraft:"))
                .collect()
        })
        .contains(id)
}

impl Default for Dropper {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
        }
    }
}

impl Dropper {
    /// `DispenserBlockEntity.getRandomSlot`: reservoir sample occupied slots
    /// in slot order. `next_int` must consume the world's random stream.
    pub fn random_slot(&self, mut next_int: impl FnMut(u32) -> u32) -> Option<usize> {
        let mut chosen = None;
        let mut seen = 0;
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.as_ref().is_some_and(|stack| stack.count > 0) {
                seen += 1;
                if next_int(seen) == 0 {
                    chosen = Some(index);
                }
            }
        }
        chosen
    }

    /// Ordinary-item path of `DropperBlock.dispenseFrom` when the facing cell
    /// has no container. Returns the one-item stack for the item entity.
    pub fn take_one(&mut self, next_int: impl FnMut(u32) -> u32) -> Option<ItemStack> {
        let slot = self.random_slot(next_int)?;
        self.take_one_from(slot)
    }

    /// Consume the item selected by `getRandomSlot` after its behavior has
    /// been chosen by the dispenser's item registry.
    pub fn take_one_from(&mut self, slot: usize) -> Option<ItemStack> {
        let stack = self.slots[slot].as_mut()?;
        let mut output = stack.clone();
        output.count = 1;
        stack.count -= 1;
        if stack.count == 0 {
            self.slots[slot] = None;
        }
        Some(output)
    }

    /// Successful `DispensibleContainerItem.emptyContents` for a single
    /// water/lava bucket returns an ordinary empty bucket to this slot.
    pub fn replace_single_filled_bucket(&mut self, slot: usize) -> bool {
        let Some(stack) = self.slots.get(slot).and_then(Option::as_ref) else {
            return false;
        };
        if stack.count != 1
            || !matches!(
                stack.id.as_str(),
                "minecraft:water_bucket" | "minecraft:lava_bucket"
            )
        {
            return false;
        }
        self.slots[slot] = Some(ItemStack::new("minecraft:bucket", 1));
        true
    }

    /// `consumeWithRemainder` after an empty bucket collects a source fluid.
    pub fn fill_empty_bucket(&mut self, slot: usize, fluid: &str) -> Option<BucketPickupOutcome> {
        let Some(stack) = self.slots.get(slot).and_then(Option::as_ref) else {
            return None;
        };
        if stack.id != "minecraft:bucket" || stack.count == 0 {
            return None;
        }
        let result = match fluid {
            "water" => "minecraft:water_bucket",
            "lava" => "minecraft:lava_bucket",
            _ => return None,
        };
        let mut filled = ItemStack::new(result, 1);
        filled.max = 1;
        if stack.count == 1 {
            self.slots[slot] = Some(filled);
            return Some(BucketPickupOutcome::Stored);
        }
        self.slots[slot].as_mut()?.count -= 1;
        if let Some(empty) = self.slots.iter_mut().find(|stack| stack.is_none()) {
            *empty = Some(filled);
            Some(BucketPickupOutcome::Stored)
        } else {
            Some(BucketPickupOutcome::EjectRemainder(filled))
        }
    }

    /// `DropperBlock.dispenseFrom` facing-container path. An unsuccessful
    /// insertion leaves the chosen source slot intact. The caller supplies
    /// only slots permitted by the destination's insertion face.
    pub fn insert_one_into(
        &mut self,
        destination: &mut [Option<ItemStack>],
        next_int: impl FnMut(u32) -> u32,
    ) -> bool {
        let Some(slot) = self.random_slot(next_int) else {
            return false;
        };
        let Some(stack) = self.slots[slot].as_mut() else {
            return false;
        };
        let mut offered = stack.clone();
        offered.count = 1;
        if !absorb_stack(&mut offered, destination) {
            return false;
        }
        stack.count -= 1;
        if stack.count == 0 {
            self.slots[slot] = None;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_default_stack_dispatch_keeps_special_items_out_of_ejection() {
        for ordinary in [
            "minecraft:stone",
            "minecraft:dirt",
            "minecraft:oak_planks",
            "minecraft:cobblestone",
            "minecraft:diamond_pickaxe",
        ] {
            assert!(!needs_special_dispenser_behavior(ordinary), "{ordinary}");
        }
        for special in [
            "minecraft:arrow",
            "minecraft:water_bucket",
            "minecraft:chest",
            "minecraft:diamond_chestplate",
            "minecraft:zombie_spawn_egg",
        ] {
            assert!(needs_special_dispenser_behavior(special), "{special}");
        }
    }

    #[test]
    fn one_slot_dispenses_one_each_time() {
        let mut dropper = Dropper::default();
        dropper.slots[0] = Some(ItemStack::new("minecraft:stone", 3));
        for remaining in [2, 1, 0] {
            assert_eq!(
                dropper.take_one(|bound| {
                    assert_eq!(bound, 1);
                    0
                }),
                Some(ItemStack::new("minecraft:stone", 1))
            );
            assert_eq!(
                dropper.slots[0].as_ref().map_or(0, |stack| stack.count),
                remaining
            );
        }
    }

    #[test]
    fn reservoir_samples_all_occupied_slots() {
        let mut dropper = Dropper::default();
        dropper.slots[0] = Some(ItemStack::new("minecraft:stone", 1));
        dropper.slots[5] = Some(ItemStack::new("minecraft:dirt", 1));
        assert_eq!(dropper.random_slot(|_| 0), Some(5));
        assert_eq!(dropper.random_slot(|bound| bound - 1), Some(0));
    }

    #[test]
    fn full_destination_preserves_selected_stack() {
        let mut dropper = Dropper::default();
        dropper.slots[0] = Some(ItemStack::new("minecraft:stone", 2));
        let mut destination = vec![Some(ItemStack::new("minecraft:dirt", 64)); 27];
        assert!(!dropper.insert_one_into(&mut destination, |_| 0));
        assert_eq!(dropper.slots[0].as_ref().unwrap().count, 2);
        destination[0] = None;
        assert!(dropper.insert_one_into(&mut destination, |_| 0));
        assert_eq!(dropper.slots[0].as_ref().unwrap().count, 1);
        assert_eq!(destination[0], Some(ItemStack::new("minecraft:stone", 1)));
    }

    #[test]
    fn single_filled_bucket_becomes_empty_bucket_after_success() {
        let mut dispenser = Dropper::default();
        dispenser.slots[0] = Some(ItemStack::new("minecraft:water_bucket", 1));
        dispenser.slots[1] = Some(ItemStack::new("minecraft:lava_bucket", 2));
        assert!(dispenser.replace_single_filled_bucket(0));
        assert_eq!(
            dispenser.slots[0],
            Some(ItemStack::new("minecraft:bucket", 1))
        );
        assert!(!dispenser.replace_single_filled_bucket(1));
        assert_eq!(
            dispenser.slots[1],
            Some(ItemStack::new("minecraft:lava_bucket", 2))
        );
    }

    #[test]
    fn single_empty_bucket_becomes_filled_after_source_pickup() {
        let mut dispenser = Dropper::default();
        dispenser.slots[0] = Some(ItemStack::new("minecraft:bucket", 1));
        assert_eq!(
            dispenser.fill_empty_bucket(0, "water"),
            Some(BucketPickupOutcome::Stored)
        );
        let filled = dispenser.slots[0].as_ref().unwrap();
        assert_eq!(filled.id, "minecraft:water_bucket");
        assert_eq!(filled.count, 1);
        assert_eq!(filled.max, 1);
    }

    #[test]
    fn stacked_empty_buckets_store_filled_result_in_first_free_slot() {
        let mut dispenser = Dropper::default();
        dispenser.slots[0] = Some(ItemStack::new("minecraft:bucket", 2));
        assert_eq!(
            dispenser.fill_empty_bucket(0, "water"),
            Some(BucketPickupOutcome::Stored)
        );
        assert_eq!(
            dispenser.slots[0],
            Some(ItemStack::new("minecraft:bucket", 1))
        );
        assert_eq!(
            dispenser.slots[1].as_ref().unwrap().id,
            "minecraft:water_bucket"
        );
        assert_eq!(dispenser.slots[1].as_ref().unwrap().max, 1);
    }

    #[test]
    fn full_dispenser_ejects_filled_bucket_remainder() {
        let mut dispenser = Dropper::default();
        dispenser.slots[0] = Some(ItemStack::new("minecraft:bucket", 2));
        for slot in &mut dispenser.slots[1..] {
            *slot = Some(ItemStack::new("minecraft:stone", 64));
        }
        let outcome = dispenser.fill_empty_bucket(0, "water");
        let Some(BucketPickupOutcome::EjectRemainder(stack)) = outcome else {
            panic!("filled bucket should be ejected when every slot is occupied");
        };
        assert_eq!(stack.id, "minecraft:water_bucket");
        assert_eq!(stack.max, 1);
        assert_eq!(dispenser.slots[0].as_ref().unwrap().count, 1);
    }
}
