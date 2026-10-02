//! Block and sky light for a chunk (26.3 `LightEngine`, `BlockLightEngine`,
//! `SkyLightEngine`, `ChunkSkyLightSources`, `LayerLightSectionStorage`).
//!
//! Vanilla propagates light incrementally with increase and decrease
//! queues. Once all updates have run, the stored levels are the fixed
//! point this module computes directly: sources at their emission (block
//! light) or 15 (sky light at and above each column's lowest source), and
//! each neighbour taking `level - max(1, dampening)` unless the two faces'
//! occlusion shapes together cover the face between them. Light only lives
//! in sections vanilla stores data for: non-empty sections and every
//! section touching one (26 neighbours), one section beyond the build
//! range included. A level travels at most 14 blocks, so a chunk's light
//! is fully determined by its 3x3 neighbourhood.
//!
//! Vanilla's `propagateFromEmptySections` also copies a sky level into the
//! stored sections beside a gap of unstored sections; that is applied as a
//! further edge of the fixed point. Known differences: vanilla fills there
//! only when the propagation raised the neighbour, new sky sections start
//! from the section above, and blocks a neighbour's decoration places while
//! a chunk's light is being initialized never reach its sky sources.

use crate::block::{flags, BlockRegistry, FaceShape};
use crate::chunk::Chunk;
use crate::pos::{Axis, Direction};
use crate::BlockStateId;

/// Light-related properties of every block state.
pub struct LightTable {
    opacity: Vec<u8>,
    emission: Vec<u8>,
    dampening: Vec<u8>,
    /// `LightEngine.isEmptyShape`: occlusion shapes count as empty.
    empty_shape: Vec<bool>,
}

impl LightTable {
    pub fn new(blocks: &BlockRegistry) -> Self {
        let count = blocks.state_count();
        let mut table = Self { opacity: Vec::with_capacity(count), emission: Vec::with_capacity(count), dampening: Vec::with_capacity(count), empty_shape: Vec::with_capacity(count) };
        for i in 0..count {
            let state = blocks.state(BlockStateId(i as u16));
            table.opacity.push(state.light_dampening.max(1));
            table.emission.push(state.light_emission);
            table.dampening.push(state.light_dampening);
            table.empty_shape.push(!state.has(flags::CAN_OCCLUDE) || !state.has(flags::USE_SHAPE_FOR_LIGHT_OCCLUSION));
        }
        table
    }

    fn occlusion_face<'a>(&self, blocks: &'a BlockRegistry, state: BlockStateId, direction: Direction) -> &'a FaceShape {
        if self.empty_shape[usize::from(state.0)] {
            &FaceShape::Empty
        } else {
            blocks.face_shape(state, direction)
        }
    }

    /// `LightEngine.shapeOccludes`: whether light cannot pass from `from`
    /// into its neighbour `to` in `direction`.
    pub fn shape_occludes(&self, blocks: &BlockRegistry, from: BlockStateId, to: BlockStateId, direction: Direction) -> bool {
        if self.empty_shape[usize::from(from.0)] && self.empty_shape[usize::from(to.0)] {
            return false;
        }
        face_shape_occludes(self.occlusion_face(blocks, from, direction), self.occlusion_face(blocks, to, direction.opposite()), direction.axis())
    }

    /// `ChunkSkyLightSources.isEdgeOccluded` between vertically adjacent states.
    fn edge_occluded(&self, blocks: &BlockRegistry, top: BlockStateId, bottom: BlockStateId) -> bool {
        self.dampening[usize::from(bottom.0)] != 0
            || face_shape_occludes(self.occlusion_face(blocks, top, Direction::Down), self.occlusion_face(blocks, bottom, Direction::Up), Axis::Y)
    }
}

/// `Shapes.faceShapeOccludes`: whether two face slices together cover the
/// whole face. The shapes are slices along `axis`, so only the other two
/// axes matter.
fn face_shape_occludes(a: &FaceShape, b: &FaceShape, axis: Axis) -> bool {
    match (a, b) {
        (FaceShape::Full, _) | (_, FaceShape::Full) => true,
        (FaceShape::Empty, FaceShape::Empty) => false,
        _ => {
            let (u, v) = match axis {
                Axis::X => (1, 2),
                Axis::Y => (0, 2),
                Axis::Z => (0, 1),
            };
            let mut rects = Vec::new();
            for shape in [a, b] {
                if let FaceShape::Boxes(boxes) = shape {
                    rects.extend(boxes.iter().map(|b| [b[u], b[v], b[u + 3], b[v + 3]]));
                }
            }
            let mut us: Vec<f64> = vec![0.0, 1.0];
            let mut vs: Vec<f64> = vec![0.0, 1.0];
            for r in &rects {
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
    }
}

/// One chunk's light, section by section from one section below the build
/// range to one above, as vanilla `DataLayer` bytes (`y << 8 | z << 4 | x`,
/// low nibble first). `None` where vanilla stores no data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChunkLight {
    /// Section Y of the first entry.
    pub min_section: i32,
    /// `ChunkAccess.isLightCorrect` (saved as `isLightOn`): the light is
    /// complete, not an intermediate state of a chunk still generating.
    pub correct: bool,
    pub block: Vec<Option<Box<[u8; 2048]>>>,
    pub sky: Vec<Option<Box<[u8; 2048]>>>,
}

impl ChunkLight {
    fn nibble(data: &[u8; 2048], x: i32, y: i32, z: i32) -> i32 {
        let index = ((y & 15) << 8 | (z & 15) << 4 | (x & 15)) as usize;
        i32::from(data[index >> 1] >> ((index & 1) * 4) & 15)
    }

    /// `SkyLightEngine.getLightValue` for a block of this chunk: below the
    /// stored sections 0; where a section stores nothing, the bottom layer
    /// of the next stored section above, or 15 above them all.
    pub fn sky_at(&self, x: i32, y: i32, z: i32) -> i32 {
        let mut section = (y >> 4) - self.min_section;
        if section < 0 {
            return 0;
        }
        let mut y = y;
        while (section as usize) < self.sky.len() {
            if let Some(data) = &self.sky[section as usize] {
                return Self::nibble(data, x, y, z);
            }
            section += 1;
            y = 0;
        }
        15
    }

    /// Block light for a block of this chunk: 0 where nothing is stored.
    pub fn block_at(&self, x: i32, y: i32, z: i32) -> i32 {
        let section = (y >> 4) - self.min_section;
        if section < 0 {
            return 0;
        }
        match self.block.get(section as usize).and_then(|d| d.as_ref()) {
            Some(data) => Self::nibble(data, x, y, z),
            None => 0,
        }
    }
}

const SIZE: usize = 48;

/// Blocks from a region coordinate to the center chunk's 16..32 span.
const CENTER_DISTANCE: [usize; SIZE] = {
    let mut d = [0; SIZE];
    let mut i = 0;
    while i < SIZE {
        d[i] = if i < 16 { 16 - i } else if i >= 32 { i - 31 } else { 0 };
        i += 1;
    }
    d
};

/// Profiling: summed time per light solve phase over the whole process.
pub mod profile {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;
    pub const STATES: usize = 0;
    pub const BLOCK: usize = 1;
    pub const SKY_SEED: usize = 2;
    pub const SKY_PROPAGATE: usize = 3;
    pub const LAYERS: usize = 4;
    const NAMES: [&str; 5] = ["light: states", "light: block", "light: sky seeds", "light: sky propagation", "light: data layers"];
    static MICROS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
    static COUNTS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
    pub fn add(phase: usize, since: Instant) {
        MICROS[phase].fetch_add(since.elapsed().as_micros() as u64, Ordering::Relaxed);
        COUNTS[phase].fetch_add(1, Ordering::Relaxed);
    }
    pub fn reset() {
        for i in 0..NAMES.len() {
            MICROS[i].store(0, Ordering::Relaxed);
            COUNTS[i].store(0, Ordering::Relaxed);
        }
    }
    pub fn report() -> String {
        let mut out = String::new();
        for (i, name) in NAMES.iter().enumerate() {
            let (micros, count) = (MICROS[i].load(Ordering::Relaxed), COUNTS[i].load(Ordering::Relaxed));
            out += &format!("    {name:32} {:8.2}s {count:7} x {:8.0} us\n", micros as f64 / 1e6, micros.checked_div(count).unwrap_or(0));
        }
        out
    }
}

/// Lights the center of a 3x3 of chunks (`(dz + 1) * 3 + (dx + 1)` order).
/// `non_empty(dx, dz, section_y)` answers for chunks up to two away, which
/// decide which sections store light; for chunks in the 3x3 it should
/// agree with their sections.
pub fn light_chunk(blocks: &BlockRegistry, table: &LightTable, chunks: [&Chunk; 9], non_empty: impl Fn(i32, i32, i32) -> bool, sky_light: bool) -> ChunkLight {
    let started = std::time::Instant::now();
    let center = chunks[4];
    let build_sections = center.sections().len();
    // Only sections next to a non-empty one (in chunks up to two away)
    // store light; the region covers just their range.
    let (mut lowest, mut highest) = (usize::MAX, 0);
    for dz in -2..=2 {
        for dx in -2..=2 {
            for b in 0..build_sections {
                if non_empty(dx, dz, center.min_section_y() + b as i32) {
                    lowest = lowest.min(b);
                    highest = highest.max(b);
                }
            }
        }
    }
    // Region section indices (the build range starts at 1).
    let crop = if lowest == usize::MAX { (0, build_sections + 2) } else { (lowest, highest + 3 - lowest) };
    let height = crop.1 * 16;
    let index = |x: usize, y: usize, z: usize| (y * SIZE + z) * SIZE + x;
    // Block states over the whole area; outside the build range, air.
    // Rows of 16 X run the same way in sections and here, so they copy
    // whole; emitters are collected on the way.
    let mut states = vec![BlockStateId::AIR; SIZE * SIZE * height];
    let mut emitters: Vec<u32> = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let (ox, oz) = ((i % 3) * 16, (i / 3) * 16);
        for (s, section) in chunk.sections().iter().enumerate() {
            if section.is_empty() {
                continue;
            }
            let base = (s + 1 - crop.0) * 16;
            match &section.blocks {
                crate::palette::PalettedContainer::Single(state) => {
                    let emits = table.emission[usize::from(state.0)] != 0;
                    for y in 0..16 {
                        for z in 0..16 {
                            let row = index(ox, base + y, oz + z);
                            states[row..row + 16].fill(*state);
                            if emits {
                                emitters.extend((row..row + 16).map(|n| n as u32));
                            }
                        }
                    }
                }
                crate::palette::PalettedContainer::Direct(values) => {
                    for y in 0..16 {
                        for z in 0..16 {
                            let row = index(ox, base + y, oz + z);
                            let from = &values[(y << 8) | (z << 4)..][..16];
                            states[row..row + 16].copy_from_slice(from);
                        }
                    }
                    // The registry's emission table is fixed, so a section's
                    // emitters are found once and kept until it changes.
                    let found = section.emitters.get_or_init(|| {
                        values.iter().enumerate().filter(|(_, s)| table.emission[usize::from(s.0)] != 0).map(|(i, _)| i as u16).collect()
                    });
                    for &i in found.iter() {
                        let i = usize::from(i);
                        emitters.push(index(ox + (i & 15), base + (i >> 8), oz + ((i >> 4) & 15)) as u32);
                    }
                }
            }
        }
    }
    // The order a full scan would find them in.
    emitters.sort_unstable();
    profile::add(profile::STATES, started);
    let region = light_region_with(blocks, table, center.min_section_y(), build_sections, crop, states, Some(emitters), true, non_empty, sky_light);
    let started = std::time::Instant::now();
    let light = region.center();
    profile::add(profile::LAYERS, started);
    light
}

/// Light over a 3x3 of chunk columns (48x48 blocks), from one section below
/// the build range to one above.
pub struct LightRegion {
    /// Section Y of the lowest section.
    pub min_section: i32,
    /// Sections below `base` and from `base + sections` up (of
    /// `full_sections`) are not held; `get` expects `base` 0.
    base: usize,
    full_sections: usize,
    sections: usize,
    storage: Vec<bool>,
    block: Vec<u8>,
    sky: Vec<u8>,
    sky_light: bool,
}

impl LightRegion {
    fn index(x: usize, y: usize, z: usize) -> usize {
        (y * SIZE + z) * SIZE + x
    }

    /// Block and sky light at region coordinates (x and z in 0..48, y from
    /// the lowest section's bottom). Outside stored sections: 0 block light,
    /// and sky light 15 above the column's data, 0 below it.
    pub fn get(&self, x: usize, y: usize, z: usize) -> (u8, u8) {
        if !self.storage[((z / 16) * 3 + x / 16) * self.sections + y / 16] {
            let above = (y / 16 + 1..self.sections).all(|s| !self.storage[((z / 16) * 3 + x / 16) * self.sections + s]);
            return (0, if self.sky_light && above { 15 } else { 0 });
        }
        let i = Self::index(x, y, z);
        (self.block[i], self.sky[i])
    }

    /// The center chunk's light in vanilla `DataLayer` form.
    pub fn center(&self) -> ChunkLight {
        let layers = |levels: &Vec<u8>| -> Vec<Option<Box<[u8; 2048]>>> {
            (0..self.full_sections)
                .map(|full| {
                    if full < self.base || full >= self.base + self.sections {
                        return None;
                    }
                    let s = full - self.base;
                    if !self.storage[4 * self.sections + s] {
                        return None;
                    }
                    let mut data = Box::new([0u8; 2048]);
                    for y in 0..16 {
                        for z in 0..16 {
                            for x in 0..16 {
                                let value = levels[Self::index(16 + x, s * 16 + y, 16 + z)];
                                let i = y << 8 | z << 4 | x;
                                data[i >> 1] |= value << (4 * (i & 1));
                            }
                        }
                    }
                    Some(data)
                })
                .collect()
        };
        ChunkLight {
            min_section: self.min_section,
            correct: true,
            block: layers(&self.block),
            sky: if self.sky_light { layers(&self.sky) } else { vec![None; self.full_sections] },
        }
    }
}

/// Lights a 3x3 area. `states` holds every block (`(y * 48 + z) * 48 + x`,
/// y from one section below the build range); `non_empty(dx, dz, section_y)`
/// says whether a chunk section up to two chunks from the center has blocks.
pub fn light_region(
    blocks: &BlockRegistry,
    table: &LightTable,
    build_min_section: i32,
    build_sections: usize,
    states: Vec<BlockStateId>,
    non_empty: impl Fn(i32, i32, i32) -> bool,
    sky_light: bool,
) -> LightRegion {
    light_region_with(blocks, table, build_min_section, build_sections, (0, build_sections + 2), states, None, false, non_empty, sky_light)
}

/// `light_region` with the emitting cells already known (in index order).
/// With `center_only`, only the center chunk's levels are exact: a level
/// spreads no further once it cannot reach the center, since every step
/// toward it (at least one per block of horizontal distance) costs one.
#[allow(clippy::too_many_arguments)]
fn light_region_with(
    blocks: &BlockRegistry,
    table: &LightTable,
    build_min_section: i32,
    build_sections: usize,
    crop: (usize, usize),
    states: Vec<BlockStateId>,
    emitters: Option<Vec<u32>>,
    center_only: bool,
    non_empty: impl Fn(i32, i32, i32) -> bool,
    sky_light: bool,
) -> LightRegion {
    // `crop`: the first region section held (index from one below the
    // build range) and how many; the others store no light.
    let full_sections = build_sections + 2;
    let (base, sections) = crop;
    let min_section = build_min_section - 1 + base as i32;
    let height = sections * 16;
    let min_y = min_section * 16;
    let top_section = build_min_section + build_sections as i32 - 1;
    assert_eq!(states.len(), SIZE * SIZE * height, "light region states");

    // Which (dx, dz, section) of the 3x3 store light.
    let stored = |dx: i32, dz: i32, s: usize| -> bool {
        let sy = min_section + s as i32;
        for ox in -1..=1 {
            for oz in -1..=1 {
                for oy in -1..=1 {
                    let y = sy + oy;
                    if y >= build_min_section && y <= top_section && non_empty(dx + ox, dz + oz, y) {
                        return true;
                    }
                }
            }
        }
        false
    };
    let mut storage = vec![false; 9 * sections];
    for dz in -1..=1 {
        for dx in -1..=1 {
            for s in 0..sections {
                storage[((dz + 1) * 3 + dx + 1) as usize * sections + s] = stored(dx, dz, s);
            }
        }
    }
    let index = |x: usize, y: usize, z: usize| (y * SIZE + z) * SIZE + x;
    let is_stored = |x: usize, y: usize, z: usize| storage[((z / 16) * 3 + x / 16) * sections + y / 16];

    // `SkyLightSectionStorage.currentLowestY`: the lowest section with data.
    let lowest_stored = (0..sections).find(|&s| (0..9).any(|c| storage[c * sections + s])).unwrap_or(sections);
    let propagate = |levels: &mut Vec<u8>, mut queue: Vec<Vec<u32>>, sky: bool| {
        for level in (2..=15u8).rev() {
            let mut i = 0;
            while i < queue[level as usize].len() {
                let node = queue[level as usize][i] as usize;
                i += 1;
                if levels[node] != level {
                    continue;
                }
                let (x, z, y) = (node % SIZE, (node / SIZE) % SIZE, node / (SIZE * SIZE));
                if center_only && usize::from(level) <= CENTER_DISTANCE[x] + CENTER_DISTANCE[z] {
                    continue;
                }
                let from = states[node];
                for direction in Direction::ALL {
                    let (ox, oy, oz) = direction.offset();
                    let (nx, ny, nz) = (x as i32 + ox, y as i32 + oy, z as i32 + oz);
                    if nx < 0 || nz < 0 || ny < 0 || nx >= SIZE as i32 || nz >= SIZE as i32 || ny >= height as i32 {
                        continue;
                    }
                    let (nx, ny, nz) = (nx as usize, ny as usize, nz as usize);
                    if !is_stored(nx, ny, nz) {
                        continue;
                    }
                    let to_node = index(nx, ny, nz);
                    let to = states[to_node];
                    let new = level.saturating_sub(table.opacity[usize::from(to.0)]);
                    if new <= levels[to_node] {
                        continue;
                    }
                    if table.shape_occludes(blocks, from, to, direction) {
                        continue;
                    }
                    levels[to_node] = new;
                    queue[new as usize].push(to_node as u32);
                    // `SkyLightEngine.propagateFromEmptySections`: from the bottom
                    // row of a section at a column edge, over a gap of sections
                    // without data, the level also fills the neighbour column's
                    // stored sections beside the gap.
                    if sky && y % 16 == 0 && oy == 0 && (nx / 16 != x / 16 || nz / 16 != z / 16) {
                        let section = y / 16;
                        let mut gap = 0;
                        while section > gap && section - gap - 1 >= lowest_stored && !is_stored(x, (section - gap - 1) * 16, z) {
                            gap += 1;
                        }
                        for s in (section - gap..section).rev() {
                            if !is_stored(nx, s * 16, nz) {
                                continue;
                            }
                            for ly in (0..16).rev() {
                                let fill = index(nx, s * 16 + ly, nz);
                                if levels[fill] < new {
                                    levels[fill] = new;
                                    queue[new as usize].push(fill as u32);
                                }
                            }
                        }
                    }
                }
            }
        }
    };

    // Block light from emitting blocks.
    let started = std::time::Instant::now();
    let mut block = vec![0u8; SIZE * SIZE * height];
    let mut queue: Vec<Vec<u32>> = vec![Vec::new(); 16];
    let emitters = emitters.unwrap_or_else(|| (0..states.len() as u32).filter(|&node| table.emission[usize::from(states[node as usize].0)] != 0).collect());
    for node in emitters {
        let node = node as usize;
        let emission = table.emission[usize::from(states[node].0)];
        let (x, z, y) = (node % SIZE, (node / SIZE) % SIZE, node / (SIZE * SIZE));
        if is_stored(x, y, z) {
            block[node] = emission;
            queue[emission as usize].push(node as u32);
        }
    }
    propagate(&mut block, queue, false);
    profile::add(profile::BLOCK, started);

    // Sky light: 15 at and above each column's lowest source.
    let started = std::time::Instant::now();
    let mut sky = vec![0u8; SIZE * SIZE * height];
    if sky_light {
        // Each column's lowest source (region Y); every stored cell from
        // there up is a source.
        let mut starts = vec![height; SIZE * SIZE];
        // Each chunk's highest non-empty section.
        let tops: Vec<Option<usize>> = (0..9)
            .map(|c| {
                let (dx, dz) = (c % 3 - 1, c / 3 - 1);
                (0..build_sections).rev().find(|&s| non_empty(dx, dz, build_min_section + s as i32))
            })
            .collect();
        for z in 0..SIZE {
            for x in 0..SIZE {
                let (dx, dz) = ((x / 16) as i32 - 1, (z / 16) as i32 - 1);
                let top = tops[(z / 16) * 3 + x / 16];
                let lowest = top.map_or(i32::MIN, |top| {
                    let stop = 1usize.saturating_sub(base);
                    lowest_source_in(blocks, table, (top + 2 - base) * 16, stop, |y| states[index(x, y, z)], |s| non_empty(dx, dz, min_section + s as i32)).map_or(i32::MIN, |y| y as i32 + min_y)
                });
                starts[z * SIZE + x] = (lowest.max(min_y) - min_y) as usize;
            }
        }
        // Row by row, as the levels lie in memory.
        for y in 0..height {
            for z in 0..SIZE {
                let row = index(0, y, z);
                for cx in 0..3 {
                    if !storage[((z / 16) * 3 + cx) * sections + y / 16] {
                        continue;
                    }
                    for x in cx * 16..cx * 16 + 16 {
                        if y >= starts[z * SIZE + x] {
                            sky[row + x] = 15;
                        }
                    }
                }
            }
        }
        // Only a source beside a stored cell that is no source can raise
        // anything: every other neighbour already holds 15. Propagating the
        // rest would change nothing, so they are left out of the queue.
        let source = |x: usize, y: usize, z: usize| y >= starts[z * SIZE + x];
        let mut queue: Vec<Vec<u32>> = vec![Vec::new(); 16];
        for z in 0..SIZE {
            for x in 0..SIZE {
                let start = starts[z * SIZE + x];
                // Above every horizontal neighbour's lowest source the only
                // neighbours left are sources too.
                let mut end = start + 1;
                for (nx, nz) in [(x.wrapping_sub(1), z), (x + 1, z), (x, z.wrapping_sub(1)), (x, z + 1)] {
                    if nx < SIZE && nz < SIZE {
                        end = end.max(starts[nz * SIZE + nx]);
                    }
                }
                for y in start..end.min(height) {
                    if !is_stored(x, y, z) {
                        continue;
                    }
                    let lit_neighbour = |nx: usize, ny: usize, nz: usize| is_stored(nx, ny, nz) && !source(nx, ny, nz);
                    let raises = (y > 0 && lit_neighbour(x, y - 1, z))
                        || (x > 0 && lit_neighbour(x - 1, y, z))
                        || (x + 1 < SIZE && lit_neighbour(x + 1, y, z))
                        || (z > 0 && lit_neighbour(x, y, z - 1))
                        || (z + 1 < SIZE && lit_neighbour(x, y, z + 1));
                    if raises {
                        queue[15].push(index(x, y, z) as u32);
                    }
                }
            }
        }
        profile::add(profile::SKY_SEED, started);
        let started = std::time::Instant::now();
        if std::env::var_os("MINECRAFTOSS_LIGHT_CHECK").is_some() {
            // Every source queued, as before: the result must not change.
            let mut all: Vec<Vec<u32>> = vec![Vec::new(); 16];
            for (node, &level) in sky.iter().enumerate() {
                if level == 15 {
                    all[15].push(node as u32);
                }
            }
            let mut reference = sky.clone();
            propagate(&mut reference, all, true);
            propagate(&mut sky, queue, true);
            assert!(reference == sky, "pruned sky sources change the light");
        } else {
            propagate(&mut sky, queue, true);
        }
        profile::add(profile::SKY_PROPAGATE, started);
    }

    LightRegion { min_section: build_min_section - 1, base, full_sections, sections, storage, block, sky, sky_light }
}

/// `findLowestSourceY` over one column of region cells: scanning down from
/// `top` (exclusive, the top of the highest non-empty section), the region Y
/// just above the first occluding edge; `None` if there is none. Empty
/// sections (`non_empty` by region section index) are skipped as air.
fn lowest_source_in(blocks: &BlockRegistry, table: &LightTable, top: usize, stop: usize, state: impl Fn(usize) -> BlockStateId, non_empty: impl Fn(usize) -> bool) -> Option<usize> {
    let mut top_state = BlockStateId::AIR;
    let mut top_y = top;
    let mut section = top / 16;
    while section > stop {
        section -= 1;
        let base = section * 16;
        if !non_empty(section) {
            top_state = BlockStateId::AIR;
            top_y = base;
            continue;
        }
        for y in (base..base + 16).rev() {
            let bottom = state(y);
            // Air under air never occludes (no dampening, empty faces).
            if !(bottom == BlockStateId::AIR && top_state == BlockStateId::AIR) && table.edge_occluded(blocks, top_state, bottom) {
                return Some(top_y);
            }
            top_state = bottom;
            top_y = y;
        }
    }
    None
}

/// `ChunkSkyLightSources.findLowestSourceY` for one column: the lowest Y
/// with full sky light above no occluding edge, or `i32::MIN` when the
/// column has none (sources extend below the world).
pub fn lowest_source_y(blocks: &BlockRegistry, table: &LightTable, chunk: &Chunk, x: usize, z: usize) -> i32 {
    let sections = chunk.sections();
    let Some(top) = sections.iter().rposition(|s| !s.is_empty()) else { return i32::MIN };
    let mut top_state = BlockStateId::AIR;
    let mut top_y = (chunk.min_section_y() + top as i32 + 1) * 16;
    for s in (0..=top).rev() {
        let section = &sections[s];
        let section_y = (chunk.min_section_y() + s as i32) * 16;
        if section.is_empty() {
            top_state = BlockStateId::AIR;
            top_y = section_y;
            continue;
        }
        for y in (0..16).rev() {
            let bottom = section.block(x, y, z);
            if table.edge_occluded(blocks, top_state, bottom) {
                return top_y;
            }
            top_state = bottom;
            top_y = section_y + y as i32;
        }
    }
    i32::MIN
}
