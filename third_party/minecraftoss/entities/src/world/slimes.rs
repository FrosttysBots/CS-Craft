//! Slimes (26.3 `Slime` over `AbstractCubeMob`) in the entity world: the
//! monster goal framework's cube goals over `CubeMobMoveControl` (it turns
//! 90° a tick to the heading the goals chose and, on the ground, hops at
//! it every 10 to 29 ticks, a third as long while hunting), the landing
//! squash and its particles' draws from the slime's random, touch damage
//! to players (`playerTouch`, `dealDamage`), and the split into two to
//! four slimes of half the size when a dead one is removed.
use super::*;
use crate::monster_ai::MonsterKind;
use super::emissions::{base_tick_fluid, play_movement, MovementSounds};
use crate::slime::{landing_particles, Slime};

/// A slime steps with its block's sound; it is no `Monster`, so it swims
/// and splashes generically.
const SLIME_SOUNDS: MovementSounds = MovementSounds::creature(None);

/// A slime on the monster goal framework with the cube move control.
#[derive(Clone)]
pub struct SlimeEntity {
    pub id: u64,
    /// Its status effects (`activeEffects`).
    pub effects: crate::effects::MobEffects,
    pub slime: Slime,
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

impl SlimeEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.slime.body.position
    }

    pub fn set_random_seed(&mut self, seed: u64) {
        self.random = LegacyRandom::new(seed);
    }

    /// The sound `name` of its size (`isTiny` slimes use the small ones).
    fn sound(&self, name: &str) -> &'static str {
        match (name, self.slime.tiny()) {
            ("hurt", true) => "entity.slime.hurt_small",
            ("hurt", false) => "entity.slime.hurt",
            ("death", true) => "entity.slime.death_small",
            ("death", false) => "entity.slime.death",
            ("jump", true) => "entity.slime.jump_small",
            ("jump", false) => "entity.slime.jump",
            ("squish", true) => "entity.slime.squish_small",
            _ => "entity.slime.squish",
        }
    }

    pub fn hurt(&mut self, amount: f32) -> DamageResult {
        // `hurtServer` resets the idle clock for any hit on a living mob,
        // before the damage cooldown decides.
        if self.slime.health > 0.0 && !self.slime.damage.dead {
            self.no_action_time = 0;
        }
        let max = self.slime.max_health();
        let result = self.slime.damage.hurt_generic(&mut self.slime.health, max, amount);
        let position = self.position();
        self.slime.damage.place_death(result, position, self.slime.body.fire_ticks > 0);
        // Only a full hit plays the hurt or death sound (`tookFullDamage`),
        // at the slime's volume and `getVoicePitch`.
        if result.applied && result.full {
            if !result.died {
                self.ambient_sound_time = -80;
            }
            let (a, b) = (self.random.next_float(), self.random.next_float());
            let pitch = (a - b) * 0.2 + 1.0;
            let event = self.sound(if result.died { "death" } else { "hurt" });
            self.voices.push((Voice::Event(event, self.slime.sound_volume(), pitch), position));
        }
        result
    }

    /// `Mob.serverAiStep` and `LivingEntity.aiStep` for an active slime:
    /// the goals, `CubeMobMoveControl` in place of the path and move
    /// control, the look and jump controls, then the jump and travel. A
    /// slime is no `Monster`: bright light does not run its idle clock.
    fn tick_ai(&mut self, world: &impl World, players: &[PlayerCandidate], game_time: i64, difficulty: i32) {
        let position = self.slime.body.position;
        // `Mob.checkDespawn` runs first: a player within 32 blocks resets
        // the idle clock.
        if players.iter().any(|p| p.alive && !p.spectator && p.position.distance_squared(position) < 32.0 * 32.0) {
            self.no_action_time = 0;
        }
        let (health, max_health, tiny) = (self.slime.health, self.slime.max_health(), self.slime.tiny());
        let (movement_speed, volume, eye_height) = (self.effects.movement_speed(self.slime.movement_speed()), self.slime.sound_volume(), self.slime.eye_height());
        let ai = &mut self.ai;
        let body = &mut self.slime.body;
        if ai.no_jump_delay > 0 {
            ai.no_jump_delay -= 1;
        }
        body.trim_small_velocity();
        self.no_action_time += 1;
        let fluid = FluidFrame::sample(world, position, body.width, body.height);
        ai.state.body = body.clone();
        ai.state.health = health;
        ai.state.max_health = max_health;
        ai.state.game_time = game_time;
        ai.state.difficulty = difficulty;
        ai.state.players = players.to_vec();
        ai.state.villagers = Vec::new();
        ai.state.fluid = fluid;
        ai.state.no_action_time = self.no_action_time;
        ai.state.tiny = tiny;
        ai.state.random = std::mem::take(&mut self.random);
        let full = self.tick_count <= 1 || (self.tick_count + self.id as i32) % 2 == 0;
        ai.tick_goals(world, full);
        self.random = std::mem::take(&mut ai.state.random);
        // `CubeMobMoveControl.tick`: the body, head and facing turn together
        // (`MoveControl.rotlerp`, which keeps the facing within 0 to 360).
        ai.yaw = crate::control::rotlerp(ai.yaw, ai.state.cube.y_rot, 90.0);
        ai.state.look_control.head_yaw = ai.yaw;
        ai.body_rotation.body_yaw = ai.yaw;
        let mut hop = false;
        if !ai.state.cube.move_to {
            ai.forward = 0.0;
        } else {
            ai.state.cube.move_to = false;
            let speed = (ai.state.cube.speed_modifier * movement_speed) as f32;
            if body.on_ground {
                ai.speed = speed;
                ai.forward = speed;
                let delay = ai.state.cube.jump_delay;
                ai.state.cube.jump_delay -= 1;
                if delay <= 0 {
                    // `getJumpDelay`, a third of it when aggressive.
                    let mut next = self.random.next_int(20) as i32 + 10;
                    if ai.state.cube.aggressive {
                        next /= 3;
                    }
                    ai.state.cube.jump_delay = next;
                    hop = true;
                    // `doPlayJumpSound` (every size) at `getSoundPitch`.
                    let (a, b) = (self.random.next_float(), self.random.next_float());
                    let pitch = ((a - b) * 0.2 + 1.0) * if tiny { 1.4 } else { 0.8 };
                    let event = if tiny { "entity.slime.jump_small" } else { "entity.slime.jump" };
                    self.voices.push((Voice::Event(event, volume, pitch), position));
                } else {
                    ai.sideways = 0.0;
                    ai.forward = 0.0;
                    ai.speed = 0.0;
                }
            } else {
                ai.speed = speed;
                ai.forward = speed;
            }
        }
        ai.state.look_control.tick(position, eye_height, ai.body_rotation.body_yaw, false);
        ai.jumping = ai.state.jump || hop;
        body.living_jump(world, fluid, ai.jumping, &mut ai.no_jump_delay, 0.4);
        let input = DVec3::new(f64::from(ai.sideways), 0.0, f64::from(ai.forward));
        let mut fall = None;
        if fluid.in_water() {
            body.travel_water(world, input, ai.yaw);
        } else if fluid.in_lava() {
            body.travel_lava(world, input, ai.yaw, fluid.lava_height);
        } else if let Some(fallen) = body.travel_air_jumping(world, input, ai.speed, ai.yaw, ai.jumping) {
            // A slime is no `Monster`: the generic fall sounds.
            fall = super::living::fall_damage(body, world, fallen, false, &mut self.voices);
        }
        play_movement(body, self.tick_count, &mut self.random, &mut self.voices, SLIME_SOUNDS);
        ai.body_rotation.tick(ai.yaw, &mut ai.state.look_control, self.previous_position, body.position);
        let yaw = ai.yaw;
        self.slime.yaw = yaw;
        if let Some(damage) = fall {
            self.hurt(damage);
        }
    }

    /// `AbstractCubeMob.tick` around the living tick: the squash eases
    /// before it; after it, landing draws its particles' two floats each
    /// (the server draws them too) and plays the squish, taking off
    /// stretches, and the squash relaxes.
    fn land(&mut self) {
        let slime = &mut self.slime;
        let on_ground = slime.body.on_ground;
        if on_ground && !slime.was_on_ground {
            for _ in 0..landing_particles(slime.size) {
                let _ = self.random.next_float();
                let _ = self.random.next_float();
            }
            let (a, b) = (self.random.next_float(), self.random.next_float());
            let pitch = ((a - b) * 0.2 + 1.0) / 0.8;
            let event = if slime.tiny() { "entity.slime.squish_small" } else { "entity.slime.squish" };
            self.voices.push((Voice::Event(event, slime.sound_volume(), pitch), slime.body.position));
            slime.target_squish = -0.5;
        } else if !on_ground && slime.was_on_ground {
            slime.target_squish = 1.0;
        }
        slime.was_on_ground = on_ground;
        slime.target_squish *= 0.6;
    }
}

impl EntityWorld {
    /// A slime with `AbstractCubeMob`'s goals, facing its yaw (NoAI slimes
    /// keep still but still squash, sound and take hits).
    pub fn spawn_slime(&mut self, slime: Slime, no_ai: bool) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let mut ai = MonsterAi::of_kind(MonsterKind::Slime, &slime.body, slime.yaw);
        ai.state.eye_height = slime.eye_height();
        ai.state.max_health = slime.max_health();
        ai.state.tiny = slime.tiny();
        // The move control's first heading: where it faces.
        ai.state.cube.y_rot = slime.yaw;
        self.slimes.push(SlimeEntity {
            effects: Default::default(),
            id,
            previous_position: slime.body.position,
            slime,
            no_ai,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            random: LegacyRandom::new(0),
            ai: Box::new(ai),
        });
        self.order.push(EntityKey::Slime(id));
        self.file_in_section(EntityKey::Slime(id));
        id
    }

    pub fn slimes(&self) -> &[SlimeEntity] {
        &self.slimes
    }

    pub fn slime_mut(&mut self, id: u64) -> Option<&mut SlimeEntity> {
        self.slimes.iter_mut().find(|entity| entity.id == id)
    }

    /// One slime's tick (`AbstractCubeMob.tick` over the living tick), then
    /// the players it touches (`playerTouch`: a slime bigger than 1 with
    /// its AI hurts a player whose touch box, the player's inflated by 1
    /// and 0.5 up and down, meets it, within its melee reach and sight).
    pub(super) fn tick_slime(&mut self, id: u64, world: &impl World, players: &[PlayerCandidate], ticks: &dyn Fn(DVec3) -> bool) {
        let (game_time, difficulty) = (self.game_time, self.difficulty);
        let mobs = self.mob_candidates();
        let Some(entity) = self.slimes.iter_mut().find(|e| e.id == id) else { return };
        entity.ai.state.mobs = mobs;
        entity.slime.previous_squish = entity.slime.squish;
        entity.slime.squish += (entity.slime.target_squish - entity.slime.squish) * 0.5;
        entity.previous_position = entity.slime.body.position;
        entity.tick_count += 1;
        // `Entity.baseTick`'s fluid interaction (the first tick never splashes).
        base_tick_fluid(&mut entity.slime.body, world, entity.tick_count == 1, &mut entity.random, &mut entity.voices, SLIME_SOUNDS);
        super::hazards::burn(entity, world, game_time);
        let eye = entity.slime.eye_height();
        super::hazards::suffocate(entity, world, eye, game_time);
        if entity.slime.health > 0.0 && super::living::breathe(&mut entity.slime.body, world, eye, false) {
            entity.hurt(2.0);
        }
        let removed = entity.slime.damage.tick();
        // `LivingEntity.baseTick` forgets an attacker after 100 ticks.
        if entity.ai.state.hurt_by.is_some_and(|(_, when)| entity.tick_count - when > 100) {
            entity.ai.state.hurt_by = None;
        }
        let max = entity.slime.max_health();
        // `tickEffects`, after the hurt and death clocks.
        for work in entity.effects.tick(false) {
            match work.resolve(entity.slime.health, max) {
                Some(EffectWork::Heal(amount)) => heal(&mut entity.slime.health, max, amount),
                Some(EffectWork::HurtMagic(amount)) => {
                    entity.hurt(amount);
                }
                _ => {}
            }
        }
        // No ambient sound, but `Mob.baseTick` still rolls for one.
        if entity.slime.health > 0.0 {
            if (entity.random.next_int(1000) as i32) < entity.ambient_sound_time {
                entity.ambient_sound_time = -80;
            } else {
                entity.ambient_sound_time += 1;
            }
        }
        let mut active = false;
        if !removed {
            if entity.slime.health <= 0.0 && !entity.no_ai {
                let yaw = entity.slime.yaw;
                let _ = super::living::dying_travel(&mut entity.slime.body, world, yaw, entity.tick_count, &mut entity.random, &mut entity.voices, SLIME_SOUNDS, Some(false));
            }
            if entity.slime.health > 0.0 && !entity.no_ai {
                entity.tick_ai(world, players, game_time, difficulty);
                active = true;
            } else if entity.no_ai {
                entity.slime.body.trim_small_velocity();
            }
            let old = entity.previous_position;
            super::hazards::blocks_act(entity, world, old, game_time);
            self.push_entities(EntityKey::Slime(id), world, game_time, ticks);
        }
        // `AbstractCubeMob.tick` lands after the living tick.
        let Some(entity) = self.slimes.iter_mut().find(|e| e.id == id) else { return };
        entity.land();
        if !active || entity.slime.tiny() {
            return;
        }
        // `playerTouch` → `dealDamage`.
        let body = &entity.slime.body;
        let half = f64::from(body.width) / 2.0;
        let (min, max) = (body.position - DVec3::new(half, 0.0, half), body.position + DVec3::new(half, f64::from(body.height), half));
        let attacker = body.position;
        let damage = entity.slime.attack_damage();
        let mut hits = Vec::new();
        for player in players.iter().filter(|p| p.alive && !p.spectator && p.attackable) {
            let touch_min = player.position - DVec3::new(0.3 + 1.0, 0.5, 0.3 + 1.0);
            let touch_max = player.position + DVec3::new(0.3 + 1.0, 1.8 + 0.5, 0.3 + 1.0);
            let touches = touch_min.x < max.x && touch_max.x > min.x && touch_min.y < max.y && touch_max.y > min.y && touch_min.z < max.z && touch_max.z > min.z;
            if !touches {
                continue;
            }
            let Some(info) = entity.ai.state.info(crate::monster_ai::Target::Player(player.id)) else { continue };
            if entity.ai.state.within_melee_range(info) && entity.ai.state.sees(world, info) {
                hits.push(PlayerHit { player_id: player.id, damage, kind: PlayerHitKind::Melee { attacker, hunger_ticks: 0, lift: 0.0 }, source: Some(id) });
            }
        }
        self.player_hits.extend(hits);
    }

    /// `AbstractCubeMob.remove` for the dead slimes leaving this tick: each
    /// bigger than 1 splits into 2 to 4 of half its size around it, placed
    /// and turned from its random; they keep its persistence and NoAI.
    pub(super) fn split_dead_slimes(&mut self) {
        let dead: Vec<usize> = (0..self.slimes.len()).filter(|&i| self.slimes[i].slime.damage.death_ticks >= 20).collect();
        let mut children = Vec::new();
        for i in dead {
            let parent = &mut self.slimes[i];
            let size = parent.slime.size;
            if size <= 1 {
                continue;
            }
            let (width, _) = crate::slime::dimensions(size);
            let offset = width / 2.0;
            let half = size / 2;
            let count = 2 + parent.random.next_int(3) as i32;
            for n in 0..count {
                let xd = ((n % 2) as f32 - 0.5) * offset;
                let zd = ((n / 2) as f32 - 0.5) * offset;
                let at = parent.slime.body.position + DVec3::new(f64::from(xd), 0.5, f64::from(zd));
                let yaw = parent.random.next_float() * 360.0;
                let mut child = Slime::new(at, half);
                child.yaw = yaw;
                child.persistence_required = parent.slime.persistence_required;
                children.push((child, parent.no_ai));
            }
        }
        for (child, no_ai) in children {
            self.spawn_slime(child, no_ai);
        }
    }
}
