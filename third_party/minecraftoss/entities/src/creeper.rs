//! Pinned 26.3 creeper fuse and base body state.
//! Sources: EntityTypes.CREEPER and Creeper.tick/ignite/explodeCreeper.
use crate::{health::DamageState, movement::Body};
use glam::DVec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CreeperExplosion {
    pub position: DVec3,
    pub radius: f32,
    pub powered: bool,
}

#[derive(Clone, Debug)]
pub struct Creeper {
    pub body: Body,
    pub health: f32,
    pub damage: DamageState,
    pub persistence_required: bool,
    pub old_swell: i32,
    pub swell: i32,
    pub max_swell: i32,
    pub explosion_radius: i32,
    pub swell_dir: i32,
    pub ignited: bool,
    pub powered: bool,
    pub exploded: bool,
    /// Facing when spawned (the AI turns it from then on).
    pub yaw: f32,
}

impl Creeper {
    pub fn new(position: DVec3) -> Self {
        Self {
            body: Body::new(position, 0.6, 1.7),
            health: 20.0,
            damage: DamageState::default(),
            persistence_required: false,
            old_swell: 0,
            swell: 0,
            max_swell: 30,
            explosion_radius: 3,
            swell_dir: -1,
            ignited: false,
            powered: false,
            exploded: false,
            yaw: 0.0,
        }
    }

    /// Creeper.tick's fuse branch runs even while Mob has NoAI. Returns the
    /// explosion request that the authoritative world must consume.
    pub fn tick_fuse(&mut self) -> Option<CreeperExplosion> {
        if self.health <= 0.0 || self.exploded {
            return None;
        }
        self.old_swell = self.swell;
        if self.ignited {
            self.swell_dir = 1;
        }
        self.swell = (self.swell + self.swell_dir).max(0);
        if self.swell >= self.max_swell {
            self.swell = self.max_swell;
            self.exploded = true;
            return Some(CreeperExplosion {
                position: self.body.position,
                radius: self.explosion_radius as f32 * if self.powered { 2.0 } else { 1.0 },
                powered: self.powered,
            });
        }
        None
    }
}
