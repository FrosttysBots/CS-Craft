//! Multi-noise biome selection (vanilla `Climate`, `OverworldBiomeBuilder`,
//! `MultiNoiseBiomeSourceParameterList` presets).
//!
//! The Overworld table is code in vanilla, not data, so it is ported here.
//! The R-tree is built and searched exactly like `Climate.RTree`, including
//! its per-thread "last result" hint, which decides exact distance ties.

use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// `Climate.quantizeCoord`: `(long) (coord * 10000.0f)`.
pub fn quantize(coord: f32) -> i64 {
    (coord * 10_000.0) as i64
}

/// A quantized closed range (`Climate.Parameter`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parameter {
    pub min: i64,
    pub max: i64,
}

impl Parameter {
    pub fn span(min: f32, max: f32) -> Self {
        assert!(min <= max, "min > max: {min} {max}");
        Self { min: quantize(min), max: quantize(max) }
    }

    pub fn point(value: f32) -> Self {
        Self::span(value, value)
    }

    pub fn join(min: Self, max: Self) -> Self {
        assert!(min.min <= max.max, "min > max");
        Self { min: min.min, max: max.max }
    }

    pub fn distance(self, target: i64) -> i64 {
        let above = target - self.max;
        let below = self.min - target;
        if above > 0 { above } else { below.max(0) }
    }

    fn span_with(self, other: Option<Self>) -> Self {
        match other {
            None => self,
            Some(o) => Self { min: self.min.min(o.min), max: self.max.max(o.max) },
        }
    }
}

/// Six climate ranges and an offset (`Climate.ParameterPoint`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParameterPoint {
    /// Temperature, humidity, continentalness, erosion, depth, weirdness.
    pub parameters: [Parameter; 6],
    pub offset: i64,
}

impl ParameterPoint {
    /// `ParameterPoint.parameterSpace()`: the six ranges plus the offset as a point.
    fn space(&self) -> [Parameter; 7] {
        let p = self.parameters;
        [p[0], p[1], p[2], p[3], p[4], p[5], Parameter { min: self.offset, max: self.offset }]
    }
}

/// A quantized climate sample (`Climate.TargetPoint`) as the search array.
pub type Target = [i64; 7];

pub fn target(temperature: f32, humidity: f32, continentalness: f32, erosion: f32, depth: f32, weirdness: f32) -> Target {
    [quantize(temperature), quantize(humidity), quantize(continentalness), quantize(erosion), quantize(depth), quantize(weirdness), 0]
}

enum Node {
    Leaf { space: [Parameter; 7], value: usize },
    Sub { space: [Parameter; 7], children: Vec<usize> },
}

impl Node {
    fn space(&self) -> &[Parameter; 7] {
        match self {
            Node::Leaf { space, .. } | Node::Sub { space, .. } => space,
        }
    }
}

static NEXT_TREE: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// `RTree.lastResult`, per tree and thread.
    static LAST_RESULT: RefCell<Vec<(usize, usize)>> = const { RefCell::new(Vec::new()) };
}

/// `Climate.RTree` over a parameter list.
pub struct RTree {
    id: usize,
    nodes: Vec<Node>,
    root: usize,
    flat: Flat,
}

/// The tree laid out for search: each node's children and their bounds
/// side by side, bounds as `[min, max]` per dimension.
struct Flat {
    space: Vec<[[i64; 2]; 7]>,
    /// Children of node `n`: `children[start[n]..start[n] + len[n]]`; no
    /// children for a leaf.
    start: Vec<u32>,
    len: Vec<u32>,
    children: Vec<u32>,
    child_space: Vec<[[i64; 2]; 7]>,
}

fn flat_distance(space: &[[i64; 2]; 7], target: &Target) -> i64 {
    let mut sum = 0;
    for d in 0..7 {
        // `Parameter.distance`: bounds never cross, so at most one side is positive.
        let gap = (target[d] - space[d][1]).max(space[d][0] - target[d]).max(0);
        sum += gap * gap;
    }
    sum
}

/// `flat_distance` if it is below `limit`.
fn distance_below(space: &[[i64; 2]; 7], target: &Target, limit: i64) -> Option<i64> {
    let mut sum = 0;
    for d in 0..7 {
        let gap = (target[d] - space[d][1]).max(space[d][0] - target[d]).max(0);
        sum += gap * gap;
        if sum >= limit {
            return None;
        }
    }
    Some(sum)
}

impl Flat {
    fn new(nodes: &[Node]) -> Self {
        let bounds = |n: &Node| n.space().map(|p| [p.min, p.max]);
        let mut flat = Flat { space: nodes.iter().map(bounds).collect(), start: Vec::new(), len: Vec::new(), children: Vec::new(), child_space: Vec::new() };
        for node in nodes {
            flat.start.push(flat.children.len() as u32);
            match node {
                Node::Leaf { .. } => flat.len.push(0),
                Node::Sub { children, .. } => {
                    flat.len.push(children.len() as u32);
                    for &c in children {
                        flat.children.push(c as u32);
                        flat.child_space.push(bounds(&nodes[c]));
                    }
                }
            }
        }
        flat
    }
}

impl RTree {
    pub fn new(points: &[ParameterPoint]) -> Self {
        assert!(!points.is_empty(), "need at least one value to build the search tree");
        let mut nodes: Vec<Node> = points.iter().enumerate().map(|(i, p)| Node::Leaf { space: p.space(), value: i }).collect();
        let leaves: Vec<usize> = (0..nodes.len()).collect();
        let root = Self::build(&mut nodes, leaves, 19);
        let flat = Flat::new(&nodes);
        Self { id: NEXT_TREE.fetch_add(1, Ordering::Relaxed), nodes, root, flat }
    }

    fn center(p: Parameter) -> i64 {
        (p.min + p.max) / 2
    }

    fn sort(nodes: &[Node], children: &mut [usize], dimension: usize, absolute: bool) {
        children.sort_by(|&a, &b| {
            for d in 0..7 {
                let axis = (dimension + d) % 7;
                let key = |n: usize| {
                    let c = Self::center(nodes[n].space()[axis]);
                    if absolute { c.abs() } else { c }
                };
                match key(a).cmp(&key(b)) {
                    std::cmp::Ordering::Equal => continue,
                    other => return other,
                }
            }
            std::cmp::Ordering::Equal
        });
    }

    fn sub_tree(nodes: &mut Vec<Node>, children: Vec<usize>) -> usize {
        let mut bounds: [Option<Parameter>; 7] = [None; 7];
        for &c in &children {
            for (d, bound) in bounds.iter_mut().enumerate() {
                *bound = Some(nodes[c].space()[d].span_with(*bound));
            }
        }
        let space = bounds.map(|b| b.expect("non-empty subtree"));
        nodes.push(Node::Sub { space, children });
        nodes.len() - 1
    }

    fn build(nodes: &mut Vec<Node>, mut children: Vec<usize>, per_node: usize) -> usize {
        assert!(!children.is_empty(), "need at least one child to build a node");
        if children.len() == 1 {
            return children[0];
        }
        if children.len() <= per_node {
            children.sort_by_key(|&c| nodes[c].space().iter().map(|p| Self::center(*p).abs()).sum::<i64>());
            return Self::sub_tree(nodes, children);
        }
        let mut min_cost = i64::MAX;
        let mut min_dimension = None;
        let mut min_buckets: Vec<Vec<usize>> = Vec::new();
        for d in 0..7 {
            Self::sort(nodes, &mut children, d, false);
            let buckets = Self::bucketize(&children, per_node);
            let cost: i64 = buckets.iter().map(|b| Self::bucket_cost(nodes, b)).sum();
            if min_cost <= cost {
                continue;
            }
            min_cost = cost;
            min_dimension = Some(d);
            min_buckets = buckets;
        }
        // Sort buckets by their own bounds, then build each.
        let bucket_nodes: Vec<usize> = min_buckets.into_iter().map(|b| Self::sub_tree(nodes, b)).collect();
        let mut sorted = bucket_nodes.clone();
        Self::sort(nodes, &mut sorted, min_dimension.expect("seven dimensions"), true);
        let built: Vec<usize> = sorted
            .into_iter()
            .map(|b| {
                let Node::Sub { children, .. } = &nodes[b] else { unreachable!() };
                let children = children.clone();
                Self::build(nodes, children, per_node)
            })
            .collect();
        Self::sub_tree(nodes, built)
    }

    fn bucketize(nodes: &[usize], per_node: usize) -> Vec<Vec<usize>> {
        let expected = (per_node as f64).powf(((nodes.len() as f64 - 0.01).ln() / (per_node as f64).ln()).floor()) as usize;
        let mut buckets = Vec::new();
        let mut current = Vec::new();
        for &n in nodes {
            current.push(n);
            if current.len() >= expected {
                buckets.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            buckets.push(current);
        }
        buckets
    }

    fn bucket_cost(nodes: &[Node], bucket: &[usize]) -> i64 {
        let mut bounds: [Option<Parameter>; 7] = [None; 7];
        for &c in bucket {
            for (d, bound) in bounds.iter_mut().enumerate() {
                *bound = Some(nodes[c].space()[d].span_with(*bound));
            }
        }
        bounds.iter().map(|b| {
            let b = b.expect("non-empty bucket");
            (b.max - b.min).abs()
        }).sum()
    }

    fn search_node(&self, node: usize, target: &Target, candidate: Option<usize>) -> usize {
        let flat = &self.flat;
        let len = flat.len[node] as usize;
        if len == 0 {
            return node;
        }
        let start = flat.start[node] as usize;
        let mut min_distance = candidate.map_or(i64::MAX, |c| flat_distance(&flat.space[c], target));
        let mut closest = candidate;
        for i in 0..len {
            let child = flat.children[start + i] as usize;
            // A child at or beyond the best distance is skipped, so its sum
            // stops once it gets there (the parts are never negative).
            let Some(child_distance) = distance_below(&flat.child_space[start + i], target, min_distance) else {
                continue;
            };
            let leaf = self.search_node(child, target, closest);
            let leaf_distance = if child == leaf { child_distance } else { flat_distance(&flat.space[leaf], target) };
            if min_distance <= leaf_distance {
                continue;
            }
            min_distance = leaf_distance;
            closest = Some(leaf);
        }
        closest.expect("a subtree always yields a leaf")
    }

    /// `RTree.search`: returns the index of the chosen parameter point.
    pub fn search(&self, target: &Target) -> usize {
        LAST_RESULT.with(|last| {
            let mut last = last.borrow_mut();
            let hint = last.iter().find(|(tree, _)| *tree == self.id).map(|(_, leaf)| *leaf);
            let leaf = self.search_node(self.root, target, hint);
            match last.iter_mut().find(|(tree, _)| *tree == self.id) {
                Some(entry) => entry.1 = leaf,
                None => last.push((self.id, leaf)),
            }
            let Node::Leaf { value, .. } = &self.nodes[leaf] else { unreachable!("search returns leaves") };
            *value
        })
    }
}

/// A parameter list and its search tree (`Climate.ParameterList`).
pub struct ParameterList {
    pub entries: Vec<(ParameterPoint, &'static str)>,
    tree: RTree,
}

impl ParameterList {
    pub fn new(entries: Vec<(ParameterPoint, &'static str)>) -> Self {
        let points: Vec<ParameterPoint> = entries.iter().map(|(p, _)| *p).collect();
        Self { tree: RTree::new(&points), entries }
    }

    pub fn find(&self, target: &Target) -> &'static str {
        self.entries[self.tree.search(target)].1
    }

    /// Index of the chosen entry.
    pub fn find_index(&self, target: &Target) -> usize {
        self.tree.search(target)
    }

    /// The Nether preset.
    pub fn nether() -> Self {
        let p = |t: f32, h: f32, offset: f32| ParameterPoint {
            parameters: [Parameter::point(t), Parameter::point(h), Parameter::point(0.0), Parameter::point(0.0), Parameter::point(0.0), Parameter::point(0.0)],
            offset: quantize(offset),
        };
        Self::new(vec![
            (p(0.0, 0.0, 0.0), "minecraft:nether_wastes"),
            (p(0.0, -0.5, 0.0), "minecraft:soul_sand_valley"),
            (p(0.4, 0.0, 0.0), "minecraft:crimson_forest"),
            (p(0.0, 0.5, 0.375), "minecraft:warped_forest"),
            (p(-0.5, 0.0, 0.175), "minecraft:basalt_deltas"),
        ])
    }

    /// The Overworld preset.
    pub fn overworld() -> Self {
        let mut entries = Vec::new();
        OverworldBuilder::new().add_biomes(&mut entries);
        Self::new(entries)
    }
}

type Biome = Option<&'static str>;

/// A port of `OverworldBiomeBuilder`.
struct OverworldBuilder {
    full: Parameter,
    temperatures: [Parameter; 5],
    humidities: [Parameter; 5],
    erosions: [Parameter; 7],
    frozen: Parameter,
    unfrozen: Parameter,
    mushroom_fields: Parameter,
    deep_ocean: Parameter,
    ocean: Parameter,
    coast: Parameter,
    inland: Parameter,
    near_inland: Parameter,
    mid_inland: Parameter,
    far_inland: Parameter,
}

const OCEANS: [[&str; 5]; 2] = [
    ["minecraft:deep_frozen_ocean", "minecraft:deep_cold_ocean", "minecraft:deep_ocean", "minecraft:deep_lukewarm_ocean", "minecraft:warm_ocean"],
    ["minecraft:frozen_ocean", "minecraft:cold_ocean", "minecraft:ocean", "minecraft:lukewarm_ocean", "minecraft:warm_ocean"],
];
const MIDDLE: [[&str; 5]; 5] = [
    ["minecraft:snowy_plains", "minecraft:snowy_plains", "minecraft:snowy_plains", "minecraft:snowy_taiga", "minecraft:taiga"],
    ["minecraft:plains", "minecraft:plains", "minecraft:forest", "minecraft:taiga", "minecraft:old_growth_spruce_taiga"],
    ["minecraft:flower_forest", "minecraft:plains", "minecraft:forest", "minecraft:birch_forest", "minecraft:dark_forest"],
    ["minecraft:savanna", "minecraft:savanna", "minecraft:forest", "minecraft:jungle", "minecraft:jungle"],
    ["minecraft:desert", "minecraft:desert", "minecraft:desert", "minecraft:desert", "minecraft:desert"],
];
const MIDDLE_VARIANT: [[Biome; 5]; 5] = [
    [Some("minecraft:ice_spikes"), None, Some("minecraft:snowy_taiga"), None, None],
    [Some("minecraft:dappled_forest"), None, None, None, Some("minecraft:old_growth_pine_taiga")],
    [Some("minecraft:sunflower_plains"), None, None, Some("minecraft:old_growth_birch_forest"), None],
    [None, None, Some("minecraft:plains"), Some("minecraft:sparse_jungle"), Some("minecraft:bamboo_jungle")],
    [None, None, None, None, None],
];
const PLATEAU: [[&str; 5]; 5] = [
    ["minecraft:snowy_plains", "minecraft:snowy_plains", "minecraft:snowy_plains", "minecraft:snowy_taiga", "minecraft:snowy_taiga"],
    ["minecraft:meadow", "minecraft:meadow", "minecraft:forest", "minecraft:taiga", "minecraft:old_growth_spruce_taiga"],
    ["minecraft:meadow", "minecraft:meadow", "minecraft:meadow", "minecraft:meadow", "minecraft:pale_garden"],
    ["minecraft:savanna_plateau", "minecraft:savanna_plateau", "minecraft:forest", "minecraft:forest", "minecraft:jungle"],
    ["minecraft:badlands", "minecraft:badlands", "minecraft:badlands", "minecraft:wooded_badlands", "minecraft:wooded_badlands"],
];
const PLATEAU_VARIANT: [[Biome; 5]; 5] = [
    [Some("minecraft:ice_spikes"), None, None, None, None],
    [Some("minecraft:cherry_grove"), None, Some("minecraft:meadow"), Some("minecraft:meadow"), Some("minecraft:old_growth_pine_taiga")],
    [Some("minecraft:cherry_grove"), Some("minecraft:cherry_grove"), Some("minecraft:forest"), Some("minecraft:birch_forest"), None],
    [None, None, None, None, None],
    [Some("minecraft:eroded_badlands"), Some("minecraft:eroded_badlands"), None, None, None],
];
const SHATTERED: [[Biome; 5]; 5] = [
    [Some("minecraft:windswept_gravelly_hills"), Some("minecraft:windswept_gravelly_hills"), Some("minecraft:windswept_hills"), Some("minecraft:windswept_forest"), Some("minecraft:windswept_forest")],
    [Some("minecraft:windswept_gravelly_hills"), Some("minecraft:windswept_gravelly_hills"), Some("minecraft:windswept_hills"), Some("minecraft:windswept_forest"), Some("minecraft:windswept_forest")],
    [Some("minecraft:windswept_hills"), Some("minecraft:windswept_hills"), Some("minecraft:windswept_hills"), Some("minecraft:windswept_forest"), Some("minecraft:windswept_forest")],
    [None, None, None, None, None],
    [None, None, None, None, None],
];

type Out = Vec<(ParameterPoint, &'static str)>;

impl OverworldBuilder {
    fn new() -> Self {
        let s = Parameter::span;
        let temperatures = [s(-1.0, -0.45), s(-0.45, -0.15), s(-0.15, 0.2), s(0.2, 0.55), s(0.55, 1.0)];
        Self {
            full: s(-1.0, 1.0),
            temperatures,
            humidities: [s(-1.0, -0.35), s(-0.35, -0.1), s(-0.1, 0.1), s(0.1, 0.3), s(0.3, 1.0)],
            erosions: [s(-1.0, -0.78), s(-0.78, -0.375), s(-0.375, -0.2225), s(-0.2225, 0.05), s(0.05, 0.45), s(0.45, 0.55), s(0.55, 1.0)],
            frozen: temperatures[0],
            unfrozen: Parameter::join(temperatures[1], temperatures[4]),
            mushroom_fields: s(-1.2, -1.05),
            deep_ocean: s(-1.05, -0.455),
            ocean: s(-0.455, -0.19),
            coast: s(-0.19, -0.11),
            inland: s(-0.11, 0.55),
            near_inland: s(-0.11, 0.03),
            mid_inland: s(0.03, 0.3),
            far_inland: s(0.3, 1.0),
        }
    }

    fn add_biomes(&self, out: &mut Out) {
        self.add_off_coast(out);
        self.add_inland(out);
        self.add_underground(out);
    }

    #[allow(clippy::too_many_arguments)]
    fn point(t: Parameter, h: Parameter, c: Parameter, e: Parameter, depth: Parameter, w: Parameter, offset: f32) -> ParameterPoint {
        ParameterPoint { parameters: [t, h, c, e, depth, w], offset: quantize(offset) }
    }

    #[allow(clippy::too_many_arguments)]
    fn surface(&self, out: &mut Out, t: Parameter, h: Parameter, c: Parameter, e: Parameter, w: Parameter, biome: &'static str) {
        out.push((Self::point(t, h, c, e, Parameter::point(0.0), w, 0.0), biome));
        out.push((Self::point(t, h, c, e, Parameter::point(1.0), w, 0.0), biome));
    }

    fn add_off_coast(&self, out: &mut Out) {
        self.surface(out, self.full, self.full, self.mushroom_fields, self.full, self.full, "minecraft:mushroom_fields");
        for (ti, &t) in self.temperatures.iter().enumerate() {
            self.surface(out, t, self.full, self.deep_ocean, self.full, self.full, OCEANS[0][ti]);
            self.surface(out, t, self.full, self.ocean, self.full, self.full, OCEANS[1][ti]);
        }
    }

    fn add_inland(&self, out: &mut Out) {
        let s = Parameter::span;
        self.add_mid_slice(out, s(-1.0, -0.933_333_34));
        self.add_high_slice(out, s(-0.933_333_34, -0.766_666_7));
        self.add_peaks(out, s(-0.766_666_7, -0.566_666_66));
        self.add_high_slice(out, s(-0.566_666_66, -0.4));
        self.add_mid_slice(out, s(-0.4, -0.266_666_68));
        self.add_low_slice(out, s(-0.266_666_68, -0.05));
        self.add_valleys(out, s(-0.05, 0.05));
        self.add_low_slice(out, s(0.05, 0.266_666_68));
        self.add_mid_slice(out, s(0.266_666_68, 0.4));
        self.add_high_slice(out, s(0.4, 0.566_666_66));
        self.add_peaks(out, s(0.566_666_66, 0.766_666_7));
        self.add_high_slice(out, s(0.766_666_7, 0.933_333_34));
        self.add_mid_slice(out, s(0.933_333_34, 1.0));
    }

    fn j(a: Parameter, b: Parameter) -> Parameter {
        Parameter::join(a, b)
    }

    fn add_peaks(&self, out: &mut Out, w: Parameter) {
        let e = &self.erosions;
        for (ti, &t) in self.temperatures.iter().enumerate() {
            for (hi, &h) in self.humidities.iter().enumerate() {
                let middle = self.pick_middle(ti, hi, w);
                let middle_or_badlands = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                let middle_or_badlands_or_slope = self.pick_middle_or_badlands_if_hot_or_slope_if_cold(ti, hi, w);
                let plateau = self.pick_plateau(ti, hi, w);
                let shattered = self.pick_shattered(ti, hi, w);
                let shattered_or_savanna = self.maybe_windswept_savanna(ti, hi, w, shattered);
                let peak = self.pick_peak(ti, hi, w);
                self.surface(out, t, h, Self::j(self.coast, self.far_inland), e[0], w, peak);
                self.surface(out, t, h, Self::j(self.coast, self.near_inland), e[1], w, middle_or_badlands_or_slope);
                self.surface(out, t, h, Self::j(self.mid_inland, self.far_inland), e[1], w, peak);
                self.surface(out, t, h, Self::j(self.coast, self.near_inland), Self::j(e[2], e[3]), w, middle);
                self.surface(out, t, h, Self::j(self.mid_inland, self.far_inland), e[2], w, plateau);
                self.surface(out, t, h, self.mid_inland, e[3], w, middle_or_badlands);
                self.surface(out, t, h, self.far_inland, e[3], w, plateau);
                self.surface(out, t, h, Self::j(self.coast, self.far_inland), e[4], w, middle);
                self.surface(out, t, h, Self::j(self.coast, self.near_inland), e[5], w, shattered_or_savanna);
                self.surface(out, t, h, Self::j(self.mid_inland, self.far_inland), e[5], w, shattered);
                self.surface(out, t, h, Self::j(self.coast, self.far_inland), e[6], w, middle);
            }
        }
    }

    fn add_high_slice(&self, out: &mut Out, w: Parameter) {
        let e = &self.erosions;
        for (ti, &t) in self.temperatures.iter().enumerate() {
            for (hi, &h) in self.humidities.iter().enumerate() {
                let middle = self.pick_middle(ti, hi, w);
                let middle_or_badlands = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                let middle_or_badlands_or_slope = self.pick_middle_or_badlands_if_hot_or_slope_if_cold(ti, hi, w);
                let plateau = self.pick_plateau(ti, hi, w);
                let shattered = self.pick_shattered(ti, hi, w);
                let middle_or_savanna = self.maybe_windswept_savanna(ti, hi, w, middle);
                let slope = self.pick_slope(ti, hi, w);
                let peak = self.pick_peak(ti, hi, w);
                self.surface(out, t, h, self.coast, Self::j(e[0], e[1]), w, middle);
                self.surface(out, t, h, self.near_inland, e[0], w, slope);
                self.surface(out, t, h, Self::j(self.mid_inland, self.far_inland), e[0], w, peak);
                self.surface(out, t, h, self.near_inland, e[1], w, middle_or_badlands_or_slope);
                self.surface(out, t, h, Self::j(self.mid_inland, self.far_inland), e[1], w, slope);
                self.surface(out, t, h, Self::j(self.coast, self.near_inland), Self::j(e[2], e[3]), w, middle);
                self.surface(out, t, h, Self::j(self.mid_inland, self.far_inland), e[2], w, plateau);
                self.surface(out, t, h, self.mid_inland, e[3], w, middle_or_badlands);
                self.surface(out, t, h, self.far_inland, e[3], w, plateau);
                self.surface(out, t, h, Self::j(self.coast, self.far_inland), e[4], w, middle);
                self.surface(out, t, h, Self::j(self.coast, self.near_inland), e[5], w, middle_or_savanna);
                self.surface(out, t, h, Self::j(self.mid_inland, self.far_inland), e[5], w, shattered);
                self.surface(out, t, h, Self::j(self.coast, self.far_inland), e[6], w, middle);
            }
        }
    }

    fn swamps(&self, out: &mut Out, w: Parameter) {
        let e = &self.erosions;
        self.surface(out, self.full, self.full, self.coast, Self::j(e[0], e[2]), w, "minecraft:stony_shore");
        self.surface(out, Self::j(self.temperatures[1], self.temperatures[2]), self.full, Self::j(self.near_inland, self.far_inland), e[6], w, "minecraft:swamp");
        self.surface(out, Self::j(self.temperatures[3], self.temperatures[4]), self.full, Self::j(self.near_inland, self.far_inland), e[6], w, "minecraft:mangrove_swamp");
    }

    fn add_mid_slice(&self, out: &mut Out, w: Parameter) {
        let e = &self.erosions;
        self.swamps(out, w);
        for (ti, &t) in self.temperatures.iter().enumerate() {
            for (hi, &h) in self.humidities.iter().enumerate() {
                let middle = self.pick_middle(ti, hi, w);
                let middle_or_badlands = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                let middle_or_badlands_or_slope = self.pick_middle_or_badlands_if_hot_or_slope_if_cold(ti, hi, w);
                let shattered = self.pick_shattered(ti, hi, w);
                let plateau = self.pick_plateau(ti, hi, w);
                let beach = Self::pick_beach(ti);
                let middle_or_savanna = self.maybe_windswept_savanna(ti, hi, w, middle);
                let shattered_coast = self.pick_shattered_coast(ti, hi, w);
                let slope = self.pick_slope(ti, hi, w);
                self.surface(out, t, h, Self::j(self.near_inland, self.far_inland), e[0], w, slope);
                self.surface(out, t, h, Self::j(self.near_inland, self.mid_inland), e[1], w, middle_or_badlands_or_slope);
                self.surface(out, t, h, self.far_inland, e[1], w, if ti == 0 { slope } else { plateau });
                self.surface(out, t, h, self.near_inland, e[2], w, middle);
                self.surface(out, t, h, self.mid_inland, e[2], w, middle_or_badlands);
                self.surface(out, t, h, self.far_inland, e[2], w, plateau);
                self.surface(out, t, h, Self::j(self.coast, self.near_inland), e[3], w, middle);
                self.surface(out, t, h, Self::j(self.mid_inland, self.far_inland), e[3], w, middle_or_badlands);
                if w.max < 0 {
                    self.surface(out, t, h, self.coast, e[4], w, beach);
                    self.surface(out, t, h, Self::j(self.near_inland, self.far_inland), e[4], w, middle);
                } else {
                    self.surface(out, t, h, Self::j(self.coast, self.far_inland), e[4], w, middle);
                }
                self.surface(out, t, h, self.coast, e[5], w, shattered_coast);
                self.surface(out, t, h, self.near_inland, e[5], w, middle_or_savanna);
                self.surface(out, t, h, Self::j(self.mid_inland, self.far_inland), e[5], w, shattered);
                if w.max < 0 {
                    self.surface(out, t, h, self.coast, e[6], w, beach);
                } else {
                    self.surface(out, t, h, self.coast, e[6], w, middle);
                }
                if ti == 0 {
                    self.surface(out, t, h, Self::j(self.near_inland, self.far_inland), e[6], w, middle);
                }
            }
        }
    }

    fn add_low_slice(&self, out: &mut Out, w: Parameter) {
        let e = &self.erosions;
        self.swamps(out, w);
        for (ti, &t) in self.temperatures.iter().enumerate() {
            for (hi, &h) in self.humidities.iter().enumerate() {
                let middle = self.pick_middle(ti, hi, w);
                let middle_or_badlands = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                let middle_or_badlands_or_slope = self.pick_middle_or_badlands_if_hot_or_slope_if_cold(ti, hi, w);
                let beach = Self::pick_beach(ti);
                let middle_or_savanna = self.maybe_windswept_savanna(ti, hi, w, middle);
                let shattered_coast = self.pick_shattered_coast(ti, hi, w);
                self.surface(out, t, h, self.near_inland, Self::j(e[0], e[1]), w, middle_or_badlands);
                self.surface(out, t, h, Self::j(self.mid_inland, self.far_inland), Self::j(e[0], e[1]), w, middle_or_badlands_or_slope);
                self.surface(out, t, h, self.near_inland, Self::j(e[2], e[3]), w, middle);
                self.surface(out, t, h, Self::j(self.mid_inland, self.far_inland), Self::j(e[2], e[3]), w, middle_or_badlands);
                self.surface(out, t, h, self.coast, Self::j(e[3], e[4]), w, beach);
                self.surface(out, t, h, Self::j(self.near_inland, self.far_inland), e[4], w, middle);
                self.surface(out, t, h, self.coast, e[5], w, shattered_coast);
                self.surface(out, t, h, self.near_inland, e[5], w, middle_or_savanna);
                self.surface(out, t, h, Self::j(self.mid_inland, self.far_inland), e[5], w, middle);
                self.surface(out, t, h, self.coast, e[6], w, beach);
                if ti == 0 {
                    self.surface(out, t, h, Self::j(self.near_inland, self.far_inland), e[6], w, middle);
                }
            }
        }
    }

    fn add_valleys(&self, out: &mut Out, w: Parameter) {
        let e = &self.erosions;
        let negative = w.max < 0;
        self.surface(out, self.frozen, self.full, self.coast, Self::j(e[0], e[1]), w, if negative { "minecraft:stony_shore" } else { "minecraft:frozen_river" });
        self.surface(out, self.unfrozen, self.full, self.coast, Self::j(e[0], e[1]), w, if negative { "minecraft:stony_shore" } else { "minecraft:river" });
        self.surface(out, self.frozen, self.full, self.near_inland, Self::j(e[0], e[1]), w, "minecraft:frozen_river");
        self.surface(out, self.unfrozen, self.full, self.near_inland, Self::j(e[0], e[1]), w, "minecraft:river");
        self.surface(out, self.frozen, self.full, Self::j(self.coast, self.far_inland), Self::j(e[2], e[5]), w, "minecraft:frozen_river");
        self.surface(out, self.unfrozen, self.full, Self::j(self.coast, self.far_inland), Self::j(e[2], e[5]), w, "minecraft:river");
        self.surface(out, self.frozen, self.full, self.coast, e[6], w, "minecraft:frozen_river");
        self.surface(out, self.unfrozen, self.full, self.coast, e[6], w, "minecraft:river");
        self.surface(out, Self::j(self.temperatures[1], self.temperatures[2]), self.full, Self::j(self.inland, self.far_inland), e[6], w, "minecraft:swamp");
        self.surface(out, Self::j(self.temperatures[3], self.temperatures[4]), self.full, Self::j(self.inland, self.far_inland), e[6], w, "minecraft:mangrove_swamp");
        self.surface(out, self.frozen, self.full, Self::j(self.inland, self.far_inland), e[6], w, "minecraft:frozen_river");
        for (ti, &t) in self.temperatures.iter().enumerate() {
            for (hi, &h) in self.humidities.iter().enumerate() {
                let middle_or_badlands = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                self.surface(out, t, h, Self::j(self.mid_inland, self.far_inland), Self::j(e[0], e[1]), w, middle_or_badlands);
            }
        }
    }

    fn add_underground(&self, out: &mut Out) {
        let s = Parameter::span;
        let e = &self.erosions;
        let under = |t, h, c, er, w, biome| (Self::point(t, h, c, er, s(0.2, 0.9), w, 0.0), biome);
        out.push(under(self.full, self.full, s(0.8, 1.0), self.full, self.full, "minecraft:dripstone_caves"));
        out.push(under(self.full, s(0.7, 1.0), self.full, self.full, self.full, "minecraft:lush_caves"));
        out.push(under(self.full, self.full, Self::j(self.coast, self.inland), Self::j(e[5], e[6]), s(-1.1, -0.85), "minecraft:sulfur_caves"));
        out.push((Self::point(self.full, self.full, self.full, Self::j(e[0], e[1]), Parameter::point(1.1), self.full, 0.0), "minecraft:deep_dark"));
    }

    fn pick_middle(&self, ti: usize, hi: usize, w: Parameter) -> &'static str {
        if w.max < 0 {
            return MIDDLE[ti][hi];
        }
        MIDDLE_VARIANT[ti][hi].unwrap_or(MIDDLE[ti][hi])
    }

    fn pick_middle_or_badlands_if_hot(&self, ti: usize, hi: usize, w: Parameter) -> &'static str {
        if ti == 4 { Self::pick_badlands(hi, w) } else { self.pick_middle(ti, hi, w) }
    }

    fn pick_middle_or_badlands_if_hot_or_slope_if_cold(&self, ti: usize, hi: usize, w: Parameter) -> &'static str {
        if ti == 0 { self.pick_slope(ti, hi, w) } else { self.pick_middle_or_badlands_if_hot(ti, hi, w) }
    }

    fn maybe_windswept_savanna(&self, ti: usize, hi: usize, w: Parameter, underlying: &'static str) -> &'static str {
        if ti > 1 && hi < 4 && w.max >= 0 { "minecraft:windswept_savanna" } else { underlying }
    }

    fn pick_shattered_coast(&self, ti: usize, hi: usize, w: Parameter) -> &'static str {
        let beach_or_middle = if w.max >= 0 { self.pick_middle(ti, hi, w) } else { Self::pick_beach(ti) };
        self.maybe_windswept_savanna(ti, hi, w, beach_or_middle)
    }

    fn pick_beach(ti: usize) -> &'static str {
        match ti {
            0 => "minecraft:snowy_beach",
            4 => "minecraft:desert",
            _ => "minecraft:beach",
        }
    }

    fn pick_badlands(hi: usize, w: Parameter) -> &'static str {
        if hi < 2 {
            if w.max < 0 { "minecraft:badlands" } else { "minecraft:eroded_badlands" }
        } else if hi < 3 {
            "minecraft:badlands"
        } else {
            "minecraft:wooded_badlands"
        }
    }

    fn pick_plateau(&self, ti: usize, hi: usize, w: Parameter) -> &'static str {
        if w.max >= 0 {
            if let Some(variant) = PLATEAU_VARIANT[ti][hi] {
                return variant;
            }
        }
        PLATEAU[ti][hi]
    }

    fn pick_peak(&self, ti: usize, hi: usize, w: Parameter) -> &'static str {
        if ti <= 2 {
            if w.max < 0 { "minecraft:jagged_peaks" } else { "minecraft:frozen_peaks" }
        } else if ti == 3 {
            "minecraft:stony_peaks"
        } else {
            Self::pick_badlands(hi, w)
        }
    }

    fn pick_slope(&self, ti: usize, hi: usize, w: Parameter) -> &'static str {
        if ti >= 3 {
            self.pick_plateau(ti, hi, w)
        } else if hi <= 1 {
            "minecraft:snowy_slopes"
        } else {
            "minecraft:grove"
        }
    }

    fn pick_shattered(&self, ti: usize, hi: usize, w: Parameter) -> &'static str {
        SHATTERED[ti][hi].unwrap_or_else(|| self.pick_middle(ti, hi, w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantization_truncates_like_java() {
        assert_eq!(quantize(-0.455), -4550);
        assert_eq!(quantize(0.933_333_34), 9333);
        assert_eq!(Parameter::span(-0.11, 0.03).distance(5000), 5000 - 300);
    }

    /// The ported builder reproduces the table captured from vanilla, in order.
    #[test]
    fn overworld_table_matches_vanilla_capture() {
        let Ok(paths) = minecraftoss_core::registries::DataPaths::discover() else { return };
        let root = paths.block_catalog.parent().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
        let path = root.join("research/private/vectors/overworld-biome-table-26.3.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!("skipping: {} not present", path.display());
            return;
        };
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let expected = json["entries"].as_array().unwrap();
        let list = ParameterList::overworld();
        assert_eq!(list.entries.len(), expected.len());
        for (i, ((point, biome), want)) in list.entries.iter().zip(expected).enumerate() {
            assert_eq!(*biome, want["biome"].as_str().unwrap(), "entry {i}");
            let ranges: Vec<[i64; 2]> = want["ranges"].as_array().unwrap().iter().map(|r| [r[0].as_i64().unwrap(), r[1].as_i64().unwrap()]).collect();
            let got: Vec<[i64; 2]> = point.parameters.iter().map(|p| [p.min, p.max]).collect();
            assert_eq!(got, ranges, "entry {i} ({biome})");
            assert_eq!(point.offset, want["offset"].as_i64().unwrap(), "entry {i}");
        }
    }
}
