//! Scheduled ticks (26.3 `LevelTicks`, `LevelChunkTicks`, `ScheduledTick`).
//!
//! A tick runs at `trigger = game time at scheduling + delay`, ordered by
//! trigger, then priority, then a level-wide sub-tick counter
//! (`ScheduledTick.INTRA_TICK_DRAIN_ORDER`). Since the counter is unique,
//! one heap over all chunks gives vanilla's order. A (type, position) pair
//! is scheduled at most once while it waits; a tick already collected for
//! this game tick does not block scheduling it again.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet, VecDeque};

/// What a tick runs: a block or a fluid type, by registry index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TickType {
    Block(u16),
    Fluid(FluidType),
}

/// `Fluids`: the four flowing kinds a tick can target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FluidType {
    Water,
    FlowingWater,
    Lava,
    FlowingLava,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    trigger: i64,
    priority: i32,
    sub_tick: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScheduledTick {
    key: Key,
    pub pos: (i32, i32, i32),
    pub kind: TickType,
}

#[derive(Default)]
pub struct LevelTicks {
    pending: BinaryHeap<Reverse<ScheduledTick>>,
    waiting: HashSet<(TickType, (i32, i32, i32))>,
    /// `toRunThisTick`: collected for the running game tick, not yet run.
    to_run: VecDeque<ScheduledTick>,
}

impl LevelTicks {
    /// `LevelChunkTicks.schedule`: ignored while the same type waits at the
    /// same position.
    pub fn schedule(&mut self, kind: TickType, pos: (i32, i32, i32), trigger: i64, priority: i32, sub_tick: i64) {
        if self.waiting.insert((kind, pos)) {
            self.pending.push(Reverse(ScheduledTick { key: Key { trigger, priority, sub_tick }, pos, kind }));
        }
    }

    /// `hasScheduledTick` for ticks still waiting.
    pub fn has_scheduled(&self, kind: TickType, pos: (i32, i32, i32)) -> bool {
        self.waiting.contains(&(kind, pos))
    }

    /// `LevelTicks.collectTicks`: the ticks due at `game_time`, at most
    /// `max`, in drain order. They stop blocking rescheduling at once.
    pub fn collect(&mut self, game_time: i64, max: usize) -> Vec<ScheduledTick> {
        let mut out = Vec::new();
        while out.len() < max {
            match self.pending.peek() {
                Some(Reverse(tick)) if tick.key.trigger <= game_time => {
                    let Reverse(tick) = self.pending.pop().expect("peeked");
                    self.waiting.remove(&(tick.kind, tick.pos));
                    out.push(tick);
                }
                _ => break,
            }
        }
        out
    }

    /// `LevelTicks.tick` step one: collect the due ticks to run one by one
    /// with `next`.
    pub fn begin(&mut self, game_time: i64, max: usize) {
        self.to_run = self.collect(game_time, max).into();
    }

    /// The next collected tick of this game tick.
    pub fn next(&mut self) -> Option<ScheduledTick> {
        self.to_run.pop_front()
    }

    /// `willTickThisTick`: collected this game tick and not run yet.
    pub fn will_tick_this_tick(&self, kind: TickType, pos: (i32, i32, i32)) -> bool {
        self.to_run.iter().any(|t| t.kind == kind && t.pos == pos)
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Every waiting tick, in drain order.
    pub fn waiting(&self) -> Vec<ScheduledTick> {
        let mut ticks: Vec<ScheduledTick> = self.pending.iter().map(|Reverse(t)| *t).collect();
        ticks.sort();
        ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_order_and_dedupe() {
        let mut ticks = LevelTicks::default();
        let water = TickType::Fluid(FluidType::Water);
        ticks.schedule(water, (0, 0, 0), 5, 0, 0);
        ticks.schedule(water, (0, 0, 0), 3, 0, 1); // waiting already: ignored
        ticks.schedule(TickType::Block(1), (1, 0, 0), 5, -1, 2);
        ticks.schedule(TickType::Block(1), (2, 0, 0), 4, 0, 3);
        assert!(ticks.collect(3, 10).is_empty());
        let due: Vec<_> = ticks.collect(5, 10).into_iter().map(|t| t.pos).collect();
        assert_eq!(due, vec![(2, 0, 0), (1, 0, 0), (0, 0, 0)]);
        ticks.schedule(water, (0, 0, 0), 6, 0, 4);
        assert!(ticks.has_scheduled(water, (0, 0, 0)));
    }
}
