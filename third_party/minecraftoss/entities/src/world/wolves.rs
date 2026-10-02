//! Wolves (26.3 `Wolf`) in the entity world. A wolf ticks as the mobs on
//! the goal framework do: `Entity.baseTick` (water and its splash, fire,
//! walls, drowning, the hurt and death clocks, the forgotten attacker, its
//! effects), `Mob.baseTick`'s ambient roll (a growl while angry, else a
//! pant or whine one time in three, else its bark, at 0.4 volume), its goals
//! and move, a bite that lands (`Mob.doHurtTarget`, no extra knockback),
//! the pack alerted by whoever hurt it, then `AgeableMob`'s growth and
//! `Wolf.aiStep`'s tail: shaking off water once on the ground and still, and
//! its anger renewed while it has a target (`NeutralMob`). `Wolf.tick`'s
//! tail eases its begging tilt and runs the shake, whose sound and droplets
//! draw from its random. Rain is not seen yet.
use super::emissions::{base_tick_fluid, play_movement, MovementSounds};
use super::*;
use crate::monster_ai::{update_enderman_anger, MonsterKind, Target};
use crate::wolf::{self, Wolf};
use minecraftoss_player::inventory::Inventory;

/// Its own step (`Wolf.playStepSound`: the sound set's, at 0.15); the
/// generic swim and splash.
fn wolf_sounds(wolf: &Wolf) -> MovementSounds {
    MovementSounds::creature(Some(wolf.sounds().step))
}

/// A wolf on the monster goal framework.
#[derive(Clone)]
pub struct WolfEntity {
    pub id: u64,
    pub wolf: Wolf,
    pub no_ai: bool,
    pub tick_count: i32,
    pub ambient_sound_time: i32,
    /// Sounds it made since the world last collected them.
    pub voices: Vec<(Voice, DVec3)>,
    pub no_action_time: i32,
    pub random: LegacyRandom,
    pub previous_position: DVec3,
    pub ai: Box<MonsterAi>,
    /// Its status effects (`activeEffects`).
    pub effects: crate::effects::MobEffects,
    /// The type of the damage it last took and when (`lastDamageSource`,
    /// `lastDamageStamp` in game time).
    pub last_damage: Option<(&'static str, i64)>,
}

impl WolfEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.wolf.body.position
    }

    pub fn set_random_seed(&mut self, seed: u64) {
        self.random = LegacyRandom::new(seed);
    }

    /// `NeutralMob.isAngry`: anger time left.
    pub fn angry(&self, game_time: i64) -> bool {
        self.ai.state.enderman.anger_end_time > game_time
    }

    /// `LivingEntity.hurtServer` for a wolf (`Wolf.hurtServer` first stops it
    /// sitting): on a full hit its hurt or death sound (its sound set's, at
    /// 0.4), and the damage and attacker remembered.
    pub fn hurt_from(&mut self, amount: f32, kind: &'static str, attacker: Option<Target>, game_time: i64) -> DamageResult {
        self.wolf.ordered_to_sit = false;
        if self.wolf.health > 0.0 && !self.wolf.damage.dead {
            self.no_action_time = 0;
        }
        let max = self.wolf.max_health();
        let result = self.wolf.damage.hurt_generic(&mut self.wolf.health, max, amount);
        let position = self.position();
        self.wolf.damage.place_death(result, position, self.wolf.body.fire_ticks > 0);
        if result.applied {
            // `Animal.actuallyHurt` ends its love.
            self.wolf.in_love = 0;
            self.last_damage = Some((kind, game_time));
            if let Some(attacker) = attacker {
                self.ai.state.hurt_by = Some((attacker, self.tick_count));
            }
            if result.full {
                if !result.died {
                    self.ambient_sound_time = -wolf::AMBIENT_INTERVAL;
                }
                let sounds = self.wolf.sounds();
                let pitch = voice_pitch(&mut self.random, self.wolf.baby());
                let event = if result.died { sounds.death } else { sounds.hurt };
                self.voices.push((Voice::Event(event, wolf::SOUND_VOLUME, pitch), position));
            }
        }
        result
    }

    /// `hurt_from` with a generic source and no attacker.
    pub fn hurt(&mut self, amount: f32) -> DamageResult {
        self.hurt_from(amount, "minecraft:generic", None, 0)
    }

    /// `Mob.baseTick`'s ambient roll with `Wolf.getAmbientSound`.
    fn ambient(&mut self, game_time: i64) {
        if (self.random.next_int(1000) as i32) >= self.ambient_sound_time {
            self.ambient_sound_time += 1;
            return;
        }
        self.ambient_sound_time = -wolf::AMBIENT_INTERVAL;
        let sounds = self.wolf.sounds();
        let event = if self.angry(game_time) {
            sounds.growl
        } else if self.random.next_int(3) == 0 {
            if self.wolf.tame && self.wolf.health < 20.0 { sounds.whine } else { sounds.pant }
        } else {
            sounds.ambient
        };
        let pitch = voice_pitch(&mut self.random, self.wolf.baby());
        self.voices.push((Voice::Event(event, wolf::SOUND_VOLUME, pitch), self.position()));
    }

    /// `LivingEntity.aiStep` for an active wolf: `Mob.serverAiStep` (goals,
    /// navigation, controls), the jump and travel. Returns the target its
    /// bite struck and where it stood, and how far it fell when it landed.
    fn tick_ai(&mut self, world: &impl World, game_time: i64, owner: Option<u64>, fights: crate::world::PlayerFights, pack_mates: Vec<u64>, mates: Vec<crate::wolf_ai::WolfMate>) -> (Option<(Target, DVec3)>, Option<f64>) {
        let (max_health, eye, sounds) = (self.wolf.max_health(), self.wolf.eye_height(), wolf_sounds(&self.wolf));
        let ai = &mut self.ai;
        let body = &mut self.wolf.body;
        if ai.no_jump_delay > 0 {
            ai.no_jump_delay -= 1;
        }
        body.trim_small_velocity();
        ai.state.body = body.clone();
        ai.state.health = self.wolf.health;
        ai.state.max_health = max_health;
        ai.state.game_time = game_time;
        ai.state.tick_count = self.tick_count;
        ai.state.on_fire = body.fire_ticks > 0;
        ai.state.wolf.tame = self.wolf.tame;
        ai.state.wolf.ordered_to_sit = self.wolf.ordered_to_sit;
        ai.state.wolf.sitting = self.wolf.sitting;
        ai.state.wolf.owner = owner;
        ai.state.wolf.teleported = false;
        ai.state.wolf.owner_hurt_by = fights.hurt_by;
        ai.state.wolf.owner_hurt_mob = fights.hurt_mob;
        ai.state.wolf.owner_damage_recent = fights.last_damage.is_some_and(|(kind, at)| game_time - at <= 100 && kind != "minecraft:sulfur_cube_hot");
        ai.state.wolf.pack_mates = pack_mates;
        ai.state.wolf.mates = mates;
        ai.state.wolf.in_love = self.wolf.in_love > 0;
        // `getLastDamageSource` forgets after 40 ticks.
        ai.state.wolf.last_damage = self.last_damage.filter(|&(_, at)| game_time - at <= 40).map(|(kind, _)| kind);
        ai.state.random = std::mem::take(&mut self.random);
        self.no_action_time += 1;
        let position = body.position;
        let fluid = FluidFrame::sample(world, position, body.width, body.height);
        ai.state.fluid = fluid;
        ai.state.no_action_time = self.no_action_time;
        let full = self.tick_count <= 1 || (self.tick_count + self.id as i32) % 2 == 0;
        ai.tick_goals(world, full);
        let struck = ai.state.attack.take().map(|target| (target, body.position));
        body.position = ai.state.body.position;
        self.wolf.sitting = ai.state.wolf.sitting;
        // A teleport to its owner is a `snapTo`: it has not moved.
        if ai.state.wolf.teleported {
            self.previous_position = body.position;
        }
        // `LeapAtTargetGoal.start`: the leap replaces its motion.
        if let Some(leap) = ai.state.leap.take() {
            body.velocity = leap;
        }
        let position = body.position;
        let (can_update, surface) = crate::navigation::ground_view(world, body, fluid, ai.state.walk.can_float);
        if let Some((wanted, speed)) = ai.state.navigation.tick_in(world, position, can_update, surface, body.width, ai.speed) {
            ai.move_control.set_wanted_position(wanted, speed);
        }
        let interested = ai.state.wolf.interested;
        self.random = std::mem::take(&mut ai.state.random);
        let obstacle_top = crate::control::obstacle_top(world, position);
        let speed = self.effects.movement_speed(wolf::MOVEMENT_SPEED);
        let control = ai.move_control.tick(position, body.on_ground, ai.yaw, ai.speed, ai.forward, ai.sideways, speed, body.width, body.step_height, obstacle_top, |_, _| true);
        ai.yaw = control.yaw;
        ai.speed = control.speed;
        ai.forward = control.forward;
        ai.sideways = control.sideways;
        ai.state.look_control.tick(position, eye, ai.body_rotation.body_yaw, !ai.state.navigation.is_done());
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
        play_movement(body, self.tick_count, &mut self.random, &mut self.voices, sounds);
        ai.body_rotation.tick(ai.yaw, &mut ai.state.look_control, self.previous_position, body.position);
        self.wolf.yaw = ai.yaw;
        self.wolf.interested = interested;
        (struck, landed)
    }

    /// `Wolf.tick`'s tail: the begging tilt eases in or out, and a wet wolf
    /// shaking off plays its shake (its pitch drawn) as it starts and throws
    /// droplets (two draws each) past 0.4 of it, done at 2.
    fn tail(&mut self) {
        if self.wolf.health <= 0.0 {
            return;
        }
        let w = &mut self.wolf;
        w.interested_angle_o = w.interested_angle;
        let goal = if w.interested { 1.0 } else { 0.0 };
        w.interested_angle += (goal - w.interested_angle) * 0.4;
        if w.body.touching_water {
            w.wet = true;
            if w.shaking {
                w.shaking = false;
                w.shake_anim = 0.0;
                w.shake_anim_o = 0.0;
            }
        } else if w.shaking {
            if w.shake_anim == 0.0 {
                let (a, b) = (self.random.next_float(), self.random.next_float());
                self.voices.push((Voice::Event("entity.wolf.shake", wolf::SOUND_VOLUME, (a - b) * 0.2 + 1.0), w.body.position));
            }
            w.shake_anim_o = w.shake_anim;
            w.shake_anim += 0.05;
            if w.shake_anim_o >= 2.0 {
                w.wet = false;
                w.shaking = false;
                w.shake_anim_o = 0.0;
                w.shake_anim = 0.0;
            }
            if w.shake_anim > 0.4 {
                let count = (mth_sin((w.shake_anim - 0.4) * std::f32::consts::PI) * 7.0) as i32;
                for _ in 0..count {
                    let _ = (self.random.next_float(), self.random.next_float());
                }
            }
        }
    }
}

impl EntityWorld {
    /// A wolf with its goals, facing its yaw (a NoAI one keeps still but
    /// still takes hits).
    pub fn spawn_wolf(&mut self, wolf: Wolf, no_ai: bool) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let ai = MonsterAi::of_kind(MonsterKind::Wolf, &wolf.body, wolf.yaw);
        self.wolves.push(WolfEntity {
            id,
            previous_position: wolf.body.position,
            wolf,
            no_ai,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            random: LegacyRandom::new(0),
            ai: Box::new(ai),
            effects: Default::default(),
            last_damage: None,
        });
        self.order.push(EntityKey::Wolf(id));
        self.file_in_section(EntityKey::Wolf(id));
        id
    }

    pub fn wolves(&self) -> &[WolfEntity] {
        &self.wolves
    }

    /// `Animal.spawnChildFromBreeding` with `Wolf.getBreedOffspring`: the
    /// pup (its constructor drawing its turn from its own random, pinned by
    /// the next born seed) takes either parent's variant (the parent's
    /// random), and of a tame parent its owner, the tame's 40 health and the
    /// parents' collars mixed (`DyeColor.getMixedColor`: the 2x1 crafting
    /// recipe of the two dyes, else one of them by the level random), and a
    /// sound variant drawn by the parent; it stands where the parent does,
    /// facing 0 with its head at its turn. Both parents wait 6000 ticks and
    /// fall out of love (event 18); with mob drops the parent draws the
    /// experience it drops.
    fn wolf_breed(&mut self, parent: u64, partner: u64) {
        let Some(partner_state) = self.wolves.iter().find(|e| e.id == partner).map(|e| (e.wolf.variant.clone(), e.wolf.collar)) else { return };
        let recipes = self.recipes.clone();
        let mob_drops = self.mob_drops;
        let mut level_random = std::mem::take(&mut self.level_random);
        let Some(entity) = self.wolves.iter_mut().find(|e| e.id == parent) else {
            self.level_random = level_random;
            return;
        };
        let variant = if entity.random.next_boolean() { entity.wolf.variant.clone() } else { partner_state.0.clone() };
        let tame = entity.wolf.tame.then(|| {
            let (a, b) = (entity.wolf.collar, partner_state.1);
            let mixed = recipes.as_deref().and_then(|book| {
                let grid = [Some(minecraftoss_player::inventory::ItemStack::new(format!("minecraft:{}_dye", wolf::DYES[usize::from(a)]), 1)), Some(minecraftoss_player::inventory::ItemStack::new(format!("minecraft:{}_dye", wolf::DYES[usize::from(b)]), 1))];
                book.matching(&grid, 2, 1).and_then(|result| wolf::dye_color(&result.id))
            });
            (entity.wolf.owner, mixed.unwrap_or_else(|| if level_random.next_boolean() { a } else { b }))
        });
        let sound = wolf::SOUND_VARIANTS[entity.random.next_int(wolf::SOUND_VARIANTS.len() as u32) as usize].to_owned();
        let at = entity.wolf.body.position;
        entity.wolf.set_age(6000);
        entity.wolf.in_love = 0;
        if mob_drops {
            let _ = entity.random.next_int(7);
        }
        self.level_random = level_random;
        if let Some(p) = self.wolves.iter_mut().find(|e| e.id == partner) {
            p.wolf.set_age(6000);
            p.wolf.in_love = 0;
        }
        self.entity_events.push((parent, 18));
        let seed = self.born_seeds.pop_front().unwrap_or_else(|| {
            self.seed_uniquifier = self.seed_uniquifier.wrapping_mul(1_181_783_497_276_652_981);
            self.seed_uniquifier
        });
        let mut random = LegacyRandom::new(seed);
        let turn = random.next_float() * (std::f64::consts::PI * 2.0) as f32;
        let mut pup = Wolf::new(at);
        pup.variant = variant;
        pup.sound_variant = sound;
        pup.persistence_required = false;
        if let Some((owner, collar)) = tame {
            pup.owner = owner;
            pup.set_tame(true, true);
            pup.collar = collar;
        }
        pup.set_age(crate::age::Age::BABY_START);
        pup.yaw = turn;
        let id = self.spawn_wolf(pup, false);
        let entity = self.wolves.iter_mut().find(|e| e.id == id).unwrap();
        entity.random = random;
        // `snapTo(..., 0, 0)`: it faces 0; its head keeps the turn.
        entity.wolf.yaw = 0.0;
        entity.ai.yaw = 0.0;
        entity.ai.body_rotation = crate::look::BodyRotation::new(0.0);
        self.born_wolves.push(id);
    }

    /// `Wolf.mobInteract` for a player's hand. Tame: its food heals it while
    /// hurt (twice the food's nutrition), its owner's dye recolours its
    /// collar, and otherwise `Animal.mobInteract` (food: love, a baby's
    /// growth) or, when that does nothing, its owner's use orders it to sit
    /// or stand. Wild: a bone, unless it is angry, is eaten and tames it one
    /// time in three (40 health, sitting, its target and path dropped);
    /// else `Animal.mobInteract`. Entity events 7 and 6 show hearts or
    /// smoke, 18 love.
    pub fn wolf_interact(&mut self, id: u64, player: u64, inventory: &mut Inventory, hand: usize, infinite_materials: bool) -> crate::cow::InteractionResult {
        use crate::cow::InteractionResult;
        let player_uuid = self.uuid_of(PLAYER_TARGET + player);
        let game_time = self.game_time;
        let mut events = Vec::new();
        let Some(entity) = self.wolves.iter_mut().find(|e| e.id == id) else { return InteractionResult::Pass };
        let item = inventory.slots.get(hand).and_then(Option::as_ref).map_or_else(|| "minecraft:air".to_owned(), |s| s.id.clone());
        let owned = entity.wolf.owner == Some(player_uuid);
        let consume = |inventory: &mut Inventory| {
            if !infinite_materials {
                crate::animal::consume_one(inventory, hand);
            }
        };
        let result = 'result: {
            if entity.wolf.tame {
                if wolf::is_food(&item) && entity.wolf.health < entity.wolf.max_health() {
                    // `TamableAnimal.feed(2, 2)` through `usePlayerItem` (a
                    // stew's bowl comes back).
                    let food = minecraftoss_player::food::catalog().get(&item);
                    let heal = food.map_or(2.0, |f| 2.0 * f32::from(f.nutrition));
                    use_player_item(inventory, hand, infinite_materials, food.and_then(|f| f.remainder.clone()));
                    let max = entity.wolf.max_health();
                    crate::effects::heal(&mut entity.wolf.health, max, heal);
                    break 'result InteractionResult::SuccessPredicted;
                }
                match wolf::dye_color(&item).filter(|_| owned) {
                    None => {
                        let (result, hearts) = animal_interact(entity, inventory, hand, infinite_materials);
                        if hearts {
                            events.push((id, 18));
                        }
                        if result == InteractionResult::Pass && owned {
                            entity.wolf.ordered_to_sit = !entity.wolf.ordered_to_sit;
                            entity.ai.jumping = false;
                            entity.ai.state.navigation.stop();
                            entity.ai.state.clear_target();
                            break 'result InteractionResult::SuccessPredicted;
                        }
                        break 'result result;
                    }
                    Some(color) if color != entity.wolf.collar => {
                        entity.wolf.collar = color;
                        consume(inventory);
                        break 'result InteractionResult::SuccessPredicted;
                    }
                    Some(_) => {}
                }
            } else if item == "minecraft:bone" && !entity.angry(game_time) {
                consume(inventory);
                // `tryToTame`.
                if entity.random.next_int(3) == 0 {
                    entity.wolf.set_tame(true, true);
                    entity.wolf.owner = Some(player_uuid);
                    entity.ai.state.navigation.stop();
                    entity.ai.state.clear_target();
                    entity.wolf.ordered_to_sit = true;
                    events.push((id, 7));
                } else {
                    events.push((id, 6));
                }
                break 'result InteractionResult::SuccessServer;
            }
            let (result, hearts) = animal_interact(entity, inventory, hand, infinite_materials);
            if hearts {
                events.push((id, 18));
            }
            result
        };
        self.entity_events.extend(events);
        result
    }

    pub fn wolf_mut(&mut self, id: u64) -> Option<&mut WolfEntity> {
        self.wolves.iter_mut().find(|entity| entity.id == id)
    }

    /// `HurtByTargetGoal.alertOthers` for a wolf: the other wolves within
    /// follow range around it (ten blocks up and down) with no target and
    /// the same owner turn on the attacker. The lookup finds dying wolves
    /// too (`getEntitiesOfClass` checks no health).
    fn alert_wolves(&mut self, wolf_id: u64, attacker: Target) {
        let Some(source) = self.wolves.iter().find(|e| e.id == wolf_id) else { return };
        let (range, owner) = (source.ai.state.follow_range, source.wolf.owner);
        let p = source.wolf.body.position;
        // `AABB.unitCubeFromLowerCorner(position).inflate(within, 10, within)`.
        let (min, max) = (p - DVec3::new(range, 10.0, range), p + DVec3::new(1.0 + range, 11.0, 1.0 + range));
        for other in &mut self.wolves {
            if other.id == wolf_id || other.wolf.owner != owner {
                continue;
            }
            let b = &other.wolf.body;
            let half = f64::from(b.width / 2.0);
            let q = b.position;
            let touches = q.x - half < max.x && q.x + half > min.x && q.y < max.y && q.y + f64::from(b.height) > min.y && q.z - half < max.z && q.z + half > min.z;
            if touches && other.ai.state.target().is_none() {
                other.ai.state.target = Some(attacker);
            }
        }
    }

    /// One wolf's tick (see the module).
    pub(super) fn tick_wolf(&mut self, id: u64, world: &mut impl World, players: &[PlayerCandidate], ticks: &dyn Fn(DVec3) -> bool) {
        let (game_time, difficulty, griefing) = (self.game_time, self.difficulty, self.mob_griefing);
        // `getOwner`: the player with its owner's UUID, if in the level;
        // what it remembers of its fights; the wolves it shares.
        let owner_uuid = self.wolves.iter().find(|e| e.id == id).and_then(|e| e.wolf.owner);
        let owner = owner_uuid.and_then(|uuid| players.iter().find(|p| self.uuid_of(PLAYER_TARGET + p.id) == uuid)).map(|p| p.id);
        let fights = owner.map(|o| self.player_fights(o)).unwrap_or_default();
        let pack_mates: Vec<u64> = self.wolves.iter().filter(|e| e.id != id && e.wolf.tame && owner_uuid.is_some() && e.wolf.owner == owner_uuid).map(|e| e.id).collect();
        let mates: Vec<crate::wolf_ai::WolfMate> = self
            .wolves
            .iter()
            .filter(|e| e.id != id)
            .map(|e| crate::wolf_ai::WolfMate {
                id: e.id,
                position: e.wolf.body.position,
                width: e.wolf.body.width,
                height: e.wolf.body.height,
                alive: e.wolf.health > 0.0,
                tame: e.wolf.tame,
                sitting: e.wolf.sitting,
                in_love: e.wolf.in_love > 0,
                panicking: e.ai.state.wolf.panic.running,
            })
            .collect();
        let mobs = self.mob_candidates();
        let Some(entity) = self.wolves.iter_mut().find(|e| e.id == id) else { return };
        entity.previous_position = entity.wolf.body.position;
        entity.tick_count += 1;
        let sounds = wolf_sounds(&entity.wolf);
        base_tick_fluid(&mut entity.wolf.body, &*world, entity.tick_count == 1, &mut entity.random, &mut entity.voices, sounds);
        super::hazards::burn(entity, &*world, game_time);
        let eye = entity.wolf.eye_height();
        super::hazards::suffocate(entity, &*world, eye, game_time);
        let water_breathing = entity.effects.has(MobEffect::WaterBreathing);
        if entity.wolf.health > 0.0 && super::living::breathe(&mut entity.wolf.body, &*world, eye, water_breathing) {
            entity.hurt_from(2.0, "minecraft:drown", None, game_time);
        }
        let removed = entity.wolf.damage.tick();
        // `LivingEntity.baseTick` forgets an attacker after 100 ticks.
        if entity.ai.state.hurt_by.is_some_and(|(_, when)| entity.tick_count - when > 100) {
            entity.ai.state.hurt_by = None;
        }
        let max = entity.wolf.max_health();
        for work in entity.effects.tick(false) {
            match work.resolve(entity.wolf.health, max) {
                Some(EffectWork::Heal(amount)) => heal(&mut entity.wolf.health, max, amount),
                Some(EffectWork::HurtMagic(amount)) => {
                    entity.hurt_from(amount, "minecraft:magic", None, game_time);
                }
                _ => {}
            }
        }
        if entity.wolf.health > 0.0 {
            entity.ambient(game_time);
        }
        if removed {
            return;
        }
        let mut struck = None;
        let mut level_random = std::mem::take(&mut self.level_random);
        let Some(entity) = self.wolves.iter_mut().find(|e| e.id == id) else {
            self.level_random = level_random;
            return;
        };
        if entity.wolf.health <= 0.0 && !entity.no_ai {
            let yaw = entity.wolf.yaw;
            let sounds = wolf_sounds(&entity.wolf);
            let landed = super::living::dying_travel(&mut entity.wolf.body, &*world, yaw, entity.tick_count, &mut entity.random, &mut entity.voices, sounds, Some(false));
            if landed.is_some_and(|fallen| super::living::fall_on_farmland(&mut entity.wolf.body, world, fallen, &mut level_random, griefing)) {
                entity.previous_position = entity.wolf.body.position;
            }
        } else if !entity.no_ai {
            let state = &mut entity.ai.state;
            state.difficulty = difficulty;
            state.players = players.to_vec();
            state.mobs = mobs;
            let (hit, landed) = entity.tick_ai(&*world, game_time, owner, fights, pack_mates, mates);
            struck = hit;
            if let Some(fallen) = landed {
                if super::living::fall_on_farmland(&mut entity.wolf.body, world, fallen, &mut level_random, griefing) {
                    entity.previous_position = entity.wolf.body.position;
                }
                let sounds = &mut entity.voices;
                if let Some(damage) = fall_damage(&entity.wolf.body, &*world, fallen, false, sounds) {
                    entity.hurt_from(damage, "minecraft:fall", None, game_time);
                }
            }
        } else {
            entity.wolf.body.trim_small_velocity();
        }
        self.level_random = level_random;
        let Some(entity) = self.wolves.iter_mut().find(|e| e.id == id) else { return };
        let old = entity.previous_position;
        super::hazards::blocks_act(entity, &*world, old, game_time);
        let alert = entity.ai.state.alert.take();
        // Its bite lands as it struck, before it pushes: the victim's hurt
        // and knockback (a wolf has no attack sound).
        if let Some((target, from)) = struck {
            match target {
                Target::Mob(victim) | Target::Villager(victim) => {
                    self.mob_hits_mob(id, victim, wolf::ATTACK_DAMAGE, from, 0.0);
                }
                Target::Player(player_id) => {
                    self.player_hits.push(PlayerHit { player_id, damage: wolf::ATTACK_DAMAGE, kind: PlayerHitKind::Melee { attacker: from, hunger_ticks: 0, lift: 0.0 }, source: Some(id) });
                }
            }
        }
        if let Some(attacker) = alert {
            self.alert_wolves(id, attacker);
        }
        if let Some(partner) = self.wolves.iter_mut().find(|e| e.id == id).and_then(|e| e.ai.state.wolf.breed_with.take()) {
            self.wolf_breed(id, partner);
        }
        self.push_entities(EntityKey::Wolf(id), &*world, game_time, ticks);
        let mobs = self.mob_candidates();
        let Some(entity) = self.wolves.iter_mut().find(|e| e.id == id) else { return };
        // `AgeableMob.aiStep`: a pup grows.
        let was_baby = entity.wolf.baby();
        let _ = entity.wolf.age.tick(entity.wolf.health > 0.0);
        if was_baby != entity.wolf.baby() {
            let ticks = entity.wolf.age.ticks;
            entity.wolf.set_age(ticks);
        }
        // `Animal.aiStep`: love lasts while it is an adult, with a heart
        // (three gaussians and three doubles) every ten ticks.
        if entity.wolf.age.ticks != 0 {
            entity.wolf.in_love = 0;
        } else if entity.wolf.in_love > 0 {
            entity.wolf.in_love -= 1;
            if entity.wolf.in_love % 10 == 0 {
                for _ in 0..3 {
                    let _ = entity.random.next_gaussian();
                }
                for _ in 0..3 {
                    let _ = entity.random.next_double();
                }
            }
        }
        // `Wolf.aiStep`'s tail: a wet wolf on the ground that is not going
        // anywhere starts shaking; `updatePersistentAnger` sees whatever its
        // bite left of its target.
        let w = &mut entity.wolf;
        if w.wet && !w.shaking && entity.ai.state.navigation.is_done() && w.body.on_ground {
            w.shaking = true;
            w.shake_anim = 0.0;
            w.shake_anim_o = 0.0;
        }
        let state = &mut entity.ai.state;
        state.mobs = mobs;
        state.random = std::mem::take(&mut entity.random);
        update_enderman_anger(state);
        entity.random = std::mem::take(&mut state.random);
        entity.tail();
        self.file_in_section(EntityKey::Wolf(id));
    }
}

/// `Animal.mobInteract` for a wolf: its food puts an adult in love or grows
/// a baby. Whether hearts show.
fn animal_interact(entity: &mut WolfEntity, inventory: &mut Inventory, hand: usize, infinite_materials: bool) -> (crate::cow::InteractionResult, bool) {
    let w = &mut entity.wolf;
    let (result, events) = crate::animal::interact(&mut w.age, &mut w.in_love, &mut w.persistence_required, inventory, hand, infinite_materials, wolf::is_food, false);
    let hearts = events.contains(&crate::animal::AnimalEvent::Hearts);
    if w.age.baby() != (w.body.height < wolf::HEIGHT) {
        let ticks = w.age.ticks;
        w.set_age(ticks);
    }
    (result, hearts)
}

/// `Animal.usePlayerItem`: one used (unless the player has infinite
/// materials), and a use remainder (a stew's bowl) takes the emptied hand.
fn use_player_item(inventory: &mut Inventory, hand: usize, infinite_materials: bool, remainder: Option<minecraftoss_player::inventory::ItemStack>) {
    if infinite_materials {
        return;
    }
    crate::animal::consume_one(inventory, hand);
    if let Some(remainder) = remainder {
        if inventory.slots[hand].is_none() {
            inventory.slots[hand] = Some(remainder);
        } else {
            // `handleExtraItemsCreatedOnUse`: into the inventory (what does not
            // fit would drop; a stew is its only case and fills its own slot).
            let _ = inventory.add_item(remainder, hand);
        }
    }
}
