//! Pinned Java 26.3 skeleton base state. Sources: EntityTypes.SKELETON,
//! AbstractSkeleton.createAttributes, and inherited Monster lifecycle.
use crate::{health::DamageState, movement::Body};
use glam::DVec3;
use minecraftoss_player::survival::EffectKind;

/// The skeletons the entity world runs (`AbstractSkeleton`'s subclasses).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SkeletonKind {
    #[default]
    Skeleton,
    /// `Stray`: its arrows slow.
    Stray,
    /// `Bogged`: its arrows poison; it can be sheared of its mushrooms.
    Bogged,
    /// `Parched`: its arrows weaken.
    Parched,
}

impl SkeletonKind {
    /// The entity type's ID.
    pub fn type_id(self) -> &'static str {
        match self {
            Self::Skeleton => "minecraft:skeleton",
            Self::Stray => "minecraft:stray",
            Self::Bogged => "minecraft:bogged",
            Self::Parched => "minecraft:parched",
        }
    }

    /// Its sounds' family (`entity.<family>.ambient`).
    pub fn sound_family(self) -> &'static str {
        match self {
            Self::Skeleton => "skeleton",
            Self::Stray => "stray",
            Self::Bogged => "bogged",
            Self::Parched => "parched",
        }
    }

    /// `#minecraft:burn_in_daylight`: all but the parched.
    pub fn burns_in_daylight(self) -> bool {
        self != Self::Parched
    }

    /// `createAttributes`: bogged and parched have 16 health.
    pub fn max_health(self) -> f32 {
        match self {
            Self::Bogged | Self::Parched => 16.0,
            Self::Skeleton | Self::Stray => 20.0,
        }
    }

    /// `getAttackInterval`, or `getHardAttackInterval` on hard.
    pub fn attack_interval(self, hard: bool) -> i32 {
        match (self, hard) {
            (Self::Bogged | Self::Parched, false) => 70,
            (Self::Bogged | Self::Parched, true) => 50,
            (_, false) => 40,
            (_, true) => 20,
        }
    }

    /// `getArrow`'s effect on the arrows it looses (its full duration: a
    /// plain arrow's `POTION_DURATION_SCALE` is 1).
    pub fn arrow_effect(self) -> Option<(EffectKind, u32)> {
        match self {
            Self::Skeleton => None,
            Self::Stray => Some((EffectKind::Slowness, 600)),
            Self::Bogged => Some((EffectKind::Poison, 100)),
            Self::Parched => Some((EffectKind::Weakness, 600)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Skeleton {
    pub kind: SkeletonKind,
    /// A bogged's `sheared` flag (its mushrooms gone).
    pub sheared: bool,
    pub body: Body,
    pub health: f32,
    pub damage: DamageState,
    pub persistence_required: bool,
    /// `Entity.remainingFireTicks`.
    /// An item in the head slot (`sunProtectionSlot`), and whether it can
    /// be damaged.
    pub head_item: Option<bool>,
    /// Its bow in the main hand (`populateDefaultEquipmentSlots`).
    pub holds_bow: bool,
    /// The head, chest, legs and feet slots' item IDs.
    pub armor: [Option<String>; 4],
    /// The main hand's drop chance (`Mob.dropChances`); above 1 the bow
    /// drops whoever killed it, unworn (`isPreserved`).
    pub bow_drop_chance: f32,
}

impl Skeleton {
    pub fn new(position: DVec3) -> Self {
        Self {
            kind: SkeletonKind::Skeleton,
            sheared: false,
            body: Body::new(position, 0.6, 1.99),
            health: 20.0,
            damage: DamageState::default(),
            persistence_required: false,
            head_item: None,
            holds_bow: true,
            armor: Default::default(),
            bow_drop_chance: 0.085,
        }
    }

    /// A skeleton of `kind` at full health.
    pub fn of_kind(kind: SkeletonKind, position: DVec3) -> Self {
        let mut skeleton = Self::new(position);
        skeleton.kind = kind;
        skeleton.health = kind.max_health();
        skeleton
    }

    pub fn eye_height(&self) -> f32 {
        1.74
    }
}
