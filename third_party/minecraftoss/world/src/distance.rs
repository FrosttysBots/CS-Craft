//! `DistanceManager`'s natural spawn chunk counter (26.3): the chunks within
//! eight of a player, propagated by `ChunkTracker` over
//! `DynamicGraphMinFixedPoint` and kept in a fastutil `Long2ByteOpenHashMap`.
//! The map's iteration order is the order `collectSpawningChunks` lists
//! chunks in before the level random shuffles them, so the map is ported
//! with fastutil 8.5.18's hashing, probing, growth, shrinking and
//! backward-shift removal, and the propagation with its queue order.

use minecraftoss_core::ChunkPos;
use std::collections::HashMap;

/// `ChunkPos.pack`.
pub fn pack(x: i32, z: i32) -> i64 {
    (i64::from(x) & 0xFFFF_FFFF) | ((i64::from(z) & 0xFFFF_FFFF) << 32)
}

/// `ChunkPos.unpack`.
pub fn unpack(key: i64) -> ChunkPos {
    ChunkPos::new(key as i32, (key >> 32) as i32)
}

/// `ChunkPos.INVALID_CHUNK_POS`: the source node of chunk trackers.
const INVALID: i64 = (1_875_066 & 0xFFFF_FFFF) | (1_875_066 << 32);

/// fastutil `HashCommon.mix(long)`.
fn mix(x: i64) -> i64 {
    let h = x.wrapping_mul(-7_046_029_254_386_353_131);
    let h = h ^ ((h as u64) >> 32) as i64;
    h ^ ((h as u64) >> 16) as i64
}

/// fastutil `HashCommon.arraySize`.
fn array_size(expected: usize, f: f32) -> usize {
    let s = f64::from(expected as f32 / f).ceil() as u64;
    let power = if s <= 1 { 1 } else { 1u64 << (64 - (s - 1).leading_zeros()) };
    power.max(2) as usize
}

/// fastutil `HashCommon.maxFill`.
fn max_fill(n: usize, f: f32) -> usize {
    (f64::from(n as f32 * f).ceil() as usize).min(n - 1)
}

/// fastutil `Long2ByteOpenHashMap`: open addressing with linear probing,
/// the null key kept apart, and iteration from the last slot down.
#[derive(Clone, Debug)]
pub struct LongByteMap {
    key: Vec<i64>,
    value: Vec<i8>,
    n: usize,
    mask: usize,
    max_fill: usize,
    min_n: usize,
    size: usize,
    contains_null: bool,
    f: f32,
    default: i8,
}

impl LongByteMap {
    pub fn new(expected: usize, f: f32, default: i8) -> Self {
        let n = array_size(expected, f);
        Self { key: vec![0; n + 1], value: vec![0; n + 1], n, mask: n - 1, max_fill: max_fill(n, f), min_n: n, size: 0, contains_null: false, f, default }
    }

    fn slot(&self, k: i64) -> usize {
        ((mix(k) as i32) & self.mask as i32) as usize
    }

    /// The key's slot, or where it would go.
    fn find(&self, k: i64) -> Result<usize, usize> {
        if k == 0 {
            return if self.contains_null { Ok(self.n) } else { Err(self.n) };
        }
        let mut pos = self.slot(k);
        loop {
            match self.key[pos] {
                0 => return Err(pos),
                c if c == k => return Ok(pos),
                _ => pos = (pos + 1) & self.mask,
            }
        }
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub fn get(&self, k: i64) -> i8 {
        self.find(k).map_or(self.default, |pos| self.value[pos])
    }

    pub fn put(&mut self, k: i64, v: i8) -> i8 {
        match self.find(k) {
            Ok(pos) => std::mem::replace(&mut self.value[pos], v),
            Err(pos) => {
                if pos == self.n {
                    self.contains_null = true;
                }
                self.key[pos] = k;
                self.value[pos] = v;
                let before = self.size;
                self.size += 1;
                if before >= self.max_fill {
                    self.rehash(array_size(self.size + 1, self.f));
                }
                self.default
            }
        }
    }

    pub fn remove(&mut self, k: i64) -> i8 {
        let old = match self.find(k) {
            Err(_) => return self.default,
            Ok(pos) if pos == self.n => {
                self.contains_null = false;
                self.size -= 1;
                self.value[self.n]
            }
            Ok(pos) => {
                let old = self.value[pos];
                self.size -= 1;
                self.shift_keys(pos);
                old
            }
        };
        if self.n > self.min_n && self.size < self.max_fill / 4 && self.n > 16 {
            self.rehash(self.n / 2);
        }
        old
    }

    /// Backward-shift deletion (`shiftKeys`).
    fn shift_keys(&mut self, mut pos: usize) {
        loop {
            let last = pos;
            pos = (last + 1) & self.mask;
            let curr = loop {
                let curr = self.key[pos];
                if curr == 0 {
                    self.key[last] = 0;
                    return;
                }
                let slot = self.slot(curr);
                let moves = if last <= pos { last >= slot || slot > pos } else { last >= slot && slot > pos };
                if moves {
                    break curr;
                }
                pos = (pos + 1) & self.mask;
            };
            self.key[last] = curr;
            self.value[last] = self.value[pos];
        }
    }

    fn rehash(&mut self, new_n: usize) {
        let mask = new_n - 1;
        let mut key = vec![0i64; new_n + 1];
        let mut value = vec![0i8; new_n + 1];
        let mut i = self.n;
        let real = self.size - usize::from(self.contains_null);
        for _ in 0..real {
            loop {
                i -= 1;
                if self.key[i] != 0 {
                    break;
                }
            }
            let mut pos = ((mix(self.key[i]) as i32) & mask as i32) as usize;
            while key[pos] != 0 {
                pos = (pos + 1) & mask;
            }
            key[pos] = self.key[i];
            value[pos] = self.value[i];
        }
        value[new_n] = self.value[self.n];
        self.n = new_n;
        self.mask = mask;
        self.max_fill = max_fill(new_n, self.f);
        self.key = key;
        self.value = value;
    }

    /// Keys in iteration order: the null key first, then slots downwards.
    pub fn keys(&self) -> Vec<i64> {
        let mut out = Vec::with_capacity(self.size);
        if self.contains_null {
            out.push(0);
        }
        out.extend((0..self.n).rev().map(|pos| self.key[pos]).filter(|&k| k != 0));
        out
    }
}

/// fastutil `LongLinkedOpenHashSet`'s order: insertion order, with removal
/// anywhere and removal of the first.
#[derive(Clone, Debug, Default)]
struct LinkedSet {
    links: HashMap<i64, (Option<i64>, Option<i64>)>,
    head: Option<i64>,
    tail: Option<i64>,
}

impl LinkedSet {
    fn add(&mut self, k: i64) {
        if self.links.contains_key(&k) {
            return;
        }
        self.links.insert(k, (self.tail, None));
        match self.tail {
            Some(t) => self.links.get_mut(&t).expect("linked").1 = Some(k),
            None => self.head = Some(k),
        }
        self.tail = Some(k);
    }

    fn remove(&mut self, k: i64) {
        let Some((prev, next)) = self.links.remove(&k) else { return };
        match prev {
            Some(p) => self.links.get_mut(&p).expect("linked").1 = next,
            None => self.head = next,
        }
        match next {
            Some(n) => self.links.get_mut(&n).expect("linked").0 = prev,
            None => self.tail = prev,
        }
    }

    fn remove_first(&mut self) -> i64 {
        let first = self.head.expect("not empty");
        self.remove(first);
        first
    }

    fn is_empty(&self) -> bool {
        self.head.is_none()
    }
}

/// `LeveledPriorityQueue`.
#[derive(Clone, Debug)]
struct LevelQueue {
    queues: Vec<LinkedSet>,
    first: i32,
    level_count: i32,
}

impl LevelQueue {
    fn new(level_count: i32) -> Self {
        Self { queues: vec![LinkedSet::default(); level_count as usize], first: level_count, level_count }
    }

    fn remove_first(&mut self) -> i64 {
        let queue = &mut self.queues[self.first as usize];
        let result = queue.remove_first();
        if queue.is_empty() {
            self.check_first(self.level_count);
        }
        result
    }

    fn is_empty(&self) -> bool {
        self.first >= self.level_count
    }

    fn dequeue(&mut self, node: i64, key: i32, upper: i32) {
        let queue = &mut self.queues[key as usize];
        queue.remove(node);
        if queue.is_empty() && self.first == key {
            self.check_first(upper);
        }
    }

    fn enqueue(&mut self, node: i64, key: i32) {
        self.queues[key as usize].add(node);
        if self.first > key {
            self.first = key;
        }
    }

    fn check_first(&mut self, upper: i32) {
        let old = self.first;
        self.first = upper;
        for i in old + 1..upper {
            if !self.queues[i as usize].is_empty() {
                self.first = i;
                break;
            }
        }
    }
}

/// `DistanceManager.FixedPlayerDistanceChunkTracker`: each chunk's
/// Chebyshev distance to the nearest chunk holding a player, for chunks
/// within `max_distance`.
#[derive(Clone, Debug)]
pub struct PlayerChunkCounter {
    max_distance: i32,
    level_count: i32,
    queue: LevelQueue,
    /// `computedLevels` (absent reads as 255).
    computed: HashMap<i64, u8>,
    /// `chunks`: level by chunk, `max_distance + 2` when absent.
    chunks: LongByteMap,
    /// `playersPerChunk` sizes.
    players: HashMap<i64, usize>,
}

impl PlayerChunkCounter {
    /// The natural spawn counter has `max_distance` 8.
    pub fn new(max_distance: i32) -> Self {
        let level_count = max_distance + 2;
        Self {
            max_distance,
            level_count,
            queue: LevelQueue::new(level_count),
            computed: HashMap::new(),
            chunks: LongByteMap::new(16, 0.75, level_count as i8),
            players: HashMap::new(),
        }
    }

    /// `DistanceManager.addPlayer`'s part for this tracker.
    pub fn add_player(&mut self, chunk: ChunkPos) {
        let key = pack(chunk.x, chunk.z);
        *self.players.entry(key).or_insert(0) += 1;
        self.update(key, 0, true);
    }

    /// `DistanceManager.removePlayer`'s part for this tracker.
    pub fn remove_player(&mut self, chunk: ChunkPos) {
        let key = pack(chunk.x, chunk.z);
        let Some(count) = self.players.get_mut(&key) else { return };
        *count -= 1;
        if *count == 0 {
            self.players.remove(&key);
            self.update(key, i32::MAX, false);
        }
    }

    /// `getNaturalSpawnChunkCount`.
    pub fn chunk_count(&mut self) -> usize {
        self.run_all_updates();
        self.chunks.len()
    }

    /// `getSpawnCandidateChunks`: the chunks in the map's order.
    pub fn candidates(&mut self) -> Vec<ChunkPos> {
        self.run_all_updates();
        self.chunks.keys().into_iter().map(unpack).collect()
    }

    /// The chunk's distance to a player's chunk (`max_distance + 2` beyond).
    pub fn distance(&mut self, chunk: ChunkPos) -> i32 {
        self.run_all_updates();
        self.level(pack(chunk.x, chunk.z))
    }

    fn level(&self, node: i64) -> i32 {
        i32::from(self.chunks.get(node))
    }

    fn set_level(&mut self, node: i64, level: i32) {
        if level > self.max_distance {
            self.chunks.remove(node);
        } else {
            self.chunks.put(node, level as i8);
        }
    }

    fn level_from_source(&self, to: i64) -> i32 {
        if self.players.get(&to).is_some_and(|&c| c > 0) { 0 } else { i32::MAX }
    }

    fn level_from_neighbor(&self, from: i64, to: i64, from_level: i32) -> i32 {
        if from == INVALID { self.level_from_source(to) } else { from_level + 1 }
    }

    fn priority(&self, level: i32, computed: i32) -> i32 {
        level.min(computed).min(self.level_count - 1)
    }

    fn computed_of(&self, node: i64) -> i32 {
        self.computed.get(&node).map_or(255, |&v| i32::from(v))
    }

    /// `ChunkTracker.getComputedLevel`.
    fn computed_level(&self, node: i64, known_parent: i64, known_level: i32) -> i32 {
        let mut computed = known_level;
        let pos = unpack(node);
        for dx in -1..=1 {
            for dz in -1..=1 {
                let mut neighbor = pack(pos.x + dx, pos.z + dz);
                if neighbor == node {
                    neighbor = INVALID;
                }
                if neighbor != known_parent {
                    let cost = self.level_from_neighbor(neighbor, node, self.level(neighbor));
                    if computed > cost {
                        computed = cost;
                    }
                    if computed == 0 {
                        return computed;
                    }
                }
            }
        }
        computed
    }

    /// `ChunkTracker.update`: an edge from the source.
    fn update(&mut self, node: i64, level_from: i32, only_decreased: bool) {
        let (level_to, computed) = (self.level(node), self.computed_of(node));
        self.check_edge(INVALID, node, level_from, level_to, computed, only_decreased);
    }

    fn check_edge(&mut self, from: i64, to: i64, level_from: i32, level_to: i32, old_computed: i32, only_decreased: bool) {
        if to == INVALID {
            return;
        }
        let top = self.level_count - 1;
        let level_from = level_from.clamp(0, top);
        let level_to = level_to.clamp(0, top);
        let was_consistent = old_computed == 255;
        let old_computed = if was_consistent { level_to } else { old_computed };
        let new_computed = if only_decreased { old_computed.min(level_from) } else { self.computed_level(to, from, level_from).clamp(0, top) };
        let old_priority = self.priority(level_to, old_computed);
        if level_to != new_computed {
            let new_priority = self.priority(level_to, new_computed);
            if old_priority != new_priority && !was_consistent {
                self.queue.dequeue(to, old_priority, new_priority);
            }
            self.queue.enqueue(to, new_priority);
            self.computed.insert(to, new_computed as u8);
        } else if !was_consistent {
            self.queue.dequeue(to, old_priority, self.level_count);
            self.computed.remove(&to);
        }
    }

    fn check_neighbor(&mut self, from: i64, to: i64, level: i32, only_decreased: bool) {
        let stored = self.computed_of(to);
        let level_from = self.level_from_neighbor(from, to, level).clamp(0, self.level_count - 1);
        if only_decreased {
            let level_to = self.level(to);
            self.check_edge(from, to, level_from, level_to, stored, true);
        } else {
            let was_consistent = stored == 255;
            let old_computed = if was_consistent { self.level(to).clamp(0, self.level_count - 1) } else { stored };
            if level_from == old_computed {
                let level_to = if was_consistent { old_computed } else { self.level(to) };
                self.check_edge(from, to, self.level_count - 1, level_to, stored, false);
            }
        }
    }

    fn check_neighbors_after_update(&mut self, node: i64, level: i32, only_decrease: bool) {
        if !only_decrease || level < self.level_count - 2 {
            let pos = unpack(node);
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let neighbor = pack(pos.x + dx, pos.z + dz);
                    if neighbor != node {
                        self.check_neighbor(node, neighbor, level, only_decrease);
                    }
                }
            }
        }
    }

    /// `runAllUpdates`.
    pub fn run_all_updates(&mut self) {
        while !self.queue.is_empty() {
            let node = self.queue.remove_first();
            let level = self.level(node).clamp(0, self.level_count - 1);
            let computed = self.computed.remove(&node).map_or(255, i32::from);
            if computed < level {
                self.set_level(node, computed);
                self.check_neighbors_after_update(node, computed, true);
            } else if computed > level {
                let top = self.level_count - 1;
                self.set_level(node, top);
                if computed != top {
                    let priority = self.priority(top, computed);
                    self.queue.enqueue(node, priority);
                    self.computed.insert(node, computed as u8);
                }
                self.check_neighbors_after_update(node, level, false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_counts_the_chunks_within_eight() {
        let mut counter = PlayerChunkCounter::new(8);
        counter.add_player(ChunkPos::new(8, 8));
        assert_eq!(counter.chunk_count(), 289);
        assert_eq!(counter.distance(ChunkPos::new(16, 3)), 8);
        assert_eq!(counter.distance(ChunkPos::new(17, 8)), 10);
        counter.remove_player(ChunkPos::new(8, 8));
        assert_eq!(counter.chunk_count(), 0);
    }

    #[test]
    fn the_map_keeps_fastutil_order_through_growth_and_removal() {
        let mut map = LongByteMap::new(16, 0.75, -1);
        for i in 0..100 {
            map.put(pack(i % 13, i / 13), i as i8);
        }
        for i in (0..100).step_by(3) {
            map.remove(pack(i % 13, i / 13));
        }
        let keys = map.keys();
        assert_eq!(keys.len(), map.len());
        assert!(keys.iter().all(|&k| map.get(k) != -1));
    }
}
