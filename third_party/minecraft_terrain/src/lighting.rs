//! Skylight for the currently loaded scene. Direct columns retain level 15
//! through air; leaves and water attenuate it, then neighboring cells spread
//! light into the shade. This follows the 26.3 SkyLightEngine propagation rule.
use crate::scene::{Block, BlockPos, ChunkPos, Scene};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

#[derive(Clone)]
pub struct SkyLight {
    min: BlockPos,
    size: (usize, usize, usize),
    levels: Vec<u8>,
    block_levels: Vec<u8>,
    /// Streamed worlds light each chunk column separately. The dense region
    /// above is empty then, and lookups route to the column's own light.
    columns: Option<Arc<HashMap<ChunkPos, Arc<SkyLight>>>>,
}

/// The light properties of one block in this model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LightCell {
    /// Skylight lost entering this block straight down from above.
    pub direct_loss: u8,
    /// Light lost spreading into this block from a neighbor.
    pub opacity: u8,
    pub emission: u8,
}

pub fn light_cell(block: Option<&Block>) -> LightCell {
    let (direct_loss, opacity) = match block {
        None => (0, 1),
        Some(block) if block.is_opaque() => (15, 15),
        Some(block) if block.id.path == "water" => (2, 2),
        Some(block) if block.id.path.ends_with("_leaves") => (1, 1),
        Some(_) => (0, 1),
    };
    LightCell {
        direct_loss,
        opacity,
        emission: block_emission(block),
    }
}

impl SkyLight {
    /// Recompute the union of edited columns and their fifteen-block light
    /// radius. Boundary light from the last accepted mesh remains unchanged;
    /// the worker's dirty-position set contains every edit since that mesh.
    pub fn updated<S: Scene>(&self, scene: &S, positions: &[BlockPos]) -> Self {
        if positions.is_empty() || self.columns.is_some() {
            return self.clone();
        }
        let range = scene.vertical_range();
        if range.start != self.min.1 || range.end + 16 != self.min.1 + self.size.1 as i32 {
            return Self::build(scene);
        }
        let scene_chunks = scene.chunks();
        if scene_chunks.iter().any(|&(cx, cz)| {
            cx * 16 - 16 < self.min.0
                || (cx + 1) * 16 + 16 > self.min.0 + self.size.0 as i32
                || cz * 16 - 16 < self.min.2
                || (cz + 1) * 16 + 16 > self.min.2 + self.size.2 as i32
        }) {
            return Self::build(scene);
        }
        let min_x = positions
            .iter()
            .map(|p| p.0 - 15)
            .min()
            .unwrap()
            .max(self.min.0);
        let max_x = positions
            .iter()
            .map(|p| p.0 + 15)
            .max()
            .unwrap()
            .min(self.min.0 + self.size.0 as i32 - 1);
        let min_z = positions
            .iter()
            .map(|p| p.2 - 15)
            .min()
            .unwrap()
            .max(self.min.2);
        let max_z = positions
            .iter()
            .map(|p| p.2 + 15)
            .max()
            .unwrap()
            .min(self.min.2 + self.size.2 as i32 - 1);
        let within = |(x, _, z): BlockPos| x >= min_x && x <= max_x && z >= min_z && z <= max_z;
        let mut next = self.clone();
        let mut sky_queue = VecDeque::new();
        let mut block_queue = VecDeque::new();
        let top = self.min.1 + self.size.1 as i32;
        for x in min_x..=max_x {
            for z in min_z..=max_z {
                let mut level = 15u8;
                for y in (self.min.1..top).rev() {
                    let pos = (x, y, z);
                    level = level.saturating_sub(light_cell(scene.block(pos)).direct_loss);
                    let index = next.index(pos).unwrap();
                    next.levels[index] = level;
                    next.block_levels[index] = block_emission(scene.block(pos));
                    if level > 1 && opacity(scene, pos) < 15 {
                        sky_queue.push_back(pos);
                    }
                    if next.block_levels[index] > 0 {
                        block_queue.push_back(pos);
                    }
                }
            }
        }
        // Light just outside the edited rectangle can propagate back in, but
        // cannot itself have changed within a fifteen-block propagation radius.
        for y in self.min.1..top {
            for x in min_x..=max_x {
                for z in [min_z - 1, max_z + 1] {
                    let pos = (x, y, z);
                    if self.index(pos).is_some() {
                        if self.get(pos) > 1 {
                            sky_queue.push_back(pos);
                        }
                        if self.get_block(pos) > 1 {
                            block_queue.push_back(pos);
                        }
                    }
                }
            }
            for z in min_z..=max_z {
                for x in [min_x - 1, max_x + 1] {
                    let pos = (x, y, z);
                    if self.index(pos).is_some() {
                        if self.get(pos) > 1 {
                            sky_queue.push_back(pos);
                        }
                        if self.get_block(pos) > 1 {
                            block_queue.push_back(pos);
                        }
                    }
                }
            }
        }
        const DIRECTIONS: [BlockPos; 6] = [
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ];
        while let Some((x, y, z)) = sky_queue.pop_front() {
            let source = next.get((x, y, z));
            for (dx, dy, dz) in DIRECTIONS {
                let target = (x + dx, y + dy, z + dz);
                if !within(target) {
                    continue;
                }
                let Some(index) = next.index(target) else {
                    continue;
                };
                let candidate = source.saturating_sub(opacity(scene, target));
                if candidate > next.levels[index] {
                    next.levels[index] = candidate;
                    if candidate > 1 {
                        sky_queue.push_back(target);
                    }
                }
            }
        }
        while let Some((x, y, z)) = block_queue.pop_front() {
            let source = next.get_block((x, y, z));
            for (dx, dy, dz) in DIRECTIONS {
                let target = (x + dx, y + dy, z + dz);
                if !within(target) {
                    continue;
                }
                let Some(index) = next.index(target) else {
                    continue;
                };
                let candidate = source.saturating_sub(opacity(scene, target));
                if candidate > next.block_levels[index] {
                    next.block_levels[index] = candidate;
                    if candidate > 1 {
                        block_queue.push_back(target);
                    }
                }
            }
        }
        next
    }

    pub fn build<S: Scene>(scene: &S) -> Self {
        let chunks = scene.chunks();
        let range = scene.vertical_range();
        let min_x = chunks.iter().map(|c| c.0).min().unwrap_or(0) * 16 - 16;
        let max_x = (chunks.iter().map(|c| c.0).max().unwrap_or(0) + 1) * 16 + 16;
        let min_z = chunks.iter().map(|c| c.1).min().unwrap_or(0) * 16 - 16;
        let max_z = (chunks.iter().map(|c| c.1).max().unwrap_or(0) + 1) * 16 + 16;
        let min = (min_x, range.start, min_z);
        let size = (
            (max_x - min_x) as usize,
            (range.end - range.start + 16) as usize,
            (max_z - min_z) as usize,
        );
        Self::build_region(min, size, |pos| light_cell(scene.block(pos)))
    }

    /// Lights a box. Direct columns keep level 15 through clear blocks from
    /// the top of the box down, then light spreads to neighbors, losing each
    /// target's opacity, until nothing more can rise. Only cells that can
    /// raise a neighbor seed the spread; the fixed point does not depend on
    /// the order cells are visited, so this equals seeding every lit cell.
    pub fn build_region(
        min: BlockPos,
        size: (usize, usize, usize),
        cell: impl Fn(BlockPos) -> LightCell,
    ) -> Self {
        let (sx, sy, sz) = size;
        let volume = sx * sy * sz;
        let mut cells = Vec::with_capacity(volume);
        for x in 0..sx {
            for z in 0..sz {
                for y in 0..sy {
                    cells.push(cell((
                        min.0 + x as i32,
                        min.1 + y as i32,
                        min.2 + z as i32,
                    )));
                }
            }
        }
        let mut light = Self {
            min,
            size,
            levels: vec![0; volume],
            block_levels: vec![0; volume],
            columns: None,
        };
        let index = |x: usize, y: usize, z: usize| (x * sz + z) * sy + y;
        // The highest cell in each column below full skylight, plus one.
        let mut shaded_top = vec![0usize; sx * sz];
        for x in 0..sx {
            for z in 0..sz {
                let mut level = 15u8;
                for y in (0..sy).rev() {
                    let i = index(x, y, z);
                    level = level.saturating_sub(cells[i].direct_loss);
                    light.levels[i] = level;
                    if level < 15 && shaded_top[x * sz + z] == 0 {
                        shaded_top[x * sz + z] = y + 1;
                    }
                }
            }
        }
        const DIRECTIONS: [(isize, isize, isize); 6] = [
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ];
        let neighbor = |(x, y, z): (usize, usize, usize), (dx, dy, dz): (isize, isize, isize)| {
            let (x, y, z) = (
                x.checked_add_signed(dx)?,
                y.checked_add_signed(dy)?,
                z.checked_add_signed(dz)?,
            );
            (x < sx && y < sy && z < sz).then_some((x, y, z))
        };
        let spread = |levels: &mut Vec<u8>, queue: &mut VecDeque<(usize, usize, usize)>| {
            while let Some(at) = queue.pop_front() {
                let source = levels[index(at.0, at.1, at.2)];
                for d in DIRECTIONS {
                    let Some(target) = neighbor(at, d) else {
                        continue;
                    };
                    let t = index(target.0, target.1, target.2);
                    let candidate = source.saturating_sub(cells[t].opacity);
                    if candidate > levels[t] {
                        levels[t] = candidate;
                        if candidate > 1 {
                            queue.push_back(target);
                        }
                    }
                }
            }
        };
        let raises_neighbor = |levels: &Vec<u8>, at: (usize, usize, usize)| {
            let source = levels[index(at.0, at.1, at.2)];
            DIRECTIONS.iter().any(|&d| {
                neighbor(at, d).is_some_and(|n| {
                    let t = index(n.0, n.1, n.2);
                    source.saturating_sub(cells[t].opacity) > levels[t]
                })
            })
        };
        let mut queue = VecDeque::new();
        for x in 0..sx {
            for z in 0..sz {
                // Above every neighboring column's shade, all cells are 15
                // and so is everything they touch.
                let mut bound = shaded_top[x * sz + z];
                for (nx, nz) in [
                    (x.wrapping_sub(1), z),
                    (x + 1, z),
                    (x, z.wrapping_sub(1)),
                    (x, z + 1),
                ] {
                    if nx < sx && nz < sz {
                        bound = bound.max(shaded_top[nx * sz + nz]);
                    }
                }
                for y in 0..(bound + 1).min(sy) {
                    let i = index(x, y, z);
                    if light.levels[i] > 1
                        && cells[i].opacity < 15
                        && raises_neighbor(&light.levels, (x, y, z))
                    {
                        queue.push_back((x, y, z));
                    }
                }
            }
        }
        spread(&mut light.levels, &mut queue);
        let mut emitters = Vec::new();
        for (i, cell) in cells.iter().enumerate() {
            if cell.emission > 0 {
                light.block_levels[i] = cell.emission;
                emitters.push((i / (sz * sy), i % sy, (i / sy) % sz));
            }
        }
        for at in emitters {
            if raises_neighbor(&light.block_levels, at) {
                queue.push_back(at);
            }
        }
        spread(&mut light.block_levels, &mut queue);
        light
    }

    /// Light from levels laid out like this type's own (`x`, then `z`, then
    /// `y`), for a box at `min` of `size`; lookups outside return full sky
    /// light and no block light.
    pub fn from_levels(min: BlockPos, size: (usize, usize, usize), levels: Vec<u8>, block_levels: Vec<u8>) -> Self {
        assert_eq!(levels.len(), size.0 * size.1 * size.2);
        assert_eq!(block_levels.len(), levels.len());
        Self { min, size, levels, block_levels, columns: None }
    }

    /// An empty streamed-world light with no columns yet.
    pub fn streamed() -> Self {
        Self {
            min: (0, 0, 0),
            size: (0, 0, 0),
            levels: Vec::new(),
            block_levels: Vec::new(),
            columns: Some(Arc::new(HashMap::new())),
        }
    }

    pub fn is_streamed(&self) -> bool {
        self.columns.is_some()
    }

    /// The part of this light inside one chunk column.
    pub fn column(&self, (cx, cz): ChunkPos) -> Self {
        self.crop(cx * 16, cz * 16, 16, 16)
    }

    /// The columns of this light in a horizontal rectangle. Cells above the
    /// highest one below full skylight or with block light are left out;
    /// lookups there return those same defaults.
    pub fn crop(&self, min_x: i32, min_z: i32, width: usize, depth: usize) -> Self {
        let x0 = (min_x - self.min.0) as usize;
        let z0 = (min_z - self.min.2) as usize;
        let mut height = 0;
        for x in x0..x0 + width {
            for z in z0..z0 + depth {
                let base = (x * self.size.2 + z) * self.size.1;
                if let Some(top) = (0..self.size.1).rev().find(|&y| {
                    self.levels[base + y] < 15 || self.block_levels[base + y] > 0
                }) {
                    height = height.max(top + 1);
                }
            }
        }
        let mut column = Self {
            min: (min_x, self.min.1, min_z),
            size: (width, height, depth),
            levels: Vec::with_capacity(width * height * depth),
            block_levels: Vec::with_capacity(width * height * depth),
            columns: None,
        };
        for x in x0..x0 + width {
            for z in z0..z0 + depth {
                let base = (x * self.size.2 + z) * self.size.1;
                column
                    .levels
                    .extend_from_slice(&self.levels[base..base + height]);
                column
                    .block_levels
                    .extend_from_slice(&self.block_levels[base..base + height]);
            }
        }
        column
    }

    /// Whether two lights differ anywhere in a box.
    pub fn differs(&self, other: &Self, min: BlockPos, max: BlockPos) -> bool {
        (min.0..=max.0).any(|x| {
            (min.2..=max.2).any(|z| {
                (min.1..=max.1).any(|y| {
                    self.get((x, y, z)) != other.get((x, y, z))
                        || self.get_block((x, y, z)) != other.get_block((x, y, z))
                })
            })
        })
    }

    /// The section Ys of a chunk column (`sections`) where two lights
    /// differ: `differs` for each section's box, comparing stored column
    /// slices directly when both lights share one dense layout.
    pub fn differing_sections(&self, other: &Self, chunk: ChunkPos, sections: std::ops::RangeInclusive<i32>) -> Vec<i32> {
        let (x0, z0) = (chunk.0 * 16, chunk.1 * 16);
        let dense = self.columns.is_none() && other.columns.is_none() && self.min == other.min && self.size == other.size;
        if !dense {
            return sections
                .filter(|&sy| self.differs(other, (x0, sy * 16, z0), (x0 + 15, sy * 16 + 15, z0 + 15)))
                .collect();
        }
        // Outside the stored box both answer the defaults, so only the
        // stored part of each section can differ.
        let clip = |lo: i32, hi: i32, min: i32, size: usize| (lo.max(min), hi.min(min + size as i32 - 1));
        let (xa, xb) = clip(x0, x0 + 15, self.min.0, self.size.0);
        let (za, zb) = clip(z0, z0 + 15, self.min.2, self.size.2);
        let mut out = Vec::new();
        for sy in sections {
            let (ya, yb) = clip(sy * 16, sy * 16 + 15, self.min.1, self.size.1);
            if ya > yb || xa > xb || za > zb {
                continue;
            }
            let (y0, y1) = ((ya - self.min.1) as usize, (yb - self.min.1) as usize + 1);
            let differs = (xa..=xb).any(|x| {
                (za..=zb).any(|z| {
                    let base = ((x - self.min.0) as usize * self.size.2 + (z - self.min.2) as usize) * self.size.1;
                    self.levels[base + y0..base + y1] != other.levels[base + y0..base + y1]
                        || self.block_levels[base + y0..base + y1] != other.block_levels[base + y0..base + y1]
                })
            });
            if differs {
                out.push(sy);
            }
        }
        out
    }

    /// Streamed worlds: the light of one chunk column, if lit.
    pub fn chunk_column(&self, chunk: ChunkPos) -> Option<&Arc<SkyLight>> {
        self.columns.as_ref()?.get(&chunk)
    }

    /// Streamed worlds: replaces or removes one chunk column's light.
    pub fn set_chunk_column(&mut self, chunk: ChunkPos, light: Option<Arc<SkyLight>>) {
        let Some(columns) = self.columns.as_mut() else {
            return;
        };
        let columns = Arc::make_mut(columns);
        match light {
            Some(light) => columns.insert(chunk, light),
            None => columns.remove(&chunk),
        };
    }

    pub fn get(&self, pos: BlockPos) -> u8 {
        if let Some(columns) = &self.columns {
            return columns
                .get(&(pos.0 >> 4, pos.2 >> 4))
                .map_or(15, |column| column.get(pos));
        }
        self.index(pos).map_or(15, |index| self.levels[index])
    }
    pub fn get_block(&self, pos: BlockPos) -> u8 {
        if let Some(columns) = &self.columns {
            return columns
                .get(&(pos.0 >> 4, pos.2 >> 4))
                .map_or(0, |column| column.get_block(pos));
        }
        self.index(pos).map_or(0, |index| self.block_levels[index])
    }

    fn index(&self, (x, y, z): BlockPos) -> Option<usize> {
        let dx = x - self.min.0;
        let dy = y - self.min.1;
        let dz = z - self.min.2;
        if dx < 0
            || dy < 0
            || dz < 0
            || dx >= self.size.0 as i32
            || dy >= self.size.1 as i32
            || dz >= self.size.2 as i32
        {
            return None;
        }
        Some(((dx as usize * self.size.2) + dz as usize) * self.size.1 + dy as usize)
    }
}

/// Blocks.REDSTONE_LAMP and REDSTONE_TORCH register a litBlockEmission
/// function of 15 and 7 respectively in the pinned common JAR.
pub fn block_emission(block: Option<&Block>) -> u8 {
    let Some(block) = block else {
        return 0;
    };
    match block.id.path.as_str() {
        "lava" => 15,
        "copper_bulb" | "waxed_copper_bulb"
            if block.properties.get("lit").is_some_and(|v| v == "true") =>
        {
            15
        }
        "exposed_copper_bulb" | "waxed_exposed_copper_bulb"
            if block.properties.get("lit").is_some_and(|v| v == "true") =>
        {
            12
        }
        "weathered_copper_bulb" | "waxed_weathered_copper_bulb"
            if block.properties.get("lit").is_some_and(|v| v == "true") =>
        {
            8
        }
        "oxidized_copper_bulb" | "waxed_oxidized_copper_bulb"
            if block.properties.get("lit").is_some_and(|v| v == "true") =>
        {
            4
        }
        "redstone_lamp" if block.properties.get("lit").is_some_and(|v| v == "true") => 15,
        "redstone_torch" | "redstone_wall_torch"
            if block.properties.get("lit").is_some_and(|v| v == "true") =>
        {
            7
        }
        _ => 0,
    }
}

fn opacity<S: Scene>(scene: &S, pos: BlockPos) -> u8 {
    light_cell(scene.block(pos)).opacity
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Block, HandcraftedScene};

    #[test]
    fn differing_sections_agrees_with_differs() {
        // A column 18 wide (chunk plus margins), 40 tall from y = -8.
        let (min, size) = ((-1, -8, -1), (18, 40, 18));
        let cells = size.0 * size.1 * size.2;
        let base: Vec<u8> = (0..cells).map(|i| (i % 16) as u8).collect();
        let a = SkyLight::from_levels(min, size, base.clone(), vec![0; cells]);
        for (x, y, z, block) in [(0, -8, 0, false), (15, 20, 15, true), (-1, 5, 3, false), (7, 31, 16, true), (3, 16, 3, false)] {
            let mut levels = base.clone();
            let mut blocks = vec![0; cells];
            let i = (((x - min.0) as usize * size.2) + (z - min.2) as usize) * size.1 + (y - min.1) as usize;
            if block {
                blocks[i] = 7;
            } else {
                levels[i] ^= 1;
            }
            let b = SkyLight::from_levels(min, size, levels, blocks);
            let slow: Vec<i32> = (-2..=3).filter(|&sy| a.differs(&b, (0, sy * 16, 0), (15, sy * 16 + 15, 15))).collect();
            assert_eq!(a.differing_sections(&b, (0, 0), -2..=3), slow, "change at {x},{y},{z}");
        }
    }

    #[test]
    fn canopy_attenuates_sky_and_casts_a_shadow() {
        let scene = HandcraftedScene::new();
        let light = SkyLight::build(&scene);
        assert_eq!(light.get((0, 2, 0)), 15);
        assert_eq!(light.get((5, 7, -7)), 14);
        assert_eq!(light.get((5, 6, -7)), 13);
        assert_eq!(light.get((5, 5, -7)), 12);
        assert!(light.get((5, 2, -7)) < light.get((0, 2, 0)));
        assert_eq!(light.get((5, 4, -7)), 0);
    }

    #[test]
    fn covered_cell_receives_light_from_open_side() {
        let mut scene = HandcraftedScene::new();
        scene.set((0, 4, 0), Some(Block::new("minecraft:stone")));
        let light = SkyLight::build(&scene);
        assert!(light.get((0, 3, 0)) > 0);
        assert!(light.get((0, 3, 0)) < 15);
    }
    #[test]
    fn lava_emits_block_light_into_dark_cells() {
        let mut scene = HandcraftedScene::default();
        scene.set(
            (0, 0, 0),
            Some(Block::new("minecraft:lava").with("level", "0")),
        );
        let light = SkyLight::build(&scene);
        assert_eq!(light.get_block((0, 0, 0)), 15);
        assert_eq!(light.get_block((1, 0, 0)), 14);
        assert_eq!(light.get_block((20, 0, 0)), 0);
    }

    #[test]
    fn copper_bulb_emission_follows_oxidation_and_lit_state() {
        for (id, strength) in [
            ("copper_bulb", 15),
            ("exposed_copper_bulb", 12),
            ("weathered_copper_bulb", 8),
            ("oxidized_copper_bulb", 4),
        ] {
            for waxed in [false, true] {
                let id = format!("minecraft:{}{id}", if waxed { "waxed_" } else { "" });
                assert_eq!(
                    block_emission(Some(&Block::new(&id).with("lit", "true"))),
                    strength
                );
                assert_eq!(
                    block_emission(Some(&Block::new(&id).with("lit", "false"))),
                    0
                );
            }
        }
    }

    #[test]
    fn redstone_lamp_and_torch_relight_after_state_changes() {
        let mut scene = HandcraftedScene::new();
        let lamp = (0, 2, 0);
        let torch = (0, 2, 3);
        scene.set(
            lamp,
            Some(Block::new("minecraft:redstone_lamp").with("lit", "true")),
        );
        scene.set(
            torch,
            Some(Block::new("minecraft:redstone_torch").with("lit", "true")),
        );
        let lit = SkyLight::build(&scene);
        assert_eq!(lit.get_block(lamp), 15);
        assert_eq!(lit.get_block((1, 2, 0)), 14);
        assert_eq!(lit.get_block(torch), 12); // lamp light overlaps this torch
        assert_eq!(lit.get_block((0, 2, 8)), 7);

        scene.set(
            lamp,
            Some(Block::new("minecraft:redstone_lamp").with("lit", "false")),
        );
        let after_lamp = lit.updated(&scene, &[lamp]);
        assert_eq!(after_lamp.get_block(torch), 7);
        assert_eq!(after_lamp.get_block((0, 2, 8)), 2);
        assert_eq!(
            after_lamp.block_levels,
            SkyLight::build(&scene).block_levels
        );

        scene.set(
            torch,
            Some(Block::new("minecraft:redstone_torch").with("lit", "false")),
        );
        let after_torch = after_lamp.updated(&scene, &[torch]);
        assert_eq!(after_torch.get_block(torch), 0);
        assert_eq!(after_torch.get_block((0, 2, 8)), 0);
        assert_eq!(
            after_torch.block_levels,
            SkyLight::build(&scene).block_levels
        );
    }

    #[test]
    fn local_relighting_matches_full_rebuild_after_mixed_edits() {
        let mut scene = HandcraftedScene::new();
        let mut previous = SkyLight::build(&scene);
        for (pos, block) in [
            ((0, 4, 0), Some(Block::new("minecraft:stone"))),
            ((0, 4, 0), None),
            ((15, 3, 0), Some(Block::new("minecraft:oak_leaves"))),
            ((16, 2, 0), Some(Block::new("minecraft:water"))),
            ((15, 3, 0), Some(Block::new("minecraft:lava"))),
            ((15, 3, 0), None),
        ] {
            scene.set(pos, block);
            let updated = previous.updated(&scene, &[pos]);
            let full = SkyLight::build(&scene);
            assert_eq!(updated.levels, full.levels, "sky edit {pos:?}");
            assert_eq!(
                updated.block_levels, full.block_levels,
                "block edit {pos:?}"
            );
            previous = updated;
        }
        let changes = [(3, 5, 3), (20, 2, 2)];
        scene.set(changes[0], Some(Block::new("minecraft:stone")));
        scene.set(changes[1], Some(Block::new("minecraft:lava")));
        let updated = previous.updated(&scene, &changes);
        let full = SkyLight::build(&scene);
        assert_eq!(updated.levels, full.levels);
        assert_eq!(updated.block_levels, full.block_levels);
    }
}
