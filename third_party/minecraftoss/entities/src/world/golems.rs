//! Iron golems (26.3 `IronGolem`) in the entity world. A golem ticks as the
//! mobs on the goal framework do, with its own quirks: moving sideways, one
//! draw in five from its random sends up a sprint particle from the block
//! it stands on (two more draws where that block shows); its ambient roll
//! finds no sound; falls and drowning never hurt it and knockback never
//! moves it; a hit that cracks it further plays `entity.iron_golem.damage`;
//! and after its move its arm swing and poppy run down and its anger is
//! renewed while it has a target. Its village goals see the level's points
//! of interest and random, lent from the world for its tick, and the
//! villagers about it with whether each wants a golem.
use super::emissions::{base_tick_fluid, play_movement, MovementSounds};
use super::*;
use crate::golem_ai::GolemVillager;
use crate::iron_golem::{self, IronGolem};
use crate::monster_ai::{update_enderman_anger, MobCandidate, MonsterKind};

/// `IronGolem.playStepSound`: its own step at full volume; not a `Monster`.
const GOLEM_SOUNDS: MovementSounds = MovementSounds::creature(Some("entity.iron_golem.step")).with_step_volume(1.0);

/// An iron golem on the monster goal framework.
#[derive(Clone)]
pub struct IronGolemEntity {
    pub id: u64,
    pub golem: IronGolem,
    /// Its status effects (`activeEffects`).
    pub effects: crate::effects::MobEffects,
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

/// `BlockState.getRenderShape() == INVISIBLE`: air, fluids and the few
/// blocks drawn by nothing (or only by their block entity).
fn renders_invisible(id: &str) -> bool {
    matches!(
        id,
        "minecraft:air"
            | "minecraft:cave_air"
            | "minecraft:void_air"
            | "minecraft:water"
            | "minecraft:lava"
            | "minecraft:bubble_column"
            | "minecraft:barrier"
            | "minecraft:light"
            | "minecraft:structure_void"
            | "minecraft:moving_piston"
            | "minecraft:end_portal"
            | "minecraft:end_gateway"
    )
}

impl IronGolemEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.golem.body.position
    }

    pub fn set_random_seed(&mut self, seed: u64) {
        self.random = LegacyRandom::new(seed);
    }

    /// `getOfferFlowerTick`: ticks left holding out a poppy.
    pub fn offer_flower_tick(&self) -> i32 {
        self.ai.state.golem.offer_flower_tick
    }

    /// `LivingEntity.hurtServer` for an iron golem: the hurt or death sound
    /// on a full hit, then `entity.iron_golem.damage` when the hit cracked
    /// it further (`IronGolem.hurtServer`).
    pub fn hurt(&mut self, amount: f32) -> DamageResult {
        if self.golem.health > 0.0 && !self.golem.damage.dead {
            self.no_action_time = 0;
        }
        let before = self.golem.crackiness();
        let result = self.golem.damage.hurt_generic(&mut self.golem.health, iron_golem::MAX_HEALTH, amount);
        let position = self.position();
        self.golem.damage.place_death(result, position, self.golem.body.fire_ticks > 0);
        if result.applied && result.full {
            if !result.died {
                self.ambient_sound_time = -iron_golem::AMBIENT_INTERVAL;
            }
            let voice = hurt_voice(&mut self.random, result.died, false);
            self.voices.push((voice, position));
        }
        if result.applied && self.golem.crackiness() != before {
            self.voices.push((Voice::Event("entity.iron_golem.damage", 1.0, 1.0), position));
        }
        result
    }

    /// `IronGolem.mobInteract` with an iron ingot: 25 health back and the
    /// repair sound (its pitch from its random), or nothing when whole.
    pub fn repair_with_ingot(&mut self) -> bool {
        let before = self.golem.health;
        if before <= 0.0 {
            return false;
        }
        heal(&mut self.golem.health, iron_golem::MAX_HEALTH, iron_golem::INGOT_HEAL);
        if self.golem.health == before {
            return false;
        }
        let (a, b) = (self.random.next_float(), self.random.next_float());
        self.voices.push((Voice::Event("entity.iron_golem.repair", 1.0, 1.0 + (a - b) * 0.2), self.position()));
        true
    }

    /// `IronGolem.canSpawnSprintParticle` and `Entity.spawnSprintParticle`
    /// (first in `Entity.baseTick`): moving sideways, one draw in five; a
    /// particle from the block it stands on takes two more where that block
    /// shows.
    fn sprint_particle(&mut self, world: &impl World) {
        let v = self.golem.body.velocity;
        if v.x * v.x + v.z * v.z > f64::from(2.500_000_3e-7_f32) && self.random.next_int(5) == 0 {
            let on = self.golem.body.on_pos(world, 0.2);
            if world.block(on).is_some_and(|b| !renders_invisible(&b.id)) {
                let _ = (self.random.next_double(), self.random.next_double());
            }
        }
    }

    /// `LivingEntity.aiStep` for an active golem: `Mob.serverAiStep` (goals,
    /// navigation, controls), the jump and travel. Returns the target its
    /// melee attack struck, the damage drawn, and where it stood.
    fn tick_ai(&mut self, world: &impl World, game_time: i64) -> Option<(crate::monster_ai::Target, f32, DVec3)> {
        let ai = &mut self.ai;
        let body = &mut self.golem.body;
        if ai.no_jump_delay > 0 {
            ai.no_jump_delay -= 1;
        }
        body.trim_small_velocity();
        ai.state.body = body.clone();
        ai.state.health = self.golem.health;
        ai.state.max_health = iron_golem::MAX_HEALTH;
        ai.state.game_time = game_time;
        ai.state.tick_count = self.tick_count;
        ai.state.golem.player_created = self.golem.player_created;
        ai.state.random = std::mem::take(&mut self.random);
        self.no_action_time += 1;
        let position = body.position;
        let fluid = FluidFrame::sample(world, position, body.width, body.height);
        ai.state.fluid = fluid;
        ai.state.no_action_time = self.no_action_time;
        let full = self.tick_count <= 1 || (self.tick_count + self.id as i32) % 2 == 0;
        ai.tick_goals(world, full);
        // `IronGolem.doHurtTarget`: the arms swing for ten ticks.
        let struck = ai.state.attack.take().map(|target| (target, ai.state.attack_damage.take().unwrap_or(0.0), body.position));
        if struck.is_some() {
            self.golem.attack_animation_tick = 10;
        }
        body.position = ai.state.body.position;
        let position = body.position;
        let (can_update, surface) = crate::navigation::ground_view(world, body, fluid, ai.state.walk.can_float);
        if let Some((wanted, speed)) = ai.state.navigation.tick_in(world, position, can_update, surface, body.width, ai.speed) {
            ai.move_control.set_wanted_position(wanted, speed);
        }
        self.random = std::mem::take(&mut ai.state.random);
        let obstacle_top = crate::control::obstacle_top(world, position);
        let speed = self.effects.movement_speed(iron_golem::MOVEMENT_SPEED);
        let control = ai.move_control.tick(position, body.on_ground, ai.yaw, ai.speed, ai.forward, ai.sideways, speed, body.width, body.step_height, obstacle_top, |_, _| true);
        ai.yaw = control.yaw;
        ai.speed = control.speed;
        ai.forward = control.forward;
        ai.sideways = control.sideways;
        ai.state.look_control.tick(position, iron_golem::EYE_HEIGHT, ai.body_rotation.body_yaw, !ai.state.navigation.is_done());
        ai.jumping = ai.state.jump || control.jump;
        body.living_jump(world, fluid, ai.jumping, &mut ai.no_jump_delay, 0.4);
        let input = DVec3::new(f64::from(ai.sideways), 0.0, f64::from(ai.forward));
        if fluid.in_water() {
            body.travel_water(world, input, ai.yaw);
        } else if fluid.in_lava() {
            body.travel_lava(world, input, ai.yaw, fluid.lava_height);
        } else {
            // `#fall_damage_immune`: a landing never hurts it.
            let _ = body.travel_air_jumping(world, input, ai.speed, ai.yaw, ai.jumping);
        }
        play_movement(body, self.tick_count, &mut self.random, &mut self.voices, GOLEM_SOUNDS);
        ai.body_rotation.tick(ai.yaw, &mut ai.state.look_control, self.previous_position, body.position);
        self.golem.yaw = ai.yaw;
        struck
    }
}

impl EntityWorld {
    /// An iron golem with its goals, facing its yaw (a NoAI one keeps
    /// still but still takes hits).
    pub fn spawn_iron_golem(&mut self, golem: IronGolem, no_ai: bool) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let ai = MonsterAi::of_kind(MonsterKind::IronGolem, &golem.body, golem.yaw);
        self.iron_golems.push(IronGolemEntity {
            id,
            previous_position: golem.body.position,
            golem,
            effects: Default::default(),
            no_ai,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            random: LegacyRandom::new(0),
            ai: Box::new(ai),
        });
        self.order.push(EntityKey::IronGolem(id));
        self.file_in_section(EntityKey::IronGolem(id));
        id
    }

    pub fn iron_golems(&self) -> &[IronGolemEntity] {
        &self.iron_golems
    }

    pub fn iron_golem_mut(&mut self, id: u64) -> Option<&mut IronGolemEntity> {
        self.iron_golems.iter_mut().find(|entity| entity.id == id)
    }

    /// The random a summoned golem gets pinned (the harness pins what
    /// vanilla draws unseeded), and the tag it is known by.
    pub fn set_summoned_golem_seed(&mut self, seed: Option<u64>) {
        self.summoned_golem_seed = seed;
    }

    /// The golems villagers summoned, newest last.
    pub fn take_summoned_golems(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.summoned_golems)
    }

    /// `SpawnUtil.trySpawnMob`'s golem, made where a villager's brain found
    /// room, facing as drawn, its follow range with the spawn bonus; its own
    /// random is unseeded in vanilla (pinned by the harness). It ticks from
    /// the next game tick.
    pub(super) fn spawn_summoned_golem(&mut self, summoned: crate::golem_spawn::SummonedGolem) {
        let mut golem = IronGolem::new(summoned.at);
        golem.yaw = summoned.yaw;
        let id = self.spawn_iron_golem(golem, false);
        let seed = self.summoned_golem_seed.unwrap_or_else(|| {
            self.seed_uniquifier = self.seed_uniquifier.wrapping_mul(1_181_783_497_276_652_981);
            self.seed_uniquifier
        });
        let Some(entity) = self.iron_golems.iter_mut().find(|e| e.id == id) else { return };
        entity.random = LegacyRandom::new(seed);
        entity.ai.state.follow_range = crate::monster_ai::FOLLOW_RANGE * (1.0 + summoned.follow_bonus);
        // Made, not loaded: `persistentAngerEndTime` keeps its 0.
        entity.ai.state.enderman.anger_end_time = 0;
        entity.ai.body_rotation.body_yaw = summoned.yaw;
        entity.ai.state.look_control.head_yaw = summoned.yaw;
        self.summoned_golems.push(id);
    }

    /// `Villager.wantsToSpawnGolem`: it slept within the last day and has
    /// not seen a golem lately.
    pub(super) fn wants_golem(&self, villager: &VillagerEntity) -> bool {
        let Some(ai) = villager.ai.as_deref() else { return false };
        let memories = &ai.brain.memories;
        memories.last_slept.get().is_some_and(|&slept| self.game_time - slept < 24_000) && !memories.golem_detected_recently.present()
    }

    /// The villagers of the entity sections a box 32 blocks around the
    /// golem reaches, in their order, as its goals see them.
    fn villagers_about(&self, body: &Body, players: &[PlayerCandidate]) -> Vec<GolemVillager> {
        let half = f64::from(body.width) / 2.0;
        let p = body.position;
        let (min, max) = (DVec3::new(p.x - half - 32.0, p.y - 32.0, p.z - half - 32.0), DVec3::new(p.x + half + 32.0, p.y + f64::from(body.height) + 32.0, p.z + half + 32.0));
        self.sections
            .living_around(min, max)
            .into_iter()
            .filter_map(|id| self.villagers.iter().find(|v| v.id == id))
            .map(|v| GolemVillager {
                id: v.id,
                position: v.villager.body.position,
                width: v.villager.body.width,
                height: v.villager.body.height,
                alive: v.villager.health > 0.0,
                wants_golem: self.wants_golem(v),
                reputations: if v.gossips.is_empty() {
                    Vec::new()
                } else {
                    players.iter().map(|p| (p.id, v.gossips.reputation(self.uuid_of(PLAYER_TARGET + p.id)))).filter(|&(_, r)| r != 0).collect()
                },
            })
            .collect()
    }

    /// One golem's tick: `Entity.baseTick` (the sprint particle, water, a
    /// splash), `LivingEntity.baseTick` (fire, walls, the hurt and death
    /// clocks, the forgotten attacker, its effects), the ambient roll, then
    /// its AI while alive, the blocks it went through and its pushes, and
    /// `IronGolem.aiStep`'s tail.
    pub(super) fn tick_iron_golem(&mut self, id: u64, world: &mut impl World, players: &[PlayerCandidate], ticks: &dyn Fn(DVec3) -> bool) {
        let (game_time, difficulty, bright_outside) = (self.game_time, self.difficulty, self.bright_outside);
        let Some(body) = self.iron_golems.iter().find(|e| e.id == id).map(|e| e.golem.body.clone()) else { return };
        let about = self.villagers_about(&body, players);
        let mobs = self.mob_candidates();
        let villagers: Vec<MobCandidate> = self
            .villagers
            .iter()
            .map(|v| MobCandidate { id: v.id, position: v.villager.body.position, eye_height: v.eye_height(), width: v.villager.body.width, height: v.villager.body.height, alive: v.villager.health > 0.0, kind: "minecraft:villager" })
            .collect();
        let pois = std::mem::take(&mut self.pois);
        let level_random = std::mem::take(&mut self.level_random);
        let Some(entity) = self.iron_golems.iter_mut().find(|e| e.id == id) else {
            (self.pois, self.level_random) = (pois, level_random);
            return;
        };
        entity.previous_position = entity.golem.body.position;
        entity.tick_count += 1;
        entity.sprint_particle(&*world);
        base_tick_fluid(&mut entity.golem.body, &*world, entity.tick_count == 1, &mut entity.random, &mut entity.voices, GOLEM_SOUNDS);
        super::hazards::burn(entity, &*world, game_time);
        super::hazards::suffocate(entity, &*world, iron_golem::EYE_HEIGHT, game_time);
        // `decreaseAirSupply` keeps its air: it never drowns.
        let removed = entity.golem.damage.tick();
        // `LivingEntity.baseTick` forgets an attacker after 100 ticks.
        if entity.ai.state.hurt_by.is_some_and(|(_, when)| entity.tick_count - when > 100) {
            entity.ai.state.hurt_by = None;
        }
        for work in entity.effects.tick(false) {
            match work.resolve(entity.golem.health, iron_golem::MAX_HEALTH) {
                Some(EffectWork::Heal(amount)) => heal(&mut entity.golem.health, iron_golem::MAX_HEALTH, amount),
                Some(EffectWork::HurtMagic(amount)) => {
                    entity.hurt(amount);
                }
                _ => {}
            }
        }
        if entity.golem.health > 0.0 {
            // `AbstractGolem.getAmbientSound` is none: the roll only resets.
            if (entity.random.next_int(1000) as i32) < entity.ambient_sound_time {
                entity.ambient_sound_time = -iron_golem::AMBIENT_INTERVAL;
            } else {
                entity.ambient_sound_time += 1;
            }
        }
        if removed {
            (self.pois, self.level_random) = (pois, level_random);
            return;
        }
        let mut struck = None;
        if entity.golem.health <= 0.0 && !entity.no_ai {
            let yaw = entity.golem.yaw;
            let _ = super::living::dying_travel(&mut entity.golem.body, &*world, yaw, entity.tick_count, &mut entity.random, &mut entity.voices, GOLEM_SOUNDS, None);
        } else if !entity.no_ai {
            let state = &mut entity.ai.state;
            state.difficulty = difficulty;
            state.bright_outside = bright_outside;
            state.players = players.to_vec();
            state.villagers = villagers;
            state.mobs = mobs;
            state.golem.villagers = about;
            state.pois = Some(pois);
            state.level_random = Some(level_random);
            struck = entity.tick_ai(&*world, game_time);
            let state = &mut entity.ai.state;
            self.pois = state.pois.take().expect("the points of interest come back");
            self.level_random = state.level_random.take().expect("the level's random comes back");
        } else {
            entity.golem.body.trim_small_velocity();
            (self.pois, self.level_random) = (pois, level_random);
        }
        let old = entity.previous_position;
        super::hazards::blocks_act(entity, &*world, old, game_time);
        // Its hit lands as it struck, before it pushes: the victim takes
        // its damage, knockback and lift, then the attack sound.
        if let Some((target, damage, from)) = struck {
            match target {
                crate::monster_ai::Target::Mob(victim) => {
                    self.mob_hits_mob(id, victim, damage, from, f64::from(0.4_f32));
                }
                crate::monster_ai::Target::Player(player_id) => {
                    self.player_hits.push(PlayerHit { player_id, damage, kind: PlayerHitKind::Melee { attacker: from, hunger_ticks: 0, lift: 0.4 }, source: Some(id) });
                }
                crate::monster_ai::Target::Villager(victim) => {
                    self.mob_hits_mob(id, victim, damage, from, f64::from(0.4_f32));
                }
            }
            if let Some(entity) = self.iron_golems.iter_mut().find(|e| e.id == id) {
                entity.voices.push((Voice::Event("entity.iron_golem.attack", 1.0, 1.0), from));
            }
        }
        self.push_entities(EntityKey::IronGolem(id), &*world, game_time, ticks);
        let Some(entity) = self.iron_golems.iter_mut().find(|e| e.id == id) else { return };
        // `IronGolem.aiStep` after `super.aiStep`.
        if entity.golem.attack_animation_tick > 0 {
            entity.golem.attack_animation_tick -= 1;
        }
        if entity.ai.state.golem.offer_flower_tick > 0 {
            entity.ai.state.golem.offer_flower_tick -= 1;
        }
        {
            // `updatePersistentAnger` sees whatever its hit left of its target.
            let mobs = self.mob_candidates();
            let Some(entity) = self.iron_golems.iter_mut().find(|e| e.id == id) else { return };
            let state = &mut entity.ai.state;
            state.mobs = mobs;
            state.random = std::mem::take(&mut entity.random);
            update_enderman_anger(state);
            entity.random = std::mem::take(&mut state.random);
        }
        self.file_in_section(EntityKey::IronGolem(id));
    }
}
