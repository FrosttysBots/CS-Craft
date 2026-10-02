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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cosmetic { pub serial:u64, pub paint: u32, pub seed: u32, pub wear: f32, pub stattrak:Option<u32>, pub souvenir:Option<u32>, pub stickers:[Option<(u32,f32)>;4] }

#[derive(Clone, Debug)]
pub struct WeaponState {
    pub def: Arc<WeaponDefinition>,
    pub cosmetic: Option<Cosmetic>,
    pub cosmetic_owner: Option<crate::PlayerId>,
    pub clip: i32,
    pub reserve: i32,
    /// Sim time at which the next shot may fire.
    pub next_attack: f32,
    /// Sim time at which an in-progress reload completes.
    pub reload_end: Option<f32>,
    /// Sim time the current (or last) reload began.
    pub reload_start: f32,
    /// Shells loaded so far in a shell-by-shell reload.
    pub shells_loaded: i32,
    /// 0 is unscoped; other levels come from the installed item schema.
    pub zoom_level: u8,
    pub next_secondary_attack: f32,
    pub zoom_changed_at: f32,
    pub zoom_from_fov: f32,
    pub resume_zoom: Option<u8>,
    pub last_melee_attack: Option<f32>,
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
            cosmetic: None,
            cosmetic_owner: None,
            clip,
            reserve,
            next_attack: 0.0,
            reload_end: None,
            reload_start: 0.0,
            shells_loaded: 0,
            zoom_level: 0,
            next_secondary_attack: 0.0,
            zoom_changed_at: 0.0,
            zoom_from_fov: 90.0,
            resume_zoom: None,
            last_melee_attack: None,
            accuracy_penalty: 0.0,
            recoil_index: 0.0,
            pattern,
        }
    }

    pub fn zoom_levels(&self) -> u8 {
        self.def.attr_f32("zoom levels").unwrap_or(0.0).clamp(0.0, 2.0) as u8
    }

    pub fn fov(&self) -> f32 {
        if self.zoom_level == 0 { 90.0 } else {
            self.def.attr_f32(&format!("zoom fov {}", self.zoom_level)).unwrap_or(90.0)
        }
    }

    pub fn rendered_fov(&self, time: f32) -> f32 {
        let duration = self.def.attr_f32(&format!("zoom time {}", self.zoom_level)).unwrap_or(0.05).max(0.001);
        let t = ((time - self.zoom_changed_at) / duration).clamp(0.0, 1.0);
        self.zoom_from_fov + (self.fov() - self.zoom_from_fov) * t
    }

    pub fn set_zoom(&mut self, level: u8, time: f32) {
        self.zoom_from_fov = self.rendered_fov(time);
        self.zoom_level = level.min(self.zoom_levels());
        self.zoom_changed_at = time;
    }

    pub fn reset_zoom(&mut self, time: f32) {
        self.set_zoom(0, time);
        self.resume_zoom = None;
    }

    pub fn alt_stat(&self, key: &str, base: f32) -> f32 {
        if self.zoom_level > 0 { self.def.attr_f32(&format!("{key} alt")).unwrap_or(base) } else { base }
    }

    pub fn max_speed(&self) -> f32 { self.alt_stat("max player speed", self.def.max_player_speed) }

    pub fn hides_viewmodel(&self) -> bool {
        self.zoom_level > 0 && self.def.attr_f32("hide view model zoomed").is_some_and(|v| v != 0.0)
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

    /// Shell-by-shell timing if this weapon loads one shell at a time.
    pub fn shell_reload(&self) -> Option<ShellReload> {
        shell_reload(&self.def)
    }

    /// Starts a reload: magazines finish after [`reload_time`]; shotguns
    /// that load shells take a start, then one insert per missing shell (up
    /// to the reserve), loading each shell as its insert completes.
    pub fn begin_reload(&mut self, time: f32) {
        self.reload_start = time;
        self.shells_loaded = 0;
        let total = match self.shell_reload() {
            Some(s) => {
                let shells = (self.def.clip_size - self.clip).min(self.reserve).max(1);
                s.start + s.insert * shells as f32
            }
            None => reload_time(&self.def),
        };
        self.reload_end = Some(time + total);
    }

    /// 0..1 through the current reload.
    pub fn reload_fraction(&self, time: f32) -> Option<f32> {
        let end = self.reload_end?;
        Some(((time - self.reload_start) / (end - self.reload_start).max(0.01)).clamp(0.0, 1.0))
    }

    /// Where a shell-by-shell reload is: the start (`None`) or the insert
    /// of shell `n` (0-based), with 0..1 progress through that stage.
    pub fn shell_stage(&self, time: f32) -> Option<(Option<i32>, f32)> {
        let s = self.shell_reload()?;
        let end = self.reload_end?;
        let t = time - self.reload_start;
        if t < s.start {
            return Some((None, (t / s.start.max(0.01)).clamp(0.0, 1.0)));
        }
        // Rendering interpolates slightly past the last tick: stay on the
        // final insert rather than start one that will not happen.
        let shells = ((end - self.reload_start - s.start) / s.insert.max(0.01)).round().max(1.0);
        let k = ((t - s.start) / s.insert.max(0.01)).clamp(0.0, shells - 1e-3);
        Some((Some(k.floor() as i32), k.fract()))
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

/// Stages of a shell-by-shell reload, in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellReload {
    /// Bringing the gun up to the loading port.
    pub start: f32,
    /// One shell.
    pub insert: f32,
    /// Racking it afterwards (cosmetic; the gun can fire).
    pub finish: f32,
}

/// Shotguns without a magazine (the item schema gives the MAG-7 a
/// `magazine model`; the Nova, XM1014 and Sawed-Off have none) load one
/// shell at a time. Stage lengths are those of the view-model sequences
/// `start_reload`, `insert` and `after_reload`.
pub fn shell_reload(def: &WeaponDefinition) -> Option<ShellReload> {
    if def.class != WeaponClass::Shotgun || def.attributes.contains_key("magazine model") {
        return None;
    }
    let (start, insert, finish) = match def.class_name.as_str() {
        "weapon_nova" => (12.0 / 30.0, 14.0 / 30.0, 25.0 / 30.0),
        "weapon_xm1014" => (21.0 / 30.0, 13.0 / 20.0, 13.0 / 30.0),
        "weapon_sawedoff" => (12.0 / 30.0, 15.0 / 30.0, 26.0 / 30.0),
        _ => (0.4, 0.5, 0.8),
    };
    Some(ShellReload { start, insert, finish })
}

/// Seconds after switching before the weapon can fire.
pub fn deploy_time(def: &WeaponDefinition) -> f32 {
    match def.class {
        WeaponClass::Knife => 0.5,
        WeaponClass::Pistol => 0.8,
        _ => 1.0,
    }
}

/// The bomb's class name.
pub const C4: &str = "weapon_c4";

/// CS:GO inventory slot: 1 primary, 2 pistol, 3 knife/zeus, 4 grenades,
/// 5 bomb.
pub fn slot(def: &WeaponDefinition) -> u8 {
    match def.class {
        WeaponClass::Pistol => 2,
        WeaponClass::Knife => 3,
        WeaponClass::Grenade => 4,
        WeaponClass::Equipment if def.class_name == C4 => 5,
        WeaponClass::Equipment => 3,
        _ => 1,
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
    let base = if ctx.ducked { w.alt_stat("inaccuracy crouch", a.inaccuracy_crouch) } else { w.alt_stat("inaccuracy stand", a.inaccuracy_stand) };
    let max_speed = w.max_speed().max(1.0);
    let lo = max_speed * 0.34;
    let hi = max_speed * 0.95;
    let move_frac = ((ctx.speed - lo) / (hi - lo)).clamp(0.0, 1.0);
    let air = if ctx.on_ground { 0.0 } else { w.alt_stat("inaccuracy jump", a.inaccuracy_jump) };
    base + w.alt_stat("inaccuracy move", a.inaccuracy_move) * move_frac + air + w.accuracy_penalty
}

/// Advances timers that run every tick whether or not the player fires.
pub fn think(w: &mut WeaponState, time: f32, dt: f32, ducked: bool, firing: bool) {
    if time >= w.next_attack && w.reload_end.is_none() {
        if let Some(level) = w.resume_zoom.take() { w.set_zoom(level, time); }
    }
    let recovery = if ducked {
        w.alt_stat("recovery time crouch", w.def.accuracy.recovery_time_crouch)
    } else {
        w.alt_stat("recovery time stand", w.def.accuracy.recovery_time_stand)
    }
    .max(0.05);
    // Decays to 10 % over the recovery time.
    w.accuracy_penalty *= 0.1f32.powf(dt / recovery);
    if !firing {
        w.recoil_index = (w.recoil_index - RECOIL_RECOVERY_RATE * dt).max(0.0);
    }
    if let Some(end) = w.reload_end {
        if let Some(s) = shell_reload(&w.def) {
            // One shell each time an insert completes (all of them by the end).
            let due = if time >= end {
                i32::MAX
            } else {
                (((time - w.reload_start - s.start) / s.insert.max(0.01)).floor() as i32).max(0)
            };
            while w.shells_loaded < due && w.clip < w.def.clip_size && w.reserve > 0 {
                w.clip += 1;
                w.reserve -= 1;
                w.shells_loaded += 1;
            }
            if time >= end || w.clip >= w.def.clip_size || w.reserve <= 0 {
                w.reload_end = None;
            }
        } else if time >= end {
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
            zoom_in_sound: String::new(),
            zoom_out_sound: String::new(),
            kill_award: 300,
            attributes: Default::default(),
        }
    }

    fn shotgun(name: &str, magazine: bool) -> WeaponDefinition {
        let mut d = ak();
        d.class_name = name.into();
        d.class = WeaponClass::Shotgun;
        d.clip_size = 8;
        d.reserve_ammo = 32;
        if magazine {
            d.attributes.insert("magazine model".into(), "models/weapons/w_shot_mag7_mag.mdl".into());
        }
        d
    }

    fn run_reload(w: &mut WeaponState, from: f32, until: f32) {
        let mut t = from;
        while t < until {
            think(w, t, crate::TICK_DT, false, false);
            t += crate::TICK_DT;
        }
    }

    #[test]
    fn shotguns_load_shell_by_shell() {
        let mut w = WeaponState::new(Arc::new(shotgun("weapon_nova", false)));
        let s = w.shell_reload().expect("the Nova loads shells");
        w.clip = 0;
        w.begin_reload(0.0);
        // Eight missing shells: the start plus eight inserts.
        let end = w.reload_end.unwrap();
        assert!((end - (s.start + 8.0 * s.insert)).abs() < 1e-4);
        assert_eq!(w.shell_stage(0.1).unwrap().0, None);
        assert_eq!(w.shell_stage(s.start + 2.5 * s.insert).unwrap().0, Some(2));
        // Shells arrive one per insert.
        run_reload(&mut w, 0.0, s.start + 2.5 * s.insert);
        assert_eq!(w.clip, 2);
        assert_eq!(w.reserve, 30);
        run_reload(&mut w, s.start + 2.5 * s.insert, end + 0.1);
        assert_eq!(w.clip, 8);
        assert_eq!(w.reserve, 24);
        assert!(w.reload_end.is_none());
        // Only as many inserts as the reserve allows.
        w.clip = 0;
        w.reserve = 3;
        w.begin_reload(10.0);
        assert!((w.reload_end.unwrap() - 10.0 - (s.start + 3.0 * s.insert)).abs() < 1e-4);
        // The MAG-7 has a magazine.
        assert!(WeaponState::new(Arc::new(shotgun("weapon_mag7", true))).shell_reload().is_none());
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
