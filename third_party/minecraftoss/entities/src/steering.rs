//! ItemBasedSteering state shared by saddle-controlled mobs in pinned 26.3.
use minecraftoss_player::rng::LegacyRandom;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SteeringState {
    pub boosting: bool,
    pub boost_time: i32,
    pub boost_time_total: i32,
}

impl SteeringState {
    /// Pig.boost delegates directly here, without a saddle or rider check.
    pub fn boost(&mut self, random: &mut LegacyRandom) -> bool {
        if self.boosting {
            return false;
        }
        self.boosting = true;
        self.boost_time = 0;
        self.boost_time_total = random.next_int(841) as i32 + 140;
        true
    }

    /// Called by Pig.tickRidden, not by its ordinary AI tick.
    pub fn tick_boost(&mut self) {
        if self.boosting {
            let previous = self.boost_time;
            self.boost_time += 1;
            if previous > self.boost_time_total {
                self.boosting = false;
            }
        }
    }
}
