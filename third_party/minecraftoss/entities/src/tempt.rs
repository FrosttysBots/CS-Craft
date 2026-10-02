//! Pinned 26.3 TemptGoal target state with species-specific held-item predicates.
use crate::follow_parent::CowCandidate;
use glam::DVec3;

#[derive(Clone, Copy, Debug)]
pub struct PlayerCandidate {
    pub id: u64,
    pub position: DVec3,
    pub eye_height: f32,
    pub main_hand_cow_food: bool,
    pub offhand_cow_food: bool,
    pub main_hand_pig_food: bool,
    pub offhand_pig_food: bool,
    pub main_hand_chicken_food: bool,
    pub offhand_chicken_food: bool,
    pub main_hand_carrot_on_a_stick: bool,
    pub offhand_carrot_on_a_stick: bool,
    /// A bone or `#wolf_food` in either hand (`BegGoal.playerHoldingInteresting`).
    pub main_hand_wolf_interest: bool,
    pub offhand_wolf_interest: bool,
    /// `#horse_tempt_items` (golden carrots and apples) in either hand.
    pub main_hand_horse_tempt: bool,
    pub offhand_horse_tempt: bool,
    pub alive: bool,
    pub spectator: bool,
    pub attackable: bool,
}

#[derive(Clone, Debug, Default)]
pub struct TemptState {
    pub running: bool,
    pub player_id: Option<u64>,
    pub calm_down: i32,
}

impl TemptState {
    pub fn can_use(&mut self, cow: CowCandidate, players: &[PlayerCandidate]) -> bool {
        self.can_use_matching(cow, players, |player| {
            player.main_hand_cow_food || player.offhand_cow_food
        })
    }

    pub fn can_use_matching(
        &mut self,
        cow: CowCandidate,
        players: &[PlayerCandidate],
        accepts: impl Fn(&PlayerCandidate) -> bool,
    ) -> bool {
        if self.calm_down > 0 {
            self.calm_down -= 1;
            return false;
        }
        let mut nearest = None;
        let mut distance = 100.0; // Animal TEMPT_RANGE = 10.0.
        for player in players {
            if !player.alive || player.spectator || !accepts(player) {
                continue;
            }
            let candidate_distance = cow.position.distance_squared(player.position);
            if candidate_distance <= distance {
                distance = candidate_distance;
                nearest = Some(player.id);
            }
        }
        self.player_id = nearest;
        nearest.is_some()
    }

    pub fn start(&mut self) {
        self.running = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.player_id = None;
        self.calm_down = 50; // Goal.reducedTickDelay(100).
    }

    pub fn tick(&self, cow: CowCandidate, players: &[PlayerCandidate]) -> Option<TemptAction> {
        let player = players.iter().find(|p| Some(p.id) == self.player_id)?;
        if cow.position.distance_squared(player.position) < 6.25 {
            Some(TemptAction::StopNavigation)
        } else {
            Some(TemptAction::Navigate(player.position))
        }
    }
}

pub enum TemptAction {
    Navigate(DVec3),
    StopNavigation,
}
