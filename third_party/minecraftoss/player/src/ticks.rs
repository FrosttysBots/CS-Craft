//! Ordered block tick queue shared by authored-world fluids and future block
//! behavior. A block/type pair keeps its first scheduled tick until it fires.
use crate::Pos;
use std::{
    collections::{BTreeMap, HashMap},
    hash::Hash,
};

#[derive(Clone)]
pub struct ScheduledTicks<K: Copy + Eq + Hash> {
    sequence: u64,
    events: BTreeMap<(u64, i8, u64), (Pos, K)>,
    scheduled: HashMap<(Pos, K), (u64, i8, u64)>,
}

impl<K: Copy + Eq + Hash> Default for ScheduledTicks<K> {
    fn default() -> Self {
        Self {
            sequence: 0,
            events: BTreeMap::new(),
            scheduled: HashMap::new(),
        }
    }
}

impl<K: Copy + Eq + Hash> ScheduledTicks<K> {
    pub fn schedule(&mut self, pos: Pos, kind: K, due: u64) {
        self.schedule_with_priority(pos, kind, due, 0);
    }

    /// Lower numeric priorities execute before higher ones at the same due
    /// tick, then insertion order decides ties, matching TickPriority.
    pub fn schedule_with_priority(&mut self, pos: Pos, kind: K, due: u64, priority: i8) {
        if self.scheduled.contains_key(&(pos, kind)) {
            return;
        }
        self.sequence = self.sequence.wrapping_add(1);
        let key = (due, priority, self.sequence);
        self.scheduled.insert((pos, kind), key);
        self.events.insert(key, (pos, kind));
    }

    pub fn has_due(&self, now: u64) -> bool {
        self.events
            .first_key_value()
            .is_some_and(|(&(due, _, _), _)| due <= now)
    }

    pub fn pop_due(&mut self, now: u64) -> Option<(Pos, K)> {
        while let Some((&(due, priority, sequence), &(pos, kind))) = self.events.first_key_value() {
            if due > now {
                return None;
            }
            let key = (due, priority, sequence);
            self.events.remove(&key);
            if self.scheduled.get(&(pos, kind)) == Some(&key) {
                self.scheduled.remove(&(pos, kind));
                return Some((pos, kind));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_pending_tick_wins_and_ties_retain_insertion_order() {
        let mut ticks = ScheduledTicks::<u8>::default();
        ticks.schedule((0, 0, 0), 1, 10);
        ticks.schedule((1, 0, 0), 1, 5);
        ticks.schedule((0, 0, 0), 1, 3);
        ticks.schedule((2, 0, 0), 1, 3);
        assert_eq!(ticks.pop_due(2), None);
        assert_eq!(ticks.pop_due(3), Some(((2, 0, 0), 1)));
        assert_eq!(ticks.pop_due(5), Some(((1, 0, 0), 1)));
        assert_eq!(ticks.pop_due(10), Some(((0, 0, 0), 1)));
        assert_eq!(ticks.pop_due(10), None);
    }

    #[test]
    fn priority_precedes_insertion_order_for_same_due_tick() {
        let mut ticks = ScheduledTicks::<u8>::default();
        ticks.schedule_with_priority((1, 0, 0), 1, 4, 0);
        ticks.schedule_with_priority((2, 0, 0), 1, 4, -1);
        ticks.schedule_with_priority((3, 0, 0), 1, 4, -1);
        assert_eq!(ticks.pop_due(4), Some(((2, 0, 0), 1)));
        assert_eq!(ticks.pop_due(4), Some(((3, 0, 0), 1)));
        assert_eq!(ticks.pop_due(4), Some(((1, 0, 0), 1)));
    }
}
