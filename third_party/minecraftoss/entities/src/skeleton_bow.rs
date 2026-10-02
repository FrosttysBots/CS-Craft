//! The pinned 26.3 `RangedBowAttackGoal` state machine. It accepts visibility
//! and distance from the owning mob, leaving sensing and pathing to the world.
//! Provenance: RangedBowAttackGoal.canUse/start/stop/tick and BowItem power.

use minecraftoss_player::rng::LegacyRandom;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BowMovement {
    Navigate { speed: f64 },
    Strafe { forward: f32, sideways: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BowTick {
    pub movement: BowMovement,
    pub shoot_power: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct SkeletonBowGoal {
    pub running: bool,
    pub aggressive: bool,
    pub using_item: bool,
    pub ticks_using_item: i32,
    pub attack_time: i32,
    pub see_time: i32,
    pub strafing_time: i32,
    pub strafing_clockwise: bool,
    pub strafing_backwards: bool,
    pub attack_interval: i32,
}

impl SkeletonBowGoal {
    pub fn normal() -> Self {
        Self::new(40)
    }
    pub fn hard() -> Self {
        Self::new(20)
    }

    pub fn new(attack_interval: i32) -> Self {
        Self {
            running: false,
            aggressive: false,
            using_item: false,
            ticks_using_item: 0,
            attack_time: -1,
            see_time: 0,
            strafing_time: -1,
            strafing_clockwise: false,
            strafing_backwards: false,
            attack_interval,
        }
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.aggressive = false;
        self.see_time = 0;
        self.attack_time = -1;
        self.using_item = false;
        self.ticks_using_item = 0;
    }

    /// Advance once at the goal's full tick rate. The item's use time is
    /// advanced by LivingEntity earlier in the same server tick.
    pub fn tick(
        &mut self,
        has_target: bool,
        holding_bow: bool,
        navigation_done: bool,
        distance_squared: f64,
        has_line_of_sight: bool,
        random: &mut LegacyRandom,
    ) -> Option<BowTick> {
        if self.using_item {
            self.ticks_using_item += 1;
        }
        if !has_target || !holding_bow {
            if self.running && (!holding_bow || navigation_done) {
                self.stop();
            }
            return None;
        }
        if !self.running {
            self.running = true;
            self.aggressive = true;
        }
        if has_line_of_sight != (self.see_time > 0) {
            self.see_time = 0;
        }
        self.see_time += if has_line_of_sight { 1 } else { -1 };

        let movement = if distance_squared > 225.0 || self.see_time < 20 {
            self.strafing_time = -1;
            BowMovement::Navigate { speed: 1.0 }
        } else {
            self.strafing_time += 1;
            if self.strafing_time >= 20 {
                if random.next_float() < 0.3 {
                    self.strafing_clockwise = !self.strafing_clockwise;
                }
                if random.next_float() < 0.3 {
                    self.strafing_backwards = !self.strafing_backwards;
                }
                self.strafing_time = 0;
            }
            if distance_squared > 225.0 * 0.75_f32 as f64 {
                self.strafing_backwards = false;
            } else if distance_squared < 225.0 * 0.25_f32 as f64 {
                self.strafing_backwards = true;
            }
            BowMovement::Strafe {
                forward: if self.strafing_backwards { -0.5 } else { 0.5 },
                sideways: if self.strafing_clockwise { 0.5 } else { -0.5 },
            }
        };
        let mut shoot_power = None;
        if self.using_item {
            if !has_line_of_sight && self.see_time < -60 {
                self.using_item = false;
                self.ticks_using_item = 0;
            } else if has_line_of_sight && self.ticks_using_item >= 20 {
                let f = self.ticks_using_item as f32 / 20.0;
                shoot_power = Some(((f * f + f * 2.0) / 3.0).min(1.0));
                self.using_item = false;
                self.ticks_using_item = 0;
                self.attack_time = self.attack_interval;
            }
        } else {
            self.attack_time -= 1;
            if self.attack_time <= 0 && self.see_time >= -60 {
                self.using_item = true;
                self.ticks_using_item = 0;
            }
        }
        Some(BowTick {
            movement,
            shoot_power,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_skeleton_bow_draw_and_strafe_timing() {
        // 26.3 scenarios/mobs/skeleton-player-bow.json, ticks 1–27.
        let mut goal = SkeletonBowGoal::normal();
        let mut random = LegacyRandom::new(59);
        for tick in 1..=27 {
            let result = goal
                .tick(true, true, false, 100.0, true, &mut random)
                .unwrap();
            assert_eq!(goal.using_item, tick < 21);
            assert_eq!(goal.ticks_using_item, if tick < 21 { tick - 1 } else { 0 });
            assert_eq!(
                result.shoot_power,
                if tick == 21 { Some(1.0) } else { None }
            );
            assert_eq!(
                matches!(result.movement, BowMovement::Strafe { .. }),
                tick >= 20
            );
            if tick < 20 {
                assert_eq!(result.movement, BowMovement::Navigate { speed: 1.0 });
            }
        }
        assert_eq!(goal.attack_time, 34);
    }
}
