//! Block drops on the server level (26.3 `Block.dropResources`,
//! `Block.popResource`, `Level.destroyBlock`, `spawnAfterBreak` for blocks
//! that drop experience), with loot from `minecraftoss_core::loot`.
//!
//! Loot draws from the world's named random sequences; placing the dropped
//! items draws from the level random. The items' own launch uses the
//! entity's unseeded random in vanilla, so their motion is not repeatable.

use super::container::Stack;
use super::entity::Entity;
use super::{update, Level};
use minecraftoss_core::loot::LootParams;
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::{BlockPos, BlockStateId};

/// Experience ranges of `DropExperienceBlock`s (`UniformInt` or constant).
fn experience_range(name: &str) -> Option<(i32, i32)> {
    Some(match name {
        "minecraft:coal_ore" | "minecraft:deepslate_coal_ore" => (0, 2),
        "minecraft:diamond_ore" | "minecraft:deepslate_diamond_ore" | "minecraft:emerald_ore" | "minecraft:deepslate_emerald_ore" => (3, 7),
        "minecraft:lapis_ore" | "minecraft:deepslate_lapis_ore" | "minecraft:nether_quartz_ore" => (2, 5),
        "minecraft:nether_gold_ore" => (0, 1),
        "minecraft:sculk" => (1, 1),
        "minecraft:sculk_catalyst" | "minecraft:sculk_sensor" | "minecraft:calibrated_sculk_sensor" | "minecraft:sculk_shrieker" => (5, 5),
        "minecraft:redstone_ore" | "minecraft:deepslate_redstone_ore" => (1, 5),
        _ => return None,
    })
}

impl Level<'_> {
    /// `Block.dropResources(state, level, pos, blockEntity, breaker, tool)`
    /// with an empty tool and no breaker.
    pub(super) fn drop_resources(&mut self, state: BlockStateId, pos: BlockPos) {
        let params = LootParams {
            origin: Some([f64::from(pos.x) + 0.5, f64::from(pos.y) + 0.5, f64::from(pos.z) + 0.5]),
            tool: Some(Stack::empty()),
            ..LootParams::default()
        };
        let registries = self.lib.registries.clone();
        let drops = registries.loot.block_drops(&registries, state, &params, &mut self.random_sequences, &mut self.random);
        match drops {
            Ok(stacks) => {
                for stack in stacks {
                    self.pop_resource(pos, stack);
                }
            }
            Err(e) => self.unsupported.push(format!("drops of {}: {e}", self.name(state))),
        }
        self.spawn_after_break(state, pos);
    }

    /// `Block.popResource`: an item entity near the block's centre.
    pub(super) fn pop_resource(&mut self, pos: BlockPos, stack: Stack) {
        // The position is drawn before the stack and game rule checks.
        let half_height = 0.125;
        let jitter = |level: &mut Self| level.random.next_f64() * 0.5 - 0.25;
        let x = f64::from(pos.x) + 0.5 + jitter(self);
        let y = f64::from(pos.y) + 0.5 + jitter(self) - half_height;
        let z = f64::from(pos.z) + 0.5 + jitter(self);
        if stack.is_empty() || !self.block_drops {
            return;
        }
        self.spawn_popped_item([x, y, z], stack);
    }

    /// `popResource`'s item entity once placed: `new ItemEntity(level, x, y,
    /// z, stack)` (its own random's throw) with the default pickup delay.
    pub fn spawn_popped_item(&mut self, at: [f64; 3], stack: Stack) {
        let dx = self.entity_random.next_f64() * 0.2 - 0.1;
        let dz = self.entity_random.next_f64() * 0.2 - 0.1;
        let mut entity = Entity::item(0, at, stack, [dx, 0.2, dz]);
        if let super::entity::EntityKind::Item(data) = &mut entity.kind {
            // `setDefaultPickUpDelay`.
            data.pickup_delay = 10;
        }
        self.add_entity(entity);
    }

    /// `Block.getDrops` for `state` at `pos` broken by a mob with no tool,
    /// from the world's random sequences (the level random for tables
    /// without one).
    pub fn mob_block_drops(&mut self, state: BlockStateId, pos: BlockPos) -> Vec<Stack> {
        let params = LootParams {
            origin: Some([f64::from(pos.x) + 0.5, f64::from(pos.y) + 0.5, f64::from(pos.z) + 0.5]),
            tool: Some(Stack::empty()),
            this_entity: true,
            ..LootParams::default()
        };
        let registries = self.lib.registries.clone();
        match registries.loot.block_drops(&registries, state, &params, &mut self.random_sequences, &mut self.random) {
            Ok(stacks) => stacks,
            Err(e) => {
                self.unsupported.push(format!("drops of {}: {e}", self.name(state)));
                Vec::new()
            }
        }
    }

    /// `BlockBehaviour.spawnAfterBreak` for experience-dropping blocks,
    /// broken without a tool (`dropExperience` true).
    fn spawn_after_break(&mut self, state: BlockStateId, pos: BlockPos) {
        let name = self.name(state).to_owned();
        if let Some((min, max)) = experience_range(&name) {
            // `UniformInt.sample(level.random)`, then experience orbs.
            let amount = if min >= max { min } else { self.random.next_i32_bound(max - min + 1) + min };
            if amount > 0 {
                let _ = pos;
                self.unsupported.push("experience orbs".to_owned());
            }
        }
    }

    /// `shouldChangedStateKeepBlockEntity`: copper chests and copper golem
    /// statues keep theirs through weathering and waxing.
    pub(super) fn keeps_block_entity(&self, old: BlockStateId, new: BlockStateId) -> bool {
        let blocks = &self.registries().blocks;
        let (old, new) = (blocks.block(blocks.block_of(old)), blocks.block(blocks.block_of(new)));
        ["CopperChestBlock", "CopperGolemStatueBlock"].iter().any(|class| new.is_a(class) && old.is_a(class))
    }

    /// The items `BlockEntity.preRemoveSideEffects` drops: a container's
    /// contents (`Containers.dropContents`). Shulker boxes keep theirs.
    pub(super) fn pre_remove_side_effects(&mut self, pos: BlockPos, old: BlockStateId) -> Vec<Stack> {
        match self.store_of(old) {
            Some(super::container::Store::ShulkerBox) => Vec::new(),
            // Every slot, empty ones included: each draws a position.
            Some(_) => self.block_container_items(pos).unwrap_or_default(),
            None => {
                let holds_items = self.block_entity(pos).is_some_and(|t| t.get("Items").is_some() || t.get("item").is_some() || t.get("RecordItem").is_some() || t.get("Book").is_some());
                if holds_items {
                    self.unsupported.push(format!("removal side effects of {}", self.name(old)));
                }
                Vec::new()
            }
        }
    }

    /// `Containers.dropItemStack`: stacks of 10 to 30 thrown from a random
    /// point in the block, all drawn from the level random.
    pub(super) fn drop_item_stack(&mut self, at: [f64; 3], mut stack: Stack) {
        let size = f64::from(0.25f32);
        let center_range = 1.0 - size;
        let half = size / 2.0;
        let x = at[0].floor() + self.random.next_f64() * center_range + half;
        let y = at[1].floor() + self.random.next_f64() * center_range;
        let z = at[2].floor() + self.random.next_f64() * center_range + half;
        let spread = 0.11485000171139836;
        while !stack.is_empty() {
            let amount = (self.random.next_i32_bound(21) + 10).min(stack.count);
            let mut part = stack.clone();
            part.count = amount;
            stack.count -= amount;
            // The constructor's own motion comes from the entity's random and
            // is replaced at once.
            let _ = (self.entity_random.next_f64(), self.entity_random.next_f64());
            let mut triangle = |mean: f64| mean + spread * (self.random.next_f64() - self.random.next_f64());
            let delta = [triangle(0.0), triangle(0.2), triangle(0.0)];
            self.add_entity(Entity::item(0, [x, y, z], part, delta));
        }
    }

    /// `Level.destroyBlock(pos, dropResources)`: the block becomes its fluid.
    pub fn destroy_block_drops(&mut self, pos: BlockPos, drop: bool, limit: i32) -> bool {
        let state = self.block(pos);
        if self.registries().blocks.is_air(state) {
            return false;
        }
        if drop {
            self.drop_resources(state, pos);
        }
        let replacement = self.fluid_legacy_block(self.fluid_state(state));
        self.set_block(pos, replacement, update::ALL, limit)
    }
}
