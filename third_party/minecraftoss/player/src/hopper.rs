//! Headless hopper transfers. Block-entity storage is independent of chunk
//! generation; a world adapter supplies neighboring container slots.
use crate::inventory::ItemStack;

#[derive(Clone, Debug)]
pub struct Hopper {
    pub slots: [Option<ItemStack>; 5],
    pub enabled: bool,
    pub cooldown: i32,
    pub last_tick: u64,
}

impl Default for Hopper {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            enabled: true,
            cooldown: -1,
            last_tick: 0,
        }
    }
}

impl Hopper {
    pub fn item_count(&self) -> u32 {
        self.slots
            .iter()
            .flatten()
            .map(|stack| stack.count as u32)
            .sum()
    }

    /// One server block-entity tick. The hopper ejects one item before it
    /// tries to pull one item from above. A successful cycle waits eight ticks.
    pub fn tick(
        &mut self,
        destination: Option<&mut [Option<ItemStack>]>,
        source: Option<&mut [Option<ItemStack>]>,
    ) -> bool {
        self.tick_at(self.last_tick + 1, destination, source)
    }

    pub fn tick_at(
        &mut self,
        game_time: u64,
        destination: Option<&mut [Option<ItemStack>]>,
        source: Option<&mut [Option<ItemStack>]>,
    ) -> bool {
        self.tick_at_with(game_time, destination, source, |_| false)
    }

    /// `suck_entities` is used only when there is no container above. Its
    /// return value follows vanilla: a partial entity-stack transfer changes
    /// inventory but does not start the eight-tick cooldown.
    pub fn tick_at_with(
        &mut self,
        game_time: u64,
        destination: Option<&mut [Option<ItemStack>]>,
        source: Option<&mut [Option<ItemStack>]>,
        suck_entities: impl FnOnce(&mut [Option<ItemStack>]) -> bool,
    ) -> bool {
        self.cooldown -= 1;
        self.last_tick = game_time;
        if self.cooldown > 0 || !self.enabled {
            return false;
        }
        self.cooldown = 0;
        let mut changed = destination.is_some_and(|slots| move_one(&mut self.slots, slots));
        if self
            .slots
            .iter()
            .any(|slot| slot.as_ref().is_none_or(|stack| stack.count < stack.max))
        {
            changed |= match source {
                Some(slots) => move_one(slots, &mut self.slots),
                None => suck_entities(&mut self.slots),
            };
        }
        if changed {
            self.cooldown = 8;
        }
        changed
    }
}

/// `AbstractContainerMenu.getRedstoneSignalFromContainer` in pinned 26.3:
/// average slot fullness followed by `Mth.lerpDiscrete(fullness, 0, 15)`.
pub fn container_signal(slots: &[Option<ItemStack>]) -> u8 {
    if slots.is_empty() {
        return 0;
    }
    let fullness = slots
        .iter()
        .flatten()
        .map(|stack| stack.count as f32 / stack.max.max(1) as f32)
        .sum::<f32>()
        / slots.len() as f32;
    ((fullness * 14.0).floor() as u8 + u8::from(fullness > 0.0)).min(15)
}

/// Item entities offer their entire stack to the hopper. A partial fit updates
/// both inventories, while the caller observes `false` for cooldown purposes.
pub fn absorb_stack(entity: &mut ItemStack, slots: &mut [Option<ItemStack>]) -> bool {
    for target in slots {
        if entity.count == 0 {
            break;
        }
        match target {
            Some(current) if current.same_item(entity) && current.count < current.max => {
                let moved = entity.count.min(current.max - current.count);
                current.count += moved;
                entity.count -= moved;
            }
            None => {
                let moved = entity.count.min(entity.max);
                let mut inserted = entity.clone();
                inserted.count = moved;
                *target = Some(inserted);
                entity.count -= moved;
            }
            _ => {}
        }
    }
    entity.count == 0
}

/// Vanilla scans source slots then destination slots; it transfers one item
/// and merges only stacks with identical item data components.
pub fn move_one(source: &mut [Option<ItemStack>], destination: &mut [Option<ItemStack>]) -> bool {
    for from in source.iter_mut() {
        let Some(stack) = from.as_mut() else {
            continue;
        };
        if stack.count == 0 {
            continue;
        }
        let to = destination.iter_mut().find(|to| {
            to.as_ref()
                .is_none_or(|current| current.same_item(stack) && current.count < current.max)
        });
        let Some(to) = to else {
            continue;
        };
        match to {
            Some(target) => target.count += 1,
            None => {
                let mut moved = stack.clone();
                moved.count = 1;
                *to = Some(moved);
            }
        }
        stack.count -= 1;
        if stack.count == 0 {
            *from = None;
        }
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparator_signal_uses_average_slot_fullness() {
        let mut slots = [None, None, None, None, None];
        assert_eq!(container_signal(&slots), 0);
        slots[0] = Some(ItemStack::new("minecraft:stone", 1));
        assert_eq!(container_signal(&slots), 1);
        slots[0] = Some(ItemStack::new("minecraft:stone", 64));
        assert_eq!(container_signal(&slots), 3);
        slots.fill(Some(ItemStack::new("minecraft:stone", 64)));
        assert_eq!(container_signal(&slots), 15);
    }

    #[test]
    fn pinned_forty_tick_transfer_moves_five_stone() {
        let mut hopper = Hopper::default();
        hopper.slots[0] = Some(ItemStack::new("minecraft:stone", 8));
        let mut chest = vec![None; 27];
        for _ in 0..40 {
            hopper.tick(Some(&mut chest), None);
        }
        assert_eq!(chest[0].as_ref().unwrap().count, 5);
        assert_eq!(hopper.slots[0].as_ref().unwrap().count, 3);
    }

    #[test]
    fn blocked_destination_keeps_source_and_retries_without_cooldown() {
        let mut hopper = Hopper::default();
        hopper.slots[0] = Some(ItemStack::new("minecraft:stone", 8));
        let mut chest = vec![Some(ItemStack::new("minecraft:dirt", 64)); 27];
        for _ in 0..40 {
            assert!(!hopper.tick(Some(&mut chest), None));
        }
        assert_eq!(hopper.cooldown, 0);
        assert_eq!(hopper.slots[0].as_ref().unwrap().count, 8);
    }

    #[test]
    fn hopper_chain_moves_one_per_cooldown_without_losing_items() {
        let mut upper = Hopper::default();
        let mut lower = Hopper::default();
        let mut chest = vec![None; 27];
        upper.slots[0] = Some(ItemStack::new("minecraft:stone", 8));
        for time in 1..=80 {
            // The lower hopper ticks first, pulling from the upper hopper;
            // the upper can then eject another item into the lower one.
            lower.tick_at(time, Some(&mut chest), Some(&mut upper.slots));
            let was_empty = lower.item_count() == 0;
            let before = lower.item_count();
            upper.tick_at(time, Some(&mut lower.slots), None);
            if was_empty && lower.item_count() > before && lower.cooldown <= 8 {
                lower.cooldown = if lower.last_tick >= time { 7 } else { 8 };
            }
            let chest_count = chest[0].as_ref().map_or(0, |s: &ItemStack| s.count as u32);
            assert_eq!(upper.item_count() + lower.item_count() + chest_count, 8);
            if let Some(expected) = match time {
                1 => Some((0, 2, 6)),
                9 => Some((1, 3, 4)),
                24 => Some((2, 4, 2)),
                40 => Some((4, 4, 0)),
                72 | 80 => Some((8, 0, 0)),
                _ => None,
            } {
                assert_eq!(
                    (chest_count, lower.item_count(), upper.item_count()),
                    expected,
                    "tick {time}"
                );
            }
        }
        assert_eq!(upper.item_count(), 0);
        assert_eq!(lower.item_count(), 0);
        assert_eq!(chest[0].as_ref().unwrap().count, 8);
    }
}
