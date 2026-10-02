//! The wolf (pinned 26.3 `Wolf`, `TamableAnimal`): 8 health wild and 40
//! tamed, 0.3 blocks a tick, 4 attack, `sized(0.6F, 0.85F)` with its eyes
//! at 0.68 (a baby at half, eyes 0.34375). Its look comes from its variant
//! (`wolf_variant`: the biome it spawned in) and its voice from its sound
//! variant (`wolf_sound_variant`, drawn at random); it growls while angry
//! (a `NeutralMob` stays angry at whatever it targets), pants or whines
//! one ambient sound in three otherwise, and speaks at 0.4 volume.
use crate::age::Age;
use crate::health::DamageState;
use crate::movement::Body;
use glam::DVec3;

/// `Wolf.createAttributes` (an `Animal`'s 16 follow range).
pub const MAX_HEALTH: f32 = 8.0;
pub const TAME_HEALTH: f32 = 40.0;
pub const MOVEMENT_SPEED: f64 = 0.3;
pub const ATTACK_DAMAGE: f32 = 4.0;
pub const FOLLOW_RANGE: f64 = 16.0;
/// `EntityTypes.WOLF` and `Wolf.BABY_DIMENSIONS`.
pub const WIDTH: f32 = 0.6;
pub const HEIGHT: f32 = 0.85;
pub const EYE_HEIGHT: f32 = 0.68;
pub const BABY_WIDTH: f32 = 0.3;
pub const BABY_HEIGHT: f32 = 0.425;
pub const BABY_EYE_HEIGHT: f32 = 0.34375;
/// `Wolf.getSoundVolume`, and `Animal.getAmbientSoundInterval`.
pub const SOUND_VOLUME: f32 = 0.4;
pub const AMBIENT_INTERVAL: i32 = 120;
/// `DyeColor.RED`, the collar's default.
pub const DEFAULT_COLLAR: u8 = 14;

/// The wolf variants (`wolf_variant`), by ID; `pale` is the default.
pub const VARIANTS: [&str; 9] = ["ashen", "black", "chestnut", "pale", "rusty", "snowy", "spotted", "striped", "woods"];
/// The sound variants (`wolf_sound_variant`) in the registry's order.
pub const SOUND_VARIANTS: [&str; 7] = ["angry", "big", "classic", "cute", "grumpy", "puglin", "sad"];

/// A sound variant's sound set (`WolfSoundVariant.WolfSoundSet`): a baby's
/// are the baby wolf's whatever its variant.
#[derive(Clone, Copy, Debug)]
pub struct WolfSounds {
    pub ambient: &'static str,
    pub death: &'static str,
    pub growl: &'static str,
    pub hurt: &'static str,
    pub pant: &'static str,
    pub step: &'static str,
    pub whine: &'static str,
}

macro_rules! adult_sounds {
    ($prefix:literal) => {
        WolfSounds {
            ambient: concat!($prefix, ".ambient"),
            death: concat!($prefix, ".death"),
            growl: concat!($prefix, ".growl"),
            hurt: concat!($prefix, ".hurt"),
            pant: concat!($prefix, ".pant"),
            step: "entity.wolf.step",
            whine: concat!($prefix, ".whine"),
        }
    };
}

/// The sound set of `variant` (a `wolf_sound_variant` ID without its
/// namespace), a baby's or an adult's.
pub fn sounds(variant: &str, baby: bool) -> WolfSounds {
    if baby {
        return WolfSounds {
            ambient: "entity.baby_wolf.ambient",
            death: "entity.baby_wolf.death",
            growl: "entity.baby_wolf.growl",
            hurt: "entity.baby_wolf.hurt",
            pant: "entity.baby_wolf.pant",
            step: "entity.baby_wolf.step",
            whine: "entity.baby_wolf.whine",
        };
    }
    match variant {
        "angry" => adult_sounds!("entity.wolf_angry"),
        "big" => adult_sounds!("entity.wolf_big"),
        "cute" => adult_sounds!("entity.wolf_cute"),
        "grumpy" => adult_sounds!("entity.wolf_grumpy"),
        "puglin" => adult_sounds!("entity.wolf_puglin"),
        "sad" => adult_sounds!("entity.wolf_sad"),
        _ => adult_sounds!("entity.wolf"),
    }
}

/// `#minecraft:wolf_food` (`#meat` and fish).
pub fn is_food(item: &str) -> bool {
    matches!(
        item.trim_start_matches("minecraft:"),
        "beef"
            | "chicken"
            | "cooked_beef"
            | "cooked_chicken"
            | "cooked_mutton"
            | "cooked_porkchop"
            | "cooked_rabbit"
            | "mutton"
            | "porkchop"
            | "rabbit"
            | "rotten_flesh"
            | "cod"
            | "cooked_cod"
            | "salmon"
            | "cooked_salmon"
            | "tropical_fish"
            | "pufferfish"
            | "rabbit_stew"
    )
}

/// `DyeColor` IDs by name, for `#wolf_collar_dyes` (`#dyes`) and their
/// `DYE` component.
pub const DYES: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black",
];

/// The dye colour of a dye item.
pub fn dye_color(item: &str) -> Option<u8> {
    let name = item.strip_prefix("minecraft:")?.strip_suffix("_dye")?;
    DYES.iter().position(|d| *d == name).map(|i| i as u8)
}

/// `BegGoal.playerHoldingInteresting` for one hand: a bone or wolf food.
pub fn interests(item: &str) -> bool {
    item == "minecraft:bone" || is_food(item)
}

#[derive(Clone, Debug)]
pub struct Wolf {
    pub body: Body,
    pub health: f32,
    pub damage: DamageState,
    pub yaw: f32,
    pub persistence_required: bool,
    pub age: Age,
    /// `wolf_variant` and `wolf_sound_variant` IDs without the namespace.
    pub variant: String,
    pub sound_variant: String,
    /// `CollarColor` (`DyeColor` ID).
    pub collar: u8,
    /// `TamableAnimal`: tamed, its owner's UUID, ordered to sit, sitting.
    pub tame: bool,
    pub owner: Option<u128>,
    pub ordered_to_sit: bool,
    pub sitting: bool,
    /// `DATA_INTERESTED_ID` (`BegGoal`) and the head's tilt it eases
    /// towards.
    pub interested: bool,
    pub interested_angle: f32,
    pub interested_angle_o: f32,
    /// `Animal.inLove`: 600 once fed, counting down.
    pub in_love: i32,
    /// The `MAX_HEALTH` attribute's base: 8 wild, 40 once taming's side
    /// effects ran (a wolf loaded tame keeps what it was saved with).
    pub max_health_base: f32,
    /// Wet from water or rain, shaking it off (`shakeAnim`).
    pub wet: bool,
    pub shaking: bool,
    pub shake_anim: f32,
    pub shake_anim_o: f32,
}

impl Wolf {
    pub fn new(position: DVec3) -> Self {
        Self {
            body: Body::new(position, WIDTH, HEIGHT),
            health: MAX_HEALTH,
            damage: DamageState::default(),
            yaw: 0.0,
            persistence_required: false,
            age: Age::default(),
            variant: "pale".to_owned(),
            sound_variant: "classic".to_owned(),
            collar: DEFAULT_COLLAR,
            tame: false,
            owner: None,
            ordered_to_sit: false,
            sitting: false,
            interested: false,
            interested_angle: 0.0,
            interested_angle_o: 0.0,
            in_love: 0,
            max_health_base: MAX_HEALTH,
            wet: false,
            shaking: false,
            shake_anim: 0.0,
            shake_anim_o: 0.0,
        }
    }

    pub fn baby(&self) -> bool {
        self.age.baby()
    }

    /// `setAge` with its size (`getDefaultDimensions`).
    pub fn set_age(&mut self, ticks: i32) {
        self.age.set(ticks);
        let (width, height) = if self.age.baby() { (BABY_WIDTH, BABY_HEIGHT) } else { (WIDTH, HEIGHT) };
        self.body.width = width;
        self.body.height = height;
    }

    pub fn eye_height(&self) -> f32 {
        if self.baby() { BABY_EYE_HEIGHT } else { EYE_HEIGHT }
    }

    pub fn max_health(&self) -> f32 {
        self.max_health_base
    }

    /// `TamableAnimal.setTame` with `Wolf.applyTamingSideEffects`: tamed,
    /// 40 health, healed full; set wild, back to 8.
    pub fn set_tame(&mut self, tame: bool, side_effects: bool) {
        self.tame = tame;
        if side_effects {
            if tame {
                self.max_health_base = TAME_HEALTH;
                self.health = TAME_HEALTH;
            } else {
                self.max_health_base = MAX_HEALTH;
            }
        }
    }

    pub fn sounds(&self) -> WolfSounds {
        sounds(&self.sound_variant, self.baby())
    }

    /// The texture it wears (`Wolf.getTexture`): its variant's tame, angry
    /// or wild look, a baby's own.
    pub fn texture(&self, angry: bool) -> String {
        let suffix = if self.tame {
            "_tame"
        } else if angry {
            "_angry"
        } else {
            ""
        };
        let baby = if self.baby() { "_baby" } else { "" };
        match self.variant.as_str() {
            // The pale wolf's textures carry no variant name.
            "pale" => format!("minecraft:entity/wolf/wolf{suffix}{baby}"),
            variant => format!("minecraft:entity/wolf/wolf_{variant}{suffix}{baby}"),
        }
    }

    /// `Wolf.getTailAngle`.
    pub fn tail_angle(&self, angry: bool) -> f32 {
        if angry {
            1.539_380_4
        } else if self.tame {
            let max = self.max_health();
            let damage = (max - self.health) / max;
            (0.55 - damage * 0.4) * std::f32::consts::PI
        } else {
            std::f32::consts::PI / 5.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_sets_and_textures_follow_their_variants() {
        assert_eq!(sounds("classic", false).ambient, "entity.wolf.ambient");
        assert_eq!(sounds("big", false).growl, "entity.wolf_big.growl");
        assert_eq!(sounds("big", false).step, "entity.wolf.step");
        assert_eq!(sounds("sad", true).step, "entity.baby_wolf.step");
        let mut wolf = Wolf::new(DVec3::ZERO);
        assert_eq!(wolf.texture(false), "minecraft:entity/wolf/wolf");
        wolf.variant = "woods".to_owned();
        assert_eq!(wolf.texture(true), "minecraft:entity/wolf/wolf_woods_angry");
        wolf.set_age(Age::BABY_START);
        assert_eq!(wolf.texture(false), "minecraft:entity/wolf/wolf_woods_baby");
        assert!(interests("minecraft:bone") && interests("minecraft:rotten_flesh") && !interests("minecraft:wheat"));
    }
}
