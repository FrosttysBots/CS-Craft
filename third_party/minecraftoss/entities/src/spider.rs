//! Pinned Java 26.3 spider base state. Sources: `EntityTypes.SPIDER` (1.4
//! wide, 0.9 tall, eyes at 0.65), `Spider.createAttributes` (16 health,
//! 0.3 speed; `Monster`'s 2 attack damage) and `Spider.tick`'s climbing
//! flag, kept on its body.
use crate::{health::DamageState, movement::Body};
use glam::DVec3;

/// `Spider.createAttributes`: `MAX_HEALTH`.
pub const MAX_HEALTH: f32 = 16.0;
/// `Monster.createMonsterAttributes`: `ATTACK_DAMAGE`.
pub const ATTACK_DAMAGE: f32 = 2.0;

/// `Spider.createAttributes`: `MOVEMENT_SPEED` (`0.3F`).
pub fn movement_speed() -> f64 {
    f64::from(0.3_f32)
}

#[derive(Clone, Debug)]
pub struct Spider {
    pub body: Body,
    pub health: f32,
    pub damage: DamageState,
    pub persistence_required: bool,
    pub yaw: f32,
}

impl Spider {
    pub fn new(position: DVec3) -> Self {
        Self {
            body: Body::new(position, 1.4, 0.9),
            health: MAX_HEALTH,
            damage: DamageState::default(),
            persistence_required: false,
            yaw: 0.0,
        }
    }

    pub fn eye_height(&self) -> f32 {
        0.65
    }
}
