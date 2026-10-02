//! Ground path following from pinned 26.3 PathNavigation/GroundPathNavigation.
//! Path search and node classification are separate future components.
use crate::movement::Body;
use crate::fluid::FluidFrame;
use crate::path_search::{find_path_with_accuracy, PathTerrain};
use crate::walk_path::{find_walk_path, find_walk_path_to_any, WalkProfile, Walker};
use glam::DVec3;
use minecraftoss_player::World;

/// `GroundPathNavigation.findSurfacePosition`: a target in the air drops to
/// the ground below it (or rises to the first block above when the column
/// is empty below), and a target inside solid blocks rises out of them.
fn find_surface_position<W: World + ?Sized>(world: &W, (x, y, z): (i32, i32, i32)) -> (i32, i32, i32) {
    let mut pos = (x, y, z);
    if world.block(pos).is_none() {
        let mut column = y - 1;
        while column >= world.min_y() && world.block((x, column, z)).is_none() {
            column -= 1;
        }
        if column >= world.min_y() {
            return (x, column + 1, z);
        }
        column = y + 1;
        while column <= world.max_y() && world.block((x, column, z)).is_none() {
            column += 1;
        }
        pos = (x, column, z);
    }
    if !world.solid(pos) {
        return pos;
    }
    let mut column = pos.1 + 1;
    while column <= world.max_y() && world.solid((x, column, z)) {
        column += 1;
    }
    (x, column, z)
}

/// `GroundPathNavigation.moveTo(x, y, z, speed)`: a path from the walk
/// evaluator over the world. The mob repaths on the ground or in a liquid
/// (`canUpdatePath`); otherwise the current path is dropped.
#[allow(clippy::too_many_arguments)]
pub fn navigate_walk_to<W: World + ?Sized>(
    body: &Body,
    navigation: &mut GroundNavigation,
    world: &W,
    profile: &WalkProfile,
    fluid: FluidFrame,
    target: DVec3,
    speed: f64,
    reach_range: i32,
) -> Option<bool> {
    let planned = plan_walk_path(body, navigation, world, profile, fluid, target, reach_range);
    Some(navigation.move_to_in(world, planned, speed, body.position))
}

/// `PathNavigation.moveTo(entity, reachRange, speed)`: as
/// [`navigate_walk_to`], but when no path can be made the current one stays.
#[allow(clippy::too_many_arguments)]
pub fn navigate_walk_to_entity<W: World + ?Sized>(
    body: &Body,
    navigation: &mut GroundNavigation,
    world: &W,
    profile: &WalkProfile,
    fluid: FluidFrame,
    target: DVec3,
    speed: f64,
    reach_range: i32,
) -> bool {
    // `moveTo(entity, reachRange, speed)`, which `WallClimberNavigation`
    // leaves alone: a climber has only recorded the target's block.
    match plan_walk_path(body, navigation, world, profile, fluid, target, reach_range) {
        Some(planned) => navigation.move_to_in(world, Some(planned), speed, body.position),
        None => false,
    }
}

/// A path `PathNavigation.createPath` hands back: its nodes, the next node
/// (a path still being followed comes back as it stands) and whether it
/// reaches its target.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlannedPath {
    nodes: Vec<(i32, i32, i32)>,
    next: usize,
    reached: bool,
    /// `Path.getTarget`.
    target: Option<(i32, i32, i32)>,
}

impl PlannedPath {
    /// The navigation's own path as it stands.
    pub fn of(navigation: &GroundNavigation) -> Self {
        Self { nodes: navigation.nodes.clone(), next: navigation.next, reached: navigation.reached, target: navigation.path_target }
    }

    /// `Path.getTarget`.
    pub fn target(&self) -> Option<(i32, i32, i32)> {
        self.target
    }

    pub fn nodes(&self) -> &[(i32, i32, i32)] {
        &self.nodes
    }

    /// `getNextNodeIndex`.
    pub fn next(&self) -> usize {
        self.next
    }

    /// `Path.canReach`.
    pub fn reached(&self) -> bool {
        self.reached
    }
}

/// `GroundPathNavigation.createPath` towards a position: `None` when the
/// mob cannot path now (off the ground and out of liquids, or below the
/// world). A new path records its target in the navigation.
pub fn plan_walk_path<W: World + ?Sized>(
    body: &Body,
    navigation: &mut GroundNavigation,
    world: &W,
    profile: &WalkProfile,
    fluid: FluidFrame,
    target: DVec3,
    reach_range: i32,
) -> Option<PlannedPath> {
    if navigation.wall_climber {
        navigation.path_to_position = Some((target.x.floor() as i32, target.y.floor() as i32, target.z.floor() as i32));
    }
    if body.position.y < f64::from(world.min_y()) || !(body.on_ground || fluid.in_water() || fluid.in_lava()) {
        return None;
    }
    let target_pos = find_surface_position(world, (target.x.floor() as i32, target.y.floor() as i32, target.z.floor() as i32));
    if !navigation.is_done() && navigation.target_pos == Some(target_pos) {
        return Some(PlannedPath::of(navigation));
    }
    let walker = Walker { position: body.position, on_ground: body.on_ground, in_floatable_fluid: fluid.in_water() };
    let target = DVec3::new(f64::from(target_pos.0), f64::from(target_pos.1), f64::from(target_pos.2));
    let path = find_walk_path(world, profile, walker, target, reach_range);
    navigation.target_pos = Some(path.target);
    navigation.reach_range = reach_range;
    navigation.reset_stuck_timeout();
    Some(PlannedPath { nodes: path.nodes, next: 0, reached: path.reached, target: Some(path.target) })
}

/// `PathNavigation.createPath(Set<BlockPos>, reachRange)` (as
/// `AcquirePoi.findPathToPois` asks it), the targets in their hash set's
/// order: a path to the best of them, not followed; the navigation takes
/// the chosen target and restarts its stuck timeout. While following a
/// path to one of them, that path comes back.
pub fn plan_walk_path_to_any<W: World + ?Sized>(
    body: &Body,
    navigation: &mut GroundNavigation,
    world: &W,
    profile: &WalkProfile,
    fluid: FluidFrame,
    targets: &[(i32, i32, i32)],
    reach_range: i32,
) -> Option<PlannedPath> {
    if targets.is_empty() || body.position.y < f64::from(world.min_y()) || !(body.on_ground || fluid.in_water() || fluid.in_lava()) {
        return None;
    }
    if !navigation.is_done() && navigation.target_pos.is_some_and(|t| targets.contains(&t)) {
        return Some(PlannedPath::of(navigation));
    }
    let walker = Walker { position: body.position, on_ground: body.on_ground, in_floatable_fluid: fluid.in_water() };
    let path = find_walk_path_to_any(world, profile, walker, targets, reach_range)?;
    navigation.target_pos = Some(path.target);
    navigation.reach_range = reach_range;
    navigation.reset_stuck_timeout();
    Some(PlannedPath { nodes: path.nodes, next: 0, reached: path.reached, target: Some(path.target) })
}

/// Request a ground path for an animal with the pinned common navigation rules.
pub fn navigate_body_to(
    body: &Body,
    navigation: &mut GroundNavigation,
    terrain: &impl PathTerrain,
    target: DVec3,
    speed: f64,
) -> Option<bool> {
    navigate_body_to_with_accuracy(body, navigation, terrain, target, speed, 1)
}

pub fn navigate_body_to_with_accuracy(
    body: &Body,
    navigation: &mut GroundNavigation,
    terrain: &impl PathTerrain,
    target: DVec3,
    speed: f64,
    accuracy: i32,
) -> Option<bool> {
    navigate_body_to_mode(
        body,
        navigation,
        terrain,
        target,
        speed,
        accuracy,
        body.on_ground,
    )
}

/// AmphibiousPathNavigation.canUpdatePath is true in water as well as on land.
pub fn navigate_amphibious_to_with_accuracy(
    body: &Body,
    navigation: &mut GroundNavigation,
    terrain: &impl PathTerrain,
    target: DVec3,
    speed: f64,
    accuracy: i32,
) -> Option<bool> {
    navigate_body_to_mode(body, navigation, terrain, target, speed, accuracy, true)
}

fn navigate_body_to_mode(
    body: &Body,
    navigation: &mut GroundNavigation,
    terrain: &impl PathTerrain,
    target: DVec3,
    speed: f64,
    accuracy: i32,
    can_update: bool,
) -> Option<bool> {
    if !can_update {
        navigation.nodes.clear();
        navigation.next = 0;
        navigation.reached = false;
        navigation.target_pos = None;
        return Some(false);
    }
    let target_pos = (
        target.x.floor() as i32,
        target.y.floor() as i32,
        target.z.floor() as i32,
    );
    if !navigation.is_done() && navigation.target_pos == Some(target_pos) {
        return Some(navigation.resume_same_path(speed, body.position));
    }
    let path = find_path_with_accuracy(terrain, body.position, can_update, target, accuracy)?;
    let target_pos = path.target;
    if navigation.nodes == path.nodes {
        navigation.target_pos = Some(target_pos);
        return Some(navigation.resume_same_path(speed, body.position));
    }
    let accepted = navigation.follow(path.nodes, speed, body.position);
    navigation.target_pos = Some(target_pos);
    navigation.reset_stuck_timeout();
    navigation.reached = path.reached;
    Some(accepted)
}

#[derive(Clone, Debug, Default)]
pub struct GroundNavigation {
    /// `GroundPathNavigation.setAvoidSun` (`RestrictSunGoal`): out of the
    /// sun, a new path stops short of the first node under open sky.
    pub avoid_sun: bool,
    pub nodes: Vec<(i32, i32, i32)>,
    pub target_pos: Option<(i32, i32, i32)>,
    /// The followed path's own target (`path.getTarget()`), which a path
    /// made but not followed does not change.
    pub path_target: Option<(i32, i32, i32)>,
    /// `reachRange` of the last path made, `timeLastRecompute`, and
    /// `hasDelayedRecomputation`.
    pub reach_range: i32,
    time_last_recompute: i64,
    delayed_recompute: bool,
    pub next: usize,
    pub speed_modifier: f64,
    pub reached: bool,
    tick: i32,
    last_stuck_check: i32,
    last_stuck_position: DVec3,
    /// `WallClimberNavigation` (spiders): the block it was last sent to,
    /// which it keeps walking straight to once its path is done or none
    /// could be made; stopping keeps it.
    pub wall_climber: bool,
    pub path_to_position: Option<(i32, i32, i32)>,
    /// The water surface a floating mob measures its height at, for the
    /// tick in hand.
    surface: Option<i32>,
    /// `isStuck`: the last check found it had barely moved.
    stuck: bool,
    /// `timeoutCachedNode`, `timeoutTimer`, `timeoutLimit` and
    /// `lastTimeoutCheck` (by the navigation's own tick).
    timeout_node: (i32, i32, i32),
    timeout_timer: i64,
    timeout_limit: f64,
    last_timeout_check: i64,
}

impl GroundNavigation {
    /// `PathNavigation.shouldRecomputePath`: a block changing within the
    /// remaining node count of the midpoint between the mob and its path's
    /// end (not while a recomputation waits).
    pub fn should_recompute(&self, pos: (i32, i32, i32), mob: DVec3) -> bool {
        if self.delayed_recompute || self.is_done() {
            return false;
        }
        let end = *self.nodes.last().expect("a path under way");
        let middle = DVec3::new((f64::from(end.0) + mob.x) / 2.0, (f64::from(end.1) + mob.y) / 2.0, (f64::from(end.2) + mob.z) / 2.0);
        let center = DVec3::new(f64::from(pos.0) + 0.5, f64::from(pos.1) + 0.5, f64::from(pos.2) + 0.5);
        let reach = (self.nodes.len() - self.next) as f64;
        center.distance_squared(middle) < reach * reach
    }

    /// `PathNavigation.recomputePath`: within 20 ticks of the last one, or
    /// unable to path now, it waits; otherwise a new path to the same
    /// target replaces the old (no path at all when none is found).
    /// Returns whether a new path was made.
    pub fn recompute<W: World + ?Sized>(&mut self, world: &W, body: &Body, profile: &WalkProfile, fluid: FluidFrame, time: i64) -> bool {
        let can_update = body.on_ground || fluid.in_water() || fluid.in_lava();
        if time - self.time_last_recompute <= 20 || !can_update {
            self.delayed_recompute = true;
            return false;
        }
        let Some(target) = self.target_pos else { return false };
        self.nodes.clear();
        self.next = 0;
        let at = DVec3::new(f64::from(target.0), f64::from(target.1), f64::from(target.2));
        let reach = self.reach_range;
        match plan_walk_path(body, self, world, profile, fluid, at, reach) {
            Some(planned) => {
                self.nodes = planned.nodes;
                self.next = 0;
                self.reached = planned.reached;
                self.path_target = planned.target;
            }
            None => self.reached = false,
        }
        self.time_last_recompute = time;
        self.delayed_recompute = false;
        true
    }

    /// `PathNavigation.tick`'s first step: a waiting recomputation.
    pub fn delayed_recompute<W: World + ?Sized>(&mut self, world: &W, body: &Body, profile: &WalkProfile, fluid: FluidFrame, time: i64) -> bool {
        self.delayed_recompute && self.recompute(world, body, profile, fluid, time)
    }

    pub fn is_done(&self) -> bool {
        self.nodes.is_empty() || self.next >= self.nodes.len()
    }

    pub fn stop(&mut self) {
        self.nodes.clear();
        self.next = 0;
        self.reached = false;
    }

    /// `PathNavigation.isStuck`.
    pub fn is_stuck(&self) -> bool {
        self.stuck
    }

    /// `resetStuckTimeout`.
    pub fn reset_stuck_timeout(&mut self) {
        self.timeout_node = (0, 0, 0);
        self.timeout_timer = 0;
        self.timeout_limit = 0.0;
        self.stuck = false;
    }

    /// `doStuckDetection`, at the end of `followThePath`: every hundred
    /// ticks a mob that has not moved a quarter of its speed's hundred
    /// ticks is stuck and stops; and a node it takes three times longer to
    /// reach than its speed says drops the path.
    fn detect_stuck(&mut self, mob_position: DVec3, speed: f32) {
        if self.tick - self.last_stuck_check > 100 {
            let effective = if speed >= 1.0 { speed } else { speed * speed };
            let threshold = effective * 100.0 * 0.25;
            if mob_position.distance_squared(self.last_stuck_position) < f64::from(threshold * threshold) {
                self.stuck = true;
                self.stop();
            } else {
                self.stuck = false;
            }
            self.last_stuck_check = self.tick;
            self.last_stuck_position = mob_position;
        }
        if !self.is_done() {
            let node = self.nodes[self.next];
            let time = i64::from(self.tick);
            if node == self.timeout_node {
                self.timeout_timer += time - self.last_timeout_check;
            } else {
                self.timeout_node = node;
                let centre = DVec3::new(f64::from(node.0) + 0.5, f64::from(node.1), f64::from(node.2) + 0.5);
                let distance = mob_position.distance(centre);
                self.timeout_limit = if speed > 0.0 { distance / f64::from(speed) * 20.0 } else { 0.0 };
            }
            if self.timeout_limit > 0.0 && self.timeout_timer as f64 > self.timeout_limit * 3.0 {
                // `timeoutPath`.
                self.reset_stuck_timeout();
                self.stop();
            }
            self.last_timeout_check = time;
        }
    }

    pub fn observed_next(&self) -> i32 {
        if self.nodes.is_empty() {
            -1
        } else {
            self.next as i32
        }
    }

    pub fn follow(
        &mut self,
        nodes: Vec<(i32, i32, i32)>,
        speed_modifier: f64,
        position: DVec3,
    ) -> bool {
        self.target_pos = nodes.last().copied();
        self.path_target = self.target_pos;
        self.nodes = nodes;
        self.next = 0;
        self.reached = true;
        if self.is_done() {
            return false;
        }
        self.speed_modifier = speed_modifier;
        self.last_stuck_check = self.tick;
        self.last_stuck_position = DVec3::new(position.x, (position.y + 0.5).floor(), position.z);
        true
    }

    /// `PathNavigation.moveTo(path, speed)`: no path drops the current one;
    /// a path with the same nodes keeps the current one's progress.
    pub fn move_to(&mut self, planned: Option<PlannedPath>, speed_modifier: f64, position: DVec3) -> bool {
        let Some(planned) = planned else {
            self.stop();
            return false;
        };
        if self.nodes != planned.nodes {
            self.nodes = planned.nodes;
            self.next = planned.next;
            self.reached = planned.reached;
            self.path_target = planned.target;
        }
        self.resume_same_path(speed_modifier, position)
    }

    /// [`GroundNavigation::move_to`] with `GroundPathNavigation.trimPath`'s
    /// sun check: when avoiding the sun and standing out of it, the path is
    /// cut at its first node under open sky.
    pub fn move_to_in<W: World + ?Sized>(&mut self, world: &W, planned: Option<PlannedPath>, speed_modifier: f64, position: DVec3) -> bool {
        let Some(mut planned) = planned else {
            self.stop();
            return false;
        };
        let replacing = self.nodes != planned.nodes;
        if self.avoid_sun {
            let head = (position.x.floor() as i32, (position.y + 0.5).floor() as i32, position.z.floor() as i32);
            let nodes = if replacing { &mut planned.nodes } else { &mut self.nodes };
            if !world.can_see_sky(head) && self.next <= nodes.len() {
                if let Some(cut) = nodes.iter().position(|&node| world.can_see_sky(node)) {
                    nodes.truncate(cut);
                }
            }
        }
        if replacing {
            self.nodes = planned.nodes;
            self.next = planned.next;
            self.reached = planned.reached;
            self.path_target = planned.target;
        }
        self.resume_same_path(speed_modifier, position)
    }

    pub fn resume_same_path(&mut self, speed_modifier: f64, position: DVec3) -> bool {
        if self.is_done() {
            return false;
        }
        self.speed_modifier = speed_modifier;
        self.last_stuck_check = self.tick;
        self.last_stuck_position = DVec3::new(position.x, (position.y + 0.5).floor(), position.z);
        true
    }

    /// Returns the wanted position to pass to MoveControl, if the path continues.
    pub fn tick<W: World + ?Sized>(
        &mut self,
        world: &W,
        position: DVec3,
        on_ground: bool,
        width: f32,
        speed: f32,
    ) -> Option<(DVec3, f64)> {
        self.tick_in(world, position, on_ground, None, width, speed)
    }

    /// `PathNavigation.tick` for a walker: it follows its path while
    /// `can_update` (`canUpdatePath`: on the ground or in a liquid), and a
    /// mob afloat in water measures its height at the surface
    /// (`getSurfaceY`, see [`ground_view`]).
    pub fn tick_in<W: World + ?Sized>(&mut self, world: &W, position: DVec3, can_update: bool, surface: Option<i32>, width: f32, speed: f32) -> Option<(DVec3, f64)> {
        if self.wall_climber && self.is_done() {
            return self.tick_climber(position, width);
        }
        self.surface = surface;
        let result = self.tick_mode(position, can_update, width, 0.0, speed, false, |_| false);
        self.surface = None;
        // `setWantedPosition(target.x, getGroundY(target), target.z)`.
        result.map(|(target, speed)| (DVec3::new(target.x, ground_y(world, target), target.z), speed))
    }

    /// `WallClimberNavigation.tick` with no path: straight for the block's
    /// corner until the mob is within its width of the block's centre (or,
    /// above it, of the centre of the column at its own height).
    fn tick_climber(&mut self, position: DVec3, width: f32) -> Option<(DVec3, f64)> {
        let (x, y, z) = self.path_to_position?;
        let reach = f64::from(width);
        // `Vec3i.closerToCenterThan`.
        let near = |cy: f64| {
            let (dx, dy, dz) = (f64::from(x) + 0.5 - position.x, cy + 0.5 - position.y, f64::from(z) + 0.5 - position.z);
            dx * dx + dy * dy + dz * dz < reach * reach
        };
        let far = !near(f64::from(y)) && (!(position.y > f64::from(y)) || !near(position.y.floor()));
        if far {
            Some((DVec3::new(f64::from(x), f64::from(y), f64::from(z)), self.speed_modifier))
        } else {
            self.path_to_position = None;
            None
        }
    }

    /// PathNavigation with AmphibiousPathNavigation's always-active follow
    /// state and a liquid-aware direct-corner check.
    pub fn tick_amphibious(
        &mut self,
        world: &impl World,
        position: DVec3,
        width: f32,
        height: f32,
        speed: f32,
        in_water: bool,
    ) -> Option<(DVec3, f64)> {
        self.tick_mode(position, true, width, height, speed, true, |target| {
            in_water
                && clear_full_cubes(
                    world,
                    position + DVec3::Y * f64::from(height) * 0.5,
                    target + DVec3::Y * f64::from(height) * 0.5,
                )
        })
    }

    fn tick_mode(
        &mut self,
        position: DVec3,
        on_ground: bool,
        width: f32,
        height: f32,
        speed: f32,
        amphibious: bool,
        can_move_directly: impl Fn(DVec3) -> bool,
    ) -> Option<(DVec3, f64)> {
        self.tick += 1;
        if self.is_done() {
            return None;
        }
        let mob_position = if amphibious {
            position + DVec3::Y * f64::from(height) * 0.5
        } else {
            // `getTempMobPos`: the surface for a mob afloat.
            let y = self.surface.map_or_else(|| (position.y + 0.5).floor(), f64::from);
            DVec3::new(position.x, y, position.z)
        };
        if on_ground {
            self.follow_path(position, mob_position, width, &can_move_directly);
            self.detect_stuck(mob_position, speed);
        } else {
            let next = self.nodes[self.next];
            let target = self.entity_pos(next, width);
            if mob_position.y > target.y
                && position.x.floor() == target.x.floor()
                && position.z.floor() == target.z.floor()
            {
                self.next += 1;
            }
        }
        if self.is_done() {
            return None;
        }
        Some((
            self.entity_pos(self.nodes[self.next], width),
            self.speed_modifier,
        ))
    }

    fn follow_path(
        &mut self,
        position: DVec3,
        mob_position: DVec3,
        width: f32,
        can_move_directly: &impl Fn(DVec3) -> bool,
    ) {
        let node = self.nodes[self.next];
        let max_distance = if width > 0.75 {
            width / 2.0
        } else {
            0.75 - width / 2.0
        };
        let close = (position.x - (f64::from(node.0) + 0.5)).abs() < f64::from(max_distance)
            && (position.z - (f64::from(node.2) + 0.5)).abs() < f64::from(max_distance)
            && (position.y - f64::from(node.1)).abs() < 1.0;
        if close || self.should_skip_node(mob_position, width, can_move_directly) {
            self.next += 1;
        }
    }

    fn should_skip_node(
        &self,
        position: DVec3,
        width: f32,
        can_move_directly: &impl Fn(DVec3) -> bool,
    ) -> bool {
        if self.next + 1 >= self.nodes.len() {
            return false;
        }
        let current = center(self.nodes[self.next]);
        if position.distance_squared(current) >= 4.0 {
            return false;
        }
        if can_move_directly(self.entity_pos(self.nodes[self.next], width)) {
            return true;
        }
        let next = center(self.nodes[self.next + 1]);
        let to_current = current - position;
        let to_next = next - position;
        let current_sq = to_current.length_squared();
        let next_sq = to_next.length_squared();
        if next_sq >= current_sq && current_sq >= 0.5 {
            return false;
        }
        to_next.normalize().dot(to_current.normalize()) < 0.0
    }

    fn entity_pos(&self, node: (i32, i32, i32), width: f32) -> DVec3 {
        let offset = f64::from((width + 1.0) as i32) * 0.5;
        DVec3::new(
            f64::from(node.0) + offset,
            f64::from(node.1),
            f64::from(node.2) + offset,
        )
    }
}

/// `PathNavigation.getGroundY`: the floor under a waypoint
/// (`WalkNodeEvaluator.getFloorLevel` of its block), or the waypoint's own
/// height when the block below it is air.
pub fn ground_y<W: World + ?Sized>(world: &W, target: DVec3) -> f64 {
    let pos = (target.x.floor() as i32, target.y.floor() as i32, target.z.floor() as i32);
    let air = world.block((pos.0, pos.1 - 1, pos.2)).is_none_or(|b| matches!(b.id.as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"));
    if air {
        target.y
    } else {
        crate::walk_path::static_floor_level(world, pos)
    }
}

/// `GroundPathNavigation`'s view of a mob this tick: whether it may follow
/// its path (`canUpdatePath`: on the ground or in water or lava) and, for
/// a mob that floats (`canFloat`) in water, the first block above its feet
/// out of the water (`getSurfaceY`, giving up after 16).
pub fn ground_view<W: World + ?Sized>(world: &W, body: &Body, fluid: FluidFrame, can_float: bool) -> (bool, Option<i32>) {
    let can_update = body.on_ground || fluid.in_water() || fluid.in_lava();
    if !(can_float && fluid.in_water()) {
        return (can_update, None);
    }
    let p = body.position;
    let (x, z) = (p.x.floor() as i32, p.z.floor() as i32);
    let start = p.y.floor() as i32;
    let mut surface = start;
    let mut steps = 0;
    while world.floatable_fluid((x, surface, z)) {
        surface += 1;
        steps += 1;
        if steps > 16 {
            return (can_update, Some(start));
        }
    }
    (can_update, Some(surface))
}

fn clear_full_cubes(world: &impl World, from: DVec3, to: DVec3) -> bool {
    let delta = to - from;
    let steps = (delta.length() * 16.0).ceil() as usize;
    for step in 0..=steps.max(1) {
        let pos = from + delta * (step as f64 / steps.max(1) as f64);
        let block_pos = (
            pos.x.floor() as i32,
            pos.y.floor() as i32,
            pos.z.floor() as i32,
        );
        if world.block(block_pos).is_some_and(|block| {
            matches!(
                block.id.as_str(),
                "minecraft:stone"
                    | "minecraft:dirt"
                    | "minecraft:grass_block"
                    | "minecraft:cobblestone"
            )
        }) {
            return false;
        }
    }
    true
}

fn center(node: (i32, i32, i32)) -> DVec3 {
    DVec3::new(
        f64::from(node.0) + 0.5,
        f64::from(node.1),
        f64::from(node.2) + 0.5,
    )
}
