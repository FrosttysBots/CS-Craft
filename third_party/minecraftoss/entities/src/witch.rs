//! The witch (pinned 26.3 `Witch`, with `Raider` and `PatrollingMonster`):
//! 26 health, a quarter block a tick, and potions. Each tick it may start
//! drinking one (water breathing with its eyes under water, fire
//! resistance while burning, healing when hurt, swiftness far from its
//! target), standing still while it drinks (`drinking`: speed -0.25) and
//! taking the potion's effect 33 ticks later; its ranged attack throws a
//! splash potion chosen for the target (slowness at range, then poison,
//! then close in now and then weakness, else harming). It shrugs off most
//! magic (`#witch_resistant_to` damage takes 15%) and all of its own.
use crate::effects::{MobEffect, MobEffects};
use crate::health::DamageState;
use crate::movement::Body;
use crate::potion::Potion;
use glam::DVec3;
use minecraftoss_player::rng::LegacyRandom;

/// `Witch.createAttributes`: `MAX_HEALTH`.
pub const MAX_HEALTH: f32 = 26.0;
/// `Witch.createAttributes`: `MOVEMENT_SPEED`.
pub const MOVEMENT_SPEED: f64 = 0.25;
/// `SPEED_MODIFIER_DRINKING` (`ADD_VALUE`).
pub const DRINKING_SPEED: f64 = -0.25;
/// `EntityTypes.WITCH`.
pub const WIDTH: f32 = 0.6;
pub const HEIGHT: f32 = 1.95;
pub const EYE_HEIGHT: f32 = 1.62;
/// A potion's use duration (`Consumables.DEFAULT_DRINK`, 1.6 seconds:
/// `(int)(1.6F * 20.0F)`).
pub const DRINK_TICKS: i32 = 32;
/// `RangedAttackGoal(this, 1.0, 60, 10.0F)`.
pub const ATTACK_INTERVAL: i32 = 60;
pub const ATTACK_RADIUS: f32 = 10.0;
/// `rangedAttackUncertainty`.
pub const UNCERTAINTY: f32 = 8.0;

#[derive(Clone, Debug)]
pub struct Witch {
    pub body: Body,
    pub health: f32,
    pub damage: DamageState,
    pub yaw: f32,
    pub persistence_required: bool,
    pub effects: MobEffects,
    /// The potion in its main hand while it drinks (`isDrinkingPotion`).
    pub drinking: Option<Potion>,
    /// `usingTime`: ticks of drinking left.
    pub using_time: i32,
    /// `healRaidersGoal`'s cooldown: counted down every tick, it only
    /// rises in a raid.
    pub heal_cooldown: i32,
    /// When fire last hurt it (`getLastDamageSource` within 40 ticks).
    pub last_fire_damage: Option<i64>,
}

impl Witch {
    pub fn new(position: DVec3) -> Self {
        Self {
            body: Body::new(position, WIDTH, HEIGHT),
            health: MAX_HEALTH,
            damage: DamageState::default(),
            yaw: 0.0,
            persistence_required: false,
            effects: MobEffects::default(),
            drinking: None,
            using_time: 0,
            heal_cooldown: 0,
            last_fire_damage: None,
        }
    }

    /// `MOVEMENT_SPEED` with the drinking modifier and its effects.
    pub fn movement_speed(&self) -> f64 {
        let value = MOVEMENT_SPEED + if self.drinking.is_some() { DRINKING_SPEED } else { 0.0 };
        self.effects.movement_speed(value)
    }

    /// `Witch.aiStep`'s choice of a potion to drink when not drinking: a
    /// float for each wish in turn until one holds. `target_distance_sq`
    /// is its target's squared distance, if it has one.
    #[allow(clippy::too_many_arguments)]
    pub fn choose_drink(&self, random: &mut LegacyRandom, eye_in_water: bool, game_time: i64, target_distance_sq: Option<f64>) -> Option<Potion> {
        if random.next_float() < 0.15 && eye_in_water && !self.effects.has(MobEffect::WaterBreathing) {
            return Some(Potion::WaterBreathing);
        }
        let burnt = self.body.fire_ticks > 0 || self.last_fire_damage.is_some_and(|at| game_time - at <= 40);
        if random.next_float() < 0.15 && burnt && !self.effects.has(MobEffect::FireResistance) {
            return Some(Potion::FireResistance);
        }
        if random.next_float() < 0.05 && self.health < MAX_HEALTH {
            return Some(Potion::Healing);
        }
        if random.next_float() < 0.5 && target_distance_sq.is_some_and(|d| d > 121.0) && !self.effects.has(MobEffect::Speed) {
            return Some(Potion::Swiftness);
        }
        None
    }
}

/// What the witch knows of its target when it throws.
#[derive(Clone, Copy, Debug)]
pub struct ThrowTarget {
    pub position: DVec3,
    pub eye_height: f32,
    pub velocity: DVec3,
    pub health: f32,
    pub slowed: bool,
    pub poisoned: bool,
    pub weakened: bool,
}

/// A potion `performRangedAttack` throws: which, along what line and how
/// hard.
#[derive(Clone, Copy, Debug)]
pub struct Throw {
    pub potion: Potion,
    pub direction: DVec3,
    pub power: f32,
}

/// `Witch.performRangedAttack` at a target that is not a raider: aimed
/// where the target will be next tick, 1.1 under its eyes, raised a fifth
/// of the distance; slowness beyond 8 blocks, poison while the target has
/// 8 health, weakness a quarter of the time within 3, else harming (each
/// only when the target lacks it). A float from its random decides
/// weakness.
pub fn aim(position: DVec3, target: ThrowTarget, random: &mut LegacyRandom) -> Throw {
    let xd = target.position.x + target.velocity.x - position.x;
    let yd = target.position.y + f64::from(target.eye_height) - f64::from(1.1_f32) - position.y;
    let zd = target.position.z + target.velocity.z - position.z;
    let dist = (xd * xd + zd * zd).sqrt();
    let potion = if dist >= 8.0 && !target.slowed {
        Potion::Slowness
    } else if target.health >= 8.0 && !target.poisoned {
        Potion::Poison
    } else if dist <= 3.0 && !target.weakened && random.next_float() < 0.25 {
        Potion::Weakness
    } else {
        Potion::Harming
    };
    Throw { potion, direction: DVec3::new(xd, yd + dist * 0.2, zd), power: if dist <= 2.0 { 0.45 } else { 0.75 } }
}

/// `Witch.getDamageAfterMagicAbsorb`: its own potions do nothing to it,
/// and it takes 15% of `#witch_resistant_to` damage (magic).
pub fn absorb(damage: f32, own: bool, resisted: bool) -> f32 {
    let damage = if own { 0.0 } else { damage };
    if resisted {
        damage * 0.15
    } else {
        damage
    }
}
