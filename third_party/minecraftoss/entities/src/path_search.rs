//! Source-informed 26.3 PathFinder search over a caller-supplied ground terrain.
//! The terrain adapter is deliberately explicit: unsupported block path types
//! must be classified before this search can be used for them.
use glam::DVec3;
use minecraftoss_player::World;
use std::collections::HashMap;

pub type NodePos = (i32, i32, i32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cell {
    Air,
    Water,
    FullSolid,
}

pub trait PathTerrain {
    fn cell(&self, pos: NodePos) -> Option<Cell>;
    fn amphibious(&self) -> bool {
        false
    }
}

/// Explicit classifier for measured full-cube floor navigation cases.
/// Other blocks stay unsupported until their path types and shapes are known.
pub struct MeasuredFullCubeTerrain<'a, W: World + ?Sized>(pub &'a W);
impl<W: World + ?Sized> PathTerrain for MeasuredFullCubeTerrain<'_, W> {
    fn cell(&self, pos: NodePos) -> Option<Cell> {
        match self.0.block(pos).map(|block| block.id) {
            None => Some(Cell::Air),
            Some(id) if id == "minecraft:air" => Some(Cell::Air),
            Some(id)
                if matches!(
                    id.as_str(),
                    "minecraft:stone"
                        | "minecraft:dirt"
                        | "minecraft:grass_block"
                        | "minecraft:cobblestone"
                ) =>
            {
                Some(Cell::FullSolid)
            }
            Some(_) => None,
        }
    }
}

/// Shallow-water floor profile for the drowned's measured amphibious route.
/// Water is traversable while the full-cube floor and walls stay solid.
pub struct MeasuredWaterFloorTerrain<'a, W: World>(pub &'a W);
impl<W: World> PathTerrain for MeasuredWaterFloorTerrain<'_, W> {
    fn amphibious(&self) -> bool {
        true
    }

    fn cell(&self, pos: NodePos) -> Option<Cell> {
        match self.0.block(pos).map(|block| block.id) {
            None => Some(Cell::Air),
            Some(id) if id == "minecraft:air" => Some(Cell::Air),
            Some(id) if id == "minecraft:water" => Some(Cell::Water),
            Some(id)
                if matches!(
                    id.as_str(),
                    "minecraft:stone"
                        | "minecraft:dirt"
                        | "minecraft:grass_block"
                        | "minecraft:cobblestone"
                ) =>
            {
                Some(Cell::FullSolid)
            }
            Some(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FoundPath {
    pub nodes: Vec<NodePos>,
    pub reached: bool,
    pub target: NodePos,
}

#[derive(Clone, Debug)]
struct Node {
    pos: NodePos,
    heap_index: i32,
    g: f32,
    h: f32,
    f: f32,
    walked: f32,
    parent: Option<usize>,
    closed: bool,
}

impl Node {
    fn new(pos: NodePos) -> Self {
        Self {
            pos,
            heap_index: -1,
            g: 0.0,
            h: 0.0,
            f: 0.0,
            walked: 0.0,
            parent: None,
            closed: false,
        }
    }
}

#[derive(Default)]
struct Heap(Vec<usize>);

impl Heap {
    fn insert(&mut self, nodes: &mut [Node], item: usize) {
        let mut index = self.0.len();
        self.0.push(item);
        let cost = nodes[item].f;
        while index > 0 {
            let parent_index = (index - 1) >> 1;
            let parent = self.0[parent_index];
            if cost >= nodes[parent].f {
                break;
            }
            self.0[index] = parent;
            nodes[parent].heap_index = index as i32;
            index = parent_index;
        }
        self.0[index] = item;
        nodes[item].heap_index = index as i32;
    }

    fn pop(&mut self, nodes: &mut [Node]) -> usize {
        let first = self.0[0];
        let last = self.0.pop().unwrap();
        if !self.0.is_empty() {
            let mut index = 0;
            let cost = nodes[last].f;
            loop {
                let left = 1 + (index << 1);
                if left >= self.0.len() {
                    break;
                }
                let right = left + 1;
                let left_node = self.0[left];
                let left_cost = nodes[left_node].f;
                let right_node = self.0.get(right).copied();
                let right_cost = right_node.map_or(f32::INFINITY, |n| nodes[n].f);
                let (child, child_index, child_cost) = if left_cost < right_cost {
                    (left_node, left, left_cost)
                } else if let Some(right_node) = right_node {
                    (right_node, right, right_cost)
                } else {
                    break;
                };
                if child_cost >= cost {
                    break;
                }
                self.0[index] = child;
                nodes[child].heap_index = index as i32;
                index = child_index;
            }
            self.0[index] = last;
            nodes[last].heap_index = index as i32;
        }
        nodes[first].heap_index = -1;
        first
    }

    fn change_cost(&mut self, nodes: &mut [Node], item: usize, new_cost: f32) {
        let old = nodes[item].f;
        nodes[item].f = new_cost;
        let mut index = nodes[item].heap_index as usize;
        if new_cost < old {
            while index > 0 {
                let parent_index = (index - 1) >> 1;
                let parent = self.0[parent_index];
                if new_cost >= nodes[parent].f {
                    break;
                }
                self.0[index] = parent;
                nodes[parent].heap_index = index as i32;
                index = parent_index;
            }
        } else {
            loop {
                let left = 1 + (index << 1);
                if left >= self.0.len() {
                    break;
                }
                let right = left + 1;
                let left_node = self.0[left];
                let left_cost = nodes[left_node].f;
                let right_node = self.0.get(right).copied();
                let right_cost = right_node.map_or(f32::INFINITY, |n| nodes[n].f);
                let (child, child_index, child_cost) = if left_cost < right_cost {
                    (left_node, left, left_cost)
                } else if let Some(right_node) = right_node {
                    (right_node, right, right_cost)
                } else {
                    break;
                };
                if child_cost >= new_cost {
                    break;
                }
                self.0[index] = child;
                nodes[child].heap_index = index as i32;
                index = child_index;
            }
        }
        self.0[index] = item;
        nodes[item].heap_index = index as i32;
    }
}

struct Search<'a, T> {
    terrain: &'a T,
    nodes: Vec<Node>,
    by_pos: HashMap<NodePos, usize>,
    heap: Heap,
    unsupported: bool,
}

impl<'a, T: PathTerrain> Search<'a, T> {
    fn new(terrain: &'a T) -> Self {
        Self {
            terrain,
            nodes: Vec::new(),
            by_pos: HashMap::new(),
            heap: Heap::default(),
            unsupported: false,
        }
    }
    fn node(&mut self, pos: NodePos) -> usize {
        if let Some(&index) = self.by_pos.get(&pos) {
            return index;
        }
        let index = self.nodes.len();
        self.nodes.push(Node::new(pos));
        self.by_pos.insert(pos, index);
        index
    }
    fn cell(&mut self, pos: NodePos) -> Cell {
        match self.terrain.cell(pos) {
            Some(cell) => cell,
            None => {
                self.unsupported = true;
                Cell::FullSolid
            }
        }
    }
    fn kind(&mut self, pos: NodePos) -> Kind {
        // WalkNodeEvaluator classifies the mob's two-block-high occupied
        // volume. Water with air in the head cell is not a WATER node for
        // this adult drowned profile.
        if self.cell(pos) == Cell::FullSolid
            || self.cell((pos.0, pos.1 + 1, pos.2)) == Cell::FullSolid
        {
            Kind::Blocked
        } else if self.cell(pos) == Cell::Water
            && self.cell((pos.0, pos.1 + 1, pos.2)) == Cell::Water
        {
            Kind::Water
        } else if self.cell((pos.0, pos.1 - 1, pos.2)) == Cell::FullSolid {
            Kind::Walkable
        } else {
            Kind::Open
        }
    }
    fn floor(&mut self, pos: NodePos) -> f64 {
        if self.cell((pos.0, pos.1 - 1, pos.2)) == Cell::FullSolid {
            pos.1 as f64
        } else {
            (pos.1 - 1) as f64
        }
    }
    fn accepted(&mut self, pos: NodePos, jump: i32, current_floor: f64) -> Option<usize> {
        if self.floor(pos) - current_floor > 1.125 {
            return None;
        }
        match self.kind(pos) {
            Kind::Walkable => Some(self.node(pos)),
            Kind::Water => Some(self.node(pos)),
            Kind::Blocked if jump > 0 => {
                self.accepted((pos.0, pos.1 + 1, pos.2), jump - 1, current_floor)
            }
            Kind::Open => {
                for drop in 1..=4 {
                    if drop > 3 {
                        return None;
                    }
                    let below = (pos.0, pos.1 - drop, pos.2);
                    match self.kind(below) {
                        Kind::Open => continue,
                        Kind::Walkable => return Some(self.node(below)),
                        Kind::Water => return Some(self.node(below)),
                        Kind::Blocked => return None,
                    }
                }
                None
            }
            Kind::Blocked => None,
        }
    }
    fn neighbors(&mut self, from: usize) -> Vec<usize> {
        let p = self.nodes[from].pos;
        let floor = self.floor(p);
        let jump = if self.kind((p.0, p.1 + 1, p.2)) != Kind::Blocked {
            1
        } else {
            0
        };
        let dirs = [(0, -1), (1, 0), (0, 1), (-1, 0)]; // Direction.Plane.HORIZONTAL: N,E,S,W.
        let mut cardinal = [None; 4];
        let mut result = Vec::with_capacity(8);
        for (i, (dx, dz)) in dirs.into_iter().enumerate() {
            let n = self.accepted((p.0 + dx, p.1, p.2 + dz), jump, floor);
            cardinal[i] = n;
            if let Some(n) = n {
                if !self.nodes[n].closed {
                    result.push(n);
                }
            }
        }
        for i in 0..4 {
            let Some(a) = cardinal[i] else {
                continue;
            };
            let Some(b) = cardinal[(i + 1) % 4] else {
                continue;
            };
            if self.nodes[a].pos.1 > p.1 || self.nodes[b].pos.1 > p.1 {
                continue;
            }
            let (dx1, dz1) = dirs[i];
            let (dx2, dz2) = dirs[(i + 1) % 4];
            if let Some(n) = self.accepted((p.0 + dx1 + dx2, p.1, p.2 + dz1 + dz2), jump, floor) {
                if !self.nodes[n].closed {
                    result.push(n);
                }
            }
        }
        if self.terrain.amphibious() && self.kind(p) == Kind::Water {
            for dy in [1, -1] {
                let candidate = (p.0, p.1 + dy, p.2);
                if self.kind(candidate) == Kind::Water {
                    let node = self.node(candidate);
                    if !self.nodes[node].closed {
                        result.push(node);
                    }
                }
            }
        }
        result
    }

    fn malus(&mut self, pos: NodePos) -> f32 {
        if !self.terrain.amphibious() {
            return 0.0;
        }
        match self.kind(pos) {
            Kind::Walkable => 6.0,
            Kind::Water => {
                for (dx, dy, dz) in [
                    (0, -1, 0),
                    (0, 1, 0),
                    (0, 0, -1),
                    (0, 0, 1),
                    (-1, 0, 0),
                    (1, 0, 0),
                ] {
                    if self.cell((pos.0 + dx, pos.1 + dy, pos.2 + dz)) == Cell::FullSolid {
                        return 4.0;
                    }
                }
                0.0
            }
            Kind::Open | Kind::Blocked => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Open,
    Walkable,
    Water,
    Blocked,
}

fn distance(a: NodePos, b: NodePos) -> f32 {
    let x = (a.0 - b.0) as f32;
    let y = (a.1 - b.1) as f32;
    let z = (a.2 - b.2) as f32;
    (x * x + y * y + z * z).sqrt()
}

/// Pinned default-cow search limits: FOLLOW_RANGE=16, reachRange=1,
/// maxVisitedNodes=256. Returns None when the terrain cannot classify a block.
pub fn find_cow_path<T: PathTerrain>(
    terrain: &T,
    position: DVec3,
    on_ground: bool,
    target: DVec3,
) -> Option<FoundPath> {
    find_path_with_accuracy(terrain, position, on_ground, target, 1)
}

pub fn find_path_with_accuracy<T: PathTerrain>(
    terrain: &T,
    position: DVec3,
    on_ground: bool,
    target: DVec3,
    accuracy: i32,
) -> Option<FoundPath> {
    // GroundPathNavigation.canUpdatePath also permits fluids/passengers; those
    // branches require their own terrain and movement states.
    if !on_ground {
        return None;
    }
    let start = (
        position.x.floor() as i32,
        (position.y + 0.5).floor() as i32,
        position.z.floor() as i32,
    );
    let target = (
        target.x.floor() as i32,
        target.y.floor() as i32,
        target.z.floor() as i32,
    );
    let mut search = Search::new(terrain);
    let start_index = search.node(start);
    let mut best_index = start_index;
    let mut best_h = distance(start, target);
    search.nodes[start_index].h = best_h;
    search.nodes[start_index].f = best_h;
    search.heap.insert(&mut search.nodes, start_index);
    let mut reached = false;
    for _ in 1..256 {
        if search.heap.0.is_empty() {
            break;
        }
        let current = search.heap.pop(&mut search.nodes);
        search.nodes[current].closed = true;
        let p = search.nodes[current].pos;
        if (p.0 - target.0).abs() + (p.1 - target.1).abs() + (p.2 - target.2).abs() <= accuracy {
            best_index = current;
            reached = true;
            break;
        }
        if distance(p, start) >= 16.0 {
            continue;
        }
        for neighbor in search.neighbors(current) {
            let q = search.nodes[neighbor].pos;
            let step = distance(p, q);
            search.nodes[neighbor].walked = search.nodes[current].walked + step;
            let g = search.nodes[current].g + step + search.malus(q);
            if search.nodes[neighbor].walked < 16.0
                && (search.nodes[neighbor].heap_index < 0 || g < search.nodes[neighbor].g)
            {
                search.nodes[neighbor].parent = Some(current);
                search.nodes[neighbor].g = g;
                let h = distance(q, target);
                if h < best_h {
                    best_h = h;
                    best_index = neighbor;
                }
                search.nodes[neighbor].h = h * 1.5;
                let f = g + search.nodes[neighbor].h;
                if search.nodes[neighbor].heap_index >= 0 {
                    search.heap.change_cost(&mut search.nodes, neighbor, f);
                } else {
                    search.nodes[neighbor].f = f;
                    search.heap.insert(&mut search.nodes, neighbor);
                }
            }
        }
    }
    if search.unsupported {
        return None;
    }
    let mut nodes = Vec::new();
    let mut current = Some(best_index);
    while let Some(i) = current {
        nodes.push(search.nodes[i].pos);
        current = search.nodes[i].parent;
    }
    nodes.reverse();
    Some(FoundPath {
        nodes,
        reached,
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct UnknownStone;
    impl PathTerrain for UnknownStone {
        fn cell(&self, pos: NodePos) -> Option<Cell> {
            if pos == (3, 1, 4) {
                None
            } else if pos.1 == 0 {
                Some(Cell::FullSolid)
            } else {
                Some(Cell::Air)
            }
        }
    }

    #[test]
    fn search_rejects_unclassified_ground_and_off_ground_requests() {
        let start = DVec3::new(2.5, 1.0, 4.5);
        let target = DVec3::new(5.5, 1.0, 4.5);
        assert!(find_cow_path(&UnknownStone, start, false, target).is_none());
        assert!(find_cow_path(&UnknownStone, start, true, target).is_none());
    }
}
