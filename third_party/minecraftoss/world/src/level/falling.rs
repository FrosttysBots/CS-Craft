//! Falling blocks on the server level (26.3 `FallingBlock`,
//! `FallingBlockEntity`, `BrushableBlock`'s fall): gravity blocks leave
//! their place as an entity when the block below is free, fall with the
//! entity movement pipeline, and land as a block, or break into an item
//! where they cannot.

use super::entity::{Entity, EntityKind, FallingData};
use super::update;
use super::Level;
use minecraftoss_core::block::{flags, FluidKind};
use minecraftoss_core::{BlockPos, BlockStateId};

impl Level<'_> {
    /// `FallingBlock.isFree`: air, fire, liquids and replaceable blocks.
    pub(super) fn is_free_for_falling(&self, state: BlockStateId) -> bool {
        let blocks = &self.registries().blocks;
        blocks.is_air(state)
            || self.lib.registries.block_in_tag(state, self.fire_tag)
            || blocks.is(state, flags::LIQUID)
            || blocks.is(state, flags::REPLACEABLE)
    }

    /// `BlockState.canBeReplaced(context)` for a context without an item
    /// (`DirectionalPlaceContext` with `ItemStack.EMPTY`).
    fn replaceable_without_item(&self, state: BlockStateId) -> bool {
        let blocks = &self.registries().blocks;
        if self.is_a(state, "MultifaceBlock") {
            return true;
        }
        if self.is_a(state, "ScaffoldingBlock") || self.is_a(state, "SlabBlock") {
            return false;
        }
        if self.name(state) == "minecraft:snow" {
            return blocks.property(state, "layers") == Some("1");
        }
        if self.name(state) == "minecraft:vine" {
            let faces = ["up", "north", "east", "south", "west"].iter().filter(|f| blocks.property(state, f) == Some("true")).count();
            return faces < 5;
        }
        blocks.is(state, flags::REPLACEABLE)
    }

    /// `FallingBlock.tick` (and `BrushableBlock.tick`, whose entity drops
    /// nothing).
    pub(super) fn falling_block_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        if !self.is_free_for_falling(self.block(pos.below())) || pos.y < self.min_y {
            return;
        }
        let brushable = self.is_a(state, "BrushableBlock");
        if brushable || self.is_a(state, "AnvilBlock") || self.is_a(state, "PointedDripstoneBlock") {
            // Their landing and block entity rules are not simulated yet.
            self.unsupported.push(format!("falling {}", self.name(state)));
        }
        // `FallingBlockEntity.fall`: the entity keeps no water.
        let mut falling = state;
        if self.registries().blocks.property(state, "waterlogged").is_some() {
            falling = self.with(state, "waterlogged", "false");
        }
        let entity = Entity::falling_block([f64::from(pos.x) + 0.5, f64::from(pos.y), f64::from(pos.z) + 0.5], FallingData { state: falling, time: 0, drop_item: !brushable });
        let fluid = self.fluid_legacy_block(self.fluid_state(state));
        self.set_block_and_update(pos, fluid);
        self.add_entity(entity);
    }

    /// `FallingBlockEntity.tick` (it skips `baseTick`).
    pub(super) fn tick_falling_block(&mut self, e: &mut Entity) {
        let EntityKind::FallingBlock(data) = &mut e.kind else { return };
        if self.registries().blocks.is_air(data.state) {
            e.removed = true;
            return;
        }
        data.time += 1;
        let (state, time, drop_item) = (data.state, data.time, data.drop_item);
        let gravity = e.gravity();
        if gravity != 0.0 {
            e.delta = [e.delta[0] + 0.0, e.delta[1] - gravity, e.delta[2] + 0.0];
        }
        let delta = e.delta;
        self.entity_move_self(e, delta);
        self.entity_apply_effects_from_blocks(e);
        let pos = BlockPos::new(e.pos[0].floor() as i32, e.pos[1].floor() as i32, e.pos[2].floor() as i32);
        let concrete = self.is_a(state, "ConcretePowderBlock");
        let mut pos = pos;
        let water_here = self.fluid_state(self.block(pos)).is_some_and(|f| f.kind == FluidKind::Water);
        let mut stuck_in_water = concrete && water_here;
        let speed = e.delta[0] * e.delta[0] + e.delta[1] * e.delta[1] + e.delta[2] * e.delta[2];
        if concrete && speed > 1.0 {
            // A fast fall looks back along its path for water sources.
            let old = e.old_position();
            if let Some(hit) = self.clip_first_hit(old, e.pos, super::physics::ClipBlocks::Collider, super::physics::ClipFluids::SourceOnly) {
                if self.fluid_state(self.block(hit)).is_some_and(|f| f.kind == FluidKind::Water) {
                    pos = hit;
                    stuck_in_water = true;
                }
            }
        }
        if !e.on_ground && !stuck_in_water {
            if time > 100 && (pos.y <= self.min_y || pos.y > self.min_y + self.height - 1) || time > 600 {
                if drop_item {
                    self.falling_block_drop(e, state);
                }
                e.removed = true;
            }
        } else {
            let current = self.block(pos);
            e.delta = [e.delta[0] * 0.7, e.delta[1] * -0.5, e.delta[2] * 0.7];
            if self.name(current) != "minecraft:moving_piston" {
                let may_replace = self.replaceable_without_item(current);
                let would_continue = self.is_free_for_falling(self.block(pos.below())) && (!concrete || !stuck_in_water);
                let would_survive = self.can_survive_state(state, pos) && !would_continue;
                if may_replace && would_survive {
                    let mut placed = state;
                    let source = self.fluid_state(current).is_some_and(|f| f.kind == FluidKind::Water && f.source);
                    if source && self.registries().blocks.property(placed, "waterlogged").is_some() {
                        placed = self.with(placed, "waterlogged", "true");
                    }
                    if self.set_block(pos, placed, update::ALL, update::LIMIT) {
                        e.removed = true;
                        self.falling_block_landed(placed, pos, current);
                    } else if drop_item {
                        e.removed = true;
                        self.falling_block_drop(e, state);
                    }
                } else {
                    e.removed = true;
                    if drop_item {
                        self.falling_block_drop(e, state);
                    }
                }
            }
        }
        let drag = f64::from(0.98f32);
        e.delta = [e.delta[0] * drag, e.delta[1] * drag, e.delta[2] * drag];
    }

    /// `Fallable.onLand`: concrete powder sets where it meets water.
    fn falling_block_landed(&mut self, state: BlockStateId, pos: BlockPos, replaced: BlockStateId) {
        if self.is_a(state, "ConcretePowderBlock") {
            let lib = self.lib;
            let touches = {
                let ctx: minecraftoss_generator::feature::Ctx = minecraftoss_generator::feature::Ctx { lib, region: self };
                minecraftoss_generator::feature::update::concrete_touches_liquid(&ctx, pos)
            };
            let in_water = self.fluid_state(replaced).is_some_and(|f| f.kind == FluidKind::Water);
            if in_water || touches {
                let name = self.name(state).trim_end_matches("_powder").to_owned();
                let concrete = self.registries().blocks.parse_state(&name).expect("vanilla block");
                self.set_block(pos, concrete, update::ALL, update::LIMIT);
            }
        }
    }

    /// `spawnAtLocation(level, block)` with the `entity_drops` game rule:
    /// the block's item, launched by the item entity's own random.
    fn falling_block_drop(&mut self, e: &Entity, state: BlockStateId) {
        if !self.entity_drops {
            return;
        }
        let name = self.name(state).to_owned();
        use minecraftoss_core::random::RandomSource;
        let dx = self.entity_random.next_f64() * 0.2 - 0.1;
        let dz = self.entity_random.next_f64() * 0.2 - 0.1;
        let mut item = Entity::item(0, e.pos, minecraftoss_core::item::ItemStack::new(&name, 1), [dx, 0.2, dz]);
        if let EntityKind::Item(data) = &mut item.kind {
            data.pickup_delay = 10;
        }
        self.add_entity(item);
    }
}
