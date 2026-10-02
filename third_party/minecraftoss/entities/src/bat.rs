//! Source-informed pinned 26.3 Bat state and post-living tick path.
//! Source: Bat constructor, tick, isFlapping, createAttributes and BatFlags.
use crate::{health::DamageState, movement::Body};
use glam::DVec3;
use minecraftoss_player::rng::LegacyRandom;

/// Caller-resolved world and tag facts used by Bat.checkBatSpawnRules.
#[derive(Clone, Copy, Debug)]
pub struct BatSpawnContext {
    pub y: i32,
    pub world_surface_y: i32,
    pub max_local_raw_brightness: i32,
    pub below_bats_spawnable_on: bool,
    pub below_valid_spawn: bool,
}

#[derive(Clone, Debug)]
pub struct Bat {
    pub body: Body,
    pub health: f32,
    pub damage: DamageState,
    pub resting: bool,
    pub persistence_required: bool,
}

impl Bat {
    /// Source: Bat.checkBatSpawnRules, Mob.checkMobSpawnRules (NATURAL).
    /// Preserve short-circuit order and the two conditional RNG draws.
    pub fn can_naturally_spawn(context: BatSpawnContext, random: &mut LegacyRandom) -> bool {
        if context.y >= context.world_surface_y || random.next_boolean() {
            return false;
        }
        if context.max_local_raw_brightness > random.next_int(4) as i32 {
            return false;
        }
        context.below_bats_spawnable_on && context.below_valid_spawn
    }

    pub fn new(position: DVec3) -> Self {
        Self {
            body: Body::new(position, 0.5, 0.9),
            health: 6.0,
            damage: DamageState::default(),
            resting: true,
            persistence_required: false,
        }
    }

    pub fn is_flapping(&self, tick_count: i32) -> bool {
        !self.resting && (tick_count as f32) % 10.0 == 0.0
    }

    /// Bat.tick runs after the inherited living tick. Living aiStep first
    /// trims velocity components below 0.003 on the measured paths;
    /// resting snaps raw Y below its integer ceiling, while flight damps Y.
    pub fn tick_after_living(&mut self) {
        if self.resting {
            self.body.velocity = DVec3::ZERO;
            self.body.position.y = self.body.position.y.floor() + 1.0 - f64::from(self.body.height);
        } else {
            self.body.velocity.y *= 0.6;
        }
    }
}
