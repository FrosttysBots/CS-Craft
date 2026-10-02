//! `MultifaceGrowthFeature` and `MultifaceSpreader` for glow lichen and sculk veins.

use super::{bool_or, float_or, int_or, shuffle};
use crate::feature::blocks::{BlockSet, FluidType};
use crate::feature::kinds::simple::can_attach_to;
use crate::feature::{Ctx, Library};
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::{BlockId, BlockPos, BlockStateId, SupportType};
use serde_json::Value;

/// `MultifaceSpreader.SpreadType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpreadType {
    SamePosition,
    SamePlane,
    WrapAround,
}

pub const DEFAULT_ORDER: &[SpreadType] = &[SpreadType::SamePosition, SpreadType::SamePlane, SpreadType::WrapAround];

/// A `MultifaceSpreader` with its config: the default one (glow lichen) or
/// `SculkVeinSpreaderConfig`.
#[derive(Clone, Copy, Debug)]
pub struct Spreader {
    pub block: BlockId,
    pub sculk: bool,
    pub types: &'static [SpreadType],
}

impl Spreader {
    fn is_this(&self, ctx: &Ctx, state: BlockStateId) -> bool {
        ctx.registries().blocks.block_of(state) == self.block
    }

    pub fn has_face(ctx: &Ctx, state: BlockStateId, face: Direction) -> bool {
        ctx.property(state, face.name()) == Some("true")
    }

    /// `MultifaceBlock.isValidStateForPlacement`.
    fn valid_for_placement(&self, ctx: &Ctx, old: BlockStateId, pos: BlockPos, direction: Direction) -> bool {
        if self.is_this(ctx, old) && Self::has_face(ctx, old, direction) {
            return false;
        }
        can_attach_to(ctx, direction, ctx.block(pos.relative(direction, 1)))
    }

    /// `MultifaceBlock.getStateForPlacement(oldState, level, pos, direction)`.
    pub fn state_for_placement(&self, ctx: &Ctx, old: BlockStateId, pos: BlockPos, direction: Direction) -> Option<BlockStateId> {
        if !self.valid_for_placement(ctx, old, pos, direction) {
            return None;
        }
        let blocks = &ctx.registries().blocks;
        let state = if self.is_this(ctx, old) {
            old
        } else if ctx.fluid(old) == FluidType::Water {
            ctx.with(blocks.block(self.block).default_state(), "waterlogged", "true")
        } else {
            blocks.block(self.block).default_state()
        };
        Some(ctx.with(state, direction.name(), "true"))
    }

    fn state_can_be_replaced(&self, ctx: &Ctx, source: BlockPos, target: BlockPos, face: Direction, existing: BlockStateId) -> bool {
        let default = |ctx: &Ctx| ctx.is_air(existing) || self.is_this(ctx, existing) || existing == ctx.lib.blocks.water;
        if !self.sculk {
            return default(ctx);
        }
        let against = ctx.block(target.relative(face, 1));
        if ctx.is(against, "minecraft:sculk") || ctx.is(against, "minecraft:sculk_catalyst") || ctx.is(against, "minecraft:moving_piston") {
            return false;
        }
        if source.dist_manhattan(target) == 2 {
            let neighbor = source.relative(face.opposite(), 1);
            if ctx.registries().blocks.is_face_sturdy(ctx.block(neighbor), face, SupportType::Full) {
                return false;
            }
        }
        let fluid = ctx.fluid(existing);
        if fluid != FluidType::Empty && fluid != FluidType::Water {
            return false;
        }
        if ctx.in_tag(existing, ctx.lib.tags.fire) {
            return false;
        }
        ctx.is_replaceable(existing) || default(ctx)
    }

    fn can_spread_into(&self, ctx: &Ctx, source: BlockPos, target: BlockPos, face: Direction) -> bool {
        let existing = ctx.block(target);
        self.state_can_be_replaced(ctx, source, target, face, existing) && self.valid_for_placement(ctx, existing, target, face)
    }

    fn other_source(&self, ctx: &Ctx, state: BlockStateId) -> bool {
        self.sculk && !self.is_this(ctx, state)
    }

    /// `spreadFromFaceTowardDirection`.
    pub fn spread_toward(&self, ctx: &mut Ctx, state: BlockStateId, pos: BlockPos, from_face: Direction, spread: Direction, post_process: bool) -> bool {
        if spread.axis() == from_face.axis() {
            return false;
        }
        if !(self.other_source(ctx, state) || Self::has_face(ctx, state, from_face) && !Self::has_face(ctx, state, spread)) {
            return false;
        }
        for &kind in self.types {
            let (target, face) = match kind {
                SpreadType::SamePosition => (pos, spread),
                SpreadType::SamePlane => (pos.relative(spread, 1), from_face),
                SpreadType::WrapAround => (pos.relative(spread, 1).relative(from_face, 1), spread.opposite()),
            };
            if self.can_spread_into(ctx, pos, target, face) {
                let old = ctx.block(target);
                let Some(new) = self.state_for_placement(ctx, old, target, face) else {
                    return false;
                };
                if post_process {
                    ctx.region.mark_post_processing(target.x, target.y, target.z);
                }
                return ctx.set_block(target, new);
            }
        }
        false
    }

    /// `spreadFromFaceTowardRandomDirection`.
    pub fn spread_toward_random(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, state: BlockStateId, pos: BlockPos, from_face: Direction, post_process: bool) -> bool {
        let mut directions = Direction::ALL;
        shuffle(&mut directions, random);
        directions.iter().any(|&spread| self.spread_toward(ctx, state, pos, from_face, spread, post_process))
    }

    /// `spreadAll`: every face that can spread, toward every direction.
    pub fn spread_all(&self, ctx: &mut Ctx, state: BlockStateId, pos: BlockPos, post_process: bool) -> i64 {
        let mut count = 0;
        for face in Direction::ALL {
            if !(self.other_source(ctx, state) || Self::has_face(ctx, state, face)) {
                continue;
            }
            for spread in Direction::ALL {
                if self.spread_toward(ctx, state, pos, face, spread, post_process) {
                    count += 1;
                }
            }
        }
        count
    }
}

#[derive(Debug)]
pub struct MultifaceGrowth {
    spreader: Spreader,
    search_range: i32,
    floor: bool,
    ceiling: bool,
    wall: bool,
    chance_of_spreading: f32,
    can_be_placed_on: BlockSet,
}

impl MultifaceGrowth {
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let name = json["block"].as_str().ok_or("multiface_growth lacks block")?;
        let block = lib.registries.blocks.block_by_name(name).ok_or_else(|| format!("unknown block {name}"))?;
        Ok(Self {
            spreader: Spreader { block, sculk: name.trim_start_matches("minecraft:") == "sculk_vein", types: DEFAULT_ORDER },
            search_range: int_or(json, "search_range", 10),
            floor: bool_or(json, "can_place_on_floor", false),
            ceiling: bool_or(json, "can_place_on_ceiling", false),
            wall: bool_or(json, "can_place_on_wall", false),
            chance_of_spreading: float_or(json, "chance_of_spreading", 0.5),
            can_be_placed_on: BlockSet::parse(&lib.registries, &json["can_be_placed_on"])?,
        })
    }

    fn valid_directions(&self) -> Vec<Direction> {
        let mut out = Vec::with_capacity(6);
        if self.ceiling {
            out.push(Direction::Up);
        }
        if self.floor {
            out.push(Direction::Down);
        }
        if self.wall {
            out.extend(Direction::HORIZONTAL);
        }
        out
    }

    fn is_air_or_water(ctx: &Ctx, state: BlockStateId) -> bool {
        ctx.is_air(state) || state == ctx.lib.blocks.water
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !Self::is_air_or_water(ctx, ctx.block(origin)) {
            return false;
        }
        let mut directions = self.valid_directions();
        shuffle(&mut directions, random);
        if self.place_growth(ctx, random, origin, ctx.block(origin), &directions) {
            return true;
        }
        for &search in &directions {
            let mut placement: Vec<Direction> = self.valid_directions().into_iter().filter(|&d| d != search.opposite()).collect();
            shuffle(&mut placement, random);
            for _ in 0..self.search_range {
                // setWithOffset(origin, direction): always one step from the origin.
                let pos = origin.relative(search, 1);
                let state = ctx.block(pos);
                if !Self::is_air_or_water(ctx, state) && ctx.registries().blocks.block_of(state) != self.spreader.block {
                    break;
                }
                if self.place_growth(ctx, random, pos, state, &placement) {
                    return true;
                }
            }
        }
        false
    }

    fn place_growth(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, pos: BlockPos, old: BlockStateId, placement: &[Direction]) -> bool {
        for &direction in placement {
            let neighbor = ctx.block(pos.relative(direction, 1));
            if !self.can_be_placed_on.contains(ctx.registries(), neighbor) {
                continue;
            }
            let Some(state) = self.spreader.state_for_placement(ctx, old, pos, direction) else {
                return false;
            };
            ctx.set_block_update(pos, state);
            ctx.region.mark_post_processing(pos.x, pos.y, pos.z);
            if random.next_f32() < self.chance_of_spreading {
                self.spreader.spread_toward_random(ctx, random, state, pos, direction, true);
            }
            return true;
        }
        false
    }
}
