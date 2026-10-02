//! Horses and donkeys (26.3 `AbstractHorse`, `Horse`, `AbstractChestedHorse`,
//! `Donkey`): their flags, counters and animation values, grazing and
//! rearing, and the attributes their base definitions give (summoned with
//! a tag; `finalizeSpawn` rolls a horse's health, speed and jump). They
//! live in the entity world as farm animals with this state beside them,
//! on the horse goal set (`horse_ai`).
use crate::walk_path::WalkProfile;

/// Which `AbstractHorse` it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HorseKind {
    Horse,
    Donkey,
}

impl HorseKind {
    pub fn type_id(self) -> &'static str {
        match self {
            HorseKind::Horse => "minecraft:horse",
            HorseKind::Donkey => "minecraft:donkey",
        }
    }

    /// Adult width and height (`EntityTypes`), and a baby's scale
    /// (`Horse.BABY_DIMENSIONS` 0.7; `AbstractChestedHorse` 0.5).
    pub fn dimensions(self) -> (f32, f32, f32) {
        match self {
            HorseKind::Horse => (1.3964844, 1.6, 0.7),
            HorseKind::Donkey => (1.3964844, 1.5, 0.5),
        }
    }

    /// `EntityTypes` eye height, scaled with a baby's dimensions.
    pub fn eye_height(self, baby: bool) -> f32 {
        let (_, _, scale) = self.dimensions();
        let eye = match self {
            HorseKind::Horse => 1.52,
            HorseKind::Donkey => 1.425,
        };
        if baby {
            eye * scale
        } else {
            eye
        }
    }

    /// `createBaseHorseAttributes` (and `createBaseChestedHorseAttributes`
    /// for donkeys): max health, movement speed and jump strength.
    pub fn base_attributes(self) -> (f32, f64, f64) {
        match self {
            HorseKind::Horse => (53.0, f64::from(0.225_f32), 0.7),
            HorseKind::Donkey => (53.0, f64::from(0.175_f32), 0.5),
        }
    }

    /// Its sound family: a baby horse has its own (`entity.baby_horse.*`).
    pub fn sound_family(self, baby: bool) -> &'static str {
        match self {
            HorseKind::Horse if baby => "baby_horse",
            HorseKind::Horse => "horse",
            HorseKind::Donkey => "donkey",
        }
    }
}

/// `AbstractHorse`'s state beside the animal it is.
#[derive(Clone, Debug)]
pub struct HorseState {
    pub kind: HorseKind,
    /// `Horse`'s type variant: the coat in the low byte, the markings in
    /// the next.
    pub variant: i32,
    /// `AbstractChestedHorse.hasChest`.
    pub chest: bool,
    /// `DATA_ID_FLAGS`.
    pub tame: bool,
    pub bred: bool,
    pub eating: bool,
    pub standing: bool,
    pub mouth_open: bool,
    pub eating_counter: i32,
    pub mouth_counter: i32,
    pub stand_counter: i32,
    pub tail_counter: i32,
    pub sprint_counter: i32,
    pub temper: i32,
    pub eat_anim: f32,
    pub eat_anim_o: f32,
    pub stand_anim: f32,
    pub stand_anim_o: f32,
    pub mouth_anim: f32,
    pub mouth_anim_o: f32,
    pub allow_stand_sliding: bool,
    /// `RandomStandGoal.nextStand`.
    pub next_stand: i32,
    pub max_health: f32,
    pub movement_speed: f64,
    pub jump_strength: f64,
}

impl HorseState {
    pub fn new(kind: HorseKind) -> Self {
        let (max_health, movement_speed, jump_strength) = kind.base_attributes();
        Self {
            kind,
            variant: 0,
            chest: false,
            tame: false,
            bred: false,
            eating: false,
            standing: false,
            mouth_open: false,
            eating_counter: 0,
            mouth_counter: 0,
            stand_counter: 0,
            tail_counter: 0,
            sprint_counter: 0,
            temper: 0,
            eat_anim: 0.0,
            eat_anim_o: 0.0,
            stand_anim: 0.0,
            stand_anim_o: 0.0,
            mouth_anim: 0.0,
            mouth_anim_o: 0.0,
            allow_stand_sliding: false,
            // `RandomStandGoal`'s constructor: `-getAmbientStandInterval()`.
            next_stand: -AMBIENT_SOUND_INTERVAL,
            max_health,
            movement_speed,
            jump_strength,
        }
    }

    /// `isImmobile`: grazing or rearing, it neither moves nor thinks.
    pub fn immobile(&self) -> bool {
        self.eating || self.standing
    }

    /// `setStanding(ticks)`.
    pub fn set_standing(&mut self, ticks: i32) {
        self.eating = false;
        self.standing = true;
        self.stand_counter = ticks;
    }

    /// `clearStanding`.
    pub fn clear_standing(&mut self) {
        self.standing = false;
        self.stand_counter = 0;
    }

    /// `standIfPossible` (every horse and donkey can rear): twenty ticks.
    pub fn stand_if_possible(&mut self) {
        self.set_standing(20);
    }

    /// The rest of `AbstractHorse.tick`, after the living tick: the mouth,
    /// rearing, tail and sprint counters, then the eating, standing and
    /// mouth animations ease towards their flags.
    pub fn tick_animation(&mut self) {
        if self.mouth_counter > 0 {
            self.mouth_counter += 1;
            if self.mouth_counter > 30 {
                self.mouth_counter = 0;
                self.mouth_open = false;
            }
        }
        if self.stand_counter > 0 {
            self.stand_counter -= 1;
            if self.stand_counter <= 0 {
                self.clear_standing();
            }
        }
        if self.tail_counter > 0 {
            self.tail_counter += 1;
            if self.tail_counter > 8 {
                self.tail_counter = 0;
            }
        }
        if self.sprint_counter > 0 {
            self.sprint_counter += 1;
            if self.sprint_counter > 300 {
                self.sprint_counter = 0;
            }
        }
        self.eat_anim_o = self.eat_anim;
        if self.eating {
            self.eat_anim += (1.0 - self.eat_anim) * 0.4 + 0.05;
            if self.eat_anim > 1.0 {
                self.eat_anim = 1.0;
            }
        } else {
            self.eat_anim += (0.0 - self.eat_anim) * 0.4 - 0.05;
            if self.eat_anim < 0.0 {
                self.eat_anim = 0.0;
            }
        }
        self.stand_anim_o = self.stand_anim;
        if self.standing {
            self.eat_anim = 0.0;
            self.eat_anim_o = self.eat_anim;
            self.stand_anim += (1.0 - self.stand_anim) * 0.4 + 0.05;
            if self.stand_anim > 1.0 {
                self.stand_anim = 1.0;
            }
        } else {
            self.allow_stand_sliding = false;
            self.stand_anim += (0.8 * self.stand_anim * self.stand_anim * self.stand_anim - self.stand_anim) * 0.6 - 0.05;
            if self.stand_anim < 0.0 {
                self.stand_anim = 0.0;
            }
        }
        self.mouth_anim_o = self.mouth_anim;
        if self.mouth_open {
            self.mouth_anim += (1.0 - self.mouth_anim) * 0.7 + 0.05;
            if self.mouth_anim > 1.0 {
                self.mouth_anim = 1.0;
            }
        } else {
            self.mouth_anim += (0.0 - self.mouth_anim) * 0.7 - 0.05;
            if self.mouth_anim < 0.0 {
                self.mouth_anim = 0.0;
            }
        }
    }

    /// Its walk search: an animal's, stepping up a whole block
    /// (`STEP_HEIGHT` 1).
    pub fn walk_profile(width: f32, height: f32) -> WalkProfile {
        let mut profile = WalkProfile::animal(width, height);
        profile.max_up_step = 1.0;
        profile
    }
}

/// `AbstractHorse.getAmbientSoundInterval` (and its stand interval).
pub const AMBIENT_SOUND_INTERVAL: i32 = 400;
/// `AbstractHorse.getSoundVolume`.
pub const SOUND_VOLUME: f32 = 0.8;
/// `SAFE_FALL_DISTANCE` and `FALL_DAMAGE_MULTIPLIER`.
pub const SAFE_FALL_DISTANCE: f64 = 6.0;
pub const FALL_DAMAGE_MULTIPLIER: f64 = 0.5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grazing_eases_in_and_rearing_clears_it() {
        let mut horse = HorseState::new(HorseKind::Horse);
        horse.eating = true;
        horse.tick_animation();
        assert_eq!(horse.eat_anim, 0.4_f32 + 0.05);
        horse.stand_if_possible();
        assert!(!horse.eating && horse.standing);
        horse.tick_animation();
        assert_eq!(horse.eat_anim, 0.0);
        for _ in 0..19 {
            horse.tick_animation();
        }
        assert!(!horse.standing, "twenty ticks of rearing");
    }
}
