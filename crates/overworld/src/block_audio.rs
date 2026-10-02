//! Local player action audio; independent of distant-world sound attenuation.
use crate::{BlockPos, Runtime};

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Step,
    Hit,
    Break,
    Place,
}
#[derive(Clone, Debug)]
pub struct Sound {
    pub event: String,
    pub volume: f32,
    pub pitch: f32,
}
impl Sound {
    pub fn for_block(id: &str, action: Action) -> Self {
        let key = if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        };
        let id = key.as_str();
        let profile = crate::terrain::audio::measured_block_sound_profile(id);
        let family = crate::terrain::audio::block_sound_family(id);
        let volume = profile.map_or(1., |p| p.volume);
        let pitch = profile.map_or(1., |p| p.pitch);
        let (suffix, volume, pitch) = match action {
            Action::Step => ("step", volume * 0.3, pitch),
            Action::Hit => ("hit", volume * 0.25, pitch * 0.5),
            Action::Break => ("break", volume, pitch * 0.8),
            Action::Place => ("place", (volume + 1.) * 0.5, pitch * 0.8),
        };
        Self {
            event: format!("minecraft:block.{family}.{suffix}"),
            volume,
            pitch,
        }
    }
}
#[derive(Default)]
pub struct Footsteps {
    last: Option<[f64; 3]>,
    distance: f64,
}
impl Footsteps {
    pub fn update(&mut self, feet: [f64; 3], grounded: bool) -> bool {
        let last = self.last.replace(feet);
        let Some(last) = last else {
            return false;
        };
        let d = ((feet[0] - last[0]).powi(2) + (feet[2] - last[2]).powi(2)).sqrt();
        if !grounded || d > 8. || (feet[1] - last[1]).abs() > 2. {
            self.distance = 0.;
            return false;
        }
        self.distance += d;
        if self.distance >= 1.65 {
            self.distance %= 1.65;
            true
        } else {
            false
        }
    }
}
#[derive(Default)]
pub struct MiningHits {
    target: Option<BlockPos>,
    ticks: u32,
}
impl MiningHits {
    /// Called at the same 20 Hz as native mining. A new target starts a new rhythm.
    pub fn tick(&mut self, target: Option<BlockPos>) -> bool {
        if target != self.target {
            self.target = target;
            self.ticks = 0;
        }
        if target.is_none() {
            self.ticks = 0;
            return false;
        }
        let hit = self.ticks % 4 == 0;
        self.ticks = self.ticks.wrapping_add(1);
        hit
    }
}
impl Runtime {
    pub fn block_sound(&mut self, id: &str, action: Action) {
        self.local_block_sounds.push(Sound::for_block(id, action));
    }
    /// Player placement only: generation, loading and failed edits remain silent.
    pub fn place_block(&mut self, pos: BlockPos, block: crate::terrain::scene::Block) -> bool {
        if self.block_name(pos).is_some() {
            return false;
        }
        let id = block.id.key();
        if !self.set_block(pos, Some(block)) {
            return false;
        }
        self.block_sound(&id, Action::Place);
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn walking_accumulates_turns_but_not_idle_air_or_teleports() {
        let mut s = Footsteps::default();
        assert!(!s.update([0.; 3], true));
        for _ in 0..20 {
            assert!(!s.update([0.; 3], true));
        }
        assert!(!s.update([0.8, 0., 0.], true));
        assert!(!s.update([0.8, 0., 0.8], true));
        assert!(s.update([0., 0., 0.8], true));
        assert!(!s.update([100., 0., 0.], true));
        assert!(!s.update([101., 1., 0.], false));
        assert!(!s.update([101., 0., 0.], true));
        assert!(!s.update([101., 1., 0.], true));
    }
    #[test]
    fn mining_hits_repeat_and_reset_with_target() {
        let mut s = MiningHits::default();
        let p = Some((1, 2, 3));
        let hits: Vec<_> = (0..9).filter(|_| s.tick(p)).collect();
        assert_eq!(hits, vec![0, 4, 8]);
        assert!(s.tick(Some((2, 2, 3))));
        assert!(!s.tick(None));
        assert!(s.tick(p));
    }
    #[test]
    fn events_keep_materials_and_action_pitch() {
        for (id, family) in [
            ("stone", "stone"),
            ("oak_planks", "wood"),
            ("grass_block", "grass"),
            ("dirt", "gravel"),
            ("glass", "glass"),
            ("sand", "sand"),
            ("white_wool", "wool"),
        ] {
            for (action, name) in [
                (Action::Step, "step"),
                (Action::Hit, "hit"),
                (Action::Break, "break"),
                (Action::Place, "place"),
            ] {
                let s = Sound::for_block(&format!("minecraft:{id}"), action);
                assert_eq!(s.event, format!("minecraft:block.{family}.{name}"));
                assert!(s.volume > 0. && s.pitch > 0.);
                let short = Sound::for_block(id, action);
                assert_eq!(
                    (s.event, s.volume, s.pitch),
                    (short.event, short.volume, short.pitch)
                );
            }
        }
    }
}
