//! AgeableMob's server aging and feeding state. Particle requests are returned
//! in source order so the entity integration can consume its own random stream.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Age {
    pub ticks: i32,
    pub forced: i32,
    pub locked: bool,
    pub forced_particle_ticks: i32,
    pub lock_particle_ticks: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgeParticle {
    Happy,
    PauseGrowth,
    ResetGrowth,
}

impl Age {
    pub const BABY_START: i32 = -24000;

    pub fn baby(&self) -> bool {
        self.ticks < 0
    }

    pub fn can_grow(&self) -> bool {
        self.baby() && !self.locked
    }

    /// Returns whether the baby/adult boundary changed. Integrators must refresh
    /// dimensions and check passenger capacity at that boundary.
    pub fn set(&mut self, ticks: i32) -> bool {
        let was_baby = self.baby();
        self.ticks = ticks;
        was_baby != self.baby()
    }

    pub fn grow(&mut self, seconds: i32, forced: bool) {
        let old = self.ticks;
        self.ticks = old.wrapping_add(seconds.wrapping_mul(20)).min(0);
        if forced {
            self.forced = self.forced.wrapping_add(self.ticks.wrapping_sub(old));
            if self.forced_particle_ticks == 0 {
                self.forced_particle_ticks = 40;
            }
        }
        if self.ticks == 0 {
            self.ticks = self.forced;
        }
    }

    /// Caller consumes the food only when this returns true. A feed near
    /// adulthood can consume food while advancing zero ticks (integer rounding).
    pub fn feed(&mut self) -> bool {
        if !self.can_grow() {
            return false;
        }
        let seconds = ((self.ticks.wrapping_neg() / 20) as f32 * 0.1_f32) as i32;
        self.grow(seconds, true);
        true
    }

    /// Caller checks the item and CANNOT_BE_AGE_LOCKED tag, consumes it, emits
    /// the sound and sets persistence when the resulting locked state is true.
    pub fn toggle_lock(&mut self, baby_start: i32) -> bool {
        if !self.baby() || self.lock_particle_ticks != 0 {
            return false;
        }
        self.locked = !self.locked;
        self.ticks = baby_start;
        self.lock_particle_ticks = 40;
        true
    }

    /// One server aiStep after the shared living/mob step; no independent clock.
    pub fn tick(&mut self, alive: bool) -> Vec<AgeParticle> {
        let mut particles = Vec::new();
        if self.forced_particle_ticks > 0 {
            if self.forced_particle_ticks % 4 == 0 {
                particles.push(AgeParticle::Happy);
            }
            self.forced_particle_ticks -= 1;
        }
        if alive {
            if self.can_grow() {
                self.ticks += 1;
            } else if self.ticks > 0 {
                self.ticks -= 1;
            }
        }
        if self.lock_particle_ticks > 0 {
            if self.lock_particle_ticks % 2 == 0 {
                particles.push(if self.locked {
                    AgeParticle::PauseGrowth
                } else {
                    AgeParticle::ResetGrowth
                });
            }
            self.lock_particle_ticks -= 1;
        }
        particles
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn food_rounding_and_forced_growth_cooldown() {
        let mut age = Age {
            ticks: -199,
            ..Age::default()
        };
        assert!(age.feed());
        assert_eq!(age.ticks, -199);
        age.ticks = -200;
        age.feed();
        assert_eq!((age.ticks, age.forced), (-180, 20));
        age.grow(9, true);
        assert_eq!((age.ticks, age.forced), (200, 200));
        age.tick(true);
        assert_eq!(age.ticks, 199);
    }

    #[test]
    fn locking_resets_age_and_has_a_forty_tick_cooldown() {
        let mut age = Age {
            ticks: -12,
            ..Age::default()
        };
        assert!(age.toggle_lock(Age::BABY_START));
        assert!(!age.feed());
        assert!(!age.toggle_lock(Age::BABY_START));
        let events: Vec<_> = (0..40).flat_map(|_| age.tick(true)).collect();
        assert_eq!(age.ticks, -24000);
        assert_eq!(events, vec![AgeParticle::PauseGrowth; 20]);
        assert!(age.toggle_lock(Age::BABY_START));
        age.tick(true);
        assert_eq!(age.ticks, -23999);
    }

    #[test]
    fn natural_growth_does_not_apply_forced_age_and_dead_mobs_do_not_grow() {
        let mut age = Age {
            ticks: -1,
            forced: 20,
            ..Age::default()
        };
        age.tick(false);
        assert_eq!(age.ticks, -1);
        age.tick(true);
        assert_eq!(age.ticks, 0);
    }
}
