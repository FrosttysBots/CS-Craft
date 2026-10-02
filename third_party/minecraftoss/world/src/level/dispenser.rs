//! Dispensers and droppers on the server level (26.3 `DispenserBlock`,
//! `DropperBlock`, `DispenserBlockEntity` and the dispense behaviours for
//! plain items, filled and empty buckets, and honeycomb).
//!
//! Items thrown into the world draw their launch from the level random as
//! vanilla does and become item entities. Behaviours not ported yet are recorded in
//! `Level::unsupported` and dispense nothing.

use super::container::{ContainerRef, Stack, Store};
use super::{update, Level};
use minecraftoss_core::block::flags;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::BlockPos;

/// Items with a registered `DispenseItemBehavior` besides the ones ported here.
const OTHER_BEHAVIOURS: &[&str] = &[
    "acacia_boat", "acacia_chest_boat", "armor_stand", "arrow", "axolotl_bucket", "bamboo_chest_raft", "bamboo_raft", "birch_boat",
    "birch_chest_boat", "blue_egg", "brown_egg", "brush", "carved_pumpkin", "cherry_boat", "cherry_chest_boat", "chest",
    "chest_minecart", "cod_bucket", "command_block_minecart", "dark_oak_boat", "dark_oak_chest_boat", "egg", "experience_bottle",
    "firework_rocket", "fire_charge", "flint_and_steel", "furnace_minecart", "glass_bottle", "glowstone", "hopper_minecart", "jungle_boat",
    "jungle_chest_boat", "lingering_potion", "mangrove_boat", "mangrove_chest_boat", "minecart", "oak_boat", "oak_chest_boat",
    "pale_oak_boat", "pale_oak_chest_boat", "poplar_boat", "poplar_chest_boat", "potion", "powder_snow_bucket", "pufferfish_bucket",
    "salmon_bucket", "shears", "snowball", "spectral_arrow", "splash_potion", "spruce_boat", "spruce_chest_boat", "sulfur_cube_bucket",
    "tadpole_bucket", "tipped_arrow", "tnt", "tnt_minecart", "tropical_fish_bucket", "wind_charge", "wither_skeleton_skull",
];

impl Level<'_> {
    fn dispenser_facing(&self, pos: BlockPos) -> Direction {
        self.registries().blocks.property(self.block(pos), "facing").and_then(Direction::from_name).unwrap_or(Direction::North)
    }

    /// `DispenserBlock.neighborChanged`.
    pub(super) fn dispenser_neighbor_changed(&mut self, state: minecraftoss_core::BlockStateId, pos: BlockPos) {
        let should = self.has_neighbor_signal(pos) || self.has_neighbor_signal(pos.above());
        let triggered = self.registries().blocks.property(state, "triggered") == Some("true");
        if should && !triggered {
            let block = self.block_id(state);
            self.schedule_block_tick_priority(pos, block, 4, super::redstone::priority::NORMAL);
            self.set_block(pos, self.with(state, "triggered", "true"), update::CLIENTS, update::LIMIT);
        } else if !should && triggered {
            self.set_block(pos, self.with(state, "triggered", "false"), update::CLIENTS, update::LIMIT);
        }
    }

    /// `DispenserBlockEntity.getRandomSlot`.
    fn random_slot(&mut self, me: ContainerRef) -> Option<usize> {
        let mut slot = None;
        let mut odds = 1;
        for i in 0..9 {
            if !self.container_item(me, i).is_empty() {
                if self.random.next_i32_bound(odds) == 0 {
                    slot = Some(i);
                }
                odds += 1;
            }
        }
        slot
    }

    /// `DispenserBlock.dispenseFrom` / `DropperBlock.dispenseFrom`.
    pub(super) fn dispense_from(&mut self, pos: BlockPos, dropper: bool) {
        if self.block_container_items(pos).is_none() {
            return;
        }
        let me = ContainerRef::Single(pos, Store::Dispenser);
        let Some(slot) = self.random_slot(me) else { return };
        let stack = self.container_item(me, slot);
        if dropper {
            if stack.is_empty() {
                return;
            }
            let direction = self.dispenser_facing(pos);
            let remaining = match self.container_at(pos.relative(direction, 1), true) {
                None => self.default_dispense(pos, stack),
                Some(into) => {
                    let mut one = stack.clone();
                    one.count = 1;
                    let left = self.add_item(Some(me), into, one, Some(direction.opposite()));
                    let mut remaining = stack.clone();
                    if left.is_empty() {
                        remaining.count -= 1;
                    }
                    remaining
                }
            };
            self.container_set_item(me, slot, remaining);
            return;
        }
        self.dispensing_slot = Some(slot);
        let name = stack.id.trim_start_matches("minecraft:").to_owned();
        let result = match name.as_str() {
            "water_bucket" | "lava_bucket" => self.dispense_filled_bucket(pos, stack),
            "bucket" => self.dispense_empty_bucket(pos, stack),
            "honeycomb" => self.dispense_honeycomb(pos, stack),
            "bone_meal" => {
                // `OptionalDispenseItemBehavior` for bone meal: nothing is
                // thrown when it fails.
                let target = pos.relative(self.dispenser_facing(pos), 1);
                let mut stack = stack;
                if self.grow_crop(target) || self.grow_water_plant(target) {
                    stack.count -= 1;
                }
                stack
            }
            _ if OTHER_BEHAVIOURS.contains(&name.as_str()) || name.ends_with("shulker_box") || name.ends_with("_spawn_egg") => {
                self.unsupported.push(format!("dispensing {}", stack.id));
                return;
            }
            _ => self.default_dispense(pos, stack),
        };
        self.container_set_item(me, slot, result);
    }

    /// `DefaultDispenseItemBehavior.execute`: one item is thrown.
    fn default_dispense(&mut self, pos: BlockPos, mut stack: Stack) -> Stack {
        let direction = self.dispenser_facing(pos);
        let mut one = stack.clone();
        one.count = 1.min(stack.count);
        stack.count -= one.count;
        self.spawn_item(pos, direction, one);
        stack
    }

    /// `DefaultDispenseItemBehavior.spawnItem` with accuracy 6.
    fn spawn_item(&mut self, pos: BlockPos, direction: Direction, stack: Stack) {
        let (dx, dy, dz) = direction.offset();
        let (x, mut y, z) = (pos.x as f64 + 0.5 + 0.7 * dx as f64, pos.y as f64 + 0.5 + 0.7 * dy as f64, pos.z as f64 + 0.5 + 0.7 * dz as f64);
        y -= if direction.axis() == minecraftoss_core::pos::Axis::Y { 0.125 } else { 0.15625 };
        let power = self.random.next_f64() * 0.1 + 0.2;
        let spread = 0.0172275 * 6.0;
        let triangle = |level: &mut Self, mean: f64| mean + spread * (level.random.next_f64() - level.random.next_f64());
        let motion = [triangle(self, dx as f64 * power), triangle(self, 0.2), triangle(self, dz as f64 * power)];
        self.spawn_item_entity([x, y, z], stack, motion);
    }

    /// `DefaultDispenseItemBehavior.consumeWithRemainder`.
    fn consume_with_remainder(&mut self, pos: BlockPos, mut dispensed: Stack, remainder: Stack) -> Stack {
        dispensed.count -= 1;
        if dispensed.is_empty() {
            return remainder;
        }
        // `addToInventoryOrDispense`: the dispensed stack shrank in its slot.
        let slot = self.dispensing_slot.expect("dispensing");
        self.put_item(ContainerRef::Single(pos, Store::Dispenser), slot, dispensed.clone());
        let left = self.dispenser_insert(pos, remainder);
        if !left.is_empty() {
            let direction = self.dispenser_facing(pos);
            self.spawn_item(pos, direction, left);
        }
        dispensed
    }

    /// `DispenserBlockEntity.insertItem`.
    fn dispenser_insert(&mut self, pos: BlockPos, mut stack: Stack) -> Stack {
        let me = ContainerRef::Single(pos, Store::Dispenser);
        let max = self.item_max_stack(&stack).min(99);
        for i in 0..9 {
            let target = self.container_item(me, i);
            let target = &target;
            if target.is_empty() || target.id == stack.id && target.components == stack.components {
                let transfer = stack.count.min(max - if target.is_empty() { 0 } else { target.count });
                if transfer > 0 {
                    if target.is_empty() {
                        let mut part = stack.clone();
                        part.count = transfer;
                        stack.count -= transfer;
                        self.container_set_item(me, i, part);
                    } else {
                        stack.count -= transfer;
                        let mut grown = target.clone();
                        grown.count += transfer;
                        self.put_item(me, i, grown);
                    }
                }
                if stack.is_empty() {
                    break;
                }
            }
        }
        stack
    }

    /// The filled-bucket behaviour: `BucketItem.emptyContents` in front.
    fn dispense_filled_bucket(&mut self, pos: BlockPos, stack: Stack) -> Stack {
        let direction = self.dispenser_facing(pos);
        let target = pos.relative(direction, 1);
        let water = stack.id == "minecraft:water_bucket";
        if self.empty_bucket_into(target, water) {
            self.consume_with_remainder(pos, stack, Stack::new("minecraft:bucket", 1))
        } else {
            self.default_dispense(pos, stack)
        }
    }

    /// `BucketItem.emptyContents(null, level, pos, null)` for water or lava.
    fn empty_bucket_into(&mut self, pos: BlockPos, water: bool) -> bool {
        let state = self.block(pos);
        let blocks = &self.registries().blocks;
        let name = self.name(state);
        let fixed = name == "minecraft:end_gateway" || name == "minecraft:end_portal";
        let may_replace = !fixed && (blocks.is(state, flags::REPLACEABLE) || !blocks.is(state, flags::LEGACY_SOLID));
        let waterloggable = blocks.property(state, "waterlogged").is_some();
        let place_liquid = may_replace || waterloggable && water;
        if !(blocks.is_air(state) || place_liquid) {
            return false;
        }
        if waterloggable && water {
            // `SimpleWaterloggedBlock.placeLiquid`.
            if blocks.property(state, "waterlogged") == Some("false") {
                self.set_block_and_update(pos, self.with(state, "waterlogged", "true"));
                self.schedule_water_tick(pos);
            }
            return true;
        }
        let liquid = blocks.is(state, flags::LIQUID);
        if may_replace && !liquid {
            self.destroy_block_drops(pos, true, update::LIMIT);
        }
        let source = self.registries().blocks.parse_state(if water { "minecraft:water" } else { "minecraft:lava" }).expect("vanilla fluid");
        let was_source = self.fluid_state(state).is_some_and(|f| f.source);
        self.set_block(pos, source, 11, update::LIMIT) || was_source
    }

    /// The empty-bucket behaviour: `BucketPickup.pickupBlock` in front.
    fn dispense_empty_bucket(&mut self, pos: BlockPos, stack: Stack) -> Stack {
        let direction = self.dispenser_facing(pos);
        let target = pos.relative(direction, 1);
        match self.bucket_pickup(target) {
            Some(bucket) => self.consume_with_remainder(pos, stack, Stack::new(bucket, 1)),
            None => self.default_dispense(pos, stack),
        }
    }

    /// `BucketPickup.pickupBlock` without a player: the bucket it fills.
    pub(super) fn bucket_pickup(&mut self, target: BlockPos) -> Option<&'static str> {
        let state = self.block(target);
        let blocks = &self.registries().blocks;
        let info = blocks.block(blocks.block_of(state));
        let picked = if info.is_a("LiquidBlock") {
            if blocks.property(state, "level") == Some("0") {
                let bucket = if self.name(state) == "minecraft:water" { "minecraft:water_bucket" } else { "minecraft:lava_bucket" };
                self.set_block(target, minecraftoss_core::BlockStateId::AIR, 11, update::LIMIT);
                Some(bucket)
            } else {
                None
            }
        } else if info.is_a("BubbleColumnBlock") {
            self.set_block(target, minecraftoss_core::BlockStateId::AIR, 11, update::LIMIT);
            Some("minecraft:water_bucket")
        } else if info.is_a("PowderSnowBlock") {
            self.set_block(target, minecraftoss_core::BlockStateId::AIR, 11, update::LIMIT);
            Some("minecraft:powder_snow_bucket")
        } else if blocks.property(state, "waterlogged").is_some() {
            if blocks.property(state, "waterlogged") == Some("true") {
                self.set_block_and_update(target, self.with(state, "waterlogged", "false"));
                if !self.can_survive_state(state, target) {
                    self.destroy_block_drops(target, true, update::LIMIT);
                }
                Some("minecraft:water_bucket")
            } else {
                None
            }
        } else {
            None
        };
        picked
    }

    /// The honeycomb behaviour: `HoneycombItem.getWaxed` in front.
    fn dispense_honeycomb(&mut self, pos: BlockPos, mut stack: Stack) -> Stack {
        let direction = self.dispenser_facing(pos);
        let target = pos.relative(direction, 1);
        let state = self.block(target);
        let name = self.name(state).to_owned();
        let waxed = name
            .strip_prefix("minecraft:")
            .filter(|n| !n.starts_with("waxed_") && n.contains("copper"))
            .and_then(|n| self.registries().blocks.block_by_name(&format!("minecraft:waxed_{n}")));
        match waxed {
            Some(block) => {
                // `withPropertiesOf`.
                let blocks = &self.registries().blocks;
                let mut next = blocks.block(block).default_state();
                for property in blocks.block(blocks.block_of(state)).properties() {
                    if let Some(value) = blocks.property(state, &property.name) {
                        if let Some(s) = blocks.with_property(next, &property.name, value) {
                            next = s;
                        }
                    }
                }
                self.set_block_and_update(target, next);
                stack.count -= 1;
                stack
            }
            None => self.default_dispense(pos, stack),
        }
    }
}
