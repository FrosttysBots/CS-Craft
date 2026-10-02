//! World item lifetime and inventory transfer. Spatial movement and rendering
//! are separate concerns; item transactions stay testable without a window.
use crate::{
    clip_motion,
    dropper::Dropper,
    hopper::absorb_stack,
    inventory::{Inventory, ItemStack},
    local_shapes,
    rng::LegacyRandom,
    shapes_near, Box3, World,
};
use glam::DVec3;

#[derive(Clone, Debug)]
pub struct ItemEntity {
    pub entity_id: u32,
    pub stack: ItemStack,
    pub position: DVec3,
    pub previous_position: DVec3,
    pub velocity: DVec3,
    pub age: u32,
    pub bob_offset: f32,
    pub pickup_delay: u16,
    pub on_ground: bool,
}

pub struct WorldItems {
    pub entities: Vec<ItemEntity>,
    pub pickup_effects: Vec<PickupEffect>,
    pickup_sounds: u32,
    pickup_transfers: Vec<ItemStack>,
    next_entity_id: u32,
    world_random: LegacyRandom,
    entity_seeds: LegacyRandom,
    player_random: LegacyRandom,
}

#[derive(Clone, Debug)]
pub struct PickupEffect {
    pub item: ItemEntity,
    pub age: u8,
    pub target: DVec3,
}

impl PickupEffect {
    pub fn position(&self, partial_tick: f32) -> DVec3 {
        // ItemPickupParticleGroup: three ticks, with squared interpolation.
        let time = ((self.age as f64 + partial_tick.clamp(0.0, 1.0) as f64) / 3.0).powi(2);
        self.item.position.lerp(self.target, time)
    }
}

impl Default for WorldItems {
    fn default() -> Self {
        Self::with_seeds(0, 1, 2)
    }
}

impl WorldItems {
    /// Share the level random stream with dispenser-family block entities.
    pub fn next_world_int(&mut self, bound: u32) -> u32 {
        self.world_random.next_int(bound)
    }

    /// `DropperBlock.dispenseFrom` ordinary-item path followed by
    /// `DefaultDispenseItemBehavior.spawnItem`. The caller must handle facing
    /// containers before using this open-air ejection path.
    pub fn dispense_dropper_item(
        &mut self,
        dropper: &mut Dropper,
        pos: (i32, i32, i32),
        facing: &str,
    ) -> Option<ItemStack> {
        let stack = dropper.take_one(|bound| self.world_random.next_int(bound))?;
        self.dispense_one_item(stack.clone(), pos, facing)?;
        Some(stack)
    }

    /// `DefaultDispenseItemBehavior.spawnItem` after the caller has selected
    /// the dispenser's slot and resolved its item behavior.
    pub fn dispense_one_item(
        &mut self,
        stack: ItemStack,
        pos: (i32, i32, i32),
        facing: &str,
    ) -> Option<ItemStack> {
        let direction = match facing {
            "down" => (0, -1, 0),
            "up" => (0, 1, 0),
            "north" => (0, 0, -1),
            "south" => (0, 0, 1),
            "west" => (-1, 0, 0),
            "east" => (1, 0, 0),
            _ => return None,
        };
        let position = DVec3::new(
            pos.0 as f64 + 0.5 + direction.0 as f64 * 0.7,
            pos.1 as f64 + 0.5 + direction.1 as f64 * 0.7
                - if direction.1 != 0 { 0.125 } else { 0.15625 },
            pos.2 as f64 + 0.5 + direction.2 as f64 * 0.7,
        );
        self.spawn(stack.clone(), position);
        let speed = self.world_random.next_double() * 0.1 + 0.2;
        let spread = 0.0172275 * 6.0;
        let triangle = |random: &mut LegacyRandom, mean: f64| {
            mean + (random.next_double() - random.next_double()) * spread
        };
        if let Some(entity) = self.entities.last_mut() {
            entity.velocity = DVec3::new(
                triangle(&mut self.world_random, direction.0 as f64 * speed),
                triangle(&mut self.world_random, 0.2),
                triangle(&mut self.world_random, direction.2 as f64 * speed),
            );
            entity.pickup_delay = 0;
        }
        Some(stack)
    }

    pub fn player_random_float(&mut self) -> f32 {
        self.player_random.next_float()
    }

    pub fn player_random_double(&mut self) -> f64 {
        self.player_random.next_double()
    }

    /// Separate streams mirror the level, fresh entity and player random
    /// sources. A harness can provide captured seeds for exact trajectories.
    pub fn with_seeds(world: u64, entities: u64, player: u64) -> Self {
        Self {
            entities: Vec::new(),
            pickup_effects: Vec::new(),
            pickup_sounds: 0,
            pickup_transfers: Vec::new(),
            next_entity_id: 2,
            world_random: LegacyRandom::new(world),
            entity_seeds: LegacyRandom::new(entities),
            player_random: LegacyRandom::new(player),
        }
    }
    /// Records a pickup the server performed: the pickup animation, sound
    /// and statistic, as `tick_and_collect` does for client items.
    pub fn note_pickup(&mut self, item: ItemEntity, target: DVec3, transfer: ItemStack) {
        self.pickup_sounds += 1;
        self.pickup_transfers.push(transfer);
        self.pickup_effects.push(PickupEffect { item, age: 0, target });
    }

    /// Ages pickup animations without simulating items (the server owns
    /// them).
    pub fn tick_pickup_effects(&mut self, target: DVec3) {
        for effect in &mut self.pickup_effects {
            effect.age += 1;
            effect.target = target;
        }
        self.pickup_effects.retain(|effect| effect.age < 3);
    }

    pub fn spawn(&mut self, stack: ItemStack, position: DVec3) {
        if stack.count != 0 {
            let mut random = LegacyRandom::new(self.entity_seeds.next_long());
            let bob_offset = random.next_float() * std::f32::consts::TAU;
            let _yaw = random.next_float() * 360.0;
            self.entities.push(ItemEntity {
                entity_id: self.next_entity_id,
                stack,
                position,
                previous_position: position,
                velocity: DVec3::new(
                    random.next_double() * 0.2 - 0.1,
                    0.2,
                    random.next_double() * 0.2 - 0.1,
                ),
                age: 0,
                bob_offset,
                pickup_delay: 10,
                on_ground: false,
            });
            self.next_entity_id = self.next_entity_id.wrapping_add(1);
        }
    }

    /// Block.popResource randomizes all three coordinates inside the broken
    /// block, with the item box's half-height subtracted from Y.
    pub fn spawn_block_drop(&mut self, stack: ItemStack, block: (i32, i32, i32)) {
        let position = DVec3::new(
            block.0 as f64 + 0.25 + self.world_random.next_double() * 0.5,
            block.1 as f64 + 0.125 + self.world_random.next_double() * 0.5,
            block.2 as f64 + 0.25 + self.world_random.next_double() * 0.5,
        );
        self.spawn(stack, position);
    }

    pub fn take_pickup_sounds(&mut self) -> u32 {
        std::mem::take(&mut self.pickup_sounds)
    }

    pub fn take_pickup_transfers(&mut self) -> Vec<ItemStack> {
        std::mem::take(&mut self.pickup_transfers)
    }

    /// `LivingEntity.createItemStackToDrop` for an item the player throws:
    /// from its eyes less `0.3F`, a 0.3 impulse along its look from `Mth`'s
    /// float table (yaw and pitch in degrees), a random sideways spread of
    /// up to 0.02 by `Math.cos`/`Math.sin` in double, and a triangular
    /// vertical spread, as vanilla mixes float and double.
    pub fn toss(&mut self, stack: ItemStack, eye: DVec3, yaw: f32, pitch: f32) {
        if stack.count == 0 {
            return;
        }
        self.spawn(stack, eye - DVec3::Y * f64::from(0.3_f32));
        if let Some(entity) = self.entities.last_mut() {
            let degrees = (std::f64::consts::PI / 180.0) as f32;
            let (sin_x, cos_x) = (crate::mth::sin(f64::from(pitch * degrees)), crate::mth::cos(f64::from(pitch * degrees)));
            let (sin_y, cos_y) = (crate::mth::sin(f64::from(yaw * degrees)), crate::mth::cos(f64::from(yaw * degrees)));
            let direction = self.player_random.next_float() * std::f32::consts::TAU;
            let power = 0.02_f32 * self.player_random.next_float();
            let rise = -sin_x * 0.3_f32 + 0.1_f32 + (self.player_random.next_float() - self.player_random.next_float()) * 0.1_f32;
            entity.velocity = DVec3::new(
                f64::from(-sin_y * cos_x * 0.3_f32) + crate::jmath::cos(f64::from(direction)) * f64::from(power),
                f64::from(rise),
                f64::from(cos_y * cos_x * 0.3_f32) + crate::jmath::sin(f64::from(direction)) * f64::from(power),
            );
            entity.pickup_delay = 40;
        }
    }

    pub fn drop_on_death(&mut self, stack: ItemStack, eye: DVec3) {
        if stack.count == 0 {
            return;
        }
        self.spawn(stack, eye - DVec3::Y * f64::from(0.3_f32));
        if let Some(entity) = self.entities.last_mut() {
            // `createItemStackToDrop(randomly)`: `Mth`'s float table.
            let power = self.player_random.next_float() * 0.5;
            let direction = self.player_random.next_float() * std::f32::consts::TAU;
            entity.velocity = DVec3::new(
                f64::from(-crate::mth::sin(f64::from(direction)) * power),
                f64::from(0.2_f32),
                f64::from(crate::mth::cos(f64::from(direction)) * power),
            );
            entity.pickup_delay = 40;
        }
    }

    /// One 20 Hz step. Returns the number of items successfully transferred.
    pub fn tick_and_collect(
        &mut self,
        world: &impl World,
        player_feet: DVec3,
        inventory: &mut Inventory,
        selected: usize,
    ) -> u32 {
        self.tick_inner(world, player_feet, Some(inventory), selected)
    }

    /// Spectators advance dropped-item physics without touching inventory.
    pub fn tick(&mut self, world: &impl World) {
        self.tick_inner(world, DVec3::ZERO, None, 0);
    }

    /// HopperBlockEntity.suckInItems queries item boxes intersecting the
    /// hopper's 16-by-21-pixel intake column. A full collision cube directly
    /// above blocks that query. Pickup delay does not apply to hoppers.
    pub fn suck_into_hopper(
        &mut self,
        world: &impl World,
        pos: (i32, i32, i32),
        slots: &mut [Option<ItemStack>],
    ) -> bool {
        if world.block((pos.0, pos.1 + 1, pos.2)).is_some_and(|block| {
            local_shapes(&block)
                .iter()
                .any(|shape| shape.min == DVec3::ZERO && shape.max == DVec3::ONE)
        }) {
            return false;
        }
        let min = DVec3::new(pos.0 as f64, pos.1 as f64 + 11.0 / 16.0, pos.2 as f64);
        let max = DVec3::new(pos.0 as f64 + 1.0, pos.1 as f64 + 2.0, pos.2 as f64 + 1.0);
        let mut index = 0;
        while index < self.entities.len() {
            let entity = &mut self.entities[index];
            let p = entity.position;
            let overlaps = p.x - 0.125 < max.x
                && p.x + 0.125 > min.x
                && p.y < max.y
                && p.y + 0.25 > min.y
                && p.z - 0.125 < max.z
                && p.z + 0.125 > min.z;
            if overlaps && absorb_stack(&mut entity.stack, slots) {
                self.entities.remove(index);
                return true;
            }
            index += 1;
        }
        false
    }

    fn tick_inner(
        &mut self,
        world: &impl World,
        player_feet: DVec3,
        mut inventory: Option<&mut Inventory>,
        selected: usize,
    ) -> u32 {
        let mut collected = 0;
        for effect in &mut self.pickup_effects {
            effect.age += 1;
            if inventory.is_some() {
                effect.target = player_feet + DVec3::Y * 0.81;
            }
        }
        self.pickup_effects.retain(|effect| effect.age < 3);
        let mut index = 0;
        while index < self.entities.len() {
            let entity = &mut self.entities[index];
            entity.previous_position = entity.position;
            entity.age += 1;
            entity.pickup_delay = entity.pickup_delay.saturating_sub(1);
            if entity.age >= 6000 {
                self.entities.swap_remove(index);
                continue;
            }
            // ItemEntity.tick applies gravity before Entity.move. The item
            // entity box is 0.25 wide and high; contact occurs when its bottom
            // reaches the block top, not when its center reaches Y+0.125.
            let fluid = item_fluid(world, entity.position);
            match fluid {
                Some("minecraft:water") => {
                    entity.velocity.x *= 0.99_f32 as f64;
                    entity.velocity.z *= 0.99_f32 as f64;
                    if entity.velocity.y < 0.06_f32 as f64 {
                        entity.velocity.y += 0.0005_f32 as f64;
                    }
                }
                Some("minecraft:lava") => {
                    entity.velocity.x *= 0.95_f32 as f64;
                    entity.velocity.z *= 0.95_f32 as f64;
                    if entity.velocity.y < 0.06_f32 as f64 {
                        entity.velocity.y += 0.0005_f32 as f64;
                    }
                }
                _ => entity.velocity.y -= 0.04,
            }
            // ItemEntity.tick leaves resting items in place on three of four
            // ticks when their horizontal speed is below this threshold.
            // Gravity still runs on those ticks; drag waits for the move tick.
            let skip_motion = entity.on_ground
                && entity.velocity.x * entity.velocity.x + entity.velocity.z * entity.velocity.z
                    <= 1.0e-5
                && (entity.age + entity.entity_id) % 4 != 0;
            if !skip_motion {
                let half_width = 0.125;
                let bbox = Box3::new(
                    entity.position + DVec3::new(-half_width, 0.0, -half_width),
                    entity.position + DVec3::new(half_width, 0.25, half_width),
                );
                let requested = entity.velocity;
                let swept = Box3::new(
                    bbox.min.min(bbox.min + requested),
                    bbox.max.max(bbox.max + requested),
                );
                let blocks = shapes_near(world, swept, 0.001);
                let movement = clip_motion(bbox, &blocks, requested);
                // Entity.move omits setPos for a clipped displacement at or
                // below 1e-7 squared when the requested displacement differs
                // by at least 1e-7 squared. Resting items still get drag and
                // collision flags on that tick.
                if movement.length_squared() > 1.0e-7
                    || requested.length_squared() - movement.length_squared() < 1.0e-7
                {
                    entity.position += movement;
                }
                entity.on_ground = requested.y < 0.0 && movement.y > requested.y;
                if movement.x != requested.x {
                    entity.velocity.x = 0.0;
                }
                if movement.z != requested.z {
                    entity.velocity.z = 0.0;
                }
                if movement.y != requested.y {
                    entity.velocity.y = 0.0;
                }
                let below = world.block((
                    entity.position.x.floor() as i32,
                    (entity.position.y - 0.999_999).floor() as i32,
                    entity.position.z.floor() as i32,
                ));
                let block_friction: f32 = below.as_ref().map_or(0.6, |block| {
                    match block.id.rsplit(':').next().unwrap_or(&block.id) {
                        "slime_block" => 0.8,
                        "ice" | "packed_ice" => 0.98,
                        "blue_ice" => 0.989,
                        _ => 0.6,
                    }
                });
                let air_drag = 0.98_f32;
                let horizontal_drag = if entity.on_ground {
                    (air_drag * block_friction) as f64
                } else {
                    air_drag as f64
                };
                entity.velocity.x *= horizontal_drag;
                entity.velocity.y *= air_drag as f64;
                entity.velocity.z *= horizontal_drag;
                if entity.on_ground && entity.velocity.y < 0.0 {
                    entity.velocity.y *= -0.5;
                }
            }
            let delta = entity.position - player_feet;
            // Player.aiStep queries entities in the player's AABB inflated
            // by (1.0, 0.5, 1.0), then calls ItemEntity.playerTouch.
            if let Some(inventory) = inventory.as_deref_mut().filter(|_| {
                entity.pickup_delay == 0
                    && delta.x.abs() < 1.425
                    && delta.z.abs() < 1.425
                    && (-0.75..2.3).contains(&delta.y)
            }) {
                let original = entity.stack.count;
                if let Some(remainder) = inventory.add_item(entity.stack.clone(), selected) {
                    let amount = original - remainder.count;
                    if amount != 0 {
                        collected += amount as u32;
                        self.pickup_sounds += 1;
                        let mut transfer = entity.stack.clone();
                        transfer.count = amount;
                        self.pickup_transfers.push(transfer);
                        self.pickup_effects.push(PickupEffect {
                            item: entity.clone(),
                            age: 0,
                            target: player_feet + DVec3::Y * 0.81,
                        });
                    }
                    entity.stack = remainder;
                } else {
                    collected += original as u32;
                    self.pickup_sounds += 1;
                    self.pickup_transfers.push(entity.stack.clone());
                    self.pickup_effects.push(PickupEffect {
                        item: entity.clone(),
                        age: 0,
                        target: player_feet + DVec3::Y * 0.81,
                    });
                    self.entities.swap_remove(index);
                    continue;
                }
            }
            index += 1;
        }
        self.merge_nearby();
        collected
    }

    fn merge_nearby(&mut self) {
        let mut index = 0;
        while index < self.entities.len() {
            let item = &self.entities[index];
            let moved = item.previous_position.floor() != item.position.floor();
            let rate = if moved { 2 } else { 40 };
            if item.age % rate != 0 || !can_merge_entity(item) {
                index += 1;
                continue;
            }
            let mut other = 0;
            let mut removed_current = false;
            while other < self.entities.len() {
                if other == index || !can_merge_pair(&self.entities[index], &self.entities[other]) {
                    other += 1;
                    continue;
                }
                let (to, from) =
                    if self.entities[other].stack.count < self.entities[index].stack.count {
                        (index, other)
                    } else {
                        (other, index)
                    };
                let source = self.entities[from].clone();
                let target = &mut self.entities[to];
                target.stack.count += source.stack.count;
                target.pickup_delay = target.pickup_delay.max(source.pickup_delay);
                target.age = target.age.min(source.age);
                self.entities.remove(from);
                if from == index {
                    removed_current = true;
                    break;
                }
                if from < index {
                    index -= 1;
                }
                other = 0;
            }
            if !removed_current {
                index += 1;
            }
        }
    }
}

fn can_merge_entity(item: &ItemEntity) -> bool {
    item.age < 6000 && item.pickup_delay != u16::MAX && item.stack.count < item.stack.max
}

fn can_merge_pair(left: &ItemEntity, right: &ItemEntity) -> bool {
    can_merge_entity(left)
        && can_merge_entity(right)
        && left.stack.same_item(&right.stack)
        && (left.stack.count as u16 + right.stack.count as u16) <= left.stack.max as u16
        && (left.position.x - right.position.x).abs() < 0.75
        && (left.position.y - right.position.y).abs() < 0.25
        && (left.position.z - right.position.z).abs() < 0.75
}

fn item_fluid(world: &impl World, position: DVec3) -> Option<&'static str> {
    for y in position.y.floor() as i32..=(position.y + 0.25).floor() as i32 {
        let Some(block) = world.block((position.x.floor() as i32, y, position.z.floor() as i32))
        else {
            continue;
        };
        let fluid = match block.id.as_str() {
            "minecraft:water" => "minecraft:water",
            "minecraft:lava" => "minecraft:lava",
            _ => continue,
        };
        let level = block
            .property("level")
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(0);
        let above = world.block((position.x.floor() as i32, y + 1, position.z.floor() as i32));
        let full = above.as_ref().is_some_and(|above| above.id == block.id);
        let height = if full || level >= 8 {
            1.0
        } else {
            (8 - level) as f64 / 9.0
        };
        let immersed = (y as f64 + height).min(position.y + 0.25) - position.y.max(y as f64);
        if immersed > 0.1_f32 as f64 {
            return Some(fluid);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Block, Pos};

    struct Floor;
    impl World for Floor {
        fn block(&self, pos: Pos) -> Option<Block> {
            (pos.1 == 0).then(|| Block::new("minecraft:stone"))
        }
        fn set_block(&mut self, _pos: Pos, _block: Option<Block>) {}
    }

    #[test]
    fn drop_waits_ten_ticks_then_reenters_inventory() {
        let mut items = WorldItems::default();
        let mut inventory = Inventory::default();
        let feet = DVec3::new(0.5, 1.0, 0.5);
        items.spawn(ItemStack::new("minecraft:stone", 5), feet + DVec3::Y * 0.5);
        for _ in 0..9 {
            assert_eq!(items.tick_and_collect(&Floor, feet, &mut inventory, 0), 0);
        }
        assert_eq!(items.tick_and_collect(&Floor, feet, &mut inventory, 0), 5);
        assert_eq!(inventory.count("minecraft:stone"), 5);
        assert!(items.entities.is_empty());
    }

    #[test]
    fn full_inventory_leaves_item_entity_in_world() {
        let mut items = WorldItems::default();
        let mut inventory = Inventory::default();
        for slot in &mut inventory.slots[..36] {
            *slot = Some(ItemStack::new("minecraft:dirt", 64));
        }
        let feet = DVec3::ZERO;
        items.spawn(ItemStack::new("minecraft:stone", 2), feet + DVec3::Y);
        for _ in 0..10 {
            items.tick_and_collect(&Floor, feet, &mut inventory, 0);
        }
        assert_eq!(items.entities[0].stack.count, 2);
    }

    #[test]
    fn hopper_takes_delayed_item_without_player_pickup_effect() {
        let mut items = WorldItems::default();
        items.spawn(
            ItemStack::new("minecraft:stone", 5),
            DVec3::new(0.5, 2.0, 0.5),
        );
        let mut slots: [Option<ItemStack>; 5] = std::array::from_fn(|_| None);
        assert!(items.suck_into_hopper(&Floor, (0, 1, 0), &mut slots));
        assert!(items.entities.is_empty());
        assert_eq!(slots[0].as_ref().unwrap().count, 5);
        assert!(items.pickup_effects.is_empty());
    }

    #[test]
    fn hopper_partial_entity_fit_changes_stack_without_cooldown_success() {
        let mut items = WorldItems::default();
        items.spawn(
            ItemStack::new("minecraft:stone", 5),
            DVec3::new(0.5, 2.0, 0.5),
        );
        let mut slots: [Option<ItemStack>; 5] =
            std::array::from_fn(|_| Some(ItemStack::new("minecraft:dirt", 64)));
        slots[0] = Some(ItemStack::new("minecraft:stone", 63));
        assert!(!items.suck_into_hopper(&Floor, (0, 1, 0), &mut slots));
        assert_eq!(slots[0].as_ref().unwrap().count, 64);
        assert_eq!(items.entities[0].stack.count, 4);
    }

    #[test]
    fn full_cube_above_hopper_blocks_item_suction() {
        let mut items = WorldItems::default();
        items.spawn(
            ItemStack::new("minecraft:stone", 1),
            DVec3::new(0.5, 0.25, 0.5),
        );
        let mut slots: [Option<ItemStack>; 5] = std::array::from_fn(|_| None);
        assert!(!items.suck_into_hopper(&Floor, (0, -1, 0), &mut slots));
        assert_eq!(items.entities[0].stack.count, 1);
    }

    #[test]
    fn resting_items_merge_on_fortieth_tick_into_larger_stack() {
        let mut items = WorldItems::default();
        for count in [2, 3] {
            items.spawn(
                ItemStack::new("minecraft:stone", count),
                DVec3::new(0.5, 1.0, 0.5),
            );
        }
        for item in &mut items.entities {
            item.age = 39;
            item.on_ground = true;
            item.velocity = DVec3::ZERO;
        }
        items.entities[0].pickup_delay = 40;
        items.entities[1].pickup_delay = 20;
        items.tick(&Floor);
        assert_eq!(items.entities.len(), 1);
        assert_eq!(items.entities[0].entity_id, 3);
        assert_eq!(items.entities[0].stack.count, 5);
        assert_eq!(items.entities[0].pickup_delay, 39);
        assert_eq!(items.entities[0].age, 40);
    }

    #[test]
    fn submerged_item_buoys_up_instead_of_receiving_air_gravity() {
        struct Pool;
        impl World for Pool {
            fn block(&self, pos: Pos) -> Option<Block> {
                match pos.1 {
                    0 => Some(Block::new("minecraft:stone")),
                    1 => Some(Block::new("minecraft:water").with("level", "0")),
                    _ => None,
                }
            }
            fn set_block(&mut self, _pos: Pos, _block: Option<Block>) {}
        }
        let mut items = WorldItems::default();
        items.spawn(
            ItemStack::new("minecraft:stone", 1),
            DVec3::new(0.5, 1.1, 0.5),
        );
        items.entities[0].velocity = DVec3::ZERO;
        items.tick(&Pool);
        assert!(items.entities[0].velocity.y > 0.0);
        assert!(items.entities[0].position.y > 1.1);
    }

    #[test]
    fn pickup_uses_inflated_player_box_and_three_tick_flight() {
        let mut items = WorldItems::default();
        let mut inventory = Inventory::default();
        let feet = DVec3::new(0.5, 1.0, 0.5);
        items.spawn(
            ItemStack::new("minecraft:dirt", 1),
            feet + DVec3::new(1.0, 0.0, 0.0),
        );
        items.entities[0].pickup_delay = 0;
        items.entities[0].velocity = DVec3::ZERO;
        assert_eq!(items.tick_and_collect(&Floor, feet, &mut inventory, 0), 1);
        assert_eq!(inventory.count("minecraft:dirt"), 1);
        let transfers = items.take_pickup_transfers();
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].count, 1);
        assert_eq!(items.pickup_effects.len(), 1);
        let start = items.pickup_effects[0].position(0.0);
        assert!(
            items.pickup_effects[0]
                .position(1.0)
                .distance(feet + DVec3::Y * 0.81)
                < start.distance(feet + DVec3::Y * 0.81)
        );
        for _ in 0..3 {
            items.tick_and_collect(&Floor, feet, &mut inventory, 0);
        }
        assert!(items.pickup_effects.is_empty());
    }

    #[test]
    fn dropped_stone_matches_pinned_client_motion() {
        // harness/run_stage3_client.py drop_stone d, Minecraft 26.3, observed
        // item age 1 onward. The random throw impulse is taken from the first
        // observed state, so this checks subsequent gravity, drag and contact.
        let start = DVec3::new(0.5101934932552035, 2.3487908125996246, 4.202374412500763);
        let mut items = WorldItems {
            entities: vec![ItemEntity {
                entity_id: 0,
                stack: ItemStack::new("minecraft:stone", 1),
                position: start,
                previous_position: start,
                velocity: DVec3::new(
                    0.009989623584524852,
                    0.028215003906279706,
                    -0.2916730814260099,
                ),
                age: 1,
                bob_offset: 0.0,
                pickup_delay: 40,
                on_ground: false,
            }],
            ..Default::default()
        };
        let mut inventory = Inventory::default();
        let samples = [
            (
                2,
                0.5201831168397283,
                2.337005816505904,
                3.9107013310747534,
                -0.01154929639662685,
                false,
            ),
            (
                9,
                0.584734559997,
                1.184550648,
                2.025953815,
                -0.268500215,
                false,
            ),
            (10, 0.5932333633226178, 1.0, 1.7778091044246156, 0.0, true),
        ];
        for age in 2..=10 {
            items.tick_and_collect(&Floor, DVec3::new(20.0, 1.0, 20.0), &mut inventory, 0);
            let actual = &items.entities[0];
            if let Some((_, x, y, z, vy, grounded)) = samples.iter().find(|sample| sample.0 == age)
            {
                assert!(
                    (actual.position.x - x).abs() < 1e-7,
                    "age {age} x: {}",
                    actual.position.x
                );
                assert!(
                    (actual.position.y - y).abs() < 1e-7,
                    "age {age} y: {}",
                    actual.position.y
                );
                assert!(
                    (actual.position.z - z).abs() < 1e-7,
                    "age {age} z: {}",
                    actual.position.z
                );
                assert!(
                    (actual.velocity.y - vy).abs() < 1e-7,
                    "age {age} vy: {}",
                    actual.velocity.y
                );
                assert_eq!(actual.on_ground, *grounded, "age {age} grounded");
            }
        }

        // Independent repeat with a different vanilla throw impulse. Its
        // first floor contact happens a tick earlier.
        let start = DVec3::new(0.4986571446011109, 2.321994743503611, 4.185405603369347);
        items.entities[0] = ItemEntity {
            entity_id: 2,
            stack: ItemStack::new("minecraft:stone", 1),
            position: start,
            previous_position: start,
            velocity: DVec3::new(
                -0.0013159983165242717,
                0.001954855681091467,
                -0.30830251469845255,
            ),
            age: 1,
            bob_offset: 0.0,
            pickup_delay: 40,
            on_ground: false,
        };
        for _ in 2..=9 {
            items.tick_and_collect(&Floor, DVec3::new(20.0, 1.0, 20.0), &mut inventory, 0);
        }
        let actual = &items.entities[0];
        assert!((actual.position.x - 0.48883736340017625).abs() < 1e-7);
        assert!((actual.position.y - 1.0).abs() < 1e-7);
        assert!((actual.position.z - 1.884898680205008).abs() < 1e-7);
        assert_eq!(actual.velocity.y, 0.0);
        assert!(actual.on_ground);
    }
}
