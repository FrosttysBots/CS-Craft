//! Scheduled liquid updates for the bounded handcrafted world.
//! Behavior follows the pinned 26.3 FlowingFluid, WaterFluid, LavaFluid and
//! LiquidBlock rules; see research/private/fluids/PROVENANCE.md.
use crate::scene::{Block, BlockPos, HandcraftedScene, Scene};
use minecraftoss_player::ticks::ScheduledTicks;
use std::collections::BTreeSet;

const HORIZONTAL: [(i32, i32, i32); 4] = [(0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)];
const ADJACENT: [(i32, i32, i32); 6] = [
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
    (-1, 0, 0),
    (1, 0, 0),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FluidKind {
    Water,
    Lava,
}
impl FluidKind {
    fn block(self, level: u8) -> Block {
        let id = match self {
            Self::Water => "minecraft:water",
            Self::Lava => "minecraft:lava",
        };
        Block::new(id).with("level", &level.to_string())
    }
    fn delay(self) -> u64 {
        match self {
            Self::Water => 5,
            Self::Lava => 30, // Overworld: FAST_LAVA is false.
        }
    }
    fn drop_off(self) -> u8 {
        match self {
            Self::Water => 1,
            Self::Lava => 2,
        }
    }
    fn slope_distance(self) -> u8 {
        match self {
            Self::Water => 4,
            Self::Lava => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FluidCell {
    pub kind: FluidKind,
    pub amount: u8,
    pub source: bool,
    pub falling: bool,
}
impl FluidCell {
    pub fn at(scene: &impl Scene, pos: BlockPos) -> Option<Self> {
        scene.fluid_at(pos)
    }
    pub fn from_block(block: &Block) -> Option<Self> {
        if block
            .properties
            .get("waterlogged")
            .is_some_and(|v| v == "true")
        {
            return Some(Self::source(FluidKind::Water));
        }
        let kind = match block.id.key().as_str() {
            "minecraft:water" => FluidKind::Water,
            "minecraft:lava" => FluidKind::Lava,
            _ => return None,
        };
        let level = block
            .properties
            .get("level")
            .and_then(|v| v.parse::<u8>().ok())
            .unwrap_or(0)
            .min(15);
        Some(if level == 0 {
            Self::source(kind)
        } else if level >= 8 {
            Self {
                kind,
                amount: 8,
                source: false,
                falling: true,
            }
        } else {
            Self {
                kind,
                amount: 8 - level,
                source: false,
                falling: false,
            }
        })
    }
    pub fn source(kind: FluidKind) -> Self {
        Self {
            kind,
            amount: 8,
            source: true,
            falling: false,
        }
    }
    fn flowing(kind: FluidKind, amount: u8, falling: bool) -> Self {
        Self {
            kind,
            amount,
            source: false,
            falling,
        }
    }
    pub fn legacy_level(self) -> u8 {
        if self.source {
            0
        } else {
            8 - self.amount.min(8) + if self.falling { 8 } else { 0 }
        }
    }
    pub fn height(self, scene: &impl Scene, pos: BlockPos) -> f32 {
        if Self::at(scene, offset(pos, (0, 1, 0))).is_some_and(|above| above.kind == self.kind) {
            1.0
        } else {
            self.amount as f32 / 9.0
        }
    }
}

fn offset(pos: BlockPos, by: (i32, i32, i32)) -> BlockPos {
    (pos.0 + by.0, pos.1 + by.1, pos.2 + by.2)
}
fn is_replaceable(block: Option<&Block>, kind: FluidKind) -> bool {
    let Some(block) = block else { return true };
    if FluidCell::from_block(block).is_some() {
        return true;
    }
    if kind == FluidKind::Water && waterloggable(block) {
        return true;
    }
    matches!(
        block.id.path.as_str(),
        "air"
            | "cave_air"
            | "void_air"
            | "short_grass"
            | "tall_grass"
            | "fern"
            | "large_fern"
            | "dead_bush"
            | "torch"
            | "redstone_torch"
            | "snow"
    ) || block.id.path.ends_with("_flower")
}
fn waterloggable(block: &Block) -> bool {
    supports_waterlogging(block)
        && block
            .properties
            .get("waterlogged")
            .is_some_and(|v| v == "false")
}
fn supports_waterlogging(block: &Block) -> bool {
    block.properties.contains_key("waterlogged")
        && !(block.id.path.ends_with("_slab")
            && block.properties.get("type").is_some_and(|v| v == "double"))
}
pub fn place_bucket(scene: &mut HandcraftedScene, pos: BlockPos, kind: FluidKind) -> bool {
    let before = scene.block(pos).cloned();
    if kind == FluidKind::Water && before.as_ref().is_some_and(waterloggable) {
        let mut block = before.unwrap();
        block.properties.insert("waterlogged".into(), "true".into());
        scene.set(pos, Some(block));
        return true;
    }
    if !is_replaceable(before.as_ref(), kind) {
        return false;
    }
    scene.set(pos, Some(kind.block(0)));
    true
}
pub fn pickup_source(scene: &mut HandcraftedScene, pos: BlockPos) -> Option<FluidKind> {
    let mut block = scene.block(pos)?.clone();
    let fluid = FluidCell::from_block(&block)?;
    if !fluid.source {
        return None;
    }
    if block
        .properties
        .get("waterlogged")
        .is_some_and(|v| v == "true")
    {
        block
            .properties
            .insert("waterlogged".into(), "false".into());
        scene.set(pos, Some(block));
    } else {
        scene.set(pos, None);
    }
    Some(fluid.kind)
}
pub(crate) fn full_collision(block: Option<&Block>) -> bool {
    let Some(block) = block else { return false };
    if supports_waterlogging(block) {
        return false;
    }
    let path = block.id.path.as_str();
    !matches!(
        path,
        "air"
            | "cave_air"
            | "void_air"
            | "water"
            | "lava"
            | "short_grass"
            | "tall_grass"
            | "fern"
            | "large_fern"
            | "dead_bush"
            | "torch"
            | "redstone_torch"
            | "snow"
    ) && !path.ends_with("_flower")
        && !path.ends_with("_slab")
        && !path.ends_with("_stairs")
}

fn collision_occupies(block: Option<&Block>, x: f64, y: f64, z: f64) -> bool {
    let Some(block) = block else { return false };
    if block.id.path.ends_with("_slab") {
        return match block.properties.get("type").map(String::as_str) {
            Some("double") => true,
            Some("top") => y >= 0.5,
            _ => y < 0.5,
        };
    }
    if block.id.path.ends_with("_stairs") {
        let top = block
            .properties
            .get("half")
            .is_some_and(|half| half == "top");
        let base = if top { y >= 0.5 } else { y < 0.5 };
        return base
            || minecraftoss_player::stair_step_contains(
                block
                    .properties
                    .get("facing")
                    .map_or("north", String::as_str),
                block
                    .properties
                    .get("shape")
                    .map_or("straight", String::as_str),
                x,
                z,
            );
    }
    full_collision(Some(block))
}

/// FlowingFluid.canPassThroughWall uses the union of both collision faces.
/// The authored slab/stair shapes align to a half-block grid, so each face's
/// four quarter-cell samples exactly describe whether that union is complete.
fn collision_faces_occlude(
    source: Option<&Block>,
    target: Option<&Block>,
    direction: (i32, i32, i32),
) -> bool {
    let epsilon = 1.0e-6;
    for a in [0.25, 0.75] {
        for b in [0.25, 0.75] {
            let (source_point, target_point) = match direction {
                (1, 0, 0) => ((1.0 - epsilon, a, b), (epsilon, a, b)),
                (-1, 0, 0) => ((epsilon, a, b), (1.0 - epsilon, a, b)),
                (0, 0, 1) => ((a, b, 1.0 - epsilon), (a, b, epsilon)),
                (0, 0, -1) => ((a, b, epsilon), (a, b, 1.0 - epsilon)),
                (0, 1, 0) => ((a, 1.0 - epsilon, b), (a, epsilon, b)),
                (0, -1, 0) => ((a, epsilon, b), (a, 1.0 - epsilon, b)),
                _ => return false,
            };
            let source_filled =
                collision_occupies(source, source_point.0, source_point.1, source_point.2);
            let target_filled =
                collision_occupies(target, target_point.0, target_point.1, target_point.2);
            if !source_filled && !target_filled {
                return false;
            }
        }
    }
    true
}

#[derive(Clone)]
pub struct FluidEngine {
    now: u64,
    ticks: ScheduledTicks<FluidKind>,
    min_x: i32,
    max_x: i32,
    min_z: i32,
    max_z: i32,
    vertical: std::ops::Range<i32>,
}
impl FluidEngine {
    pub fn new(scene: &impl Scene) -> Self {
        let chunks = scene.chunks();
        Self {
            now: 0,
            ticks: ScheduledTicks::default(),
            min_x: chunks.iter().map(|c| c.0 * 16).min().unwrap_or(-16),
            max_x: chunks.iter().map(|c| c.0 * 16 + 15).max().unwrap_or(31),
            min_z: chunks.iter().map(|c| c.1 * 16).min().unwrap_or(-16),
            max_z: chunks.iter().map(|c| c.1 * 16 + 15).max().unwrap_or(31),
            vertical: -2..320,
        }
    }
    /// A streamed world: fluids tick wherever the world's height allows.
    pub fn streamed(vertical: std::ops::Range<i32>) -> Self {
        Self {
            now: 0,
            ticks: ScheduledTicks::default(),
            min_x: i32::MIN,
            max_x: i32::MAX,
            min_z: i32::MIN,
            max_z: i32::MAX,
            vertical,
        }
    }
    pub fn advance_time(&mut self, now: u64) {
        self.now = now;
    }
    pub fn has_due(&self, now: u64) -> bool {
        self.ticks.has_due(now)
    }
    fn loaded(&self, pos: BlockPos) -> bool {
        self.vertical.contains(&pos.1)
            && (self.min_x..=self.max_x).contains(&pos.0)
            && (self.min_z..=self.max_z).contains(&pos.2)
    }
    fn schedule(&mut self, pos: BlockPos, kind: FluidKind) {
        if !self.loaded(pos) {
            return;
        }
        let due = self.now + kind.delay();
        self.ticks.schedule(pos, kind, due);
    }
    pub fn changed(
        &mut self,
        scene: &mut HandcraftedScene,
        positions: &[BlockPos],
    ) -> Vec<BlockPos> {
        let mut reacted = BTreeSet::new();
        for &pos in positions {
            for at in std::iter::once(pos).chain(ADJACENT.into_iter().map(|d| offset(pos, d))) {
                self.react_lava(scene, at, &mut reacted);
            }
        }
        for &pos in positions.iter().chain(reacted.iter()) {
            for at in std::iter::once(pos).chain(ADJACENT.into_iter().map(|d| offset(pos, d))) {
                if let Some(cell) = FluidCell::at(scene, at) {
                    self.schedule(at, cell.kind);
                }
            }
        }
        reacted.into_iter().collect()
    }
    fn react_lava(
        &self,
        scene: &mut HandcraftedScene,
        pos: BlockPos,
        reacted: &mut BTreeSet<BlockPos>,
    ) {
        let Some(lava) = FluidCell::at(scene, pos).filter(|f| f.kind == FluidKind::Lava) else {
            return;
        };
        // LiquidBlock.shouldSpreadLiquid checks above and four horizontal neighbours.
        if [
            (0, 1, 0),
            HORIZONTAL[0],
            HORIZONTAL[1],
            HORIZONTAL[2],
            HORIZONTAL[3],
        ]
        .into_iter()
        .any(|d| FluidCell::at(scene, offset(pos, d)).is_some_and(|f| f.kind == FluidKind::Water))
        {
            scene.set(
                pos,
                Some(Block::new(if lava.source {
                    "minecraft:obsidian"
                } else {
                    "minecraft:cobblestone"
                })),
            );
            reacted.insert(pos);
        }
    }
    pub fn tick(&mut self, scene: &mut HandcraftedScene, now: u64) -> Vec<BlockPos> {
        self.now = now;
        let mut changed = BTreeSet::new();
        while let Some((pos, kind)) = self.ticks.pop_due(now) {
            if FluidCell::at(scene, pos).is_some_and(|f| f.kind == kind) {
                self.tick_cell(scene, pos, kind, &mut changed);
            }
        }
        changed.into_iter().collect()
    }
    fn set_cell(
        &mut self,
        scene: &mut HandcraftedScene,
        pos: BlockPos,
        cell: Option<FluidCell>,
        changed: &mut BTreeSet<BlockPos>,
    ) {
        if !self.loaded(pos) {
            return;
        }
        let next = if let Some(fluid) = cell {
            if fluid.kind == FluidKind::Water
                && scene.block(pos).is_some_and(|b| {
                    b.properties
                        .get("waterlogged")
                        .is_some_and(|v| v == "false")
                })
            {
                let mut block = scene.block(pos).unwrap().clone();
                block.properties.insert("waterlogged".into(), "true".into());
                Some(block)
            } else {
                Some(fluid.kind.block(fluid.legacy_level()))
            }
        } else {
            None
        };
        if scene.block(pos) == next.as_ref() {
            return;
        }
        scene.set(pos, next);
        changed.insert(pos);
        let reactions = self.changed(scene, &[pos]);
        changed.extend(reactions);
    }
    fn tick_cell(
        &mut self,
        scene: &mut HandcraftedScene,
        pos: BlockPos,
        kind: FluidKind,
        changed: &mut BTreeSet<BlockPos>,
    ) {
        let mut cell = FluidCell::at(scene, pos).unwrap();
        if !cell.source {
            let next = self.new_liquid(scene, pos, kind);
            if next != Some(cell) {
                self.set_cell(scene, pos, next, changed);
                if let Some(next) = next {
                    cell = next
                } else {
                    return;
                }
            }
        }
        let below = offset(pos, (0, -1, 0));
        if self.can_pass(scene, pos, below, kind) {
            let next = self.new_liquid(scene, below, kind);
            if let Some(next) = next.filter(|_| self.can_replace(scene, below, kind, true)) {
                if kind == FluidKind::Lava
                    && FluidCell::at(scene, below).is_some_and(|f| f.kind == FluidKind::Water)
                {
                    self.set_cell(scene, below, None, changed);
                    scene.set(below, Some(Block::new("minecraft:stone")));
                    changed.insert(below);
                    self.changed(scene, &[below]);
                } else {
                    self.set_cell(scene, below, Some(next), changed);
                }
                if self.source_neighbors(scene, pos, kind) < 3 {
                    return;
                }
            }
        }
        if !cell.source && self.is_hole(scene, pos, kind) {
            return;
        }
        let amount = if cell.falling {
            7
        } else {
            cell.amount.saturating_sub(kind.drop_off())
        };
        if amount == 0 {
            return;
        }
        let mut candidates = Vec::new();
        let mut best = u8::MAX;
        for direction in HORIZONTAL {
            let target = offset(pos, direction);
            if !self.can_pass(scene, pos, target, kind) {
                continue;
            }
            let Some(next) = self.new_liquid(scene, target, kind) else {
                continue;
            };
            let distance = if self.is_hole(scene, target, kind) {
                0
            } else {
                self.slope_distance(scene, target, kind, opposite(direction), 1)
            };
            if distance < best {
                candidates.clear();
                best = distance;
            }
            if distance <= best && self.can_replace(scene, target, kind, false) {
                candidates.push((target, next));
            }
        }
        for (target, next) in candidates {
            self.set_cell(scene, target, Some(next), changed);
        }
    }
    fn new_liquid(&self, scene: &impl Scene, pos: BlockPos, kind: FluidKind) -> Option<FluidCell> {
        let mut strongest = 0;
        let mut sources = 0;
        for d in HORIZONTAL {
            let from = offset(pos, d);
            if !self.pass_wall(scene, from, pos) {
                continue;
            }
            if let Some(fluid) = FluidCell::at(scene, from).filter(|f| f.kind == kind) {
                strongest = strongest.max(fluid.amount);
                sources += u8::from(fluid.source);
            }
        }
        let below = offset(pos, (0, -1, 0));
        if sources >= 2
            && kind == FluidKind::Water
            && (full_collision(scene.block(below))
                || FluidCell::at(scene, below).is_some_and(|f| f.kind == kind && f.source))
        {
            return Some(FluidCell::source(kind));
        }
        if FluidCell::at(scene, offset(pos, (0, 1, 0))).is_some_and(|f| f.kind == kind) {
            return Some(FluidCell::flowing(kind, 8, true));
        }
        let amount = strongest.saturating_sub(kind.drop_off());
        (amount > 0).then(|| FluidCell::flowing(kind, amount, false))
    }
    fn source_neighbors(&self, scene: &impl Scene, pos: BlockPos, kind: FluidKind) -> usize {
        HORIZONTAL
            .into_iter()
            .filter(|&d| {
                FluidCell::at(scene, offset(pos, d)).is_some_and(|f| f.kind == kind && f.source)
            })
            .count()
    }
    fn pass_wall(&self, scene: &impl Scene, from: BlockPos, to: BlockPos) -> bool {
        !collision_faces_occlude(
            scene.block(from),
            scene.block(to),
            (to.0 - from.0, to.1 - from.1, to.2 - from.2),
        )
    }
    fn can_pass(&self, scene: &impl Scene, from: BlockPos, to: BlockPos, kind: FluidKind) -> bool {
        self.loaded(to)
            && !FluidCell::at(scene, to).is_some_and(|f| f.kind == kind && f.source)
            && is_replaceable(scene.block(to), kind)
            && self.pass_wall(scene, from, to)
            && !(kind == FluidKind::Water
                && scene.block(to).is_some_and(waterloggable)
                && !self
                    .new_liquid(scene, to, kind)
                    .is_some_and(|next| next.source))
    }
    fn can_replace(
        &self,
        scene: &impl Scene,
        pos: BlockPos,
        kind: FluidKind,
        downward: bool,
    ) -> bool {
        match FluidCell::at(scene, pos) {
            None => true,
            Some(other) if other.kind == FluidKind::Water => downward && kind == FluidKind::Lava,
            Some(other) => kind == FluidKind::Water && other.height(scene, pos) >= 4.0 / 9.0,
        }
    }
    fn is_hole(&self, scene: &impl Scene, pos: BlockPos, kind: FluidKind) -> bool {
        let below = offset(pos, (0, -1, 0));
        self.loaded(below)
            && self.pass_wall(scene, pos, below)
            && (FluidCell::at(scene, below).is_some_and(|f| f.kind == kind)
                || is_replaceable(scene.block(below), kind))
    }
    fn slope_distance(
        &self,
        scene: &impl Scene,
        pos: BlockPos,
        kind: FluidKind,
        from: (i32, i32, i32),
        depth: u8,
    ) -> u8 {
        let mut best = u8::MAX;
        for direction in HORIZONTAL {
            if direction == from {
                continue;
            }
            let target = offset(pos, direction);
            if !self.can_pass(scene, pos, target, kind) {
                continue;
            }
            if self.is_hole(scene, target, kind) {
                return depth;
            }
            if depth < kind.slope_distance() {
                best = best.min(self.slope_distance(
                    scene,
                    target,
                    kind,
                    opposite(direction),
                    depth + 1,
                ));
            }
        }
        best
    }
}
fn opposite(d: (i32, i32, i32)) -> (i32, i32, i32) {
    (-d.0, -d.1, -d.2)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn floor() -> HandcraftedScene {
        let mut scene = HandcraftedScene::default();
        for x in -10..=10 {
            for z in -10..=10 {
                scene.set((x, 0, z), Some(Block::new("minecraft:stone")));
            }
        }
        scene
    }
    #[test]
    fn water_source_spreads_after_five_ticks_and_falling_prefers_down() {
        let mut scene = floor();
        let mut sim = FluidEngine::new(&scene);
        scene.set((0, 2, 0), Some(FluidKind::Water.block(0)));
        sim.changed(&mut scene, &[(0, 2, 0)]);
        assert!(sim.tick(&mut scene, 4).is_empty());
        assert!(sim.tick(&mut scene, 5).contains(&(0, 1, 0)));
        assert_eq!(FluidCell::at(&scene, (0, 1, 0)).unwrap().legacy_level(), 8);
        assert!(sim.tick(&mut scene, 10).contains(&(1, 1, 0)));
    }
    #[test]
    fn two_sources_form_new_water_source_only_over_support() {
        let mut scene = floor();
        let mut sim = FluidEngine::new(&scene);
        for x in [-1, 1] {
            scene.set((x, 1, 0), Some(FluidKind::Water.block(0)));
        }
        sim.changed(&mut scene, &[(-1, 1, 0), (1, 1, 0)]);
        sim.tick(&mut scene, 5);
        sim.tick(&mut scene, 10);
        assert_eq!(
            FluidCell::at(&scene, (0, 1, 0)),
            Some(FluidCell::source(FluidKind::Water))
        );
    }
    #[test]
    fn lava_is_slower_and_water_contact_solidifies() {
        let mut scene = floor();
        let mut sim = FluidEngine::new(&scene);
        scene.set((0, 1, 0), Some(FluidKind::Lava.block(0)));
        sim.changed(&mut scene, &[(0, 1, 0)]);
        assert!(sim.tick(&mut scene, 29).is_empty());
        assert!(!sim.tick(&mut scene, 30).is_empty());
        scene.set((0, 2, 0), Some(FluidKind::Water.block(0)));
        assert!(sim.changed(&mut scene, &[(0, 2, 0)]).contains(&(0, 1, 0)));
        assert_eq!(scene.block((0, 1, 0)).unwrap().id.path, "obsidian");
    }
    #[test]
    fn buckets_place_sources_and_only_collect_sources() {
        let mut scene = floor();
        assert!(place_bucket(&mut scene, (0, 1, 0), FluidKind::Water));
        assert_eq!(
            FluidCell::at(&scene, (0, 1, 0)),
            Some(FluidCell::source(FluidKind::Water))
        );
        assert_eq!(pickup_source(&mut scene, (0, 1, 0)), Some(FluidKind::Water));
        assert!(scene.block((0, 1, 0)).is_none());
        scene.set((0, 1, 0), Some(FluidKind::Water.block(3)));
        assert_eq!(pickup_source(&mut scene, (0, 1, 0)), None);
        scene.set(
            (1, 1, 0),
            Some(
                Block::new("minecraft:oak_slab")
                    .with("type", "bottom")
                    .with("waterlogged", "false"),
            ),
        );
        assert!(place_bucket(&mut scene, (1, 1, 0), FluidKind::Water));
        assert_eq!(
            scene
                .block((1, 1, 0))
                .unwrap()
                .properties
                .get("waterlogged")
                .unwrap(),
            "true"
        );
        assert_eq!(pickup_source(&mut scene, (1, 1, 0)), Some(FluidKind::Water));
        assert_eq!(
            scene
                .block((1, 1, 0))
                .unwrap()
                .properties
                .get("waterlogged")
                .unwrap(),
            "false"
        );
        assert!(!place_bucket(&mut scene, (1, 1, 0), FluidKind::Lava));
    }
    #[test]
    fn flowing_water_does_not_fill_a_dry_waterloggable_slab() {
        let mut scene = floor();
        scene.set(
            (1, 1, 0),
            Some(
                Block::new("minecraft:oak_slab")
                    .with("type", "top")
                    .with("waterlogged", "false"),
            ),
        );
        scene.set((0, 1, 0), Some(FluidKind::Water.block(0)));
        let mut sim = FluidEngine::new(&scene);
        sim.changed(&mut scene, &[(0, 1, 0), (1, 1, 0)]);
        sim.tick(&mut scene, 5);
        assert_eq!(
            scene
                .block((1, 1, 0))
                .unwrap()
                .properties
                .get("waterlogged"),
            Some(&"false".to_string())
        );
        assert!(FluidCell::at(&scene, (0, 1, 1)).is_some());
    }
    #[test]
    fn complementary_slab_faces_occlude_horizontal_flow() {
        let bottom = Block::new("minecraft:oak_slab").with("type", "bottom");
        let top = Block::new("minecraft:oak_slab").with("type", "top");
        assert!(collision_faces_occlude(
            Some(&bottom),
            Some(&top),
            (1, 0, 0)
        ));
        assert!(!collision_faces_occlude(
            Some(&bottom),
            Some(&bottom),
            (1, 0, 0)
        ));
    }
    #[test]
    fn flat_floor_steps_match_repeatable_26_3_reference() {
        for (kind, observations) in [
            (FluidKind::Water, &[(5, 5, 1), (10, 13, 2), (15, 25, 3)][..]),
            (FluidKind::Lava, &[(30, 5, 2), (60, 13, 4)][..]),
        ] {
            let mut scene = floor();
            let mut sim = FluidEngine::new(&scene);
            scene.set((0, 1, 0), Some(kind.block(0)));
            sim.changed(&mut scene, &[(0, 1, 0)]);
            for &(when, expected_count, edge_level) in observations {
                sim.tick(&mut scene, when);
                let count = (-3..=3)
                    .flat_map(|x| (-3..=3).filter_map(move |z| Some((x, z))))
                    .filter(|&(x, z)| {
                        FluidCell::at(&scene, (x, 1, z)).is_some_and(|f| f.kind == kind)
                    })
                    .count();
                assert_eq!(count, expected_count, "{kind:?} tick {when}");
                let radius = if kind == FluidKind::Water {
                    when / 5
                } else {
                    when / 30
                };
                assert_eq!(
                    FluidCell::at(&scene, (radius as i32, 1, 0))
                        .unwrap()
                        .legacy_level(),
                    edge_level
                );
                for x in -3_i32..=3 {
                    for z in -3_i32..=3 {
                        let distance = x.abs() + z.abs();
                        let expected =
                            (distance as u64 <= radius).then_some(distance as u8 * kind.drop_off());
                        let actual = FluidCell::at(&scene, (x, 1, z)).map(FluidCell::legacy_level);
                        assert_eq!(actual, expected, "{kind:?} tick {when} at {x},{z}");
                    }
                }
            }
        }
    }
}
