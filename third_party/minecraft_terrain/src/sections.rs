//! Client render sections: which 16x16x16 sections are visible, which need
//! building, and in what order. Follows vanilla 26.3 `VisGraph`,
//! `VisibilitySet`, `SectionOcclusionGraph`, `SectionUpdateTracker`, the
//! section-update pass of `LevelExtractor` and `SectionTaskDynamicQueue`
//! (pinned client JAR 5856085e...). As in vanilla's `ViewArea`, sections
//! live in an array over the view area that wraps around as the camera
//! moves, and full graph rebuilds run on a background thread over a copy of
//! their inputs while the previous graph stays in use.
//!
//! The graph spreads from the camera's section through faces a section's
//! open cells connect. An uncompiled section connects nothing and an all-air
//! section connects everything, so the visible world grows outward as
//! sections finish compiling. Only frustum-visible sections are compiled.

use crate::frame_spans::span;
use glam::{DVec3, Mat4, Vec3, Vec4};
use crate::fast_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::collections::VecDeque;

pub type SectionPos = (i32, i32, i32);
pub type ChunkPos = (i32, i32);

/// Vanilla `Direction` order: down, up, north, south, west, east.
pub const DIRECTIONS: [(i32, i32, i32); 6] = [
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
    (-1, 0, 0),
    (1, 0, 0),
];
const fn opposite(direction: usize) -> usize {
    direction ^ 1
}
fn offset((x, y, z): SectionPos, direction: usize) -> SectionPos {
    let (dx, dy, dz) = DIRECTIONS[direction];
    (x + dx, y + dy, z + dz)
}

/// `VisibilitySet`: which pairs of section faces open cells connect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisibilitySet(u64);

impl VisibilitySet {
    pub const ALL: Self = Self((1 << 36) - 1);
    pub const NONE: Self = Self(0);

    fn add(&mut self, faces: u8) {
        for a in 0..6 {
            for b in 0..6 {
                if faces & (1 << a) != 0 && faces & (1 << b) != 0 {
                    self.0 |= 1 << (a + b * 6);
                }
            }
        }
    }

    pub fn between(self, a: usize, b: usize) -> bool {
        self.0 & (1 << (a + b * 6)) != 0
    }
}

/// `VisGraph`: flood fills the non-solid cells of one section.
pub struct VisGraph {
    solid: Box<[bool; 4096]>,
    open: usize,
}

impl Default for VisGraph {
    fn default() -> Self {
        Self {
            solid: Box::new([false; 4096]),
            open: 4096,
        }
    }
}

impl VisGraph {
    fn index(x: usize, y: usize, z: usize) -> usize {
        x | z << 4 | y << 8
    }

    /// Marks a solid-render block (`BlockState.isSolidRender`).
    pub fn set_opaque(&mut self, x: usize, y: usize, z: usize) {
        let i = Self::index(x, y, z);
        if !self.solid[i] {
            self.solid[i] = true;
            self.open -= 1;
        }
    }

    pub fn resolve(mut self) -> VisibilitySet {
        let mut set = VisibilitySet::NONE;
        if 4096 - self.open < 256 {
            return VisibilitySet::ALL;
        }
        if self.open == 0 {
            return set;
        }
        for x in 0..16 {
            for y in 0..16 {
                for z in 0..16 {
                    if x != 0 && x != 15 && y != 0 && y != 15 && z != 0 && z != 15 {
                        continue;
                    }
                    let start = Self::index(x, y, z);
                    if !self.solid[start] {
                        set.add(self.flood_fill(start));
                    }
                }
            }
        }
        set
    }

    fn flood_fill(&mut self, start: usize) -> u8 {
        let mut faces = 0u8;
        let mut queue = VecDeque::from([start]);
        self.solid[start] = true;
        while let Some(i) = queue.pop_front() {
            let (x, z, y) = (i & 15, (i >> 4) & 15, (i >> 8) & 15);
            faces |= match x {
                0 => 1 << 4,
                15 => 1 << 5,
                _ => 0,
            } | match y {
                0 => 1 << 0,
                15 => 1 << 1,
                _ => 0,
            } | match z {
                0 => 1 << 2,
                15 => 1 << 3,
                _ => 0,
            };
            let neighbors = [
                (y > 0).then(|| i - 256),
                (y < 15).then(|| i + 256),
                (z > 0).then(|| i - 16),
                (z < 15).then(|| i + 16),
                (x > 0).then(|| i - 1),
                (x < 15).then(|| i + 1),
            ];
            for n in neighbors.into_iter().flatten() {
                if !self.solid[n] {
                    self.solid[n] = true;
                    queue.push_back(n);
                }
            }
        }
        faces
    }
}

/// A section's compiled state (`SectionMesh`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshState {
    Uncompiled,
    /// An all-air section: nothing to build, and every face sees every other.
    Empty,
    Compiled(VisibilitySet),
}

impl MeshState {
    fn faces_can_see_each_other(self, a: usize, b: usize) -> bool {
        match self {
            Self::Uncompiled => false,
            Self::Empty => true,
            Self::Compiled(set) => set.between(a, b),
        }
    }
}

/// One render section (`RenderSection` and its `SectionDirtyState`).
#[derive(Clone, Debug)]
pub struct Section {
    pub mesh: MeshState,
    pub dirty: bool,
    pub dirty_from_player: bool,
    /// Game-clock milliseconds when the first mesh was set (`uploadedTime`).
    pub uploaded_at: Option<u64>,
}

impl Default for Section {
    fn default() -> Self {
        Self {
            mesh: MeshState::Uncompiled,
            dirty: true,
            dirty_from_player: false,
            uploaded_at: None,
        }
    }
}

impl Section {
    /// `RenderSection.getVisibility`: the fade-in from fog color.
    pub fn visibility(&self, now: u64, fade_millis: u64) -> f32 {
        let elapsed = now.saturating_sub(self.uploaded_at.unwrap_or(0));
        if elapsed >= fade_millis {
            1.0
        } else {
            elapsed as f32 / fade_millis as f32
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Node {
    source_directions: u8,
    directions: u8,
}

impl Node {
    fn new(source: Option<usize>) -> Self {
        Self {
            source_directions: source.map_or(0, |d| 1 << d),
            directions: 0,
        }
    }
}

/// The graph's nodes over the view area, as a dense grid: the occlusion
/// search and its distant-section rays look nodes up millions of times.
#[derive(Default)]
struct NodeGrid {
    /// The lowest section the grid covers, and its extent.
    origin: SectionPos,
    size: (i32, i32, i32),
    cells: Vec<Option<Node>>,
}

impl NodeGrid {
    /// Empties the grid and sizes it to the view area around `center`.
    fn reset(&mut self, center: SectionPos, view_distance: i32, min_y: i32, max_y: i32) {
        self.origin = (center.0 - view_distance, min_y, center.2 - view_distance);
        self.size = (2 * view_distance + 1, max_y - min_y + 1, 2 * view_distance + 1);
        let len = (self.size.0 * self.size.1 * self.size.2).max(0) as usize;
        self.cells.clear();
        self.cells.resize(len, None);
    }

    #[inline]
    fn index(&self, (x, y, z): SectionPos) -> Option<usize> {
        let (dx, dy, dz) = (x - self.origin.0, y - self.origin.1, z - self.origin.2);
        if dx < 0 || dy < 0 || dz < 0 || dx >= self.size.0 || dy >= self.size.1 || dz >= self.size.2 {
            return None;
        }
        Some(((dx * self.size.2 + dz) * self.size.1 + dy) as usize)
    }

    #[inline]
    fn get(&self, pos: SectionPos) -> Option<&Node> {
        self.index(pos).and_then(|i| self.cells[i].as_ref())
    }

    #[inline]
    fn get_mut(&mut self, pos: SectionPos) -> Option<&mut Node> {
        self.index(pos).and_then(|i| self.cells[i].as_mut())
    }

    #[inline]
    fn contains_key(&self, pos: &SectionPos) -> bool {
        self.get(*pos).is_some()
    }

    fn insert(&mut self, pos: SectionPos, node: Node) {
        if let Some(i) = self.index(pos) {
            self.cells[i] = Some(node);
        }
    }
}

/// The camera used to cull and order sections.
#[derive(Clone, Copy, Debug)]
pub struct CullCamera {
    pub position: DVec3,
    pub forward: Vec3,
    pub fov_degrees: f32,
    pub aspect: f32,
    /// Rotation in degrees, for vanilla's two-degree frustum refresh.
    pub yaw_degrees: f32,
    pub pitch_degrees: f32,
}

/// `Frustum`, tested against boxes relative to a camera origin that
/// `offset_to_include_camera_cube` may pull backward.
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    planes: [Vec4; 6],
    view_vector: Vec3,
    origin: DVec3,
}

impl Frustum {
    pub fn new(camera: &CullCamera) -> Self {
        // OpenGL clip space, as the matrices vanilla culls with.
        let projection =
            Mat4::perspective_rh_gl(camera.fov_degrees.to_radians(), camera.aspect.max(0.1), 0.05, 1024.0);
        let view = Mat4::look_at_rh(Vec3::ZERO, camera.forward, Vec3::Y);
        let m = projection * view;
        let (r0, r1, r2, r3) = (m.row(0), m.row(1), m.row(2), m.row(3));
        Self {
            planes: [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r3 + r2, r3 - r2],
            view_vector: r2.truncate(),
            origin: camera.position,
        }
    }

    /// `FrustumIntersection.intersectAab`: 2 inside, 1 intersecting, 0 outside.
    fn classify(&self, min: Vec3, max: Vec3) -> u8 {
        let mut inside = true;
        for plane in &self.planes {
            let n = plane.truncate();
            let far = Vec3::new(
                if n.x >= 0.0 { max.x } else { min.x },
                if n.y >= 0.0 { max.y } else { min.y },
                if n.z >= 0.0 { max.z } else { min.z },
            );
            if n.dot(far) + plane.w < 0.0 {
                return 0;
            }
            let near = Vec3::new(
                if n.x >= 0.0 { min.x } else { max.x },
                if n.y >= 0.0 { min.y } else { max.y },
                if n.z >= 0.0 { min.z } else { max.z },
            );
            inside &= n.dot(near) + plane.w >= 0.0;
        }
        if inside {
            2
        } else {
            1
        }
    }

    fn relative(&self, min: DVec3, max: DVec3) -> (Vec3, Vec3) {
        ((min - self.origin).as_vec3(), (max - self.origin).as_vec3())
    }

    pub fn is_visible(&self, min: DVec3, max: DVec3) -> bool {
        let (min, max) = self.relative(min, max);
        self.classify(min, max) != 0
    }

    /// `offsetToFullyIncludeCameraCube`: steps the origin back until the
    /// camera's grid-aligned cube is entirely inside.
    pub fn offset_to_include_camera_cube(mut self, size: f64) -> Self {
        let (low, high) = (
            (self.origin / size).floor() * size,
            (self.origin / size).ceil() * size,
        );
        for _ in 0..64 {
            let (min, max) = self.relative(low, high);
            if self.classify(min, max) == 2 {
                break;
            }
            self.origin -= self.view_vector.as_dvec3() * 4.0;
        }
        self
    }
}

/// What the renderer should do after a frame's section update.
#[derive(Default)]
pub struct Scheduled {
    /// Sections to compile: position and whether it is a recompile.
    pub compile: Vec<(SectionPos, bool)>,
}

/// The occlusion graph: nodes, the non-empty sections it reached (in reach
/// order) and the sections waiting for their chunk.
#[derive(Default)]
struct Graph {
    nodes: NodeGrid,
    reached: Vec<SectionPos>,
    reached_set: HashSet<SectionPos>,
    waiting_for_chunks: HashMap<ChunkPos, Vec<SectionPos>>,
}

/// The view area a graph search runs in.
#[derive(Clone, Copy, Debug)]
struct Area {
    view_distance: i32,
    min_section_y: i32,
    max_section_y: i32,
    camera_section: SectionPos,
}

impl Area {
    fn in_view_area(&self, (x, y, z): SectionPos) -> bool {
        let (cx, _, cz) = self.camera_section;
        (x - cx).abs() <= self.view_distance && (z - cz).abs() <= self.view_distance && (self.min_section_y..=self.max_section_y).contains(&y)
    }

    /// `ChunkTrackingView.isInViewDistance` from the camera section.
    fn in_view_distance(&self, (x, _, z): SectionPos) -> bool {
        let (cx, _, cz) = self.camera_section;
        let dx = i64::from(0.max((x - cx).abs() - 1));
        let dz = i64::from(0.max((z - cz).abs() - 1));
        dx * dx + dz * dz < i64::from(self.view_distance) * i64::from(self.view_distance)
    }
}

/// What a graph search reads, kept densely over the view area as vanilla's
/// `ViewArea` keeps sections: a ring indexed by coordinates modulo the
/// area's width, so the camera moving only rewrites the columns that
/// enter or leave. Each section is one word (mesh state, visibility,
/// emptiness, whether it has a render section); each column records
/// whether its chunk is loaded.
#[derive(Clone, Default)]
struct Ring {
    width: i32,
    min_y: i32,
    height: i32,
    cells: Vec<u64>,
    loaded: Vec<bool>,
}

const RING_COMPILED: u64 = 1 << 62;
const RING_EMPTY_MESH: u64 = 1 << 61;
const RING_EMPTY: u64 = 1 << 60;
/// `Sections` holds a render section here.
const RING_SECTION: u64 = 1 << 59;
const RING_VISIBILITY: u64 = (1 << 36) - 1;

impl Ring {
    fn new(view_distance: i32, min_y: i32, max_y: i32) -> Self {
        let width = 2 * view_distance + 1;
        let height = max_y - min_y + 1;
        Self { width, min_y, height, cells: vec![0; (width * width * height) as usize], loaded: vec![false; (width * width) as usize] }
    }

    /// Becomes a copy of `other`, reusing this ring's memory.
    fn copy_from(&mut self, other: &Ring) {
        self.width = other.width;
        self.min_y = other.min_y;
        self.height = other.height;
        self.cells.clone_from(&other.cells);
        self.loaded.clone_from(&other.loaded);
    }

    fn column(&self, x: i32, z: i32) -> usize {
        (x.rem_euclid(self.width) * self.width + z.rem_euclid(self.width)) as usize
    }

    fn cell(&self, (x, y, z): SectionPos) -> Option<usize> {
        let dy = y - self.min_y;
        (0..self.height).contains(&dy).then(|| self.column(x, z) * self.height as usize + dy as usize)
    }

    fn mesh(&self, pos: SectionPos) -> MeshState {
        let Some(i) = self.cell(pos) else { return MeshState::Uncompiled };
        let word = self.cells[i];
        if word & RING_COMPILED != 0 {
            MeshState::Compiled(VisibilitySet(word & RING_VISIBILITY))
        } else if word & RING_EMPTY_MESH != 0 {
            MeshState::Empty
        } else {
            MeshState::Uncompiled
        }
    }

    fn set_mesh(&mut self, pos: SectionPos, mesh: MeshState) {
        let Some(i) = self.cell(pos) else { return };
        let keep = self.cells[i] & (RING_EMPTY | RING_SECTION);
        self.cells[i] = keep
            | match mesh {
                MeshState::Uncompiled => 0,
                MeshState::Empty => RING_EMPTY_MESH,
                MeshState::Compiled(set) => RING_COMPILED | (set.0 & RING_VISIBILITY),
            };
    }

    fn empty(&self, pos: SectionPos) -> bool {
        self.cell(pos).is_some_and(|i| self.cells[i] & RING_EMPTY != 0)
    }

    fn has_section(&self, pos: SectionPos) -> bool {
        self.cell(pos).is_some_and(|i| self.cells[i] & RING_SECTION != 0)
    }

    fn set_has_section(&mut self, pos: SectionPos) {
        if let Some(i) = self.cell(pos) {
            self.cells[i] |= RING_SECTION;
        }
    }

    fn set_empty(&mut self, pos: SectionPos, empty: bool) {
        if let Some(i) = self.cell(pos) {
            if empty {
                self.cells[i] |= RING_EMPTY;
            } else {
                self.cells[i] &= !RING_EMPTY;
            }
        }
    }

    fn loaded(&self, (x, z): ChunkPos) -> bool {
        self.loaded[self.column(x, z)]
    }

    fn set_loaded(&mut self, (x, z): ChunkPos, loaded: bool) {
        let i = self.column(x, z);
        self.loaded[i] = loaded;
    }

    /// Forgets a column (it left the view area, or its chunk unloaded).
    fn clear_column(&mut self, x: i32, z: i32) {
        let column = self.column(x, z);
        self.loaded[column] = false;
        let start = column * self.height as usize;
        self.cells[start..start + self.height as usize].fill(0);
    }
}

/// What a graph search reads about the sections.
struct Inputs<'a> {
    ring: &'a Ring,
}

impl Inputs<'_> {
    /// The mesh state the search sees: an uncompiled all-air section is
    /// marked empty when the search reaches it.
    fn mesh(&self, pos: SectionPos) -> MeshState {
        let mesh = self.ring.mesh(pos);
        if mesh == MeshState::Uncompiled && self.ring.empty(pos) {
            MeshState::Empty
        } else {
            mesh
        }
    }
}

/// Section changes a search makes, applied to `Sections` afterwards:
/// sections created for new nodes (`ViewArea.getRenderSection`), all-air
/// sections reached, and sections newly reached.
#[derive(Default)]
struct Effects {
    created: Vec<SectionPos>,
    emptied: Vec<SectionPos>,
    added: Vec<SectionPos>,
}

struct FullUpdateJob {
    area: Area,
    camera: CullCamera,
    ring: Ring,
}

struct FullUpdateResult {
    area: Area,
    graph: Graph,
    effects: Effects,
    /// The job's ring, reused for the next snapshot.
    ring: Ring,
}

/// The background thread running full updates, as vanilla schedules them
/// on its executor (`SectionOcclusionGraph.scheduleFullUpdate`).
struct FullUpdateWorker {
    jobs: std::sync::mpsc::Sender<FullUpdateJob>,
    results: std::sync::mpsc::Receiver<FullUpdateResult>,
}

impl FullUpdateWorker {
    fn spawn() -> Self {
        let (jobs, job_rx) = std::sync::mpsc::channel::<FullUpdateJob>();
        let (result_tx, results) = std::sync::mpsc::channel::<FullUpdateResult>();
        std::thread::Builder::new()
            .name("Section graph".into())
            .spawn(move || {
                minecraftoss_core::thread_priority::background();
                while let Ok(job) = job_rx.recv() {
                    let inputs = Inputs { ring: &job.ring };
                    let (graph, effects) = full_update(job.area, &job.camera, &inputs);
                    if result_tx.send(FullUpdateResult { area: job.area, graph, effects, ring: job.ring }).is_err() {
                        return;
                    }
                }
            })
            .expect("section graph thread starts");
        Self { jobs, results }
    }
}

/// Every section around the camera, the occlusion graph and the visible list.
pub struct Sections {
    view_distance: i32,
    min_section_y: i32,
    max_section_y: i32,
    camera_section: SectionPos,
    /// The sections of the view area by ring cell (`Ring::cell`): a cell
    /// holds its in-area position's section once the graph has reached it.
    sections: Vec<Option<Section>>,
    loaded_chunks: HashSet<ChunkPos>,
    empty_sections: HashSet<SectionPos>,
    /// The graph inputs over the view area (see `Ring`).
    ring: Ring,
    graph: Graph,
    to_propagate: Vec<SectionPos>,
    needs_full_update: bool,
    needs_frustum_update: bool,
    last_cell: Option<(i64, i64, i64)>,
    last_fov: Option<i32>,
    last_aspect: Option<i32>,
    last_rotation: Option<(i32, i32)>,
    visible: Vec<SectionPos>,
    /// Full updates run on a background thread unless this is set (tests).
    pub synchronous: bool,
    worker: Option<FullUpdateWorker>,
    /// A background full update is running.
    in_flight: bool,
    /// A ring the last full update returned, for the next snapshot.
    spare_ring: Option<Ring>,
}

impl Sections {
    pub fn new(view_distance: i32, min_section_y: i32, max_section_y: i32) -> Self {
        Self {
            view_distance,
            min_section_y,
            max_section_y,
            camera_section: (0, 0, 0),
            sections: vec![None; ((2 * view_distance + 1) * (2 * view_distance + 1) * (max_section_y - min_section_y + 1)) as usize],
            loaded_chunks: HashSet::default(),
            empty_sections: HashSet::default(),
            ring: Ring::new(view_distance, min_section_y, max_section_y),
            graph: Graph::default(),
            to_propagate: Vec::new(),
            needs_full_update: true,
            needs_frustum_update: false,
            last_cell: None,
            last_fov: None,
            last_aspect: None,
            last_rotation: None,
            visible: Vec::new(),
            synchronous: false,
            worker: None,
            in_flight: false,
            spare_ring: None,
        }
    }

    pub fn view_distance(&self) -> i32 {
        self.view_distance
    }

    pub fn section(&self, pos: SectionPos) -> Option<&Section> {
        self.sections[self.slot(pos)?].as_ref()
    }

    fn section_mut(&mut self, pos: SectionPos) -> Option<&mut Section> {
        let slot = self.slot(pos)?;
        self.sections[slot].as_mut()
    }

    /// The cell of an in-area position.
    fn slot(&self, pos: SectionPos) -> Option<usize> {
        if self.in_view_area(pos) { self.ring.cell(pos) } else { None }
    }

    /// Takes the section out of a position's cell, in the area or not
    /// (a column leaving the area still holds its sections).
    fn take_section(&mut self, pos: SectionPos) -> Option<Section> {
        let cell = self.ring.cell(pos)?;
        self.sections[cell].take()
    }

    /// Diagnostics: one section's state (None when it has no section).
    pub fn debug_section(&self, pos: SectionPos) -> Option<(MeshState, bool, bool, bool, bool)> {
        let section = self.section(pos)?;
        Some((
            section.mesh,
            section.dirty,
            self.graph.nodes.contains_key(&pos),
            self.graph.reached_set.contains(&pos),
            self.visible.contains(&pos),
        ))
    }

    pub fn camera_section(&self) -> SectionPos {
        self.camera_section
    }

    pub fn is_empty_section(&self, pos: SectionPos) -> bool {
        self.empty_sections.contains(&pos)
    }

    /// Graph state for diagnostics.
    pub fn diag(&self) -> String {
        let sections = || self.sections.iter().flatten();
        let compiled = sections().filter(|s| matches!(s.mesh, MeshState::Compiled(_))).count();
        let uncompiled = sections().filter(|s| s.mesh == MeshState::Uncompiled).count();
        let dirty = sections().filter(|s| s.dirty).count();
        format!(
            "graph: camera {:?} sections {} (compiled {compiled}, uncompiled {uncompiled}, dirty {dirty}) loaded chunks {} reached {} waiting chunks {} to_propagate {} in_flight {} needs_full {}",
            self.camera_section,
            sections().count(),
            self.loaded_chunks.len(),
            self.graph.reached.len(),
            self.graph.waiting_for_chunks.len(),
            self.to_propagate.len(),
            self.in_flight,
            self.needs_full_update,
        )
    }

    /// Frustum-visible sections from the last update, nearest first.
    pub fn visible(&self) -> &[SectionPos] {
        &self.visible
    }

    fn area(&self) -> Area {
        Area {
            view_distance: self.view_distance,
            min_section_y: self.min_section_y,
            max_section_y: self.max_section_y,
            camera_section: self.camera_section,
        }
    }

    fn in_view_area(&self, pos: SectionPos) -> bool {
        self.area().in_view_area(pos)
    }

    /// A chunk arrived: its sections and its neighbors' are dirty
    /// (`ClientPacketListener.enableChunkLight`).
    pub fn chunk_loaded(&mut self, chunk: ChunkPos, empty: impl IntoIterator<Item = i32>) {
        self.loaded_chunks.insert(chunk);
        let in_area = self.in_view_area((chunk.0, self.min_section_y, chunk.1));
        if in_area {
            self.ring.set_loaded(chunk, true);
        }
        for y in empty {
            self.empty_sections.insert((chunk.0, y, chunk.1));
            if in_area {
                self.ring.set_empty((chunk.0, y, chunk.1), true);
            }
        }
        self.set_range_dirty(chunk, false);
        if let Some(waiting) = self.graph.waiting_for_chunks.remove(&chunk) {
            self.to_propagate.extend(waiting);
        }
    }

    /// A chunk was forgotten. Returns its sections that had meshes.
    pub fn chunk_unloaded(&mut self, chunk: ChunkPos) -> Vec<SectionPos> {
        self.loaded_chunks.remove(&chunk);
        let in_area = self.in_view_area((chunk.0, self.min_section_y, chunk.1));
        if in_area {
            self.ring.clear_column(chunk.0, chunk.1);
        }
        let mut dropped = Vec::new();
        for y in self.min_section_y..=self.max_section_y {
            let pos = (chunk.0, y, chunk.1);
            self.empty_sections.remove(&pos);
            if let Some(section) = in_area.then(|| self.take_section(pos)).flatten() {
                if matches!(section.mesh, MeshState::Compiled(_)) {
                    dropped.push(pos);
                }
            }
        }
        self.needs_full_update = true;
        dropped
    }

    /// Marks the sections of a chunk and its eight neighbors dirty.
    pub fn set_range_dirty(&mut self, (cx, cz): ChunkPos, from_player: bool) {
        for x in cx - 1..=cx + 1 {
            for z in cz - 1..=cz + 1 {
                for y in self.min_section_y..=self.max_section_y {
                    self.set_dirty((x, y, z), from_player);
                }
            }
        }
    }

    /// `SectionUpdateTracker.setDirty`.
    pub fn set_dirty(&mut self, pos: SectionPos, from_player: bool) {
        if let Some(section) = self.section_mut(pos) {
            section.dirty = true;
            section.dirty_from_player |= from_player;
        }
    }

    /// An edit changed whether a section holds only air.
    pub fn set_empty(&mut self, pos: SectionPos, empty: bool) {
        if self.in_view_area(pos) {
            self.ring.set_empty(pos, empty);
        }
        if empty {
            self.empty_sections.insert(pos);
        } else if self.empty_sections.remove(&pos) && self.section(pos).is_some() {
            self.to_propagate.push(pos);
        }
    }

    /// A compile finished (`setSectionMesh` then `schedulePropagationFrom`).
    pub fn compiled(&mut self, pos: SectionPos, visibility: VisibilitySet, now: u64) {
        if let Some(section) = self.section_mut(pos) {
            section.mesh = MeshState::Compiled(visibility);
            section.uploaded_at.get_or_insert(now);
            let mesh = section.mesh;
            self.ring.set_mesh(pos, mesh);
            self.to_propagate.push(pos);
        }
    }

    /// Positions the view area on the camera. Returns sections that left it
    /// and had meshes to release.
    pub fn reposition(&mut self, camera: &CullCamera) -> Vec<SectionPos> {
        let block = camera.position.floor().as_ivec3();
        let section = (block.x >> 4, block.y >> 4, block.z >> 4);
        let mut dropped = Vec::new();
        if section != self.camera_section {
            let old = self.camera_section;
            self.camera_section = section;
            let (cx, cz, vd) = (section.0, section.2, self.view_distance);
            if (section.0, section.2) != (old.0, old.2) {
                // Only columns of the old area can have left the new one.
                for x in old.0 - vd..=old.0 + vd {
                    for z in old.2 - vd..=old.2 + vd {
                        if (x - cx).abs() <= vd && (z - cz).abs() <= vd {
                            continue;
                        }
                        self.ring.clear_column(x, z);
                        for y in self.min_section_y..=self.max_section_y {
                            if let Some(state) = self.take_section((x, y, z)) {
                                if matches!(state.mesh, MeshState::Compiled(_)) {
                                    dropped.push((x, y, z));
                                }
                            }
                        }
                    }
                }
                // Columns entering the area take their current state.
                for x in cx - vd..=cx + vd {
                    for z in cz - vd..=cz + vd {
                        if (x - old.0).abs() <= vd && (z - old.2).abs() <= vd {
                            continue;
                        }
                        self.ring.clear_column(x, z);
                        if self.loaded_chunks.contains(&(x, z)) {
                            self.ring.set_loaded((x, z), true);
                        }
                        // Their cells were emptied as the columns sharing them left.
                        for y in self.min_section_y..=self.max_section_y {
                            let pos = (x, y, z);
                            if self.empty_sections.contains(&pos) {
                                self.ring.set_empty(pos, true);
                            }
                        }
                    }
                }
            }
        }
        dropped
    }

    /// Applies what a search did to the sections.
    fn apply_effects(&mut self, effects: &Effects, now: u64) {
        for &pos in &effects.created {
            if let Some(slot) = self.slot(pos) {
                self.sections[slot].get_or_insert_with(Section::default);
                self.ring.set_has_section(pos);
            }
        }
        for &pos in &effects.emptied {
            let Some(slot) = self.slot(pos) else { continue };
            if let Some(section) = self.sections[slot].as_mut() {
                if section.mesh == MeshState::Uncompiled && self.empty_sections.contains(&pos) {
                    section.mesh = MeshState::Empty;
                    section.uploaded_at.get_or_insert(now);
                    self.ring.set_mesh(pos, MeshState::Empty);
                }
            }
        }
    }

    /// Installs a finished full update: sections compiled or chunks loaded
    /// while it ran are propagated through the new graph.
    fn install(&mut self, result: FullUpdateResult, now: u64) {
        let _ = result.area;
        self.spare_ring = Some(result.ring);
        self.graph = result.graph;
        self.apply_effects(&result.effects, now);
        let loaded: Vec<ChunkPos> = self.graph.waiting_for_chunks.keys().copied().filter(|c| self.loaded_chunks.contains(c)).collect();
        for chunk in loaded {
            if let Some(waiting) = self.graph.waiting_for_chunks.remove(&chunk) {
                self.to_propagate.extend(waiting);
            }
        }
        self.needs_frustum_update = true;
    }

    /// One frame: graph updates, the visible list, then the sections to
    /// compile. `ready(pos)` is `hasAllNeighbors` for a first compile.
    pub fn update(&mut self, camera: &CullCamera, now: u64, ready: impl Fn(SectionPos) -> bool) -> Scheduled {
        let cell = (
            (camera.position.x / 8.0).floor() as i64,
            (camera.position.y / 8.0).floor() as i64,
            (camera.position.z / 8.0).floor() as i64,
        );
        let fov = camera.fov_degrees as i32;
        let aspect=(camera.aspect*1000.) as i32;
        if self.last_aspect!=Some(aspect) {self.needs_full_update=true;self.needs_frustum_update=true;}
        self.last_aspect=Some(aspect);
        if self.last_cell != Some(cell) || self.last_fov != Some(fov) {
            self.needs_full_update = true;
        }
        self.last_cell = Some(cell);
        self.last_fov = Some(fov);
        // A finished background update replaces the graph.
        if self.in_flight {
            let finished = self.worker.as_ref().and_then(|w| w.results.try_iter().last());
            if let Some(result) = finished {
                let _span = span("  sections.install");
                self.in_flight = false;
                self.install(result, now);
            }
        }
        if self.needs_full_update && !self.in_flight {
            self.needs_full_update = false;
            let area = self.area();
            if self.synchronous {
                let _span = span("  sections.full_update");
                self.to_propagate.clear();
                let inputs = Inputs { ring: &self.ring };
                let (graph, effects) = full_update(area, camera, &inputs);
                self.graph = graph;
                self.apply_effects(&effects, now);
                self.needs_frustum_update = true;
            } else {
                let _span = span("  sections.snapshot");
                let ring = match self.spare_ring.take() {
                    Some(mut ring) => {
                        ring.copy_from(&self.ring);
                        ring
                    }
                    None => self.ring.clone(),
                };
                let job = FullUpdateJob { area, camera: *camera, ring };
                let worker = self.worker.get_or_insert_with(FullUpdateWorker::spawn);
                if worker.jobs.send(job).is_ok() {
                    self.in_flight = true;
                }
            }
        }
        // Propagation waits while a full update runs; it replays on the new graph.
        if !self.in_flight && !self.to_propagate.is_empty() {
            let mut queue: VecDeque<SectionPos> = VecDeque::new();
            for pos in std::mem::take(&mut self.to_propagate) {
                if self.graph.nodes.contains_key(&pos) {
                    queue.push_back(pos);
                }
            }
            let frustum = Frustum::new(camera).offset_to_include_camera_cube(8.0);
            let _span = span("  sections.propagate");
            let mut effects = Effects::default();
            let area = self.area();
            {
                let inputs = Inputs { ring: &self.ring };
                run_updates(&mut self.graph, area, &inputs, camera, queue, &mut effects);
            }
            self.apply_effects(&effects, now);
            if effects.added.iter().any(|&pos| is_section_visible(&frustum, pos)) {
                self.needs_frustum_update = true;
            }
        }
        let rotation = (
            (camera.pitch_degrees / 2.0).floor() as i32,
            (camera.yaw_degrees / 2.0).floor() as i32,
        );
        if std::mem::take(&mut self.needs_frustum_update) || self.last_rotation != Some(rotation) {
            self.last_rotation = Some(rotation);
            let _span = span("  sections.apply_frustum");
            self.apply_frustum(camera);
        }
        let mut scheduled = Scheduled::default();
        for player_edit in [true,false] {
        for &pos in &self.visible {
            if scheduled.compile.len() >= 8 { break; }
            let Some(slot) = self.slot(pos) else { continue };
            let Some(section) = self.sections[slot].as_mut() else {
                continue;
            };
            if section.dirty_from_player!=player_edit {continue;}
            let uncompiled = section.mesh == MeshState::Uncompiled;
            if !section.dirty || (uncompiled && !ready(pos)) {
                continue;
            }
            section.dirty = false;
            section.dirty_from_player = false;
            scheduled.compile.push((pos, !uncompiled));
        }
        }
        scheduled
    }

    fn apply_frustum(&mut self, camera: &CullCamera) {
        let frustum = Frustum::new(camera).offset_to_include_camera_cube(8.0);
        let eye = camera.position;
        let mut visible: Vec<(f64, SectionPos)> = self
            .graph
            .reached
            .iter()
            .copied()
            .filter(|&pos| self.graph.nodes.contains_key(&pos) && is_section_visible(&frustum, pos))
            .map(|pos| (section_distance_sq(pos, eye), pos))
            .collect();
        // Stable: equal distances keep the order the graph reached them.
        visible.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.visible.clear();
        self.visible.extend(visible.into_iter().map(|(_, pos)| pos));
    }
}

/// `scheduleFullUpdate`'s search: a new graph from the camera section.
fn full_update(area: Area, camera: &CullCamera, inputs: &Inputs) -> (Graph, Effects) {
    let mut graph = Graph::default();
    let mut effects = Effects::default();
    let camera_section = area.camera_section;
    graph.nodes.reset(camera_section, area.view_distance, area.min_section_y, area.max_section_y);
    let mut queue = VecDeque::new();
    if area.in_view_area(camera_section) {
        if !inputs.ring.has_section(camera_section) {
            effects.created.push(camera_section);
        }
        graph.nodes.insert(camera_section, Node::new(None));
        queue.push_back(camera_section);
    } else {
        // Above or below the world: start from the nearest layer,
        // nearest columns first.
        let below = camera_section.1 < area.min_section_y;
        let y = if below { area.min_section_y } else { area.max_section_y };
        let source = if below { 1 } else { 0 };
        let vd = area.view_distance;
        let mut start = Vec::new();
        for dx in -vd..=vd {
            for dz in -vd..=vd {
                let pos = (camera_section.0 + dx, y, camera_section.2 + dz);
                if !area.in_view_distance(pos) {
                    continue;
                }
                let mut node = Node::new(Some(source));
                node.directions |= 1 << source;
                if dx > 0 {
                    node.directions |= 1 << 5;
                } else if dx < 0 {
                    node.directions |= 1 << 4;
                }
                if dz > 0 {
                    node.directions |= 1 << 3;
                } else if dz < 0 {
                    node.directions |= 1 << 2;
                }
                start.push((pos, node));
            }
        }
        let eye = camera.position.floor();
        start.sort_by(|a, b| section_distance_sq(a.0, eye).total_cmp(&section_distance_sq(b.0, eye)));
        for (pos, node) in start {
            if !inputs.ring.has_section(pos) {
                effects.created.push(pos);
            }
            graph.nodes.insert(pos, node);
            queue.push_back(pos);
        }
    }
    run_updates(&mut graph, area, inputs, camera, queue, &mut effects);
    (graph, effects)
}

/// `runUpdates`: breadth-first spread with smart culling.
fn run_updates(graph: &mut Graph, area: Area, inputs: &Inputs, camera: &CullCamera, mut queue: VecDeque<SectionPos>, effects: &mut Effects) {
    let camera_section = area.camera_section;
    let eye = camera.position;
    let camera_center = DVec3::new(
        f64::from(camera_section.0 * 16 + 8),
        f64::from(camera_section.1 * 16 + 8),
        f64::from(camera_section.2 * 16 + 8),
    );
    const CULL_SECTIONS: i32 = 60 >> 4;
    let diagonal = (3.0f64.sqrt() * 16.0).ceil();
    while let Some(pos) = queue.pop_front() {
        let Some(&node) = graph.nodes.get(pos) else { continue };
        if !inputs.ring.loaded((pos.0, pos.2)) {
            graph.waiting_for_chunks.entry((pos.0, pos.2)).or_default().push(pos);
            continue;
        }
        if !inputs.ring.empty(pos) {
            if graph.reached_set.insert(pos) {
                graph.reached.push(pos);
                effects.added.push(pos);
            }
        } else if inputs.ring.mesh(pos) == MeshState::Uncompiled {
            effects.emptied.push(pos);
        }
        let mesh = inputs.mesh(pos);
        let distant = (pos.0 - camera_section.0).abs() > CULL_SECTIONS
            || (pos.1 - camera_section.1).abs() > CULL_SECTIONS
            || (pos.2 - camera_section.2).abs() > CULL_SECTIONS;
        for direction in 0..6 {
            let next = offset(pos, direction);
            if !area.in_view_distance(next) || (camera_section.1 - next.1).abs() > area.view_distance || !area.in_view_area(next) {
                continue;
            }
            if node.directions & (1 << opposite(direction)) != 0 {
                continue;
            }
            if node.source_directions != 0
                && !(0..6).any(|source| node.source_directions & (1 << source) != 0 && mesh.faces_can_see_each_other(opposite(source), direction))
            {
                continue;
            }
            if distant && !ray_visible(graph, area, pos, direction, eye, camera_center, diagonal) {
                continue;
            }
            if let Some(existing) = graph.nodes.get_mut(next) {
                existing.source_directions |= 1 << direction;
                continue;
            }
            let mut child = Node::new(Some(direction));
            child.directions |= node.directions | 1 << direction;
            if !inputs.ring.has_section(next) {
                effects.created.push(next);
            }
            graph.nodes.insert(next, child);
            queue.push_back(next);
        }
    }
}

/// The distant-section check: march from the section's near corner
/// toward the camera; every section passed must already be in the graph.
fn ray_visible(graph: &Graph, area: Area, pos: SectionPos, direction: usize, eye: DVec3, center: DVec3, diagonal: f64) -> bool {
    let origin = DVec3::new(f64::from(pos.0 * 16), f64::from(pos.1 * 16), f64::from(pos.2 * 16));
    let axis = direction / 2;
    let toward = |a: usize, c: f64, o: f64| if axis == a { c > o } else { c < o };
    let max = (
        toward(2, center.x, origin.x),
        toward(0, center.y, origin.y),
        toward(1, center.z, origin.z),
    );
    let mut check = origin
        + DVec3::new(
            if max.0 { 16.0 } else { 0.0 },
            if max.1 { 16.0 } else { 0.0 },
            if max.2 { 16.0 } else { 0.0 },
        );
    let step = (eye - check).normalize() * diagonal;
    let (min_y, max_y) = (f64::from(area.min_section_y * 16), f64::from((area.max_section_y + 1) * 16));
    while check.distance_squared(eye) > 3600.0 {
        check += step;
        if check.y > max_y || check.y < min_y {
            break;
        }
        let block = check.floor().as_ivec3();
        let at = (block.x >> 4, block.y >> 4, block.z >> 4);
        if !(area.in_view_area(at) && graph.nodes.contains_key(&at)) {
            return false;
        }
    }
    true
}

fn section_min((x, y, z): SectionPos) -> DVec3 {
    DVec3::new(f64::from(x * 16), f64::from(y * 16), f64::from(z * 16))
}

fn is_section_visible(frustum: &Frustum, pos: SectionPos) -> bool {
    let min = section_min(pos);
    frustum.is_visible(min, min + DVec3::splat(16.0))
}

/// Squared distance from the camera to a section's center
/// (`SectionTaskDynamicQueue` ordering).
pub fn section_distance_sq(pos: SectionPos, eye: DVec3) -> f64 {
    (section_min(pos) + DVec3::splat(8.0)).distance_squared(eye)
}

/// `SectionTaskDynamicQueue.poll`: the nearest first compile, unless a
/// nearer recompile is waiting; up to two recompiles may jump ahead in a row.
pub struct CompileQueue<T> {
    tasks: Vec<(SectionPos, bool, T)>,
    recompile_quota: u8,
}

impl<T> Default for CompileQueue<T> {
    fn default() -> Self {
        Self {
            tasks: Vec::new(),
            recompile_quota: 2,
        }
    }
}

impl<T> CompileQueue<T> {
    pub fn push(&mut self, pos: SectionPos, recompile: bool, task: T) {
        self.tasks.push((pos, recompile, task));
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn retain(&mut self, mut keep: impl FnMut(SectionPos, &T) -> bool) {
        self.tasks.retain(|(pos, _, task)| keep(*pos, task));
    }

    pub fn poll(&mut self, eye: DVec3) -> Option<(SectionPos, T)> {
        let mut best_initial: Option<(usize, f64)> = None;
        let mut best_recompile: Option<(usize, f64)> = None;
        for (i, (pos, recompile, _)) in self.tasks.iter().enumerate() {
            let d = section_distance_sq(*pos, eye);
            let best = if *recompile { &mut best_recompile } else { &mut best_initial };
            if best.is_none_or(|(_, b)| d < b) {
                *best = Some((i, d));
            }
        }
        let index = match (best_initial, best_recompile) {
            (initial, Some((r, rd)))
                if initial.is_none_or(|(_, id)| self.recompile_quota > 0 && rd < id) =>
            {
                self.recompile_quota = self.recompile_quota.saturating_sub(1);
                r
            }
            (Some((i, _)), _) => {
                self.recompile_quota = 2;
                i
            }
            (None, None) => return None,
            (None, Some((r, _))) => r,
        };
        // An ordered removal keeps vanilla's tie-breaking between equal distances.
        let (pos, _, task) = self.tasks.remove(index);
        Some((pos, task))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_and_closed_sections_resolve_like_vis_graph() {
        assert_eq!(VisGraph::default().resolve(), VisibilitySet::ALL);
        let mut solid = VisGraph::default();
        for x in 0..16 {
            for y in 0..16 {
                for z in 0..16 {
                    solid.set_opaque(x, y, z);
                }
            }
        }
        assert_eq!(solid.resolve(), VisibilitySet::NONE);
        // A horizontal slab splits the section: up and down no longer connect.
        let mut slab = VisGraph::default();
        for x in 0..16 {
            for z in 0..16 {
                slab.set_opaque(x, 8, z);
            }
        }
        let set = slab.resolve();
        assert!(!set.between(0, 1));
        assert!(set.between(1, 2) && set.between(0, 4));
        assert!(set.between(2, 3), "north and south connect above the slab");
    }

    #[test]
    fn graph_spreads_only_through_compiled_or_empty_sections() {
        let camera = CullCamera {
            position: DVec3::new(8.0, 100.0, 8.0),
            forward: Vec3::new(0.0, -0.2, -1.0).normalize(),
            fov_degrees: 70.0,
            aspect: 16.0 / 9.0,
            yaw_degrees: 0.0,
            pitch_degrees: 10.0,
        };
        let mut sections = Sections::new(4, -4, 19);
        sections.synchronous = true;
        sections.reposition(&camera);
        for x in -5..=5 {
            for z in -5..=5 {
                // Terrain up to section 4, air above.
                sections.chunk_loaded((x, z), 5..=19);
            }
        }
        sections.update(&camera, 0, |_| true);
        // Air spreads everywhere in view, within the view distance
        // vertically too; the first solid layer below it is reached but
        // blocks further spread until compiled.
        assert!(sections.graph.nodes.contains_key(&(3, 10, -3)));
        assert!(!sections.graph.nodes.contains_key(&(0, 11, 0)));
        assert!(sections.graph.nodes.contains_key(&(0, 4, -2)));
        assert!(!sections.graph.nodes.contains_key(&(0, 3, -2)));
        let mut queued=std::collections::HashSet::new();
        for _ in 0..100 {
            let scheduled=sections.update(&camera,0,|_|true);
            assert!(scheduled.compile.len()<=8);
            if scheduled.compile.is_empty() {break;}
            for (pos,_) in scheduled.compile {assert!(queued.insert(pos),"dirty sections must not be scheduled twice");}
        }
        assert!(sections.update(&camera,0,|_|true).compile.is_empty());
        sections.compiled((0, 4, -2), VisibilitySet::ALL, 10);
        sections.update(&camera, 10, |_| true);
        assert!(sections.graph.nodes.contains_key(&(0, 3, -2)));
    }

    #[test]
    fn background_full_update_matches_the_synchronous_one() {
        let camera = CullCamera {
            position: DVec3::new(8.0, 100.0, 8.0),
            forward: Vec3::new(0.0, -0.2, -1.0).normalize(),
            fov_degrees: 70.0,
            aspect: 16.0 / 9.0,
            yaw_degrees: 30.0,
            pitch_degrees: 10.0,
        };
        let build = |synchronous: bool| {
            let mut sections = Sections::new(6, -4, 19);
            sections.synchronous = synchronous;
            sections.reposition(&camera);
            for x in -7..=7 {
                for z in -7..=7 {
                    sections.chunk_loaded((x, z), 5..=19);
                }
            }
            for _ in 0..1000 {
                sections.update(&camera, 0, |_| true);
                if !sections.in_flight && !sections.needs_full_update {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            for x in -6..=6 {
                sections.compiled((x, 4, -2), VisibilitySet::ALL, 5);
            }
            for _ in 0..1000 {
                sections.update(&camera, 10, |_| true);
                if !sections.in_flight && sections.to_propagate.is_empty() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let mut reached = sections.graph.reached.clone();
            reached.sort_unstable();
            (reached, sections.visible().to_vec())
        };
        assert_eq!(build(true), build(false));
    }

    #[test]
    fn moving_the_view_area_keeps_the_graph_inputs() {
        let camera_at = |x: f64| CullCamera {
            position: DVec3::new(x, 100.0, 8.0),
            forward: Vec3::new(0.0, -0.2, -1.0).normalize(),
            fov_degrees: 70.0,
            aspect: 16.0 / 9.0,
            yaw_degrees: 0.0,
            pitch_degrees: 10.0,
        };
        let load = |sections: &mut Sections| {
            for x in -12..=12 {
                for z in -8..=8 {
                    sections.chunk_loaded((x, z), 5..=19);
                }
            }
        };
        let (start, end) = (camera_at(8.0), camera_at(8.0 + 16.0 * 4.0));
        // Loaded around the start, then moved four chunks east.
        let mut moved = Sections::new(5, -4, 19);
        moved.synchronous = true;
        moved.reposition(&start);
        load(&mut moved);
        moved.update(&start, 0, |_| true);
        moved.compiled((1, 4, -2), VisibilitySet::ALL, 1);
        moved.compiled((5, 4, -2), VisibilitySet::ALL, 1);
        moved.update(&start, 2, |_| true);
        moved.reposition(&end);
        moved.update(&end, 3, |_| true);
        // Loaded and compiled at the end position directly.
        let mut fresh = Sections::new(5, -4, 19);
        fresh.synchronous = true;
        fresh.reposition(&end);
        load(&mut fresh);
        fresh.update(&end, 0, |_| true);
        fresh.compiled((1, 4, -2), VisibilitySet::ALL, 1);
        fresh.compiled((5, 4, -2), VisibilitySet::ALL, 1);
        fresh.update(&end, 3, |_| true);
        let reached = |s: &Sections| {
            let mut r = s.graph.reached.clone();
            r.sort_unstable();
            r
        };
        assert_eq!(reached(&moved), reached(&fresh));
        assert_eq!(moved.visible(), fresh.visible());
    }

    #[test]
    fn queue_prefers_near_initial_compiles_with_a_recompile_quota() {
        let eye = DVec3::new(8.0, 8.0, 8.0);
        let mut queue = CompileQueue::default();
        queue.push((5, 0, 0), false, 'a');
        queue.push((2, 0, 0), false, 'b');
        queue.push((0, 0, 1), true, 'c');
        queue.push((0, 1, 0), true, 'd');
        queue.push((0, 0, 2), true, 'e');
        // Nearer recompiles jump ahead twice, then the nearest first compile.
        assert_eq!(queue.poll(eye).unwrap().1, 'c');
        assert_eq!(queue.poll(eye).unwrap().1, 'd');
        assert_eq!(queue.poll(eye).unwrap().1, 'b');
        assert_eq!(queue.poll(eye).unwrap().1, 'e');
        assert_eq!(queue.poll(eye).unwrap().1, 'a');
    }
}
