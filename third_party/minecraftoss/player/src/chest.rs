//! Chest item storage and menu transactions. Each placed block owns 27 slots;
//! a double-chest menu combines two halves in vanilla's right-then-left order.
use crate::inventory::{click_stack, Inventory, ItemStack};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Chest {
    pub slots: Vec<Option<ItemStack>>,
}

impl Default for Chest {
    fn default() -> Self {
        Self {
            slots: vec![None; 27],
        }
    }
}

impl Chest {
    pub fn combined(right: &Self, left: &Self) -> Self {
        let mut slots = right.slots.clone();
        slots.extend(left.slots.iter().cloned());
        Self { slots }
    }

    pub fn split_into(self, right: &mut Self, left: &mut Self) {
        right.slots.clone_from_slice(&self.slots[..27]);
        left.slots.clone_from_slice(&self.slots[27..54]);
    }

    pub fn click_slot(
        &mut self,
        index: usize,
        right: bool,
        shift: bool,
        inventory: &mut Inventory,
    ) {
        if index >= self.slots.len() {
            return;
        }
        if shift {
            let Some(mut stack) = self.slots[index].take() else {
                return;
            };
            let moved_item = stack.clone();
            let original_count = stack.count;
            // ChestMenu.quickMoveStack reverses the player-inventory range.
            let indices = (0..36).rev();
            move_stack(&mut stack, &mut inventory.slots, indices);
            if stack.count < original_count {
                inventory.notice_item_changed(&moved_item);
            }
            if stack.count > 0 {
                self.slots[index] = Some(stack);
            }
        } else {
            click_stack(&mut self.slots[index], &mut inventory.cursor, right);
        }
    }

    pub fn quick_move_from_inventory(&mut self, index: usize, inventory: &mut Inventory) {
        if index >= 36 {
            return;
        }
        let Some(mut stack) = inventory.slots[index].take() else {
            return;
        };
        let chest_slots = self.slots.len();
        move_stack(&mut stack, &mut self.slots, 0..chest_slots);
        if stack.count > 0 {
            inventory.slots[index] = Some(stack);
        }
    }

    pub fn number_swap(&mut self, index: usize, hotbar: usize, inventory: &mut Inventory) {
        if index < self.slots.len() && hotbar < 9 {
            let before = inventory.slots[hotbar].clone();
            std::mem::swap(&mut self.slots[index], &mut inventory.slots[hotbar]);
            inventory.notice_slot_after_change(hotbar, before);
        }
    }

    /// Drag distribution can cross chest and player slots. Player indices are
    /// 0..43; chest indices begin at 43 and end at 70 or 97.
    pub fn distribute(&mut self, indices: &[usize], right: bool, inventory: &mut Inventory) {
        let Some(mut carried) = inventory.cursor.take() else {
            return;
        };
        let eligible = indices
            .iter()
            .copied()
            .filter(|&index| {
                let slot = if index < inventory.slots.len() {
                    inventory.slots.get(index)
                } else {
                    self.slots.get(index - inventory.slots.len())
                };
                slot.is_some_and(|slot| {
                    slot.as_ref()
                        .is_none_or(|stack| stack.same_item(&carried) && stack.count < stack.max)
                })
            })
            .collect::<Vec<_>>();
        if eligible.is_empty() {
            inventory.cursor = Some(carried);
            return;
        }
        let each = if right {
            1
        } else {
            carried.count / eligible.len() as u8
        };
        for index in eligible {
            if carried.count == 0 {
                break;
            }
            let before = inventory.slots.get(index).cloned().flatten();
            let slot = if index < inventory.slots.len() {
                &mut inventory.slots[index]
            } else {
                &mut self.slots[index - inventory.slots.len()]
            };
            let target = slot.get_or_insert_with(|| ItemStack {
                count: 0,
                ..carried.clone()
            });
            let moved = each.min(target.max - target.count).min(carried.count);
            target.count += moved;
            carried.count -= moved;
            if moved > 0 && index < inventory.slots.len() {
                inventory.notice_slot_after_change(index, before);
            }
        }
        if carried.count > 0 {
            inventory.cursor = Some(carried);
        }
    }
    pub fn pickup_all(&mut self, right: bool, inventory: &mut Inventory) {
        let Some(mut carried) = inventory.cursor.take() else {
            return;
        };
        let total = self.slots.len() + inventory.slots.len();
        for pass in 0..2 {
            let indices: Box<dyn Iterator<Item = usize>> = if right {
                Box::new((0..total).rev())
            } else {
                Box::new(0..total)
            };
            for index in indices {
                if carried.count >= carried.max {
                    break;
                }
                let slot = if index < self.slots.len() {
                    &mut self.slots[index]
                } else {
                    &mut inventory.slots[index - self.slots.len()]
                };
                let Some(stack) = slot.as_ref() else { continue };
                if !stack.same_item(&carried) || (pass == 0 && stack.count == stack.max) {
                    continue;
                }
                let before = slot.clone();
                let stack = slot.as_mut().unwrap();
                let moved = stack.count.min(carried.max - carried.count);
                stack.count -= moved;
                carried.count += moved;
                if stack.count == 0 {
                    *slot = None;
                }
                if index >= self.slots.len() {
                    inventory.notice_slot_after_change(index - self.slots.len(), before);
                }
            }
        }
        inventory.cursor = Some(carried);
    }

    pub fn take_contents(&mut self) -> Vec<ItemStack> {
        self.slots.iter_mut().filter_map(Option::take).collect()
    }
}

fn move_stack(
    source: &mut ItemStack,
    slots: &mut [Option<ItemStack>],
    indices: impl Clone + Iterator<Item = usize>,
) {
    for index in indices.clone() {
        if let Some(target) = slots[index].as_mut() {
            if target.same_item(source) && target.count < target.max {
                let moved = (target.max - target.count).min(source.count);
                target.count += moved;
                source.count -= moved;
                if source.count == 0 {
                    return;
                }
            }
        }
    }
    for index in indices {
        if slots[index].is_none() {
            let moved = source.count.min(source.max);
            slots[index] = Some(ItemStack {
                count: moved,
                ..source.clone()
            });
            source.count -= moved;
            if source.count == 0 {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total(chest: &Chest, inventory: &Inventory) -> u32 {
        chest
            .slots
            .iter()
            .chain(&inventory.slots)
            .chain(std::iter::once(&inventory.cursor))
            .filter_map(Option::as_ref)
            .map(|stack| stack.count as u32)
            .sum()
    }

    #[test]
    fn shift_transfer_merges_then_fills_and_preserves_items() {
        let mut chest = Chest::default();
        let mut inventory = Inventory::default();
        chest.slots[0] = Some(ItemStack::new("minecraft:stone", 60));
        inventory.slots[9] = Some(ItemStack::new("minecraft:stone", 10));
        chest.quick_move_from_inventory(9, &mut inventory);
        assert_eq!(total(&chest, &inventory), 70);
        assert_eq!(chest.slots[0].as_ref().unwrap().count, 64);
        assert_eq!(chest.slots[1].as_ref().unwrap().count, 6);
        chest.click_slot(0, false, true, &mut inventory);
        assert_eq!(total(&chest, &inventory), 70);
        assert!(chest.slots[0].is_none());
        assert_eq!(inventory.slots[35].as_ref().unwrap().count, 64);
    }

    #[test]
    fn full_chest_keeps_source_and_break_returns_contents() {
        let mut chest = Chest::default();
        for slot in &mut chest.slots {
            *slot = Some(ItemStack::new("minecraft:dirt", 64));
        }
        let mut inventory = Inventory::default();
        inventory.slots[0] = Some(ItemStack::new("minecraft:stone", 7));
        chest.quick_move_from_inventory(0, &mut inventory);
        assert_eq!(inventory.slots[0].as_ref().unwrap().count, 7);
        let contents = chest.take_contents();
        assert_eq!(contents.len(), 27);
        assert!(chest.slots.iter().all(Option::is_none));
    }

    #[test]
    fn dragging_across_chest_and_player_slots_conserves_stack() {
        let mut chest = Chest::default();
        let mut inventory = Inventory::default();
        inventory.cursor = Some(ItemStack::new("minecraft:stone", 9));
        chest.distribute(&[43, 44, 0], false, &mut inventory);
        assert_eq!(chest.slots[0].as_ref().unwrap().count, 3);
        assert_eq!(chest.slots[1].as_ref().unwrap().count, 3);
        assert_eq!(inventory.slots[0].as_ref().unwrap().count, 3);
        assert!(inventory.cursor.is_none());
    }

    #[test]
    fn double_menu_stores_in_right_then_left_and_splits_without_loss() {
        let mut right = Chest::default();
        let mut left = Chest::default();
        right.slots[26] = Some(ItemStack::new("minecraft:oak_log", 2));
        left.slots[0] = Some(ItemStack::new("minecraft:stone", 3));
        let mut menu = Chest::combined(&right, &left);
        assert_eq!(menu.slots.len(), 54);
        assert_eq!(menu.slots[27].as_ref().unwrap().id, "minecraft:stone");
        let mut inventory = Inventory::default();
        inventory.slots[9] = Some(ItemStack::new("minecraft:stone", 4));
        menu.quick_move_from_inventory(9, &mut inventory);
        assert_eq!(menu.slots[27].as_ref().unwrap().count, 7);
        menu.split_into(&mut right, &mut left);
        assert_eq!(right.slots[26].as_ref().unwrap().count, 2);
        assert_eq!(left.slots[0].as_ref().unwrap().count, 7);
        assert!(inventory.slots[9].is_none());
    }
}
