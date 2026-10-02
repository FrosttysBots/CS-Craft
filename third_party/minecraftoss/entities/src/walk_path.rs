//! Ground path search over any world, source-informed by pinned 26.3
//! `PathFinder`, `WalkNodeEvaluator`, `NodeEvaluator`, `PathfindingContext`,
//! `Node`, `Target`, `BinaryHeap` and `BlockCollisions` (common JAR
//! 52f29c47...). Block path types, collision shapes and fluids come from the
//! world (`World::path_type_from_state` and friends), so the same search
//! runs on authored fixtures and on the catalog-backed server level.

use crate::path_search::{FoundPath, NodePos};
use glam::DVec3;
use minecraftoss_player::path_type::PathType;
use minecraftoss_player::World;
use std::collections::HashMap;

/// A mob's pathfinding settings (`Mob` maluses and size, the navigation's
/// evaluator flags and `PathNavigation`'s limits).
#[derive(Clone, Debug)]
pub struct WalkProfile {
    pub width: f32,
    pub height: f32,
    /// `LivingEntity.maxUpStep` (`STEP_HEIGHT`, 0.6 by default).
    pub max_up_step: f32,
    /// `Mob.getMaxFallDistance` (3 with no target).
    pub max_fall_distance: i32,
    pub can_float: bool,
    pub can_pass_doors: bool,
    pub can_open_doors: bool,
    pub can_walk_over_fences: bool,
    pub can_stand_on_fluid: bool,
    /// `Mob.setPathfindingMalus` overrides, by `PathType`.
    malus: [Option<f32>; PathType::COUNT],
    /// `FOLLOW_RANGE`'s base: a sixteenth of the node budget, and the
    /// longest path unless `max_path_length` says otherwise.
    pub follow_range: f32,
    /// `FOLLOW_RANGE` with its modifiers (`PathNavigation.getMaxPathLength`),
    /// when a spawn bonus makes it differ from the base.
    pub max_path_length: Option<f32>,
    /// How random positions are weighed (`PathfinderMob.getWalkTargetValue`).
    pub walk_target: WalkTarget,
    /// The node budget when `setRequiredPathLength` recounted it
    /// (`floor(max(FOLLOW_RANGE, required) * 16)`).
    pub max_visited: Option<i32>,
}

/// `PathfinderMob.getWalkTargetValue` families.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WalkTarget {
    /// `Animal`: grass blocks, then light.
    #[default]
    Animal,
    /// `Monster`: darkness.
    Monster,
    /// `Enderman.getWalkTargetValue`: every spot alike.
    Zero,
}

impl WalkProfile {
    /// An `Animal`: fire in a neighbour costs 16, fire itself is avoided;
    /// its `FloatGoal` lets it float; follow range 16.
    pub fn animal(width: f32, height: f32) -> Self {
        let mut profile = Self {
            width,
            height,
            max_up_step: 0.6,
            max_fall_distance: 3,
            can_float: true,
            can_pass_doors: true,
            can_open_doors: false,
            can_walk_over_fences: false,
            can_stand_on_fluid: false,
            malus: [None; PathType::COUNT],
            follow_range: 16.0,
            max_path_length: None,
            walk_target: WalkTarget::Animal,
            max_visited: None,
        };
        profile.set_malus(PathType::FireInNeighbor, 16.0);
        profile.set_malus(PathType::Fire, -1.0);
        profile
    }

    pub fn set_malus(&mut self, path_type: PathType, malus: f32) {
        self.malus[path_type as usize] = Some(malus);
    }

    pub fn clear_malus(&mut self, path_type: PathType) {
        self.malus[path_type as usize] = None;
    }

    /// `Mob.getPathfindingMalus`.
    pub fn malus(&self, path_type: PathType) -> f32 {
        self.malus[path_type as usize].unwrap_or(path_type.default_malus())
    }

    /// `PathNavigation`'s node budget: `floor(FOLLOW_RANGE * 16)`.
    pub fn max_visited_nodes(&self) -> i32 {
        self.max_visited.unwrap_or((f64::from(self.follow_range) * 16.0).floor() as i32)
    }
}

/// The mob a search starts from.
#[derive(Clone, Copy, Debug)]
pub struct Walker {
    pub position: DVec3,
    pub on_ground: bool,
    /// `Mob.isInFloatableFluid` (touching water deep enough to float).
    pub in_floatable_fluid: bool,
}

#[derive(Clone, Debug)]
struct Node {
    pos: NodePos,
    heap_index: i32,
    g: f32,
    h: f32,
    f: f32,
    walked: f32,
    came_from: Option<usize>,
    closed: bool,
    cost_malus: f32,
    path_type: Option<PathType>,
}

impl Node {
    fn new(pos: NodePos) -> Self {
        Self { pos, heap_index: -1, g: 0.0, h: 0.0, f: 0.0, walked: 0.0, came_from: None, closed: false, cost_malus: 0.0, path_type: None }
    }
}

/// `Node.createHash`: nodes share an entry when their Y differ by 256.
fn node_hash((x, y, z): NodePos) -> i32 {
    (y & 0xFF) | ((x & 32767) << 8) | ((z & 32767) << 24) | (if x < 0 { i32::MIN } else { 0 }) | (if z < 0 { 32768 } else { 0 })
}

fn distance(a: NodePos, b: NodePos) -> f32 {
    let x = (b.0 - a.0) as f32;
    let y = (b.1 - a.1) as f32;
    let z = (b.2 - a.2) as f32;
    (x * x + y * y + z * z).sqrt()
}

fn distance_manhattan(a: NodePos, b: NodePos) -> f32 {
    ((b.0 - a.0).abs() as f32) + ((b.1 - a.1).abs() as f32) + ((b.2 - a.2).abs() as f32)
}

/// `BinaryHeap` over node indices, ordered by `f`.
#[derive(Default)]
struct Heap(Vec<usize>);

impl Heap {
    fn insert(&mut self, nodes: &mut [Node], item: usize) {
        self.0.push(item);
        nodes[item].heap_index = (self.0.len() - 1) as i32;
        self.up_heap(nodes, self.0.len() - 1);
    }

    fn pop(&mut self, nodes: &mut [Node]) -> usize {
        let top = self.0[0];
        let last = self.0.pop().expect("non-empty heap");
        if !self.0.is_empty() {
            self.0[0] = last;
            nodes[last].heap_index = 0;
            self.down_heap(nodes, 0);
        }
        nodes[top].heap_index = -1;
        top
    }

    fn change_cost(&mut self, nodes: &mut [Node], item: usize, cost: f32) {
        let old = nodes[item].f;
        nodes[item].f = cost;
        let index = nodes[item].heap_index as usize;
        if cost < old {
            self.up_heap(nodes, index);
        } else {
            self.down_heap(nodes, index);
        }
    }

    fn up_heap(&mut self, nodes: &mut [Node], mut index: usize) {
        let item = self.0[index];
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

    fn down_heap(&mut self, nodes: &mut [Node], mut index: usize) {
        let item = self.0[index];
        let cost = nodes[item].f;
        loop {
            let left_index = 1 + (index << 1);
            let right_index = left_index + 1;
            if left_index >= self.0.len() {
                break;
            }
            let left = self.0[left_index];
            let left_cost = nodes[left].f;
            let (child_index, child, child_cost) = if right_index >= self.0.len() {
                (left_index, left, left_cost)
            } else {
                let right = self.0[right_index];
                let right_cost = nodes[right].f;
                if left_cost < right_cost { (left_index, left, left_cost) } else { (right_index, right, right_cost) }
            };
            if child_cost >= cost {
                break;
            }
            self.0[index] = child;
            nodes[child].heap_index = index as i32;
            index = child_index;
        }
        self.0[index] = item;
        nodes[item].heap_index = index as i32;
    }
}

/// An axis-aligned box, compared by its exact coordinates for the
/// collision cache.
#[derive(Clone, Copy, Debug)]
struct Aabb {
    min: DVec3,
    max: DVec3,
}

impl Aabb {
    /// `new AABB(x1, y1, z1, x2, y2, z2)` orders each axis.
    fn new(a: DVec3, b: DVec3) -> Self {
        Self { min: a.min(b), max: a.max(b) }
    }

    fn key(self) -> [u64; 6] {
        [self.min.x.to_bits(), self.min.y.to_bits(), self.min.z.to_bits(), self.max.x.to_bits(), self.max.y.to_bits(), self.max.z.to_bits()]
    }
}

/// `WalkNodeEvaluator` for one search.
struct Evaluator<'a, W: World + ?Sized> {
    world: &'a W,
    profile: &'a WalkProfile,
    walker: Walker,
    nodes: Vec<Node>,
    by_hash: HashMap<i32, usize>,
    path_types: HashMap<NodePos, PathType>,
    /// `getPathTypeStatic` by position: the world holds still during a
    /// search, so each position is classified once.
    static_types: HashMap<NodePos, PathType>,
    collisions: HashMap<[u64; 6], bool>,
    entity_width: i32,
    entity_height: i32,
    entity_depth: i32,
}

/// `Direction.Plane.HORIZONTAL`: north, east, south, west.
const HORIZONTAL: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

impl<'a, W: World + ?Sized> Evaluator<'a, W> {
    fn new(world: &'a W, profile: &'a WalkProfile, walker: Walker) -> Self {
        let width = (profile.width + 1.0).floor() as i32;
        Self {
            world,
            profile,
            walker,
            nodes: Vec::new(),
            by_hash: HashMap::new(),
            path_types: HashMap::new(),
            static_types: HashMap::new(),
            collisions: HashMap::new(),
            entity_width: width,
            entity_height: (profile.height + 1.0).floor() as i32,
            entity_depth: width,
        }
    }

    fn node(&mut self, pos: NodePos) -> usize {
        let hash = node_hash(pos);
        if let Some(&index) = self.by_hash.get(&hash) {
            return index;
        }
        let index = self.nodes.len();
        self.nodes.push(Node::new(pos));
        self.by_hash.insert(hash, index);
        index
    }

    fn block_pos(&self) -> NodePos {
        let p = self.walker.position;
        (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
    }

    fn bounding_box(&self) -> Aabb {
        let half = f64::from(self.profile.width) / 2.0;
        let p = self.walker.position;
        Aabb { min: DVec3::new(p.x - half, p.y, p.z - half), max: DVec3::new(p.x + half, p.y + f64::from(self.profile.height), p.z + half) }
    }

    /// `WalkNodeEvaluator.getStart`.
    fn start(&mut self) -> usize {
        let p = self.walker.position;
        let (bx, _, bz) = self.block_pos();
        let mut start_y = p.y.floor() as i32;
        let at = |y: i32| (p.x.floor() as i32, y, p.z.floor() as i32);
        if !(self.profile.can_stand_on_fluid && self.world.floatable_fluid(at(start_y))) {
            if self.profile.can_float && self.walker.in_floatable_fluid {
                while self.world.floatable_fluid(at(start_y)) {
                    start_y += 1;
                }
                start_y -= 1;
            } else if self.walker.on_ground {
                start_y = (p.y + 0.5).floor() as i32;
            } else {
                let mut y = (p.y + 1.0).floor() as i32;
                while y > self.world.min_y() {
                    start_y = y;
                    y -= 1;
                    let below = at(y);
                    // Not air and not pathfindable: a blocking floor.
                    if self.world.block(below).is_some() && !self.world.pathfindable(below) {
                        break;
                    }
                }
            }
        } else {
            while self.world.floatable_fluid(at(start_y)) {
                start_y += 1;
            }
            start_y -= 1;
        }
        if !self.can_start_at((bx, start_y, bz)) {
            let bb = self.bounding_box();
            for (x, z) in [(bb.min.x, bb.min.z), (bb.min.x, bb.max.z), (bb.max.x, bb.min.z), (bb.max.x, bb.max.z)] {
                let pos = (x.floor() as i32, start_y, z.floor() as i32);
                if self.can_start_at(pos) {
                    return self.start_node(pos);
                }
            }
        }
        self.start_node((bx, start_y, bz))
    }

    fn start_node(&mut self, pos: NodePos) -> usize {
        let node = self.node(pos);
        let pos = self.nodes[node].pos;
        let path_type = self.cached_path_type(pos);
        self.nodes[node].path_type = Some(path_type);
        self.nodes[node].cost_malus = self.profile.malus(path_type);
        node
    }

    fn can_start_at(&mut self, pos: NodePos) -> bool {
        let path_type = self.cached_path_type(pos);
        path_type != PathType::Open && self.profile.malus(path_type) >= 0.0
    }

    /// `WalkNodeEvaluator.getNeighbors`.
    fn neighbors(&mut self, current: usize) -> Vec<usize> {
        let pos = self.nodes[current].pos;
        let mut out = Vec::with_capacity(8);
        let mut jump_size = 0;
        let above = self.cached_path_type((pos.0, pos.1 + 1, pos.2));
        let here = self.cached_path_type(pos);
        if self.profile.malus(above) >= 0.0 && here != PathType::StickyHoney {
            jump_size = (1.0f32.max(self.profile.max_up_step)).floor() as i32;
        }
        let height = self.floor_level(pos);
        let mut sides = [None; 4];
        for (i, (dx, dz)) in HORIZONTAL.into_iter().enumerate() {
            let node = self.find_accepted_node((pos.0 + dx, pos.1, pos.2 + dz), jump_size, height, (dx, dz), here);
            sides[i] = node;
            if self.neighbor_valid(node, current) {
                out.push(node.expect("valid"));
            }
        }
        for i in 0..4 {
            let clockwise = (i + 1) % 4;
            if self.diagonal_valid(current, sides[i], sides[clockwise]) {
                let (dx, dz) = HORIZONTAL[i];
                let (cx, cz) = HORIZONTAL[clockwise];
                let node = self.find_accepted_node((pos.0 + dx + cx, pos.1, pos.2 + dz + cz), jump_size, height, (dx, dz), here);
                if self.diagonal_node_valid(node) {
                    out.push(node.expect("valid"));
                }
            }
        }
        out
    }

    fn neighbor_valid(&self, neighbor: Option<usize>, current: usize) -> bool {
        neighbor.is_some_and(|n| !self.nodes[n].closed && (self.nodes[n].cost_malus >= 0.0 || self.nodes[current].cost_malus < 0.0))
    }

    fn diagonal_valid(&self, current: usize, ew: Option<usize>, ns: Option<usize>) -> bool {
        let (Some(ew), Some(ns)) = (ew, ns) else { return false };
        let pos = self.nodes[current].pos;
        let (ew, ns) = (&self.nodes[ew], &self.nodes[ns]);
        if ns.pos.1 > pos.1 || ew.pos.1 > pos.1 {
            return false;
        }
        if ew.path_type == Some(PathType::WalkableDoor) || ns.path_type == Some(PathType::WalkableDoor) {
            return false;
        }
        if self.profile.width > 1.0 && (ew.cost_malus > 0.0 || ns.cost_malus > 0.0) {
            return false;
        }
        let between_posts = ns.path_type == Some(PathType::Fence) && ew.path_type == Some(PathType::Fence) && self.profile.width < 0.5;
        (ns.pos.1 < pos.1 || ns.cost_malus >= 0.0 || between_posts) && (ew.pos.1 < pos.1 || ew.cost_malus >= 0.0 || between_posts)
    }

    fn diagonal_node_valid(&self, diagonal: Option<usize>) -> bool {
        diagonal.is_some_and(|d| {
            let node = &self.nodes[d];
            !node.closed && node.path_type != Some(PathType::WalkableDoor) && node.cost_malus >= 0.0
        })
    }

    fn partial_collision(path_type: PathType) -> bool {
        matches!(path_type, PathType::Fence | PathType::DoorWoodClosed | PathType::DoorIronClosed)
    }

    /// `canReachWithoutCollision`: the mob's box stepped toward the node.
    fn can_reach_without_collision(&mut self, node: usize) -> bool {
        let mut bb = self.bounding_box();
        let (nx, ny, nz) = self.nodes[node].pos;
        let p = self.walker.position;
        let size = bb.max - bb.min;
        let mut delta = DVec3::new(f64::from(nx) - p.x + size.x / 2.0, f64::from(ny) - p.y + size.y / 2.0, f64::from(nz) - p.z + size.z / 2.0);
        let box_size = (size.x + size.y + size.z) / 3.0;
        let steps = (delta.length() / box_size).ceil() as i32;
        // `delta.scale(1.0F / steps)`: the factor is a float.
        delta *= f64::from(1.0f32 / steps as f32);
        for _ in 1..=steps {
            bb = Aabb { min: bb.min + delta, max: bb.max + delta };
            if self.has_collisions(bb) {
                return false;
            }
        }
        true
    }

    /// `getFloorLevel`: half a block up in floatable fluid for a floating
    /// mob, else the top of the collision shape below.
    fn floor_level(&self, pos: NodePos) -> f64 {
        if self.profile.can_float && self.world.floatable_fluid(pos) {
            return f64::from(pos.1) + 0.5;
        }
        static_floor_level(self.world, pos)
    }

    /// `findAcceptedNode`.
    fn find_accepted_node(&mut self, (x, y, z): NodePos, jump_size: i32, node_height: f64, travel: (i32, i32), current: PathType) -> Option<usize> {
        let max_y_target = self.floor_level((x, y, z));
        if max_y_target - node_height > self.jump_height() {
            return None;
        }
        let path_type = self.cached_path_type((x, y, z));
        let cost = self.profile.malus(path_type);
        let mut best = None;
        if cost >= 0.0 {
            best = Some(self.node_with_cost((x, y, z), path_type, cost));
        }
        if Self::partial_collision(current) && best.is_some_and(|b| self.nodes[b].cost_malus >= 0.0) && !self.can_reach_without_collision(best.expect("checked")) {
            best = None;
        }
        if path_type == PathType::Walkable {
            return best;
        }
        if (best.is_none() || best.is_some_and(|b| self.nodes[b].cost_malus < 0.0))
            && jump_size > 0
            && (path_type != PathType::Fence || self.profile.can_walk_over_fences)
            && path_type != PathType::UnpassableRail
            && path_type != PathType::Trapdoor
            && path_type != PathType::PowderSnow
        {
            best = self.try_jump_on((x, y, z), jump_size, node_height, travel, current);
        } else if path_type == PathType::Water && !self.profile.can_float {
            best = self.first_non_water_below((x, y, z), best);
        } else if path_type == PathType::Open {
            best = Some(self.first_ground_node_below((x, y, z)));
        } else if Self::partial_collision(path_type) && best.is_none() {
            best = Some(self.closed_node((x, y, z), path_type));
        }
        best
    }

    /// `getMobJumpHeight`.
    fn jump_height(&self) -> f64 {
        1.125f64.max(f64::from(self.profile.max_up_step))
    }

    fn node_with_cost(&mut self, pos: NodePos, path_type: PathType, cost: f32) -> usize {
        let node = self.node(pos);
        self.nodes[node].path_type = Some(path_type);
        self.nodes[node].cost_malus = self.nodes[node].cost_malus.max(cost);
        node
    }

    fn blocked_node(&mut self, pos: NodePos) -> usize {
        let node = self.node(pos);
        self.nodes[node].path_type = Some(PathType::Blocked);
        self.nodes[node].cost_malus = -1.0;
        node
    }

    fn closed_node(&mut self, pos: NodePos, path_type: PathType) -> usize {
        let node = self.node(pos);
        self.nodes[node].closed = true;
        self.nodes[node].path_type = Some(path_type);
        self.nodes[node].cost_malus = path_type.default_malus();
        node
    }

    /// `tryJumpOn`.
    fn try_jump_on(&mut self, (x, y, z): NodePos, jump_size: i32, node_height: f64, travel: (i32, i32), current: PathType) -> Option<usize> {
        let above = self.find_accepted_node((x, y + 1, z), jump_size - 1, node_height, travel, current)?;
        if self.profile.width >= 1.0 {
            return Some(above);
        }
        let above_type = self.nodes[above].path_type;
        if above_type != Some(PathType::Open) && above_type != Some(PathType::Walkable) {
            return Some(above);
        }
        let center_x = f64::from(x - travel.0) + 0.5;
        let center_z = f64::from(z - travel.1) + 0.5;
        let half = f64::from(self.profile.width) / 2.0;
        let below_center = (center_x.floor() as i32, y + 1, center_z.floor() as i32);
        let above_pos = self.nodes[above].pos;
        let grow = Aabb::new(
            DVec3::new(center_x - half, self.floor_level(below_center) + 0.001, center_z - half),
            DVec3::new(center_x + half, f64::from(self.profile.height) + self.floor_level(above_pos) - 0.002, center_z + half),
        );
        if self.has_collisions(grow) {
            None
        } else {
            Some(above)
        }
    }

    /// `tryFindFirstNonWaterBelow`.
    fn first_non_water_below(&mut self, (x, mut y, z): NodePos, mut best: Option<usize>) -> Option<usize> {
        y -= 1;
        while y > self.world.min_y() {
            let path_type = self.cached_path_type((x, y, z));
            if path_type != PathType::Water {
                return best;
            }
            let cost = self.profile.malus(path_type);
            best = Some(self.node_with_cost((x, y, z), path_type, cost));
            y -= 1;
        }
        best
    }

    /// `tryFindFirstGroundNodeBelow`.
    fn first_ground_node_below(&mut self, (x, y, z): NodePos) -> usize {
        let mut current_y = y - 1;
        while current_y >= self.world.min_y() {
            if y - current_y > self.profile.max_fall_distance {
                return self.blocked_node((x, current_y, z));
            }
            let path_type = self.cached_path_type((x, current_y, z));
            let cost = self.profile.malus(path_type);
            if path_type != PathType::Open {
                if cost >= 0.0 {
                    return self.node_with_cost((x, current_y, z), path_type, cost);
                }
                return self.blocked_node((x, current_y, z));
            }
            current_y -= 1;
        }
        self.blocked_node((x, y, z))
    }

    /// `hasCollisions`: `CollisionGetter.noCollision` over block shapes,
    /// cached per box for the search.
    fn has_collisions(&mut self, aabb: Aabb) -> bool {
        let key = aabb.key();
        if let Some(&hit) = self.collisions.get(&key) {
            return hit;
        }
        let hit = block_collides(self.world, aabb);
        self.collisions.insert(key, hit);
        hit
    }

    fn static_type(&mut self, pos: NodePos) -> PathType {
        if let Some(&path_type) = self.static_types.get(&pos) {
            return path_type;
        }
        let path_type = path_type_static(self.world, pos);
        self.static_types.insert(pos, path_type);
        path_type
    }

    /// `getCachedPathType`.
    fn cached_path_type(&mut self, pos: NodePos) -> PathType {
        if let Some(&path_type) = self.path_types.get(&pos) {
            return path_type;
        }
        let path_type = self.path_type_of_mob(pos);
        self.path_types.insert(pos, path_type);
        path_type
    }

    /// `getPathTypeOfMob`.
    fn path_type_of_mob(&mut self, (x, y, z): NodePos) -> PathType {
        let types = self.path_types_within_mob_box((x, y, z));
        if types.len() == 1 {
            return types[0];
        }
        if types.contains(&PathType::Fence) {
            return PathType::Fence;
        }
        if types.contains(&PathType::UnpassableRail) {
            return PathType::UnpassableRail;
        }
        let mut highest = PathType::Blocked;
        let mut highest_malus = self.profile.malus(highest);
        for &path_type in &types {
            let malus = self.profile.malus(path_type);
            if malus < 0.0 {
                return path_type;
            }
            if malus >= highest_malus {
                highest_malus = malus;
                highest = path_type;
            }
        }
        let current = self.static_type((x, y, z));
        if self.entity_width > 1 {
            let cheaper = self.profile.malus(current) < highest_malus;
            let cap = cheaper && self.profile.malus(PathType::BigMobsCloseToDanger) < highest_malus;
            if cap { PathType::BigMobsCloseToDanger } else { highest }
        } else if current == PathType::Open && highest != PathType::Open && highest_malus == 0.0 {
            PathType::Open
        } else {
            highest
        }
    }

    /// `getPathTypeWithinMobBB`, in `EnumSet` (declaration) order.
    fn path_types_within_mob_box(&mut self, (x, y, z): NodePos) -> Vec<PathType> {
        let mut types = Vec::new();
        let mob = self.block_pos();
        for dx in 0..self.entity_width {
            for dy in 0..self.entity_height {
                for dz in 0..self.entity_depth {
                    let mut path_type = self.static_type((x + dx, y + dy, z + dz));
                    if path_type == PathType::DoorWoodClosed && self.profile.can_open_doors && self.profile.can_pass_doors {
                        path_type = PathType::WalkableDoor;
                    }
                    if path_type == PathType::DoorOpen && !self.profile.can_pass_doors {
                        path_type = PathType::Blocked;
                    }
                    if path_type == PathType::Rail && self.static_type(mob) != PathType::Rail && self.static_type((mob.0, mob.1 - 1, mob.2)) != PathType::Rail {
                        path_type = PathType::UnpassableRail;
                    }
                    if !types.contains(&path_type) {
                        types.push(path_type);
                    }
                }
            }
        }
        types.sort();
        types
    }
}

/// `WalkNodeEvaluator.getFloorLevel(level, pos)`: the top of the collision
/// shape below, or that block's own Y when it has none.
pub fn static_floor_level<W: World + ?Sized>(world: &W, (x, y, z): NodePos) -> f64 {
    let below = (x, y - 1, z);
    let top = world.collision_boxes(below).iter().map(|b| b[4]).fold(f64::NEG_INFINITY, f64::max);
    f64::from(below.1) + if top == f64::NEG_INFINITY { 0.0 } else { top }
}

/// `WalkNodeEvaluator.getPathTypeStatic`.
pub fn path_type_static<W: World + ?Sized>(world: &W, (x, y, z): NodePos) -> PathType {
    let path_type = world.path_type_from_state((x, y, z));
    if path_type != PathType::Open || y < world.min_y() + 1 {
        return path_type;
    }
    match world.path_type_from_state((x, y - 1, z)) {
        PathType::Open | PathType::Water | PathType::Lava | PathType::Walkable => PathType::Open,
        PathType::Fire => PathType::Fire,
        PathType::Damaging => PathType::Damaging,
        PathType::StickyHoney => PathType::StickyHoney,
        PathType::PowderSnow => PathType::OnTopOfPowderSnow,
        PathType::DamageCautious => PathType::DamageCautious,
        PathType::Trapdoor => PathType::OnTopOfTrapdoor,
        _ => check_neighbour_blocks(world, (x, y, z), PathType::Walkable),
    }
}

/// `WalkNodeEvaluator.checkNeighbourBlocks`.
pub fn check_neighbour_blocks<W: World + ?Sized>(world: &W, (x, y, z): NodePos, path_type: PathType) -> PathType {
    for dx in -1..=1 {
        for dy in -1..=1 {
            for dz in -1..=1 {
                if dx == 0 && dz == 0 {
                    continue;
                }
                match world.path_type_from_state((x + dx, y + dy, z + dz)) {
                    PathType::Damaging => return PathType::DamagingInNeighbor,
                    PathType::Fire | PathType::Lava => return PathType::FireInNeighbor,
                    PathType::Water => return PathType::WaterBorder,
                    PathType::DamageCautious => return PathType::DamageCautious,
                    _ => {}
                }
            }
        }
    }
    path_type
}

/// Whether block collision shapes overlap a box (`BlockCollisions` with
/// `Cursor3D`'s edge rules: corner cells are skipped, edge cells count
/// only moving pistons, face cells only shapes larger than their cell).
fn block_collides<W: World + ?Sized>(world: &W, aabb: Aabb) -> bool {
    let x0 = (aabb.min.x - 1.0e-7).floor() as i32 - 1;
    let x1 = (aabb.max.x + 1.0e-7).floor() as i32 + 1;
    let y0 = (aabb.min.y - 1.0e-7).floor() as i32 - 1;
    let y1 = (aabb.max.y + 1.0e-7).floor() as i32 + 1;
    let z0 = (aabb.min.z - 1.0e-7).floor() as i32 - 1;
    let z1 = (aabb.max.z + 1.0e-7).floor() as i32 + 1;
    for y in y0..=y1 {
        for z in z0..=z1 {
            for x in x0..=x1 {
                let edges = usize::from(x == x0 || x == x1) + usize::from(y == y0 || y == y1) + usize::from(z == z0 || z == z1);
                if edges == 3 {
                    continue;
                }
                let boxes = world.collision_boxes((x, y, z));
                if boxes.is_empty() {
                    continue;
                }
                if edges == 2 && world.block((x, y, z)).is_none_or(|b| b.id != "minecraft:moving_piston") {
                    continue;
                }
                let large = boxes.iter().any(|b| b[0] < 0.0 || b[1] < 0.0 || b[2] < 0.0 || b[3] > 1.0 || b[4] > 1.0 || b[5] > 1.0);
                if edges == 1 && !large {
                    continue;
                }
                let (ox, oy, oz) = (f64::from(x), f64::from(y), f64::from(z));
                let hit = boxes.iter().any(|b| {
                    aabb.min.x < b[3] + ox && aabb.max.x > b[0] + ox && aabb.min.y < b[4] + oy && aabb.max.y > b[1] + oy && aabb.min.z < b[5] + oz && aabb.max.z > b[2] + oz
                });
                if hit {
                    return true;
                }
            }
        }
    }
    false
}

/// `PathFinder.findPath` for one target, as `PathNavigation.createPath`
/// asks it (`maxPathLength` = follow range, the node budget from it).
pub fn find_walk_path<W: World + ?Sized>(world: &W, profile: &WalkProfile, walker: Walker, target: DVec3, reach_range: i32) -> FoundPath {
    let target_pos = (target.x.floor() as i32, target.y.floor() as i32, target.z.floor() as i32);
    find_walk_path_to_any(world, profile, walker, &[target_pos], reach_range).expect("one target gives a path")
}

/// The JDK's iteration order of hash keys inserted in `order` into a table
/// of `size` buckets (no chain long enough to matter): by bucket of the
/// spread hash, insertion order within one.
fn java_hash_order<T: Copy>(order: &[T], hash: impl Fn(T) -> i32, size: usize) -> Vec<T> {
    let mut keyed: Vec<(usize, usize, T)> = order.iter().enumerate().map(|(i, &t)| {
        let h = hash(t);
        ((h ^ ((h as u32) >> 16) as i32) as u32 as usize & (size - 1), i, t)
    }).collect();
    keyed.sort_by_key(|&(bucket, i, _)| (bucket, i));
    keyed.into_iter().map(|(_, _, t)| t).collect()
}

/// `PathFinder.findPath` towards several targets (`PathNavigation.createPath(Set<BlockPos>, reachRange)`),
/// the targets given in their `HashSet<BlockPos>`'s order. The search
/// stops at the first node within reach of any; the path goes to the
/// reached target's nearest node with the fewest nodes, or when none is
/// reached, to the nearest node of the target it ends closest to. Each
/// target keeps the node nearest it by raw distance (`Target.updateBest`).
pub fn find_walk_path_to_any<W: World + ?Sized>(world: &W, profile: &WalkProfile, walker: Walker, targets: &[NodePos], reach_range: i32) -> Option<FoundPath> {
    if targets.is_empty() {
        return None;
    }
    let mut evaluator = Evaluator::new(world, profile, walker);
    let start = evaluator.start();
    // `Collectors.toMap` of `getTarget`: the targets' nodes are made before
    // the search, and the map (16 buckets) orders them by node hash.
    let mut keys = Vec::with_capacity(targets.len());
    for &pos in targets {
        let node = evaluator.node(pos);
        keys.push((evaluator.nodes[node].pos, pos));
    }
    let keys = java_hash_order(&keys, |(at, _)| node_hash(at), 16);
    let max_path_length = profile.max_path_length.unwrap_or(profile.follow_range);
    let max_visited = profile.max_visited_nodes();
    let mut best: Vec<(f32, usize)> = vec![(f32::MAX, start); keys.len()];
    let best_h = |nodes: &[Node], node: usize, best: &mut [(f32, usize)]| {
        let mut min = f32::MAX;
        for (i, &(at, _)) in keys.iter().enumerate() {
            let h = distance(nodes[node].pos, at);
            if h < best[i].0 {
                best[i] = (h, node);
            }
            min = min.min(h);
        }
        min
    };
    let mut heap = Heap::default();
    let h = best_h(&evaluator.nodes, start, &mut best);
    evaluator.nodes[start].g = 0.0;
    evaluator.nodes[start].h = h;
    evaluator.nodes[start].f = h;
    heap.insert(&mut evaluator.nodes, start);
    let start_pos = evaluator.nodes[start].pos;
    let mut reached: Vec<usize> = Vec::new();
    let mut count = 0;
    while !heap.0.is_empty() {
        count += 1;
        if count >= max_visited {
            break;
        }
        let current = heap.pop(&mut evaluator.nodes);
        evaluator.nodes[current].closed = true;
        for (i, &(at, _)) in keys.iter().enumerate() {
            if distance_manhattan(evaluator.nodes[current].pos, at) <= reach_range as f32 && !reached.contains(&i) {
                reached.push(i);
            }
        }
        if !reached.is_empty() {
            break;
        }
        if distance(evaluator.nodes[current].pos, start_pos) >= max_path_length {
            continue;
        }
        for neighbor in evaluator.neighbors(current) {
            let step = distance(evaluator.nodes[current].pos, evaluator.nodes[neighbor].pos);
            evaluator.nodes[neighbor].walked = evaluator.nodes[current].walked + step;
            let g = evaluator.nodes[current].g + step + evaluator.nodes[neighbor].cost_malus;
            if evaluator.nodes[neighbor].walked < max_path_length && (evaluator.nodes[neighbor].heap_index < 0 || g < evaluator.nodes[neighbor].g) {
                evaluator.nodes[neighbor].came_from = Some(current);
                evaluator.nodes[neighbor].g = g;
                let h = best_h(&evaluator.nodes, neighbor, &mut best) * 1.5;
                evaluator.nodes[neighbor].h = h;
                if evaluator.nodes[neighbor].heap_index >= 0 {
                    let f = g + h;
                    heap.change_cost(&mut evaluator.nodes, neighbor, f);
                } else {
                    evaluator.nodes[neighbor].f = g + h;
                    heap.insert(&mut evaluator.nodes, neighbor);
                }
            }
        }
    }
    let reconstruct = |i: usize| {
        let mut nodes = Vec::new();
        let mut current = Some(best[i].1);
        while let Some(index) = current {
            nodes.push(evaluator.nodes[index].pos);
            current = evaluator.nodes[index].came_from;
        }
        nodes.reverse();
        nodes
    };
    let (chosen, is_reached) = if reached.is_empty() {
        // `min(distToTarget, then node count)` in the map's order.
        let mut chosen: Option<(f32, usize, usize)> = None;
        for i in 0..keys.len() {
            let nodes = reconstruct(i);
            let to_target = nodes.last().map_or(f32::MAX, |&last| distance_manhattan(last, keys[i].1));
            if chosen.is_none_or(|(d, n, _)| to_target < d || (to_target == d && nodes.len() < n)) {
                chosen = Some((to_target, nodes.len(), i));
            }
        }
        (chosen?.2, false)
    } else {
        // The reached set (`newHashSetWithExpectedSize`) in its own order,
        // the fewest nodes first.
        // Guava's `Maps.capacity`: n + 1 below 3, else ceil(n / 0.75).
        let size = match keys.len() {
            n @ 0..=2 => (n + 1).next_power_of_two(),
            n => ((n as f64 / 0.75).ceil() as usize).next_power_of_two(),
        };
        let order = java_hash_order(&reached, |i| node_hash(keys[i].0), size);
        let mut chosen: Option<(usize, usize)> = None;
        for i in order {
            let n = reconstruct(i).len();
            if chosen.is_none_or(|(m, _)| n < m) {
                chosen = Some((n, i));
            }
        }
        (chosen?.1, true)
    };
    Some(FoundPath { nodes: reconstruct(chosen), reached: is_reached, target: keys[chosen].1 })
}
