//! The iron golem (pinned 26.3 `IronGolem`, `AbstractGolem`): 100 health,
//! a quarter block a tick, 15 attack, a one-block step, immune to
//! knockback (`KNOCKBACK_RESISTANCE` 1), to falls (`#fall_damage_immune`)
//! and to drowning (`decreaseAirSupply` keeps its air). It cracks as it
//! loses health (`Crackiness.GOLEM`), holds out a poppy now and then
//! (`offerFlowerTick`), swings its arms when it hits (`attackAnimationTick`),
//! and as a `NeutralMob` stays angry at whoever it targets.
use crate::effects::MobEffects;
use crate::health::DamageState;
use crate::movement::Body;
use glam::DVec3;

/// `IronGolem.createAttributes`.
pub const MAX_HEALTH: f32 = 100.0;
pub const MOVEMENT_SPEED: f64 = 0.25;
pub const ATTACK_DAMAGE: f32 = 15.0;
pub const STEP_HEIGHT: f32 = 1.0;
/// `EntityTypes.IRON_GOLEM`: `sized(1.4F, 2.7F)`, the default eyes.
pub const WIDTH: f32 = 1.4;
pub const HEIGHT: f32 = 2.7;
pub const EYE_HEIGHT: f32 = HEIGHT * 0.85;
/// `AbstractGolem.getAmbientSoundInterval` (it has no ambient sound).
pub const AMBIENT_INTERVAL: i32 = 120;
/// `OfferFlowerGoal.OFFER_TICKS`.
pub const OFFER_TICKS: i32 = 400;
/// `IronGolem.IRON_INGOT_HEAL_AMOUNT`.
pub const INGOT_HEAL: f32 = 25.0;

/// `Crackiness.Level` by `Crackiness.GOLEM`'s fractions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Crackiness {
    None,
    Low,
    Medium,
    High,
}

impl Crackiness {
    /// `Crackiness.GOLEM.byFraction(health / maxHealth)`.
    pub fn of(health: f32) -> Self {
        let fraction = health / MAX_HEALTH;
        if fraction < 0.25 {
            Self::High
        } else if fraction < 0.5 {
            Self::Medium
        } else if fraction < 0.75 {
            Self::Low
        } else {
            Self::None
        }
    }

    /// The crack overlay's texture (`IronGolemCrackinessLayer`), none when
    /// whole.
    pub fn texture(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Low => Some("minecraft:entity/iron_golem/iron_golem_crackiness_low"),
            Self::Medium => Some("minecraft:entity/iron_golem/iron_golem_crackiness_medium"),
            Self::High => Some("minecraft:entity/iron_golem/iron_golem_crackiness_high"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct IronGolem {
    pub body: Body,
    pub health: f32,
    pub damage: DamageState,
    pub yaw: f32,
    pub persistence_required: bool,
    /// `PlayerCreated`: built by a player, it never attacks players.
    pub player_created: bool,
    pub effects: MobEffects,
    /// `attackAnimationTick`: ten ticks of swinging arms after a hit.
    pub attack_animation_tick: i32,
}

impl IronGolem {
    pub fn new(position: DVec3) -> Self {
        let mut body = Body::new(position, WIDTH, HEIGHT);
        body.step_height = STEP_HEIGHT;
        Self {
            body,
            health: MAX_HEALTH,
            damage: DamageState::default(),
            yaw: 0.0,
            persistence_required: false,
            player_created: false,
            effects: MobEffects::default(),
            attack_animation_tick: 0,
        }
    }

    pub fn crackiness(&self) -> Crackiness {
        Crackiness::of(self.health)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cracks_by_quarters_of_health() {
        assert_eq!(Crackiness::of(100.0), Crackiness::None);
        assert_eq!(Crackiness::of(75.0), Crackiness::None);
        assert_eq!(Crackiness::of(74.9), Crackiness::Low);
        assert_eq!(Crackiness::of(50.0), Crackiness::Low);
        assert_eq!(Crackiness::of(49.0), Crackiness::Medium);
        assert_eq!(Crackiness::of(24.0), Crackiness::High);
    }
}
