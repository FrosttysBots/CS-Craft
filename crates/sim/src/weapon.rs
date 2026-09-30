//! Weapon behaviour: firing rate, ammo, reloads, CS:GO-style inaccuracy and
//! a deterministic per-weapon recoil pattern.
//!
//! Stats come from [`weapons::WeaponDefinition`] (the game's own
//! `items_game.txt`). Values the item schema does not carry (reload and
//! deploy times) are approximations listed in [`reload_time`].
//!
//! Inaccuracy follows the public description of CS:GO's model: a base value
//! (standing/crouched), plus a movement term that grows between 34 % and
//! 95 % of max speed, plus a jump term in the air, plus a firing penalty
//! that decays to 10 % over the weapon's recovery time. Inaccuracy and
//! spread values are in milliradians.

use crate::rng::Rng;
use std::sync::Arc;
use weapons::{WeaponClass, WeaponDefinition};

/// Degrees of aim punch per unit of `recoil magnitude`.
pub const RECOIL_DEGREES_PER_UNIT: f32 = 0.02;
/// CS:GO `weapon_recoil_scale`: bullets follow twice the aim punch.
pub const RECOIL_SCALE: f32 = 2.0;
/// Fraction of the (scaled) aim punch shown by the camera.
pub const VIEW_PUNCH_FRACTION: f32 = 0.5;
/// Recoil recovery when not firing, in pattern steps per second.
pub const RECOIL_RECOVERY_RATE: f32 = 12.0;

#[derive(Clone, Debug)]
pub struct WeaponState {
    pub def: Arc<WeaponDefinition>,
    pub clip: i32,
    pub reserve: i32,
    /// Sim time at which the next shot may fire.
    pub next_attack: f32,
    /// Sim time at which an in-progress reload completes.
    pub reload_end: Option<f32>,
    /// Firing inaccuracy, milliradians; decays with recovery time.
    pub accuracy_penalty: f32,
    /// Position in the recoil pattern (shots, fractional while recovering).
    pub recoil_index: f32,
    /// Precomputed cumulative recoil offsets (pitch, yaw) in degrees.
    pattern: Arc<Vec<[f32; 2]>>,
}

impl WeaponState {
    pub fn new(def: Arc<WeaponDefinition>) -> Self {
        let clip = def.clip_size.max(0);
        let reserve = def.reserve_ammo.max(0);
        let pattern = Arc::new(recoil_pattern(&def));
        Self {
            def,
            clip,
            reserve,
            next_attack: 0.0,
            reload_end: None,
            accuracy_penalty: 0.0,
            recoil_index: 0.0,
            pattern,
        }
    }

    pub fn is_melee(&self) -> bool {
        self.def.class == WeaponClass::Knife
    }

    pub fn uses_ammo(&self) -> bool {
        self.def.clip_size > 0
    }

    pub fn can_reload(&self) -> bool {
        self.uses_ammo() && self.reload_end.is_none() && self.clip < self.def.clip_size && self.reserve > 0
    }

    /// Current aim punch (pitch, yaw) in degrees from the recoil pattern,
    /// interpolated at the fractional recoil index. Negative pitch is up.
    pub fn aim_punch(&self) -> [f32; 2] {
        if self.pattern.is_empty() || self.recoil_index <= 0.0 {
            return [0.0, 0.0];
        }
        let max = (self.pattern.len() - 1) as f32;
        let i = self.recoil_index.min(max);
        let lo = i.floor() as usize;
        let hi = (lo + 1).min(self.pattern.len() - 1);
        let t = i - lo as f32;
        let a = self.pattern[lo];
        let b = self.pattern[hi];
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    }
}

/// A fixed pattern per weapon, generated from its recoil stats and seed:
/// each shot kicks by `magnitude` along `angle ± variance` (0° = up).
pub fn recoil_pattern(def: &WeaponDefinition) -> Vec<[f32; 2]> {
    let shots = (def.clip_size.max(1) as usize + 1).min(256);
    let mut rng = Rng::from_parts(&[def.recoil.seed as u64, def.def_index as u64]);
    let mut out = Vec::with_capacity(shots + 1);
    let (mut pitch, mut yaw) = (0.0f32, 0.0f32);
    out.push([0.0, 0.0]);
    for shot in 0..shots {
        let angle = def.recoil.angle + rng.range(-def.recoil.angle_variance, def.recoil.angle_variance);
        let magnitude = def.recoil.magnitude + rng.range(-def.recoil.magnitude_variance, def.recoil.magnitude_variance);
        // The first shots of a spray climb almost straight up.
        let straight = if shot < 3 { 0.25 } else { 1.0 };
        let a = (angle * straight).to_radians();
        let deg = magnitude.max(0.0) * RECOIL_DEGREES_PER_UNIT;
        pitch -= deg * a.cos();
        yaw += deg * a.sin();
        out.push([pitch, yaw]);
    }
    out
}

/// Seconds to reload. The item schema does not store this (it comes from
/// the view-model animations), so these are measured approximations.
pub fn reload_time(def: &WeaponDefinition) -> f32 {
    match def.class_name.as_str() {
        "weapon_ak47" => 2.43,
        "weapon_m4a1" => 3.07,
        "weapon_m4a1_silencer" => 3.07,
        "weapon_awp" => 3.67,
        "weapon_ssg08" => 3.7,
        "weapon_deagle" => 2.2,
        "weapon_glock" => 2.27,
        "weapon_usp_silencer" | "weapon_hkp2000" => 2.17,
        "weapon_famas" => 3.3,
        "weapon_galilar" => 3.0,
        "weapon_aug" => 3.8,
        "weapon_sg556" => 2.83,
        "weapon_negev" | "weapon_m249" => 5.7,
        _ => match def.class {
            WeaponClass::Pistol => 2.2,
            WeaponClass::Smg => 2.6,
            WeaponClass::Rifle => 2.9,
            WeaponClass::Sniper => 3.7,
            WeaponClass::Shotgun => 3.0,
            WeaponClass::MachineGun => 5.7,
            _ => 2.5,
        },
    }
}

/// Seconds after switching before the weapon can fire.
pub fn deploy_time(def: &WeaponDefinition) -> f32 {
    match def.class {
        WeaponClass::Knife => 0.5,
        WeaponClass::Pistol => 0.8,
        _ => 1.0,
    }
}

/// Inputs to the inaccuracy model for one shot.
#[derive(Clone, Copy, Debug)]
pub struct AccuracyContext {
    pub speed: f32,
    pub on_ground: bool,
    pub ducked: bool,
}

/// Total inaccuracy (milliradians) for the next shot.
pub fn inaccuracy(w: &WeaponState, ctx: AccuracyContext) -> f32 {
    let a = &w.def.accuracy;
    let base = if ctx.ducked { a.inaccuracy_crouch } else { a.inaccuracy_stand };
    let max_speed = w.def.max_player_speed.max(1.0);
    let lo = max_speed * 0.34;
    let hi = max_speed * 0.95;
    let move_frac = ((ctx.speed - lo) / (hi - lo)).clamp(0.0, 1.0);
    let air = if ctx.on_ground { 0.0 } else { a.inaccuracy_jump };
    base + a.inaccuracy_move * move_frac + air + w.accuracy_penalty
}

/// Advances timers that run every tick whether or not the player fires.
pub fn think(w: &mut WeaponState, time: f32, dt: f32, ducked: bool, firing: bool) {
    let recovery = if ducked {
        w.def.accuracy.recovery_time_crouch
    } else {
        w.def.accuracy.recovery_time_stand
    }
    .max(0.05);
    // Decays to 10 % over the recovery time.
    w.accuracy_penalty *= 0.1f32.powf(dt / recovery);
    if !firing {
        w.recoil_index = (w.recoil_index - RECOIL_RECOVERY_RATE * dt).max(0.0);
    }
    if let Some(end) = w.reload_end {
        if time >= end {
            let need = (w.def.clip_size - w.clip).max(0);
            let take = need.min(w.reserve);
            w.clip += take;
            w.reserve -= take;
            w.reload_end = None;
        }
    }
}

/// Damage after range falloff, hit group and armour, as CS:GO computes it.
/// Returns (health damage, armour damage).
pub fn damage(def: &WeaponDefinition, distance: f32, group: HitGroup, armor: i32, helmet: bool) -> (i32, i32) {
    let mut dmg = def.damage * def.range_modifier.powf(distance / 500.0);
    dmg *= group.multiplier();
    let armored = armor > 0 && match group {
        HitGroup::Head => helmet,
        HitGroup::LeftLeg | HitGroup::RightLeg => false,
        _ => true,
    };
    if !armored {
        return (dmg as i32, 0);
    }
    let ratio = def.armor_ratio * 0.5;
    let mut health = dmg * ratio;
    let mut armor_dmg = (dmg - health) * 0.5;
    if armor_dmg > armor as f32 {
        armor_dmg = armor as f32;
        health = dmg - armor_dmg * 2.0;
    }
    (health as i32, armor_dmg as i32)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitGroup {
    Head,
    Chest,
    Stomach,
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
}

impl HitGroup {
    pub fn multiplier(self) -> f32 {
        match self {
            HitGroup::Head => 4.0,
            HitGroup::Stomach => 1.25,
            HitGroup::LeftLeg | HitGroup::RightLeg => 0.75,
            _ => 1.0,
        }
    }

    /// Hit group from where a bullet entered a standing or crouched hull,
    /// as a fraction of the hull height (until player hitboxes are loaded
    /// from the character models).
    pub fn from_height(frac: f32, side: f32) -> HitGroup {
        if frac > 0.86 {
            HitGroup::Head
        } else if frac > 0.62 {
            if side.abs() > 0.7 {
                if side > 0.0 { HitGroup::RightArm } else { HitGroup::LeftArm }
            } else {
                HitGroup::Chest
            }
        } else if frac > 0.46 {
            HitGroup::Stomach
        } else if side >= 0.0 {
            HitGroup::RightLeg
        } else {
            HitGroup::LeftLeg
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use content::{ContentId, ContentKind};

    fn ak() -> WeaponDefinition {
        WeaponDefinition {
            id: ContentId::csgo(ContentKind::Weapon, "ak47"),
            class_name: "weapon_ak47".into(),
            def_index: 7,
            name_token: String::new(),
            class: WeaponClass::Rifle,
            side: weapons::Side::Terrorist,
            price: 2700,
            damage: 36.0,
            armor_ratio: 1.55,
            penetration: 2.0,
            range: 8192.0,
            range_modifier: 0.98,
            cycle_time: 0.1,
            full_auto: true,
            bullets: 1,
            clip_size: 30,
            reserve_ammo: 90,
            max_player_speed: 215.0,
            accuracy: weapons::Accuracy {
                spread: 0.6,
                inaccuracy_stand: 6.41,
                inaccuracy_crouch: 4.81,
                inaccuracy_move: 175.06,
                inaccuracy_jump: 140.76,
                inaccuracy_land: 0.242,
                inaccuracy_fire: 7.8,
                recovery_time_stand: 0.368,
                recovery_time_crouch: 0.305,
            },
            recoil: weapons::Recoil {
                angle: 0.0,
                angle_variance: 70.0,
                magnitude: 30.0,
                magnitude_variance: 0.0,
                seed: 223,
            },
            view_model: String::new(),
            world_model: String::new(),
            fire_sound: String::new(),
            kill_award: 300,
            attributes: Default::default(),
        }
    }

    #[test]
    fn ak_damage_matches_csgo() {
        // Known CS:GO values at close range: head 111 with a helmet, 143
        // without; chest 27 through armour, 35 without.
        let (hp, _) = damage(&ak(), 100.0, HitGroup::Head, 100, true);
        assert_eq!(hp, 111);
        let (hp, _) = damage(&ak(), 100.0, HitGroup::Head, 100, false);
        assert_eq!(hp, 143);
        let (hp, armor) = damage(&ak(), 100.0, HitGroup::Chest, 100, false);
        assert_eq!(hp, 27);
        assert!(armor > 0);
        let (hp, _) = damage(&ak(), 100.0, HitGroup::Chest, 0, false);
        assert_eq!(hp, 35);
    }

    #[test]
    fn running_is_inaccurate() {
        let w = WeaponState::new(Arc::new(ak()));
        let still = inaccuracy(&w, AccuracyContext { speed: 0.0, on_ground: true, ducked: false });
        let walking = inaccuracy(&w, AccuracyContext { speed: 70.0, on_ground: true, ducked: false });
        let running = inaccuracy(&w, AccuracyContext { speed: 215.0, on_ground: true, ducked: false });
        assert_eq!(still, walking);
        assert!(running > still * 20.0);
    }

    #[test]
    fn recoil_pattern_is_fixed_and_climbs() {
        let a = recoil_pattern(&ak());
        let b = recoil_pattern(&ak());
        assert_eq!(a, b);
        assert!(a[10][0] < -3.0, "pitch after 10 shots {}", a[10][0]);
    }
}
