//! Points of interest, from pinned Java 26.3 `PoiTypes`, `PoiType`,
//! `PoiManager`, `PoiSection` and `PoiRecord`: the blocks villagers (and
//! bees, portals, compasses, lightning) look for, kept by 16-block section
//! with their free tickets.
//!
//! Queries walk the chunks of a square around the centre (x fastest), each
//! chunk's sections upwards, each section's types, and each type's records
//! in its `HashSet`'s order ([`JavaPosSet`]). Vanilla keys a section's
//! types on `Holder` identity hashes, an order that differs between runs;
//! here types follow the registry. The distance to a village
//! (`sectionsToVillage`) is `PoiManager.DistanceTracker`'s settled level
//! ([`VillageTracker`], a `SectionTracker` over the 26 neighbours): 0 in a
//! section holding an occupied village record, one more per section away,
//! stored up to 6 and 7 beyond. As in vanilla, a section whose village is
//! gone keeps the top level 6 rather than 7.
use minecraftoss_player::{Block, Pos};
use std::collections::HashMap;

/// `PoiTypes`, in registry order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PoiType {
    Armorer,
    Butcher,
    Cartographer,
    Cleric,
    Farmer,
    Fisherman,
    Fletcher,
    Leatherworker,
    Librarian,
    Mason,
    Shepherd,
    Toolsmith,
    Weaponsmith,
    Home,
    Meeting,
    Beehive,
    BeeNest,
    NetherPortal,
    Lodestone,
    TestInstance,
    LightningRod,
}

const BED_COLORS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black",
];

impl PoiType {
    pub const ALL: [PoiType; 21] = [
        Self::Armorer,
        Self::Butcher,
        Self::Cartographer,
        Self::Cleric,
        Self::Farmer,
        Self::Fisherman,
        Self::Fletcher,
        Self::Leatherworker,
        Self::Librarian,
        Self::Mason,
        Self::Shepherd,
        Self::Toolsmith,
        Self::Weaponsmith,
        Self::Home,
        Self::Meeting,
        Self::Beehive,
        Self::BeeNest,
        Self::NetherPortal,
        Self::Lodestone,
        Self::TestInstance,
        Self::LightningRod,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Armorer => "minecraft:armorer",
            Self::Butcher => "minecraft:butcher",
            Self::Cartographer => "minecraft:cartographer",
            Self::Cleric => "minecraft:cleric",
            Self::Farmer => "minecraft:farmer",
            Self::Fisherman => "minecraft:fisherman",
            Self::Fletcher => "minecraft:fletcher",
            Self::Leatherworker => "minecraft:leatherworker",
            Self::Librarian => "minecraft:librarian",
            Self::Mason => "minecraft:mason",
            Self::Shepherd => "minecraft:shepherd",
            Self::Toolsmith => "minecraft:toolsmith",
            Self::Weaponsmith => "minecraft:weaponsmith",
            Self::Home => "minecraft:home",
            Self::Meeting => "minecraft:meeting",
            Self::Beehive => "minecraft:beehive",
            Self::BeeNest => "minecraft:bee_nest",
            Self::NetherPortal => "minecraft:nether_portal",
            Self::Lodestone => "minecraft:lodestone",
            Self::TestInstance => "minecraft:test_instance",
            Self::LightningRod => "minecraft:lightning_rod",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    /// `PoiType.maxTickets`: how many may claim it.
    pub fn max_tickets(self) -> i32 {
        match self {
            Self::Meeting => 32,
            Self::Beehive | Self::BeeNest | Self::NetherPortal | Self::Lodestone | Self::TestInstance | Self::LightningRod => 0,
            _ => 1,
        }
    }

    /// `PoiType.validRange`: how near counts as there.
    pub fn valid_range(self) -> i32 {
        if self == Self::Meeting {
            6
        } else {
            1
        }
    }

    /// `#minecraft:acquirable_job_site`.
    pub fn acquirable_job_site(self) -> bool {
        self <= Self::Weaponsmith
    }

    /// `#minecraft:village`: the job sites, homes and meeting points.
    pub fn village(self) -> bool {
        self <= Self::Meeting
    }

    /// `PoiTypes.forState`: the type a block state marks, if any (a bed
    /// only by its head).
    pub fn of_block(block: &Block) -> Option<Self> {
        let name = block.id.strip_prefix("minecraft:")?;
        Some(match name {
            "blast_furnace" => Self::Armorer,
            "smoker" => Self::Butcher,
            "cartography_table" => Self::Cartographer,
            "brewing_stand" => Self::Cleric,
            "composter" => Self::Farmer,
            "barrel" => Self::Fisherman,
            "fletching_table" => Self::Fletcher,
            "cauldron" | "lava_cauldron" | "water_cauldron" | "powder_snow_cauldron" => Self::Leatherworker,
            "lectern" => Self::Librarian,
            "stonecutter" => Self::Mason,
            "loom" => Self::Shepherd,
            "smithing_table" => Self::Toolsmith,
            "grindstone" => Self::Weaponsmith,
            "bell" => Self::Meeting,
            "beehive" => Self::Beehive,
            "bee_nest" => Self::BeeNest,
            "nether_portal" => Self::NetherPortal,
            "lodestone" => Self::Lodestone,
            "test_instance_block" => Self::TestInstance,
            _ if name.ends_with("lightning_rod") => Self::LightningRod,
            _ if name.strip_suffix("_bed").is_some_and(|color| BED_COLORS.contains(&color)) => {
                if block.property("part") == Some("head") {
                    Self::Home
                } else {
                    return None;
                }
            }
            _ => return None,
        })
    }

    /// Whether a block by this name can mark a point of interest (for
    /// watching block changes cheaply).
    pub fn maybe_block(id: &str) -> bool {
        let block = Block::new(id).with("part", "head");
        Self::of_block(&block).is_some()
    }
}

/// `PoiManager.Occupancy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Occupancy {
    HasSpace,
    IsOccupied,
    Any,
}

/// A `PoiRecord`: where, what, and how many tickets are left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoiRecord {
    pub pos: Pos,
    pub kind: PoiType,
    pub free_tickets: i32,
}

impl PoiRecord {
    fn fits(&self, occupancy: Occupancy) -> bool {
        match occupancy {
            Occupancy::HasSpace => self.free_tickets > 0,
            Occupancy::IsOccupied => self.free_tickets != self.kind.max_tickets(),
            Occupancy::Any => true,
        }
    }
}

/// `HashMap.hash(BlockPos)`: `Vec3i.hashCode` spread by its high half.
fn spread((x, y, z): Pos) -> i32 {
    let h = y.wrapping_add(z.wrapping_mul(31)).wrapping_mul(31).wrapping_add(x);
    h ^ ((h as u32) >> 16) as i32
}

/// A `java.util.HashSet<BlockPos>` in the JDK's iteration order: a
/// power-of-two table from 16, grown past three quarters full (or when a
/// chain passes eight while the table is under 64), insertion order within
/// a bucket. Chains long enough to become trees (48 or more elements in
/// one set) are kept as chains.
#[derive(Clone, Debug)]
pub struct JavaPosSet {
    bins: Vec<Vec<Pos>>,
    len: usize,
}

impl Default for JavaPosSet {
    fn default() -> Self {
        Self::with_table(16)
    }
}

impl JavaPosSet {
    /// A set whose table starts at `size` buckets (a power of two).
    pub fn with_table(size: usize) -> Self {
        Self { bins: vec![Vec::new(); size], len: 0 }
    }

    fn bucket(&self, pos: Pos) -> usize {
        spread(pos) as u32 as usize & (self.bins.len() - 1)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn contains(&self, pos: Pos) -> bool {
        self.bins[self.bucket(pos)].contains(&pos)
    }

    /// `HashSet.add`: false when already present.
    pub fn insert(&mut self, pos: Pos) -> bool {
        let b = self.bucket(pos);
        if self.bins[b].contains(&pos) {
            return false;
        }
        self.bins[b].push(pos);
        let chain = self.bins[b].len();
        self.len += 1;
        // `treeifyBin` grows a small table instead.
        if chain > 8 && self.bins.len() < 64 {
            self.resize();
        }
        if self.len > self.bins.len() * 3 / 4 {
            self.resize();
        }
        true
    }

    fn resize(&mut self) {
        let order: Vec<Pos> = self.iter().collect();
        self.bins = vec![Vec::new(); self.bins.len() * 2];
        for pos in order {
            let b = self.bucket(pos);
            self.bins[b].push(pos);
        }
    }

    pub fn remove(&mut self, pos: Pos) -> bool {
        let b = self.bucket(pos);
        let Some(i) = self.bins[b].iter().position(|&p| p == pos) else { return false };
        self.bins[b].remove(i);
        self.len -= 1;
        true
    }

    pub fn iter(&self) -> impl Iterator<Item = Pos> + '_ {
        self.bins.iter().flat_map(|b| b.iter().copied())
    }
}

/// A `PoiSection`: its records, and each type's set of them.
#[derive(Clone, Debug, Default)]
struct PoiSection {
    records: HashMap<Pos, PoiRecord>,
    by_type: Vec<(PoiType, JavaPosSet)>,
}

impl PoiSection {
    fn add(&mut self, pos: Pos, kind: PoiType) -> bool {
        if let Some(old) = self.records.get(&pos) {
            if old.kind == kind {
                return false;
            }
            // Vanilla logs the mismatch and replaces the record, leaving
            // the old one in its type's set.
        }
        self.records.insert(pos, PoiRecord { pos, kind, free_tickets: kind.max_tickets() });
        let at = match self.by_type.binary_search_by_key(&kind, |(k, _)| *k) {
            Ok(i) => i,
            Err(i) => {
                self.by_type.insert(i, (kind, JavaPosSet::default()));
                i
            }
        };
        self.by_type[at].1.insert(pos);
        true
    }

    fn remove(&mut self, pos: Pos) -> bool {
        let Some(record) = self.records.remove(&pos) else { return false };
        if let Some((_, set)) = self.by_type.iter_mut().find(|(k, _)| *k == record.kind) {
            set.remove(pos);
        }
        true
    }

    /// `getRecords`: by type, then by each type's set order.
    fn records<'a>(&'a self, types: &'a dyn Fn(PoiType) -> bool, occupancy: Occupancy) -> impl Iterator<Item = PoiRecord> + 'a {
        self.by_type
            .iter()
            .filter(move |(k, _)| types(*k))
            .flat_map(move |(_, set)| set.iter().filter_map(|pos| self.records.get(&pos).copied()))
            .filter(move |r| r.fits(occupancy))
    }

    fn village_center(&self) -> bool {
        self.records.values().any(|r| r.kind.village() && r.fits(Occupancy::IsOccupied))
    }
}

fn section_of((x, y, z): Pos) -> Pos {
    (x >> 4, y >> 4, z >> 4)
}

/// `SectionPos.asLong`.
fn section_key((x, y, z): Pos) -> i64 {
    ((i64::from(x) & 0x3F_FFFF) << 42) | ((i64::from(z) & 0x3F_FFFF) << 20) | (i64::from(y) & 0xF_FFFF)
}

/// `SectionPos.x/y/z` of a key.
fn section_of_key(key: i64) -> Pos {
    ((key << 0 >> 42) as i32, (key << 44 >> 44) as i32, (key << 22 >> 42) as i32)
}

/// `DynamicGraphMinFixedPoint.SOURCE`.
const SOURCE: i64 = i64::MAX;

/// A `LongLinkedOpenHashSet`: insertion order, removal anywhere.
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

/// `PoiManager.DistanceTracker` (`SectionTracker`, 7 levels): each
/// section's distance in sections to a village centre.
#[derive(Clone, Debug)]
pub struct VillageTracker {
    levels: HashMap<i64, u8>,
    computed: HashMap<i64, u8>,
    queue: LevelQueue,
}

impl Default for VillageTracker {
    fn default() -> Self {
        Self { levels: HashMap::new(), computed: HashMap::new(), queue: LevelQueue::new(Self::LEVELS) }
    }
}

impl VillageTracker {
    const LEVELS: i32 = 7;

    fn level(&self, node: i64) -> i32 {
        self.levels.get(&node).map_or(7, |&l| i32::from(l))
    }

    fn set_level(&mut self, node: i64, level: i32) {
        if level > 6 {
            self.levels.remove(&node);
        } else {
            self.levels.insert(node, level as u8);
        }
    }

    fn computed_of(&self, node: i64) -> i32 {
        self.computed.get(&node).map_or(255, |&v| i32::from(v))
    }

    fn priority(level: i32, computed: i32) -> i32 {
        level.min(computed).min(Self::LEVELS - 1)
    }

    fn from_neighbor(from: i64, to: i64, from_level: i32, center: &dyn Fn(i64) -> bool) -> i32 {
        if from == SOURCE {
            if center(to) {
                0
            } else {
                7
            }
        } else {
            from_level + 1
        }
    }

    /// `SectionTracker.getComputedLevel`.
    fn computed_level(&self, node: i64, known_parent: i64, known_level: i32, center: &dyn Fn(i64) -> bool) -> i32 {
        let mut computed = known_level;
        let (x, y, z) = section_of_key(node);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let mut neighbor = section_key((x + dx, y + dy, z + dz));
                    if neighbor == node {
                        neighbor = SOURCE;
                    }
                    if neighbor != known_parent {
                        let cost = Self::from_neighbor(neighbor, node, self.level(neighbor), center);
                        if computed > cost {
                            computed = cost;
                        }
                        if computed == 0 {
                            return computed;
                        }
                    }
                }
            }
        }
        computed
    }

    /// `SectionTracker.update`: the edge from the source.
    fn update(&mut self, node: i64, level_from: i32, only_decreased: bool, center: &dyn Fn(i64) -> bool) {
        let (level_to, computed) = (self.level(node), self.computed_of(node));
        self.check_edge(SOURCE, node, level_from, level_to, computed, only_decreased, center);
    }

    #[allow(clippy::too_many_arguments)]
    fn check_edge(&mut self, from: i64, to: i64, level_from: i32, level_to: i32, old_computed: i32, only_decreased: bool, center: &dyn Fn(i64) -> bool) {
        if to == SOURCE {
            return;
        }
        let top = Self::LEVELS - 1;
        let level_from = level_from.clamp(0, top);
        let level_to = level_to.clamp(0, top);
        let was_consistent = old_computed == 255;
        let old_computed = if was_consistent { level_to } else { old_computed };
        let new_computed = if only_decreased { old_computed.min(level_from) } else { self.computed_level(to, from, level_from, center).clamp(0, top) };
        let old_priority = Self::priority(level_to, old_computed);
        if level_to != new_computed {
            let new_priority = Self::priority(level_to, new_computed);
            if old_priority != new_priority && !was_consistent {
                self.queue.dequeue(to, old_priority, new_priority);
            }
            self.queue.enqueue(to, new_priority);
            self.computed.insert(to, new_computed as u8);
        } else if !was_consistent {
            self.queue.dequeue(to, old_priority, Self::LEVELS);
            self.computed.remove(&to);
        }
    }

    fn check_neighbor(&mut self, from: i64, to: i64, level: i32, only_decreased: bool, center: &dyn Fn(i64) -> bool) {
        let stored = self.computed_of(to);
        let level_from = Self::from_neighbor(from, to, level, center).clamp(0, Self::LEVELS - 1);
        if only_decreased {
            let level_to = self.level(to);
            self.check_edge(from, to, level_from, level_to, stored, true, center);
        } else {
            let was_consistent = stored == 255;
            let old_computed = if was_consistent { self.level(to).clamp(0, Self::LEVELS - 1) } else { stored };
            if level_from == old_computed {
                let level_to = if was_consistent { old_computed } else { self.level(to) };
                self.check_edge(from, to, Self::LEVELS - 1, level_to, stored, false, center);
            }
        }
    }

    fn check_neighbors_after_update(&mut self, node: i64, level: i32, only_decrease: bool, center: &dyn Fn(i64) -> bool) {
        if !only_decrease || level < Self::LEVELS - 2 {
            let (x, y, z) = section_of_key(node);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let neighbor = section_key((x + dx, y + dy, z + dz));
                        if neighbor != node {
                            self.check_neighbor(node, neighbor, level, only_decrease, center);
                        }
                    }
                }
            }
        }
    }

    /// `runAllUpdates`.
    fn run_all_updates(&mut self, center: &dyn Fn(i64) -> bool) {
        while !self.queue.is_empty() {
            let node = self.queue.remove_first();
            let level = self.level(node).clamp(0, Self::LEVELS - 1);
            let computed = self.computed.remove(&node).map_or(255, i32::from);
            if computed < level {
                self.set_level(node, computed);
                self.check_neighbors_after_update(node, computed, true, center);
            } else if computed > level {
                let top = Self::LEVELS - 1;
                self.set_level(node, top);
                if computed != top {
                    let priority = Self::priority(top, computed);
                    self.queue.enqueue(node, priority);
                    self.computed.insert(node, computed as u8);
                }
                self.check_neighbors_after_update(node, level, false, center);
            }
        }
    }
}

/// `PoiManager` over the loaded sections of one dimension.
#[derive(Clone, Debug, Default)]
pub struct PoiManager {
    sections: HashMap<Pos, PoiSection>,
    tracker: VillageTracker,
    min_section_y: i32,
    max_section_y: i32,
}

impl PoiManager {
    /// A manager for a dimension whose sections run from `min_section_y`
    /// to `max_section_y` inclusive.
    pub fn new(min_section_y: i32, max_section_y: i32) -> Self {
        Self { min_section_y, max_section_y, ..Self::default() }
    }

    /// `setDirty` (and `onSectionLoad`): the section's own level anew.
    fn touched(&mut self, section: Pos) {
        let level = if self.sections.get(&section).is_some_and(PoiSection::village_center) { 0 } else { 7 };
        let sections = &self.sections;
        let center = |key: i64| sections.get(&section_of_key(key)).is_some_and(PoiSection::village_center);
        self.tracker.update(section_key(section), level, false, &center);
    }

    /// `DistanceTracker.runAllUpdates`.
    pub fn settle(&mut self) {
        let sections = &self.sections;
        let center = |key: i64| sections.get(&section_of_key(key)).is_some_and(PoiSection::village_center);
        self.tracker.run_all_updates(&center);
    }

    /// `PoiManager.add`: false when that type is already there.
    pub fn add(&mut self, pos: Pos, kind: PoiType) -> bool {
        let section = section_of(pos);
        let added = self.sections.entry(section).or_default().add(pos, kind);
        self.touched(section);
        added
    }

    /// `PoiManager.remove`.
    pub fn remove(&mut self, pos: Pos) {
        let section = section_of(pos);
        if let Some(s) = self.sections.get_mut(&section) {
            s.remove(pos);
        }
        self.touched(section);
    }

    /// A block changed (`ServerLevel.updatePOIOnBlockStateChange`, run as a
    /// server task): the old type's record goes and the new one's comes.
    pub fn block_changed(&mut self, pos: Pos, old: Option<PoiType>, new: Option<PoiType>) {
        if old == new {
            return;
        }
        if old.is_some() {
            self.remove(pos);
        }
        if let Some(kind) = new {
            self.add(pos, kind);
        }
    }

    /// Forgets the sections of an unloaded chunk (vanilla keeps them until
    /// saved; their village levels stay).
    pub fn unload_chunk(&mut self, cx: i32, cz: i32) {
        let gone: Vec<Pos> = self.sections.keys().copied().filter(|&(x, _, z)| (x, z) == (cx, cz)).collect();
        for section in gone {
            self.sections.remove(&section);
            self.touched(section);
        }
    }

    /// `checkConsistencyWithBlocks` for a newly loaded section: its
    /// blocks' records, in `blocksInside` order (x fastest, then y, z).
    pub fn load_section(&mut self, section: Pos, mut block: impl FnMut(Pos) -> Option<PoiType>) {
        let (sx, sy, sz) = section;
        for z in 0..16 {
            for y in 0..16 {
                for x in 0..16 {
                    let pos = (sx * 16 + x, sy * 16 + y, sz * 16 + z);
                    if let Some(kind) = block(pos) {
                        self.sections.entry(section).or_default().add(pos, kind);
                    }
                }
            }
        }
        self.touched(section);
    }

    /// Every record of a chunk in query order (`getInChunk` for any type).
    pub fn records_in_chunk(&self, cx: i32, cz: i32) -> Vec<PoiRecord> {
        self.in_chunk(cx, cz, &|_| true, Occupancy::Any).collect()
    }

    /// `getInChunk`.
    fn in_chunk<'a>(&'a self, cx: i32, cz: i32, types: &'a dyn Fn(PoiType) -> bool, occupancy: Occupancy) -> impl Iterator<Item = PoiRecord> + 'a {
        (self.min_section_y..=self.max_section_y).filter_map(move |sy| self.sections.get(&(cx, sy, cz))).flat_map(move |s| s.records(types, occupancy))
    }

    /// `getInSquare`: chunk by chunk (x fastest), then the square.
    pub fn in_square(&self, types: &dyn Fn(PoiType) -> bool, center: Pos, radius: i32, occupancy: Occupancy) -> Vec<PoiRecord> {
        let chunks = radius.div_euclid(16) + 1;
        let (ccx, ccz) = (center.0 >> 4, center.2 >> 4);
        let mut out = Vec::new();
        for cz in ccz - chunks..=ccz + chunks {
            for cx in ccx - chunks..=ccx + chunks {
                out.extend(self.in_chunk(cx, cz, types, occupancy).filter(|r| (r.pos.0 - center.0).abs() <= radius && (r.pos.2 - center.2).abs() <= radius));
            }
        }
        out
    }

    /// `getInRange`: the square, then within `radius` (block distance
    /// squared, `distSqr`).
    pub fn in_range(&self, types: &dyn Fn(PoiType) -> bool, center: Pos, radius: i32, occupancy: Occupancy) -> Vec<PoiRecord> {
        let limit = f64::from(radius * radius);
        self.in_square(types, center, radius, occupancy).into_iter().filter(|r| dist_sqr(r.pos, center) <= limit).collect()
    }

    /// `find(type, filter, center, radius, occupancy)`: the first in range
    /// the filter accepts (not the nearest).
    pub fn find_first(&self, types: &dyn Fn(PoiType) -> bool, filter: &dyn Fn(Pos) -> bool, center: Pos, radius: i32, occupancy: Occupancy) -> Option<Pos> {
        self.in_range(types, center, radius, occupancy).into_iter().map(|r| r.pos).find(|&p| filter(p))
    }

    /// `findClosest(type, center, radius, occupancy)`: the first nearest.
    pub fn find_closest(&self, types: &dyn Fn(PoiType) -> bool, center: Pos, radius: i32, occupancy: Occupancy) -> Option<Pos> {
        let mut best: Option<(f64, Pos)> = None;
        for r in self.in_range(types, center, radius, occupancy) {
            let d = dist_sqr(r.pos, center);
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, r.pos));
            }
        }
        best.map(|(_, pos)| pos)
    }

    /// `take`: the first record in range with space that `filter` accepts
    /// gives up a ticket.
    pub fn take(&mut self, types: &dyn Fn(PoiType) -> bool, filter: impl Fn(PoiType, Pos) -> bool, center: Pos, radius: i32) -> Option<Pos> {
        let pos = self.in_range(types, center, radius, Occupancy::HasSpace).into_iter().find(|r| filter(r.kind, r.pos))?.pos;
        let section = section_of(pos);
        if let Some(record) = self.sections.get_mut(&section).and_then(|s| s.records.get_mut(&pos)) {
            if record.free_tickets > 0 {
                record.free_tickets -= 1;
            }
        }
        self.touched(section);
        Some(pos)
    }

    /// `PoiRecord.acquireTicket` for the record at `pos` (false when it has
    /// none free).
    pub fn acquire(&mut self, pos: Pos) -> bool {
        let section = section_of(pos);
        let Some(record) = self.sections.get_mut(&section).and_then(|s| s.records.get_mut(&pos)) else { return false };
        let taken = record.free_tickets > 0;
        if taken {
            record.free_tickets -= 1;
        }
        self.touched(section);
        taken
    }

    /// `release`: a ticket back (false when all are free).
    pub fn release(&mut self, pos: Pos) -> bool {
        let section = section_of(pos);
        let Some(record) = self.sections.get_mut(&section).and_then(|s| s.records.get_mut(&pos)) else { return false };
        let freed = record.free_tickets < record.kind.max_tickets();
        if freed {
            record.free_tickets += 1;
        }
        self.touched(section);
        freed
    }

    /// `getType`.
    pub fn kind(&self, pos: Pos) -> Option<PoiType> {
        self.sections.get(&section_of(pos)).and_then(|s| s.records.get(&pos)).map(|r| r.kind)
    }

    /// `exists`.
    pub fn exists(&self, pos: Pos, types: impl Fn(PoiType) -> bool) -> bool {
        self.kind(pos).is_some_and(types)
    }

    /// The record at `pos`.
    pub fn record(&self, pos: Pos) -> Option<PoiRecord> {
        self.sections.get(&section_of(pos)).and_then(|s| s.records.get(&pos)).copied()
    }

    /// `sectionsToVillage`: 0 in a village centre, up to 6 around one, 7
    /// otherwise (updates settled first).
    pub fn sections_to_village(&mut self, section: Pos) -> i32 {
        self.settle();
        self.village_level(section)
    }

    /// The settled level (see [`PoiManager::settle`]).
    pub fn village_level(&self, section: Pos) -> i32 {
        self.tracker.level(section_key(section))
    }

    /// `ServerLevel.isVillage` (`isCloseToVillage(pos, 1)`).
    pub fn is_village(&mut self, pos: Pos) -> bool {
        self.sections_to_village(section_of(pos)) <= 1
    }

    /// `BehaviorUtils.findSectionClosestToVillage`: of the sections within
    /// `radius` of `center` (x fastest, then y, then z), the first nearest
    /// a village that is nearer than the centre, else the centre.
    pub fn closest_village_section(&mut self, center: Pos, radius: i32) -> Pos {
        self.settle();
        let here = self.village_level(center);
        let mut best: Option<(i32, Pos)> = None;
        for dz in -radius..=radius {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let section = (center.0 + dx, center.1 + dy, center.2 + dz);
                    let d = self.village_level(section);
                    if d < here && best.is_none_or(|(b, _)| d < b) {
                        best = Some((d, section));
                    }
                }
            }
        }
        best.map_or(center, |(_, s)| s)
    }

    /// Every record, section by section (for saving and inspection).
    pub fn all(&self) -> Vec<PoiRecord> {
        let mut out: Vec<PoiRecord> = self.sections.values().flat_map(|s| s.records.values().copied()).collect();
        out.sort_by_key(|r| (r.pos.1, r.pos.2, r.pos.0));
        out
    }
}

/// `Vec3i.distSqr`.
pub fn dist_sqr(a: Pos, b: Pos) -> f64 {
    let (dx, dy, dz) = (f64::from(a.0 - b.0), f64::from(a.1 - b.1), f64::from(a.2 - b.2));
    dx * dx + dy * dy + dz * dz
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beds_count_by_their_head() {
        assert_eq!(PoiType::of_block(&Block::new("minecraft:red_bed").with("part", "head")), Some(PoiType::Home));
        assert_eq!(PoiType::of_block(&Block::new("minecraft:red_bed").with("part", "foot")), None);
        assert_eq!(PoiType::of_block(&Block::new("minecraft:water_cauldron")), Some(PoiType::Leatherworker));
        assert_eq!(PoiType::of_block(&Block::new("minecraft:waxed_oxidized_lightning_rod")), Some(PoiType::LightningRod));
        assert_eq!(PoiType::of_block(&Block::new("minecraft:stone")), None);
    }

    #[test]
    fn tickets_make_a_village() {
        let mut pois = PoiManager::new(-4, 19);
        assert!(pois.add((5, 64, 5), PoiType::Home));
        assert!(!pois.add((5, 64, 5), PoiType::Home));
        assert_eq!(pois.sections_to_village((0, 4, 0)), 7);
        assert_eq!(pois.take(&|t| t == PoiType::Home, |_, _| true, (0, 64, 0), 48), Some((5, 64, 5)));
        assert_eq!(pois.take(&|t| t == PoiType::Home, |_, _| true, (0, 64, 0), 48), None);
        assert_eq!(pois.sections_to_village((0, 4, 0)), 0);
        assert_eq!(pois.sections_to_village((3, 5, -2)), 3);
        assert_eq!(pois.sections_to_village((7, 4, 0)), 7);
        assert!(pois.release((5, 64, 5)));
        assert!(!pois.release((5, 64, 5)));
        // The old centre keeps the top level, as vanilla's tracker does.
        assert_eq!(pois.sections_to_village((0, 4, 0)), 6);
        assert_eq!(pois.sections_to_village((20, 4, 0)), 7);
    }

    #[test]
    fn hash_set_order_follows_the_jdk() {
        // Vec3i hashes (y + z*31)*31 + x: positions a table's width apart in
        // x share a bucket and keep their insertion order.
        let mut set = JavaPosSet::default();
        for pos in [(16, 0, 0), (0, 0, 0), (1, 0, 0), (17, 0, 0)] {
            set.insert(pos);
        }
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![(16, 0, 0), (0, 0, 0), (1, 0, 0), (17, 0, 0)]);
        set.remove((16, 0, 0));
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![(0, 0, 0), (1, 0, 0), (17, 0, 0)]);
    }
}
