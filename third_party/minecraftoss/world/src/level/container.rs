//! Block containers and hoppers on the server level (26.3 `Container`,
//! `BaseContainerBlockEntity`, `CompoundContainer` for double chests,
//! `ShulkerBoxBlockEntity`'s faces, `HopperBlockEntity` and
//! `AbstractContainerMenu.getRedstoneSignalFromContainer`).
//!
//! Items live in the block entity's saved tag (`Items`), so every change is
//! what the chunk saves. Simulated containers: chests (single and double,
//! trapped and copper), barrels, shulker boxes, hoppers, dispensers and
//! droppers. Not yet: furnaces, brewing stands, crafters, bookshelves, pots,
//! shelves, container entities, item entities, and loot table unpacking.

use super::redstone::Kind;
use super::Level;
use minecraftoss_core::nbt::Tag;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::{BlockPos, BlockStateId};

pub use minecraftoss_core::item::ItemStack as Stack;

/// Which block entity holds the items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Chest,
    Barrel,
    ShulkerBox,
    Hopper,
    Dispenser,
}

impl Store {
    fn size(self) -> usize {
        match self {
            Self::Hopper => 5,
            Self::Dispenser => 9,
            _ => 27,
        }
    }
}

/// A container a hopper or comparator reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerRef {
    Single(BlockPos, Store),
    /// `CompoundContainer(first, second)` of a double chest.
    Double(BlockPos, BlockPos),
}

impl ContainerRef {
    fn size(self) -> usize {
        match self {
            Self::Single(_, store) => store.size(),
            Self::Double(..) => 54,
        }
    }

    /// The block entity and slot within it.
    fn locate(self, slot: usize) -> (BlockPos, Store, usize) {
        match self {
            Self::Single(pos, store) => (pos, store, slot),
            Self::Double(first, second) => {
                if slot >= 27 {
                    (second, Store::Chest, slot - 27)
                } else {
                    (first, Store::Chest, slot)
                }
            }
        }
    }

    fn positions(self) -> Vec<BlockPos> {
        match self {
            Self::Single(pos, _) => vec![pos],
            Self::Double(first, second) => vec![first, second],
        }
    }
}

impl Level<'_> {
    pub(super) fn store_of(&self, state: BlockStateId) -> Option<Store> {
        let blocks = &self.registries().blocks;
        let info = blocks.block(blocks.block_of(state));
        if info.is_a("ChestBlock") {
            Some(Store::Chest)
        } else if info.is_a("BarrelBlock") {
            Some(Store::Barrel)
        } else if info.is_a("ShulkerBoxBlock") {
            Some(Store::ShulkerBox)
        } else if info.is_a("HopperBlock") {
            Some(Store::Hopper)
        } else if info.is_a("DispenserBlock") {
            Some(Store::Dispenser)
        } else {
            None
        }
    }

    pub(super) fn block_entity(&self, pos: BlockPos) -> Option<&Tag> {
        self.chunk(pos.chunk())?.block_entities.entities.get(&(pos.x, pos.y, pos.z))
    }

    fn block_entity_mut(&mut self, pos: BlockPos) -> Option<&mut Tag> {
        self.chunks.get_mut(&pos.chunk())?.block_entities.entities.get_mut(&(pos.x, pos.y, pos.z))
    }

    /// `ChestBlock.isChestBlockedAt` (cats are not simulated).
    fn chest_blocked(&self, pos: BlockPos) -> bool {
        let above = self.block(pos.above());
        self.registries().blocks.is(above, minecraftoss_core::block::flags::REDSTONE_CONDUCTOR)
    }

    /// `HopperBlockEntity.getBlockContainer` / `ChestBlock.getContainer`.
    pub fn container_at(&self, pos: BlockPos, ignore_blocked: bool) -> Option<ContainerRef> {
        let state = self.block(pos);
        let store = self.store_of(state)?;
        self.block_entity(pos)?;
        if store != Store::Chest {
            return Some(ContainerRef::Single(pos, store));
        }
        if !ignore_blocked && self.chest_blocked(pos) {
            return None;
        }
        let blocks = &self.registries().blocks;
        let kind = blocks.property(state, "type").unwrap_or("single");
        if kind == "single" {
            return Some(ContainerRef::Single(pos, store));
        }
        let facing = blocks.property(state, "facing").and_then(Direction::from_name).unwrap_or(Direction::North);
        let connected = if kind == "left" { facing.clockwise() } else { facing.counter_clockwise() };
        let neighbour_pos = pos.relative(connected, 1);
        let neighbour = self.block(neighbour_pos);
        let same = blocks.block_of(neighbour) == blocks.block_of(state);
        let neighbour_kind = blocks.property(neighbour, "type").unwrap_or("single");
        if same && neighbour_kind != "single" && neighbour_kind != kind && blocks.property(neighbour, "facing") == blocks.property(state, "facing") {
            if !ignore_blocked && self.chest_blocked(neighbour_pos) {
                return None;
            }
            if self.block_entity(neighbour_pos).is_some() {
                // `RIGHT` is `FIRST`.
                let (first, second) = if kind == "right" { (pos, neighbour_pos) } else { (neighbour_pos, pos) };
                return Some(ContainerRef::Double(first, second));
            }
        }
        Some(ContainerRef::Single(pos, store))
    }

    fn read_items(&self, pos: BlockPos, size: usize) -> Vec<Stack> {
        let mut items = vec![Stack::empty(); size];
        if let Some(list) = self.block_entity(pos).and_then(|t| t.get("Items")).and_then(Tag::as_list) {
            for (slot, stack) in list.iter().filter_map(Stack::from_tag) {
                if slot < size {
                    items[slot] = stack;
                }
            }
        }
        items
    }

    fn write_items(&mut self, pos: BlockPos, items: &[Stack]) {
        let list: Vec<Tag> = items.iter().enumerate().filter(|(_, s)| !s.is_empty()).map(|(i, s)| s.to_tag(i)).collect();
        if let Some(Tag::Compound(map)) = self.block_entity_mut(pos) {
            map.insert("Items".to_owned(), Tag::List(list));
        }
    }

    /// `item replace block ... container.N with ...`: the block entity's own
    /// container, `setItem` through its slot access.
    pub fn replace_block_item(&mut self, pos: BlockPos, slot: usize, stack: Stack) -> bool {
        let Some(store) = self.store_of(self.block(pos)) else { return false };
        if self.block_entity(pos).is_none() || slot >= store.size() {
            return false;
        }
        self.container_set_item(ContainerRef::Single(pos, store), slot, stack);
        true
    }

    /// A block entity's own container slots (what the harness observes).
    pub fn block_container_items(&self, pos: BlockPos) -> Option<Vec<Stack>> {
        let store = self.store_of(self.block(pos))?;
        self.block_entity(pos)?;
        Some(self.read_items(pos, store.size()))
    }

    pub fn container_item(&self, c: ContainerRef, slot: usize) -> Stack {
        let (pos, store, slot) = c.locate(slot);
        self.read_items(pos, store.size()).swap_remove(slot)
    }

    /// Writes a slot with no side effects.
    pub(super) fn put_item(&mut self, c: ContainerRef, slot: usize, stack: Stack) {
        let (pos, store, slot) = c.locate(slot);
        let mut items = self.read_items(pos, store.size());
        items[slot] = stack;
        self.write_items(pos, &items);
    }

    /// `ItemStack.getMaxStackSize`, with a `max_stack_size` component.
    pub fn item_max_stack(&self, stack: &Stack) -> i32 {
        stack
            .components
            .as_ref()
            .and_then(|c| c.get("minecraft:max_stack_size"))
            .and_then(Tag::as_i64)
            .map_or_else(|| self.registries().items.max_stack(&stack.id), |m| m as i32)
    }

    /// `Container.getMaxStackSize(itemStack)`: 99 for every simulated container.
    fn container_max_stack(&self, stack: &Stack) -> i32 {
        self.item_max_stack(stack).min(99)
    }

    /// `BlockEntity.setChanged`: comparators re-read the container.
    fn block_entity_changed(&mut self, pos: BlockPos) {
        let state = self.block(pos);
        if !self.registries().blocks.is_air(state) {
            let block = self.block_id(state);
            self.update_neighbour_for_output_signal(pos, block);
        }
    }

    pub fn container_set_changed(&mut self, c: ContainerRef) {
        for pos in c.positions() {
            self.block_entity_changed(pos);
        }
    }

    /// `Container.setItem`: hoppers do not report the change themselves.
    pub fn container_set_item(&mut self, c: ContainerRef, slot: usize, mut stack: Stack) {
        let max = self.container_max_stack(&stack);
        if !stack.is_empty() && stack.count > max {
            stack.count = max;
        }
        self.put_item(c, slot, stack);
        let (pos, store, _) = c.locate(slot);
        if store != Store::Hopper {
            self.block_entity_changed(pos);
        }
    }

    /// `Container.removeItem` (`ContainerHelper.removeItem`): splits off up
    /// to `count`; the slot keeps what is left, possibly an empty stack.
    fn container_remove_item(&mut self, c: ContainerRef, slot: usize, count: i32) -> Stack {
        let mut current = self.container_item(c, slot);
        if current.is_empty() || count <= 0 {
            return Stack::empty();
        }
        let taken = count.min(current.count);
        let mut result = current.clone();
        result.count = taken;
        current.count -= taken;
        self.put_item(c, slot, current);
        let (pos, store, _) = c.locate(slot);
        if store != Store::Hopper {
            self.block_entity_changed(pos);
        }
        result
    }

    fn container_is_empty(&self, c: ContainerRef) -> bool {
        (0..c.size()).all(|slot| self.container_item(c, slot).is_empty())
    }

    /// `WorldlyContainer.getSlotsForFace`, or every slot.
    fn container_slots(&self, c: ContainerRef, _direction: Direction) -> Vec<usize> {
        (0..c.size()).collect()
    }

    /// `canPlaceItem` and, for worldly containers, `canPlaceItemThroughFace`.
    fn container_can_place(&self, c: ContainerRef, _slot: usize, stack: &Stack, direction: Option<Direction>) -> bool {
        match c {
            ContainerRef::Single(_, Store::ShulkerBox) if direction.is_some() => !self.is_shulker_box_item(&stack.id),
            _ => true,
        }
    }

    fn is_shulker_box_item(&self, id: &str) -> bool {
        let blocks = &self.registries().blocks;
        blocks.block_by_name(id).is_some_and(|b| blocks.block(b).is_a("ShulkerBoxBlock"))
    }

    /// `AbstractContainerMenu.getRedstoneSignalFromContainer`.
    pub(super) fn container_signal(&self, c: Option<ContainerRef>) -> i32 {
        let Some(c) = c else { return 0 };
        let mut total = 0.0f32;
        for slot in 0..c.size() {
            let stack = self.container_item(c, slot);
            if !stack.is_empty() {
                total += stack.count as f32 / self.container_max_stack(&stack) as f32;
            }
        }
        total /= c.size() as f32;
        // `Mth.lerpDiscrete(total, 0, 15)`.
        (total * 14.0).floor() as i32 + i32::from(total > 0.0)
    }

    // ---- hoppers -----------------------------------------------------------------

    fn hopper_cooldown(&self, pos: BlockPos) -> i32 {
        self.block_entity(pos).and_then(|t| t.get("TransferCooldown")).and_then(Tag::as_i64).unwrap_or(-1) as i32
    }

    fn set_hopper_cooldown(&mut self, pos: BlockPos, value: i32) {
        if let Some(Tag::Compound(map)) = self.block_entity_mut(pos) {
            map.insert("TransferCooldown".to_owned(), Tag::Int(value));
        }
    }

    /// `HopperBlockEntity.pushItemsTick`.
    pub(super) fn hopper_tick(&mut self, pos: BlockPos) {
        let cooldown = self.hopper_cooldown(pos) - 1;
        self.set_hopper_cooldown(pos, cooldown);
        self.hopper_ticked.insert(pos, self.game_time);
        if cooldown <= 0 {
            self.set_hopper_cooldown(pos, 0);
            self.hopper_try_move(pos);
        }
    }

    /// `HopperBlockEntity.tryMoveItems` with `suckInItems`.
    fn hopper_try_move(&mut self, pos: BlockPos) -> bool {
        let state = self.block(pos);
        if self.hopper_cooldown(pos) > 0 || self.registries().blocks.property(state, "enabled") != Some("true") {
            return false;
        }
        let me = ContainerRef::Single(pos, Store::Hopper);
        let mut changed = false;
        if !self.container_is_empty(me) {
            changed = self.hopper_eject(pos, state);
        }
        if !self.hopper_full(me) {
            changed |= self.hopper_suck(pos);
        }
        if changed {
            self.set_hopper_cooldown(pos, 8);
            self.block_entity_changed(pos);
            return true;
        }
        false
    }

    fn hopper_full(&self, me: ContainerRef) -> bool {
        (0..me.size()).all(|slot| {
            let stack = self.container_item(me, slot);
            !stack.is_empty() && stack.count == self.item_max_stack(&stack)
        })
    }

    /// `HopperBlockEntity.ejectItems`.
    fn hopper_eject(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        let facing = self.registries().blocks.property(state, "facing").and_then(Direction::from_name).unwrap_or(Direction::Down);
        let Some(target) = self.container_at(pos.relative(facing, 1), true) else { return false };
        let direction = facing.opposite();
        if self.container_slots(target, direction).into_iter().all(|slot| {
            let stack = self.container_item(target, slot);
            stack.count >= self.item_max_stack(&stack)
        }) {
            return false;
        }
        let me = ContainerRef::Single(pos, Store::Hopper);
        for slot in 0..me.size() {
            let stack = self.container_item(me, slot);
            if stack.is_empty() {
                continue;
            }
            let taken = self.container_remove_item(me, slot, 1);
            let result = self.add_item(Some(me), target, taken, Some(direction));
            if result.is_empty() {
                self.container_set_changed(target);
                return true;
            }
            // The split stack in the slot goes back to its count.
            self.put_item(me, slot, stack.clone());
            if stack.count == 1 {
                self.container_set_item(me, slot, stack);
            }
        }
        false
    }

    /// `HopperBlockEntity.suckInItems` from a container above.
    fn hopper_suck(&mut self, pos: BlockPos) -> bool {
        let Some(source) = self.container_at(pos.above(), true) else { return false };
        let me = ContainerRef::Single(pos, Store::Hopper);
        for slot in self.container_slots(source, Direction::Down) {
            let stack = self.container_item(source, slot);
            if stack.is_empty() {
                continue;
            }
            let taken = self.container_remove_item(source, slot, 1);
            let result = self.add_item(Some(source), me, taken, None);
            if result.is_empty() {
                self.container_set_changed(source);
                return true;
            }
            // The split stack in the slot goes back to its count.
            self.put_item(source, slot, stack.clone());
            if stack.count == 1 {
                self.container_set_item(source, slot, stack);
            }
        }
        false
    }

    /// `HopperBlockEntity.addItem(from, container, stack, direction)`.
    pub(super) fn add_item(&mut self, from: Option<ContainerRef>, into: ContainerRef, mut stack: Stack, direction: Option<Direction>) -> Stack {
        let slots = match direction {
            Some(d) => self.container_slots(into, d),
            None => (0..into.size()).collect(),
        };
        for slot in slots {
            if stack.is_empty() {
                break;
            }
            stack = self.try_move_in_item(from, into, stack, slot, direction);
        }
        stack
    }

    /// `HopperBlockEntity.tryMoveInItem`.
    fn try_move_in_item(&mut self, from: Option<ContainerRef>, into: ContainerRef, mut stack: Stack, slot: usize, direction: Option<Direction>) -> Stack {
        if !self.container_can_place(into, slot, &stack, direction) {
            return stack;
        }
        let current = self.container_item(into, slot);
        let was_empty = self.container_is_empty(into);
        let mut success = false;
        if current.is_empty() {
            self.container_set_item(into, slot, stack);
            stack = Stack::empty();
            success = true;
        } else if current.count <= self.item_max_stack(&current) && current.same_item_same_components(&stack) {
            let space = self.item_max_stack(&stack) - current.count;
            let count = stack.count.min(space);
            stack.count -= count;
            let mut grown = current;
            grown.count += count;
            self.put_item(into, slot, grown);
            success = count > 0;
        }
        if success {
            if let ContainerRef::Single(pos, Store::Hopper) = into {
                if was_empty && self.hopper_cooldown(pos) <= 8 {
                    let mut skip = 0;
                    if let Some(ContainerRef::Single(from_pos, Store::Hopper)) = from {
                        let mine = self.hopper_ticked.get(&pos).copied().unwrap_or(0);
                        let theirs = self.hopper_ticked.get(&from_pos).copied().unwrap_or(0);
                        if mine >= theirs {
                            skip = 1;
                        }
                    }
                    self.set_hopper_cooldown(pos, 8 - skip);
                }
            }
            self.container_set_changed(into);
        }
        stack
    }

    /// `HopperBlock.checkPoweredState`.
    pub(super) fn hopper_check_powered(&mut self, pos: BlockPos, state: BlockStateId) {
        let should_be_on = !self.has_neighbor_signal(pos);
        let on = self.registries().blocks.property(state, "enabled") == Some("true");
        if should_be_on != on {
            let next = self.with(state, "enabled", if should_be_on { "true" } else { "false" });
            self.set_block(pos, next, super::update::CLIENTS, super::update::LIMIT);
        }
    }

    pub(super) fn is_hopper(&self, state: BlockStateId) -> bool {
        self.store_of(state) == Some(Store::Hopper)
    }

    pub(super) fn has_ticker(&self, state: BlockStateId) -> bool {
        self.is_hopper(state)
            || self.redstone_kind(state) == Some(Kind::MovingPiston)
            || self.redstone_kind(state) == Some(Kind::DaylightDetector) && self.sky.as_ref().is_some_and(|s| s.has_sky_light)
    }
}
