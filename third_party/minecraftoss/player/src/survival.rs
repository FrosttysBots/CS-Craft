//! Player survival state. Movement exhaustion follows 26.3 ServerPlayer;
//! food ticking follows FoodData and peaceful regeneration follows ServerPlayer.

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Difficulty {
    Peaceful,
    Easy,
    Normal,
    Hard,
}

impl Difficulty {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "peaceful" => Some(Self::Peaceful),
            "easy" => Some(Self::Easy),
            "normal" => Some(Self::Normal),
            "hard" => Some(Self::Hard),
            _ => None,
        }
    }
}

/// The damage type's 26.3 `DamageScaling` rule. The attacker condition is
/// evaluated by `DamageSource.scalesWithDifficulty` before `Player.hurtServer`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DamageScaling {
    Never,
    WhenCausedByLivingNonPlayer,
    Always,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum EffectKind {
    Hunger,
    Poison,
    Regeneration,
    Absorption,
    Resistance,
    FireResistance,
    Nausea,
    /// `MobEffects.SLOWNESS`: movement speed -15% a level.
    Slowness,
    /// `MobEffects.WEAKNESS`: attack damage -4 a level.
    Weakness,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectInstance {
    pub duration: u32,
    pub amplifier: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FoodData {
    pub level: u8,
    pub saturation: f32,
    pub exhaustion: f32,
    tick_timer: u32,
}

impl Default for FoodData {
    fn default() -> Self {
        Self {
            level: 20,
            saturation: 5.0,
            exhaustion: 0.0,
            tick_timer: 0,
        }
    }
}

impl FoodData {
    pub fn add_exhaustion(&mut self, amount: f32) {
        self.exhaustion = (self.exhaustion + amount).min(40.0);
    }

    pub fn eat(&mut self, nutrition: u8, saturation: f32) {
        self.level = self.level.saturating_add(nutrition).min(20);
        self.saturation = (self.saturation + saturation).clamp(0.0, self.level as f32);
    }

    pub fn tick(
        &mut self,
        health: &mut f32,
        max_health: f32,
        natural_regeneration: bool,
        difficulty: Difficulty,
    ) -> bool {
        let mut starvation_hit = false;
        if self.exhaustion > 4.0 {
            self.exhaustion -= 4.0;
            if self.saturation > 0.0 {
                self.saturation = (self.saturation - 1.0).max(0.0);
            } else if difficulty != Difficulty::Peaceful {
                self.level = self.level.saturating_sub(1);
            }
        }
        if natural_regeneration && self.saturation > 0.0 && *health < max_health && self.level >= 20
        {
            self.tick_timer += 1;
            if self.tick_timer >= 10 {
                let spent = self.saturation.min(6.0);
                *health = (*health + spent / 6.0).min(max_health);
                self.add_exhaustion(spent);
                self.tick_timer = 0;
            }
        } else if natural_regeneration && self.level >= 18 && *health < max_health {
            self.tick_timer += 1;
            if self.tick_timer >= 80 {
                *health = (*health + 1.0).min(max_health);
                self.add_exhaustion(6.0);
                self.tick_timer = 0;
            }
        } else if self.level == 0 {
            self.tick_timer += 1;
            if self.tick_timer >= 80 {
                let can_starve = match difficulty {
                    Difficulty::Peaceful | Difficulty::Easy => *health > 10.0,
                    Difficulty::Normal => *health > 1.0,
                    Difficulty::Hard => *health > 0.0,
                };
                if can_starve {
                    starvation_hit = true;
                }
                self.tick_timer = 0;
            }
        } else {
            self.tick_timer = 0;
        }
        starvation_hit
    }
}

/// The armor a hit meets: the `ARMOR` attribute's value and
/// `ARMOR_TOUGHNESS`, or none for damage that bypasses armor.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Armor {
    pub value: Option<(f32, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurvivalStatus {
    pub health: f32,
    pub max_health: f32,
    pub food: FoodData,
    pub experience_level: u32,
    pub experience_progress: f32,
    pub total_experience: u32,
    pub armor: u8,
    pub air: i32,
    pub max_air: i32,
    pub absorption: f32,
    pub fire_ticks: u32,
    damage_cooldown: u8,
    last_hurt: f32,
    effects: [Option<EffectInstance>; 9],
}

impl Default for SurvivalStatus {
    fn default() -> Self {
        Self {
            health: 20.0,
            max_health: 20.0,
            food: FoodData::default(),
            experience_level: 0,
            experience_progress: 0.0,
            total_experience: 0,
            armor: 0,
            air: 300,
            max_air: 300,
            absorption: 0.0,
            fire_ticks: 0,
            damage_cooldown: 0,
            last_hurt: 0.0,
            effects: [None; 9],
        }
    }
}

impl SurvivalStatus {
    pub fn damage_with_difficulty(
        &mut self,
        amount: f32,
        difficulty: Difficulty,
        scaling: DamageScaling,
        caused_by_living_non_player: bool,
    ) {
        let scales = match scaling {
            DamageScaling::Never => false,
            DamageScaling::WhenCausedByLivingNonPlayer => caused_by_living_non_player,
            DamageScaling::Always => true,
        };
        let amount = if scales {
            match difficulty {
                Difficulty::Peaceful => 0.0,
                Difficulty::Easy => (amount / 2.0 + 1.0).min(amount),
                Difficulty::Normal => amount,
                Difficulty::Hard => amount * 3.0 / 2.0,
            }
        } else {
            amount
        };
        if amount > 0.0 {
            self.damage(amount);
        }
    }

    pub fn effect(&self, kind: EffectKind) -> Option<EffectInstance> {
        self.effects[kind as usize]
    }

    /// The attack damage attribute's `ADD_VALUE` modifiers from effects:
    /// weakness's -4 a level (`MobEffects.WEAKNESS`).
    pub fn attack_damage_modifier(&self) -> f64 {
        self.effect(EffectKind::Weakness).map_or(0.0, |e| -4.0 * (f64::from(e.amplifier) + 1.0))
    }

    /// `LivingEntity.heal`: only the living heal, up to their maximum.
    pub fn heal(&mut self, amount: f32) {
        if self.health > 0.0 {
            self.health = (self.health + amount).clamp(0.0, self.max_health);
        }
    }

    pub fn add_effect(&mut self, kind: EffectKind, duration: u32, amplifier: u8) {
        let slot = &mut self.effects[kind as usize];
        if slot.is_none_or(|old| {
            amplifier > old.amplifier || (amplifier == old.amplifier && duration > old.duration)
        }) {
            *slot = Some(EffectInstance {
                duration,
                amplifier,
            });
        }
        if kind == EffectKind::Absorption {
            self.absorption = self.absorption.max(4.0 * (amplifier as f32 + 1.0));
        }
    }

    pub fn remove_effect(&mut self, kind: EffectKind) {
        self.effects[kind as usize] = None;
        if kind == EffectKind::Absorption {
            self.absorption = 0.0;
        }
    }

    pub fn damage(&mut self, amount: f32) {
        self.damage_internal(amount, false);
    }

    /// `LivingEntity.hurtServer`'s damage cooldown around
    /// `Player.actuallyHurt`: within the cooldown's second half only the
    /// part above the last hit lands. `None` when the cooldown turns the
    /// hit away; otherwise whether it landed in full (knockback and the
    /// hurt animation follow a full hit).
    pub fn hurt(&mut self, damage: f32, armor: Armor, exhaustion: f32) -> Option<bool> {
        if self.health <= 0.0 {
            return None;
        }
        let damage = damage.max(0.0);
        if f32::from(self.damage_cooldown) > 10.0 {
            if damage <= self.last_hurt {
                return None;
            }
            self.actually_hurt(damage - self.last_hurt, armor, exhaustion);
            self.last_hurt = damage;
            Some(false)
        } else {
            self.last_hurt = damage;
            self.damage_cooldown = 20;
            self.actually_hurt(damage, armor, exhaustion);
            Some(true)
        }
    }

    /// `Player.actuallyHurt`: armor (`CombatRules.getDamageAfterAbsorb`),
    /// resistance, absorption, then health and the damage type's food
    /// exhaustion.
    fn actually_hurt(&mut self, mut damage: f32, armor: Armor, exhaustion: f32) {
        if let Some(armor) = armor.value {
            let toughness = 2.0 + armor.1 / 4.0;
            let real = (armor.0 - damage / toughness).clamp(armor.0 * 0.2, 20.0);
            damage *= 1.0 - real / 25.0;
        }
        if let Some(effect) = self.effect(EffectKind::Resistance) {
            let absorb = 25 - (i32::from(effect.amplifier) + 1) * 5;
            damage = (damage * absorb as f32 / 25.0).max(0.0);
        }
        if damage <= 0.0 {
            damage = 0.0;
        }
        let original = damage;
        damage = (damage - self.absorption).max(0.0);
        self.absorption = (self.absorption - (original - damage)).max(0.0);
        if damage != 0.0 {
            self.food.add_exhaustion(exhaustion);
            self.health = (self.health - damage).clamp(0.0, self.max_health);
            self.absorption = (self.absorption - damage).max(0.0);
        }
    }

    fn damage_internal(&mut self, amount: f32, bypass_effects: bool) {
        if self.health <= 0.0 {
            return;
        }
        let amount = if self.damage_cooldown > 10 {
            if amount <= self.last_hurt {
                return;
            }
            let difference = amount - self.last_hurt;
            self.last_hurt = amount;
            difference
        } else {
            self.last_hurt = amount;
            self.damage_cooldown = 20;
            amount
        };
        let resistance = if bypass_effects {
            0.0
        } else {
            self.effect(EffectKind::Resistance)
                .map_or(0.0, |effect| (effect.amplifier as f32 + 1.0) * 0.2)
        };
        let mut remaining = amount * (1.0 - resistance).max(0.0);
        let absorbed = remaining.min(self.absorption);
        self.absorption -= absorbed;
        remaining -= absorbed;
        self.health = (self.health - remaining).max(0.0);
    }

    /// MobEffectInstance checks the remaining duration before decrementing.
    pub fn tick_effects(&mut self) {
        self.damage_cooldown = self.damage_cooldown.saturating_sub(1);
        for kind in [
            EffectKind::Hunger,
            EffectKind::Poison,
            EffectKind::Regeneration,
            EffectKind::Absorption,
            EffectKind::Resistance,
            EffectKind::FireResistance,
            EffectKind::Nausea,
            EffectKind::Slowness,
            EffectKind::Weakness,
        ] {
            let Some(mut effect) = self.effect(kind) else {
                continue;
            };
            match kind {
                EffectKind::Hunger => self
                    .food
                    .add_exhaustion(0.005 * (effect.amplifier as f32 + 1.0)),
                EffectKind::Poison if self.health > 1.0 => {
                    let interval = (25_u32 >> effect.amplifier.min(31)).max(1);
                    if effect.duration % interval == 0 {
                        self.damage(1.0);
                    }
                }
                EffectKind::Regeneration => {
                    let interval = (50_u32 >> effect.amplifier.min(31)).max(1);
                    if effect.duration % interval == 0 {
                        self.health = (self.health + 1.0).min(self.max_health);
                    }
                }
                _ => {}
            }
            effect.duration -= 1;
            if effect.duration == 0 {
                self.remove_effect(kind);
            } else {
                self.effects[kind as usize] = Some(effect);
            }
        }
    }

    /// Entity.baseTick applies fire once per 20 remaining ticks outside lava.
    /// Contact with water extinguishes it; lava contact itself is applied by
    /// the movement caller after block intersection.
    pub fn tick_fire(&mut self, in_lava: bool, in_water: bool) {
        if in_water {
            self.fire_ticks = 0;
            return;
        }
        if self.fire_ticks > 0 {
            if self.fire_ticks % 20 == 0
                && !in_lava
                && self.effect(EffectKind::FireResistance).is_none()
            {
                self.damage(1.0);
            }
            self.fire_ticks -= 1;
        }
    }

    pub fn touch_lava(&mut self) {
        self.fire_ticks = self.fire_ticks.max(300);
        if self.effect(EffectKind::FireResistance).is_none() {
            self.damage(4.0);
        }
    }

    /// LivingEntity.baseTick consumes one air per submerged tick. Without
    /// oxygen-bonus or water-breathing effects, damage lands at -20 air and
    /// resets air to zero; surfacing restores four air per tick.
    pub fn tick_air(&mut self, eye_submerged: bool, invulnerable: bool) -> bool {
        if eye_submerged && !invulnerable {
            self.air -= 1;
            if self.air <= -20 {
                self.air = 0;
                self.damage(2.0);
                return true;
            }
        } else if self.air < self.max_air {
            self.air = (self.air + 4).min(self.max_air);
        }
        false
    }

    pub fn tick_normal(&mut self) {
        self.tick_with_rules(true, Difficulty::Normal, 0);
    }

    pub fn tick_normal_with_regen(&mut self, natural_regeneration: bool) {
        self.tick_with_rules(natural_regeneration, Difficulty::Normal, 0);
    }

    pub fn tick_with_rules(
        &mut self,
        natural_regeneration: bool,
        difficulty: Difficulty,
        player_tick: u64,
    ) {
        if natural_regeneration && difficulty == Difficulty::Peaceful {
            if player_tick % 20 == 0 {
                self.health = (self.health + 1.0).min(self.max_health);
                self.food.saturation = (self.food.saturation + 1.0).min(20.0);
            }
            if player_tick % 10 == 0 {
                self.food.level = self.food.level.saturating_add(1).min(20);
            }
        }
        let starvation_hit = self.food.tick(
            &mut self.health,
            self.max_health,
            natural_regeneration,
            difficulty,
        );
        if starvation_hit {
            // The default starve damage type bypasses armor and effects, but
            // still enters LivingEntity's hurt cooldown and absorption path.
            self.damage_internal(1.0, true);
        }
    }

    pub fn experience_to_next_level(&self) -> u32 {
        match self.experience_level {
            0..=14 => 7 + self.experience_level * 2,
            15..=29 => 37 + (self.experience_level - 15) * 5,
            level => 112 + (level - 30) * 9,
        }
    }

    pub fn give_experience_points(&mut self, points: u32) {
        self.experience_progress += points as f32 / self.experience_to_next_level() as f32;
        self.total_experience = self.total_experience.saturating_add(points);
        while self.experience_progress >= 1.0 {
            self.experience_progress =
                (self.experience_progress - 1.0) * self.experience_to_next_level() as f32;
            self.experience_level = self.experience_level.saturating_add(1);
            self.experience_progress /= self.experience_to_next_level() as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhaustion_spends_saturation_then_food_one_point_per_tick() {
        let mut food = FoodData::default();
        let mut health = 20.0;
        food.add_exhaustion(8.1);
        food.tick(&mut health, 20.0, true, Difficulty::Normal);
        assert_eq!((food.saturation, food.level), (4.0, 20));
        food.tick(&mut health, 20.0, true, Difficulty::Normal);
        assert_eq!((food.saturation, food.level), (3.0, 20));
        food.saturation = 0.0;
        food.add_exhaustion(4.0);
        food.tick(&mut health, 20.0, true, Difficulty::Normal);
        assert_eq!(food.level, 19);
    }

    #[test]
    fn experience_crosses_vanilla_level_thresholds() {
        let mut status = SurvivalStatus::default();
        status.give_experience_points(7);
        assert_eq!(status.experience_level, 1);
        assert_eq!(status.experience_progress, 0.0);
        status.give_experience_points(4);
        assert_eq!(status.experience_level, 1);
        assert!((status.experience_progress - 4.0 / 9.0).abs() < 1e-6);
    }

    #[test]
    fn underwater_air_damages_every_twenty_ticks_and_recovers_by_four() {
        let mut status = SurvivalStatus::default();
        for _ in 0..319 {
            status.tick_effects();
            assert!(!status.tick_air(true, false));
        }
        assert_eq!(status.air, -19);
        assert_eq!(status.health, 20.0);
        status.tick_effects();
        assert!(status.tick_air(true, false));
        assert_eq!((status.air, status.health), (0, 18.0));
        for _ in 0..19 {
            status.tick_effects();
            assert!(!status.tick_air(true, false));
        }
        status.tick_effects();
        assert!(status.tick_air(true, false));
        assert_eq!(status.health, 16.0);
        assert!(!status.tick_air(false, false));
        assert_eq!(status.air, 4);
    }

    #[test]
    fn enclosed_water_client_trace_repeats_the_air_cycle() {
        // Two isolated 26.3 client runs of scenarios/survival/drown.json had
        // identical air at every one of 360 recorded ticks. The setup command
        // had already consumed 20 air before tick zero.
        let mut status = SurvivalStatus::default();
        status.air = 280;
        for tick in 0..360 {
            status.tick_air(true, false);
            if let Some(expected) = match tick {
                0 => Some(279),
                279 | 299 | 319 | 339 | 359 => Some(0),
                280 => Some(-1),
                298 | 318 | 338 | 358 => Some(-19),
                _ => None,
            } {
                assert_eq!(status.air, expected, "client tick {tick}");
            }
        }
    }

    #[test]
    fn disabled_natural_regeneration_allows_drowning_to_kill() {
        let mut status = SurvivalStatus::default();
        status.air = 280;
        let mut fatal_tick = None;
        for tick in 0..500 {
            status.tick_effects();
            status.tick_air(true, false);
            status.tick_normal_with_regen(false);
            if status.health == 0.0 {
                fatal_tick = Some(tick);
                break;
            }
        }
        assert_eq!(fatal_tick, Some(479));
        assert_eq!(status.food.level, 20);
        assert_eq!(status.food.saturation, 5.0);
    }

    #[test]
    fn peaceful_refills_food_and_heals_while_normal_regeneration_spends_hunger() {
        let mut peaceful = SurvivalStatus::default();
        peaceful.health = 2.0;
        peaceful.food.saturation = 0.0;
        peaceful.food.exhaustion = 40.0;
        for tick in 1..=500 {
            peaceful.tick_with_rules(true, Difficulty::Peaceful, tick);
        }
        assert_eq!(peaceful.food.level, 20);
        assert_eq!(peaceful.health, 20.0);

        let mut normal = SurvivalStatus::default();
        normal.health = 2.0;
        normal.food.saturation = 0.0;
        for tick in 1..=500 {
            normal.tick_with_rules(true, Difficulty::Normal, tick);
        }
        assert!(normal.food.level < 18);
        assert!(normal.health < 20.0);
        let stopped_at = normal.health;
        for tick in 501..=650 {
            normal.tick_with_rules(true, Difficulty::Normal, tick);
        }
        assert_eq!(normal.health, stopped_at);
    }

    #[test]
    fn starvation_respects_easy_normal_and_hard_health_floors() {
        for (difficulty, expected) in [
            (Difficulty::Easy, 10.0),
            (Difficulty::Normal, 1.0),
            (Difficulty::Hard, 0.0),
        ] {
            let mut status = SurvivalStatus::default();
            status.health = 12.0;
            status.food.level = 0;
            status.food.saturation = 0.0;
            for tick in 1..=1000 {
                status.tick_effects();
                status.tick_with_rules(false, difficulty, tick);
            }
            assert_eq!(status.health, expected, "{difficulty:?}");
        }
    }

    #[test]
    fn starvation_uses_hurt_cooldown_and_bypasses_resistance_before_absorption() {
        let mut status = SurvivalStatus::default();
        status.health = 12.0;
        status.food.level = 0;
        status.food.saturation = 0.0;
        status.add_effect(EffectKind::Resistance, 200, 4);
        status.add_effect(EffectKind::Absorption, 200, 0);
        for tick in 1..=80 {
            status.tick_effects();
            status.tick_with_rules(false, Difficulty::Hard, tick);
        }
        assert_eq!(status.health, 12.0);
        assert_eq!(status.absorption, 3.0);

        let mut recently_hurt = SurvivalStatus::default();
        recently_hurt.health = 12.0;
        recently_hurt.food.level = 0;
        recently_hurt.food.saturation = 0.0;
        for tick in 1..=80 {
            recently_hurt.tick_effects();
            if tick == 75 {
                recently_hurt.damage(2.0);
            }
            recently_hurt.tick_with_rules(false, Difficulty::Hard, tick);
        }
        assert_eq!(recently_hurt.health, 10.0);
    }

    #[test]
    fn all_difficulties_match_repeated_26_3_server_food_traces_every_tick() {
        // Each fixture row was observed at END_SERVER_TICK after an explicit
        // START_SERVER_TICK setup. The exporter refuses non-identical vanilla
        // repeats before this CSV can be regenerated.
        let reference = include_str!("../../../scenarios/survival/difficulty-reference.csv");
        let mut current_profile = "";
        let mut status = SurvivalStatus::default();
        for row in reference.lines().skip(1) {
            let fields: Vec<_> = row.split(',').collect();
            assert_eq!(fields.len(), 7);
            let profile = fields[0];
            if profile != current_profile {
                current_profile = profile;
                status = SurvivalStatus::default();
                status.health = if profile.ends_with("_starve") {
                    2.0
                } else {
                    12.0
                };
                status.food.level = if profile.ends_with("_starve") { 0 } else { 20 };
                status.food.saturation = 0.0;
            }
            let tick: u64 = fields[1].parse().unwrap();
            let player_tick: u64 = fields[2].parse().unwrap();
            assert_eq!(player_tick, tick + 1, "{profile} tick {tick}");
            let difficulty = Difficulty::parse(profile.split('_').next().unwrap()).unwrap();
            status.tick_effects();
            status.tick_with_rules(true, difficulty, player_tick);
            assert_eq!(
                (status.health as f64).to_bits(),
                u64::from_str_radix(fields[3], 16).unwrap(),
                "{profile} tick {tick}: health"
            );
            assert_eq!(
                status.food.level,
                fields[4].parse::<u8>().unwrap(),
                "{profile} tick {tick}: food"
            );
            assert_eq!(
                (status.food.saturation as f64).to_bits(),
                u64::from_str_radix(fields[5], 16).unwrap(),
                "{profile} tick {tick}: saturation"
            );
            assert_eq!(
                status.health <= 0.0,
                fields[6] == "1",
                "{profile} tick {tick}: death"
            );
        }
    }

    #[test]
    fn difficulty_scales_mob_damage_but_not_fall_damage() {
        // 26.3 integrated-server probes: six base damage, player at twenty
        // health, hit after the initial join protection expires.
        for (difficulty, mob_health) in [
            (Difficulty::Peaceful, 20.0),
            (Difficulty::Easy, 16.0),
            (Difficulty::Normal, 14.0),
            (Difficulty::Hard, 11.0),
        ] {
            let mut mob = SurvivalStatus::default();
            mob.damage_with_difficulty(
                6.0,
                difficulty,
                DamageScaling::WhenCausedByLivingNonPlayer,
                true,
            );
            assert_eq!(mob.health, mob_health, "{difficulty:?} mob attack");
            if difficulty == Difficulty::Peaceful {
                // Zero damage must not start the ordinary hurt cooldown.
                mob.damage_with_difficulty(
                    6.0,
                    difficulty,
                    DamageScaling::WhenCausedByLivingNonPlayer,
                    false,
                );
                assert_eq!(mob.health, 14.0);
            }
            let mut fall = SurvivalStatus::default();
            fall.damage_with_difficulty(
                6.0,
                difficulty,
                DamageScaling::WhenCausedByLivingNonPlayer,
                false,
            );
            assert_eq!(fall.health, 14.0, "{difficulty:?} fall");
        }
        let mut player_attacker = SurvivalStatus::default();
        player_attacker.damage_with_difficulty(
            6.0,
            Difficulty::Hard,
            DamageScaling::WhenCausedByLivingNonPlayer,
            false,
        );
        assert_eq!(player_attacker.health, 14.0);
    }

    #[test]
    fn all_difficulties_match_repeated_26_3_server_damage_traces() {
        let reference = include_str!("../../../scenarios/survival/difficulty-damage-reference.csv");
        let mut current_profile = "";
        let mut status = SurvivalStatus::default();
        for row in reference.lines().skip(1) {
            let fields: Vec<_> = row.split(',').collect();
            assert_eq!(fields.len(), 7);
            let profile = fields[0];
            let difficulty = Difficulty::parse(profile.split('_').next().unwrap()).unwrap();
            if profile != current_profile {
                current_profile = profile;
                status = SurvivalStatus::default();
                let mob = profile.ends_with("_mob_damage");
                status.damage_with_difficulty(
                    6.0,
                    difficulty,
                    DamageScaling::WhenCausedByLivingNonPlayer,
                    mob,
                );
            }
            let tick: u64 = fields[1].parse().unwrap();
            let player_tick: u64 = fields[2].parse().unwrap();
            status.tick_effects();
            status.tick_with_rules(true, difficulty, player_tick);
            assert_eq!(
                (status.health as f64).to_bits(),
                u64::from_str_radix(fields[3], 16).unwrap(),
                "{profile} tick {tick}: health"
            );
            assert_eq!(
                status.food.level,
                fields[4].parse::<u8>().unwrap(),
                "{profile} tick {tick}: food"
            );
            assert_eq!(
                (status.food.saturation as f64).to_bits(),
                u64::from_str_radix(fields[5], 16).unwrap(),
                "{profile} tick {tick}: saturation"
            );
            assert_eq!(
                status.health <= 0.0,
                fields[6] == "1",
                "{profile} tick {tick}: death"
            );
        }
    }

    #[test]
    fn lava_damage_obeys_living_entity_cooldown_and_water_extinguishes_fire() {
        let mut status = SurvivalStatus::default();
        status.touch_lava();
        assert_eq!((status.health, status.fire_ticks), (16.0, 300));
        for _ in 0..9 {
            status.tick_effects();
            status.tick_fire(true, false);
            status.touch_lava();
        }
        assert_eq!(status.health, 16.0);
        status.tick_effects();
        status.tick_fire(true, false);
        status.touch_lava();
        assert_eq!(status.health, 12.0);
        status.tick_fire(false, true);
        assert_eq!(status.fire_ticks, 0);
        status.add_effect(EffectKind::FireResistance, 100, 0);
        status.touch_lava();
        assert_eq!(status.health, 12.0);
    }

    #[test]
    fn measured_food_effects_tick_at_remaining_duration_intervals() {
        let mut status = SurvivalStatus::default();
        status.health = 10.0;
        status.add_effect(EffectKind::Regeneration, 100, 1);
        status.add_effect(EffectKind::Hunger, 2, 2);
        status.tick_effects();
        assert_eq!(status.health, 11.0);
        assert!((status.food.exhaustion - 0.015).abs() < 1e-6);
        assert_eq!(
            status.effect(EffectKind::Regeneration).unwrap().duration,
            99
        );
        status.tick_effects();
        assert_eq!(status.effect(EffectKind::Hunger), None);
        for _ in 0..24 {
            status.tick_effects();
        }
        assert_eq!(status.health, 12.0);
    }

    #[test]
    fn poison_stops_at_one_health_point_and_absorption_precedes_resisted_damage() {
        let mut status = SurvivalStatus::default();
        status.health = 2.0;
        status.add_effect(EffectKind::Poison, 25, 0);
        status.tick_effects();
        assert_eq!(status.health, 1.0);
        for _ in 0..24 {
            status.tick_effects();
        }
        assert_eq!(status.health, 1.0);
        status.add_effect(EffectKind::Resistance, 100, 0);
        status.add_effect(EffectKind::Absorption, 100, 0);
        status.damage(5.0);
        assert_eq!((status.absorption, status.health), (0.0, 1.0));
    }
}
