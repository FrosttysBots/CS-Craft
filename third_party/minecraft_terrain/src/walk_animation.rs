//! The client's limb swing for walking mobs (26.3 `WalkAnimationState`,
//! `LivingEntity.calculateEntityAnimation`/`updateWalkAnimation`): each
//! tick a mob's horizontal movement sets a target speed, the speed eases
//! towards it and the swing position advances by it. Mobs arrive once a
//! tick as server snapshots, whose previous and current positions give the
//! movement.
use glam::DVec3;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default)]
pub struct WalkAnimation {
    speed_old: f32,
    speed: f32,
    position: f32,
    position_scale: f32,
}

impl WalkAnimation {
    /// `updateWalkAnimation`: a quarter block a tick is full speed; babies
    /// swing three times as fast.
    pub fn update(&mut self, distance: f32, baby: bool) {
        let target = (distance * 4.0).min(1.0);
        self.speed_old = self.speed;
        self.speed += (target - self.speed) * 0.4;
        self.position += self.speed;
        self.position_scale = if baby { 3.0 } else { 1.0 };
    }

    /// `WalkAnimationState.stop`.
    pub fn stop(&mut self) {
        self.speed_old = 0.0;
        self.speed = 0.0;
        self.position = 0.0;
    }

    /// `WalkAnimationState.speed()`: the current speed, unclamped.
    pub fn raw_speed(&self) -> f32 {
        self.speed
    }

    /// `WalkAnimationState.setSpeed`.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
    }

    /// `WalkAnimationState.position(partialTicks)`.
    pub fn position(&self, partial: f32) -> f32 {
        (self.position - self.speed * (1.0 - partial)) * self.position_scale
    }

    /// `WalkAnimationState.speed(partialTicks)`.
    pub fn speed(&self, partial: f32) -> f32 {
        (self.speed_old + partial * (self.speed - self.speed_old)).min(1.0)
    }
}

/// The swing of every tracked mob, by entity ID.
#[derive(Default)]
pub struct WalkAnimations(HashMap<u64, WalkAnimation>);

impl WalkAnimations {
    /// A new snapshot: each mob moved from `previous` to `current` this
    /// tick; mobs no longer tracked are forgotten.
    pub fn tick(&mut self, mobs: impl IntoIterator<Item = (u64, DVec3, DVec3, bool)>) {
        let mut next = HashMap::with_capacity(self.0.len());
        for (id, previous, current, baby) in mobs {
            let mut walk = self.0.get(&id).copied().unwrap_or_default();
            let (dx, dz) = (current.x - previous.x, current.z - previous.z);
            walk.update((dx * dx + dz * dz).sqrt() as f32, baby);
            next.insert(id, walk);
        }
        self.0 = next;
    }

    pub fn get(&self, id: u64) -> WalkAnimation {
        self.0.get(&id).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walking_winds_the_swing_up_and_standing_winds_it_down() {
        let mut walk = WalkAnimation::default();
        for _ in 0..20 {
            walk.update(0.2, false);
        }
        assert!((walk.speed(1.0) - 0.8).abs() < 1.0e-3);
        let moved = walk.position(1.0);
        for _ in 0..20 {
            walk.update(0.0, false);
        }
        assert!(walk.speed(1.0) < 1.0e-3);
        assert!(walk.position(1.0) > moved);
    }
}
