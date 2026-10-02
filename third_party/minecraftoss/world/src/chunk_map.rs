//! Player-driven chunk loading, generation and sending.
//!
//! Vanilla 26.3 behavior kept here:
//! - A player tracks `ChunkTrackingView.of(chunkPosition, viewDistance)`,
//!   with the view distance clamped to `2..=32` (`ChunkMap.getPlayerViewDistance`);
//!   this client's render distance option goes on to 64.
//! - Loading work starts nearest first by chessboard distance, the level
//!   `FixedPlayerDistanceChunkTracker` assigns to player tickets before the
//!   `ThrottlingChunkTaskDispatcher` releases them.
//! - A chunk is FULL once it and its eight neighbours have run FEATURES:
//!   vanilla's LIGHT step needs INITIALIZE_LIGHT, and so FEATURES, one ring
//!   out, so no later decoration writes into it. FEATURES itself needs its
//!   3x3 at TERRAIN and writes into all nine chunks.
//! - A chunk is sent once it is block ticking, its whole 3x3 FULL
//!   (`ChunkMap.prepareTickingChunk`), right after
//!   `LevelChunk.postProcessGeneration` re-shapes its marked positions.
//! - A chunk becomes pending for the player when it is ready and tracked
//!   (`onChunkReadyToSend`, `applyChunkTrackingView`). Leaving the view drops a
//!   pending chunk silently or tells the client to forget a sent one
//!   (`PlayerChunkSender.dropChunk`).
//! - Every tick a memory connection receives all pending chunks, sorted by
//!   squared distance to the player's chunk (`PlayerChunkSender.collectChunksToSend`).
//!
//! Vanilla generates a handful of ticketed chunks at a time. Here every idle
//! worker takes the best available job, TERRAIN or FEATURES, so the same
//! order completes far sooner. Decorations whose 3x3 regions overlap never
//! run at once: a FEATURES job checks out all nine chunks.

use crate::storage::ChunkStorage;
use crate::view::{EXTENDED_VIEW_DISTANCE, MIN_VIEW_DISTANCE, TrackingView};
use minecraftoss_core::chunk::{ChunkStatus, HeightmapKind};
use minecraftoss_core::{BlockPos, BlockStateId, Chunk, ChunkPos};
use minecraftoss_generator::feature::post_process::post_process;
use minecraftoss_generator::feature::{Decoration, Library, Region};
use minecraftoss_generator::structure::beardifier::Beardifier;
use minecraftoss_generator::structure::Structures;
use minecraftoss_generator::terrain::{NeighborBiomes, TerrainGenerator};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::thread::JoinHandle;
use std::time::Instant;

/// Where the generation workers spend their time (profiling): summed
/// microseconds and counts per stage, over the whole process.
pub mod stage_times {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;
    pub const TERRAIN_LOAD: usize = 0;
    pub const TERRAIN_BIOMES: usize = 1;
    pub const TERRAIN_REFERENCES: usize = 2;
    pub const TERRAIN_BEARDIFIER: usize = 3;
    pub const TERRAIN_BUILD: usize = 4;
    pub const FEATURES_REFERENCES: usize = 5;
    pub const FEATURES_DECORATE: usize = 6;
    pub const LOCK_WAIT: usize = 7;
    pub const LOCKED_FINISH: usize = 8;
    pub const TICK_POST_PROCESS: usize = 9;
    pub const TICK_LIGHT: usize = 10;
    pub const PICK: usize = 11;
    pub const IDLE: usize = 12;
    pub const SAVE: usize = 13;
    pub const SAVE_RESOLVE: usize = 14;
    pub const SAVE_NBT: usize = 15;
    pub const SAVE_COMPRESS: usize = 16;
    const NAMES: [&str; 17] = [
        "terrain load",
        "terrain biomes",
        "terrain references",
        "terrain beardifier",
        "terrain build",
        "features references",
        "features decorate",
        "world lock wait",
        "locked finish (finalize/spawn)",
        "tick post-process",
        "tick light",
        "pick (locked)",
        "idle",
        "save (saver thread)",
        "  save: copy and edits",
        "  save: chunk NBT",
        "  save: serialize and compress",
    ];
    static MICROS: [AtomicU64; 17] = [const { AtomicU64::new(0) }; 17];
    static COUNTS: [AtomicU64; 17] = [const { AtomicU64::new(0) }; 17];
    static MAX: [AtomicU64; 17] = [const { AtomicU64::new(0) }; 17];
    pub fn add(stage: usize, since: Instant) {
        let micros = since.elapsed().as_micros() as u64;
        MICROS[stage].fetch_add(micros, Ordering::Relaxed);
        COUNTS[stage].fetch_add(1, Ordering::Relaxed);
        MAX[stage].fetch_max(micros, Ordering::Relaxed);
    }
    /// Starts the sums again.
    pub fn reset() {
        for i in 0..NAMES.len() {
            MICROS[i].store(0, Ordering::Relaxed);
            COUNTS[i].store(0, Ordering::Relaxed);
            MAX[i].store(0, Ordering::Relaxed);
        }
    }
    /// Each stage's total seconds, count and mean.
    pub fn report() -> String {
        let mut out = String::new();
        for (i, name) in NAMES.iter().enumerate() {
            let (micros, count) = (MICROS[i].load(Ordering::Relaxed), COUNTS[i].load(Ordering::Relaxed));
            out += &format!("    {name:32} {:8.2}s {count:7} x {:8.0} us, max {:8.1} ms\n", micros as f64 / 1e6, micros.checked_div(count).unwrap_or(0), MAX[i].load(Ordering::Relaxed) as f64 / 1000.0);
        }
        out
    }
}

/// What the server sends the client, in order.
#[derive(Clone, Debug)]
pub enum ChunkEvent {
    /// `ClientboundSetChunkCacheCenterPacket`.
    Center(ChunkPos),
    /// `ClientboundLevelChunkWithLightPacket`.
    Load(Arc<Chunk>),
    /// `ClientboundForgetLevelChunkPacket`.
    Forget(ChunkPos),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ChunkMapStats {
    pub loaded: usize,
    pub queued: usize,
    pub generating: usize,
    pub generated: u64,
    pub decorated: u64,
    pub biome_chunks: usize,
    /// Mean worker time per TERRAIN chunk, including any BIOMES work it did.
    pub mean_generation_micros: u64,
    /// Mean worker time per FEATURES job.
    pub mean_decoration_micros: u64,
}

/// Everything one dimension needs to generate chunks up to FEATURES.
pub struct WorldGen {
    pub terrain: Arc<TerrainGenerator>,
    pub library: Library,
    pub decoration: Decoration,
    pub structures: Structures,
    /// Per-state light properties for lighting chunks.
    pub light: minecraftoss_core::light::LightTable,
    /// `DimensionType.hasSkyLight`.
    pub sky_light: bool,
    /// The SPAWN step's creatures.
    pub spawns: crate::natural_spawner::CreatureSpawns,
}

impl WorldGen {
    /// The Overworld.
    pub fn new(terrain: Arc<TerrainGenerator>) -> Result<Self, String> {
        Self::for_dimension(terrain, "minecraft:overworld")
    }

    /// A dimension with the given `dimension_type`.
    pub fn for_dimension(terrain: Arc<TerrainGenerator>, dimension_type: &str) -> Result<Self, String> {
        let mut library = Library::load(terrain.registries.clone(), terrain.generation_context())?;
        library.set_dimension_type(dimension_type)?;
        let decoration = Decoration::new(&library, &terrain.possible_biomes())?;
        let structures = Structures::load(&mut library, &terrain, true)?;
        let light = minecraftoss_core::light::LightTable::new(&terrain.registries.blocks);
        let id = minecraftoss_core::Identifier::parse(dimension_type)?;
        let sky_light = terrain.registries.datapack.read_json("dimension_type", &id)?.get("has_skylight").and_then(|v| v.as_bool()).unwrap_or(true);
        let spawns = crate::natural_spawner::CreatureSpawns::load(terrain.registries.clone(), dimension_type, terrain.disable_mob_generation)?;
        Ok(Self { terrain, library, decoration, structures, light, sky_light, spawns })
    }
}

/// A generated chunk in the map.
enum Slot {
    /// TERRAIN done; `decorated` once its own FEATURES step ran.
    Proto { chunk: Box<Chunk>, decorated: bool },
    /// Checked out to a FEATURES job.
    Out { decorated: bool },
    /// Its whole 3x3 is decorated: nothing will write into it again (FULL).
    Final(Arc<Chunk>),
}

impl Slot {
    fn decorated(&self) -> bool {
        match self {
            Self::Proto { decorated, .. } | Self::Out { decorated } => *decorated,
            Self::Final(_) => true,
        }
    }
}

/// What a position needs for the current view.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Need {
    Terrain,
    Features,
}

struct World {
    slots: HashMap<ChunkPos, Slot>,
    /// FULL chunks already post-processed (or being), which are then sent.
    ticking: HashSet<ChunkPos>,
    terrain_in_flight: HashSet<ChunkPos>,
    /// Wanted positions, best priority first.
    wanted: Vec<(ChunkPos, Need)>,
    /// Every entry of `wanted` before this is done.
    wanted_head: usize,
    jobs_in_flight: usize,
    stop: bool,
}

enum Job {
    Terrain(ChunkPos),
    /// The 3x3's chunks, and which of them are FULL copies whose slots
    /// stay as they are.
    Features(ChunkPos, Vec<Chunk>, [bool; 9]),
}

/// BIOMES chunks are shared by up to nine TERRAIN jobs. The first job to need
/// one computes it; concurrent jobs wait on the same cell.
type BiomeCell = Arc<OnceLock<Arc<Chunk>>>;

struct Shared {
    world: Mutex<World>,
    work: Condvar,
    /// Workers with a lower index than this take jobs; the rest wait (host
    /// change: all of them while pre-generating, few during play).
    active_workers: AtomicUsize,
    /// Signalled whenever a job finishes (for synchronous loads).
    done: Condvar,
    biomes: Mutex<HashMap<ChunkPos, BiomeCell>>,
    generated: AtomicU64,
    decorated: AtomicU64,
    generation_micros: AtomicU64,
    decoration_micros: AtomicU64,
    /// Saved chunks load instead of generating; dropped chunks are saved.
    storage: Option<Arc<ChunkStorage>>,
    /// Chunks dropped from the map but not yet saved: a chunk that returns
    /// comes back from here (or storage) as it was, never generated again,
    /// as vanilla reloads saved proto chunks. Without storage they stay here.
    evicted: Mutex<HashMap<ChunkPos, Evicted>>,
    registries: Arc<minecraftoss_core::Registries>,
    /// Wakes the saver thread.
    evicted_ready: Condvar,
    saver_stop: std::sync::atomic::AtomicBool,
}

/// A chunk dropped from the map but not yet saved.
#[derive(Clone)]
struct Evicted {
    chunk: Arc<Chunk>,
    /// The status it keeps.
    status: ChunkStatus,
    /// Block changes not yet written into it (see `ChunkMap::edits`); the
    /// saver or a reload writes them, off the thread that dropped it.
    edits: Arc<Vec<(BlockPos, BlockStateId)>>,
}

impl Evicted {
    /// The chunk as it is saved or reloaded: its status set and its edits
    /// written.
    fn resolve(self, registries: &minecraftoss_core::Registries) -> Chunk {
        let mut chunk = Arc::try_unwrap(self.chunk).unwrap_or_else(|shared| (*shared).clone());
        chunk.status = self.status;
        apply_edits(&mut chunk, &self.edits, registries);
        chunk
    }
}

pub struct ChunkMap {
    worldgen: Arc<WorldGen>,
    generator: Arc<TerrainGenerator>,
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
    saver: Option<JoinHandle<()>>,
    finished: mpsc::Receiver<Arc<Chunk>>,
    sender: mpsc::Sender<Arc<Chunk>>,
    /// Final chunks: ready to send.
    chunks: HashMap<ChunkPos, Arc<Chunk>>,
    view_distance: i32,
    view: Option<TrackingView>,
    /// `PlayerChunkSender.pendingChunks`.
    pending_send: HashSet<ChunkPos>,
    /// The tracking view the generation queue was last built for.
    queued_for: Option<TrackingView>,
    /// How long the last tick's steps took, in milliseconds (profiling):
    /// tracking, collecting, waiting for the world lock, scheduling, sending.
    pub last_tick_ms: [f64; 7],
    /// Block changes to FULL chunks not yet written into them. A sent chunk
    /// is shared with the client and the level, so writing each tick's
    /// changes would copy it every tick; instead they wait here and are
    /// written in one copy when the chunk is sent again, dropped or saved.
    edits: HashMap<ChunkPos, Vec<(BlockPos, BlockStateId)>>,
}

/// Generation order: the player ticket level (chessboard distance), then
/// Euclidean distance so each ring starts at its sides.
fn priority(center: ChunkPos, pos: ChunkPos) -> (i32, i32) {
    (center.chebyshev(pos), center.distance_squared(pos))
}

fn neighbours(pos: ChunkPos) -> impl Iterator<Item = ChunkPos> {
    (0..9).map(move |i| ChunkPos::new(pos.x + i % 3 - 1, pos.z + i / 3 - 1))
}

impl World {
    /// The best job for the wanted list, or for one target chunk.
    fn pick(&mut self, target: Option<ChunkPos>) -> Option<Job> {
        let job = match target {
            Some(t) => {
                let mut list: Vec<(ChunkPos, Need)> = Vec::new();
                for dz in -2..=2 {
                    for dx in -2..=2 {
                        let pos = ChunkPos::new(t.x + dx, t.z + dz);
                        let need = if t.chebyshev(pos) <= 1 { Need::Features } else { Need::Terrain };
                        list.push((pos, need));
                    }
                }
                // `ChunkGenerationTask.scheduleLayer`: after TERRAIN, the
                // FEATURES layer runs one chunk at a time, X-major then Z.
                list.sort_by_key(|&(p, need)| match need {
                    Need::Terrain => (0, priority(t, p).0, priority(t, p).1),
                    Need::Features => (1, p.x - t.x, p.z - t.z),
                });
                let mut job = None;
                for (p, n) in list {
                    if !self.done(p, n) {
                        job = self.try_job(p, n);
                        if job.is_some() {
                            break;
                        }
                    }
                }
                job
            }
            None => {
                // Done entries stay done, so the scan starts past the done
                // prefix and steps over done entries later in the list.
                let mut job = None;
                let mut prefix_done = true;
                let mut i = self.wanted_head;
                while i < self.wanted.len() {
                    let (pos, need) = self.wanted[i];
                    i += 1;
                    if self.done(pos, need) {
                        if prefix_done {
                            self.wanted_head = i;
                        }
                        continue;
                    }
                    prefix_done = false;
                    if let Some(found) = self.try_job(pos, need) {
                        job = Some(found);
                        break;
                    }
                }
                job
            }
        };
        if job.is_some() {
            self.jobs_in_flight += 1;
        }
        job
    }

    /// Whether a wanted position already has what it needs.
    fn done(&self, pos: ChunkPos, need: Need) -> bool {
        match self.slots.get(&pos) {
            Some(Slot::Final(_)) => true,
            Some(slot) => need == Need::Terrain || slot.decorated(),
            None => false,
        }
    }

    /// The job a wanted position can start now, if any.
    fn try_job(&mut self, pos: ChunkPos, need: Need) -> Option<Job> {
        match self.slots.get(&pos) {
            None => {
                if !self.terrain_in_flight.contains(&pos) {
                    self.terrain_in_flight.insert(pos);
                    return Some(Job::Terrain(pos));
                }
            }
            Some(Slot::Proto { decorated: false, .. }) if need == Need::Features => {
                // A FULL neighbour only happens when a save was cut
                // short (a chunk saved undecorated beside one saved
                // FULL). Vanilla's FEATURES step runs beside it anyway;
                // here it reads a copy, and the FULL chunk, already sent
                // and ticking, keeps its blocks.
                let ready = neighbours(pos).all(|n| matches!(self.slots.get(&n), Some(Slot::Proto { .. } | Slot::Final(_))));
                if ready {
                    let mut full = [false; 9];
                    let chunks = neighbours(pos)
                        .enumerate()
                        .map(|(i, n)| {
                            let slot = self.slots.get_mut(&n).expect("checked above");
                            if let Slot::Final(chunk) = slot {
                                full[i] = true;
                                return (**chunk).clone();
                            }
                            let decorated = slot.decorated();
                            match std::mem::replace(slot, Slot::Out { decorated }) {
                                Slot::Proto { chunk, .. } => *chunk,
                                _ => unreachable!("checked above"),
                            }
                        })
                        .collect();
                    return Some(Job::Features(pos, chunks, full));
                }
            }
            _ => {}
        }
        None
    }

    /// Chunks whose 3x3 is now decorated and returned become final. A job
    /// at `pos` can complete the 3x3 of any chunk within two rings. Returns
    /// the chunks that became FULL.
    fn finalize_around(&mut self, pos: ChunkPos, worldgen: &WorldGen) -> Vec<ChunkPos> {
        let registries = &*worldgen.terrain.registries;
        let mut finalized = Vec::new();
        for i in 0..25 {
            let candidate = ChunkPos::new(pos.x + i % 5 - 2, pos.z + i / 5 - 2);
            if !matches!(self.slots.get(&candidate), Some(Slot::Proto { decorated: true, .. })) {
                continue;
            }
            let all = neighbours(candidate).all(|n| matches!(self.slots.get(&n), Some(slot) if slot.decorated() && !matches!(slot, Slot::Out { .. })));
            if !all {
                continue;
            }
            // SPAWN: the 3x3 is decorated (and so lit) before the chunk is FULL.
            let spawned = self.spawn_original_mobs(candidate, worldgen);
            let slot = self.slots.get_mut(&candidate).expect("present");
            if let Slot::Proto { mut chunk, .. } = std::mem::replace(slot, Slot::Out { decorated: true }) {
                chunk.generation.entities.extend(spawned);
                promote_block_entities(&mut chunk, registries);
                *slot = Slot::Final(Arc::new(*chunk));
                finalized.push(candidate);
            }
        }
        finalized
    }

    /// The SPAWN step for a chunk whose 3x3 is decorated: the creatures
    /// `spawnOriginalMobs` adds to it.
    fn spawn_original_mobs(&self, pos: ChunkPos, worldgen: &WorldGen) -> Vec<minecraftoss_core::nbt::Tag> {
        let chunk = |p: ChunkPos| -> Option<&Chunk> {
            match self.slots.get(&p) {
                Some(Slot::Proto { chunk, .. }) => Some(chunk),
                Some(Slot::Final(chunk)) => Some(chunk),
                _ => None,
            }
        };
        let Some(chunks) = neighbours(pos).map(chunk).collect::<Option<Vec<&Chunk>>>() else { return Vec::new() };
        let chunks: [&Chunk; 9] = chunks.try_into().expect("nine chunks");
        spawn_in(worldgen, chunks, |dx, dz| chunk(ChunkPos::new(pos.x + dx, pos.z + dz)))
    }

    /// FULL chunks next to newly FULL ones whose whole 3x3 is now FULL:
    /// each with its nine chunks in `Region` order, ready to post-process.
    fn ready_to_tick(&mut self, finalized: &[ChunkPos]) -> Vec<(ChunkPos, Vec<Arc<Chunk>>)> {
        let mut ready = Vec::new();
        for &pos in finalized {
            for candidate in neighbours(pos) {
                if self.ticking.contains(&candidate) {
                    continue;
                }
                let around: Option<Vec<Arc<Chunk>>> = neighbours(candidate)
                    .map(|n| match self.slots.get(&n) {
                        Some(Slot::Final(chunk)) => Some(chunk.clone()),
                        _ => None,
                    })
                    .collect();
                if let Some(around) = around {
                    self.ticking.insert(candidate);
                    ready.push((candidate, around));
                }
            }
        }
        ready
    }
}

impl ChunkMap {
    /// Starts `workers` generation threads (at least one).
    pub fn new(generator: Arc<TerrainGenerator>, view_distance: i32, workers: usize) -> Self {
        let worldgen = Arc::new(WorldGen::new(generator).expect("load features"));
        Self::with_worldgen(worldgen, view_distance, workers)
    }

    /// A chunk map over an already loaded dimension.
    pub fn with_worldgen(worldgen: Arc<WorldGen>, view_distance: i32, workers: usize) -> Self {
        Self::with_storage(worldgen, view_distance, workers, None)
    }

    /// A chunk map that loads and saves chunks through `storage`.
    pub fn with_storage(worldgen: Arc<WorldGen>, view_distance: i32, workers: usize, storage: Option<ChunkStorage>) -> Self {
        let storage = storage.map(Arc::new);
        let generator = worldgen.terrain.clone();
        let shared = Arc::new(Shared {
            world: Mutex::new(World { slots: HashMap::new(), ticking: HashSet::new(), terrain_in_flight: HashSet::new(), wanted: Vec::new(), wanted_head: 0, jobs_in_flight: 0, stop: false }),
            work: Condvar::new(),
            done: Condvar::new(),
            biomes: Mutex::new(HashMap::new()),
            generated: AtomicU64::new(0),
            active_workers: AtomicUsize::new(workers.max(1)),
            decorated: AtomicU64::new(0),
            generation_micros: AtomicU64::new(0),
            decoration_micros: AtomicU64::new(0),
            storage,
            evicted: Mutex::new(HashMap::new()),
            registries: generator.registries.clone(),
            evicted_ready: Condvar::new(),
            saver_stop: std::sync::atomic::AtomicBool::new(false),
        });
        let saver = shared.storage.is_some().then(|| {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("chunk-saver".into())
                .spawn(move || {
                    minecraftoss_core::thread_priority::background();
                    saver(&shared)
                })
                .expect("spawn chunk saver")
        });
        let (sender, finished) = mpsc::channel();
        let workers = (0..workers.max(1))
            .map(|index| {
                let (shared, worldgen, sender) = (shared.clone(), worldgen.clone(), sender.clone());
                std::thread::Builder::new()
                    .name(format!("chunk-gen-{index}"))
                    .spawn(move || {
                        minecraftoss_core::thread_priority::background();
                        worker(index, &shared, &worldgen, &sender)
                    })
                    .expect("spawn chunk generation worker")
            })
            .collect();
        Self {
            worldgen,
            generator,
            shared,
            workers,
            saver,
            finished,
            sender,
            chunks: HashMap::new(),
            edits: HashMap::new(),
            last_tick_ms: [0.0; 7],
            view_distance: view_distance.clamp(MIN_VIEW_DISTANCE, EXTENDED_VIEW_DISTANCE),
            view: None,
            pending_send: HashSet::new(),
            queued_for: None,
        }
    }

    pub fn generator(&self) -> &Arc<TerrainGenerator> {
        &self.generator
    }

    pub fn world_gen(&self) -> &Arc<WorldGen> {
        &self.worldgen
    }

    /// A final chunk, generating and decorating its surroundings on this
    /// thread first if needed (spawn searches, before any player is tracking).
    pub fn load_now(&mut self, pos: ChunkPos) -> Arc<Chunk> {
        self.collect_finished();
        self.write_edits(pos);
        if let Some(chunk) = self.chunks.get(&pos) {
            return chunk.clone();
        }
        loop {
            let job = {
                let mut world = self.shared.world.lock().expect("chunk world");
                loop {
                    if let Some(Slot::Final(chunk)) = world.slots.get(&pos) {
                        let chunk = chunk.clone();
                        drop(world);
                        self.collect_finished();
                        self.chunks.insert(pos, chunk.clone());
                        return chunk;
                    }
                    if let Some(job) = world.pick(Some(pos)) {
                        break job;
                    }
                    world = self.shared.done.wait(world).expect("chunk world");
                }
            };
            run_job(&self.shared, &self.worldgen, job, &self.sender);
        }
    }

    pub fn view_distance(&self) -> i32 {
        self.view_distance
    }

    /// Generation threads started.
    pub fn workers(&self) -> usize {
        self.workers.len()
    }

    /// Limits how many generation threads take jobs (at least one). Fewer
    /// threads leave more CPU to the host's main and render threads.
    pub fn set_active_workers(&self, n: usize) {
        self.shared.active_workers.store(n.clamp(1, self.workers.len().max(1)), Ordering::Relaxed);
        // Under the lock, so no worker misses the wake-up.
        let _world = self.shared.world.lock().expect("chunk world");
        self.shared.work.notify_all();
    }

    /// Takes effect on the next tick, like `ChunkMap.setServerViewDistance`.
    pub fn set_view_distance(&mut self, view_distance: i32) {
        self.view_distance = view_distance.clamp(MIN_VIEW_DISTANCE, EXTENDED_VIEW_DISTANCE);
    }

    /// A final chunk with its edits, whether or not it has been sent.
    pub fn chunk(&mut self, pos: ChunkPos) -> Option<&Arc<Chunk>> {
        self.write_edits(pos);
        self.chunks.get(&pos)
    }

    /// Runs one server tick for the single tracked player and returns what to
    /// send to the client, in order.
    pub fn tick(&mut self, player_chunk: ChunkPos) -> Vec<ChunkEvent> {
        let mut events = Vec::new();
        let started = Instant::now();
        let lap = || started.elapsed().as_secs_f64() * 1000.0;
        self.last_tick_ms = [0.0; 7];
        self.update_tracking(player_chunk, &mut events);
        let tracked = lap();
        self.collect_finished();
        let collected = lap();
        self.schedule();
        let scheduled = lap();
        self.send(player_chunk, &mut events);
        let sent = lap();
        let [_, _, lock_wait, _, _, locked, evicting] = self.last_tick_ms;
        self.last_tick_ms = [tracked, collected - tracked, lock_wait, scheduled - collected - lock_wait, sent - scheduled, locked, evicting];
        events
    }

    /// `ChunkMap.updateChunkTracking` / `applyChunkTrackingView`.
    fn update_tracking(&mut self, player_chunk: ChunkPos, events: &mut Vec<ChunkEvent>) {
        let next = TrackingView::new(player_chunk, self.view_distance);
        if self.view == Some(next) {
            return;
        }
        if self.view.is_none_or(|last| last.center != next.center) {
            events.push(ChunkEvent::Center(next.center));
        }
        let (mut entered, mut left) = (Vec::new(), Vec::new());
        TrackingView::difference(self.view, Some(next), |pos| entered.push(pos), |pos| left.push(pos));
        for pos in entered {
            if self.chunks.contains_key(&pos) {
                self.pending_send.insert(pos);
            }
        }
        for pos in left {
            if !self.pending_send.remove(&pos) {
                events.push(ChunkEvent::Forget(pos));
            }
        }
        self.view = Some(next);
    }

    fn collect_finished(&mut self) {
        for chunk in self.finished.try_iter().collect::<Vec<_>>() {
            // `onChunkReadyToSend`: tracked chunks become pending immediately.
            if self.view.is_some_and(|view| view.contains(chunk.pos)) {
                self.pending_send.insert(chunk.pos);
            }
            self.chunks.insert(chunk.pos, chunk);
        }
    }

    /// Rebuilds the job list when the tracked area moves, and unloads what
    /// fell out of range.
    fn schedule(&mut self) {
        let Some(view) = self.view else { return };
        if self.queued_for == Some(view) {
            return;
        }
        self.queued_for = Some(view);
        let center = view.center;
        // Loaded chunks are kept a few rings beyond the generated area so
        // small back-and-forth movement does not regenerate them.
        let keep = self.view_distance + 5;
        self.chunks.retain(|&pos, _| center.chebyshev(pos) <= keep);
        self.shared.biomes.lock().expect("biome cache").retain(|&pos, _| center.chebyshev(pos) <= keep + 1);

        // Tracked chunks must be block ticking, their 3x3 FULL: FEATURES two
        // rings out, TERRAIN three.
        // The tracked set dilated by two chunks (FEATURES) and three
        // (TERRAIN), as separable Chebyshev max filters on a dense grid.
        let half = self.view_distance + 1 + 3;
        let size = (2 * half + 1) as usize;
        let origin = (center.x - half, center.z - half);
        let mut tracked = vec![false; size * size];
        view.for_each(|pos| tracked[(pos.x - origin.0) as usize * size + (pos.z - origin.1) as usize] = true);
        let dilate = |mask: &[bool], radius: i32| -> Vec<bool> {
            let mut along_z = vec![false; size * size];
            for x in 0..size {
                for z in 0..size {
                    let lo = (z as i32 - radius).max(0) as usize;
                    let hi = (z as i32 + radius).min(size as i32 - 1) as usize;
                    along_z[x * size + z] = (lo..=hi).any(|k| mask[x * size + k]);
                }
            }
            let mut out = vec![false; size * size];
            for x in 0..size {
                let lo = (x as i32 - radius).max(0) as usize;
                let hi = (x as i32 + radius).min(size as i32 - 1) as usize;
                for z in 0..size {
                    out[x * size + z] = (lo..=hi).any(|k| along_z[k * size + z]);
                }
            }
            out
        };
        let features = dilate(&tracked, 2);
        let terrain = dilate(&tracked, 3);
        let mut wanted: Vec<(ChunkPos, Need)> = Vec::new();
        for x in 0..size {
            for z in 0..size {
                let i = x * size + z;
                if terrain[i] {
                    let need = if features[i] { Need::Features } else { Need::Terrain };
                    wanted.push((ChunkPos::new(origin.0 + x as i32, origin.1 + z as i32), need));
                }
            }
        }
        wanted.sort_unstable_by_key(|&(pos, _)| priority(center, pos));
        let waited = Instant::now();
        let mut world = self.shared.world.lock().expect("chunk world");
        self.last_tick_ms[2] = waited.elapsed().as_secs_f64() * 1000.0;
        let locked = Instant::now();
        let dropped: Vec<ChunkPos> = world
            .slots
            .iter()
            .filter(|&(&pos, slot)| !matches!(slot, Slot::Out { .. }) && center.chebyshev(pos) > keep)
            .map(|(&pos, _)| pos)
            .collect();
        let dropped: Vec<(ChunkPos, Slot)> = dropped.into_iter().filter_map(|pos| world.slots.remove(&pos).map(|slot| (pos, slot))).collect();
        let World { slots, ticking, .. } = &mut *world;
        ticking.retain(|pos| slots.contains_key(pos));
        world.wanted = wanted;
        world.wanted_head = 0;
        drop(world);
        self.last_tick_ms[5] = locked.elapsed().as_secs_f64() * 1000.0;
        self.shared.work.notify_all();
        let evicting = Instant::now();
        // Dropped chunks, with any waiting edits, stay in memory until the
        // saver has written them.
        if !dropped.is_empty() {
            let mut evicted = self.shared.evicted.lock().expect("evicted chunks");
            for (pos, slot) in dropped {
                let (chunk, status) = match slot {
                    Slot::Proto { chunk, decorated } => (Arc::new(*chunk), if decorated { ChunkStatus::Features } else { ChunkStatus::Terrain }),
                    Slot::Final(chunk) => (chunk, ChunkStatus::Full),
                    Slot::Out { .. } => continue,
                };
                let edits = Arc::new(self.edits.remove(&pos).unwrap_or_default());
                evicted.insert(pos, Evicted { chunk, status, edits });
            }
            drop(evicted);
            self.shared.evicted_ready.notify_one();
        }
        self.last_tick_ms[6] = evicting.elapsed().as_secs_f64() * 1000.0;
    }

    /// Sets blocks in FULL chunks (player and level changes), so the
    /// changes are kept and saved with the chunks. Chunks the client has
    /// not been sent are left alone.
    pub fn set_blocks(&mut self, edits: &[(BlockPos, BlockStateId)]) {
        for &(pos, state) in edits {
            let chunk = pos.chunk();
            if self.chunks.contains_key(&chunk) {
                self.edits.entry(chunk).or_default().push((pos, state));
            }
        }
    }

    pub fn set_block(&mut self, pos: BlockPos, state: BlockStateId) {
        self.set_blocks(&[(pos, state)]);
    }

    /// Writes a chunk's waiting edits into its FULL slot and sendable copy.
    fn write_edits(&mut self, pos: ChunkPos) {
        let Some(edits) = self.edits.remove(&pos) else { return };
        let mut world = self.shared.world.lock().expect("chunk world");
        if let Some(Slot::Final(chunk)) = world.slots.get_mut(&pos) {
            apply_edits(Arc::make_mut(chunk), &edits, &self.generator.registries);
            let updated = chunk.clone();
            drop(world);
            self.chunks.insert(pos, updated);
        }
    }

    /// The storage chunks load from and save to, which the level shares
    /// for its entities.
    pub fn storage(&self) -> Option<Arc<ChunkStorage>> {
        self.shared.storage.clone()
    }

    /// Saves every chunk in memory and writes the region files.
    pub fn save_all(&mut self) {
        let edited: Vec<ChunkPos> = self.edits.keys().copied().collect();
        for pos in edited {
            self.write_edits(pos);
        }
        let world = self.shared.world.lock().expect("chunk world");
        save_slots(&self.shared, world.slots.values());
        drop(world);
        let evicted: Vec<Evicted> = self.shared.evicted.lock().expect("evicted chunks").values().cloned().collect();
        save_evicted(&self.shared, evicted);
    }

    /// `PlayerChunkSender.sendNextChunks` on a memory connection.
    fn send(&mut self, player_chunk: ChunkPos, events: &mut Vec<ChunkEvent>) {
        if self.pending_send.is_empty() {
            return;
        }
        // A chunk sent again carries its edits.
        let edited: Vec<ChunkPos> = self.pending_send.iter().filter(|pos| self.edits.contains_key(pos)).copied().collect();
        for pos in edited {
            self.write_edits(pos);
        }
        let mut ready: Vec<&Arc<Chunk>> = self.pending_send.iter().filter_map(|pos| self.chunks.get(pos)).collect();
        ready.sort_by_key(|chunk| player_chunk.distance_squared(chunk.pos));
        events.extend(ready.into_iter().map(|chunk| ChunkEvent::Load(chunk.clone())));
        self.pending_send.clear();
    }

    /// Diagnostics: the state of one position in the chunk map.
    pub fn debug_slot(&self, pos: ChunkPos) -> String {
        let world = self.shared.world.lock().expect("chunk world");
        let slot = match world.slots.get(&pos) {
            None if world.terrain_in_flight.contains(&pos) => "terrain generating",
            None => "none",
            Some(Slot::Proto { decorated: false, .. }) => "terrain",
            Some(Slot::Proto { decorated: true, .. }) => "decorated",
            Some(Slot::Out { .. }) => "in a decoration job",
            Some(Slot::Final(_)) => "full",
        };
        let wanted = world.wanted.iter().find(|(p, _)| *p == pos).map(|(_, need)| match need {
            Need::Terrain => " wanted:terrain",
            Need::Features => " wanted:features",
        });
        format!(
            "{slot}{}{}{}{}",
            wanted.unwrap_or(""),
            if world.ticking.contains(&pos) { " ticking" } else { "" },
            if self.chunks.contains_key(&pos) { " sendable" } else { "" },
            if self.pending_send.contains(&pos) { " pending-send" } else { "" },
        )
    }

    /// Diagnostics: jobs in flight and wanted positions.
    pub fn debug_queue(&self) -> String {
        let world = self.shared.world.lock().expect("chunk world");
        format!("jobs in flight {} wanted {} terrain in flight {} slots {} ticking {} sendable {} pending-send {} view {:?}", world.jobs_in_flight, world.wanted.len() - world.wanted_head, world.terrain_in_flight.len(), world.slots.len(), world.ticking.len(), self.chunks.len(), self.pending_send.len(), self.view)
    }

    /// Diagnostics: the first wanted positions, each with its slot and its
    /// 3x3's slots.
    pub fn debug_wanted(&self, count: usize) -> Vec<String> {
        let world = self.shared.world.lock().expect("chunk world");
        let slot = |pos: ChunkPos| match world.slots.get(&pos) {
            None if world.terrain_in_flight.contains(&pos) => "gen",
            None => "-",
            Some(Slot::Proto { decorated: false, .. }) => "T",
            Some(Slot::Proto { decorated: true, .. }) => "D",
            Some(Slot::Out { .. }) => "out",
            Some(Slot::Final(_)) => "F",
        };
        world
            .wanted
            .iter()
            .take(count)
            .map(|&(pos, need)| {
                let around: Vec<&str> = neighbours(pos).map(slot).collect();
                format!("{pos:?} {} {} 3x3 {around:?}", if need == Need::Features { "wants features" } else { "wants terrain" }, slot(pos))
            })
            .collect()
    }

    pub fn stats(&self) -> ChunkMapStats {
        let world = self.shared.world.lock().expect("chunk world");
        let generated = self.shared.generated.load(Ordering::Relaxed);
        let decorated = self.shared.decorated.load(Ordering::Relaxed);
        ChunkMapStats {
            loaded: self.chunks.len(),
            queued: world.wanted.len() - world.wanted_head,
            generating: world.jobs_in_flight,
            generated,
            decorated,
            biome_chunks: self.shared.biomes.lock().expect("biome cache").len(),
            mean_generation_micros: self.shared.generation_micros.load(Ordering::Relaxed).checked_div(generated).unwrap_or(0),
            mean_decoration_micros: self.shared.decoration_micros.load(Ordering::Relaxed).checked_div(decorated).unwrap_or(0),
        }
    }

    /// True once every tracked chunk has been generated and sent.
    pub fn is_idle(&self) -> bool {
        let Some(view) = self.view else { return true };
        let mut all = true;
        view.for_each(|pos| all &= self.chunks.contains_key(&pos));
        all && self.pending_send.is_empty()
    }
}

impl Drop for ChunkMap {
    fn drop(&mut self) {
        self.shared.world.lock().expect("chunk world").stop = true;
        self.shared.work.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        self.shared.saver_stop.store(true, Ordering::Relaxed);
        self.shared.evicted_ready.notify_all();
        if let Some(saver) = self.saver.take() {
            let _ = saver.join();
        }
        self.save_all();
    }
}

/// Saves chunks with the status their slot stands for: decorated protos as
/// FEATURES, the rest as TERRAIN, and FULL chunks (whose post-processing
/// marks are kept until they tick) as FULL.
/// Writes evicted chunks with the status they were dropped at.
fn save_evicted(shared: &Shared, chunks: Vec<Evicted>) {
    let Some(storage) = &shared.storage else { return };
    for evicted in chunks {
        let started = Instant::now();
        let chunk = evicted.resolve(&shared.registries);
        stage_times::add(stage_times::SAVE_RESOLVE, started);
        if let Err(e) = storage.save(&chunk) {
            eprintln!("chunk save failed: {e}");
        }
        stage_times::add(stage_times::SAVE, started);
    }
    if let Err(e) = storage.flush() {
        eprintln!("region write failed: {e}");
    }
}

/// The saver thread: writes evicted chunks in batches, then forgets each
/// one unless it was reloaded or evicted again meanwhile.
fn saver(shared: &Shared) {
    loop {
        let batch: Vec<(ChunkPos, Evicted)> = {
            let mut evicted = shared.evicted.lock().expect("evicted chunks");
            while evicted.is_empty() && !shared.saver_stop.load(Ordering::Relaxed) {
                evicted = shared.evicted_ready.wait(evicted).expect("evicted chunks");
            }
            if evicted.is_empty() {
                return;
            }
            evicted.iter().take(64).map(|(&pos, entry)| (pos, entry.clone())).collect()
        };
        let saved: Vec<(ChunkPos, Arc<Chunk>)> = batch.iter().map(|(pos, entry)| (*pos, entry.chunk.clone())).collect();
        save_evicted(shared, batch.into_iter().map(|(_, entry)| entry).collect());
        let mut evicted = shared.evicted.lock().expect("evicted chunks");
        for (pos, chunk) in saved {
            if evicted.get(&pos).is_some_and(|current| Arc::ptr_eq(&current.chunk, &chunk)) {
                evicted.remove(&pos);
            }
        }
    }
}

/// Block changes written into a chunk, with its heightmaps.
fn apply_edits(chunk: &mut Chunk, edits: &[(BlockPos, BlockStateId)], registries: &minecraftoss_core::Registries) {
    // The light no longer matches the blocks: it is solved again from them
    // (as the level does) rather than sent stale.
    if !edits.is_empty() {
        chunk.light = None;
    }
    for &(pos, state) in edits {
        if pos.y < chunk.min_y() || pos.y >= chunk.min_y() + chunk.height() {
            continue;
        }
        let (x, z) = ((pos.x & 15) as usize, (pos.z & 15) as usize);
        chunk.set_block_raw(x, pos.y, z, state, registries);
        for kind in HeightmapKind::FINAL {
            chunk.update_heightmap(kind, x, pos.y, z, state, registries);
        }
    }
}

fn save_slots<'a>(shared: &Shared, slots: impl Iterator<Item = &'a Slot>) {
    let Some(storage) = &shared.storage else { return };
    for slot in slots {
        let result = match slot {
            Slot::Proto { chunk, decorated } => {
                let mut chunk = (**chunk).clone();
                chunk.status = if *decorated { ChunkStatus::Features } else { ChunkStatus::Terrain };
                storage.save(&chunk)
            }
            Slot::Final(chunk) => {
                let mut chunk = (**chunk).clone();
                chunk.status = ChunkStatus::Full;
                storage.save(&chunk)
            }
            Slot::Out { .. } => Ok(()),
        };
        if let Err(e) = result {
            eprintln!("chunk save failed: {e}");
        }
    }
    if let Err(e) = storage.flush() {
        eprintln!("region write failed: {e}");
    }
}

fn worker(index: usize, shared: &Shared, worldgen: &WorldGen, finished: &mpsc::Sender<Arc<Chunk>>) {
    loop {
        let job = {
            let waited = Instant::now();
            let mut world = shared.world.lock().expect("chunk world");
            stage_times::add(stage_times::LOCK_WAIT, waited);
            loop {
                if world.stop {
                    return;
                }
                if index >= shared.active_workers.load(Ordering::Relaxed) {
                    world = shared.work.wait(world).expect("chunk world");
                    continue;
                }
                let picking = Instant::now();
                let job = world.pick(None);
                stage_times::add(stage_times::PICK, picking);
                if let Some(job) = job {
                    break job;
                }
                let idle = Instant::now();
                world = shared.work.wait(world).expect("chunk world");
                stage_times::add(stage_times::IDLE, idle);
            }
        };
        run_job(shared, worldgen, job, finished);
    }
}

fn run_job(shared: &Shared, worldgen: &WorldGen, job: Job, finished: &mpsc::Sender<Arc<Chunk>>) {
    let started = Instant::now();
    match job {
        Job::Terrain(pos) => {
            // A chunk dropped earlier comes back as it was: from memory if
            // it is not saved yet, else from storage.
            let evicted = shared.evicted.lock().expect("evicted chunks").remove(&pos);
            let evicted = evicted.map(|entry| entry.resolve(&shared.registries));
            let saved = evicted.or_else(|| {
                shared.storage.as_ref().and_then(|s| {
                    s.load(pos).unwrap_or_else(|e| {
                        eprintln!("chunk {pos:?} failed to load, regenerating: {e}");
                        None
                    })
                })
            });
            let loaded = saved.is_some();
            stage_times::add(stage_times::TERRAIN_LOAD, started);
            let chunk = saved.unwrap_or_else(|| generate_terrain(shared, worldgen, pos));
            shared.generation_micros.fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
            shared.generated.fetch_add(1, Ordering::Relaxed);
            let waited = Instant::now();
            let mut world = shared.world.lock().expect("chunk world");
            stage_times::add(stage_times::LOCK_WAIT, waited);
            let locked = Instant::now();
            world.terrain_in_flight.remove(&pos);
            let status = chunk.status;
            let slot = if loaded && status == ChunkStatus::Full {
                Slot::Final(Arc::new(chunk))
            } else {
                Slot::Proto { chunk: Box::new(chunk), decorated: loaded && status >= ChunkStatus::Features }
            };
            world.slots.insert(pos, slot);
            world.jobs_in_flight -= 1;
            if loaded {
                // A saved chunk can complete 3x3s around it.
                let mut finalized = world.finalize_around(pos, worldgen);
                if status == ChunkStatus::Full {
                    finalized.push(pos);
                }
                let ready = world.ready_to_tick(&finalized);
                drop(world);
                stage_times::add(stage_times::LOCKED_FINISH, locked);
                tick_ready(shared, worldgen, ready, finished);
            } else {
                drop(world);
                stage_times::add(stage_times::LOCKED_FINISH, locked);
            }
        }
        Job::Features(pos, chunks, full) => {
            let seed = worldgen.terrain.seed;
            let mut region = Region::new(pos, chunks, worldgen.terrain.registries.clone(), seed);
            region.set_level_random(worldgen.terrain.region_random(pos));
            let references = worldgen.structures.references(&worldgen.library, &worldgen.terrain, pos);
            stage_times::add(stage_times::FEATURES_REFERENCES, started);
            let decorating = Instant::now();
            worldgen.decoration.decorate_with(&worldgen.library, &mut region, seed, Some((&worldgen.structures, &references)));
            let chunks = region.into_chunks();
            stage_times::add(stage_times::FEATURES_DECORATE, decorating);
            shared.decoration_micros.fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
            shared.decorated.fetch_add(1, Ordering::Relaxed);
            let waited = Instant::now();
            let mut world = shared.world.lock().expect("chunk world");
            stage_times::add(stage_times::LOCK_WAIT, waited);
            let locked = Instant::now();
            for (i, (n, chunk)) in neighbours(pos).zip(chunks).enumerate() {
                if full[i] {
                    continue;
                }
                let decorated = n == pos || world.slots.get(&n).is_some_and(Slot::decorated);
                world.slots.insert(n, Slot::Proto { chunk: Box::new(chunk), decorated });
            }
            world.jobs_in_flight -= 1;
            let finalized = world.finalize_around(pos, worldgen);
            let ready = world.ready_to_tick(&finalized);
            drop(world);
            stage_times::add(stage_times::LOCKED_FINISH, locked);
            tick_ready(shared, worldgen, ready, finished);
        }
    }
    shared.work.notify_all();
    shared.done.notify_all();
}

/// `ChunkStatusTasks.generateSpawn` for the centre of a decorated 3x3;
/// `around(dx, dz)` reaches chunks up to two away, which decide where the
/// centre stores light.
pub fn spawn_in<'a>(worldgen: &WorldGen, chunks: [&'a Chunk; 9], around: impl Fn(i32, i32) -> Option<&'a Chunk>) -> Vec<minecraftoss_core::nbt::Tag> {
    let center = chunks[4];
    let registries = &*worldgen.terrain.registries;
    let compute_light = || {
        let min_section = center.min_section_y();
        minecraftoss_core::light::light_chunk(
            &registries.blocks,
            &worldgen.light,
            chunks,
            |dx, dz, sy| {
                let sections = around(dx, dz).map(|c| c.sections());
                sections.and_then(|s| s.get((sy - min_section) as usize)).is_some_and(|s| !s.is_empty())
            },
            worldgen.sky_light,
        )
    };
    let seed = worldgen.terrain.seed;
    let mut region = crate::natural_spawner::GenerationRegion::new(registries, chunks, seed, worldgen.terrain.region_random(center.pos), &compute_light, worldgen.sky_light);
    let max_y = center.min_y() + center.height() - 1;
    worldgen.spawns.spawn_original_mobs(&mut region, center.pos, seed, max_y);
    region.entities
}

/// `LevelChunk` from a `ProtoChunk`: pending block entity tags become
/// block entities (`registerAllBlockEntitiesAfterLevelLoad`).
fn promote_block_entities(chunk: &mut Chunk, registries: &minecraftoss_core::Registries) {
    let Some(catalog) = registries.block_entities.as_ref() else { return };
    let mut store = std::mem::take(&mut chunk.block_entities);
    catalog.promote(&mut store, |(x, y, z)| {
        let state = chunk.block((x & 15) as usize, y, (z & 15) as usize);
        registries.blocks.block(registries.blocks.block_of(state)).name.as_str().to_owned()
    });
    chunk.block_entities = store;
}

/// Post-processes chunks whose 3x3 became FULL and sends them.
fn tick_ready(shared: &Shared, worldgen: &WorldGen, ready: Vec<(ChunkPos, Vec<Arc<Chunk>>)>, finished: &mpsc::Sender<Arc<Chunk>>) {
    for (pos, around) in ready {
        // Which sections of the 5x5 hold blocks: they decide where light is stored.
        let world = shared.world.lock().expect("chunk world");
        let non_empty: Vec<Option<Vec<bool>>> = (0..25)
            .map(|i| {
                let p = ChunkPos::new(pos.x + i % 5 - 2, pos.z + i / 5 - 2);
                let chunk: Option<&Chunk> = match world.slots.get(&p) {
                    Some(Slot::Proto { chunk, .. }) => Some(chunk),
                    Some(Slot::Final(chunk)) => Some(chunk),
                    _ => None,
                };
                chunk.map(|c| c.sections().iter().map(|s| !s.is_empty()).collect())
            })
            .collect();
        drop(world);
        let chunk = start_ticking(worldgen, around, &non_empty);
        let mut world = shared.world.lock().expect("chunk world");
        if let Some(slot @ Slot::Final(_)) = world.slots.get_mut(&pos) {
            *slot = Slot::Final(chunk.clone());
        }
        drop(world);
        let _ = finished.send(chunk);
    }
}

/// `prepareTickingChunk` for the center of a FULL 3x3: post-processing,
/// skipped (and the chunk shared as is) when generation marked nothing.
fn start_ticking(worldgen: &WorldGen, around: Vec<Arc<Chunk>>, non_empty: &[Option<Vec<bool>>]) -> Arc<Chunk> {
    let started = Instant::now();
    let mut center = if around[4].generation.post_processing.is_empty() {
        (*around[4]).clone()
    } else {
        let chunks = around.iter().map(|c| (**c).clone()).collect();
        post_process(&worldgen.library, chunks, worldgen.terrain.seed)
    };
    stage_times::add(stage_times::TICK_POST_PROCESS, started);
    let lighting = Instant::now();
    if center.light.is_none() {
        // Light from the post-processed center and its neighbours.
        let min_section = center.min_section_y();
        let lit = {
            let chunks: [&Chunk; 9] = std::array::from_fn(|i| if i == 4 { &center } else { &*around[i] });
            minecraftoss_core::light::light_chunk(
                &worldgen.terrain.registries.blocks,
                &worldgen.light,
                chunks,
                |dx, dz, sy| {
                    let sections = non_empty[((dz + 2) * 5 + dx + 2) as usize].as_ref();
                    sections.and_then(|s| s.get((sy - min_section) as usize).copied()).unwrap_or(false)
                },
                worldgen.sky_light,
            )
        };
        center.light = Some(Arc::new(lit));
    }
    stage_times::add(stage_times::TICK_LIGHT, lighting);
    Arc::new(center)
}

fn biome_chunk(shared: &Shared, generator: &TerrainGenerator, pos: ChunkPos) -> Arc<Chunk> {
    let cell = shared.biomes.lock().expect("biome cache").entry(pos).or_default().clone();
    cell.get_or_init(|| {
        let mut chunk = generator.new_chunk(pos);
        generator.fill_biomes(&mut chunk);
        Arc::new(chunk)
    })
    .clone()
}

/// TERRAIN for one chunk, with BIOMES for its 3x3 neighborhood and the
/// terrain adaptation of structures that reference it.
fn generate_terrain(shared: &Shared, worldgen: &WorldGen, pos: ChunkPos) -> Chunk {
    let generator = &*worldgen.terrain;
    let started = Instant::now();
    let biomes: [Arc<Chunk>; 9] = std::array::from_fn(|i| {
        let (dx, dz) = (i as i32 % 3 - 1, i as i32 / 3 - 1);
        biome_chunk(shared, generator, ChunkPos::new(pos.x + dx, pos.z + dz))
    });
    let neighbors = NeighborBiomes { center: pos, chunks: std::array::from_fn(|i| &*biomes[i]) };
    let possible = neighbors.possible();
    let mut chunk = (*biomes[4]).clone();
    stage_times::add(stage_times::TERRAIN_BIOMES, started);
    let referencing = Instant::now();
    let references = worldgen.structures.references(&worldgen.library, generator, pos);
    stage_times::add(stage_times::TERRAIN_REFERENCES, referencing);
    let bearding = Instant::now();
    let beardifier = Beardifier::for_chunk(&worldgen.structures, &references, pos).map(Arc::new);
    stage_times::add(stage_times::TERRAIN_BEARDIFIER, bearding);
    let building = Instant::now();
    generator.build_terrain_with(&mut chunk, &neighbors, &possible, beardifier);
    stage_times::add(stage_times::TERRAIN_BUILD, building);
    chunk
}
