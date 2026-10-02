//! Flowing water and lava (26.3 `FlowingFluid`, `WaterFluid`, `LavaFluid`,
//! `LiquidBlock.shouldSpreadLiquid`, `SimpleWaterloggedBlock`).

use super::ticks::FluidType;
use super::{update, Level};
use minecraftoss_core::block::{flags, FaceShape, FluidInfo, FluidKind};
use minecraftoss_core::pos::{Axis, Direction};
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::{BlockPos, BlockStateId};
use std::collections::HashMap;

/// The fluid type a fluid state belongs to (`FluidState.getType`).
pub fn fluid_type(fluid: &FluidInfo) -> FluidType {
    match (fluid.kind, fluid.source) {
        (FluidKind::Water, true) => FluidType::Water,
        (FluidKind::Water, false) => FluidType::FlowingWater,
        (FluidKind::Lava, true) => FluidType::Lava,
        (FluidKind::Lava, false) => FluidType::FlowingLava,
    }
}

/// `Direction.Plane.HORIZONTAL`.
const HORIZONTAL: [Direction; 4] = [Direction::North, Direction::East, Direction::South, Direction::West];

/// A fluid state (`FluidState`) in this module: kind, amount (1..=8),
/// source and falling.
pub type Fluid = FluidInfo;

fn is_source_of(state: Option<Fluid>, kind: FluidKind) -> bool {
    state.is_some_and(|f| f.kind == kind && f.source)
}

impl Level<'_> {
    /// `BlockState.getFluidState`.
    pub fn fluid_state(&self, state: BlockStateId) -> Option<Fluid> {
        self.registries().blocks.state(state).fluid
    }

    fn fluid_at(&self, pos: BlockPos) -> Option<Fluid> {
        self.fluid_state(self.block(pos))
    }

    /// `FluidState.createLegacyBlock`: the liquid block, or air.
    pub fn fluid_legacy_block(&self, fluid: Option<Fluid>) -> BlockStateId {
        let Some(f) = fluid else { return self.lib.blocks.air };
        // `FlowingFluid.getLegacyLevel`.
        let level = if f.source { 0 } else { 8 - f.amount.min(8) + if f.falling { 8 } else { 0 } };
        let name = match f.kind {
            FluidKind::Water => "minecraft:water",
            FluidKind::Lava => "minecraft:lava",
        };
        self.registries().blocks.parse_state(&format!("{name}[level={level}]")).expect("liquid level states exist")
    }

    /// `FlowingFluid.getTickDelay`: water 5; lava 30, or 10 with fast lava.
    pub fn fluid_tick_delay(&self, state: BlockStateId) -> i32 {
        match self.fluid_state(state).map(|f| f.kind) {
            Some(FluidKind::Lava) => {
                if self.fast_lava { 10 } else { 30 }
            }
            _ => 5,
        }
    }

    fn drop_off(&self, kind: FluidKind) -> u8 {
        match kind {
            FluidKind::Water => 1,
            FluidKind::Lava => {
                if self.fast_lava { 1 } else { 2 }
            }
        }
    }

    fn slope_find_distance(&self, kind: FluidKind) -> u8 {
        match kind {
            FluidKind::Water => 4,
            FluidKind::Lava => {
                if self.fast_lava { 4 } else { 2 }
            }
        }
    }

    fn can_convert_to_source(&self, kind: FluidKind) -> bool {
        match kind {
            FluidKind::Water => self.water_source_conversion,
            FluidKind::Lava => self.lava_source_conversion,
        }
    }

    /// `LiquidBlock.shouldSpreadLiquid`: lava touching water (below or at
    /// a side, in `POSSIBLE_FLOW_DIRECTIONS`) hardens; lava over soul soil
    /// next to blue ice becomes basalt.
    pub(super) fn should_spread_liquid(&mut self, state: BlockStateId, pos: BlockPos) -> bool {
        if self.fluid_state(state).map(|f| f.kind) != Some(FluidKind::Lava) {
            return true;
        }
        let over_soul_soil = self.name(self.block(pos.below())) == "minecraft:soul_soil";
        for direction in [Direction::Down, Direction::South, Direction::North, Direction::East, Direction::West] {
            let neighbour = pos.relative(direction.opposite(), 1);
            if self.fluid_at(neighbour).is_some_and(|f| f.kind == FluidKind::Water) {
                let source = self.fluid_at(pos).is_some_and(|f| f.source);
                let block = if source { "minecraft:obsidian" } else { "minecraft:cobblestone" };
                let block = self.registries().blocks.parse_state(block).expect("exists");
                self.set_block_and_update(pos, block);
                return false;
            }
            if over_soul_soil && self.name(self.block(neighbour)) == "minecraft:blue_ice" {
                let basalt = self.registries().blocks.parse_state("minecraft:basalt").expect("exists");
                self.set_block_and_update(pos, basalt);
                return false;
            }
        }
        true
    }

    /// `FluidState.tick` → `FlowingFluid.tick`.
    pub(super) fn tick_fluid(&mut self, pos: BlockPos, state: BlockStateId) {
        let Some(mut fluid) = self.fluid_state(state) else { return };
        let mut block_state = state;
        let kind = fluid.kind;
        if !fluid.source {
            let new_fluid = self.new_liquid(kind, pos, self.block(pos));
            let delay = self.spread_delay(kind, pos, Some(fluid), new_fluid);
            match new_fluid {
                None => {
                    block_state = self.lib.blocks.air;
                    self.set_block_and_update(pos, block_state);
                    // `spread` of an empty fluid state does nothing.
                    return;
                }
                Some(new) if new != fluid => {
                    fluid = new;
                    block_state = self.fluid_legacy_block(Some(new));
                    self.set_block_and_update(pos, block_state);
                    self.schedule_fluid_state_tick(pos, block_state, delay);
                }
                _ => {}
            }
        }
        self.spread(kind, pos, block_state, fluid);
    }

    /// `FlowingFluid.getSpreadDelay`; lava sometimes waits four times as long
    /// when it rises.
    fn spread_delay(&mut self, kind: FluidKind, pos: BlockPos, old: Option<Fluid>, new: Option<Fluid>) -> i32 {
        let delay = if kind == FluidKind::Lava && self.fast_lava { 10 } else if kind == FluidKind::Lava { 30 } else { 5 };
        if kind != FluidKind::Lava {
            return delay;
        }
        if let (Some(old), Some(new)) = (old, new) {
            if !old.falling && !new.falling && self.fluid_height(new, pos) > self.fluid_height(old, pos) && self.random.next_i32_bound(4) != 0 {
                return delay * 4;
            }
        }
        delay
    }

    /// `FluidState.getHeight`: full when the same fluid is above.
    fn fluid_height(&self, fluid: Fluid, pos: BlockPos) -> f32 {
        if self.fluid_at(pos.above()).is_some_and(|f| f.kind == fluid.kind) {
            1.0
        } else {
            f32::from(fluid.amount) / 9.0
        }
    }

    /// `FlowingFluid.spread`.
    fn spread(&mut self, kind: FluidKind, pos: BlockPos, state: BlockStateId, fluid: Fluid) {
        let below_pos = pos.below();
        let below_state = self.block(below_pos);
        let below_fluid = self.fluid_state(below_state);
        if self.can_maybe_pass_through(kind, pos, state, Direction::Down, below_pos, below_state, below_fluid) {
            let new_below = self.new_liquid(kind, below_pos, below_state);
            if self.can_be_replaced_with(below_fluid, below_pos, new_below, Direction::Down) && self.can_hold_specific_fluid(below_state, new_below) {
                self.spread_to(kind, below_pos, below_state, Direction::Down, new_below);
                if self.source_neighbor_count(kind, pos) >= 3 {
                    self.spread_to_sides(kind, pos, fluid, state);
                }
                return;
            }
        }
        if fluid.source || !self.is_water_hole(kind, pos, state, below_pos, below_state) {
            self.spread_to_sides(kind, pos, fluid, state);
        }
    }

    /// `FlowingFluid.spreadToSides`: the spread's directions in `EnumMap`
    /// (ordinal) order.
    fn spread_to_sides(&mut self, kind: FluidKind, pos: BlockPos, fluid: Fluid, state: BlockStateId) {
        let mut amount = i32::from(fluid.amount) - i32::from(self.drop_off(kind));
        if fluid.falling {
            amount = 7;
        }
        if amount <= 0 {
            return;
        }
        let spreads = self.get_spread(kind, pos, state);
        for direction in [Direction::North, Direction::South, Direction::West, Direction::East] {
            if let Some(&(new_fluid,)) = spreads.get(&direction) {
                let neighbor = pos.relative(direction, 1);
                let neighbor_state = self.block(neighbor);
                self.spread_to(kind, neighbor, neighbor_state, direction, Some(new_fluid));
            }
        }
    }

    /// `FlowingFluid.getNewLiquid` for a fluid kind at a position.
    fn new_liquid(&self, kind: FluidKind, pos: BlockPos, state: BlockStateId) -> Option<Fluid> {
        let mut highest = 0;
        let mut sources = 0;
        for direction in HORIZONTAL {
            let relative = pos.relative(direction, 1);
            let relative_state = self.block(relative);
            if let Some(f) = self.fluid_state(relative_state).filter(|f| f.kind == kind) {
                if self.can_pass_through_wall(direction, state, relative_state) {
                    if f.source {
                        sources += 1;
                    }
                    highest = highest.max(f.amount);
                }
            }
        }
        if sources >= 2 && self.can_convert_to_source(kind) {
            let below = self.block(pos.below());
            if self.registries().blocks.is(below, flags::LEGACY_SOLID) || is_source_of(self.fluid_state(below), kind) {
                return Some(Fluid { kind, amount: 8, source: true, falling: false });
            }
        }
        let above = self.block(pos.above());
        if self.fluid_state(above).is_some_and(|f| f.kind == kind) && self.can_pass_through_wall(Direction::Up, state, above) {
            return Some(Fluid { kind, amount: 8, source: false, falling: true });
        }
        let amount = i32::from(highest) - i32::from(self.drop_off(kind));
        (amount > 0).then_some(Fluid { kind, amount: amount as u8, source: false, falling: false })
    }

    /// `FlowingFluid.canPassThroughWall` over collision shapes.
    fn can_pass_through_wall(&self, direction: Direction, source: BlockStateId, target: BlockStateId) -> bool {
        let blocks = &self.registries().blocks;
        let empty = FaceShape::Empty;
        let target_shape = blocks.collision_shape(target).unwrap_or(&empty);
        if matches!(target_shape, FaceShape::Full) {
            return false;
        }
        let source_shape = blocks.collision_shape(source).unwrap_or(&empty);
        if matches!(source_shape, FaceShape::Full) {
            return false;
        }
        if matches!(source_shape, FaceShape::Empty) && matches!(target_shape, FaceShape::Empty) {
            return true;
        }
        !merged_face_occludes(source_shape, target_shape, direction)
    }

    /// `canHoldAnyFluid`: a `LiquidBlockContainer`, or a block fluids wash away.
    fn can_hold_any_fluid(&self, state: BlockStateId) -> bool {
        is_liquid_container(self, state) || self.registries().block_in_tag(state, self.washed_away)
    }

    /// `canHoldSpecificFluid`: containers take only still water (waterlogging).
    fn can_hold_specific_fluid(&self, state: BlockStateId, fluid: Option<Fluid>) -> bool {
        if !is_liquid_container(self, state) {
            return true;
        }
        waterloggable(self, state) && fluid.is_some_and(|f| f.kind == FluidKind::Water && f.source && !f.falling)
    }

    /// `canMaybePassThrough`.
    #[allow(clippy::too_many_arguments)]
    fn can_maybe_pass_through(
        &self,
        kind: FluidKind,
        _source_pos: BlockPos,
        source: BlockStateId,
        direction: Direction,
        _test_pos: BlockPos,
        test: BlockStateId,
        test_fluid: Option<Fluid>,
    ) -> bool {
        !is_source_of(test_fluid, kind) && self.can_hold_any_fluid(test) && self.can_pass_through_wall(direction, source, test)
    }

    /// `FluidState.canBeReplacedWith`: empty fluids always; water by
    /// anything but water only downward; lava by water when at least
    /// 4/9 high.
    fn can_be_replaced_with(&self, current: Option<Fluid>, pos: BlockPos, new: Option<Fluid>, direction: Direction) -> bool {
        let Some(current) = current else { return true };
        let new_kind = new.map(|f| f.kind);
        match current.kind {
            FluidKind::Water => direction == Direction::Down && new_kind != Some(FluidKind::Water),
            FluidKind::Lava => self.fluid_height(current, pos) >= 0.444_444_45 && new_kind == Some(FluidKind::Water),
        }
    }

    fn source_neighbor_count(&self, kind: FluidKind, pos: BlockPos) -> usize {
        HORIZONTAL.iter().filter(|&&d| is_source_of(self.fluid_at(pos.relative(d, 1)), kind)).count()
    }

    /// `isWaterHole`.
    fn is_water_hole(&self, kind: FluidKind, _top_pos: BlockPos, top: BlockStateId, _bottom_pos: BlockPos, bottom: BlockStateId) -> bool {
        if !self.can_pass_through_wall(Direction::Down, top, bottom) {
            return false;
        }
        if self.fluid_state(bottom).is_some_and(|f| f.kind == kind) {
            return true;
        }
        let flowing = Some(Fluid { kind, amount: 1, source: false, falling: false });
        self.can_hold_any_fluid(bottom) && self.can_hold_specific_fluid(bottom, flowing)
    }

    /// `FlowingFluid.getSpread`: the lowest slope distance wins; ties spread
    /// to all.
    fn get_spread(&self, kind: FluidKind, pos: BlockPos, state: BlockStateId) -> HashMap<Direction, (Fluid,)> {
        let mut lowest = 1000;
        let mut result = HashMap::new();
        let mut holes: HashMap<(i32, i32, i32), bool> = HashMap::new();
        for direction in HORIZONTAL {
            let test_pos = pos.relative(direction, 1);
            let test = self.block(test_pos);
            let test_fluid = self.fluid_state(test);
            if !self.can_maybe_pass_through(kind, pos, state, direction, test_pos, test, test_fluid) {
                continue;
            }
            let new_fluid = self.new_liquid(kind, test_pos, test);
            if !self.can_hold_specific_fluid(test, new_fluid) {
                continue;
            }
            let distance = if self.is_hole_cached(kind, test_pos, &mut holes) {
                0
            } else {
                self.slope_distance(kind, test_pos, 1, direction.opposite(), test, &mut holes)
            };
            if distance < lowest {
                result.clear();
            }
            if distance <= lowest {
                if let Some(new) = new_fluid.filter(|_| self.can_be_replaced_with(test_fluid, test_pos, new_fluid, direction)) {
                    result.insert(direction, (new,));
                }
                lowest = distance;
            }
        }
        result
    }

    fn is_hole_cached(&self, kind: FluidKind, pos: BlockPos, cache: &mut HashMap<(i32, i32, i32), bool>) -> bool {
        *cache.entry((pos.x, pos.y, pos.z)).or_insert_with(|| {
            let below = pos.below();
            self.is_water_hole(kind, pos, self.block(pos), below, self.block(below))
        })
    }

    /// `FlowingFluid.getSlopeDistance`.
    fn slope_distance(&self, kind: FluidKind, pos: BlockPos, pass: u8, from: Direction, state: BlockStateId, holes: &mut HashMap<(i32, i32, i32), bool>) -> i32 {
        let mut lowest = 1000;
        for direction in HORIZONTAL {
            if direction == from {
                continue;
            }
            let test_pos = pos.relative(direction, 1);
            let test = self.block(test_pos);
            let test_fluid = self.fluid_state(test);
            let flowing = Some(Fluid { kind, amount: 1, source: false, falling: false });
            if self.can_maybe_pass_through(kind, pos, state, direction, test_pos, test, test_fluid) && self.can_hold_specific_fluid(test, flowing) {
                if self.is_hole_cached(kind, test_pos, holes) {
                    return i32::from(pass);
                }
                if pass < self.slope_find_distance(kind) {
                    let v = self.slope_distance(kind, test_pos, pass + 1, direction.opposite(), test, holes);
                    if v < lowest {
                        lowest = v;
                    }
                }
            }
        }
        lowest
    }

    /// `FlowingFluid.spreadTo` (with `LavaFluid.spreadTo`'s stone).
    fn spread_to(&mut self, kind: FluidKind, pos: BlockPos, state: BlockStateId, direction: Direction, target: Option<Fluid>) {
        if kind == FluidKind::Lava && direction == Direction::Down && self.fluid_state(state).is_some_and(|f| f.kind == FluidKind::Water) {
            if self.is_a(state, "LiquidBlock") {
                let stone = self.registries().blocks.parse_state("minecraft:stone").expect("exists");
                self.set_block_and_update(pos, stone);
            }
            return;
        }
        if is_liquid_container(self, state) {
            // `SimpleWaterloggedBlock.placeLiquid`.
            if waterloggable(self, state) && self.registries().blocks.property(state, "waterlogged") == Some("false") && target.is_some_and(|f| f.kind == FluidKind::Water && f.source) {
                let logged = self.registries().blocks.with_property(state, "waterlogged", "true").expect("waterlogged");
                self.set_block_and_update(pos, logged);
                self.schedule_fluid_state_tick(pos, logged, 5);
            }
            return;
        }
        // `beforeDestroyingBlock`: water drops the block's loot, lava fizzes.
        if !self.registries().blocks.is_air(state) && target.is_some_and(|f| f.kind == FluidKind::Water) {
            self.drop_resources(state, pos);
        }
        let block = self.fluid_legacy_block(target);
        self.set_block(pos, block, update::ALL, update::LIMIT);
    }
}

/// `LiquidBlockContainer`: waterloggable blocks, kelp and seagrass.
fn is_liquid_container(level: &Level, state: BlockStateId) -> bool {
    waterloggable(level, state) || matches!(level.name(state), "minecraft:kelp" | "minecraft:kelp_plant" | "minecraft:seagrass" | "minecraft:tall_seagrass")
}

/// `SimpleWaterloggedBlock` (blocks with a `waterlogged` property).
fn waterloggable(level: &Level, state: BlockStateId) -> bool {
    level.registries().blocks.property(state, "waterlogged").is_some()
}

/// `Shapes.mergedFaceOccludes` for collision shapes: the faces two shapes
/// present to each other across `direction` together cover the face.
fn merged_face_occludes(shape: &FaceShape, occluder: &FaceShape, direction: Direction) -> bool {
    if matches!(shape, FaceShape::Full) || matches!(occluder, FaceShape::Full) {
        return true;
    }
    let (axis, positive) = match direction {
        Direction::Down => (Axis::Y, false),
        Direction::Up => (Axis::Y, true),
        Direction::North => (Axis::Z, false),
        Direction::South => (Axis::Z, true),
        Direction::West => (Axis::X, false),
        Direction::East => (Axis::X, true),
    };
    let (first, second) = if positive { (shape, occluder) } else { (occluder, shape) };
    let a = match axis {
        Axis::X => 0,
        Axis::Y => 1,
        Axis::Z => 2,
    };
    let (u, v) = match axis {
        Axis::X => (1, 2),
        Axis::Y => (0, 2),
        Axis::Z => (0, 1),
    };
    let mut rects: Vec<[f64; 4]> = Vec::new();
    // The first shape's layer at its maximum, the second's at its minimum.
    if let FaceShape::Boxes(boxes) = first {
        let max = boxes.iter().map(|b| b[a + 3]).fold(f64::MIN, f64::max);
        if (max - 1.0).abs() <= 1e-7 {
            rects.extend(boxes.iter().filter(|b| (b[a + 3] - 1.0).abs() <= 1e-7).map(|b| [b[u], b[v], b[u + 3], b[v + 3]]));
        }
    }
    if let FaceShape::Boxes(boxes) = second {
        let min = boxes.iter().map(|b| b[a]).fold(f64::MAX, f64::min);
        if min.abs() <= 1e-7 {
            rects.extend(boxes.iter().filter(|b| b[a].abs() <= 1e-7).map(|b| [b[u], b[v], b[u + 3], b[v + 3]]));
        }
    }
    covers_unit_square(&rects)
}

fn covers_unit_square(rects: &[[f64; 4]]) -> bool {
    if rects.is_empty() {
        return false;
    }
    let mut us = vec![0.0, 1.0];
    let mut vs = vec![0.0, 1.0];
    for r in rects {
        us.extend([r[0].clamp(0.0, 1.0), r[2].clamp(0.0, 1.0)]);
        vs.extend([r[1].clamp(0.0, 1.0), r[3].clamp(0.0, 1.0)]);
    }
    us.sort_by(f64::total_cmp);
    us.dedup();
    vs.sort_by(f64::total_cmp);
    vs.dedup();
    for i in 0..us.len() - 1 {
        for j in 0..vs.len() - 1 {
            let (cu, cv) = ((us[i] + us[i + 1]) / 2.0, (vs[j] + vs[j + 1]) / 2.0);
            if !rects.iter().any(|r| r[0] <= cu && cu <= r[2] && r[1] <= cv && cv <= r[3]) {
                return false;
            }
        }
    }
    true
}
