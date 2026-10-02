//! Endermen (26.3 `Enderman`) in the entity world: the monster goal
//! framework's enderman goals and targets, `NeutralMob` anger renewed while
//! it has a target (a draw from its random each tick), the daylight
//! teleport (`customServerAiStep`), teleports that stop its path and play
//! `entity.enderman.teleport` twice where it lands, the screams of a
//! creepy enderman, blocks it takes and puts down where mobs may grief, and
//! water: in water or rain it takes drowning damage each tick and, as for
//! any hurt without a living attacker, nine times in ten teleports.
use super::emissions::{base_tick_fluid, play_movement, MovementSounds};
use super::*;
use crate::enderman::{Enderman, PlayerView};
use crate::monster_ai::{enderman_daylight_step, update_enderman_anger, MonsterKind};

/// An enderman steps with its block's sound; it is a `Monster`.
const ENDERMAN_SOUNDS: MovementSounds = MovementSounds::monster(None);

/// An enderman on the monster goal framework.
#[derive(Clone)]
pub struct EndermanEntity {
    pub id: u64,
    /// Its status effects (`activeEffects`).
    pub effects: crate::effects::MobEffects,
    pub enderman: Enderman,
    pub no_ai: bool,
    pub tick_count: i32,
    pub ambient_sound_time: i32,
    /// Sounds it made since the world last collected them.
    pub voices: Vec<(Voice, DVec3)>,
    pub no_action_time: i32,
    pub random: LegacyRandom,
    pub previous_position: DVec3,
    pub ai: Box<MonsterAi>,
    /// An arrow hit it: it dodges (`repeatedlyTryToTeleport`) when it
    /// next ticks.
    pub dodge_pending: bool,
}

impl EndermanEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.enderman.body.position
    }

    pub fn set_random_seed(&mut self, seed: u64) {
        self.random = LegacyRandom::new(seed);
    }

    /// The block it carries (`getCarriedBlock`).
    pub fn carried(&self) -> Option<&minecraftoss_player::Block> {
        self.ai.state.enderman.carried.as_ref()
    }

    /// `Enderman.isCreepy`: it has a target (its mouth opens).
    pub fn creepy(&self) -> bool {
        self.ai.state.enderman.creepy
    }

    /// `LivingEntity.hurtServer` for a hit with a living attacker (a
    /// player's, a blast's): no teleport follows.
    pub fn hurt(&mut self, amount: f32) -> DamageResult {
        if self.enderman.health > 0.0 && !self.enderman.damage.dead {
            self.no_action_time = 0;
        }
        let result = self.enderman.damage.hurt_generic(&mut self.enderman.health, crate::enderman::MAX_HEALTH, amount);
        let position = self.position();
        self.enderman.damage.place_death(result, position, self.enderman.body.fire_ticks > 0);
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

    /// `Enderman.hurtServer` without a living attacker (drowning): the hurt,
    /// then nine times in ten a teleport, whether the hurt took or not.
    pub(super) fn hurt_by_environment(&mut self, amount: f32, world: &dyn World) -> DamageResult {
        let result = self.hurt(amount);
        if self.random.next_int(10) != 0 {
            self.teleport(world);
        }
        result
    }

    /// `Enderman.teleport()` outside the goals, while alive.
    fn teleport(&mut self, world: &dyn World) -> bool {
        if self.enderman.health <= 0.0 {
            return false;
        }
        let moved = crate::enderman::teleport(&mut self.enderman.body, &mut self.random, world);
        if moved {
            self.ai.state.navigation.stop();
            self.landed(self.enderman.body.position);
        }
        moved
    }

    /// After a teleport: `snapTo` moves the old position along, and both
    /// teleport sounds play where it landed (the one at `xo` too).
    fn landed(&mut self, at: DVec3) {
        self.previous_position = at;
        for _ in 0..2 {
            self.voices.push((Voice::Event("entity.enderman.teleport", 1.0, 1.0), at));
        }
    }

    /// `Enderman.repeatedlyTryToTeleport`: up to 64 tries.
    fn dodge(&mut self, world: &dyn World) {
        for _ in 0..64 {
            if self.teleport(world) {
                return;
            }
        }
    }

    /// `Enderman.aiStep` and `LivingEntity.aiStep` for an active enderman:
    /// its anger first, then `Mob.serverAiStep` (goals, the blocks they
    /// move, navigation, the daylight teleport, the controls), the jump
    /// and travel (the harm water does comes after pushing). Returns the
    /// target its attack goal hit and where it stood (`Mob.doHurtTarget`
    /// runs in the goals).
    #[allow(clippy::too_many_arguments)]
    fn tick_ai(&mut self, world: &mut impl World, players: &[PlayerCandidate], views: &[(u64, PlayerView)], game_time: i64, difficulty: i32, mob_griefing: bool, bright_outside: bool) -> Option<(crate::monster_ai::Target, DVec3)> {
        let position = self.enderman.body.position;
        // `Mob.checkDespawn`: a player within 32 blocks resets the idle clock.
        if players.iter().any(|p| p.alive && !p.spectator && p.position.distance_squared(position) < 32.0 * 32.0) {
            self.no_action_time = 0;
        }
        let health = self.enderman.health;
        let ai = &mut self.ai;
        let body = &mut self.enderman.body;
        ai.state.body = body.clone();
        ai.state.health = health;
        ai.state.max_health = crate::enderman::MAX_HEALTH;
        ai.state.game_time = game_time;
        ai.state.difficulty = difficulty;
        ai.state.mob_griefing = mob_griefing;
        ai.state.bright_outside = bright_outside;
        ai.state.players = players.to_vec();
        ai.state.villagers = Vec::new();
        ai.state.views = views.to_vec();
        ai.state.tick_count = self.tick_count;
        ai.state.enderman.teleports.clear();
        ai.state.random = std::mem::take(&mut self.random);
        update_enderman_anger(&mut ai.state);
        // `Monster.updateNoActionTime`: bright light runs the idle clock.
        let eye = (position.x.floor() as i32, (position.y + f64::from(crate::enderman::EYE_HEIGHT)).floor() as i32, position.z.floor() as i32);
        if world.light_path_cost(eye) > 0.0 {
            self.no_action_time += 2;
        }
        if ai.no_jump_delay > 0 {
            ai.no_jump_delay -= 1;
        }
        body.trim_small_velocity();
        ai.state.body.velocity = body.velocity;
        self.no_action_time += 1;
        let fluid = FluidFrame::sample(&*world, position, body.width, body.height);
        ai.state.fluid = fluid;
        ai.state.no_action_time = self.no_action_time;
        let full = self.tick_count <= 1 || (self.tick_count + self.id as i32) % 2 == 0;
        ai.tick_goals(&*world, full);
        let hit = ai.state.attack.take().map(|target| (target, ai.state.body.position));
        // The goals took or put down a block.
        if let Some((pos, block)) = ai.state.enderman.block_change.take() {
            world.set_block(pos, block);
        }
        body.position = ai.state.body.position;
        let position = body.position;
        let (can_update, surface) = crate::navigation::ground_view(&*world, body, fluid, ai.state.walk.can_float);
        if let Some((wanted, speed)) = ai.state.navigation.tick_in(&*world, position, can_update, surface, body.width, ai.speed) {
            ai.move_control.set_wanted_position(wanted, speed);
        }
        enderman_daylight_step(&mut ai.state, &*world);
        body.position = ai.state.body.position;
        self.random = std::mem::take(&mut ai.state.random);
        let teleports = std::mem::take(&mut ai.state.enderman.teleports);
        let position = body.position;
        let obstacle_top = crate::control::obstacle_top(&*world, position);
        let speed = self.effects.movement_speed(crate::enderman::movement_speed(ai.state.target().is_some()));
        let control = ai.move_control.tick(position, body.on_ground, ai.yaw, ai.speed, ai.forward, ai.sideways, speed, body.width, body.step_height, obstacle_top, |_, _| true);
        ai.yaw = control.yaw;
        ai.speed = control.speed;
        ai.forward = control.forward;
        ai.sideways = control.sideways;
        ai.state.look_control.tick(position, crate::enderman::EYE_HEIGHT, ai.body_rotation.body_yaw, !ai.state.navigation.is_done());
        ai.jumping = ai.state.jump || control.jump;
        body.living_jump(&*world, fluid, ai.jumping, &mut ai.no_jump_delay, 0.4);
        let input = DVec3::new(f64::from(ai.sideways), 0.0, f64::from(ai.forward));
        let mut landed = None;
        if fluid.in_water() {
            body.travel_water(&*world, input, ai.yaw);
        } else if fluid.in_lava() {
            body.travel_lava(&*world, input, ai.yaw, fluid.lava_height);
        } else {
            landed = body.travel_air_jumping(&*world, input, ai.speed, ai.yaw, ai.jumping);
        }
        play_movement(body, self.tick_count, &mut self.random, &mut self.voices, ENDERMAN_SOUNDS);
        // A teleport moved the old position along.
        let previous = teleports.last().copied().unwrap_or(self.previous_position);
        ai.body_rotation.tick(ai.yaw, &mut ai.state.look_control, previous, body.position);
        self.enderman.yaw = ai.yaw;
        for at in teleports {
            self.landed(at);
        }
        // A fall hurts it without a living attacker, so it may teleport.
        if let Some(damage) = landed.and_then(|fallen| super::living::fall_damage(&self.enderman.body, &*world, fallen, true, &mut self.voices)) {
            self.hurt_by_environment(damage, &*world);
        }
        hit
    }

    /// `isSensitiveToWater`: water or rain hurts it (`isInWaterOrRain`),
    /// last in `LivingEntity.aiStep`.
    fn hurt_by_water(&mut self, world: &impl World) {
        let p = self.enderman.body.position;
        let top = (p.x.floor() as i32, (p.y + f64::from(self.enderman.body.height)).floor() as i32, p.z.floor() as i32);
        let wet = self.enderman.body.touching_water || world.rain_at((p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)) || world.rain_at(top);
        if wet {
            self.hurt_by_environment(1.0, world);
        }
    }
}

/// `Mob.getVoicePitch` for an adult.
fn adult_pitch(random: &mut LegacyRandom) -> f32 {
    let (a, b) = (random.next_float(), random.next_float());
    (a - b) * 0.2 + 1.0
}

impl EntityWorld {
    /// An enderman with its goals, facing its yaw (a NoAI one keeps still
    /// but still sounds and takes hits).
    pub fn spawn_enderman(&mut self, enderman: Enderman, no_ai: bool) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let ai = MonsterAi::of_kind(MonsterKind::Enderman, &enderman.body, enderman.yaw);
        self.endermen.push(EndermanEntity {
            effects: Default::default(),
            id,
            previous_position: enderman.body.position,
            enderman,
            no_ai,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            random: LegacyRandom::new(0),
            ai: Box::new(ai),
            dodge_pending: false,
        });
        self.order.push(EntityKey::Enderman(id));
        self.file_in_section(EntityKey::Enderman(id));
        id
    }

    pub fn endermen(&self) -> &[EndermanEntity] {
        &self.endermen
    }

    pub fn enderman_mut(&mut self, id: u64) -> Option<&mut EndermanEntity> {
        self.endermen.iter_mut().find(|entity| entity.id == id)
    }

    /// Where the players look, for endermen (level and south when unset).
    pub fn set_player_views(&mut self, views: Vec<(u64, PlayerView)>) {
        self.player_views = views;
    }

    /// One enderman's tick: `Entity.baseTick` (water, a splash), the
    /// forgotten attacker, the ambient roll (a scream when creepy), then its
    /// AI while alive.
    pub(super) fn tick_enderman(&mut self, id: u64, world: &mut impl World, players: &[PlayerCandidate], ticks: &dyn Fn(DVec3) -> bool) {
        let (game_time, difficulty, mob_griefing, bright_outside) = (self.game_time, self.difficulty, self.mob_griefing, self.bright_outside);
        let views = self.player_views.clone();
        let mobs = self.mob_candidates();
        let Some(entity) = self.endermen.iter_mut().find(|e| e.id == id) else { return };
        entity.ai.state.mobs = mobs;
        entity.previous_position = entity.enderman.body.position;
        entity.tick_count += 1;
        if std::mem::take(&mut entity.dodge_pending) {
            entity.dodge(&*world);
        }
        base_tick_fluid(&mut entity.enderman.body, &*world, entity.tick_count == 1, &mut entity.random, &mut entity.voices, ENDERMAN_SOUNDS);
        super::hazards::burn(entity, &*world, game_time);
        super::hazards::suffocate(entity, &*world, crate::enderman::EYE_HEIGHT, game_time);
        let water_breathing = entity.effects.has(crate::effects::MobEffect::WaterBreathing);
        if entity.enderman.health > 0.0 && super::living::breathe(&mut entity.enderman.body, &*world, crate::enderman::EYE_HEIGHT, water_breathing) {
            entity.hurt_by_environment(2.0, &*world);
        }
        let removed = entity.enderman.damage.tick();
        // `LivingEntity.baseTick` forgets an attacker after 100 ticks.
        if entity.ai.state.hurt_by.is_some_and(|(_, when)| entity.tick_count - when > 100) {
            entity.ai.state.hurt_by = None;
        }
        // `tickEffects`, after the hurt and death clocks.
        for work in entity.effects.tick(false) {
            match work.resolve(entity.enderman.health, crate::enderman::MAX_HEALTH) {
                Some(EffectWork::Heal(amount)) => heal(&mut entity.enderman.health, crate::enderman::MAX_HEALTH, amount),
                Some(EffectWork::HurtMagic(amount)) => {
                    entity.hurt_by_environment(amount, &*world);
                }
                _ => {}
            }
        }

        if entity.enderman.health > 0.0 {
            if (entity.random.next_int(1000) as i32) < entity.ambient_sound_time {
                entity.ambient_sound_time = -80;
                let pitch = adult_pitch(&mut entity.random);
                // `getAmbientSound`: a creepy enderman screams.
                let voice = if entity.creepy() { Voice::Event("entity.enderman.scream", 1.0, pitch) } else { Voice::Ambient(pitch) };
                entity.voices.push((voice, entity.position()));
            } else {
                entity.ambient_sound_time += 1;
            }
        }
        let mut hit = None;
        let mut active = false;
        if !removed {
            if entity.enderman.health <= 0.0 && !entity.no_ai {
                let yaw = entity.enderman.yaw;
                let _ = super::living::dying_travel(&mut entity.enderman.body, &*world, yaw, entity.tick_count, &mut entity.random, &mut entity.voices, ENDERMAN_SOUNDS, Some(true));
            }
            if entity.enderman.health > 0.0 && !entity.no_ai {
                hit = entity.tick_ai(world, players, &views, game_time, difficulty, mob_griefing, bright_outside);
                active = true;
            } else if entity.no_ai {
                entity.enderman.body.trim_small_velocity();
                if entity.no_ai {
                    monster_idle_without_ai(&*world, players, entity.enderman.body.position, crate::enderman::EYE_HEIGHT, &mut entity.no_action_time);
                }
            }
            let old = entity.previous_position;
            super::hazards::blocks_act(entity, &*world, old, game_time);
            self.push_entities(EntityKey::Enderman(id), &*world, game_time, ticks);
        }
        if active {
            if let Some(entity) = self.endermen.iter_mut().find(|e| e.id == id) {
                entity.hurt_by_water(&*world);
            }
            // Water may have sent it off.
            self.file_in_section(EntityKey::Enderman(id));
        }
        if let Some((crate::monster_ai::Target::Player(player_id), attacker)) = hit {
            self.player_hits.push(PlayerHit { player_id, damage: crate::enderman::ATTACK_DAMAGE, kind: PlayerHitKind::Melee { attacker, hunger_ticks: 0, lift: 0.0 }, source: Some(id) });
        }
        if let Some((crate::monster_ai::Target::Mob(victim), attacker)) = hit {
            self.mob_hits_mob(id, victim, crate::enderman::ATTACK_DAMAGE, attacker, 0.0);
        }
    }
}
