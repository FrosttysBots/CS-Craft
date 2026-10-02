//! Potions and thrown splash potions (pinned 26.3 `Potions`,
//! `PotionContents`, `Projectile.shoot`/`getMovementToShoot`,
//! `ThrowableProjectile.tick`, `ThrowableItemProjectile`,
//! `ProjectileUtil.getHitResultOnMoveVector`/`getEntityHitResult`/
//! `computeMargin`, `AbstractThrownPotion.onHit` and
//! `ThrownSplashPotion.onHitAsPotion`). A splash potion falls under 0.05
//! gravity with 0.99 drag (0.8 in water), strikes the first block or
//! pickable entity along its motion (the thrower only once it has flown
//! clear of it), and breaks there: every living thing within 4 blocks of
//! its box takes its effects scaled by nearness (instant effects at once,
//! the others for a share of their time).
use crate::effects::{EffectInstance, MobEffect};
use crate::sight;
use glam::DVec3;
use minecraftoss_player::rng::LegacyRandom;
use minecraftoss_player::World;

/// `EntityTypes.SPLASH_POTION`: 0.25 blocks each way.
pub const SIZE: f64 = 0.25;
/// `AbstractThrownPotion.getDefaultGravity`.
pub const GRAVITY: f64 = 0.05;

/// A potion (`Potions`) the witches brew.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Potion {
    Harming,
    Healing,
    Slowness,
    Poison,
    Weakness,
    WaterBreathing,
    FireResistance,
    Swiftness,
    Regeneration,
}

impl Potion {
    pub const ALL: [Potion; 9] = [
        Self::Harming,
        Self::Healing,
        Self::Slowness,
        Self::Poison,
        Self::Weakness,
        Self::WaterBreathing,
        Self::FireResistance,
        Self::Swiftness,
        Self::Regeneration,
    ];

    /// Its registry ID (`potion_contents`' `potion`).
    pub fn id(self) -> &'static str {
        match self {
            Self::Harming => "minecraft:harming",
            Self::Healing => "minecraft:healing",
            Self::Slowness => "minecraft:slowness",
            Self::Poison => "minecraft:poison",
            Self::Weakness => "minecraft:weakness",
            Self::WaterBreathing => "minecraft:water_breathing",
            Self::FireResistance => "minecraft:fire_resistance",
            Self::Swiftness => "minecraft:swiftness",
            Self::Regeneration => "minecraft:regeneration",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    /// Its one effect: the effect, its ticks and level (`Potions`).
    pub fn effect(self) -> EffectInstance {
        let (effect, duration) = match self {
            Self::Harming => (MobEffect::InstantDamage, 1),
            Self::Healing => (MobEffect::InstantHealth, 1),
            Self::Slowness => (MobEffect::Slowness, 1800),
            Self::Poison => (MobEffect::Poison, 900),
            Self::Weakness => (MobEffect::Weakness, 1800),
            Self::WaterBreathing => (MobEffect::WaterBreathing, 3600),
            Self::FireResistance => (MobEffect::FireResistance, 3600),
            Self::Swiftness => (MobEffect::Speed, 3600),
            Self::Regeneration => (MobEffect::Regeneration, 900),
        };
        EffectInstance::new(effect, duration, 0)
    }

    /// `Potion.hasInstantEffects`.
    pub fn instant(self) -> bool {
        self.effect().effect.instantaneous()
    }

    /// `PotionContents.getColor`: its effect's colour (RGB).
    pub fn color(self) -> u32 {
        match self.effect().effect {
            MobEffect::Speed => 3402751,
            MobEffect::Slowness => 9154528,
            MobEffect::InstantHealth => 16262179,
            MobEffect::InstantDamage => 11101546,
            MobEffect::Regeneration => 13458603,
            MobEffect::FireResistance => 16750848,
            MobEffect::WaterBreathing => 10017472,
            MobEffect::Weakness => 4738376,
            MobEffect::Poison => 8889187,
        }
    }
}

/// Something a potion can strike in flight (`canHitEntity`): a living,
/// pickable entity's box, and whether it is the thrower (not struck until
/// the potion has left it).
#[derive(Clone, Copy, Debug)]
pub struct PotionTarget {
    pub id: u64,
    pub min: DVec3,
    pub max: DVec3,
    pub owner: bool,
}

/// What a potion struck this tick: where, and the entity if it was one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PotionHit {
    pub location: DVec3,
    pub entity: Option<u64>,
}

/// A thrown splash potion (`ThrownSplashPotion`).
#[derive(Clone, Debug)]
pub struct ThrownPotion {
    pub position: DVec3,
    pub velocity: DVec3,
    pub potion: Potion,
    /// Who threw it (`getOwner`).
    pub owner: Option<u64>,
    /// `tickCount`, raised before each tick.
    pub tick_count: i32,
    /// `leftOwner`: it has flown clear of its thrower.
    pub left_owner: bool,
    /// `wasTouchingWater` from its last tick.
    pub in_water: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub alive: bool,
    first_tick: bool,
}

impl ThrownPotion {
    /// `ThrowableItemProjectile(type, owner, level, item)` and
    /// `Projectile.shoot`: at the thrower's eyes less 0.1, along
    /// (`xd`, `yd`, `zd`) at `power` with the potion's own random's spread.
    pub fn shoot(potion: Potion, owner: u64, position: DVec3, direction: DVec3, power: f32, uncertainty: f32, random: &mut LegacyRandom) -> Self {
        // `Vec3.normalize`: none under 1.0E-5F long.
        let length = (direction.x * direction.x + direction.y * direction.y + direction.z * direction.z).sqrt();
        let unit = if length < f64::from(1.0e-5_f32) { DVec3::ZERO } else { DVec3::new(direction.x / length, direction.y / length, direction.z / length) };
        // `random.triangle(0, 0.0172275 * uncertainty)` three times.
        let spread = 0.0172275 * f64::from(uncertainty);
        let mut triangle = || spread * (random.next_double() - random.next_double());
        let (tx, ty, tz) = (triangle(), triangle(), triangle());
        let power = f64::from(power);
        let velocity = DVec3::new((unit.x + tx) * power, (unit.y + ty) * power, (unit.z + tz) * power);
        let horizontal = (velocity.x * velocity.x + velocity.z * velocity.z).sqrt();
        let yaw = (crate::control::minecraft_atan2(velocity.x, velocity.z) * minecraftoss_player::mth::RAD_TO_DEG) as f32;
        let pitch = (crate::control::minecraft_atan2(velocity.y, horizontal) * minecraftoss_player::mth::RAD_TO_DEG) as f32;
        Self { position, velocity, potion, owner: Some(owner), tick_count: 0, left_owner: false, in_water: false, yaw, pitch, alive: true, first_tick: true }
    }

    /// Its box (`makeBoundingBox`: 0.125 either side, 0.25 up).
    pub fn bounds(&self) -> (DVec3, DVec3) {
        bounds_at(self.position)
    }

    /// `ProjectileUtil.computeMargin`: the grace a target's box grows by,
    /// from nothing to 0.3 over its first ticks.
    pub fn margin(&self) -> f64 {
        f64::from(0.0_f32.max(0.3_f32.min((self.tick_count - 2) as f32 / 20.0)))
    }

    /// `ThrowableProjectile.tick`: gravity, drag, the hit along its motion,
    /// the move, `Projectile.checkLeftOwner` and the water check of
    /// `Entity.baseTick`. Returns what it struck (it breaks there).
    /// `owner_box` is the thrower's box while it lives.
    pub fn tick(&mut self, world: &impl World, targets: &[PotionTarget], owner_box: Option<(DVec3, DVec3)>) -> Option<PotionHit> {
        self.tick_count += 1;
        // `applyGravity`, then `applyInertia` (water from its last tick).
        self.velocity.y -= GRAVITY;
        let inertia = f64::from(if self.in_water { 0.8_f32 } else { 0.99_f32 });
        self.velocity = DVec3::new(self.velocity.x * inertia, self.velocity.y * inertia, self.velocity.z * inertia);
        let from = self.position;
        let movement = self.velocity;
        let mut to = DVec3::new(from.x + movement.x, from.y + movement.y, from.z + movement.z);
        let block = sight::clip(world, from, to);
        if let Some((_, at)) = block {
            to = at;
        }
        // `getEntityHitResult` in its box swept along the motion and grown
        // by a block, each target's box grown by the margin.
        let (min, max) = self.bounds();
        let (search_min, search_max) = sweep(min, max, movement);
        let (search_min, search_max) = (search_min - DVec3::ONE, search_max + DVec3::ONE);
        let margin = self.margin();
        let mut nearest = f64::MAX;
        let mut entity = None;
        for target in targets {
            if target.owner && !self.left_owner {
                continue;
            }
            if !(target.min.x < search_max.x && target.max.x > search_min.x && target.min.y < search_max.y && target.max.y > search_min.y && target.min.z < search_max.z && target.max.z > search_min.z) {
                continue;
            }
            let grow = DVec3::splat(margin);
            if let Some(at) = sight::clip_box(target.min - grow, target.max + grow, from, to) {
                let d = from.distance_squared(at);
                if d < nearest {
                    nearest = d;
                    entity = Some((target.id, at));
                }
            }
        }
        let hit = match (entity, block) {
            (Some((id, at)), _) => Some(PotionHit { location: at, entity: Some(id) }),
            // `hitTargetOrDeflectSelf`: a block hit on air strikes nothing.
            (None, Some((pos, at))) if world.block(pos).is_some_and(|b| b.id != "minecraft:air") => Some(PotionHit { location: at, entity: None }),
            _ => None,
        };
        self.position = match (entity, block) {
            (Some((_, at)), _) | (None, Some((_, at))) => at,
            _ => DVec3::new(from.x + movement.x, from.y + movement.y, from.z + movement.z),
        };
        self.update_rotation();
        // `Projectile.tick`: `checkLeftOwner` after the hit test.
        if !self.left_owner {
            self.left_owner = match owner_box {
                Some((owner_min, owner_max)) => {
                    let (min, max) = self.bounds();
                    let (min, max) = sweep(min, max, self.velocity);
                    let (min, max) = (min - DVec3::ONE, max + DVec3::ONE);
                    !(owner_min.x < max.x && owner_max.x > min.x && owner_min.y < max.y && owner_max.y > min.y && owner_min.z < max.z && owner_max.z > min.z)
                }
                None => true,
            };
        }
        // `Entity.baseTick`: the water around it for its next drag, and the
        // floor 64 below the world.
        self.in_water = crate::fluid::FluidFrame::sample(world, self.position, SIZE as f32, SIZE as f32).in_water();
        self.first_tick = false;
        if self.position.y < f64::from(world.min_y() - 64) {
            self.alive = false;
        }
        if hit.is_some() && self.alive {
            self.alive = false;
            return hit;
        }
        None
    }

    /// `Projectile.updateRotation`: a fifth of the way to its heading.
    fn update_rotation(&mut self) {
        let v = self.velocity;
        let horizontal = (v.x * v.x + v.z * v.z).sqrt();
        let pitch = (crate::control::minecraft_atan2(v.y, horizontal) * minecraftoss_player::mth::RAD_TO_DEG) as f32;
        let yaw = (crate::control::minecraft_atan2(v.x, v.z) * minecraftoss_player::mth::RAD_TO_DEG) as f32;
        self.pitch = lerp_rotation(self.pitch, pitch);
        self.yaw = lerp_rotation(self.yaw, yaw);
    }
}

/// `Projectile.lerpRotation`.
fn lerp_rotation(mut from: f32, to: f32) -> f32 {
    while to - from < -180.0 {
        from -= 360.0;
    }
    while to - from >= 180.0 {
        from += 360.0;
    }
    from + 0.2 * (to - from)
}

/// A potion's box at `position`.
pub fn bounds_at(position: DVec3) -> (DVec3, DVec3) {
    let half = SIZE / 2.0;
    (DVec3::new(position.x - half, position.y, position.z - half), DVec3::new(position.x + half, position.y + SIZE, position.z + half))
}

/// `AABB.expandTowards`.
fn sweep(min: DVec3, max: DVec3, by: DVec3) -> (DVec3, DVec3) {
    let mut min = min;
    let mut max = max;
    for axis in 0..3 {
        if by[axis] < 0.0 {
            min[axis] += by[axis];
        } else if by[axis] > 0.0 {
            max[axis] += by[axis];
        }
    }
    (min, max)
}

/// `ThrownSplashPotion.onHitAsPotion` for one living thing whose box is
/// (`min`, `max`): the share of the potion it takes (1 at the potion's
/// box, nothing 4 blocks off), when it is near enough. The search box
/// (`inflate(4, 2, 4)`) must meet its box.
pub fn splash_scale(potion_at: DVec3, margin: f64, min: DVec3, max: DVec3) -> Option<f64> {
    let (pmin, pmax) = bounds_at(potion_at);
    let (smin, smax) = (DVec3::new(pmin.x - 4.0, pmin.y - 2.0, pmin.z - 4.0), DVec3::new(pmax.x + 4.0, pmax.y + 2.0, pmax.z + 4.0));
    if !(smin.x < max.x && smax.x > min.x && smin.y < max.y && smax.y > min.y && smin.z < max.z && smax.z > min.z) {
        return None;
    }
    let (min, max) = (min - DVec3::splat(margin), max + DVec3::splat(margin));
    // `AABB.distanceToSqr(AABB)`.
    let dx = (pmin.x - max.x).max(min.x - pmax.x).max(0.0);
    let dy = (pmin.y - max.y).max(min.y - pmax.y).max(0.0);
    let dz = (pmin.z - max.z).max(min.z - pmax.z).max(0.0);
    let distance = dx * dx + dy * dy + dz * dz;
    (distance < 16.0).then(|| 1.0 - distance.sqrt() / 4.0)
}

/// The effect a splash leaves at `scale` for a lasting effect: its time
/// scaled and rounded (`mapDuration`), none when it would end within a
/// second (`endsWithin(20)`).
pub fn splashed_effect(effect: &EffectInstance, scale: f64) -> Option<EffectInstance> {
    let duration = effect.map_duration(|d| (scale * f64::from(d) * 1.0 + 0.5) as i32);
    let splashed = EffectInstance { duration, hidden: None, ..effect.clone() };
    (!splashed.ends_within(20)).then_some(splashed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearer_takes_more_and_far_takes_nothing() {
        let at = DVec3::new(0.0, 0.0, 0.0);
        // A mob 1.5 blocks off its box.
        let scale = splash_scale(at, 0.0, DVec3::new(1.625, 0.0, -0.3), DVec3::new(2.225, 1.8, 0.3)).unwrap();
        assert_eq!(scale, 1.0 - 1.5 / 4.0);
        assert!(splash_scale(at, 0.0, DVec3::new(5.0, 0.0, -0.3), DVec3::new(5.6, 1.8, 0.3)).is_none());
        let poison = splashed_effect(&Potion::Poison.effect(), 0.625).unwrap();
        assert_eq!(poison.duration, 563);
        assert!(splashed_effect(&Potion::Poison.effect(), 0.01).is_none());
    }
}
