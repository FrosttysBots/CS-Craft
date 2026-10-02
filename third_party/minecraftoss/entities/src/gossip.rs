//! `GossipContainer` (26.3): what a villager has heard about other entities,
//! by UUID, as gossip of five types, each with a weight towards reputation,
//! a cap, a daily decay and a loss when passed on. Villagers pass gossip on
//! as they meet (`transferFrom`, drawing from the listener's random), which
//! picks entries by their weighted value in the container's iteration
//! order, so that order is kept as vanilla's: a `HashMap<UUID, _>`
//! (`UUID.hashCode`, `HashMap.hash`, head insertion by `computeIfAbsent`,
//! resizes splitting buckets in order). Within one target vanilla iterates
//! an `Object2IntOpenHashMap` keyed by the enum, whose identity hash codes
//! change from run to run; here a target's entries follow the enum order.
use minecraftoss_player::rng::LegacyRandom;
use std::collections::HashMap;

/// `GossipType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GossipType {
    MajorNegative,
    MinorNegative,
    MinorPositive,
    MajorPositive,
    Trading,
}

impl GossipType {
    pub const ALL: [Self; 5] = [Self::MajorNegative, Self::MinorNegative, Self::MinorPositive, Self::MajorPositive, Self::Trading];

    pub fn id(self) -> &'static str {
        match self {
            Self::MajorNegative => "major_negative",
            Self::MinorNegative => "minor_negative",
            Self::MinorPositive => "minor_positive",
            Self::MajorPositive => "major_positive",
            Self::Trading => "trading",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    /// Its weight towards reputation.
    pub fn weight(self) -> i32 {
        match self {
            Self::MajorNegative => -5,
            Self::MinorNegative => -1,
            Self::MinorPositive | Self::Trading => 1,
            Self::MajorPositive => 5,
        }
    }

    pub fn max(self) -> i32 {
        match self {
            Self::MajorNegative => 100,
            Self::MinorNegative => 200,
            Self::MinorPositive | Self::Trading => 25,
            Self::MajorPositive => 20,
        }
    }

    fn decay_per_day(self) -> i32 {
        match self {
            Self::MajorNegative => 10,
            Self::MinorNegative => 20,
            Self::MinorPositive => 1,
            Self::MajorPositive => 0,
            Self::Trading => 2,
        }
    }

    fn decay_per_transfer(self) -> i32 {
        match self {
            Self::MajorNegative => 10,
            Self::MinorNegative | Self::MajorPositive | Self::Trading => 20,
            Self::MinorPositive => 5,
        }
    }
}

/// `GossipContainer.DISCARD_THRESHOLD`: gossip below it is forgotten.
const DISCARD_THRESHOLD: i32 = 2;

/// `UUID.hashCode` spread by `HashMap.hash`.
fn java_hash(uuid: u128) -> u32 {
    let hilo = (uuid >> 64) as u64 ^ uuid as u64;
    let h = (hilo >> 32) as u32 ^ hilo as u32;
    h ^ (h >> 16)
}

/// A `HashMap<UUID, _>`'s table as far as its iteration order goes: each
/// bucket's keys in link order, the size and the resize threshold.
#[derive(Clone, Debug, Default, PartialEq)]
struct Table {
    buckets: Vec<Vec<u128>>,
    size: usize,
    threshold: usize,
}

impl Table {
    /// `HashMap.resize`: 16 buckets at first, then twice as many, each
    /// bucket split in order between its old index and that plus the old
    /// size.
    fn resize(&mut self) {
        let old = self.buckets.len();
        if old == 0 {
            self.buckets = vec![Vec::new(); 16];
            self.threshold = 12;
            return;
        }
        let new = old * 2;
        let mut buckets = vec![Vec::new(); new];
        for list in self.buckets.drain(..) {
            for key in list {
                buckets[java_hash(key) as usize & (new - 1)].push(key);
            }
        }
        self.buckets = buckets;
        self.threshold *= 2;
    }

    /// `computeIfAbsent`'s table work: a resize first when the map has
    /// outgrown its threshold (or has no table), then a new key goes to the
    /// head of its bucket. Whether the key was new.
    fn compute_if_absent(&mut self, key: u128) -> bool {
        if self.size > self.threshold || self.buckets.is_empty() {
            self.resize();
        }
        let index = java_hash(key) as usize & (self.buckets.len() - 1);
        if self.buckets[index].contains(&key) {
            return false;
        }
        self.buckets[index].insert(0, key);
        self.size += 1;
        true
    }

    fn remove(&mut self, key: u128) {
        if self.buckets.is_empty() {
            return;
        }
        let index = java_hash(key) as usize & (self.buckets.len() - 1);
        if let Some(at) = self.buckets[index].iter().position(|&k| k == key) {
            self.buckets[index].remove(at);
            self.size -= 1;
        }
    }

    /// `HashMap.clear`: the table keeps its size.
    fn clear(&mut self) {
        self.buckets.iter_mut().for_each(Vec::clear);
        self.size = 0;
    }

    fn keys(&self) -> impl Iterator<Item = u128> + '_ {
        self.buckets.iter().flatten().copied()
    }
}

/// `GossipContainer`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gossips {
    table: Table,
    /// Each target's entries, in type order.
    entries: HashMap<u128, Vec<(GossipType, i32)>>,
}

impl Gossips {
    /// `new GossipContainer(entries)` (the codec): each entry put in list
    /// order.
    pub fn from_entries(list: &[(u128, GossipType, i32)]) -> Self {
        let mut out = Self::default();
        for &(target, kind, value) in list {
            let entries = out.get_or_create(target);
            match entries.iter_mut().find(|(t, _)| *t == kind) {
                Some(entry) => entry.1 = value,
                None => insert_sorted(entries, kind, value),
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.table.size == 0
    }

    fn get_or_create(&mut self, target: u128) -> &mut Vec<(GossipType, i32)> {
        if self.table.compute_if_absent(target) {
            self.entries.insert(target, Vec::new());
        }
        self.entries.get_mut(&target).expect("a target in the table has entries")
    }

    fn remove_target(&mut self, target: u128) {
        self.table.remove(target);
        self.entries.remove(&target);
    }

    /// `unpack`: every entry, targets in the map's order.
    pub fn unpack(&self) -> Vec<(u128, GossipType, i32)> {
        self.table.keys().flat_map(|target| self.entries[&target].iter().map(move |&(kind, value)| (target, kind, value))).collect()
    }

    /// `decay`: a day's decay for every entry; spent entries and targets
    /// with none left go.
    pub fn decay(&mut self) {
        let targets: Vec<u128> = self.table.keys().collect();
        for target in targets {
            let entries = self.entries.get_mut(&target).expect("a target in the table has entries");
            entries.retain_mut(|(kind, value)| {
                *value -= kind.decay_per_day();
                *value >= DISCARD_THRESHOLD
            });
            if entries.is_empty() {
                self.remove_target(target);
            }
        }
    }

    /// `selectGossipsForTransfer`: `max_count` draws, each an entry picked
    /// by its share of the summed absolute weighted values; the distinct
    /// entries picked (vanilla collects them in an identity set, whose
    /// order does not matter but for new targets sharing a bucket).
    fn select_for_transfer(&self, random: &mut LegacyRandom, max_count: i32) -> Vec<(u128, GossipType, i32)> {
        let entries = self.unpack();
        if entries.is_empty() {
            return Vec::new();
        }
        let mut ranges = Vec::with_capacity(entries.len());
        let mut end = 0;
        for &(_, kind, value) in &entries {
            end += (value * kind.weight()).abs();
            ranges.push(end - 1);
        }
        let mut picked = vec![false; entries.len()];
        for _ in 0..max_count {
            let choice = random.next_int(end as u32) as i32;
            // `Arrays.binarySearch`: the entry whose range holds the draw.
            let index = ranges.partition_point(|&r| r < choice);
            picked[index] = true;
        }
        entries.into_iter().zip(picked).filter_map(|(entry, p)| p.then_some(entry)).collect()
    }

    /// `transferFrom`: gossip heard from `source`, each picked entry less
    /// its type's loss in the telling, kept if still worth remembering and
    /// larger than what was known. How many entries were picked.
    pub fn transfer_from(&mut self, source: &Gossips, random: &mut LegacyRandom, max_count: i32) -> i32 {
        let picked = source.select_for_transfer(random, max_count);
        for &(target, kind, value) in &picked {
            let decayed = value - kind.decay_per_transfer();
            if decayed >= DISCARD_THRESHOLD {
                let entries = self.get_or_create(target);
                match entries.iter_mut().find(|(t, _)| *t == kind) {
                    Some(entry) => entry.1 = entry.1.max(decayed),
                    None => insert_sorted(entries, kind, decayed),
                }
            }
        }
        picked.len() as i32
    }

    /// `getReputation(target, all types)`: the weighted sum of what it
    /// heard about the target.
    pub fn reputation(&self, target: u128) -> i32 {
        self.entries.get(&target).map_or(0, |entries| entries.iter().map(|&(kind, value)| value * kind.weight()).sum())
    }

    /// `add`: more gossip of a type (less for a negative amount), capped
    /// at the type's maximum; forgotten below the discard threshold.
    pub fn add(&mut self, target: u128, kind: GossipType, amount: i32) {
        let entries = self.get_or_create(target);
        match entries.iter_mut().find(|(t, _)| *t == kind) {
            // `mergeValuesForAddition`: past the cap, the cap (or more if
            // it already had more).
            Some(entry) => {
                let sum = entry.1 + amount;
                entry.1 = if sum > kind.max() { kind.max().max(entry.1) } else { sum };
            }
            None => insert_sorted(entries, kind, amount),
        }
        // `makeSureValueIsntTooLowOrTooHigh`.
        let position = entries.iter().position(|(t, _)| *t == kind).expect("just merged");
        if entries[position].1 > kind.max() {
            entries[position].1 = kind.max();
        }
        if entries[position].1 < DISCARD_THRESHOLD {
            entries.remove(position);
        }
        if entries.is_empty() {
            self.remove_target(target);
        }
    }

    /// `clear` then `putAll`: how `readAdditionalSaveData` takes saved
    /// gossip (targets in the saved container's order).
    pub fn replace_with(&mut self, other: &Gossips) {
        self.table.clear();
        self.entries.clear();
        for target in other.table.keys() {
            let theirs = other.entries[&target].clone();
            let entries = self.get_or_create(target);
            for (kind, value) in theirs {
                match entries.iter_mut().find(|(t, _)| *t == kind) {
                    Some(entry) => entry.1 = value,
                    None => insert_sorted(entries, kind, value),
                }
            }
        }
    }
}

fn insert_sorted(entries: &mut Vec<(GossipType, i32)>, kind: GossipType, value: i32) {
    let at = entries.partition_point(|(t, _)| *t < kind);
    entries.insert(at, (kind, value));
}

/// A UUID from its four ints as NBT stores it (`UUIDUtil.uuidFromIntArray`).
pub fn uuid_from_ints(ints: [i32; 4]) -> u128 {
    ints.iter().fold(0u128, |acc, &i| (acc << 32) | u128::from(i as u32))
}

/// A UUID's four ints (`UUIDUtil.uuidToIntArray`).
pub fn uuid_to_ints(uuid: u128) -> [i32; 4] {
    [(uuid >> 96) as u32 as i32, (uuid >> 64) as u32 as i32, (uuid >> 32) as u32 as i32, uuid as u32 as i32]
}

/// A UUID written as `UUID.toString` writes it.
pub fn uuid_string(uuid: u128) -> String {
    let hex = format!("{uuid:032x}");
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

/// A UUID from its string form.
pub fn parse_uuid(text: &str) -> Option<u128> {
    let hex: String = text.chars().filter(|&c| c != '-').collect();
    (hex.len() == 32).then(|| u128::from_str_radix(&hex, 16).ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(i: u32) -> u128 {
        u128::from(i)
    }

    #[test]
    fn keys_iterate_as_a_java_hash_map() {
        // UUID(0, i): hash i ^ (i >>> 16), so small ones sit in bucket i.
        let mut gossips = Gossips::default();
        for i in [5, 3, 21, 1] {
            gossips.add(uuid(i), GossipType::Trading, 2);
        }
        // Buckets 1, 3, 5 (21 joined 5 at its head), 16 buckets.
        let order: Vec<u128> = gossips.unpack().into_iter().map(|(t, _, _)| t).collect();
        assert_eq!(order, vec![1, 3, 21, 5]);
        // A thirteenth key passes the threshold; the fourteenth insertion
        // resizes first, splitting 5 and 21 in order.
        for i in 100..109 {
            gossips.add(uuid(i), GossipType::Trading, 2);
        }
        assert_eq!(gossips.table.buckets.len(), 16);
        gossips.add(uuid(7), GossipType::Trading, 2);
        assert_eq!(gossips.table.buckets.len(), 32);
        let order: Vec<u128> = gossips.unpack().into_iter().map(|(t, _, _)| t).filter(|&t| t < 100).collect();
        assert_eq!(order, vec![1, 3, 5, 7, 21]);
    }

    #[test]
    fn adds_cap_and_forget() {
        let mut gossips = Gossips::default();
        gossips.add(uuid(1), GossipType::MinorNegative, 25);
        gossips.add(uuid(1), GossipType::MinorNegative, 190);
        assert_eq!(gossips.reputation(uuid(1)), -200);
        gossips.add(uuid(1), GossipType::Trading, 30);
        assert_eq!(gossips.reputation(uuid(1)), -175, "trading caps at 25");
        gossips.add(uuid(2), GossipType::Trading, 1);
        assert_eq!(gossips.unpack().len(), 2, "below two is forgotten");
        for _ in 0..13 {
            gossips.decay();
        }
        assert!(gossips.is_empty(), "a day at a time it fades");
    }

    #[test]
    fn transfers_pick_by_weight() {
        let mut source = Gossips::default();
        source.add(uuid(1), GossipType::MajorNegative, 25);
        let mut listener = Gossips::default();
        let mut random = LegacyRandom::new(1);
        assert_eq!(listener.transfer_from(&source, &mut random, 10), 1);
        assert_eq!(listener.unpack(), vec![(uuid(1), GossipType::MajorNegative, 15)]);
        // Nothing to tell draws nothing.
        let before = random.clone();
        assert_eq!(listener.transfer_from(&Gossips::default(), &mut random, 10), 0);
        assert_eq!(random.next_int(1000), before.clone().next_int(1000));
    }

    #[test]
    fn uuids_round_trip() {
        let uuid = 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210u128;
        assert_eq!(uuid_from_ints(uuid_to_ints(uuid)), uuid);
        assert_eq!(uuid_string(uuid), "01234567-89ab-cdef-fedc-ba9876543210");
        assert_eq!(parse_uuid(&uuid_string(uuid)), Some(uuid));
    }
}
