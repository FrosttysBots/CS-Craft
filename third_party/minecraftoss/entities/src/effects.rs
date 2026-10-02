//! Status effects on mobs (pinned 26.3 `MobEffectInstance`, `MobEffects`,
//! `HealOrHarmMobEffect`, `PoisonMobEffect`, `RegenerationMobEffect` and
//! `LivingEntity.addEffect`/`canBeAffected`/`tickEffects`): the effects the
//! potions of witches carry. An instance keeps a weaker but longer effect
//! hidden under a stronger one (`update`), and falls back to it when its
//! own time runs out. Each server tick counts every effect down after its
//! work for the tick (poison and regeneration on their intervals, an
//! instant effect once). Speed and slowness scale `MOVEMENT_SPEED`
//! (`ADD_MULTIPLIED_TOTAL`), weakness lowers `ATTACK_DAMAGE`.
//!
//! Vanilla keeps the effects in a `HashMap` keyed by identity-hashed
//! holders, so the order two effects tick in on the same tick varies from
//! run to run; here they tick in the order they were added.

/// A status effect a mob can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MobEffect {
    Speed,
    Slowness,
    InstantHealth,
    InstantDamage,
    Regeneration,
    FireResistance,
    WaterBreathing,
    Weakness,
    Poison,
}

impl MobEffect {
    pub const ALL: [MobEffect; 9] = [
        Self::Speed,
        Self::Slowness,
        Self::InstantHealth,
        Self::InstantDamage,
        Self::Regeneration,
        Self::FireResistance,
        Self::WaterBreathing,
        Self::Weakness,
        Self::Poison,
    ];

    /// Its registry ID.
    pub fn id(self) -> &'static str {
        match self {
            Self::Speed => "minecraft:speed",
            Self::Slowness => "minecraft:slowness",
            Self::InstantHealth => "minecraft:instant_health",
            Self::InstantDamage => "minecraft:instant_damage",
            Self::Regeneration => "minecraft:regeneration",
            Self::FireResistance => "minecraft:fire_resistance",
            Self::WaterBreathing => "minecraft:water_breathing",
            Self::Weakness => "minecraft:weakness",
            Self::Poison => "minecraft:poison",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.id() == id)
    }

    /// `MobEffect.isInstantaneous`: instant health and damage.
    pub fn instantaneous(self) -> bool {
        matches!(self, Self::InstantHealth | Self::InstantDamage)
    }

    /// `shouldApplyEffectTickThisTick` for the ticks left: an instant
    /// effect while any are left, poison every 25 ticks and regeneration
    /// every 50, halved a level (every tick once the shift runs out).
    fn applies_this_tick(self, ticks: i32, amplifier: i32) -> bool {
        let every = |interval: i32| {
            // Java's shift takes its distance modulo 32.
            let interval = interval.wrapping_shr(amplifier as u32);
            interval <= 0 || ticks % interval == 0
        };
        match self {
            Self::InstantHealth | Self::InstantDamage => ticks >= 1,
            Self::Poison => every(25),
            Self::Regeneration => every(50),
            _ => false,
        }
    }

    /// Its `MOVEMENT_SPEED` modifier's amount at level 1
    /// (`ADD_MULTIPLIED_TOTAL`, a float literal widened).
    fn speed_amount(self) -> Option<f64> {
        match self {
            Self::Speed => Some(f64::from(0.2_f32)),
            Self::Slowness => Some(f64::from(-0.15_f32)),
            _ => None,
        }
    }
}

/// `MobEffectInstance`: an effect's time left (-1 forever), level and
/// display flags, with the weaker effect hidden under it.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectInstance {
    pub effect: MobEffect,
    pub duration: i32,
    pub amplifier: i32,
    pub ambient: bool,
    pub visible: bool,
    pub show_icon: bool,
    pub hidden: Option<Box<EffectInstance>>,
}

impl EffectInstance {
    /// `new MobEffectInstance(effect, duration, amplifier)`: not ambient,
    /// with particles and an icon.
    pub fn new(effect: MobEffect, duration: i32, amplifier: i32) -> Self {
        Self { effect, duration, amplifier: amplifier.clamp(0, 255), ambient: false, visible: true, show_icon: true, hidden: None }
    }

    /// `MobEffectInstance(copy)`: the details without the hidden effect.
    fn copy(&self) -> Self {
        Self { hidden: None, ..self.clone() }
    }

    pub fn infinite(&self) -> bool {
        self.duration == -1
    }

    fn has_remaining_duration(&self) -> bool {
        self.infinite() || self.duration > 0
    }

    fn shorter_than(&self, other: &Self) -> bool {
        !self.infinite() && (self.duration < other.duration || other.infinite())
    }

    /// `endsWithin`.
    pub fn ends_within(&self, ticks: i32) -> bool {
        !self.infinite() && self.duration <= ticks
    }

    /// `mapDuration`: infinite and spent durations stay as they are.
    pub fn map_duration(&self, f: impl FnOnce(i32) -> i32) -> i32 {
        if !self.infinite() && self.duration != 0 {
            f(self.duration)
        } else {
            self.duration
        }
    }

    /// `update(takeOver)`: a stronger effect takes over (hiding this one if
    /// it would outlast it), a longer one of the same level extends it, and
    /// a longer weaker one hides beneath. Returns whether it changed.
    fn update(&mut self, take_over: &Self) -> bool {
        let mut changed = false;
        if take_over.amplifier > self.amplifier {
            if take_over.shorter_than(self) {
                let previous = self.hidden.take();
                let mut hidden = self.copy();
                hidden.hidden = previous;
                self.hidden = Some(Box::new(hidden));
            }
            self.amplifier = take_over.amplifier;
            self.duration = take_over.duration;
            changed = true;
        } else if self.shorter_than(take_over) {
            if take_over.amplifier == self.amplifier {
                self.duration = take_over.duration;
                changed = true;
            } else if let Some(hidden) = self.hidden.as_mut() {
                hidden.update(take_over);
            } else {
                self.hidden = Some(Box::new(take_over.copy()));
            }
        }
        if (!take_over.ambient && self.ambient) || changed {
            self.ambient = take_over.ambient;
            changed = true;
        }
        if take_over.visible != self.visible {
            self.visible = take_over.visible;
            changed = true;
        }
        if take_over.show_icon != self.show_icon {
            self.show_icon = take_over.show_icon;
            changed = true;
        }
        changed
    }

    fn tick_down(&mut self) {
        if let Some(hidden) = self.hidden.as_mut() {
            hidden.tick_down();
        }
        self.duration = self.map_duration(|d| d - 1);
    }

    /// `downgradeToHiddenEffect`: spent, the hidden effect comes back.
    fn downgrade_to_hidden(&mut self) -> bool {
        if self.duration != 0 {
            return false;
        }
        let Some(hidden) = self.hidden.take() else { return false };
        let hidden = *hidden;
        self.duration = hidden.duration;
        self.amplifier = hidden.amplifier;
        self.ambient = hidden.ambient;
        self.visible = hidden.visible;
        self.show_icon = hidden.show_icon;
        self.hidden = hidden.hidden;
        true
    }
}

/// What an effect does to its mob on a tick it works
/// (`applyEffectTick`), for the mob to carry out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EffectWork {
    /// `heal`: only a living mob heals.
    Heal(f32),
    /// `hurtServer` with `magic`.
    HurtMagic(f32),
    /// Poison: 1 magic damage while more than 1 health is left.
    Poison,
    /// Regeneration: 1 health below full.
    Regenerate,
}

impl EffectWork {
    /// What it comes to for a mob at `health` of `max_health` (poison and
    /// regeneration look at the health when they work).
    pub fn resolve(self, health: f32, max_health: f32) -> Option<EffectWork> {
        match self {
            Self::Poison => (health > 1.0).then_some(Self::HurtMagic(1.0)),
            Self::Regenerate => (health < max_health).then_some(Self::Heal(1.0)),
            other => Some(other),
        }
    }
}

/// A mob's active effects.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MobEffects {
    active: Vec<EffectInstance>,
}

impl MobEffects {
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &EffectInstance> {
        self.active.iter()
    }

    /// `hasEffect`.
    pub fn has(&self, effect: MobEffect) -> bool {
        self.active.iter().any(|e| e.effect == effect)
    }

    pub fn get(&self, effect: MobEffect) -> Option<&EffectInstance> {
        self.active.iter().find(|e| e.effect == effect)
    }

    /// `LivingEntity.addEffect` once `canBeAffected` has agreed: a new
    /// effect, or an update of the one it has. Returns whether it changed.
    pub fn add(&mut self, instance: EffectInstance) -> bool {
        match self.active.iter_mut().find(|e| e.effect == instance.effect) {
            None => {
                self.active.push(instance);
                true
            }
            Some(effect) => effect.update(&instance),
        }
    }

    /// Puts an effect back as saved (`readAdditionalSaveData`).
    pub fn insert(&mut self, instance: EffectInstance) {
        self.active.retain(|e| e.effect != instance.effect);
        self.active.push(instance);
    }

    /// The server half of `LivingEntity.tickEffects`: each effect works if
    /// its time has come (`MobEffectInstance.tickServer`), counts down,
    /// takes back its hidden effect when spent and goes when nothing is
    /// left. Returns the work due, in order, for the mob to carry out (none
    /// of these effects stops on its work, and none looks at the others').
    /// `inverted` is `isInvertedHealAndHarm` (the undead).
    pub fn tick(&mut self, inverted: bool) -> Vec<EffectWork> {
        let mut work = Vec::new();
        self.active.retain_mut(|effect| {
            if !effect.has_remaining_duration() {
                return false;
            }
            // Infinite effects tick by the mob's age; none here are.
            if effect.effect.applies_this_tick(effect.duration, effect.amplifier) {
                work.extend(effect_work(effect.effect, effect.amplifier, inverted));
            }
            effect.tick_down();
            effect.downgrade_to_hidden();
            effect.has_remaining_duration()
        });
        work
    }

    /// `MOVEMENT_SPEED` from its value before the effects' modifiers:
    /// speed's and slowness's `ADD_MULTIPLIED_TOTAL` in the order the
    /// attribute's hash map keeps them (`effect.speed`'s slot comes
    /// before `effect.slowness`'s), clamped to the attribute's range.
    pub fn movement_speed(&self, value: f64) -> f64 {
        let mut result = value;
        for effect in [MobEffect::Speed, MobEffect::Slowness] {
            if let (Some(instance), Some(amount)) = (self.get(effect), effect.speed_amount()) {
                result *= 1.0 + amount * f64::from(instance.amplifier + 1);
            }
        }
        result.clamp(0.0, 1024.0)
    }

    /// `ATTACK_DAMAGE`'s `ADD_VALUE` modifiers: weakness's -4 a level.
    pub fn attack_damage_modifier(&self) -> f64 {
        self.get(MobEffect::Weakness).map_or(0.0, |e| -4.0 * f64::from(e.amplifier + 1))
    }
}

/// `applyEffectTick` for one effect: instant health heals 4 a level
/// (doubling) and instant damage hurts 6 (the other way round for the
/// undead), poison hurts 1 while more than 1 health is left, regeneration
/// heals 1 below full health ([`EffectWork::resolve`] checks the health).
fn effect_work(effect: MobEffect, amplifier: i32, inverted: bool) -> Option<EffectWork> {
    let heal = || EffectWork::Heal(4_i32.wrapping_shl(amplifier as u32).max(0) as f32);
    let harm = || EffectWork::HurtMagic(6_i32.wrapping_shl(amplifier as u32) as f32);
    match effect {
        MobEffect::InstantHealth => Some(if inverted { harm() } else { heal() }),
        MobEffect::InstantDamage => Some(if inverted { heal() } else { harm() }),
        MobEffect::Poison => Some(EffectWork::Poison),
        MobEffect::Regeneration => Some(EffectWork::Regenerate),
        _ => None,
    }
}

/// `applyInstantaneousEffect` from a splash at `scale` (1 at the heart):
/// the heal or the harm, rounded (`(int)(scale * n + 0.5)`).
pub fn instant_work(effect: MobEffect, amplifier: i32, inverted: bool, scale: f64) -> Option<EffectWork> {
    let heal_amount = || (scale * f64::from(4_i32.wrapping_shl(amplifier as u32)) + 0.5) as i32 as f32;
    let harm_amount = || (scale * f64::from(6_i32.wrapping_shl(amplifier as u32)) + 0.5) as i32 as f32;
    let harm = matches!(effect, MobEffect::InstantDamage);
    if !effect.instantaneous() {
        return None;
    }
    Some(if harm == inverted { EffectWork::Heal(heal_amount()) } else { EffectWork::HurtMagic(harm_amount()) })
}

/// The effects a mob of this kind cannot take (`canBeAffected`): the
/// undead ignore poison and regeneration (`#ignores_poison_and_regen`),
/// spiders poison (`Spider.canBeAffected`).
pub fn can_be_affected(effect: MobEffect, undead: bool, spider: bool) -> bool {
    match effect {
        MobEffect::Poison => !undead && !spider,
        MobEffect::Regeneration => !undead,
        _ => true,
    }
}

/// `LivingEntity.heal`: only the living heal, up to their maximum.
pub fn heal(health: &mut f32, max_health: f32, amount: f32) {
    if *health > 0.0 {
        *health = (*health + amount).clamp(0.0, max_health);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stronger_shorter_effect_hides_the_longer_one_until_it_runs_out() {
        let mut effects = MobEffects::default();
        effects.add(EffectInstance::new(MobEffect::Speed, 100, 0));
        assert!(effects.add(EffectInstance::new(MobEffect::Speed, 3, 1)));
        let speed = effects.get(MobEffect::Speed).unwrap();
        assert_eq!((speed.amplifier, speed.duration), (1, 3));
        assert_eq!(speed.hidden.as_ref().unwrap().duration, 100);
        for _ in 0..3 {
            effects.tick(false);
        }
        let speed = effects.get(MobEffect::Speed).unwrap();
        assert_eq!((speed.amplifier, speed.duration), (0, 97));
        assert!(speed.hidden.is_none());
    }

    #[test]
    fn poison_hurts_every_25_ticks_and_instant_health_once() {
        let mut effects = MobEffects::default();
        effects.add(EffectInstance::new(MobEffect::Poison, 50, 0));
        effects.add(EffectInstance::new(MobEffect::InstantHealth, 1, 0));
        assert_eq!(effects.tick(false), vec![EffectWork::Poison, EffectWork::Heal(4.0)]);
        assert_eq!(EffectWork::Poison.resolve(1.0, 20.0), None);
        assert!(!effects.has(MobEffect::InstantHealth));
        let hurts = (0..49).map(|_| effects.tick(false).len()).sum::<usize>();
        assert_eq!(hurts, 1);
        assert!(!effects.has(MobEffect::Poison));
    }

    #[test]
    fn undead_are_healed_by_harming_and_ignore_poison() {
        assert_eq!(instant_work(MobEffect::InstantDamage, 0, true, 0.75), Some(EffectWork::Heal(3.0)));
        assert_eq!(instant_work(MobEffect::InstantDamage, 0, false, 0.75), Some(EffectWork::HurtMagic(5.0)));
        assert!(!can_be_affected(MobEffect::Poison, true, false));
        assert!(!can_be_affected(MobEffect::Poison, false, true));
    }

    #[test]
    fn speed_and_slowness_scale_movement_speed() {
        let mut effects = MobEffects::default();
        effects.add(EffectInstance::new(MobEffect::Speed, 100, 0));
        assert_eq!(effects.movement_speed(0.25), 0.25 * (1.0 + f64::from(0.2_f32)));
        effects.add(EffectInstance::new(MobEffect::Slowness, 100, 1));
        assert_eq!(effects.movement_speed(0.25), 0.25 * (1.0 + f64::from(0.2_f32)) * (1.0 + f64::from(-0.15_f32) * 2.0));
    }
}
