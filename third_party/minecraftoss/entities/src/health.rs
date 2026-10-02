//! Shared dry generic-damage and death-timer path from 26.3 LivingEntity.
//! Source armor absorption is a separate input step; effects, absorption
//! and attacker knockback remain separate layers. A death is kept as its
//! loot will see it (`LivingEntity.die`, `dropAllDeathLoot`) until the
//! entity world reports it.

use glam::DVec3;
use std::sync::atomic::{AtomicU64, Ordering};

/// Deaths in the order they happen, across every entity world: loot rolls
/// follow that order (each table draws from its own named sequence).
static DEATHS: AtomicU64 = AtomicU64::new(0);

/// `LivingEntity.resolvePlayerResponsibleForDamage`: a player's hit counts
/// toward the kill for this many ticks.
pub const PLAYER_MEMORY_TICKS: i32 = 100;

/// How a mob died, as `LivingEntity.die` hands it to its loot.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Death {
    /// When it died, relative to other deaths.
    pub order: u64,
    /// Where it died: its drops appear there (`Entity.spawnAtLocation`).
    pub position: DVec3,
    /// A player had hurt it recently (`lastHurtByPlayerMemoryTime > 0`).
    pub killed_by_player: bool,
    /// It was burning (`Entity.isOnFire`).
    pub on_fire: bool,
    /// The killing source's entity type (`DamageSource.getEntity`), and
    /// whether the source had a direct entity (`getDirectEntity`).
    pub attacker: Option<&'static str>,
    pub direct: bool,
    /// The charged creeper whose blast killed it (`Creeper.killedEntity`).
    pub charged_creeper: Option<u64>,
}

/// Pinned 26.3 `CombatRules.getDamageAfterAbsorb` for the no-enchantment
/// path. The caller supplies the target's effective armor and toughness.
pub fn damage_after_armor(damage: f32, armor: f32, toughness: f32) -> f32 {
    let toughness_factor = 2.0_f32 + toughness / 4.0;
    let effective = (armor - damage / toughness_factor).clamp(armor * 0.2, 20.0);
    let protection = effective / 25.0;
    damage * (1.0 - protection)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DamageState {
    pub cooldown_ticks: i32,
    pub last_hurt: f32,
    pub hurt_ticks: i32,
    /// Full hits taken, each one a `broadcastDamageEvent` for clients.
    pub hurts: u32,
    pub death_ticks: i32,
    pub dead: bool,
    /// `lastHurtByPlayerMemoryTime`.
    pub player_memory: i32,
    /// A death the world has not reported yet.
    pub death: Option<Death>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageResult {
    pub applied: bool,
    pub dealt: f32,
    pub died: bool,
    /// A full hit (`tookFullDamage`): outside the damage cooldown, so it
    /// knocks back and plays the hurt sound; inside, only the increase over
    /// the last hit lands.
    pub full: bool,
}

impl DamageState {
    /// LivingEntity.hurtServer for an unarmored, unbuffed mob with a generic
    /// damage source. The caller supplies max health from attributes.
    pub fn hurt_generic(&mut self, health: &mut f32, max_health: f32, amount: f32) -> DamageResult {
        self.hurt_absorbed(health, max_health, amount, |damage| damage)
    }

    /// `hurtServer` whose `actuallyHurt` absorbs what gets past the damage
    /// cooldown (`getDamageAfterMagicAbsorb`): the cooldown compares the
    /// damage as dealt, the health loses what `absorb` leaves of it (or of
    /// its increase over the last hit), and nothing at all when that is 0.
    pub fn hurt_absorbed(&mut self, health: &mut f32, max_health: f32, amount: f32, absorb: impl Fn(f32) -> f32) -> DamageResult {
        if self.dead || *health <= 0.0 {
            return DamageResult {
                applied: false,
                dealt: 0.0,
                died: false,
                full: false,
            };
        }
        let amount = amount.max(0.0);
        let full = self.cooldown_ticks <= 10;
        let dealt = if !full {
            if amount <= self.last_hurt {
                return DamageResult {
                    applied: false,
                    dealt: 0.0,
                    died: false,
                    full: false,
                };
            }
            let delta = amount - self.last_hurt;
            self.last_hurt = amount;
            delta
        } else {
            self.last_hurt = amount;
            self.cooldown_ticks = 20;
            self.hurt_ticks = 10;
            self.hurts = self.hurts.wrapping_add(1);
            amount
        };
        let dealt = absorb(dealt);
        if dealt != 0.0 {
            *health = (*health - dealt).clamp(0.0, max_health);
        }
        let died = *health <= 0.0;
        if died {
            self.dead = true;
            self.death = Some(Death {
                order: DEATHS.fetch_add(1, Ordering::Relaxed),
                killed_by_player: self.player_memory > 0,
                ..Death::default()
            });
        }
        DamageResult {
            applied: true,
            dealt,
            died,
            full,
        }
    }

    /// Where a death this hit just caused happened, and whether the mob
    /// was burning.
    pub fn place_death(&mut self, result: DamageResult, position: DVec3, on_fire: bool) {
        if let Some(death) = self.death.as_mut().filter(|_| result.died) {
            death.position = position;
            death.on_fire = on_fire;
        }
    }

    /// Who an applied hit came from (`resolveMobResponsibleForDamage`,
    /// `resolvePlayerResponsibleForDamage`): a player's hit is remembered,
    /// and a killing hit names its source's entity.
    pub fn credit(&mut self, result: DamageResult, attacker: &'static str, direct: bool) {
        if !result.applied {
            return;
        }
        let player = attacker == "minecraft:player";
        if player {
            self.player_memory = PLAYER_MEMORY_TICKS;
        }
        if let Some(death) = self.death.as_mut().filter(|_| result.died) {
            death.attacker = Some(attacker);
            death.direct = direct;
            death.killed_by_player |= player;
        }
    }

    /// LivingEntity.baseTick decrements damage cooldown and ticks death before
    /// the entity is removed at death tick 20, then forgets an old player hit.
    pub fn tick(&mut self) -> bool {
        if self.cooldown_ticks > 0 {
            self.cooldown_ticks -= 1;
        }
        if self.hurt_ticks > 0 {
            self.hurt_ticks -= 1;
        }
        if self.dead {
            self.death_ticks += 1;
        }
        if self.player_memory > 0 {
            self.player_memory -= 1;
        }
        self.death_ticks >= 20
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_zombie_arrow_armor_damage() {
        assert_eq!(
            damage_after_armor(4.0, 2.0, 0.0).to_bits(),
            3.936_f32.to_bits()
        );
    }

    #[test]
    fn cooldown_rejects_weaker_hits_and_applies_only_the_increase() {
        let (mut state, mut health) = (DamageState::default(), 10.0);
        assert_eq!(state.hurt_generic(&mut health, 10.0, 4.0).dealt, 4.0);
        state.tick();
        assert!(!state.hurt_generic(&mut health, 10.0, 2.0).applied);
        state.tick();
        assert_eq!(state.hurt_generic(&mut health, 10.0, 6.0).dealt, 2.0);
        assert_eq!(health, 4.0);
    }

    #[test]
    fn a_player_hit_counts_for_a_hundred_ticks() {
        let (mut state, mut health) = (DamageState::default(), 20.0);
        let hit = state.hurt_generic(&mut health, 20.0, 1.0);
        state.credit(hit, "minecraft:player", true);
        for _ in 0..99 {
            state.tick();
        }
        assert!(state.hurt_generic(&mut health, 20.0, 30.0).died);
        assert!(state.death.unwrap().killed_by_player);
        let (mut late, mut health) = (DamageState::default(), 20.0);
        let hit = late.hurt_generic(&mut health, 20.0, 1.0);
        late.credit(hit, "minecraft:player", true);
        for _ in 0..100 {
            late.tick();
        }
        assert!(late.hurt_generic(&mut health, 20.0, 30.0).died);
        assert!(!late.death.unwrap().killed_by_player);
    }

    #[test]
    fn fatal_hit_removes_after_twenty_death_ticks() {
        let (mut state, mut health) = (DamageState::default(), 10.0);
        assert!(state.hurt_generic(&mut health, 10.0, 12.0).died);
        for _ in 0..19 {
            assert!(!state.tick());
        }
        assert!(state.tick());
    }
}
