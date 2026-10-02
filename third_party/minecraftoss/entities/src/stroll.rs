//! 26.3 random stroll and panic position choice. Source:
//! `RandomStrollGoal`, `WaterAvoidingRandomStrollGoal`, `LandRandomPos`,
//! `DefaultRandomPos`, `RandomPos` and `GoalUtils` in the pinned 26.3 common
//! JAR, with `Animal.getWalkTargetValue`. Mobs here have no home, so no
//! position is restricted.
use crate::walk_path::{path_type_static, WalkProfile, WalkTarget};
use glam::DVec3;
use minecraftoss_player::{rng::LegacyRandom, World};

type BlockPos = (i32, i32, i32);

/// `getWalkTargetValue`: for animals grass below is worth 10, elsewhere
/// the light-dependent cost; monsters take the negated cost (darkness).
fn walk_target_value(world: &dyn World, profile: &WalkProfile, (x, y, z): BlockPos) -> f64 {
    match profile.walk_target {
        WalkTarget::Animal => {
            let grass = world.block((x, y - 1, z)).is_some_and(|b| b.id == "minecraft:grass_block");
            f64::from(if grass { 10.0 } else { world.light_path_cost((x, y, z)) })
        }
        WalkTarget::Monster => f64::from(-world.light_path_cost((x, y, z))),
        WalkTarget::Zero => 0.0,
    }
}

/// `RandomPos.generateRandomDirection`.
fn random_direction(random: &mut LegacyRandom, horizontal: u32, vertical: u32) -> BlockPos {
    let x = random.next_int(2 * horizontal + 1) as i32 - horizontal as i32;
    let y = random.next_int(2 * vertical + 1) as i32 - vertical as i32;
    let z = random.next_int(2 * horizontal + 1) as i32 - horizontal as i32;
    (x, y, z)
}

/// `RandomPos.generateRandomPosTowardDirection` for a mob with no home.
fn toward(position: DVec3, (dx, dy, dz): BlockPos) -> BlockPos {
    (
        (f64::from(dx) + position.x).floor() as i32,
        (f64::from(dy) + position.y).floor() as i32,
        (f64::from(dz) + position.z).floor() as i32,
    )
}

/// `GoalUtils.isOutsideLimits`, `isNotStable` (`PathNavigation.isStableDestination`:
/// a solid-render block below).
fn unstable_or_outside(world: &dyn World, (x, y, z): BlockPos) -> bool {
    y < world.min_y() || y > world.max_y() || !world.solid_render((x, y - 1, z))
}

/// `GoalUtils.hasMalus`.
fn has_malus(world: &dyn World, profile: &WalkProfile, pos: BlockPos) -> bool {
    profile.malus(path_type_static(world, pos)) != 0.0
}

/// `RandomPos.generateRandomPos`: the best of ten candidates by walk
/// target value, the first on ties.
fn best_of(world: &dyn World, profile: &WalkProfile, candidate: impl FnMut() -> Option<BlockPos>) -> Option<DVec3> {
    best_by(|pos| walk_target_value(world, profile, pos), candidate)
}

/// `RandomPos.generateRandomPos` by a weight of its own.
fn best_by(mut weight: impl FnMut(BlockPos) -> f64, mut candidate: impl FnMut() -> Option<BlockPos>) -> Option<DVec3> {
    let mut best = None;
    let mut best_weight = f64::NEG_INFINITY;
    for _ in 0..10 {
        if let Some(pos) = candidate() {
            let weight = weight(pos);
            if weight > best_weight {
                best_weight = weight;
                best = Some(pos);
            }
        }
    }
    best.map(|(x, y, z)| DVec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5))
}

/// One `LandRandomPos.getPos` candidate: a stable spot moved up out of
/// solid blocks, kept unless in water or costly to stand in.
fn land_candidate(world: &dyn World, profile: &WalkProfile, position: DVec3, horizontal: u32, vertical: u32, random: &mut LegacyRandom) -> Option<BlockPos> {
    let direction = random_direction(random, horizontal, vertical);
    let pos = toward(position, direction);
    if unstable_or_outside(world, pos) {
        return None;
    }
    // `RandomPos.moveUpOutOfSolid`.
    let (x, mut y, z) = pos;
    if world.solid((x, y, z)) {
        y += 1;
        while y <= world.max_y() && world.solid((x, y, z)) {
            y += 1;
        }
    }
    let pos = (x, y, z);
    (!world.floatable_fluid(pos) && !has_malus(world, profile, pos)).then_some(pos)
}

/// `LandRandomPos.getPos`: a stable candidate is moved up out of solid
/// blocks, then kept unless in water or costly to stand in.
pub fn land_random_position(world: &dyn World, profile: &WalkProfile, position: DVec3, horizontal: u32, vertical: u32, random: &mut LegacyRandom) -> Option<DVec3> {
    best_of(world, profile, || land_candidate(world, profile, position, horizontal, vertical, random))
}

/// `LandRandomPos.getPos(mob, horizontal, vertical, weight)`: as
/// [`land_random_position`], the candidates weighed by `weight`.
pub fn land_random_position_by(world: &dyn World, profile: &WalkProfile, position: DVec3, horizontal: u32, vertical: u32, random: &mut LegacyRandom, weight: impl FnMut(BlockPos) -> f64) -> Option<DVec3> {
    best_by(weight, || land_candidate(world, profile, position, horizontal, vertical, random))
}

/// `DefaultRandomPos.getPos`: a stable candidate that costs nothing to
/// stand in.
pub fn default_random_position(world: &dyn World, profile: &WalkProfile, position: DVec3, horizontal: u32, vertical: u32, random: &mut LegacyRandom) -> Option<DVec3> {
    best_of(world, profile, || {
        let direction = random_direction(random, horizontal, vertical);
        let pos = toward(position, direction);
        (!unstable_or_outside(world, pos) && !has_malus(world, profile, pos)).then_some(pos)
    })
}

#[derive(Clone, Debug, Default)]
pub struct StrollState {
    pub wanted: Option<DVec3>,
}

impl StrollState {
    /// `RandomStrollGoal.canUse` with `WaterAvoidingRandomStrollGoal`'s
    /// position: land positions, from farther in water, and one time in a
    /// thousand on dry land any stable one.
    pub fn can_use(&mut self, position: DVec3, no_action_time: i32, in_water: bool, world: &dyn World, profile: &WalkProfile, random: &mut LegacyRandom) -> bool {
        self.can_use_with(position, no_action_time, in_water, world, profile, random, 0.001)
    }

    /// [`StrollState::can_use`] with the goal's `probability` of any stable
    /// spot on dry land (the enderman's is 0).
    #[allow(clippy::too_many_arguments)]
    pub fn can_use_with(&mut self, position: DVec3, no_action_time: i32, in_water: bool, world: &dyn World, profile: &WalkProfile, random: &mut LegacyRandom, probability: f32) -> bool {
        if no_action_time >= 100 || random.next_int(60) != 0 {
            return false;
        }
        self.wanted = if in_water {
            land_random_position(world, profile, position, 15, 7, random).or_else(|| default_random_position(world, profile, position, 10, 7, random))
        } else if random.next_float() >= probability {
            land_random_position(world, profile, position, 10, 7, random)
        } else {
            default_random_position(world, profile, position, 10, 7, random)
        };
        self.wanted.is_some()
    }
}

/// `RandomPos.generateRandomDirectionWithinRadians`: a direction within
/// `max_radians` of (`x_dir`, `z_dir`), out to `max` blocks (the square's
/// corners cut off), `vertical` up or down.
fn random_direction_within(random: &mut LegacyRandom, min: f64, max: f64, vertical: u32, x_dir: f64, z_dir: f64, max_radians: f64) -> Option<BlockPos> {
    let center = crate::control::minecraft_atan2(z_dir, x_dir) - f64::from(std::f32::consts::FRAC_PI_2);
    let radians = center + f64::from(2.0 * random.next_float() - 1.0) * max_radians;
    // `Mth.lerp(sqrt(nextDouble), min, max) * Mth.SQRT_OF_TWO`.
    let distance = (min + random.next_double().sqrt() * (max - min)) * f64::from(std::f32::consts::SQRT_2);
    let xt = -distance * minecraftoss_player::jmath::sin(radians);
    let zt = distance * minecraftoss_player::jmath::cos(radians);
    if xt.abs() > max || zt.abs() > max {
        return None;
    }
    let yt = random.next_int(2 * vertical + 1) as i32 - vertical as i32;
    Some((xt.floor() as i32, yt, zt.floor() as i32))
}

/// `DefaultRandomPos.getPosTowards`: a stable, free spot within
/// `max_radians` of the way to `towards`.
#[allow(clippy::too_many_arguments)]
pub fn default_random_position_towards(world: &dyn World, profile: &WalkProfile, position: DVec3, horizontal: u32, vertical: u32, towards: DVec3, max_radians: f32, random: &mut LegacyRandom) -> Option<DVec3> {
    let dir = towards - position;
    best_of(world, profile, || {
        let direction = random_direction_within(random, 0.0, f64::from(horizontal), vertical, dir.x, dir.z, f64::from(max_radians))?;
        let pos = toward(position, direction);
        (!unstable_or_outside(world, pos) && !has_malus(world, profile, pos)).then_some(pos)
    })
}

/// `DefaultRandomPos.getPosAway`: a stable, free spot within a quarter turn
/// of straight away from `avoid` (`mob.position().subtract(avoidPos)`).
pub fn default_random_position_away(world: &dyn World, profile: &WalkProfile, position: DVec3, horizontal: u32, vertical: u32, avoid: DVec3, random: &mut LegacyRandom) -> Option<DVec3> {
    let dir = position - avoid;
    best_of(world, profile, || {
        let direction = random_direction_within(random, 0.0, f64::from(horizontal), vertical, dir.x, dir.z, f64::from(std::f32::consts::FRAC_PI_2))?;
        let pos = toward(position, direction);
        (!unstable_or_outside(world, pos) && !has_malus(world, profile, pos)).then_some(pos)
    })
}

/// `LandRandomPos.getPosAway(mob, 0, horizontal, vertical, avoid)`: a land
/// spot within a quarter turn of the way away from `avoid`.
pub fn land_random_position_away(world: &dyn World, profile: &WalkProfile, position: DVec3, horizontal: u32, vertical: u32, avoid: DVec3, random: &mut LegacyRandom) -> Option<DVec3> {
    let mut dir = position - avoid;
    if dir.length() == 0.0 {
        dir = DVec3::new(random.next_double() - 0.5, 0.0, random.next_double() - 0.5);
    }
    land_random_position_in_direction(world, profile, position, horizontal, vertical, dir, random)
}

/// `LandRandomPos.getPosTowards`: a land spot within a quarter turn of the
/// way to `towards`.
pub fn land_random_position_towards(world: &dyn World, profile: &WalkProfile, position: DVec3, horizontal: u32, vertical: u32, towards: DVec3, random: &mut LegacyRandom) -> Option<DVec3> {
    land_random_position_in_direction(world, profile, position, horizontal, vertical, towards - position, random)
}

/// `LandRandomPos.getPosInDirection` with no minimum distance.
fn land_random_position_in_direction(world: &dyn World, profile: &WalkProfile, position: DVec3, horizontal: u32, vertical: u32, dir: DVec3, random: &mut LegacyRandom) -> Option<DVec3> {
    best_of(world, profile, || {
        let direction = random_direction_within(random, 0.0, f64::from(horizontal), vertical, dir.x, dir.z, f64::from(std::f32::consts::FRAC_PI_2))?;
        let pos = toward(position, direction);
        if unstable_or_outside(world, pos) {
            return None;
        }
        let (x, mut y, z) = pos;
        if world.solid((x, y, z)) {
            y += 1;
            while y <= world.max_y() && world.solid((x, y, z)) {
                y += 1;
            }
        }
        let pos = (x, y, z);
        (!world.floatable_fluid(pos) && !has_malus(world, profile, pos)).then_some(pos)
    })
}
