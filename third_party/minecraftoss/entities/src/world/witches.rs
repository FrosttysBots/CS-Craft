//! Witches (26.3 `Witch`) and their thrown splash potions in the entity
//! world. A witch ticks as the other monsters do, with its own `aiStep`
//! first: its raider-healing cooldown runs down, a potion it drinks takes
//! effect when its use time runs out, and otherwise it may start drinking
//! one (a float from its random for each wish in turn, a sound, standing
//! still while it drinks); a float in 1333 or so asks the client for its
//! particles. Its ranged attack throws a splash potion from its eyes, whose
//! random (pinned by the harness) spreads its aim; the potion ticks from
//! the next game tick and breaks on the first block or mob in its way.
//! Every living thing near the break takes its effect (`splash`).
use super::emissions::{base_tick_fluid, play_movement, MovementSounds};
use super::*;
use crate::effects::{can_be_affected, heal, instant_work, EffectInstance, EffectWork, MobEffect, MobEffects};
use crate::monster_ai::{MonsterKind, PlayerVitals};
use crate::potion::{splash_scale, splashed_effect, PotionHit, PotionTarget, ThrownPotion};
use crate::witch::{self, Throw, Witch};

/// A witch steps with its block's sound; it is a `Monster`.
const WITCH_SOUNDS: MovementSounds = MovementSounds::monster(None);

/// A witch on the monster goal framework.
#[derive(Clone)]
pub struct WitchEntity {
    pub id: u64,
    pub witch: Witch,
    pub no_ai: bool,
    pub tick_count: i32,
    pub ambient_sound_time: i32,
    /// Sounds it made since the world last collected them.
    pub voices: Vec<(Voice, DVec3)>,
    pub no_action_time: i32,
    pub random: LegacyRandom,
    pub previous_position: DVec3,
    pub ai: Box<MonsterAi>,
}

/// A splash potion in flight.
#[derive(Clone)]
pub struct PotionEntity {
    pub id: u64,
    pub potion: ThrownPotion,
    /// Where it was a tick ago, for drawing between ticks.
    pub previous_position: DVec3,
}

/// Where a splash potion broke (`AbstractThrownPotion.onHit`'s level
/// events: the break sound and the client's particles in its colour, the
/// instant kind's for an instant potion).
#[derive(Clone, Copy, Debug)]
pub struct PotionBreak {
    pub position: DVec3,
    pub color: u32,
    pub instant: bool,
}

/// A potion breaking near a player: its lasting effect for the player's
/// side (with its level and ticks), or its instant heal or harm, and where
/// it broke.
#[derive(Clone, Copy, Debug)]
pub struct PlayerSplash {
    pub effect: MobEffect,
    pub amplifier: i32,
    pub duration: i32,
    /// The instant effect's amount (`applyInstantaneousEffect`).
    pub instant: Option<EffectWork>,
    pub at: DVec3,
}

impl WitchEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.witch.body.position
    }

    pub fn set_random_seed(&mut self, seed: u64) {
        self.random = LegacyRandom::new(seed);
    }

    /// `hurtServer` on a witch: `resisted` for `#witch_resistant_to`
    /// damage (magic), `own` for its own potions (`getEntity() == this`).
    pub fn hurt(&mut self, amount: f32, resisted: bool, own: bool) -> DamageResult {
        if self.witch.health > 0.0 && !self.witch.damage.dead {
            self.no_action_time = 0;
        }
        let result = self.witch.damage.hurt_absorbed(&mut self.witch.health, witch::MAX_HEALTH, amount, |damage| witch::absorb(damage, own, resisted));
        let position = self.position();
        self.witch.damage.place_death(result, position, self.witch.body.fire_ticks > 0);
        // Only a full hit plays the hurt or death sound (`tookFullDamage`).
        if result.applied && result.full {
            if !result.died {
                self.ambient_sound_time = -80;
            }
            let voice = hurt_voice(&mut self.random, result.died, false);
            self.voices.push((voice, position));
        }
        result
    }

    /// `Witch.aiStep` before `super.aiStep`: the drink it finishes or the
    /// one it starts, then the particle roll.
    fn drink(&mut self, world: &impl World, game_time: i64, target_distance_sq: Option<f64>) {
        self.witch.heal_cooldown -= 1;
        let position = self.witch.body.position;
        if let Some(potion) = self.witch.drinking {
            let left = self.witch.using_time;
            self.witch.using_time -= 1;
            if left <= 0 {
                self.witch.drinking = None;
                // `forEachEffect` with the duration scaled by 1
                // (`withScaledDuration`: at least a tick).
                let mut effect = potion.effect();
                effect.duration = effect.map_duration(|d| ((d as f32 * 1.0).floor() as i32).max(1));
                self.witch.effects.add(effect);
            }
        } else {
            let eye_in_water = FluidFrame::eye_in_water(world, position, witch::EYE_HEIGHT);
            if let Some(potion) = self.witch.choose_drink(&mut self.random, eye_in_water, game_time, target_distance_sq) {
                self.witch.drinking = Some(potion);
                self.witch.using_time = witch::DRINK_TICKS;
                let pitch = 0.8 + self.random.next_float() * 0.4;
                self.voices.push((Voice::Event("entity.witch.drink", 1.0, pitch), position));
            }
        }
        // Entity event 15: the client's witch particles.
        let _ = self.random.next_float() < 7.5e-4;
    }

    /// `Witch.aiStep` and `LivingEntity.aiStep` for an active witch: the
    /// drinking, then `Mob.serverAiStep` (goals, navigation, controls),
    /// the jump and travel. Returns a potion it threw and where from.
    #[allow(clippy::too_many_arguments)]
    fn tick_ai(&mut self, world: &impl World, players: &[PlayerCandidate], vitals: &[(u64, PlayerVitals)], game_time: i64, difficulty: i32, bright_outside: bool) -> Option<(Throw, DVec3)> {
        let position = self.witch.body.position;
        // `Mob.checkDespawn`: a player within 32 blocks resets the idle clock.
        if players.iter().any(|p| p.alive && !p.spectator && p.position.distance_squared(position) < 32.0 * 32.0) {
            self.no_action_time = 0;
        }
        let health = self.witch.health;
        {
            let state = &mut self.ai.state;
            state.body = self.witch.body.clone();
            state.health = health;
            state.max_health = witch::MAX_HEALTH;
            state.game_time = game_time;
            state.difficulty = difficulty;
            state.bright_outside = bright_outside;
            state.players = players.to_vec();
            state.villagers = Vec::new();
            state.vitals = vitals.to_vec();
            state.tick_count = self.tick_count;
        }
        let target_distance_sq = self.ai.state.target().map(|t| t.position.distance_squared(position));
        self.drink(world, game_time, target_distance_sq);
        // `MOVEMENT_SPEED` with the drinking modifier and effects.
        let speed = self.witch.movement_speed();
        let drinking = self.witch.drinking.is_some();
        let ai = &mut self.ai;
        let body = &mut self.witch.body;
        ai.state.witch.drinking = drinking;
        ai.state.random = std::mem::take(&mut self.random);
        // `Raider.updateNoActionTime`: the idle clock runs double, light or
        // dark.
        self.no_action_time += 2;
        if ai.no_jump_delay > 0 {
            ai.no_jump_delay -= 1;
        }
        body.trim_small_velocity();
        ai.state.body.velocity = body.velocity;
        self.no_action_time += 1;
        let fluid = FluidFrame::sample(world, position, body.width, body.height);
        ai.state.fluid = fluid;
        ai.state.no_action_time = self.no_action_time;
        let full = self.tick_count <= 1 || (self.tick_count + self.id as i32) % 2 == 0;
        ai.tick_goals(world, full);
        let thrown = ai.state.witch.throw.take();
        if let Some((_, pitch)) = thrown {
            self.voices.push((Voice::Event("entity.witch.throw", 1.0, pitch), position));
        }
        let position = body.position;
        let (can_update, surface) = crate::navigation::ground_view(world, body, fluid, ai.state.walk.can_float);
        if let Some((wanted, speed)) = ai.state.navigation.tick_in(world, position, can_update, surface, body.width, ai.speed) {
            ai.move_control.set_wanted_position(wanted, speed);
        }
        self.random = std::mem::take(&mut ai.state.random);
        // `MoveControl`'s obstacle, as for the other monsters.
        let feet = (position.x.floor() as i32, position.y.floor() as i32, position.z.floor() as i32);
        let obstacle_top = world
            .block(feet)
            .filter(|b| !b.id.ends_with("_door") && !b.id.ends_with("_fence"))
            .and_then(|_| world.collision_boxes(feet).iter().map(|b| b[4]).reduce(f64::max))
            .map(|top| top + f64::from(feet.1));
        let control = ai.move_control.tick(position, body.on_ground, ai.yaw, ai.speed, ai.forward, ai.sideways, speed, body.width, body.step_height, obstacle_top, |_, _| true);
        ai.yaw = control.yaw;
        ai.speed = control.speed;
        ai.forward = control.forward;
        ai.sideways = control.sideways;
        ai.state.look_control.tick(position, witch::EYE_HEIGHT, ai.body_rotation.body_yaw, !ai.state.navigation.is_done());
        ai.jumping = ai.state.jump || control.jump;
        body.living_jump(world, fluid, ai.jumping, &mut ai.no_jump_delay, 0.4);
        let input = DVec3::new(f64::from(ai.sideways), 0.0, f64::from(ai.forward));
        let mut landed = None;
        if fluid.in_water() {
            body.travel_water(world, input, ai.yaw);
        } else if fluid.in_lava() {
            body.travel_lava(world, input, ai.yaw, fluid.lava_height);
        } else {
            landed = body.travel_air_jumping(world, input, ai.speed, ai.yaw, ai.jumping);
        }
        play_movement(body, self.tick_count, &mut self.random, &mut self.voices, WITCH_SOUNDS);
        ai.body_rotation.tick(ai.yaw, &mut ai.state.look_control, self.previous_position, body.position);
        self.witch.yaw = ai.yaw;
        if let Some(damage) = landed.and_then(|fallen| super::living::fall_damage(&self.witch.body, world, fallen, true, &mut self.voices)) {
            self.hurt(damage, false, false);
        }
        thrown.map(|(throw, _)| (throw, position))
    }
}

/// `Mob.getVoicePitch` for an adult.
fn adult_pitch(random: &mut LegacyRandom) -> f32 {
    let (a, b) = (random.next_float(), random.next_float());
    (a - b) * 0.2 + 1.0
}

impl EntityWorld {
    /// A witch with its goals, facing its yaw (a NoAI one keeps still but
    /// still sounds and takes hits).
    pub fn spawn_witch(&mut self, witch: Witch, no_ai: bool) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let ai = MonsterAi::of_kind(MonsterKind::Witch, &witch.body, witch.yaw);
        self.witches.push(WitchEntity {
            id,
            previous_position: witch.body.position,
            witch,
            no_ai,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            random: LegacyRandom::new(0),
            ai: Box::new(ai),
        });
        self.order.push(EntityKey::Witch(id));
        self.file_in_section(EntityKey::Witch(id));
        id
    }

    pub fn witches(&self) -> &[WitchEntity] {
        &self.witches
    }

    pub fn witch_mut(&mut self, id: u64) -> Option<&mut WitchEntity> {
        self.witches.iter_mut().find(|entity| entity.id == id)
    }

    /// The splash potions in flight.
    pub fn potions(&self) -> &[PotionEntity] {
        &self.potions
    }

    /// A potion already in flight (a saved one).
    pub fn spawn_potion(&mut self, potion: ThrownPotion) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let previous_position = potion.position;
        self.potions.push(PotionEntity { id, potion, previous_position });
        self.order.push(EntityKey::Potion(id));
        id
    }

    /// The players' health, effects and motion, for the monsters that
    /// read them (full health, still and unaffected when unset).
    pub fn set_player_vitals(&mut self, vitals: Vec<(u64, PlayerVitals)>) {
        self.player_vitals = vitals;
    }

    /// Splashes on players since the last call (only when players are
    /// pickable: the harness's probes are not in the entity lookup).
    pub fn take_player_splashes(&mut self) -> Vec<(u64, PlayerSplash)> {
        std::mem::take(&mut self.player_splashes)
    }

    /// Where potions broke since the last call.
    pub fn take_potion_breaks(&mut self) -> Vec<PotionBreak> {
        std::mem::take(&mut self.potion_breaks)
    }

    /// One witch's tick: `Entity.baseTick` (water, a splash),
    /// `LivingEntity.baseTick` (its air, the hurt and death clocks, the
    /// forgotten attacker, its effects), the ambient roll, then its AI
    /// while alive, and the potion it threw.
    pub(super) fn tick_witch(&mut self, id: u64, world: &mut impl World, players: &[PlayerCandidate], ticks: &dyn Fn(DVec3) -> bool) {
        let (game_time, difficulty, bright_outside) = (self.game_time, self.difficulty, self.bright_outside);
        let vitals = self.player_vitals.clone();
        let mobs = self.mob_candidates();
        let Some(entity) = self.witches.iter_mut().find(|e| e.id == id) else { return };
        entity.ai.state.mobs = mobs;
        entity.previous_position = entity.witch.body.position;
        entity.tick_count += 1;
        base_tick_fluid(&mut entity.witch.body, &*world, entity.tick_count == 1, &mut entity.random, &mut entity.voices, WITCH_SOUNDS);
        super::hazards::burn(entity, &*world, game_time);
        super::hazards::suffocate(entity, &*world, witch::EYE_HEIGHT, game_time);
        let water_breathing = entity.witch.effects.has(MobEffect::WaterBreathing);
        if entity.witch.health > 0.0 && super::living::breathe(&mut entity.witch.body, &*world, witch::EYE_HEIGHT, water_breathing) {
            entity.hurt(2.0, false, false);
        }
        let removed = entity.witch.damage.tick();
        // `LivingEntity.baseTick` forgets an attacker after 100 ticks.
        if entity.ai.state.hurt_by.is_some_and(|(_, when)| entity.tick_count - when > 100) {
            entity.ai.state.hurt_by = None;
        }
        for work in entity.witch.effects.tick(false) {
            match work.resolve(entity.witch.health, witch::MAX_HEALTH) {
                Some(EffectWork::Heal(amount)) => heal(&mut entity.witch.health, witch::MAX_HEALTH, amount),
                Some(EffectWork::HurtMagic(amount)) => {
                    entity.hurt(amount, true, false);
                }
                _ => {}
            }
        }
        if entity.witch.health > 0.0 {
            if (entity.random.next_int(1000) as i32) < entity.ambient_sound_time {
                entity.ambient_sound_time = -80;
                let pitch = adult_pitch(&mut entity.random);
                entity.voices.push((Voice::Ambient(pitch), entity.position()));
            } else {
                entity.ambient_sound_time += 1;
            }
        }
        if removed {
            return;
        }
        let mut thrown = None;
        if entity.witch.health <= 0.0 && !entity.no_ai {
            let yaw = entity.witch.yaw;
            let _ = super::living::dying_travel(&mut entity.witch.body, &*world, yaw, entity.tick_count, &mut entity.random, &mut entity.voices, WITCH_SOUNDS, Some(true));
        } else if !entity.no_ai {
            thrown = entity.tick_ai(&*world, players, &vitals, game_time, difficulty, bright_outside);
        } else {
            entity.witch.body.trim_small_velocity();
            monster_idle_without_ai(&*world, players, entity.witch.body.position, witch::EYE_HEIGHT, &mut entity.no_action_time);
        }
        let old = entity.previous_position;
        super::hazards::blocks_act(entity, &*world, old, game_time);
        self.push_entities(EntityKey::Witch(id), &*world, game_time, ticks);
        if let Some((throw, from)) = thrown {
            // `ThrowableItemProjectile`: from its eyes less 0.1.
            let start = DVec3::new(from.x, from.y + f64::from(witch::EYE_HEIGHT) - f64::from(0.1_f32), from.z);
            let seed = self.arrow_shoot_seed.unwrap_or_else(|| self.projectile_seed_random.next_long());
            let mut random = LegacyRandom::new(seed);
            let potion = ThrownPotion::shoot(throw.potion, id, start, throw.direction, throw.power, witch::UNCERTAINTY, &mut random);
            self.spawn_potion(potion);
        }
    }

    /// Every living, pickable thing a potion may strike (`canHitEntity`),
    /// the thrower marked.
    fn potion_targets(&self, owner: Option<u64>, players: &[PlayerCandidate]) -> Vec<PotionTarget> {
        let mut targets: Vec<PotionTarget> = self
            .order
            .iter()
            .filter_map(|key| {
                let id = key.id();
                let (body, health) = self.mob_body(id)?;
                if health <= 0.0 {
                    return None;
                }
                let half = f64::from(body.width / 2.0);
                let p = body.position;
                Some(PotionTarget { id, min: DVec3::new(p.x - half, p.y, p.z - half), max: DVec3::new(p.x + half, p.y + f64::from(body.height), p.z + half), owner: Some(id) == owner })
            })
            .collect();
        targets.extend(players.iter().filter(|p| self.players_pickable && p.alive && !p.spectator).map(|p| PotionTarget {
            id: PLAYER_TARGET + p.id,
            min: p.position - DVec3::new(0.3, 0.0, 0.3),
            max: p.position + DVec3::new(0.3, 1.8, 0.3),
            owner: false,
        }));
        targets
    }

    /// One potion's tick; a break splashes its effect.
    pub(super) fn tick_potion(&mut self, id: u64, world: &mut impl World, players: &[PlayerCandidate]) {
        let Some(owner) = self.potions.iter().find(|p| p.id == id).map(|p| p.potion.owner) else { return };
        let targets = self.potion_targets(owner, players);
        // The thrower's box while it is in the world (`getOwner`).
        let owner_box = owner.and_then(|o| self.mob_body(o)).map(|(body, _)| {
            let half = f64::from(body.width / 2.0);
            let p = body.position;
            (DVec3::new(p.x - half, p.y, p.z - half), DVec3::new(p.x + half, p.y + f64::from(body.height), p.z + half))
        });
        let Some(entity) = self.potions.iter_mut().find(|p| p.id == id) else { return };
        entity.previous_position = entity.potion.position;
        if let Some(hit) = entity.potion.tick(&*world, &targets, owner_box) {
            let potion = entity.potion.clone();
            self.potion_breaks.push(PotionBreak { position: potion.position, color: potion.potion.color(), instant: potion.potion.instant() });
            self.splash(&potion, hit, players);
        }
    }

    /// `ThrownSplashPotion.onHitAsPotion`: every living thing within reach
    /// of the break (not dying) takes the potion's effect scaled by how
    /// near it is: an instant effect at once, a lasting one for a share of
    /// its time unless that would end within a second.
    fn splash(&mut self, potion: &ThrownPotion, _hit: PotionHit, players: &[PlayerCandidate]) {
        let at = potion.position;
        let margin = potion.margin();
        let effect = potion.potion.effect();
        for key in self.order.clone() {
            let id = key.id();
            let Some((body, health)) = self.mob_body(id) else { continue };
            if health <= 0.0 {
                continue;
            }
            let half = f64::from(body.width / 2.0);
            let p = body.position;
            let (min, max) = (DVec3::new(p.x - half, p.y, p.z - half), DVec3::new(p.x + half, p.y + f64::from(body.height), p.z + half));
            let Some(scale) = splash_scale(at, margin, min, max) else { continue };
            if effect.effect.instantaneous() {
                self.splash_instant(id, effect.effect, effect.amplifier, scale, at, potion.owner);
            } else if let Some(splashed) = splashed_effect(&effect, scale) {
                self.add_mob_effect(id, splashed);
            }
        }
        if self.players_pickable {
            for player in players.iter().filter(|p| p.alive && !p.spectator) {
                let (min, max) = (player.position - DVec3::new(0.3, 0.0, 0.3), player.position + DVec3::new(0.3, 1.8, 0.3));
                let Some(scale) = splash_scale(at, margin, min, max) else { continue };
                let (instant, duration) = if effect.effect.instantaneous() {
                    (instant_work(effect.effect, effect.amplifier, false, scale), 0)
                } else {
                    match splashed_effect(&effect, scale) {
                        Some(splashed) => (None, splashed.duration),
                        None => continue,
                    }
                };
                self.player_splashes.push((player.id, PlayerSplash { effect: effect.effect, amplifier: effect.amplifier, duration, instant, at }));
            }
        }
    }

    /// `LivingEntity.addEffect` on a mob, by ID, if it can take it.
    pub fn add_mob_effect(&mut self, id: u64, effect: EffectInstance) -> bool {
        let Some((effects, undead, spider)) = self.mob_effects_mut(id) else { return false };
        can_be_affected(effect.effect, undead, spider) && effects.add(effect)
    }

    /// A mob's effects, with whether it is undead or a spider.
    pub fn mob_effects_mut(&mut self, id: u64) -> Option<(&mut MobEffects, bool, bool)> {
        if let Some(e) = self.witches.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.witch.effects, false, false));
        }
        if let Some(e) = self.bats.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.effects, false, false));
        }
        if let Some(e) = self.zombies.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.effects, true, false));
        }
        if let Some(e) = self.skeletons.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.effects, true, false));
        }
        if let Some(e) = self.creepers.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.effects, false, false));
        }
        if let Some(e) = self.spiders.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.effects, false, true));
        }
        if let Some(e) = self.slimes.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.effects, false, false));
        }
        if let Some(e) = self.endermen.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.effects, false, false));
        }
        if let Some(e) = self.villagers.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.effects, false, false));
        }
        if let Some(e) = self.iron_golems.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.effects, false, false));
        }
        if let Some(e) = self.wolves.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.effects, false, false));
        }
        if let Some(e) = self.cows.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.effects, false, false));
        }
        if let Some(e) = self.sheep.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.effects, false, false));
        }
        if let Some(e) = self.pigs.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.effects, false, false));
        }
        if let Some(e) = self.chickens.iter_mut().find(|e| e.id == id) {
            return Some((&mut e.effects, false, false));
        }
        None
    }

    /// A mob's effects, by ID.
    pub fn mob_effects(&self, id: u64) -> Option<&MobEffects> {
        if let Some(e) = self.witches.iter().find(|e| e.id == id) {
            return Some(&e.witch.effects);
        }
        self.bats.iter().find(|e| e.id == id).map(|e| &e.effects)
            .or_else(|| self.zombies.iter().find(|e| e.id == id).map(|e| &e.effects))
            .or_else(|| self.skeletons.iter().find(|e| e.id == id).map(|e| &e.effects))
            .or_else(|| self.creepers.iter().find(|e| e.id == id).map(|e| &e.effects))
            .or_else(|| self.spiders.iter().find(|e| e.id == id).map(|e| &e.effects))
            .or_else(|| self.slimes.iter().find(|e| e.id == id).map(|e| &e.effects))
            .or_else(|| self.endermen.iter().find(|e| e.id == id).map(|e| &e.effects))
            .or_else(|| self.villagers.iter().find(|e| e.id == id).map(|e| &e.effects))
            .or_else(|| self.iron_golems.iter().find(|e| e.id == id).map(|e| &e.effects))
            .or_else(|| self.wolves.iter().find(|e| e.id == id).map(|e| &e.effects))
            .or_else(|| self.cows.iter().find(|e| e.id == id).map(|e| &e.effects))
            .or_else(|| self.sheep.iter().find(|e| e.id == id).map(|e| &e.effects))
            .or_else(|| self.pigs.iter().find(|e| e.id == id).map(|e| &e.effects))
            .or_else(|| self.chickens.iter().find(|e| e.id == id).map(|e| &e.effects))
    }

    /// `HealOrHarmMobEffect.applyInstantaneousEffect` on a mob: the heal,
    /// or the harm as `indirect_magic` from the potion (knocked back from
    /// where it broke, crediting the thrower); the undead the other way
    /// round. An enderman teleports away from a potion instead of taking
    /// its harm (`Enderman.hurtServer`).
    fn splash_instant(&mut self, id: u64, effect: MobEffect, amplifier: i32, scale: f64, at: DVec3, owner: Option<u64>) {
        let undead = self.zombies.iter().any(|e| e.id == id) || self.skeletons.iter().any(|e| e.id == id);
        match instant_work(effect, amplifier, undead, scale) {
            Some(EffectWork::Heal(amount)) => self.heal_mob(id, amount),
            Some(EffectWork::HurtMagic(amount)) => {
                let result = self.hurt_mob_by_magic(id, amount, owner);
                if let Some(result) = result {
                    let attacker = owner.and_then(|o| self.entity_type(o)).unwrap_or("minecraft:splash_potion");
                    self.credit(id, result, attacker, true);
                    if result.applied && result.full {
                        if let Some((body, _)) = self.mob_body(id) {
                            let (xd, zd) = (at.x - body.position.x, at.z - body.position.z);
                            self.knock_back(id, f64::from(0.4_f32), xd, zd);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// `LivingEntity.heal` on a mob, by ID.
    fn heal_mob(&mut self, id: u64, amount: f32) {
        let (health, max) = if let Some(e) = self.witches.iter_mut().find(|e| e.id == id) {
            (&mut e.witch.health, witch::MAX_HEALTH)
        } else if let Some(e) = self.bats.iter_mut().find(|e| e.id == id) {
            (&mut e.bat.health, 6.0)
        } else if let Some(e) = self.zombies.iter_mut().find(|e| e.id == id) {
            (&mut e.zombie.health, 20.0)
        } else if let Some(e) = self.skeletons.iter_mut().find(|e| e.id == id) {
            let max = e.skeleton.kind.max_health();
            (&mut e.skeleton.health, max)
        } else if let Some(e) = self.creepers.iter_mut().find(|e| e.id == id) {
            (&mut e.creeper.health, 20.0)
        } else if let Some(e) = self.spiders.iter_mut().find(|e| e.id == id) {
            (&mut e.spider.health, crate::spider::MAX_HEALTH)
        } else if let Some(e) = self.slimes.iter_mut().find(|e| e.id == id) {
            let max = e.slime.max_health();
            (&mut e.slime.health, max)
        } else if let Some(e) = self.endermen.iter_mut().find(|e| e.id == id) {
            (&mut e.enderman.health, crate::enderman::MAX_HEALTH)
        } else if let Some(e) = self.villagers.iter_mut().find(|e| e.id == id) {
            (&mut e.villager.health, 20.0)
        } else if let Some(e) = self.iron_golems.iter_mut().find(|e| e.id == id) {
            (&mut e.golem.health, crate::iron_golem::MAX_HEALTH)
        } else if let Some(e) = self.wolves.iter_mut().find(|e| e.id == id) {
            let max = e.wolf.max_health();
            (&mut e.wolf.health, max)
        } else if let Some(e) = self.cows.iter_mut().find(|e| e.id == id) {
            (&mut e.cow.health, 10.0)
        } else if let Some(e) = self.sheep.iter_mut().find(|e| e.id == id) {
            (&mut e.health, 8.0)
        } else if let Some(e) = self.pigs.iter_mut().find(|e| e.id == id) {
            (&mut e.pig.health, 10.0)
        } else if let Some(e) = self.chickens.iter_mut().find(|e| e.id == id) {
            (&mut e.chicken.health, 4.0)
        } else {
            return;
        };
        heal(health, max, amount);
    }

    /// `hurtServer` with `indirect_magic` (from a potion thrown by `owner`)
    /// on a mob, by ID: past armor (`#bypasses_armor`); animals panic.
    fn hurt_mob_by_magic(&mut self, id: u64, amount: f32, owner: Option<u64>) -> Option<DamageResult> {
        let source = DamageSourceKind::Magic;
        let result = if let Some(e) = self.witches.iter_mut().find(|e| e.id == id) {
            e.hurt(amount, true, owner == Some(id))
        } else if let Some(e) = self.bats.iter_mut().find(|e| e.id == id) {
            e.hurt(amount)
        } else if let Some(e) = self.zombies.iter_mut().find(|e| e.id == id) {
            e.hurt(amount)
        } else if let Some(e) = self.skeletons.iter_mut().find(|e| e.id == id) {
            e.hurt(amount)
        } else if let Some(e) = self.creepers.iter_mut().find(|e| e.id == id) {
            e.hurt(amount)
        } else if let Some(e) = self.spiders.iter_mut().find(|e| e.id == id) {
            e.hurt(amount)
        } else if let Some(e) = self.slimes.iter_mut().find(|e| e.id == id) {
            e.hurt(amount)
        } else if let Some(e) = self.endermen.iter_mut().find(|e| e.id == id) {
            // A potion's hit teleports it away, up to 64 tries.
            e.dodge_pending = true;
            DamageResult { applied: false, dealt: 0.0, died: false, full: false }
        } else if let Some(e) = self.villagers.iter_mut().find(|e| e.id == id) {
            // `indirectMagic(potion, thrower)`.
            let kind = if owner.is_some() { "minecraft:indirect_magic" } else { "minecraft:magic" };
            e.hurt_from(amount, kind, owner, self.game_time)
        } else if let Some(e) = self.iron_golems.iter_mut().find(|e| e.id == id) {
            e.hurt(amount)
        } else if let Some(e) = self.wolves.iter_mut().find(|e| e.id == id) {
            let kind = if owner.is_some() { "minecraft:indirect_magic" } else { "minecraft:magic" };
            e.hurt_from(amount, kind, owner.map(crate::monster_ai::Target::Mob), self.game_time)
        } else if let Some(e) = self.cows.iter_mut().find(|e| e.id == id) {
            e.hurt(amount, source)
        } else if let Some(e) = self.sheep.iter_mut().find(|e| e.id == id) {
            e.hurt(amount, source)
        } else if let Some(e) = self.pigs.iter_mut().find(|e| e.id == id) {
            e.hurt(amount, source)
        } else if let Some(e) = self.chickens.iter_mut().find(|e| e.id == id) {
            e.hurt_with_source(amount, source)
        } else {
            return None;
        };
        if let (true, Some(owner)) = (result.applied, owner) {
            self.villager_hurt_by(id, owner);
        }
        Some(result)
    }
}
