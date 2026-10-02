//! Pinned 26.3 BreedGoal pursuit and breeding trigger for the cow family.
//! Child creation and parental state are applied by EntityWorld in tick order.
use crate::follow_parent::CowCandidate;
use glam::DVec3;

#[derive(Clone, Debug, Default)]
pub struct BreedState {
    pub running: bool,
    pub partner_id: Option<u64>,
    pub love_time: i32,
}

impl BreedState {
    pub fn can_use(&mut self, cow: CowCandidate, candidates: &[CowCandidate]) -> bool {
        if cow.in_love <= 0 {
            return false;
        }
        let mut closest = None;
        let mut closest_distance = f64::MAX;
        for candidate in candidates {
            if candidate.id == cow.id
                || !candidate.alive
                || candidate.in_love <= 0
                || candidate.panicking
                || !inside_scan(cow, *candidate)
            {
                continue;
            }
            let distance = cow.position.distance_squared(candidate.position);
            if distance < closest_distance && distance <= 64.0 {
                closest_distance = distance;
                closest = Some(candidate.id);
            }
        }
        self.partner_id = closest;
        closest.is_some()
    }

    pub fn can_continue(&self, candidates: &[CowCandidate]) -> bool {
        self.love_time < 60
            && candidates.iter().any(|candidate| {
                Some(candidate.id) == self.partner_id
                    && candidate.alive
                    && candidate.in_love > 0
                    && !candidate.panicking
            })
    }

    pub fn start(&mut self) {
        self.running = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.partner_id = None;
        self.love_time = 0;
    }

    /// Returns the navigation target and any partner whose breeding threshold was met.
    pub fn tick(
        &mut self,
        cow: CowCandidate,
        candidates: &[CowCandidate],
    ) -> Option<(DVec3, Option<u64>)> {
        let partner = candidates
            .iter()
            .find(|candidate| Some(candidate.id) == self.partner_id)?;
        self.love_time += 1;
        let breed = if self.love_time >= 30 && cow.position.distance_squared(partner.position) < 9.0
        {
            self.partner_id
        } else {
            None
        };
        Some((partner.position, breed))
    }
}

fn inside_scan(cow: CowCandidate, candidate: CowCandidate) -> bool {
    let c = cow.position;
    let p = candidate.position;
    let horizontal = 8.0 + f64::from(cow.width + candidate.width) * 0.5;
    p.x >= c.x - horizontal
        && p.x <= c.x + horizontal
        && p.z >= c.z - horizontal
        && p.z <= c.z + horizontal
        && p.y + f64::from(candidate.height) >= c.y - 8.0
        && p.y <= c.y + f64::from(cow.height) + 8.0
}
