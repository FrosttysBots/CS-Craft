//! Entities on the server level: the shared `Entity` movement pipeline
//! (26.3 `Entity.tick`/`baseTick`, `Entity.move` with `collide`,
//! `setOnGroundWithMovement`/`checkSupportingBlock`, `checkFallDamage`,
//! restitution, block speed factors, and `applyEffectsFromBlocks` with
//! `BlockGetter.forEachBlockIntersectedBetween`), item entities
//! (`ItemEntity`) and the block callbacks entities trigger (pressure plates).
//!
//! Entities without their own simulation yet are not created. Fluid
//! interaction (swimming, currents) and moving out of blocks are recorded
//! in `Level::unsupported` when they would apply.

use super::container::Stack;
use super::physics::{collide_with_shapes, Aabb};
use super::{update, Level};
use minecraftoss_core::{BlockPos, BlockStateId};
use std::collections::HashSet;

/// `Entity.Movement`: one step of this tick's movement.
#[derive(Clone, Copy, Debug)]
struct Movement {
    from: [f64; 3],
    to: [f64; 3],
    /// The requested movement, when the step came from `move`.
    axis_dependent: Option<[f64; 3]>,
}

/// `ItemEntity` state.
#[derive(Clone, Debug)]
pub struct ItemData {
    pub stack: Stack,
    pub pickup_delay: i32,
    pub age: i32,
    /// `ItemEntity.health`.
    pub health: i32,
}

/// `FallingBlockEntity` state.
#[derive(Clone, Debug)]
pub struct FallingData {
    pub state: BlockStateId,
    pub time: i32,
    pub drop_item: bool,
}

/// `PrimedTnt` state.
#[derive(Clone, Debug)]
pub struct TntData {
    pub fuse: i32,
    pub power: f32,
}

/// `ExperienceOrb` state.
#[derive(Clone, Debug)]
pub struct OrbData {
    /// `DATA_VALUE`: the experience one pickup gives.
    pub value: i32,
    /// Orbs merged into this one: the pickups it has left.
    pub count: i32,
    pub age: i32,
    pub health: i32,
    /// The player it follows (`followingPlayer`), as an index into
    /// `Level::living_players`.
    pub following: Option<usize>,
}

/// `ExperienceOrb.getExperienceValue`: the largest orb size that fits.
pub fn orb_value(amount: i32) -> i32 {
    const SIZES: [i32; 10] = [2477, 1237, 617, 307, 149, 73, 37, 17, 7, 3];
    SIZES.into_iter().find(|&size| amount >= size).unwrap_or(1)
}

/// `ExperienceOrb.getIcon`: which of the texture's sprites an orb shows.
pub fn orb_icon(value: i32) -> u32 {
    const SIZES: [i32; 10] = [3, 7, 17, 37, 73, 149, 307, 617, 1237, 2477];
    SIZES.into_iter().filter(|&size| value >= size).count() as u32
}

/// `LightningBolt` state.
#[derive(Clone, Debug)]
pub struct LightningData {
    pub life: i32,
    pub flashes: i32,
    pub visual_only: bool,
}

#[derive(Clone, Debug)]
pub enum EntityKind {
    Item(ItemData),
    LightningBolt(LightningData),
    PrimedTnt(TntData),
    FallingBlock(FallingData),
    ExperienceOrb(OrbData),
}

/// A simulated entity.
#[derive(Clone, Debug)]
pub struct Entity {
    pub id: i32,
    pub kind: EntityKind,
    pub pos: [f64; 3],
    pub delta: [f64; 3],
    pub width: f32,
    pub height: f32,
    pub bb: Aabb,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub vertical_collision: bool,
    pub vertical_collision_below: bool,
    pub fall_distance: f64,
    pub tick_count: i32,
    pub removed: bool,
    pub first_tick: bool,
    pub no_physics: bool,
    pub no_gravity: bool,
    pub tags: Vec<String>,
    main_support: Option<BlockPos>,
    on_ground_no_blocks: bool,
    /// `xo`, `yo`, `zo` (`oldPosition`).
    old: [f64; 3],
    movement_this_tick: Vec<Movement>,
    final_movements: Vec<Movement>,
    /// `wasTouchingWater` (`isInWater`).
    pub was_touching_water: bool,
    /// `EntityFluidInteraction` heights of water and lava above the feet.
    pub water_height: f64,
    pub lava_height: f64,
    /// `isEyeInFluid(WATER)`: the eyes are under a water surface.
    pub eye_in_water: bool,
    /// `ItemEntity.target` / thrower are not tracked; merging compares none.
    _reserved: (),
}

/// `ItemEntity.pickupDelay` for `setNeverPickUp`.
pub const NEVER_PICK_UP: i32 = 32767;

impl Entity {
    fn new(id: i32, kind: EntityKind, pos: [f64; 3], width: f32, height: f32) -> Self {
        Self {
            id,
            kind,
            pos,
            delta: [0.0; 3],
            width,
            height,
            bb: Aabb::for_entity(pos[0], pos[1], pos[2], width, height),
            on_ground: false,
            horizontal_collision: false,
            vertical_collision: false,
            vertical_collision_below: false,
            fall_distance: 0.0,
            tick_count: 0,
            removed: false,
            first_tick: true,
            no_physics: false,
            no_gravity: false,
            tags: Vec::new(),
            main_support: None,
            on_ground_no_blocks: false,
            old: pos,
            movement_this_tick: Vec::new(),
            final_movements: Vec::new(),
            was_touching_water: false,
            water_height: 0.0,
            lava_height: 0.0,
            eye_in_water: false,
            _reserved: (),
        }
    }

    /// `ItemEntity(level, x, y, z, stack, dx, dy, dz)`.
    pub fn item(id: i32, pos: [f64; 3], stack: Stack, delta: [f64; 3]) -> Self {
        let mut entity = Self::new(id, EntityKind::Item(ItemData { stack, pickup_delay: 0, age: 0, health: 5 }), pos, 0.25, 0.25);
        entity.delta = delta;
        entity
    }

    /// `PrimedTnt` (0.98 wide and tall).
    pub fn primed_tnt(pos: [f64; 3], delta: [f64; 3], data: TntData) -> Self {
        let mut entity = Self::new(0, EntityKind::PrimedTnt(data), pos, 0.98, 0.98);
        entity.delta = delta;
        entity
    }

    /// `FallingBlockEntity(level, x, y, z, state)` (0.98 wide and tall).
    pub fn falling_block(pos: [f64; 3], data: FallingData) -> Self {
        Self::new(0, EntityKind::FallingBlock(data), pos, 0.98, 0.98)
    }

    /// A still `ExperienceOrb` (half a block wide and tall) worth `value`,
    /// as `summon` makes one.
    pub fn experience_orb(pos: [f64; 3], value: i32) -> Self {
        Self::new(0, EntityKind::ExperienceOrb(OrbData { value, count: 1, age: 0, health: 5, following: None }), pos, 0.5, 0.5)
    }

    pub fn orb_data(&self) -> Option<&OrbData> {
        match &self.kind {
            EntityKind::ExperienceOrb(data) => Some(data),
            _ => None,
        }
    }

    pub fn orb_data_mut(&mut self) -> Option<&mut OrbData> {
        match &mut self.kind {
            EntityKind::ExperienceOrb(data) => Some(data),
            _ => None,
        }
    }

    pub fn falling_data(&self) -> Option<&FallingData> {
        match &self.kind {
            EntityKind::FallingBlock(data) => Some(data),
            _ => None,
        }
    }

    /// The type's eye height (`EntityType.Builder.eyeHeight`).
    pub fn eye_height(&self) -> f32 {
        match self.kind {
            EntityKind::Item(_) => 0.2125,
            EntityKind::PrimedTnt(_) => 0.15,
            EntityKind::FallingBlock(_) => 0.98 * 0.85,
            EntityKind::LightningBolt(_) => 0.0,
            EntityKind::ExperienceOrb(_) => 0.5 * 0.85,
        }
    }

    pub fn tnt_data(&self) -> Option<&TntData> {
        match &self.kind {
            EntityKind::PrimedTnt(data) => Some(data),
            _ => None,
        }
    }

    pub fn item_data(&self) -> Option<&ItemData> {
        match &self.kind {
            EntityKind::Item(data) => Some(data),
            _ => None,
        }
    }

    /// The position at the start of the tick (`xo`, `yo`, `zo`).
    pub fn old_position(&self) -> [f64; 3] {
        self.old
    }

    fn set_pos(&mut self, pos: [f64; 3]) {
        self.pos = pos;
        self.bb = Aabb::for_entity(pos[0], pos[1], pos[2], self.width, self.height);
    }

    fn make_bounding_box(&self, pos: [f64; 3]) -> Aabb {
        Aabb::for_entity(pos[0], pos[1], pos[2], self.width, self.height)
    }

    /// `getDefaultGravity`.
    fn default_gravity(&self) -> f64 {
        match self.kind {
            EntityKind::Item(_) | EntityKind::PrimedTnt(_) | EntityKind::FallingBlock(_) => 0.04,
            EntityKind::ExperienceOrb(_) => 0.03,
            EntityKind::LightningBolt(_) => 0.0,
        }
    }

    pub(super) fn gravity(&self) -> f64 {
        if self.no_gravity { 0.0 } else { self.default_gravity() }
    }
}

fn floor(v: f64) -> i32 {
    v.floor() as i32
}

/// `Vec3.normalize`.
pub(super) fn normalize(v: [f64; 3]) -> [f64; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length < f64::from(1.0e-5f32) {
        [0.0; 3]
    } else {
        [v[0] / length, v[1] / length, v[2] / length]
    }
}

/// `EntityFluidInteraction.CurrentAccumulator.applyTo` for non-players.
fn apply_current(e: &mut Entity, current: &(f64, [f64; 3], u32), scale: f64) {
    let (_, sum, count) = *current;
    if count == 0 || (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]) < f64::from(1.0e-5f32) {
        return;
    }
    let mut impulse = normalize(sum);
    impulse = [impulse[0] * scale, impulse[1] * scale, impulse[2] * scale];
    let length = (impulse[0] * impulse[0] + impulse[1] * impulse[1] + impulse[2] * impulse[2]).sqrt();
    if e.delta[0].abs() < 0.003 && e.delta[2].abs() < 0.003 && length < 0.004_500_000_000_000_000_5 {
        let n = normalize(impulse);
        impulse = [n[0] * 0.0045, n[1] * 0.0045, n[2] * 0.0045];
    }
    e.delta = [e.delta[0] + impulse[0], e.delta[1] + impulse[1], e.delta[2] + impulse[2]];
}

/// `Mth.equal` for doubles: within `1.0E-5F`.
fn mth_equal(a: f64, b: f64) -> bool {
    (b - a).abs() < f64::from(1.0e-5f32)
}

/// Block friction (`BlockBehaviour.Properties.friction`).
fn friction(name: &str) -> f32 {
    match name {
        "minecraft:ice" | "minecraft:packed_ice" | "minecraft:frosted_ice" => 0.98,
        "minecraft:blue_ice" => 0.989,
        "minecraft:slime_block" => 0.8,
        _ => 0.6,
    }
}

/// Block speed factor (`BlockBehaviour.Properties.speedFactor`).
fn speed_factor(name: &str) -> f32 {
    match name {
        "minecraft:soul_sand" | "minecraft:honey_block" => 0.4,
        _ => 1.0,
    }
}

/// `BlockPos.betweenCornersInDirection`.
fn corners_in_direction(a: [i32; 3], b: [i32; 3], direction: [f64; 3]) -> Vec<BlockPos> {
    let min = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
    let max = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
    let diff = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    let start: [i32; 3] = std::array::from_fn(|i| if direction[i] >= 0.0 { min[i] } else { max[i] });
    let order = if direction[0].abs() < direction[2].abs() { [1, 2, 0] } else { [1, 0, 2] };
    let step: [i32; 3] = std::array::from_fn(|i| if direction[i] >= 0.0 { 1 } else { -1 });
    let mut out = Vec::new();
    for first in 0..=diff[order[0]] {
        for second in 0..=diff[order[1]] {
            for third in 0..=diff[order[2]] {
                let mut p = start;
                p[order[0]] += step[order[0]] * first;
                p[order[1]] += step[order[1]] * second;
                p[order[2]] += step[order[2]] * third;
                out.push(BlockPos::new(p[0], p[1], p[2]));
            }
        }
    }
    out
}

/// `AABB.clip(min, max, from, to)`: the entry point, if any.
fn aabb_clip(min: [f64; 3], max: [f64; 3], from: [f64; 3], to: [f64; 3]) -> Option<[f64; 3]> {
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let mut scale = 1.0;
    let mut hit = false;
    // Axis order X, Y, Z; each clips against the entry face.
    let axes = [(0usize, 1usize, 2usize), (1, 2, 0), (2, 0, 1)];
    for (a, b, c) in axes {
        let face = if d[a] > 1.0e-7 {
            min[a]
        } else if d[a] < -1.0e-7 {
            max[a]
        } else {
            continue;
        };
        let s = (face - from[a]) / d[a];
        let pb = from[b] + s * d[b];
        let pc = from[c] + s * d[c];
        if 0.0 < s && s < scale && min[b] - 1.0e-7 < pb && pb < max[b] + 1.0e-7 && min[c] - 1.0e-7 < pc && pc < max[c] + 1.0e-7 {
            scale = s;
            hit = true;
        }
    }
    hit.then(|| [from[0] + scale * d[0], from[1] + scale * d[1], from[2] + scale * d[2]])
}

/// `BlockGetter.getFurthestCorner`.
fn furthest_corner(d: [f64; 3]) -> [i32; 3] {
    let (xd, yd, zd) = (d[0].abs(), d[1].abs(), d[2].abs());
    let sign = |v: f64| if v >= 0.0 { 1 } else { -1 };
    let (xs, ys, zs) = (sign(d[0]), sign(d[1]), sign(d[2]));
    if xd <= yd && xd <= zd {
        [-xs, -zs, ys]
    } else if yd <= zd {
        [zs, -ys, -xs]
    } else {
        [-ys, xs, -zs]
    }
}

/// `BlockGetter.forEachBlockIntersectedBetween`: blocks with the step
/// iteration they were reached in.
fn blocks_intersected_between(from: [f64; 3], to: [f64; 3], at_target: &Aabb) -> Vec<(BlockPos, i32)> {
    let travel = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len_sqr = travel[0] * travel[0] + travel[1] * travel[1] + travel[2] * travel[2];
    let corners = |b: &Aabb| ([floor(b.min[0]), floor(b.min[1]), floor(b.min[2])], [floor(b.max[0]), floor(b.max[1]), floor(b.max[2])]);
    let mut out = Vec::new();
    if len_sqr < f64::from(1.0e-5f32) * f64::from(1.0e-5f32) {
        // `BlockPos.betweenClosed(aabb)`: x fastest, then y, then z.
        let (lo, hi) = corners(at_target);
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    out.push((BlockPos::new(x, y, z), 0));
                }
            }
        }
        return out;
    }
    let mut visited: HashSet<BlockPos> = HashSet::new();
    let back = at_target.moved([-travel[0], -travel[1], -travel[2]]);
    let (lo, hi) = corners(&back);
    for pos in corners_in_direction(lo, hi, travel) {
        out.push((pos, 0));
        visited.insert(pos);
    }
    // `addCollisionsAlongTravel`.
    let size = [at_target.max[0] - at_target.min[0], at_target.max[1] - at_target.min[1], at_target.max[2] - at_target.min[2]];
    let corner = furthest_corner(travel);
    let center = [(at_target.min[0] + at_target.max[0]) / 2.0, (at_target.min[1] + at_target.max[1]) / 2.0, (at_target.min[2] + at_target.max[2]) / 2.0];
    let to_corner: [f64; 3] = std::array::from_fn(|i| center[i] + size[i] * 0.5 * f64::from(corner[i]));
    let from_corner: [f64; 3] = std::array::from_fn(|i| to_corner[i] - travel[i]);
    let mut block = [floor(from_corner[0]), floor(from_corner[1]), floor(from_corner[2])];
    let sign: [i32; 3] = std::array::from_fn(|i| if travel[i] > 0.0 { 1 } else if travel[i] < 0.0 { -1 } else { 0 });
    let t_delta: [f64; 3] = std::array::from_fn(|i| if sign[i] == 0 { f64::MAX } else { f64::from(sign[i]) / travel[i] });
    let frac = |v: f64| v - v.floor();
    let mut t: [f64; 3] = std::array::from_fn(|i| t_delta[i] * if sign[i] > 0 { 1.0 - frac(from_corner[i]) } else { frac(from_corner[i]) });
    let mut iterations = 0;
    while t[0] <= 1.0 || t[1] <= 1.0 || t[2] <= 1.0 {
        if t[0] < t[1] {
            if t[0] < t[2] {
                block[0] += sign[0];
                t[0] += t_delta[0];
            } else {
                block[2] += sign[2];
                t[2] += t_delta[2];
            }
        } else if t[1] < t[2] {
            block[1] += sign[1];
            t[1] += t_delta[1];
        } else {
            block[2] += sign[2];
            t[2] += t_delta[2];
        }
        let min = [f64::from(block[0]), f64::from(block[1]), f64::from(block[2])];
        let max = [min[0] + 1.0, min[1] + 1.0, min[2] + 1.0];
        if let Some(hit) = aabb_clip(min, max, from_corner, to_corner) {
            iterations += 1;
            let eps = f64::from(1.0e-5f32);
            let clamped: [f64; 3] = std::array::from_fn(|i| hit[i].clamp(min[i] + eps, min[i] + 1.0 - eps));
            let opposite: [i32; 3] = std::array::from_fn(|i| floor(clamped[i] - size[i] * f64::from(corner[i])));
            for pos in corners_in_direction(block, opposite, travel) {
                if visited.insert(pos) {
                    out.push((pos, iterations));
                }
            }
        }
    }
    let (lo, hi) = corners(at_target);
    for pos in corners_in_direction(lo, hi, travel) {
        if visited.insert(pos) {
            out.push((pos, iterations + 1));
        }
    }
    out
}

impl Level<'_> {
    /// Adds an entity (`Level.addFreshEntity`), ticking from this tick on.
    pub fn add_entity(&mut self, mut entity: Entity) -> i32 {
        if entity.id == 0 {
            entity.id = self.next_entity_id;
            self.next_entity_id += 1;
        }
        let id = entity.id;
        self.entities.push(entity);
        id
    }

    /// `ItemEntity(level, x, y, z, stack)` with a given launch.
    pub fn spawn_item_entity(&mut self, pos: [f64; 3], stack: Stack, delta: [f64; 3]) -> i32 {
        let entity = Entity::item(0, pos, stack, delta);
        self.add_entity(entity)
    }

    /// `Entity.spawnAtLocation`: `ItemEntity(level, x, y, z, stack)` with its
    /// random launch (from the entity random, as `pop_resource` draws it)
    /// and the default pickup delay.
    pub fn spawn_at_location(&mut self, pos: [f64; 3], stack: Stack) -> i32 {
        use minecraftoss_core::random::RandomSource;
        let dx = self.entity_random.next_f64() * 0.2 - 0.1;
        let dz = self.entity_random.next_f64() * 0.2 - 0.1;
        self.spawn_item_with(pos, stack, [dx, 0.2, dz], 10, 0)
    }

    /// `ItemEntity(level, x, y, z, stack, dx, dy, dz)` with a pickup delay
    /// and age, for items a client hands to the server.
    pub fn spawn_item_with(&mut self, pos: [f64; 3], stack: Stack, delta: [f64; 3], pickup_delay: i32, age: i32) -> i32 {
        let mut entity = Entity::item(0, pos, stack, delta);
        if let EntityKind::Item(data) = &mut entity.kind {
            data.pickup_delay = pickup_delay;
            data.age = age;
        }
        self.add_entity(entity)
    }

    /// `ItemEntity.playerTouch` for items touching a player standing at
    /// `feet` (`Player.aiStep`: the player's box inflated by 1, 0.5, 1).
    /// `take` returns how many of the offered stack the inventory accepted;
    /// the result lists each pickup's entity ID, position and taken stack.
    pub fn player_touch_items(&mut self, feet: [f64; 3], mut take: impl FnMut(&Stack) -> i32) -> Vec<(i32, [f64; 3], Stack)> {
        let player = Aabb::for_entity(feet[0], feet[1], feet[2], 0.6, 1.8);
        let touch = Aabb::new(
            player.min[0] - 1.0,
            player.min[1] - 0.5,
            player.min[2] - 1.0,
            player.max[0] + 1.0,
            player.max[1] + 0.5,
            player.max[2] + 1.0,
        );
        let mut picked = Vec::new();
        for e in &mut self.entities {
            if e.removed || !e.bb.intersects(&touch) {
                continue;
            }
            let EntityKind::Item(data) = &mut e.kind else { continue };
            if data.pickup_delay != 0 || data.stack.is_empty() {
                continue;
            }
            let taken = take(&data.stack).clamp(0, data.stack.count);
            if taken == 0 {
                continue;
            }
            let mut stack = data.stack.clone();
            stack.count = taken;
            data.stack.count -= taken;
            if data.stack.is_empty() {
                e.removed = true;
            }
            picked.push((e.id, e.pos, stack));
        }
        self.entities.retain(|e| !e.removed);
        picked
    }

    /// The item entities whose boxes meet the box from `min` to `max`
    /// (`getEntitiesOfClass(ItemEntity.class, box)`): ID, position, stack
    /// and pickup delay.
    pub fn items_touching(&self, min: [f64; 3], max: [f64; 3]) -> Vec<(i32, [f64; 3], Stack, i32)> {
        let bb = Aabb::new(min[0], min[1], min[2], max[0], max[1], max[2]);
        self.entities_in(&bb).filter_map(|e| e.item_data().map(|d| (e.id, e.pos, d.stack.clone(), d.pickup_delay))).collect()
    }

    /// An item entity by ID: position, stack and pickup delay.
    pub fn item_entity(&self, id: i32) -> Option<([f64; 3], Stack, i32)> {
        self.entities.iter().find(|e| e.id == id && !e.removed).and_then(|e| e.item_data().map(|d| (e.pos, d.stack.clone(), d.pickup_delay)))
    }

    /// A mob takes `count` from an item entity (`ItemEntity.getItem().shrink`,
    /// or `discard` when none are left).
    pub fn take_item(&mut self, id: i32, count: i32) {
        if let Some(e) = self.entities.iter_mut().find(|e| e.id == id && !e.removed) {
            if let EntityKind::Item(data) = &mut e.kind {
                data.stack.count -= count;
                if data.stack.count <= 0 {
                    e.removed = true;
                }
            }
        }
    }

    /// Entities whose bounding boxes overlap a box (`getEntitiesOfClass`).
    pub fn entities_in(&self, bb: &Aabb) -> impl Iterator<Item = &Entity> {
        let bb = *bb;
        self.entities.iter().filter(move |e| !e.removed && e.bb.intersects(&bb))
    }

    /// Ticks every entity in insertion order (`entityTickList.forEach`).
    pub(super) fn tick_entities(&mut self) {
        // Entities added during the pass wait for the next tick
        // (`EntityTickList` iterates a snapshot).
        let count = self.entities.len();
        let mut i = 0;
        while i < count {
            if !self.entities[i].removed {
                let mut entity = self.entities[i].clone();
                // `Entity.commonTick`: the old position, then the tick count.
                entity.old = entity.pos;
                entity.tick_count += 1;
                self.tick_entity(&mut entity);
                self.entities[i] = entity;
            }
            i += 1;
        }
        self.entities.retain(|e| !e.removed);
    }

    fn tick_entity(&mut self, e: &mut Entity) {
        match &e.kind {
            EntityKind::Item(_) => self.tick_item(e),
            EntityKind::LightningBolt(_) => self.tick_lightning(e),
            EntityKind::PrimedTnt(_) => self.tick_tnt(e),
            EntityKind::FallingBlock(_) => self.tick_falling_block(e),
            EntityKind::ExperienceOrb(_) => self.tick_orb(e),
        }
    }

    /// `summon minecraft:lightning_bolt`: `LightningBolt(type, level)`
    /// draws its seed and flash count from the entity's own random
    /// (unseeded in vanilla).
    pub fn summon_lightning(&mut self, pos: [f64; 3]) -> i32 {
        use minecraftoss_core::random::RandomSource;
        self.entity_random.next_i64();
        let flashes = self.entity_random.next_i32_bound(3) + 1;
        let data = LightningData { life: 2, flashes, visual_only: false };
        let entity = Entity::new(0, EntityKind::LightningBolt(data), pos, 0.0, 0.0);
        self.add_entity(entity)
    }

    /// `LightningBolt.tick` on a peaceful server without players: no fire.
    fn tick_lightning(&mut self, e: &mut Entity) {
        use minecraftoss_core::random::RandomSource;
        self.base_tick(e);
        let EntityKind::LightningBolt(data) = &e.kind else { return };
        if data.life == 2 {
            let strike = BlockPos::new(floor(e.pos[0]), floor(e.pos[1] - 1.0e-6), floor(e.pos[2]));
            let state = self.block(strike);
            if self.redstone_kind(state) == Some(super::redstone::Kind::LightningRod) {
                self.lightning_rod_strike(state, strike);
            }
            self.clear_copper_on_strike(strike);
        }
        let EntityKind::LightningBolt(data) = &mut e.kind else { return };
        data.life -= 1;
        if data.life < 0 {
            if data.flashes == 0 {
                e.removed = true;
            } else if data.life < -self.entity_random.next_i32_bound(10) {
                data.flashes -= 1;
                data.life = 1;
                self.entity_random.next_i64();
            }
        }
        let EntityKind::LightningBolt(data) = &e.kind else { return };
        if data.life >= 0 && !data.visual_only {
            let (x, y, z) = (e.pos[0], e.pos[1], e.pos[2]);
            let area = Aabb::new(x - 3.0, y - 3.0, z - 3.0, x + 3.0, y + 6.0 + 3.0, z + 3.0);
            if self.entities_in(&area).any(|o| o.id != e.id) {
                self.unsupported.push("lightning hitting entities".to_owned());
            }
        }
    }

    /// `LightningBolt.clearCopperOnLightningStrike`: the struck copper is
    /// cleaned, then random walks from the level random scrape more.
    fn clear_copper_on_strike(&mut self, struck: BlockPos) {
        use minecraftoss_core::random::RandomSource;
        let state = self.block(struck);
        let weathering = self.is_weathering_copper(state);
        let name = self.name(state);
        let waxed = name.starts_with("minecraft:waxed_") && (name.contains("copper") || name.contains("lightning_rod"));
        if !weathering && !waxed {
            return;
        }
        if weathering {
            let first = self.copper_first(self.block(struck));
            self.set_block_and_update(struck, first);
        }
        let strikes = self.random.next_i32_bound(3) + 3;
        for _ in 0..strikes {
            let steps = self.random.next_i32_bound(8) + 1;
            let mut work = struck;
            for _ in 0..steps {
                let mut found = None;
                for _ in 0..10 {
                    let x = work.x - 1 + self.random.next_i32_bound(3);
                    let y = work.y - 1 + self.random.next_i32_bound(3);
                    let z = work.z - 1 + self.random.next_i32_bound(3);
                    let candidate = BlockPos::new(x, y, z);
                    let s = self.block(candidate);
                    if self.is_weathering_copper(s) {
                        if let Some(previous) = self.copper_previous(s) {
                            self.set_block_and_update(candidate, previous);
                        }
                        found = Some(candidate);
                        break;
                    }
                }
                match found {
                    Some(p) => work = p,
                    None => break,
                }
            }
        }
    }

    /// `WeatheringCopper`: the block's class chain has a `Weathering...` class.
    fn is_weathering_copper(&self, state: BlockStateId) -> bool {
        let blocks = &self.registries().blocks;
        blocks.block(blocks.block_of(state)).classes().iter().any(|c| c.starts_with("Weathering"))
    }

    /// A copper block at another weathering stage, with the same properties.
    fn copper_stage(&self, state: BlockStateId, name: &str) -> Option<BlockStateId> {
        let blocks = &self.registries().blocks;
        let block = blocks.block_by_name(name)?;
        let mut next = blocks.block(block).default_state();
        for property in blocks.block(blocks.block_of(state)).properties() {
            if let Some(value) = blocks.property(state, &property.name) {
                if let Some(s) = blocks.with_property(next, &property.name, value) {
                    next = s;
                }
            }
        }
        Some(next)
    }

    /// Weathering stage (0 unaffected .. 3 oxidized) and the unaffected name.
    fn copper_base_name(name: &str) -> (usize, String) {
        let bare = name.trim_start_matches("minecraft:");
        for (stage, prefix) in [(1, "exposed_"), (2, "weathered_"), (3, "oxidized_")] {
            if let Some(rest) = bare.strip_prefix(prefix) {
                let rest = if rest == "copper" { "copper_block".to_owned() } else { rest.to_owned() };
                return (stage, rest);
            }
        }
        (0, bare.to_owned())
    }

    fn copper_stage_name(stage: usize, base: &str) -> String {
        let prefix = ["", "exposed_", "weathered_", "oxidized_"][stage];
        let base = if stage > 0 && base == "copper_block" { "copper" } else { base };
        format!("minecraft:{prefix}{base}")
    }

    /// `WeatheringCopper.getFirst`.
    fn copper_first(&self, state: BlockStateId) -> BlockStateId {
        let (_, base) = Self::copper_base_name(self.name(state));
        self.copper_stage(state, &Self::copper_stage_name(0, &base)).unwrap_or(state)
    }

    /// `WeatheringCopper.getPrevious`.
    fn copper_previous(&self, state: BlockStateId) -> Option<BlockStateId> {
        let (stage, base) = Self::copper_base_name(self.name(state));
        if stage == 0 {
            return None;
        }
        self.copper_stage(state, &Self::copper_stage_name(stage - 1, &base))
    }

    // ---- Entity ---------------------------------------------------------------------

    /// `Entity.baseTick` for entities that neither burn nor ride.
    fn base_tick(&mut self, e: &mut Entity) {
        self.update_fluid_interaction(e);
        if e.lava_height > 0.0 && !e.first_tick {
            e.fall_distance *= 0.5;
            self.unsupported.push("entity in lava (fire)".to_owned());
        }
        if e.pos[1] < f64::from(self.min_y - 64) {
            e.removed = true;
        }
        e.first_tick = false;
    }

    /// `Entity.updateFluidInteraction` with `EntityFluidInteraction.update`:
    /// fluid heights over the entity's box and the currents that push it.
    pub(super) fn entity_update_fluid_interaction(&mut self, e: &mut Entity) -> bool {
        self.update_fluid_interaction(e)
    }

    fn update_fluid_interaction(&mut self, e: &mut Entity) -> bool {
        use minecraftoss_core::block::FluidKind;
        e.water_height = 0.0;
        e.lava_height = 0.0;
        e.eye_in_water = false;
        let bb = e.bb.inflate(-0.001);
        let (x0, y0, z0) = (floor(bb.min[0]), floor(bb.min[1]), floor(bb.min[2]));
        let (x1, y1, z1) = (bb.max[0].ceil() as i32 - 1, bb.max[1].ceil() as i32 - 1, bb.max[2].ceil() as i32 - 1);
        if !self.has_fluid_sections(x0 - 1, y0, z0 - 1, x1 + 1, y1, z1 + 1) {
            e.was_touching_water = false;
            return false;
        }
        let entity_y = e.bb.min[1];
        // The eyes: the column the feet are in, at the eye height.
        let (eye_x, eye_z) = (floor(e.pos[0]), floor(e.pos[2]));
        let eye_y = e.pos[1] + f64::from(e.eye_height());
        let mut any = false;
        // Current accumulators per fluid: height, sum of flows, count.
        let mut water_current = (0.0f64, [0.0f64; 3], 0u32);
        let mut lava_current = (0.0f64, [0.0f64; 3], 0u32);
        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    let pos = BlockPos::new(x, y, z);
                    let Some(fluid) = self.fluid_state(self.block(pos)) else { continue };
                    let bottom = f64::from(y);
                    let top = bottom + f64::from(self.fluid_height_at(fluid, pos));
                    if top < bb.min[1] {
                        continue;
                    }
                    any = true;
                    if fluid.kind == FluidKind::Water && x == eye_x && z == eye_z && eye_y >= bottom {
                        // `FluidState.getHeightForCamera`: a source under a
                        // sturdy face fills its block.
                        let sturdy_above = fluid.source
                            && self.registries().blocks.is_face_sturdy(self.block(pos.above()), minecraftoss_core::pos::Direction::Down, minecraftoss_core::SupportType::Full);
                        let camera_top = bottom + if sturdy_above { 1.0 } else { f64::from(self.fluid_height_at(fluid, pos)) };
                        if eye_y <= camera_top {
                            e.eye_in_water = true;
                        }
                    }
                    let (height, current) = match fluid.kind {
                        FluidKind::Water => (&mut e.water_height, &mut water_current),
                        FluidKind::Lava => (&mut e.lava_height, &mut lava_current),
                    };
                    *height = height.max(top - entity_y);
                    let mut flow = self.fluid_flow(fluid, pos);
                    current.0 = current.0.max(*height);
                    if current.0 < 0.4 {
                        flow = [flow[0] * current.0, flow[1] * current.0, flow[2] * current.0];
                    }
                    current.1 = [current.1[0] + flow[0], current.1[1] + flow[1], current.1[2] + flow[2]];
                    current.2 += 1;
                }
            }
        }
        let in_water = e.water_height > 0.0;
        if in_water {
            e.fall_distance = 0.0;
        }
        e.was_touching_water = in_water;
        if in_water {
            apply_current(e, &water_current, 0.014);
        }
        if e.lava_height > 0.0 {
            let scale = if self.fast_lava { 0.007 } else { 0.002_333_333_333_333_333_5 };
            apply_current(e, &lava_current, scale);
        }
        any
    }

    /// `EntityFluidInteraction.hasFluidAndLoaded`.
    fn has_fluid_sections(&self, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) -> bool {
        let mut has = false;
        for cz in (z0 >> 4)..=(z1 >> 4) {
            for cx in (x0 >> 4)..=(x1 >> 4) {
                let Some(chunk) = self.chunk(minecraftoss_core::ChunkPos::new(cx, cz)) else { return false };
                for sy in (y0 >> 4)..=(y1 >> 4) {
                    let index = sy - chunk.min_section_y();
                    if index < 0 || index as usize >= chunk.sections().len() {
                        continue;
                    }
                    let base = sy * 16;
                    // `LevelChunkSection.hasFluid`: any fluid state in the section.
                    'section: for ly in 0..16 {
                        for lz in 0..16 {
                            for lx in 0..16 {
                                if self.registries().blocks.state(chunk.block(lx, base + ly, lz)).fluid.is_some() {
                                    has = true;
                                    break 'section;
                                }
                            }
                        }
                    }
                }
            }
        }
        has
    }

    /// `FluidState.getHeight`: full under the same fluid, else its own height.
    fn fluid_height_at(&self, fluid: super::fluid::Fluid, pos: BlockPos) -> f32 {
        if self.fluid_state(self.block(pos.above())).is_some_and(|f| f.kind == fluid.kind) {
            1.0
        } else {
            f32::from(fluid.amount) / 9.0
        }
    }

    /// `FlowingFluid.getFlow`.
    fn fluid_flow(&self, fluid: super::fluid::Fluid, pos: BlockPos) -> [f64; 3] {
        let own = f32::from(fluid.amount) / 9.0;
        let affects = |f: Option<super::fluid::Fluid>| f.is_none_or(|f| f.kind == fluid.kind);
        let own_height = |f: Option<super::fluid::Fluid>| f.map_or(0.0, |f| f32::from(f.amount) / 9.0);
        let (mut fx, mut fz) = (0.0f64, 0.0f64);
        for direction in minecraftoss_core::pos::Direction::HORIZONTAL {
            let side = pos.relative(direction, 1);
            let neighbour = self.fluid_state(self.block(side));
            if !affects(neighbour) {
                continue;
            }
            let mut height = own_height(neighbour);
            let mut distance = 0.0f32;
            if height == 0.0 {
                if !self.lib.registries.block_in_tag(self.block(side), self.blocks_fluid_flow_tag) {
                    let below = self.fluid_state(self.block(side.below()));
                    if affects(below) {
                        height = own_height(below);
                        if height > 0.0 {
                            distance = own - (height - 0.888_888_9);
                        }
                    }
                }
            } else if height > 0.0 {
                distance = own - height;
            }
            if distance != 0.0 {
                let (dx, _, dz) = direction.offset();
                fx += f64::from(dx) * f64::from(distance);
                fz += f64::from(dz) * f64::from(distance);
            }
        }
        let mut flow = [fx, 0.0, fz];
        if fluid.falling {
            for direction in minecraftoss_core::pos::Direction::HORIZONTAL {
                let side = pos.relative(direction, 1);
                if self.solid_face_for_flow(fluid, side, direction) || self.solid_face_for_flow(fluid, side.above(), direction) {
                    let n = normalize(flow);
                    flow = [n[0], n[1] - 6.0, n[2]];
                    break;
                }
            }
        }
        normalize(flow)
    }

    /// `FlowingFluid.isSolidFace`.
    fn solid_face_for_flow(&self, fluid: super::fluid::Fluid, pos: BlockPos, direction: minecraftoss_core::pos::Direction) -> bool {
        let state = self.block(pos);
        if self.fluid_state(state).is_some_and(|f| f.kind == fluid.kind) {
            return false;
        }
        if direction == minecraftoss_core::pos::Direction::Up {
            return true;
        }
        if self.is_a(state, "IceBlock") {
            return false;
        }
        self.registries().blocks.is_face_sturdy(state, direction, minecraftoss_core::SupportType::Full)
    }

    /// `Entity.collide` for entities without step height (non-living).
    fn entity_collide(&self, e: &Entity, movement: [f64; 3]) -> [f64; 3] {
        let len_sqr = movement[0] * movement[0] + movement[1] * movement[1] + movement[2] * movement[2];
        if len_sqr == 0.0 {
            return movement;
        }
        let shapes = self.block_collisions(&e.bb.expand_towards(movement));
        collide_with_shapes(movement, &e.bb, &shapes)
    }

    /// `Entity.move(MoverType.SELF, delta)`.
    pub(super) fn entity_move_self(&mut self, e: &mut Entity, delta: [f64; 3]) {
        self.entity_move(e, delta);
    }

    fn entity_move(&mut self, e: &mut Entity, delta: [f64; 3]) {
        if e.no_physics {
            e.set_pos([e.pos[0] + delta[0], e.pos[1] + delta[1], e.pos[2] + delta[2]]);
            e.horizontal_collision = false;
            e.vertical_collision = false;
            e.vertical_collision_below = false;
            return;
        }
        let movement = self.entity_collide(e, delta);
        let length = movement[0] * movement[0] + movement[1] * movement[1] + movement[2] * movement[2];
        let delta_length = delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2];
        if length > 1.0e-7 || delta_length - length < 1.0e-7 {
            if e.fall_distance != 0.0 && length >= 1.0 {
                // A long fall through vines, water and the like resets.
                let check = length.sqrt().min(8.0);
                let unit = normalize(movement);
                let to = [e.pos[0] + unit[0] * check, e.pos[1] + unit[1] * check, e.pos[2] + unit[2] * check];
                if self.clip_hits(e.pos, to, super::physics::ClipBlocks::FallDamageResetting, true) {
                    e.fall_distance = 0.0;
                }
            }
            let from = e.pos;
            let to = [from[0] + movement[0], from[1] + movement[1], from[2] + movement[2]];
            if e.movement_this_tick.len() >= 100 {
                let first = e.movement_this_tick.remove(0);
                let second = e.movement_this_tick.remove(0);
                e.movement_this_tick.insert(0, Movement { from: first.from, to: second.to, axis_dependent: None });
            }
            e.movement_this_tick.push(Movement { from, to, axis_dependent: Some(delta) });
            e.set_pos(to);
        }
        let x_collision = !mth_equal(delta[0], movement[0]);
        let z_collision = !mth_equal(delta[2], movement[2]);
        e.horizontal_collision = x_collision || z_collision;
        let moved_vertically = delta[1].abs() > 0.0;
        // Server entities are authoritative.
        e.vertical_collision = delta[1] != movement[1];
        e.vertical_collision_below = e.vertical_collision && delta[1] < 0.0;
        let below = e.vertical_collision_below;
        self.set_on_ground_with_movement(e, below, movement);
        let effect_pos = self.on_pos(e, 0.2);
        let effect_state = self.block(effect_pos);
        self.check_fall_damage(e, movement[1], effect_state, effect_pos);
        if e.removed {
            return;
        }
        if moved_vertically && e.vertical_collision || e.horizontal_collision {
            self.restitute(e, effect_state, x_collision, z_collision, movement);
        }
        let factor = f64::from(self.block_speed_factor(e));
        e.delta = [e.delta[0] * factor, e.delta[1], e.delta[2] * factor];
    }

    /// `Entity.setOnGroundWithMovement` and `checkSupportingBlock`.
    fn set_on_ground_with_movement(&mut self, e: &mut Entity, on_ground: bool, movement: [f64; 3]) {
        e.on_ground = on_ground;
        if on_ground {
            let bb = e.bb;
            let test = Aabb { min: [bb.min[0], bb.min[1] - 1.0e-6, bb.min[2]], max: [bb.max[0], bb.min[1], bb.max[2]] };
            let mut support = self.find_supporting_block(e, &test);
            if support.is_some() || e.on_ground_no_blocks {
                e.main_support = support;
            } else {
                support = self.find_supporting_block(e, &test.moved([-movement[0], 0.0, -movement[2]]));
                e.main_support = support;
            }
            e.on_ground_no_blocks = support.is_none();
        } else {
            e.on_ground_no_blocks = false;
            e.main_support = None;
        }
    }

    /// `CollisionGetter.findSupportingBlock`: the colliding block nearest
    /// the entity's position.
    fn find_supporting_block(&self, e: &Entity, test: &Aabb) -> Option<BlockPos> {
        let mut best: Option<(BlockPos, f64)> = None;
        for pos in self.block_collision_positions(test) {
            let d = [f64::from(pos.x) + 0.5 - e.pos[0], f64::from(pos.y) + 0.5 - e.pos[1], f64::from(pos.z) + 0.5 - e.pos[2]];
            let distance = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            let better = match best {
                None => true,
                Some((current, current_distance)) => distance < current_distance || distance == current_distance && block_pos_cmp(current, pos) < 0,
            };
            if better {
                best = Some((pos, distance));
            }
        }
        best.map(|(p, _)| p)
    }

    /// `Entity.getOnPos(offset)`.
    fn on_pos(&self, e: &Entity, offset: f32) -> BlockPos {
        if let Some(support) = e.main_support {
            if offset <= 1.0e-5 {
                return support;
            }
            let below = self.block(support);
            let blocks = &self.registries().blocks;
            let info = blocks.block(blocks.block_of(below));
            let fence = self.lib.registries.block_in_tag(below, self.fences_tag);
            let wall = self.lib.registries.block_in_tag(below, self.walls_tag);
            if (f64::from(offset) > 0.5 || !fence) && !wall && !info.is_a("FenceGateBlock") {
                return BlockPos::new(support.x, floor(e.pos[1] - f64::from(offset)), support.z);
            }
            return support;
        }
        BlockPos::new(floor(e.pos[0]), floor(e.pos[1] - f64::from(offset)), floor(e.pos[2]))
    }

    /// `Entity.checkFallDamage`: falling is counted and ends on the ground.
    fn check_fall_damage(&mut self, e: &mut Entity, ya: f64, _state: BlockStateId, _pos: BlockPos) {
        if ya < 0.0 {
            e.fall_distance -= f64::from(ya as f32);
        }
        if e.on_ground {
            // `Block.fallOn`: items take no fall damage; farmland and turtle
            // eggs react only to living entities.
            e.fall_distance = 0.0;
        }
    }

    /// `Entity.restituteMovementAfterCollisions` (non-living, no bounce of
    /// their own; slime blocks bounce).
    fn restitute(&mut self, e: &mut Entity, effect_state: BlockStateId, x_collision: bool, z_collision: bool, movement: [f64; 3]) {
        let current = e.delta;
        let mut after = current;
        if x_collision {
            after[0] = -current[0] * 0.0;
        }
        if z_collision {
            after[2] = -current[2] * 0.0;
        }
        if e.vertical_collision {
            let mut restitution = 0.0;
            if e.vertical_collision_below {
                let gravity = e.gravity();
                let suppresses = self.lib.registries.block_in_tag(effect_state, self.suppresses_bounce_tag);
                if !(-current[1] <= gravity) && !suppresses && self.name(effect_state) == "minecraft:slime_block" {
                    restitution = f64::from(1.0f32 * 0.8f32);
                }
            }
            let (compensation, drag) = if restitution > 0.0 {
                let portion = movement[1] / current[1];
                (portion * e.gravity(), 1.0 + portion * (f64::from(0.98f32) - 1.0))
            } else {
                (0.0, 1.0)
            };
            after[1] = (compensation - current[1]) * drag * restitution;
            if restitution > 0.0 {
                self.unsupported.push("slime bounce events".to_owned());
            }
        }
        e.delta = after;
    }

    /// `Entity.getBlockSpeedFactor`.
    fn block_speed_factor(&self, e: &Entity) -> f32 {
        let here = self.block(BlockPos::new(floor(e.pos[0]), floor(e.pos[1]), floor(e.pos[2])));
        let name = self.name(here);
        let factor = speed_factor(name);
        if name == "minecraft:water" || name == "minecraft:bubble_column" {
            return factor;
        }
        if factor == 1.0 {
            speed_factor(self.name(self.block(self.on_pos(e, 0.500001))))
        } else {
            factor
        }
    }

    /// `Entity.applyEffectsFromBlocks()` after a move.
    pub(super) fn entity_apply_effects_from_blocks(&mut self, e: &mut Entity) {
        self.apply_effects_from_blocks(e);
    }

    fn apply_effects_from_blocks(&mut self, e: &mut Entity) {
        e.final_movements = std::mem::take(&mut e.movement_this_tick);
        if e.final_movements.is_empty() {
            e.final_movements.push(Movement { from: e.old, to: e.pos, axis_dependent: None });
        } else {
            let last = e.final_movements.last().expect("non-empty").to;
            let d = [last[0] - e.pos[0], last[1] - e.pos[1], last[2] - e.pos[2]];
            if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > f64::from(9.999_999_4e-11_f32) {
                e.final_movements.push(Movement { from: last, to: e.pos, axis_dependent: None });
            }
        }
        let movements = e.final_movements.clone();
        self.apply_effects_from_movements(e, &movements);
    }

    /// `Entity.applyEffectsFromBlocks(movements)` and `checkInsideBlocks`.
    fn apply_effects_from_movements(&mut self, e: &mut Entity, movements: &[Movement]) {
        if e.removed || e.no_physics {
            return;
        }
        if e.on_ground {
            let pos = self.on_pos(e, 0.2);
            let state = self.block(pos);
            self.step_on(e, state, pos);
        }
        let mut visited: HashSet<BlockPos> = HashSet::new();
        for movement in movements {
            let mut max_iterations = 16;
            let delta = [movement.to[0] - movement.from[0], movement.to[1] - movement.from[1], movement.to[2] - movement.from[2]];
            let delta_sqr = delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2];
            match movement.axis_dependent {
                Some(original) if delta_sqr > 0.0 => {
                    let order = if original[0].abs() < original[2].abs() { [1, 2, 0] } else { [1, 0, 2] };
                    let mut pos = movement.from;
                    for axis in order {
                        if delta[axis] != 0.0 {
                            let mut to = pos;
                            to[axis] += delta[axis];
                            max_iterations -= self.check_inside_blocks(e, pos, to, &mut visited, max_iterations);
                            pos = to;
                        }
                    }
                }
                _ => max_iterations -= self.check_inside_blocks(e, movement.from, movement.to, &mut visited, 16),
            }
            if max_iterations <= 0 {
                self.check_inside_blocks(e, movement.to, movement.to, &mut visited, 1);
            }
        }
    }

    /// One segment of `Entity.checkInsideBlocks`; returns iterations used.
    fn check_inside_blocks(&mut self, e: &mut Entity, from: [f64; 3], to: [f64; 3], visited: &mut HashSet<BlockPos>, max_iterations: i32) -> i32 {
        let at_target = e.make_bounding_box(to).inflate(-f64::from(1.0e-5f32));
        let mut last = 0;
        for (pos, iteration) in blocks_intersected_between(from, to, &at_target) {
            if e.removed || iteration >= max_iterations {
                break;
            }
            last = iteration;
            let state = self.block(pos);
            if self.registries().blocks.is_air(state) {
                continue;
            }
            // `getEntityInsideCollisionShape` is the full block for every
            // simulated block.
            if visited.insert(pos) {
                self.entity_inside(e, state, pos);
            }
        }
        last + 1
    }

    /// `BlockBehaviour.stepOn`.
    fn step_on(&mut self, _e: &mut Entity, state: BlockStateId, _pos: BlockPos) {
        let name = self.name(state);
        if matches!(name, "minecraft:redstone_ore" | "minecraft:deepslate_redstone_ore" | "minecraft:sculk_sensor" | "minecraft:calibrated_sculk_sensor" | "minecraft:sculk_shrieker") {
            self.unsupported.push(format!("stepping on {name}"));
        }
    }

    /// `BlockBehaviour.entityInside`.
    fn entity_inside(&mut self, e: &mut Entity, state: BlockStateId, pos: BlockPos) {
        let blocks = &self.registries().blocks;
        let info = blocks.block(blocks.block_of(state));
        if info.is_a("BasePressurePlateBlock") {
            if self.plate_signal(state) == 0 {
                self.plate_check_pressed(pos, state, 0);
            }
            return;
        }
        const CALLBACKS: [&str; 22] = [
            "BaseFireBlock", "BigDripleafBlock", "BubbleColumnBlock", "ButtonBlock", "CactusBlock", "CampfireBlock", "CropBlock", "DetectorRailBlock",
            "EndGatewayBlock", "EndPortalBlock", "EyeblossomBlock", "FrogspawnBlock", "HoneyBlock", "HopperBlock", "LavaCauldronBlock",
            "LayeredCauldronBlock", "LilyPadBlock", "NetherPortalBlock", "PowderSnowBlock", "SweetBerryBushBlock", "TripWireBlock", "WebBlock",
        ];
        if let Some(class) = CALLBACKS.iter().find(|c| info.is_a(c)) {
            let _ = e;
            self.unsupported.push(format!("entity inside {class}"));
        }
    }

    // ---- pressure plates --------------------------------------------------------------

    /// `getSignalForState`.
    pub(super) fn plate_signal(&self, state: BlockStateId) -> i32 {
        let blocks = &self.registries().blocks;
        match blocks.property(state, "power") {
            Some(p) => p.parse().unwrap_or(0),
            None => {
                if blocks.property(state, "powered") == Some("true") {
                    15
                } else {
                    0
                }
            }
        }
    }

    /// `getSignalStrength`: entities touching the plate's detection box.
    fn plate_signal_strength(&self, state: BlockStateId, pos: BlockPos) -> i32 {
        let (x, y, z) = (f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
        let touch = Aabb::new(x + 0.0625, y, z + 0.0625, x + 0.9375, y + 0.25, z + 0.9375);
        let name = self.name(state).to_owned();
        let count = self.entities_in(&touch).filter(|e| !mobs_only(&name) || is_living(e)).count() as i32;
        let weight = match name.as_str() {
            "minecraft:light_weighted_pressure_plate" => Some(15),
            "minecraft:heavy_weighted_pressure_plate" => Some(150),
            _ => None,
        };
        match weight {
            Some(max) => {
                let count = count.min(max);
                if count > 0 {
                    (count.min(max) as f32 / max as f32 * 15.0).ceil() as i32
                } else {
                    0
                }
            }
            None => {
                if count > 0 {
                    15
                } else {
                    0
                }
            }
        }
    }

    /// `BasePressurePlateBlock.checkPressed`.
    pub(super) fn plate_check_pressed(&mut self, pos: BlockPos, state: BlockStateId, old: i32) {
        let signal = self.plate_signal_strength(state, pos);
        if old != signal {
            let next = if self.registries().blocks.property(state, "power").is_some() {
                self.with(state, "power", &signal.to_string())
            } else {
                self.with(state, "powered", if signal > 0 { "true" } else { "false" })
            };
            self.set_block(pos, next, update::CLIENTS, update::LIMIT);
            let block = self.block_id(state);
            self.update_neighbors_at(pos, block);
            self.update_neighbors_at(pos.below(), block);
        }
        if signal > 0 {
            let block = self.block_id(state);
            let delay = if self.registries().blocks.property(state, "power").is_some() { 10 } else { 20 };
            self.schedule_block_tick_priority(pos, block, delay, super::redstone::priority::NORMAL);
        }
    }

    // ---- item entities -----------------------------------------------------------------

    /// `ItemEntity.tick`.
    fn tick_item(&mut self, e: &mut Entity) {
        let EntityKind::Item(data) = &e.kind else { return };
        if data.stack.is_empty() {
            e.removed = true;
            return;
        }
        self.base_tick(e);
        let EntityKind::Item(data) = &mut e.kind else { return };
        if data.pickup_delay > 0 && data.pickup_delay != NEVER_PICK_UP {
            data.pickup_delay -= 1;
        }
        e.old = e.pos;
        if e.was_touching_water && e.water_height > f64::from(0.1f32) {
            // `setUnderwaterMovement`.
            let dy = e.delta[1] + if e.delta[1] < f64::from(0.06f32) { f64::from(5.0e-4f32) } else { 0.0 };
            e.delta = [e.delta[0] * f64::from(0.99f32), dy, e.delta[2] * f64::from(0.99f32)];
        } else if !e.first_tick && e.lava_height > f64::from(0.1f32) {
            // `setUnderLavaMovement`.
            let dy = e.delta[1] + if e.delta[1] < f64::from(0.06f32) { f64::from(5.0e-4f32) } else { 0.0 };
            e.delta = [e.delta[0] * f64::from(0.95f32), dy, e.delta[2] * f64::from(0.95f32)];
        } else {
            let gravity = e.gravity();
            if gravity != 0.0 {
                // `add(0.0, -gravity, 0.0)`: adding zero turns -0.0 into 0.0.
                e.delta = [e.delta[0] + 0.0, e.delta[1] - gravity, e.delta[2] + 0.0];
            }
        }
        e.no_physics = !self.no_block_collision(&e.bb.inflate(-1.0e-7));
        if e.no_physics {
            self.unsupported.push("item moving out of blocks".to_owned());
        }
        let horizontal = e.delta[0] * e.delta[0] + e.delta[2] * e.delta[2];
        if e.on_ground && !(horizontal > f64::from(1.0e-5f32)) && (e.tick_count + e.id) % 4 != 0 {
            let movements = e.final_movements.clone();
            self.apply_effects_from_movements(e, &movements);
        } else {
            let delta = e.delta;
            self.entity_move(e, delta);
            self.apply_effects_from_blocks(e);
            let air = f64::from(0.98f32);
            let mut ground = 0.98f32;
            if e.on_ground {
                ground *= friction(self.name(self.block(self.on_pos(e, 0.999999))));
            }
            let ground = f64::from(ground);
            e.delta = [e.delta[0] * ground, e.delta[1] * air, e.delta[2] * ground];
            if e.on_ground && e.delta[1] < 0.0 {
                e.delta[1] *= -0.5;
            }
        }
        let moved = floor(e.old[0]) != floor(e.pos[0]) || floor(e.old[1]) != floor(e.pos[1]) || floor(e.old[2]) != floor(e.pos[2]);
        let rate = if moved { 2 } else { 40 };
        if e.tick_count % rate == 0 && self.item_mergable(e) {
            self.merge_with_neighbours(e);
        }
        let EntityKind::Item(data) = &mut e.kind else { return };
        if data.age != -32768 {
            data.age += 1;
        }
        self.update_fluid_interaction(e);
        let EntityKind::Item(data) = &e.kind else { return };
        if data.age >= 6000 {
            e.removed = true;
        }
    }

    fn item_mergable(&self, e: &Entity) -> bool {
        let EntityKind::Item(data) = &e.kind else { return false };
        !e.removed && data.pickup_delay != NEVER_PICK_UP && data.age != -32768 && data.age < 6000 && data.stack.count < self.item_max_stack(&data.stack)
    }

    /// `ItemEntity.mergeWithNeighbours`.
    fn merge_with_neighbours(&mut self, e: &mut Entity) {
        let bb = Aabb { min: [e.bb.min[0] - 0.5, e.bb.min[1], e.bb.min[2] - 0.5], max: [e.bb.max[0] + 0.5, e.bb.max[1], e.bb.max[2] + 0.5] };
        let candidates: Vec<usize> = (0..self.entities.len())
            .filter(|&i| {
                let o = &self.entities[i];
                o.id != e.id && matches!(o.kind, EntityKind::Item(_)) && o.bb.intersects(&bb) && self.item_mergable(o)
            })
            .collect();
        for i in candidates {
            if !self.item_mergable(&self.entities[i]) {
                continue;
            }
            let mut other = std::mem::replace(&mut self.entities[i], e.clone());
            self.try_to_merge(e, &mut other);
            self.entities[i] = other;
            if e.removed {
                break;
            }
        }
    }

    /// `ItemEntity.tryToMerge`: the bigger stack takes from the smaller.
    fn try_to_merge(&mut self, this: &mut Entity, other: &mut Entity) {
        let (EntityKind::Item(a), EntityKind::Item(b)) = (&this.kind, &other.kind) else { return };
        let (this_stack, other_stack) = (a.stack.clone(), b.stack.clone());
        if other_stack.count + this_stack.count > self.item_max_stack(&other_stack) || !this_stack.same_item_same_components(&other_stack) {
            return;
        }
        if other_stack.count < this_stack.count {
            self.merge_items(this, other);
        } else {
            self.merge_items(other, this);
        }
    }

    /// `ItemEntity.merge(toItem, toStack, fromItem, fromStack)`.
    fn merge_items(&mut self, to: &mut Entity, from: &mut Entity) {
        let (EntityKind::Item(to_data), EntityKind::Item(from_data)) = (&mut to.kind, &mut from.kind) else { return };
        let max = self.lib.registries.items.max_stack(&to_data.stack.id).min(64);
        let delta = (max - to_data.stack.count).min(from_data.stack.count);
        to_data.stack.count += delta;
        from_data.stack.count -= delta;
        to_data.pickup_delay = to_data.pickup_delay.max(from_data.pickup_delay);
        to_data.age = to_data.age.min(from_data.age);
        if from_data.stack.is_empty() {
            from.removed = true;
        }
    }

    // ---- experience orbs --------------------------------------------------------------

    /// `ExperienceOrb.award`: `amount` in orbs of vanilla's sizes, largest
    /// first. Each joins a matching orb within half a block whose ID falls
    /// in the group the level random picks, or is thrown up and out as a
    /// new orb.
    pub fn award_experience(&mut self, pos: [f64; 3], mut amount: i32) {
        use minecraftoss_core::random::RandomSource;
        while amount > 0 {
            let value = orb_value(amount);
            amount -= value;
            // `tryMergeToExisting`.
            let group = self.random.next_i32_bound(40);
            let bb = Aabb::new(pos[0] - 0.5, pos[1] - 0.5, pos[2] - 0.5, pos[0] + 0.5, pos[1] + 0.5, pos[2] + 0.5);
            let joined = self.entities.iter_mut().find(|o| {
                !o.removed && o.bb.intersects(&bb) && (o.id - group) % 40 == 0 && o.orb_data().is_some_and(|d| d.value == value)
            });
            if let Some(data) = joined.and_then(Entity::orb_data_mut) {
                data.count += 1;
                data.age = 0;
                continue;
            }
            // `ExperienceOrb(level, pos, Vec3.ZERO, value)`: a random yaw,
            // then the throw, from the orb's own random.
            let _yaw = self.entity_random.next_f32() * 360.0;
            let dx = (self.entity_random.next_f64() * 0.2 - 0.1) * 2.0;
            let dy = self.entity_random.next_f64() * 0.2 * 2.0;
            let dz = (self.entity_random.next_f64() * 0.2 - 0.1) * 2.0;
            let mut orb = Entity::experience_orb(pos, value);
            orb.delta = [dx, dy, dz];
            if !self.no_block_collision(&orb.bb) {
                self.unsupported.push("experience orb placed inside blocks".to_owned());
            }
            self.add_entity(orb);
        }
    }

    /// `new ExperienceOrb(level, x, y, z, value)` added to the level whole
    /// (a trade's reward: neither split nor merged): a random yaw and a
    /// throw from the orb's own random.
    pub fn spawn_experience_orb(&mut self, pos: [f64; 3], value: i32) {
        use minecraftoss_core::random::RandomSource;
        let _yaw = self.entity_random.next_f32() * 360.0;
        let dx = (self.entity_random.next_f64() * 0.2 - 0.1) * 2.0;
        let dy = self.entity_random.next_f64() * 0.2 * 2.0;
        let dz = (self.entity_random.next_f64() * 0.2 - 0.1) * 2.0;
        let mut orb = Entity::experience_orb(pos, value);
        orb.delta = [dx, dy, dz];
        self.add_entity(orb);
    }

    /// `ExperienceOrb.playerTouch` for a player standing at `feet` who can
    /// take experience (`takeXpDelay` 0): `Player.aiStep` touches one of
    /// the orbs within its box grown by 1, 0.5, 1, picked at random. The
    /// orb gives one pickup's worth and goes when it has none left. Returns
    /// the orb's ID, position and value.
    pub fn player_touch_orb(&mut self, feet: [f64; 3]) -> Option<(i32, [f64; 3], i32)> {
        use minecraftoss_core::random::RandomSource;
        let player = Aabb::for_entity(feet[0], feet[1], feet[2], 0.6, 1.8);
        let touch = Aabb::new(player.min[0] - 1.0, player.min[1] - 0.5, player.min[2] - 1.0, player.max[0] + 1.0, player.max[1] + 0.5, player.max[2] + 1.0);
        let touching: Vec<usize> = (0..self.entities.len())
            .filter(|&i| {
                let e = &self.entities[i];
                !e.removed && e.orb_data().is_some() && e.bb.intersects(&touch)
            })
            .collect();
        if touching.is_empty() {
            return None;
        }
        // `Util.getRandom(orbs, player.random)`.
        let chosen = touching[self.entity_random.next_i32_bound(touching.len() as i32) as usize];
        let orb = &mut self.entities[chosen];
        let (id, pos) = (orb.id, orb.pos);
        let data = orb.orb_data_mut()?;
        let value = data.value;
        data.count -= 1;
        if data.count == 0 {
            orb.removed = true;
            self.entities.remove(chosen);
        }
        Some((id, pos, value))
    }

    /// The experience the level's orbs hold (each orb's value times its
    /// merged count).
    pub fn experience_total(&self) -> i64 {
        self.entities.iter().filter(|e| !e.removed).filter_map(Entity::orb_data).map(|d| i64::from(d.value) * i64::from(d.count)).sum()
    }

    /// `ExperienceOrb.tick`.
    fn tick_orb(&mut self, e: &mut Entity) {
        use minecraftoss_core::block::FluidKind;
        use minecraftoss_core::random::RandomSource;
        self.base_tick(e);
        let colliding = !self.no_block_collision(&e.bb);
        if e.eye_in_water {
            // `setUnderwaterMovement`.
            let d = e.delta;
            e.delta = [d[0] * f64::from(0.99f32), (d[1] + f64::from(5.0e-4f32)).min(f64::from(0.06f32)), d[2] * f64::from(0.99f32)];
        } else if !colliding {
            // `applyGravity`: adding zero turns -0.0 into 0.0.
            let gravity = e.gravity();
            if gravity != 0.0 {
                e.delta = [e.delta[0] + 0.0, e.delta[1] - gravity, e.delta[2] + 0.0];
            }
        }
        let feet = BlockPos::new(floor(e.pos[0]), floor(e.pos[1]), floor(e.pos[2]));
        if self.fluid_state(self.block(feet)).is_some_and(|f| f.kind == FluidKind::Lava) {
            let mut r = || (self.entity_random.next_f32() - self.entity_random.next_f32()) * 0.2;
            let dx = r();
            let dz = r();
            e.delta = [f64::from(dx), f64::from(0.2f32), f64::from(dz)];
        }
        if e.tick_count % 20 == 1 {
            self.scan_for_orb_merges(e);
        }
        self.follow_nearby_player(e);
        let following = e.orb_data().and_then(|d| d.following).is_some();
        if !following && colliding {
            let d = e.delta;
            let next = Aabb::new(e.bb.min[0] + d[0], e.bb.min[1] + d[1], e.bb.min[2] + d[2], e.bb.max[0] + d[0], e.bb.max[1] + d[1], e.bb.max[2] + d[2]);
            if !self.no_block_collision(&next) {
                self.unsupported.push("experience orb moving towards the closest space".to_owned());
            }
        }
        let fall_speed = e.delta[1];
        let delta = e.delta;
        self.entity_move(e, delta);
        self.apply_effects_from_blocks(e);
        let mut drag = 0.98f32;
        if e.on_ground {
            drag *= friction(self.name(self.block(self.on_pos(e, 0.999999))));
        }
        let drag = f64::from(drag);
        e.delta = [e.delta[0] * drag, e.delta[1] * drag, e.delta[2] * drag];
        if e.vertical_collision_below && fall_speed < -e.gravity() {
            e.delta[1] = -fall_speed * 0.4;
        }
        let Some(data) = e.orb_data_mut() else { return };
        data.age += 1;
        if data.age >= 6000 {
            e.removed = true;
        }
    }

    /// `ExperienceOrb.scanForMerges`: orbs of the same value within half a
    /// block whose IDs are in this orb's group of forty join it.
    fn scan_for_orb_merges(&mut self, e: &mut Entity) {
        let Some(value) = e.orb_data().map(|d| d.value) else { return };
        let bb = e.bb.inflate(0.5);
        for other in &mut self.entities {
            if other.id == e.id || other.removed || !other.bb.intersects(&bb) || (other.id - e.id) % 40 != 0 {
                continue;
            }
            let Some(theirs) = other.orb_data().filter(|d| d.value == value).cloned() else { continue };
            let Some(ours) = e.orb_data_mut() else { return };
            ours.count += theirs.count;
            ours.age = ours.age.min(theirs.age);
            other.removed = true;
        }
    }

    /// `ExperienceOrb.followNearbyPlayer`: the nearest living player within
    /// eight blocks pulls the orb towards its middle, harder the closer.
    fn follow_nearby_player(&mut self, e: &mut Entity) {
        let pos = e.pos;
        let distance_sqr = |p: [f64; 3]| {
            let (dx, dy, dz) = (p[0] - pos[0], p[1] - pos[1], p[2] - pos[2]);
            dx * dx + dy * dy + dz * dz
        };
        let players = &self.living_players;
        let Some(data) = e.orb_data_mut() else { return };
        let keep = data.following.and_then(|i| players.get(i)).is_some_and(|&(feet, _)| distance_sqr(feet) <= 64.0);
        if !keep {
            // `getNearestPlayer(this, 8.0)`: strictly within eight blocks.
            let mut best: Option<(usize, f64)> = None;
            for (i, &(feet, _)) in players.iter().enumerate() {
                let d = distance_sqr(feet);
                if d < 64.0 && best.is_none_or(|(_, b)| d < b) {
                    best = Some((i, d));
                }
            }
            data.following = best.map(|(i, _)| i);
        }
        let Some((feet, eye_height)) = data.following.map(|i| players[i]) else { return };
        let to = [feet[0] - pos[0], feet[1] + eye_height / 2.0 - pos[1], feet[2] - pos[2]];
        let length = to[0] * to[0] + to[1] * to[1] + to[2] * to[2];
        let power = 1.0 - length.sqrt() / 8.0;
        let pull = power * power * 0.1;
        let n = normalize(to);
        e.delta = [e.delta[0] + n[0] * pull, e.delta[1] + n[1] * pull, e.delta[2] + n[2] * pull];
    }

    /// `/kill @e[tag=...]`.
    pub fn kill_tagged(&mut self, tag: &str) {
        for e in &mut self.entities {
            if e.tags.iter().any(|t| t == tag) {
                e.removed = true;
            }
        }
        self.entities.retain(|e| !e.removed);
    }
}

/// Stone and polished blackstone plates sense only living entities.
fn mobs_only(name: &str) -> bool {
    matches!(name, "minecraft:stone_pressure_plate" | "minecraft:polished_blackstone_pressure_plate")
}

fn is_living(e: &Entity) -> bool {
    match e.kind {
        EntityKind::Item(_) | EntityKind::LightningBolt(_) | EntityKind::PrimedTnt(_) | EntityKind::FallingBlock(_) | EntityKind::ExperienceOrb(_) => false,
    }
}

/// `BlockPos.compareTo` (`Vec3i`: y, then z, then x).
fn block_pos_cmp(a: BlockPos, b: BlockPos) -> i32 {
    let ord = a.y.cmp(&b.y).then(a.z.cmp(&b.z)).then(a.x.cmp(&b.x));
    ord as i32
}
