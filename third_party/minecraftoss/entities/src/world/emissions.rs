//! What mobs hear of their own movement (pinned 26.3 `Entity`): the step a
//! move made (`playStepSound`: the mob's own sound where it overrides the
//! block's, then the amethyst chime), its swim (`playSwimSound`) and the
//! splash of entering water (`doWaterSplashEffect`), with the draws each
//! makes from the mob's random.
use super::*;
use crate::movement::Emission;

/// A mob's movement sounds: its own step (played at 0.15 and pitch 1, as
/// the `playStepSound` overrides do) or none for the block's, and its swim
/// and splash (`getSwimSound`, `getSwimSplashSound`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct MovementSounds {
    pub step: Option<&'static str>,
    /// The own step's volume (0.15 but for the iron golem's).
    pub step_volume: f32,
    pub swim: &'static str,
    pub splash: &'static str,
    /// `AbstractHorse.playStepSound`: its own step (or its wood step on a
    /// wooden block) at the block's step volume and pitch.
    pub horse_step: Option<(&'static str, &'static str)>,
}

impl MovementSounds {
    /// A `Monster`: the hostile swim and splash.
    pub(crate) const fn monster(step: Option<&'static str>) -> Self {
        Self { step, step_volume: 0.15, swim: "entity.hostile.swim", splash: "entity.hostile.splash", horse_step: None }
    }

    /// `Entity`'s generic swim and splash.
    pub(crate) const fn creature(step: Option<&'static str>) -> Self {
        Self { step, step_volume: 0.15, swim: "entity.generic.swim", splash: "entity.generic.splash", horse_step: None }
    }

    /// The own step at another volume.
    pub(crate) const fn with_step_volume(self, volume: f32) -> Self {
        Self { step_volume: volume, ..self }
    }
}

/// `Entity.baseTick`'s fluid interaction: the body's water state, and the
/// splash of water it entered since (never on its first tick).
pub(crate) fn base_tick_fluid(body: &mut Body, world: &impl World, first_tick: bool, random: &mut LegacyRandom, voices: &mut Vec<(Voice, DVec3)>, sounds: MovementSounds) -> FluidFrame {
    let frame = body.update_fluid(world, first_tick);
    splash(body, random, voices, sounds);
    frame
}

/// After a travel: the splash of water entered during the move, then the
/// step (and chime) or swim the move made.
pub(crate) fn play_movement(body: &mut Body, tick_count: i32, random: &mut LegacyRandom, voices: &mut Vec<(Voice, DVec3)>, sounds: MovementSounds) {
    splash(body, random, voices, sounds);
    let at = body.position;
    match body.emission.take() {
        Some(Emission::Step(block, crystal)) => {
            match (sounds.step, block) {
                (_, block) if sounds.horse_step.is_some() => {
                    if let (Some((step, wood)), Some((event, volume, pitch))) = (sounds.horse_step, block) {
                        // `isWoodSoundType`.
                        let wooden = matches!(event.as_str(), "block.wood.step" | "block.nether_wood.step" | "block.stem.step" | "block.cherry_wood.step" | "block.bamboo_wood.step");
                        voices.push((Voice::Event(if wooden { wood } else { step }, volume, pitch), at));
                    }
                }
                (Some(own), _) => voices.push((Voice::Event(own, sounds.step_volume, 1.0), at)),
                (None, Some((event, volume, pitch))) => voices.push((Voice::Step(event, volume, pitch), at)),
                (None, None) => {}
            }
            // `playAmethystStepSound`, at most once a second.
            if crystal && tick_count >= body.last_crystal_sound_tick + 20 {
                let fade = 0.997_f64.powf(f64::from(tick_count - body.last_crystal_sound_tick)) as f32;
                body.crystal_sound_intensity = (body.crystal_sound_intensity * fade + 0.07).min(1.0);
                let pitch = 0.5 + body.crystal_sound_intensity * random.next_float() * 1.2;
                let volume = 0.1 + body.crystal_sound_intensity * 1.2;
                voices.push((Voice::Event("block.amethyst_block.chime", volume, pitch), at));
                body.last_crystal_sound_tick = tick_count;
            }
        }
        Some(Emission::Swim(volume)) => {
            let (a, b) = (random.next_float(), random.next_float());
            voices.push((Voice::Event(sounds.swim, volume, 1.0 + (a - b) * 0.4), at));
        }
        None => {}
    }
}

/// `doWaterSplashEffect` for a splash the body made: the splash (the
/// high-speed one is `Entity`'s generic splash for every mob here), then
/// the bubble and splash particles' draws, `1 + width * 20` of each.
fn splash(body: &mut Body, random: &mut LegacyRandom, voices: &mut Vec<(Voice, DVec3)>, sounds: MovementSounds) {
    let Some(volume) = body.splash.take() else { return };
    let event = if volume < 0.25 { sounds.splash } else { "entity.generic.splash" };
    let (a, b) = (random.next_float(), random.next_float());
    voices.push((Voice::Event(event, volume, 1.0 + (a - b) * 0.4), body.position));
    let limit = 1.0_f32 + body.width * 20.0_f32;
    let count = (0..).take_while(|&i| (i as f32) < limit).count();
    for _ in 0..count {
        let _ = (random.next_double(), random.next_double(), random.next_double());
    }
    for _ in 0..count {
        let _ = (random.next_double(), random.next_double());
    }
}
