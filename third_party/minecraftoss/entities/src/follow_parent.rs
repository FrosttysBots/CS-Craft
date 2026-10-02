//! Pinned 26.3 FollowParentGoal state for the cow family.
use glam::DVec3;

#[derive(Clone, Copy, Debug)]
pub struct CowCandidate {
    pub id: u64,
    pub position: DVec3,
    pub width: f32,
    pub height: f32,
    pub age: i32,
    pub alive: bool,
    pub in_love: i32,
    pub panicking: bool,
}

#[derive(Clone, Debug, Default)]
pub struct FollowParentState {
    pub running: bool,
    pub parent_id: Option<u64>,
    pub time_to_recalc_path: i32,
}

impl FollowParentState {
    pub fn can_use(&mut self, child: CowCandidate, candidates: &[CowCandidate]) -> bool {
        if child.age >= 0 {
            return false;
        }
        let mut closest = None;
        let mut closest_distance = f64::MAX;
        for candidate in candidates {
            if candidate.id == child.id || candidate.age < 0 || !inside_scan(child, *candidate) {
                continue;
            }
            let distance = child.position.distance_squared(candidate.position);
            if distance <= closest_distance {
                closest_distance = distance;
                closest = Some(candidate.id);
            }
        }
        if closest_distance < 9.0 {
            return false;
        }
        self.parent_id = closest;
        closest.is_some()
    }

    pub fn can_continue(&self, child: CowCandidate, candidates: &[CowCandidate]) -> bool {
        if child.age >= 0 {
            return false;
        }
        let Some(parent) = candidates
            .iter()
            .find(|candidate| Some(candidate.id) == self.parent_id && candidate.alive)
        else {
            return false;
        };
        let distance = child.position.distance_squared(parent.position);
        (9.0..=256.0).contains(&distance)
    }

    pub fn start(&mut self) {
        self.running = true;
        self.time_to_recalc_path = 0;
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.parent_id = None;
    }

    pub fn tick(&mut self, candidates: &[CowCandidate]) -> Option<DVec3> {
        self.time_to_recalc_path -= 1;
        if self.time_to_recalc_path > 0 {
            return None;
        }
        // Goal.adjustedTickDelay(10) halves for non-every-tick goals.
        self.time_to_recalc_path = 5;
        candidates
            .iter()
            .find(|candidate| Some(candidate.id) == self.parent_id)
            .map(|parent| parent.position)
    }
}

fn inside_scan(child: CowCandidate, candidate: CowCandidate) -> bool {
    let c = child.position;
    let p = candidate.position;
    let horizontal = 8.0 + f64::from(child.width + candidate.width) * 0.5;
    p.x >= c.x - horizontal
        && p.x <= c.x + horizontal
        && p.z >= c.z - horizontal
        && p.z <= c.z + horizontal
        && p.y + f64::from(candidate.height) >= c.y - 4.0
        && p.y <= c.y + f64::from(child.height) + 4.0
}
