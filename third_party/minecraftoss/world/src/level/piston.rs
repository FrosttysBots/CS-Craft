//! Pistons on the server level (26.3 `PistonBaseBlock`,
//! `PistonStructureResolver`, `MovingPistonBlock`, `PistonMovingBlockEntity`
//! and `PistonHeadBlock`), with the block events and block entity ticking
//! they run through (`ServerLevel.blockEvent` / `runBlockEvents`,
//! `Level.tickBlockEntities` with `LevelChunk`'s rebindable tickers).
//!
//! Entities are not moved yet, and blocks a piston breaks drop nothing.

use super::redstone::Kind;
use super::{update, Level};
use minecraftoss_core::block::PushReaction;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::{BlockId, BlockPos, BlockStateId};
use minecraftoss_generator::feature::template::update_from_neighbour_shapes;
use minecraftoss_generator::feature::Ctx;
use std::collections::HashMap;

/// `PistonMovingBlockEntity`.
#[derive(Clone, Copy, Debug)]
pub struct MovingBlock {
    pub moved: BlockStateId,
    pub direction: Direction,
    pub extending: bool,
    pub source: bool,
    pub progress: f32,
    pub progress_o: f32,
    pub last_ticked: i64,
}

impl MovingBlock {
    fn push_direction(&self) -> Direction {
        if self.extending { self.direction } else { self.direction.opposite() }
    }
}

/// `BlockEventData`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockEvent {
    pub pos: BlockPos,
    pub block: BlockId,
    pub a: i32,
    pub b: i32,
}

/// Moving blocks with vanilla's ticker list: a position keeps its place in
/// the list while its block entity is replaced (`RebindableTickingBlockEntityWrapper`).
#[derive(Default)]
pub struct MovingBlocks {
    pub entities: HashMap<BlockPos, MovingBlock>,
    /// Ticker list: position and whether it was unbound (`NULL_TICKER`).
    tickers: Vec<(BlockPos, bool)>,
    pending: Vec<(BlockPos, bool)>,
    /// `tickersInLevel`: whether a live wrapper exists for a position, and
    /// where (list or pending, index).
    live: HashMap<BlockPos, (bool, usize)>,
    ticking: bool,
}

impl MovingBlocks {
    /// `LevelChunk.addAndRegisterBlockEntity` for a moving block.
    fn set(&mut self, pos: BlockPos, entity: MovingBlock) {
        self.entities.insert(pos, entity);
        if self.live.contains_key(&pos) {
            // Rebound in place.
            return;
        }
        if self.ticking {
            self.pending.push((pos, false));
            self.live.insert(pos, (true, self.pending.len() - 1));
        } else {
            self.tickers.push((pos, false));
            self.live.insert(pos, (false, self.tickers.len() - 1));
        }
    }

    /// `LevelChunk.updateBlockEntityTicker` for another ticking block
    /// entity (a hopper): a new wrapper unless one is bound here.
    pub fn register(&mut self, pos: BlockPos) {
        if self.live.contains_key(&pos) {
            return;
        }
        if self.ticking {
            self.pending.push((pos, false));
            self.live.insert(pos, (true, self.pending.len() - 1));
        } else {
            self.tickers.push((pos, false));
            self.live.insert(pos, (false, self.tickers.len() - 1));
        }
    }

    /// `LevelChunk.removeBlockEntity` and `removeBlockEntityTicker`.
    pub fn remove(&mut self, pos: BlockPos) {
        self.entities.remove(&pos);
        if let Some((pending, index)) = self.live.remove(&pos) {
            if pending {
                self.pending[index].1 = true;
            } else {
                self.tickers[index].1 = true;
            }
        }
    }
}

/// `Direction.get3DDataValue` is `Direction::index`; `from3DDataValue`.
fn from_3d(value: i32) -> Direction {
    Direction::ALL[(value & 7).rem_euclid(6) as usize]
}

/// Iteration order of a vanilla `HashMap`/`HashSet` of block positions
/// that never held more than `max_size` entries, given the surviving keys
/// in insertion order: by bucket, then insertion order.
pub(super) fn java_hash_order(keys: &[BlockPos], max_size: usize) -> Vec<BlockPos> {
    let mut capacity = 16usize;
    while max_size * 4 > capacity * 3 {
        capacity *= 2;
    }
    let bucket = |p: &BlockPos| {
        // `Vec3i.hashCode`, then `HashMap.hash`.
        let h = (p.y.wrapping_add(p.z.wrapping_mul(31))).wrapping_mul(31).wrapping_add(p.x);
        ((h ^ ((h as u32) >> 16) as i32) as u32 as usize) & (capacity - 1)
    };
    let mut out = keys.to_vec();
    out.sort_by_key(bucket);
    out
}

/// `PistonStructureResolver`.
struct Resolver {
    piston: BlockPos,
    extending: bool,
    start: BlockPos,
    push: Direction,
    piston_direction: Direction,
    to_push: Vec<BlockPos>,
    to_destroy: Vec<BlockPos>,
}

const MAX_PUSH_DEPTH: usize = 12;

impl Level<'_> {
    fn is_block(&self, state: BlockStateId, name: &str) -> bool {
        self.name(state) == name
    }

    fn is_piston_base(&self, state: BlockStateId) -> bool {
        matches!(self.redstone_kind(state), Some(Kind::Piston { .. }))
    }

    fn push_reaction(&self, state: BlockStateId) -> PushReaction {
        self.registries().blocks.state(state).push_reaction
    }

    fn is_sticky_block(&self, state: BlockStateId) -> bool {
        self.is_block(state, "minecraft:slime_block") || self.is_block(state, "minecraft:honey_block")
    }

    fn can_stick(&self, a: BlockStateId, b: BlockStateId) -> bool {
        let (a_honey, a_slime) = (self.is_block(a, "minecraft:honey_block"), self.is_block(a, "minecraft:slime_block"));
        let (b_honey, b_slime) = (self.is_block(b, "minecraft:honey_block"), self.is_block(b, "minecraft:slime_block"));
        if a_honey && b_slime || a_slime && b_honey {
            return false;
        }
        self.is_sticky_block(a) || self.is_sticky_block(b)
    }

    /// `PistonBaseBlock.isPushable`.
    fn is_pushable(&self, state: BlockStateId, pos: BlockPos, direction: Direction, allow_destroyable: bool, connection: Direction) -> bool {
        let (min_y, max_y) = (self.min_y, self.min_y + self.height - 1);
        if pos.y < min_y || pos.y > max_y {
            return false;
        }
        let blocks = &self.registries().blocks;
        if blocks.is_air(state) {
            return true;
        }
        if direction == Direction::Down && pos.y == min_y || direction == Direction::Up && pos.y == max_y {
            return false;
        }
        if self.is_piston_base(state) {
            if self.prop_is(state, "extended") {
                return false;
            }
        } else {
            if blocks.state(state).destroy_speed == -1.0 {
                return false;
            }
            match self.push_reaction(state) {
                PushReaction::Immoveable => return false,
                PushReaction::Popped => return allow_destroyable,
                PushReaction::Push => return direction == connection,
                PushReaction::PushPull => {}
            }
        }
        !blocks.is(state, minecraftoss_core::block::flags::HAS_BLOCK_ENTITY)
    }

    fn prop_is(&self, state: BlockStateId, name: &str) -> bool {
        self.registries().blocks.property(state, name) == Some("true")
    }

    fn state_facing(&self, state: BlockStateId) -> Direction {
        self.registries().blocks.property(state, "facing").and_then(Direction::from_name).unwrap_or(Direction::North)
    }

    fn resolver(&self, piston: BlockPos, direction: Direction, extending: bool) -> Resolver {
        let (push, start) = if extending { (direction, piston.relative(direction, 1)) } else { (direction.opposite(), piston.relative(direction, 2)) };
        Resolver { piston, extending, start, push, piston_direction: direction, to_push: Vec::new(), to_destroy: Vec::new() }
    }

    /// `PistonStructureResolver.resolve`.
    fn resolve(&self, r: &mut Resolver) -> bool {
        r.to_push.clear();
        r.to_destroy.clear();
        let next = self.block(r.start);
        if !self.is_pushable(next, r.start, r.push, false, r.piston_direction) {
            if r.extending && self.push_reaction(next) == PushReaction::Popped {
                r.to_destroy.push(r.start);
                return true;
            }
            return false;
        }
        if !self.add_block_line(r, r.start, r.push) {
            return false;
        }
        let mut i = 0;
        while i < r.to_push.len() {
            let pos = r.to_push[i];
            if self.is_sticky_block(self.block(pos)) && !self.add_branching_blocks(r, pos) {
                return false;
            }
            i += 1;
        }
        true
    }

    fn add_block_line(&self, r: &mut Resolver, start: BlockPos, direction: Direction) -> bool {
        let mut next = self.block(start);
        if self.registries().blocks.is_air(next) {
            return true;
        }
        if !self.is_pushable(next, start, r.push, false, direction) || start == r.piston || r.to_push.contains(&start) {
            return true;
        }
        let mut count = 1;
        if count + r.to_push.len() > MAX_PUSH_DEPTH {
            return false;
        }
        while self.is_sticky_block(next) {
            let pos = start.relative(r.push.opposite(), count as i32);
            let previous = next;
            next = self.block(pos);
            if self.registries().blocks.is_air(next)
                || !self.can_stick(previous, next)
                || !self.is_pushable(next, pos, r.push, false, r.push.opposite())
                || pos == r.piston
            {
                break;
            }
            count += 1;
            if count + r.to_push.len() > MAX_PUSH_DEPTH {
                return false;
            }
        }
        let mut added = 0;
        for i in (0..count).rev() {
            r.to_push.push(start.relative(r.push.opposite(), i as i32));
            added += 1;
        }
        let mut i = 1;
        loop {
            let pos = start.relative(r.push, i);
            if let Some(collision) = r.to_push.iter().position(|&p| p == pos) {
                // `reorderListAtCollision`.
                let len = r.to_push.len();
                let mut reordered = r.to_push[..collision].to_vec();
                reordered.extend_from_slice(&r.to_push[len - added..]);
                reordered.extend_from_slice(&r.to_push[collision..len - added]);
                r.to_push = reordered;
                for j in 0..=collision + added {
                    let p = r.to_push[j];
                    if self.is_sticky_block(self.block(p)) && !self.add_branching_blocks(r, p) {
                        return false;
                    }
                }
                return true;
            }
            next = self.block(pos);
            if self.registries().blocks.is_air(next) {
                return true;
            }
            if !self.is_pushable(next, pos, r.push, true, r.push) || pos == r.piston {
                return false;
            }
            if self.push_reaction(next) == PushReaction::Popped {
                r.to_destroy.push(pos);
                return true;
            }
            if r.to_push.len() >= MAX_PUSH_DEPTH {
                return false;
            }
            r.to_push.push(pos);
            added += 1;
            i += 1;
        }
    }

    fn add_branching_blocks(&self, r: &mut Resolver, from: BlockPos) -> bool {
        let from_state = self.block(from);
        for direction in Direction::ALL {
            if direction.axis() != r.push.axis() {
                let neighbour = from.relative(direction, 1);
                if self.can_stick(self.block(neighbour), from_state) && !self.add_block_line(r, neighbour, direction) {
                    return false;
                }
            }
        }
        true
    }

    // ---- piston base ------------------------------------------------------------

    /// `PistonBaseBlock.getNeighborSignal` (quasi-connectivity included).
    fn piston_neighbor_signal(&self, pos: BlockPos, push: Direction) -> bool {
        for direction in Direction::ALL {
            if direction != push && self.signal(pos.relative(direction, 1), direction) > 0 {
                return true;
            }
        }
        if self.signal(pos, Direction::Down) > 0 {
            return true;
        }
        let above = pos.above();
        Direction::ALL.into_iter().any(|d| d != Direction::Down && self.signal(above.relative(d, 1), d) > 0)
    }

    /// `PistonBaseBlock.checkIfExtend`.
    pub(super) fn piston_check_if_extend(&mut self, pos: BlockPos, state: BlockStateId) {
        let direction = self.state_facing(state);
        let extend = self.piston_neighbor_signal(pos, direction);
        let extended = self.prop_is(state, "extended");
        let block = self.block_id(state);
        if extend && !extended {
            let mut r = self.resolver(pos, direction, true);
            if self.resolve(&mut r) {
                self.block_event(pos, block, 0, direction.index() as i32);
            }
        } else if !extend && extended {
            let pushed = pos.relative(direction, 2);
            let pushed_state = self.block(pushed);
            let mut event = 1;
            if self.redstone_kind(pushed_state) == Some(Kind::MovingPiston) && self.state_facing(pushed_state) == direction {
                if let Some(entity) = self.moving.entities.get(&pushed) {
                    if entity.extending && (entity.progress_o < 0.5 || self.game_time == entity.last_ticked || self.handling_tick) {
                        event = 2;
                    }
                }
            }
            self.block_event(pos, block, event, direction.index() as i32);
        }
    }

    /// `ServerLevel.blockEvent`: queued once per identical event.
    pub(super) fn block_event(&mut self, pos: BlockPos, block: BlockId, a: i32, b: i32) {
        let event = BlockEvent { pos, block, a, b };
        if !self.block_events.contains(&event) {
            self.block_events.push_back(event);
        }
    }

    /// `ServerLevel.runBlockEvents`.
    pub(super) fn run_block_events(&mut self) {
        while let Some(event) = self.block_events.pop_front() {
            let state = self.block(event.pos);
            if self.block_id(state) == event.block {
                self.trigger_event(state, event.pos, event.a, event.b);
            }
        }
    }

    /// `BlockBehaviour.triggerEvent`.
    fn trigger_event(&mut self, state: BlockStateId, pos: BlockPos, a: i32, b: i32) {
        match self.redstone_kind(state) {
            Some(Kind::Piston { sticky }) => self.piston_trigger_event(state, pos, a, b, sticky),
            Some(Kind::NoteBlock) => self.note_block_event(state),
            _ => {}
        }
    }

    /// `PistonBaseBlock.triggerEvent`.
    fn piston_trigger_event(&mut self, state: BlockStateId, pos: BlockPos, a: i32, b: i32, sticky: bool) {
        let direction = self.state_facing(state);
        let extended_state = self.with(state, "extended", "true");
        let extend = self.piston_neighbor_signal(pos, direction);
        if extend && (a == 1 || a == 2) {
            self.set_block(pos, extended_state, update::CLIENTS, update::LIMIT);
            return;
        }
        if !extend && a == 0 {
            return;
        }
        if a == 0 {
            if !self.move_blocks(pos, direction, true, sticky) {
                return;
            }
            self.set_block(pos, extended_state, update::ALL | update::MOVE_BY_PISTON, update::LIMIT);
            // The extend sound's pitch draws from the level random.
            self.random.next_f32();
        } else if a == 1 || a == 2 {
            let arm = pos.relative(direction, 1);
            if self.moving.entities.contains_key(&arm) {
                self.moving_final_tick(arm);
            }
            let kind = if sticky { "sticky" } else { "normal" };
            let moving_state = self.with(self.with(self.moving_piston_default(), "facing", direction.name()), "type", kind);
            self.set_block(pos, moving_state, update::INVISIBLE | update::KNOWN_SHAPE | update::SKIP_BLOCK_ENTITY_SIDEEFFECTS, update::LIMIT);
            let base = self.with(self.with(self.registries().blocks.block(self.block_id(state)).default_state(), "facing", from_3d(b).name()), "extended", "false");
            self.set_moving(pos, MovingBlock { moved: base, direction, extending: false, source: true, progress: 0.0, progress_o: 0.0, last_ticked: 0 });
            let moving_block = self.block_id(moving_state);
            self.update_neighbors_at(pos, moving_block);
            self.update_neighbour_shapes(moving_state, pos, update::CLIENTS, update::LIMIT);
            if sticky {
                let two = pos.relative(direction, 2);
                let moving = self.block(two);
                let mut piston_piece = false;
                if self.redstone_kind(moving) == Some(Kind::MovingPiston) {
                    if let Some(entity) = self.moving.entities.get(&two).copied() {
                        if entity.direction == direction && entity.extending {
                            self.moving_final_tick(two);
                            piston_piece = true;
                        }
                    }
                }
                if !piston_piece {
                    let pullable = self.push_reaction(moving) == PushReaction::PushPull || self.is_piston_base(moving);
                    if a != 1
                        || self.registries().blocks.is_air(moving)
                        || !self.is_pushable(moving, two, direction.opposite(), false, direction)
                        || !pullable
                    {
                        self.remove_block(arm, false);
                    } else {
                        self.move_blocks(pos, direction, false, sticky);
                    }
                }
            } else {
                self.remove_block(arm, false);
            }
            // The contract sound's pitch.
            self.random.next_f32();
        }
    }

    fn moving_piston_default(&self) -> BlockStateId {
        let blocks = &self.registries().blocks;
        blocks.block(blocks.block_by_name("minecraft:moving_piston").expect("vanilla block")).default_state()
    }

    fn set_moving(&mut self, pos: BlockPos, entity: MovingBlock) {
        // `LevelChunk.setBlockEntity` needs the moving piston there.
        if self.redstone_kind(self.block(pos)) == Some(Kind::MovingPiston) {
            self.moving.set(pos, entity);
        }
    }

    /// `PistonBaseBlock.moveBlocks`.
    fn move_blocks(&mut self, piston: BlockPos, direction: Direction, extending: bool, sticky: bool) -> bool {
        let arm = piston.relative(direction, 1);
        if !extending && self.is_block(self.block(arm), "minecraft:piston_head") {
            self.set_block(arm, BlockStateId::AIR, update::INVISIBLE | update::KNOWN_SHAPE | update::SKIP_BLOCK_ENTITY_SIDEEFFECTS, update::LIMIT);
        }
        let mut r = self.resolver(piston, direction, extending);
        if !self.resolve(&mut r) {
            return false;
        }
        let mut delete_after_move: Vec<(BlockPos, BlockStateId)> = Vec::new();
        let mut shapes = Vec::new();
        for &pos in &r.to_push {
            let state = self.block(pos);
            shapes.push(state);
            delete_after_move.push((pos, state));
        }
        let max_size = delete_after_move.len();
        let mut to_update = Vec::new();
        let push_direction = if extending { direction } else { direction.opposite() };
        for &pos in r.to_destroy.iter().rev() {
            let state = self.block(pos);
            self.drop_resources(state, pos);
            self.set_block(pos, BlockStateId::AIR, update::CLIENTS | update::KNOWN_SHAPE, update::LIMIT);
            to_update.push(state);
        }
        let facing_moving = self.with(self.moving_piston_default(), "facing", direction.name());
        for i in (0..r.to_push.len()).rev() {
            let pos = r.to_push[i];
            let state = self.block(pos);
            let target = pos.relative(push_direction, 1);
            delete_after_move.retain(|(p, _)| *p != target);
            self.set_block(target, facing_moving, update::INVISIBLE | update::MOVE_BY_PISTON | update::SKIP_BLOCK_ENTITY_SIDEEFFECTS, update::LIMIT);
            self.set_moving(target, MovingBlock { moved: shapes[i], direction, extending, source: false, progress: 0.0, progress_o: 0.0, last_ticked: 0 });
            to_update.push(state);
        }
        if extending {
            let kind = if sticky { "sticky" } else { "normal" };
            let blocks = &self.registries().blocks;
            let head_default = blocks.block(blocks.block_by_name("minecraft:piston_head").expect("vanilla block")).default_state();
            let head = self.with(self.with(head_default, "facing", direction.name()), "type", kind);
            let moving_state = self.with(facing_moving, "type", kind);
            delete_after_move.retain(|(p, _)| *p != arm);
            self.set_block(arm, moving_state, update::INVISIBLE | update::MOVE_BY_PISTON | update::SKIP_BLOCK_ENTITY_SIDEEFFECTS, update::LIMIT);
            self.set_moving(arm, MovingBlock { moved: head, direction, extending: true, source: true, progress: 0.0, progress_o: 0.0, last_ticked: 0 });
        }
        let keys: Vec<BlockPos> = delete_after_move.iter().map(|(p, _)| *p).collect();
        let order = java_hash_order(&keys, max_size);
        let old_states: HashMap<BlockPos, BlockStateId> = delete_after_move.into_iter().collect();
        for &pos in &order {
            self.set_block(pos, BlockStateId::AIR, update::CLIENTS | update::KNOWN_SHAPE | update::MOVE_BY_PISTON, update::LIMIT);
        }
        for &pos in &order {
            let old = old_states[&pos];
            self.update_indirect_neighbour_shapes(old, pos, update::CLIENTS, update::LIMIT);
            self.update_neighbour_shapes(BlockStateId::AIR, pos, update::CLIENTS, update::LIMIT);
            self.update_indirect_neighbour_shapes(BlockStateId::AIR, pos, update::CLIENTS, update::LIMIT);
        }
        let mut index = 0;
        for &pos in r.to_destroy.iter().rev() {
            let state = to_update[index];
            index += 1;
            self.affect_neighbors_after_removal(state, pos, false);
            self.update_indirect_neighbour_shapes(state, pos, update::CLIENTS, update::LIMIT);
            let block = self.block_id(state);
            self.update_neighbors_at(pos, block);
        }
        for i in (0..r.to_push.len()).rev() {
            let block = self.block_id(to_update[index]);
            index += 1;
            self.update_neighbors_at(r.to_push[i], block);
        }
        if extending {
            let blocks = &self.registries().blocks;
            let head = blocks.block_by_name("minecraft:piston_head").expect("vanilla block");
            self.update_neighbors_at(arm, head);
        }
        true
    }

    // ---- moving blocks -------------------------------------------------------------

    /// `PistonMovingBlockEntity.finalTick`.
    pub(super) fn moving_final_tick(&mut self, pos: BlockPos) {
        let Some(entity) = self.moving.entities.get(&pos).copied() else { return };
        if entity.progress_o >= 1.0 {
            return;
        }
        self.moving.remove(pos);
        if self.redstone_kind(self.block(pos)) == Some(Kind::MovingPiston) {
            let new_state = if entity.source {
                BlockStateId::AIR
            } else {
                let lib = self.lib;
                update_from_neighbour_shapes(&mut Ctx { lib, region: self }, entity.moved, pos)
            };
            self.set_block_and_update(pos, new_state);
            let block = self.block_id(new_state);
            self.add_and_run(super::Update::Simple { pos, block });
        }
    }

    /// `PistonMovingBlockEntity.tick`.
    fn moving_tick(&mut self, pos: BlockPos) {
        let Some(entity) = self.moving.entities.get_mut(&pos) else { return };
        entity.last_ticked = self.game_time;
        entity.progress_o = entity.progress;
        if entity.progress_o >= 1.0 {
            let entity = *entity;
            self.moving.remove(pos);
            if self.redstone_kind(self.block(pos)) != Some(Kind::MovingPiston) {
                return;
            }
            let lib = self.lib;
            let mut new_state = update_from_neighbour_shapes(&mut Ctx { lib, region: self }, entity.moved, pos);
            if self.registries().blocks.is_air(new_state) {
                self.set_block(
                    pos,
                    entity.moved,
                    update::INVISIBLE | update::KNOWN_SHAPE | update::MOVE_BY_PISTON | update::SKIP_BLOCK_ENTITY_SIDEEFFECTS,
                    update::LIMIT,
                );
                self.update_or_destroy(entity.moved, new_state, pos, update::ALL, update::LIMIT);
            } else {
                if self.registries().blocks.property(new_state, "waterlogged") == Some("true") {
                    new_state = self.with(new_state, "waterlogged", "false");
                }
                self.set_block(pos, new_state, update::ALL | update::MOVE_BY_PISTON, update::LIMIT);
                let block = self.block_id(new_state);
                self.add_and_run(super::Update::Simple { pos, block });
            }
            let _ = entity.push_direction();
        } else {
            // Entity pushing is not simulated yet.
            entity.progress = (entity.progress + 0.5).min(1.0);
        }
    }

    /// `Level.tickBlockEntities` for moving blocks.
    pub(super) fn tick_block_entities(&mut self) {
        self.moving.ticking = true;
        let pending = std::mem::take(&mut self.moving.pending);
        let offset = self.moving.tickers.len();
        for (i, (pos, removed)) in pending.into_iter().enumerate() {
            if !removed {
                self.moving.live.insert(pos, (false, offset + i));
            }
            self.moving.tickers.push((pos, removed));
        }
        let mut i = 0;
        while i < self.moving.tickers.len() {
            let (pos, removed) = self.moving.tickers[i];
            if !removed {
                let state = self.block(pos);
                if self.redstone_kind(state) == Some(Kind::MovingPiston) {
                    self.moving_tick(pos);
                } else if self.is_hopper(state) {
                    self.hopper_tick(pos);
                } else if self.redstone_kind(state) == Some(Kind::DaylightDetector) {
                    self.daylight_tick(pos);
                }
            }
            i += 1;
        }
        // Pruning removed tickers keeps the live ones' order.
        let mut kept = Vec::new();
        for (pos, removed) in std::mem::take(&mut self.moving.tickers) {
            if !removed {
                self.moving.live.insert(pos, (false, kept.len()));
                kept.push((pos, removed));
            }
        }
        self.moving.tickers = kept;
        self.moving.ticking = false;
    }

    /// `PistonHeadBlock` neighbour change: passed on to its base.
    pub(super) fn piston_head_neighbor_changed(&mut self, state: BlockStateId, pos: BlockPos, source: BlockId) {
        if self.can_survive_state(state, pos) {
            let base = pos.relative(self.state_facing(state).opposite(), 1);
            self.add_and_run(super::Update::Simple { pos: base, block: source });
        }
    }

    /// `PistonHeadBlock.affectNeighborsAfterRemoval`: the extended base
    /// goes too.
    pub(super) fn piston_head_after_removal(&mut self, state: BlockStateId, pos: BlockPos) {
        let base_pos = pos.relative(self.state_facing(state).opposite(), 1);
        let base = self.block(base_pos);
        let sticky = self.registries().blocks.property(state, "type") == Some("sticky");
        let fitting = self.is_block(base, if sticky { "minecraft:sticky_piston" } else { "minecraft:piston" })
            && self.prop_is(base, "extended")
            && self.state_facing(base) == self.state_facing(state);
        if fitting {
            self.destroy_block_drops(base_pos, true, update::LIMIT);
        }
    }
}
