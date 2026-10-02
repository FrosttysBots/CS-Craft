//! Ordered entity state advanced by the caller's shared 20 Hz world tick.
//! Shared entity registry and ordered ticks for measured mob slices.
use crate::age::Age;
use crate::bat::Bat;
use crate::breed::BreedState;
use crate::chicken::Chicken;
use crate::chicken_ai::{
    registered_chicken_goals, ChickenGoalContext, ChickenGoalEffect,
    GOAL_NAMES as CHICKEN_GOAL_NAMES,
};
use crate::control::{minecraft_atan2, MoveControl};
use crate::cow::Cow;
use crate::cow_ai::{
    registered_goals, registered_horse_goals, CowGoalContext, CowGoalEffect, Species, StandState, GOAL_NAMES, HORSE_GOAL_NAMES,
};
use crate::creeper::{Creeper, CreeperExplosion};
use crate::effects::{heal, EffectWork, MobEffect};
use crate::monster_ai::{CreeperAi, MonsterAi};
use minecraftoss_player::survival::EffectKind;

mod combat;
mod merchants;
mod emissions;
mod endermen;
mod golems;
mod wolves;
mod hazards;
mod living;
mod pushing;
mod slimes;
mod villagers;
pub use villagers::VillagerAi;
mod witches;
use living::{breathe, dying_travel, fall_damage};
pub use endermen::EndermanEntity;
pub use golems::IronGolemEntity;
pub use wolves::WolfEntity;
pub use witches::{PlayerSplash, PotionBreak, PotionEntity, WitchEntity};
use emissions::{base_tick_fluid, play_movement, MovementSounds};
pub use slimes::SlimeEntity;
pub use combat::{AttackResult, PlayerAttack};
pub use merchants::{max_xp_for_level, min_xp_for_level, VillagerUse};
use crate::eat_block::{
    edible_for_sheep, registered_sheep_goals, SheepGoalContext, SheepGoalEffect,
    GOAL_NAMES as SHEEP_GOAL_NAMES,
};
use crate::fluid::FluidFrame;
use crate::follow_parent::{CowCandidate, FollowParentState};
use crate::goals::GoalSelector;
use crate::health::{damage_after_armor, DamageResult, DamageState, Death};
use crate::loot::EntityLootContext;
use crate::look::{BodyRotation, LookAtPlayerState, LookControl, RandomLookState};
use crate::mooshroom::{MushroomCow, MushroomCowState, MushroomVariant};
use crate::movement::Body;
use crate::navigation::{navigate_amphibious_to_with_accuracy, navigate_walk_to, GroundNavigation};
use crate::walk_path::WalkProfile;
use minecraftoss_player::path_type::PathType;
use crate::panic::PanicState;
use crate::path_search::MeasuredWaterFloorTerrain;
use crate::pig::Pig;
use crate::pig_ai::{
    registered_pig_goals, PigGoalContext, PigGoalEffect, GOAL_NAMES as PIG_GOAL_NAMES,
};
use crate::projectile::{Arrow, ArrowImpact, ArrowTarget};
use crate::sheep::Sheep;
use crate::skeleton::Skeleton;
use crate::spider::Spider;
use crate::skeleton_bow::{BowMovement, SkeletonBowGoal};
use crate::steering::SteeringState;
use crate::stroll::StrollState;
use crate::tempt::{PlayerCandidate, TemptState};
use crate::villager::Villager;
use crate::zombie::{Zombie, ZombieKind};
use glam::DVec3;
use minecraftoss_player::{
    crafting::RecipeBook, inventory::ItemStack, rng::LegacyRandom, Block, World,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone)]
pub struct CowEntity {
    pub id: u64,
    /// Its status effects (`activeEffects`).
    pub effects: crate::effects::MobEffects,
    pub cow: Cow,
    pub mooshroom: Option<MushroomCowState>,
    /// A horse or donkey: its `AbstractHorse` state.
    pub horse: Option<crate::horse::HorseState>,
    pub previous_position: DVec3,
    pub previous_yaw: f32,
    pub no_ai: bool,
    pub tick_count: i32,
    pub ambient_sound_time: i32,
    /// Sounds it made since the world last collected them.
    pub voices: Vec<(Voice, DVec3)>,
    pub no_action_time: i32,
    pub fluid: FluidFrame,
    pub jumping: bool,
    /// `LivingEntity.noJumpDelay`.
    pub no_jump_delay: i32,
    pub was_touching_water: bool,
    pub last_damage_source: Option<DamageSourceKind>,
    pub last_damage_tick: i32,
    pub follow_parent: FollowParentState,
    pub breed: BreedState,
    pub tempt: TemptState,
    pub random_look: RandomLookState,
    pub look_at_player: LookAtPlayerState,
    pub stroll: StrollState,
    pub panic: PanicState,
    pub random: LegacyRandom,
    pub look_control: LookControl,
    pub body_rotation: BodyRotation,
    goals: GoalSelector<CowGoalContext>,
}

#[derive(Clone)]
pub struct SheepEntity {
    pub id: u64,
    /// Its status effects (`activeEffects`).
    pub effects: crate::effects::MobEffects,
    pub sheep: Sheep,
    pub body: Body,
    pub health: f32,
    pub damage: DamageState,
    pub no_ai: bool,
    pub tick_count: i32,
    pub ambient_sound_time: i32,
    /// Sounds it made since the world last collected them.
    pub voices: Vec<(Voice, DVec3)>,
    pub no_action_time: i32,
    pub last_damage_source: Option<DamageSourceKind>,
    pub last_damage_tick: i32,
    pub fluid: FluidFrame,
    pub jumping: bool,
    /// `LivingEntity.noJumpDelay`.
    pub no_jump_delay: i32,
    pub was_touching_water: bool,
    pub eat_animation_ticks: i32,
    pub random: LegacyRandom,
    pub previous_position: DVec3,
    pub previous_yaw: f32,
    pub navigation: GroundNavigation,
    pub move_control: MoveControl,
    pub look_control: LookControl,
    pub body_rotation: BodyRotation,
    pub yaw: f32,
    pub speed: f32,
    pub forward: f32,
    pub sideways: f32,
    pub breed: BreedState,
    pub tempt: TemptState,
    pub follow_parent: FollowParentState,
    pub random_look: RandomLookState,
    pub look_at_player: LookAtPlayerState,
    pub stroll: StrollState,
    pub panic: PanicState,
    goals: GoalSelector<SheepGoalContext>,
}

#[derive(Clone)]
pub struct PigEntity {
    pub id: u64,
    /// Its status effects (`activeEffects`).
    pub effects: crate::effects::MobEffects,
    pub pig: Pig,
    pub previous_position: DVec3,
    pub previous_yaw: f32,
    pub no_ai: bool,
    pub tick_count: i32,
    pub ambient_sound_time: i32,
    /// Sounds it made since the world last collected them.
    pub voices: Vec<(Voice, DVec3)>,
    pub no_action_time: i32,
    pub last_damage_source: Option<DamageSourceKind>,
    pub last_damage_tick: i32,
    pub fluid: FluidFrame,
    pub was_touching_water: bool,
    pub jumping: bool,
    /// `LivingEntity.noJumpDelay`.
    pub no_jump_delay: i32,
    pub follow_parent: FollowParentState,
    pub breed: BreedState,
    pub steering: SteeringState,
    pub panic: PanicState,
    pub stroll: StrollState,
    pub tempt_stick: TemptState,
    pub tempt_food: TemptState,
    pub random_look: RandomLookState,
    pub look_at_player: LookAtPlayerState,
    pub random: LegacyRandom,
    pub look_control: LookControl,
    pub body_rotation: BodyRotation,
    goals: GoalSelector<PigGoalContext>,
}

#[derive(Clone)]
pub struct ChickenEntity {
    pub id: u64,
    /// Its status effects (`activeEffects`).
    pub effects: crate::effects::MobEffects,
    pub chicken: Chicken,
    pub previous_position: DVec3,
    pub previous_yaw: f32,
    pub no_ai: bool,
    pub tick_count: i32,
    pub ambient_sound_time: i32,
    /// Sounds it made since the world last collected them.
    pub voices: Vec<(Voice, DVec3)>,
    pub no_action_time: i32,
    pub last_damage_source: Option<DamageSourceKind>,
    pub last_damage_tick: i32,
    pub fluid: FluidFrame,
    pub was_touching_water: bool,
    pub jumping: bool,
    /// `LivingEntity.noJumpDelay`.
    pub no_jump_delay: i32,
    pub follow_parent: FollowParentState,
    pub breed: BreedState,
    pub panic: PanicState,
    pub stroll: StrollState,
    pub tempt: TemptState,
    pub random_look: RandomLookState,
    pub look_at_player: LookAtPlayerState,
    pub random: LegacyRandom,
    pub look_control: LookControl,
    pub body_rotation: BodyRotation,
    pub eggs_laid: usize,
    goals: GoalSelector<ChickenGoalContext>,
}

#[derive(Clone)]
pub struct BatEntity {
    pub id: u64,
    /// Its status effects (`activeEffects`).
    pub effects: crate::effects::MobEffects,
    pub bat: Bat,
    pub no_ai: bool,
    pub tick_count: i32,
    pub ambient_sound_time: i32,
    /// Sounds it made since the world last collected them.
    pub voices: Vec<(Voice, DVec3)>,
    pub no_action_time: i32,
    pub previous_position: DVec3,
    pub yaw: f32,
    pub forward: f32,
    pub random: LegacyRandom,
    pub target_position: Option<(i32, i32, i32)>,
    pub look_control: LookControl,
    pub body_rotation: BodyRotation,
}

#[derive(Clone)]
pub struct ZombieEntity {
    pub id: u64,
    /// Its status effects (`activeEffects`).
    pub effects: crate::effects::MobEffects,
    pub zombie: Zombie,
    pub no_ai: bool,
    pub tick_count: i32,
    pub ambient_sound_time: i32,
    /// Sounds it made since the world last collected them.
    pub voices: Vec<(Voice, DVec3)>,
    pub no_action_time: i32,
    underwater_last_tick: bool,
    pub previous_position: DVec3,
    pub random: LegacyRandom,
    pub attack_only: bool,
    pub target_player_id: Option<u64>,
    pub target_villager_id: Option<u64>,
    pub pending_attack_villager: Option<(u64, DVec3)>,
    /// A melee hit on a player this tick: the player and the zombie's position.
    pub pending_attack_player: Option<(u64, DVec3)>,
    pub aggressive: bool,
    pub attack_goal_running: bool,
    pub navigation: GroundNavigation,
    pub move_control: MoveControl,
    pub look_control: LookControl,
    pub body_rotation: BodyRotation,
    pub yaw: f32,
    pub speed: f32,
    pub forward: f32,
    pub sideways: f32,
    pub last_can_use_check: i64,
    pub ticks_until_next_path_recalculation: i32,
    pub ticks_until_next_attack: i32,
    pub pathed_target: DVec3,
    pub raise_arm_ticks: i32,
    /// The zombie's goal and target selectors (`Zombie.registerGoals`), for
    /// zombies on the goal framework.
    pub ai: Option<Box<MonsterAi>>,
}

#[derive(Clone)]
pub struct SkeletonEntity {
    pub id: u64,
    /// Its status effects (`activeEffects`).
    pub effects: crate::effects::MobEffects,
    pub skeleton: Skeleton,
    pub no_ai: bool,
    pub tick_count: i32,
    pub ambient_sound_time: i32,
    /// Sounds it made since the world last collected them.
    pub voices: Vec<(Voice, DVec3)>,
    pub no_action_time: i32,
    pub previous_position: DVec3,
    pub random: LegacyRandom,
    pub target_player_id: Option<u64>,
    pub bow: SkeletonBowGoal,
    pub navigation: GroundNavigation,
    pub move_control: MoveControl,
    pub look_control: LookControl,
    pub body_rotation: BodyRotation,
    pub yaw: f32,
    pub speed: f32,
    pub forward: f32,
    pub sideways: f32,
    /// The skeleton's goal and target selectors
    /// (`AbstractSkeleton.registerGoals`), for skeletons on the goal
    /// framework.
    pub ai: Option<Box<MonsterAi>>,
}

#[derive(Clone)]
pub struct ArrowEntity {
    pub id: u64,
    pub owner_id: u64,
    pub arrow: Arrow,
    /// The effect it carries (`Arrow.addEffect`), with its duration.
    pub effect: Option<(EffectKind, u32)>,
}

/// A spider on the monster goal framework (`Spider.registerGoals`,
/// `WallClimberNavigation`), climbing the walls it meets.
#[derive(Clone)]
pub struct SpiderEntity {
    pub id: u64,
    /// Its status effects (`activeEffects`).
    pub effects: crate::effects::MobEffects,
    pub spider: Spider,
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

impl SpiderEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.spider.body.position
    }

    pub fn set_random_seed(&mut self, seed: u64) {
        self.random = LegacyRandom::new(seed);
    }

    /// `Mob.serverAiStep` and `LivingEntity.aiStep` for an active spider;
    /// returns the target its attack goal hit this tick and where the
    /// spider stood (`Mob.doHurtTarget` runs in the goals, before travel).
    fn tick_ai(&mut self, world: &impl World, players: &[PlayerCandidate], game_time: i64, difficulty: i32) -> Option<(crate::monster_ai::Target, DVec3)> {
        let position = self.spider.body.position;
        let step = MonsterStep { players, villagers: &[], game_time, difficulty, movement_speed: self.effects.movement_speed(crate::spider::movement_speed()), sounds: SPIDER_SOUNDS };
        let landed = monster_ai_step(&mut self.ai, &mut self.spider.body, self.spider.health, &mut self.random, &mut self.voices, &mut self.no_action_time, self.previous_position, self.tick_count, self.id, world, &step);
        self.spider.yaw = self.ai.yaw;
        if let Some(damage) = landed.and_then(|fallen| fall_damage(&self.spider.body, world, fallen, true, &mut self.voices)) {
            self.hurt(damage);
        }
        self.ai.state.attack.take().map(|target| (target, position))
    }

    pub fn hurt(&mut self, amount: f32) -> DamageResult {
        // `hurtServer` resets the idle clock for any hit on a living mob,
        // before the damage cooldown decides.
        if self.spider.health > 0.0 && !self.spider.damage.dead {
            self.no_action_time = 0;
        }
        let result = self.spider.damage.hurt_generic(&mut self.spider.health, crate::spider::MAX_HEALTH, amount);
        let position = self.position();
        self.spider.damage.place_death(result, position, self.spider.body.fire_ticks > 0);
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
}

#[derive(Clone)]
pub struct CreeperEntity {
    pub id: u64,
    /// Its status effects (`activeEffects`).
    pub effects: crate::effects::MobEffects,
    pub creeper: Creeper,
    pub no_ai: bool,
    pub tick_count: i32,
    pub ambient_sound_time: i32,
    /// Sounds it made since the world last collected them.
    pub voices: Vec<(Voice, DVec3)>,
    pub no_action_time: i32,
    pub random: LegacyRandom,
    pub previous_position: DVec3,
    pub ai: CreeperAi,
}

impl CreeperEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.creeper.body.position
    }

    pub fn set_random_seed(&mut self, seed: u64) {
        self.random = LegacyRandom::new(seed);
    }

    /// `Monster.aiStep` (the idle clock runs faster in bright light) and
    /// `LivingEntity.aiStep` for an active creeper: `Mob.serverAiStep`'s
    /// goals, navigation, move and look controls, then the jump, travel and
    /// body turn.
    fn tick_ai(&mut self, world: &impl World, players: &[PlayerCandidate], game_time: i64, difficulty: i32) {
        self.ai.state.swell_dir = self.creeper.swell_dir;
        let step = MonsterStep { players, villagers: &[], game_time, difficulty, movement_speed: self.effects.movement_speed(crate::monster_ai::MOVEMENT_SPEED), sounds: CREEPER_SOUNDS };
        let landed = monster_ai_step(&mut self.ai, &mut self.creeper.body, self.creeper.health, &mut self.random, &mut self.voices, &mut self.no_action_time, self.previous_position, self.tick_count, self.id, world, &step);
        self.creeper.swell_dir = self.ai.state.swell_dir;
        if let Some(damage) = landed.and_then(|fallen| fall_damage(&self.creeper.body, world, fallen, true, &mut self.voices)) {
            self.hurt(damage);
        }
    }

    pub fn hurt(&mut self, amount: f32) -> DamageResult {
        // `hurtServer` resets the idle clock for any hit on a living mob,
        // before the damage cooldown decides.
        if self.creeper.health > 0.0 && !self.creeper.damage.dead {
            self.no_action_time = 0;
        }
        let result = self
            .creeper
            .damage
            .hurt_generic(&mut self.creeper.health, 20.0, amount);
        let position = self.position();
        self.creeper.damage.place_death(result, position, self.creeper.body.fire_ticks > 0);
        // Only a full hit plays the hurt or death sound (`tookFullDamage`).
        if result.applied && result.full {
            if !result.died {
                self.ambient_sound_time = -80;
            }
            let voice = hurt_voice(&mut self.random, result.died, false);
            self.voices.push((voice, self.position()));
        }
        result
    }
}

#[derive(Clone)]
pub struct VillagerEntity {
    pub id: u64,
    /// Its status effects (`activeEffects`).
    pub effects: crate::effects::MobEffects,
    pub villager: Villager,
    pub no_ai: bool,
    /// Its yaw (head and body alike) while it has no AI state to turn it.
    pub yaw: f32,
    pub tick_count: i32,
    pub ambient_sound_time: i32,
    /// Sounds it made since the world last collected them.
    pub voices: Vec<(Voice, DVec3)>,
    pub no_action_time: i32,
    pub previous_position: DVec3,
    pub random: LegacyRandom,
    /// Its brain and controls, when it has AI.
    pub ai: Option<Box<VillagerAi>>,
    /// `lastDamageSource` and `lastDamageStamp`, for its hurt-by sensor.
    pub last_damage: Option<LastDamage>,
    /// `getSleepingPos`: the bed it sleeps in.
    pub sleeping: Option<(i32, i32, i32)>,
    /// Hurt while asleep: it gets up at its next tick (vanilla at once).
    pub wake_pending: bool,
    /// `AbstractVillager.offers`: made when first needed (`getOffers`).
    pub offers: Option<Vec<crate::trading::MerchantOffer>>,
    /// `lastRestockGameTime`, `numberOfRestocksToday` and
    /// `lastRestockCheckDay`.
    pub last_restock: i64,
    pub restocks_today: i32,
    pub last_restock_check_day: i64,
    /// `Villager.gossips`, and `lastGossipDecayTime`.
    pub gossips: Arc<crate::gossip::Gossips>,
    pub last_gossip_decay: i64,
    /// `AbstractVillager.DATA_UNHAPPY_COUNTER`: it shakes its head while
    /// above zero.
    pub unhappy: i32,
    /// The player trading with it (`tradingPlayer`), and the one it last
    /// traded with this tick (`lastTradedPlayer`), by player ID.
    pub trading_player: Option<u64>,
    pub last_traded_player: Option<u64>,
    /// Its main-hand item: an offer's result it shows a player.
    pub held_item: Option<crate::trading::TradeItem>,
    /// Its eight slots and its food level (`foodLevel`).
    pub inventory: crate::villager_inventory::VillagerInventory,
    pub food_level: i32,
    /// `Mob.canPickUpLoot`: set by `Villager`'s constructor, but read as
    /// false from saved data without `CanPickUpLoot` (so a villager
    /// summoned with data picks nothing up unless told to).
    pub can_pick_up_loot: bool,
    /// The item entities its memories name, as last seen: an
    /// `EntityTracker` or memory holds a removed item where it was.
    pub tracked_items: Vec<crate::villager_brain::Seen>,
}

/// A damage source as a brain remembers it: the damage type, the living
/// entity behind it (players as `PLAYER_TARGET` plus theirs), and the
/// game time it landed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LastDamage {
    pub kind: &'static str,
    pub attacker: Option<u64>,
    pub time: i64,
}

#[derive(Clone, Copy)]
struct VillagerCandidate {
    id: u64,
    position: DVec3,
    eye_height: f32,
    width: f32,
    height: f32,
    alive: bool,
}

#[derive(Clone, Copy)]
struct PursuitTarget {
    position: DVec3,
    eye_height: f32,
    width: f32,
    height: f32,
    villager_id: Option<u64>,
    player_id: Option<u64>,
}

/// A mob's hit on a player (`Mob.doHurtTarget`, `AbstractArrow.onHitEntity`),
/// for the player's side to apply (`Player.hurtServer`).
#[derive(Clone, Copy, Debug)]
pub struct PlayerHit {
    pub player_id: u64,
    pub damage: f32,
    pub kind: PlayerHitKind,
    /// The mob behind it (`DamageSource.getEntity`: the biter, the
    /// arrow's shooter, the creeper), which a hurt player remembers
    /// (`setLastHurtByMob`).
    pub source: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
pub enum PlayerHitKind {
    /// A melee attack (`mob_attack`) from the attacker's position; a husk's
    /// bite, if it lands, starves for this many ticks (`Husk.doHurtTarget`),
    /// and an iron golem's throws the player up by `lift` less what
    /// knockback resistance takes (`IronGolem.doHurtTarget`).
    Melee { attacker: DVec3, hunger_ticks: i32, lift: f32 },
    /// An arrow (`arrow`) along its velocity.
    /// An arrow in flight at `velocity`; a tipped one's effect lands with
    /// the hit (`Arrow.doPostHurtEffects`).
    Arrow { velocity: DVec3, effect: Option<(EffectKind, u32)> },
    /// A creeper's blast (`player_explosion`, no default knockback): the
    /// push `ServerExplosion` hands the player (`hitPlayers`).
    Explosion { knockback: DVec3 },
}

/// A mob's death as its drops see it (`LivingEntity.die`,
/// `dropAllDeathLoot`), from [`EntityWorld::take_deaths`].
#[derive(Clone, Debug)]
pub struct MobDeath {
    pub id: u64,
    /// Its loot table (`EntityType.getDefaultLootTable`), unless it drops
    /// none (`shouldDropLoot`: other mobs than monsters only as adults).
    pub table: Option<&'static str>,
    /// Where its drops appear (`Entity.spawnAtLocation`).
    pub position: DVec3,
    /// What its loot tables see.
    pub context: EntityLootContext,
    /// The charged creeper whose blast killed it: the first victim with a
    /// head drops it (`Creeper.killedEntity`), before its own loot.
    pub charged_creeper: Option<u64>,
    /// Equipment the kill shook loose (`Mob.dropCustomDeathLoot`), dropped
    /// after the loot.
    pub equipment: Vec<ItemStack>,
    /// The experience it leaves (`LivingEntity.dropExperience`).
    pub experience: i32,
}

/// `Items.BOW`'s durability.
const BOW_DURABILITY: u32 = 384;

/// A reported death: its order and what its drops see.
fn death_of(id: u64, damage: &mut DamageState, kind: &'static str, table: Option<&'static str>, context: EntityLootContext) -> Option<(u64, MobDeath)> {
    let Death { order, position, killed_by_player, on_fire, attacker, direct, charged_creeper } = damage.death.take()?;
    let context = EntityLootContext { this_type: Some(kind), on_fire, killed_by_player, attacker, has_direct_attacker: direct, ..context };
    Some((order, MobDeath { id, table, position, context, charged_creeper, equipment: Vec::new(), experience: 0 }))
}

/// A sound a mob made, for the client to play: its event, where, volume,
/// pitch and the sound options category it falls under.
#[derive(Clone, Debug, PartialEq)]
pub struct MobSound {
    pub event: String,
    pub position: DVec3,
    pub volume: f32,
    pub pitch: f32,
    pub category: &'static str,
}

/// Which of its sounds a mob made and at what pitch.
#[derive(Clone, Debug, PartialEq)]
pub enum Voice {
    Ambient(f32),
    Hurt(f32),
    Death(f32),
    /// A named event at a volume and pitch.
    Event(&'static str, f32, f32),
    /// A block's step sound at a volume and pitch.
    Step(String, f32, f32),
}

/// A UUID for an entity whose own is not known (vanilla draws fresh ones
/// from an unseeded random): its ID in the low bits of a version-4 UUID.
fn fallback_uuid(id: u64) -> u128 {
    (0x4000u128 << 64) | (0x8000_0000_0000_0000u128) | u128::from(id)
}

/// `LivingEntity.getVoicePitch`: two draws from the mob's random, higher
/// for babies.
/// A zombie's hurt or death sound: `getVoicePitch` with its type's baby
/// voice.
fn zombie_voice(random: &mut LegacyRandom, zombie: &Zombie, died: bool) -> Voice {
    let (a, b) = (random.next_float(), random.next_float());
    let pitch = (a - b) * 0.2 + if zombie.baby { zombie.kind.baby_voice() } else { 1.0 };
    if died { Voice::Death(pitch) } else { Voice::Hurt(pitch) }
}

fn voice_pitch(random: &mut LegacyRandom, baby: bool) -> f32 {
    let (a, b) = (random.next_float(), random.next_float());
    (a - b) * 0.2 + if baby { 1.5 } else { 1.0 }
}

/// `DifficultyInstance.calculateDifficulty`: the difficulty's ID scaled by
/// how long the world has run, how long the chunk has been lived in and
/// the moon's brightness.
pub fn regional_difficulty(difficulty: i32, total_time: i64, inhabited_time: i64, moon_brightness: f32) -> f32 {
    if difficulty == 0 {
        return 0.0;
    }
    let hard = difficulty == 3;
    let global = ((total_time as f32 - 72000.0) / 1_440_000.0).clamp(0.0, 1.0) * 0.25;
    let mut local = (inhabited_time as f32 / 3_600_000.0).clamp(0.0, 1.0) * if hard { 1.0 } else { 0.75 };
    local += (moon_brightness * 0.25).clamp(0.0, global);
    if difficulty == 1 {
        local *= 0.5;
    }
    difficulty as f32 * (0.75 + global + local)
}

/// The hurt or death sound a full hit plays (`playHurtSound`, or the death
/// sound on a fatal one).
fn hurt_voice(random: &mut LegacyRandom, died: bool, baby: bool) -> Voice {
    let pitch = voice_pitch(random, baby);
    if died { Voice::Death(pitch) } else { Voice::Hurt(pitch) }
}

/// A mob's voiced sounds as named events: its sound family, volume
/// (`getSoundVolume`) and category.
fn resolve_voices(out: &mut Vec<MobSound>, voices: &mut Vec<(Voice, DVec3)>, family: &str, volume: f32, category: &'static str) {
    for (voice, position) in voices.drain(..) {
        let (event, volume, pitch) = match voice {
            Voice::Ambient(pitch) => (format!("entity.{family}.ambient"), volume, pitch),
            Voice::Hurt(pitch) => (format!("entity.{family}.hurt"), volume, pitch),
            Voice::Death(pitch) => (format!("entity.{family}.death"), volume, pitch),
            Voice::Event(event, volume, pitch) => (event.to_owned(), volume, pitch),
            Voice::Step(event, volume, pitch) => (event, volume, pitch),
        };
        out.push(MobSound { event, position, volume, pitch, category });
    }
}

/// What a player remembers of its fights, for its tame wolves
/// (`LivingEntity.lastHurtByMob` and `lastHurtMob` with their stamps from
/// its `tickCount`, and `lastDamageSource` with its game time).
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerFights {
    pub tick_count: i32,
    pub hurt_by: Option<(crate::monster_ai::Target, i32)>,
    pub hurt_mob: Option<(crate::monster_ai::Target, i32)>,
    pub last_damage: Option<(&'static str, i64)>,
}

/// Arrow target IDs above this are players (the rest are entity IDs).
const PLAYER_TARGET: u64 = u64::MAX / 2;

impl VillagerEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.villager.body.position
    }

    pub fn set_random_seed(&mut self, seed: u64) {
        self.random = LegacyRandom::new(seed);
    }

    pub fn hurt(&mut self, amount: f32) -> DamageResult {
        // `hurtServer` resets the idle clock for any hit on a living mob,
        // before the damage cooldown decides.
        if self.villager.health > 0.0 && !self.villager.damage.dead {
            self.no_action_time = 0;
        }
        let result = self
            .villager
            .damage
            .hurt_generic(&mut self.villager.health, 20.0, amount);
        let position = self.position();
        self.villager.damage.place_death(result, position, self.villager.body.fire_ticks > 0);
        // Only a full hit plays the hurt or death sound (`tookFullDamage`).
        if result.applied && result.full {
            if !result.died {
                self.ambient_sound_time = -80;
            }
            let voice = hurt_voice(&mut self.random, result.died, self.villager.age.baby());
            self.voices.push((voice, self.position()));
        }
        result
    }

    /// `hurt` from a source: a hit past the damage cooldown becomes the
    /// last damage source.
    pub fn hurt_from(&mut self, amount: f32, kind: &'static str, attacker: Option<u64>, time: i64) -> DamageResult {
        // `hurtServer` wakes a sleeper.
        if self.sleeping.is_some() && self.villager.health > 0.0 {
            self.wake_pending = true;
        }
        let result = self.hurt(amount);
        if result.applied {
            self.last_damage = Some(LastDamage { kind, attacker, time });
        }
        result
    }

    /// `getLastDamageSource()`: the last damage within 40 ticks.
    pub fn recent_damage(&self, time: i64) -> Option<LastDamage> {
        self.last_damage.filter(|d| time - d.time <= 40)
    }

    pub fn knockback_from(&mut self, attacker: DVec3) {
        // LivingEntity.knockback(0.4F, attacker.x - x, attacker.z - z).
        self.knockback_from_components(
            attacker.x - self.villager.body.position.x,
            attacker.z - self.villager.body.position.z,
        );
    }

    pub fn knockback_from_projectile(&mut self, velocity: DVec3) {
        self.knockback_from_components(-velocity.x, -velocity.z);
    }

    fn knockback_from_components(&mut self, mut xd: f64, mut zd: f64) {
        knockback_body(&mut self.villager.body, &mut self.random, &mut xd, &mut zd);
    }
}

fn knockback_body(body: &mut Body, random: &mut LegacyRandom, xd: &mut f64, zd: &mut f64) {
    while *xd * *xd + *zd * *zd < 1.0e-5_f32 as f64 {
        *xd = (random.next_double() - random.next_double()) * 0.01;
        *zd = (random.next_double() - random.next_double()) * 0.01;
    }
    let length = (*xd * *xd + *zd * *zd).sqrt();
    let power = 0.4_f32 as f64;
    body.velocity.x = body.velocity.x / 2.0 - *xd / length * power;
    body.velocity.z = body.velocity.z / 2.0 - *zd / length * power;
    if body.on_ground {
        body.velocity.y = (body.velocity.y / 2.0 + power).min(0.4);
    }
    body.needs_sync = true;
}

/// `Mob.burnUndead` (26.3) for a mob in `#burn_in_daylight`: while the
/// `gameplay/monsters_burn` attribute holds, a roll against the light at
/// the eyes, out of water and rain, under open sky, sets it on fire for
/// eight seconds; an item on its head takes the sun instead (a damage roll
/// for damageable ones).
fn burn_undead(world: &impl World, random: &mut LegacyRandom, body: &mut Body, eye_height: f32, monsters_burn: bool, head_item: Option<bool>) {
    if !monsters_burn {
        return;
    }
    let p = body.position;
    let eye = ((p.x.floor()) as i32, (p.y + f64::from(eye_height)).floor() as i32, p.z.floor() as i32);
    // `getLightLevelDependentMagicValue` at the eyes.
    let magic = world.light_path_cost(eye) + 0.5;
    if magic <= 0.5 {
        return;
    }
    if random.next_float() * 30.0 >= (magic - 0.4) * 2.0 {
        return;
    }
    let feet = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
    let top = (feet.0, (p.y + f64::from(body.height)).floor() as i32, feet.2);
    let in_water = crate::fluid::FluidFrame::sample(world, p, body.width, body.height).in_water();
    if in_water || world.rain_at(feet) || world.rain_at(top) || !world.can_see_sky(eye) {
        return;
    }
    match head_item {
        Some(true) => {
            let _ = random.next_int(2);
        }
        Some(false) => {}
        // `igniteForSeconds(8)`.
        None => body.fire_ticks = body.fire_ticks.max(160),
    }
}

/// `Sheep.playStepSound`.
const SHEEP_SOUNDS: MovementSounds = MovementSounds::creature(Some("entity.sheep.step"));

/// `Chicken.playStepSound`: its sound set's step, a baby's own.
fn chicken_sounds(baby: bool) -> MovementSounds {
    MovementSounds::creature(Some(if baby { "entity.baby_chicken.step" } else { "entity.chicken.step" }))
}

/// `Pig.playStepSound`: its sound set's step, a baby's own (every adult
/// variant steps as the classic pig).
fn pig_sounds(baby: bool) -> MovementSounds {
    MovementSounds::creature(Some(if baby { "entity.baby_pig.step" } else { "entity.pig.step" }))
}

/// `AbstractCow.playStepSound`: its sound set's step (mooshrooms keep the
/// classic set).
fn cow_sounds(mooshroom: bool, variant: crate::cow::CowSoundVariant) -> MovementSounds {
    let moody = !mooshroom && variant == crate::cow::CowSoundVariant::Moody;
    MovementSounds::creature(Some(if moody { "entity.cow_moody.step" } else { "entity.cow.step" }))
}

impl ChickenEntity {
    fn movement_sounds(&self) -> MovementSounds {
        chicken_sounds(self.chicken.age.baby())
    }
}

impl PigEntity {
    fn movement_sounds(&self) -> MovementSounds {
        pig_sounds(self.pig.age.baby())
    }
}

impl CowEntity {
    fn movement_sounds(&self) -> MovementSounds {
        if let Some(horse) = &self.horse {
            let family = horse.kind.sound_family(self.cow.age.baby());
            let step = match family {
                "baby_horse" => "entity.baby_horse.step",
                _ => "entity.horse.step",
            };
            return MovementSounds { horse_step: Some((step, "entity.horse.step_wood")), ..MovementSounds::creature(None) };
        }
        cow_sounds(self.mooshroom.is_some(), self.cow.sound_variant)
    }

    /// `getAmbientSoundInterval`.
    fn ambient_interval(&self) -> i32 {
        if self.horse.is_some() { crate::horse::AMBIENT_SOUND_INTERVAL } else { 120 }
    }

    /// Its eye height.
    fn eye_height(&self) -> f32 {
        match (&self.horse, self.cow.age.baby(), self.mooshroom.is_some()) {
            (Some(horse), baby, _) => horse.kind.eye_height(baby),
            (None, false, _) => 1.3,
            (None, true, true) => 0.69,
            (None, true, false) => 0.665,
        }
    }
}

/// A spider's own step (`Spider.playStepSound`).
const SPIDER_SOUNDS: MovementSounds = MovementSounds::monster(Some("entity.spider.step"));
/// A creeper steps with its block's sound.
const CREEPER_SOUNDS: MovementSounds = MovementSounds::monster(None);

impl ZombieKind {
    /// `Zombie.playStepSound` with its type's `getStepSound`; the drowned
    /// swims with its own sound.
    fn movement_sounds(self) -> MovementSounds {
        let mut sounds = MovementSounds::monster(Some(match self {
            Self::Zombie => "entity.zombie.step",
            Self::Husk => "entity.husk.step",
            Self::ZombieVillager => "entity.zombie_villager.step",
            Self::Drowned => "entity.drowned.step",
        }));
        if self == Self::Drowned {
            sounds.swim = "entity.drowned.swim";
        }
        sounds
    }
}

impl crate::skeleton::SkeletonKind {
    /// `AbstractSkeleton.playStepSound` with its type's `getStepSound`.
    fn movement_sounds(self) -> MovementSounds {
        use crate::skeleton::SkeletonKind;
        MovementSounds::monster(Some(match self {
            SkeletonKind::Skeleton => "entity.skeleton.step",
            SkeletonKind::Stray => "entity.stray.step",
            SkeletonKind::Bogged => "entity.bogged.step",
            SkeletonKind::Parched => "entity.parched.step",
        }))
    }
}

/// What a monster's AI step reads from its world this tick.
struct MonsterStep<'a> {
    players: &'a [PlayerCandidate],
    villagers: &'a [crate::monster_ai::MobCandidate],
    game_time: i64,
    difficulty: i32,
    /// `MOVEMENT_SPEED` with its modifiers.
    movement_speed: f64,
    /// Its step, swim and splash sounds.
    sounds: MovementSounds,
}

/// `Mob.checkDespawn` and `Monster.updateNoActionTime` for a NoAI monster:
/// a player within 32 blocks resets its idle clock, and bright light at its
/// eyes runs the clock two ticks at a time (no goals add the third).
fn monster_idle_without_ai(world: &impl World, players: &[PlayerCandidate], position: DVec3, eye_height: f32, no_action_time: &mut i32) {
    if players.iter().any(|player| player.alive && !player.spectator && player.position.distance_squared(position) < 32.0 * 32.0) {
        *no_action_time = 0;
    }
    let eye = (position.x.floor() as i32, (position.y + f64::from(eye_height)).floor() as i32, position.z.floor() as i32);
    if world.light_path_cost(eye) > 0.0 {
        *no_action_time += 2;
    }
}

/// `Monster.aiStep` (the idle clock runs faster in bright light) and
/// `LivingEntity.aiStep` for a monster on the goal framework:
/// `Mob.serverAiStep`'s goals, navigation, move and look controls, then the
/// jump, travel and body turn.
#[allow(clippy::too_many_arguments)]
fn monster_ai_step(ai: &mut MonsterAi, body: &mut Body, health: f32, random: &mut LegacyRandom, voices: &mut Vec<(Voice, DVec3)>, no_action_time: &mut i32, previous_position: DVec3, tick_count: i32, id: u64, world: &impl World, step: &MonsterStep) -> Option<f64> {
    let position = body.position;
    let eye_height = ai.state.eye_height;
    // `Mob.checkDespawn` runs first: a player within 32 blocks resets the
    // idle clock.
    if step.players.iter().any(|player| player.alive && !player.spectator && player.position.distance_squared(position) < 32.0 * 32.0) {
        *no_action_time = 0;
    }
    let eye = (position.x.floor() as i32, (position.y + f64::from(eye_height)).floor() as i32, position.z.floor() as i32);
    if world.light_path_cost(eye) > 0.0 {
        *no_action_time += 2;
    }
    if ai.no_jump_delay > 0 {
        ai.no_jump_delay -= 1;
    }
    body.trim_small_velocity();
    *no_action_time += 1;
    let fluid = FluidFrame::sample(world, position, body.width, body.height);
    ai.state.body = body.clone();
    ai.state.health = health;
    ai.state.game_time = step.game_time;
    ai.state.difficulty = step.difficulty;
    ai.state.players = step.players.to_vec();
    ai.state.villagers = step.villagers.to_vec();
    ai.state.fluid = fluid;
    ai.state.no_action_time = *no_action_time;
    ai.state.random = std::mem::take(random);
    let full = tick_count <= 1 || (tick_count + id as i32) % 2 == 0;
    ai.tick_goals(world, full);
    *random = std::mem::take(&mut ai.state.random);
    if let Some(velocity) = ai.state.leap.take() {
        body.velocity = velocity;
    }
    let (can_update, surface) = crate::navigation::ground_view(world, body, fluid, ai.state.walk.can_float);
    if let Some((wanted, speed)) = ai.state.navigation.tick_in(world, position, can_update, surface, body.width, ai.speed) {
        ai.move_control.set_wanted_position(wanted, speed);
    }
    let obstacle_top = crate::control::obstacle_top(world, position);
    // `LivingEntity.applyInput`, before the AI: last tick's inputs fade. A
    // move leaves the sideways input a strafe set, so it keeps fading.
    ai.sideways *= 0.98;
    ai.forward *= 0.98;
    let control = ai.move_control.tick(
        position,
        body.on_ground,
        ai.yaw,
        ai.speed,
        ai.forward,
        ai.sideways,
        step.movement_speed,
        body.width,
        body.step_height,
        obstacle_top,
        |_, _| true,
    );
    ai.yaw = control.yaw;
    ai.speed = control.speed;
    ai.forward = control.forward;
    ai.sideways = control.sideways;
    ai.state.look_control.tick(position, eye_height, ai.body_rotation.body_yaw, !ai.state.navigation.is_done());
    ai.jumping = ai.state.jump || control.jump;
    body.living_jump(world, fluid, ai.jumping, &mut ai.no_jump_delay, 0.4);
    let input = DVec3::new(f64::from(ai.sideways), 0.0, f64::from(ai.forward));
    let landed = if fluid.in_water() {
        body.travel_water(world, input, ai.yaw);
        None
    } else if fluid.in_lava() {
        body.travel_lava(world, input, ai.yaw, fluid.lava_height);
        None
    } else {
        body.travel_air_jumping(world, input, ai.speed, ai.yaw, ai.jumping)
    };
    play_movement(body, tick_count, random, voices, step.sounds);
    ai.body_rotation.tick(ai.yaw, &mut ai.state.look_control, previous_position, body.position);
    landed
}

/// `ServerExplosion.hurtEntities` for one body: the unit direction from the
/// centre to its eyes (`Vec3.normalize`, which divides) and the share of it
/// the blast sees (`getSeenPercent` over its bounding box).
fn explosion_exposure(world: &impl World, body: &Body, eye_height: f32, center: DVec3) -> (DVec3, f32) {
    let offset = body.position + DVec3::new(0.0, f64::from(eye_height), 0.0) - center;
    let length = (offset.x * offset.x + offset.y * offset.y + offset.z * offset.z).sqrt();
    let direction = if length < f64::from(1.0e-5_f32) { DVec3::ZERO } else { DVec3::new(offset.x / length, offset.y / length, offset.z / length) };
    let half = f64::from(body.width / 2.0);
    let p = body.position;
    let min = DVec3::new(p.x - half, p.y, p.z - half);
    let max = DVec3::new(p.x + half, p.y + f64::from(body.height), p.z + half);
    (direction, crate::sight::seen_percent(world, center, min, max))
}

fn projectile_knockback(body: &mut Body, random: &mut LegacyRandom, velocity: DVec3) {
    let mut xd = -velocity.x;
    let mut zd = -velocity.z;
    knockback_body(body, random, &mut xd, &mut zd);
}

impl SkeletonEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.skeleton.body.position
    }

    pub fn set_random_seed(&mut self, seed: u64) {
        self.random = LegacyRandom::new(seed);
    }

    pub fn hurt(&mut self, amount: f32) -> DamageResult {
        // `hurtServer` resets the idle clock for any hit on a living mob,
        // before the damage cooldown decides.
        if self.skeleton.health > 0.0 && !self.skeleton.damage.dead {
            self.no_action_time = 0;
        }
        let result = self
            .skeleton
            .damage
            .hurt_generic(&mut self.skeleton.health, self.skeleton.kind.max_health(), amount);
        let position = self.position();
        self.skeleton.damage.place_death(result, position, self.skeleton.body.fire_ticks > 0);
        // Only a full hit plays the hurt or death sound (`tookFullDamage`).
        if result.applied && result.full {
            if !result.died {
                self.ambient_sound_time = -80;
            }
            let voice = hurt_voice(&mut self.random, result.died, false);
            self.voices.push((voice, self.position()));
        }
        result
    }

    /// A skeleton on the goal framework: its AI step, then the arrow its
    /// bow goal loosed (from where it stood when the goal ran) and the
    /// shot's sound pitch (`performRangedAttack`).
    #[allow(clippy::too_many_arguments)]
    fn tick_monster_ai(
        &mut self,
        world: &impl World,
        players: &[PlayerCandidate],
        game_time: i64,
        difficulty: i32,
        bright_outside: bool,
        arrow_shoot_seed: Option<u64>,
        arrow_damage_seed: Option<u64>,
        world_random: &mut LegacyRandom,
    ) -> Option<Arrow> {
        let position = self.skeleton.body.position;
        let ai = self.ai.as_deref_mut().unwrap();
        ai.state.bright_outside = bright_outside;
        ai.state.on_fire = self.skeleton.body.fire_ticks > 0;
        ai.state.helmet = self.skeleton.head_item.is_some();
        // `AbstractSkeleton.createAttributes`: 0.25 speed.
        let step = MonsterStep { players, villagers: &[], game_time, difficulty, movement_speed: self.effects.movement_speed(0.25), sounds: self.skeleton.kind.movement_sounds() };
        let landed = monster_ai_step(ai, &mut self.skeleton.body, self.skeleton.health, &mut self.random, &mut self.voices, &mut self.no_action_time, self.previous_position, self.tick_count, self.id, world, &step);
        // The gates, renderer and census read the skeleton's own fields.
        self.yaw = ai.yaw;
        self.speed = ai.speed;
        self.forward = ai.forward;
        self.sideways = ai.sideways;
        self.body_rotation = ai.body_rotation.clone();
        self.look_control = ai.state.look_control.clone();
        self.navigation = ai.state.navigation.clone();
        self.move_control = ai.move_control.clone();
        self.bow.aggressive = ai.state.melee.aggressive;
        self.bow.using_item = ai.state.bow.using_item;
        self.bow.ticks_using_item = ai.state.bow.ticks_using_item;
        self.bow.attack_time = ai.state.bow.attack_time;
        self.bow.see_time = ai.state.bow.see_time;
        self.target_player_id = match ai.state.target {
            Some(crate::monster_ai::Target::Player(id)) if ai.state.target().is_some() => Some(id),
            _ => None,
        };
        // The bow goal loosed before the move whose landing hurts.
        let shot = self.loose_arrow(world, position, difficulty, arrow_shoot_seed, arrow_damage_seed, world_random);
        if let Some(damage) = landed.and_then(|fallen| fall_damage(&self.skeleton.body, world, fallen, true, &mut self.voices)) {
            self.hurt(damage);
        }
        shot
    }

    /// The arrow the bow goal loosed this tick, if it did (`performRangedAttack`).
    fn loose_arrow(&mut self, _world: &impl World, position: DVec3, difficulty: i32, arrow_shoot_seed: Option<u64>, arrow_damage_seed: Option<u64>, world_random: &mut LegacyRandom) -> Option<Arrow> {
        let ai = self.ai.as_deref_mut()?;
        let (target, power) = ai.state.shoot.take()?;
        let target = ai.state.info(target)?;
        let mut random = LegacyRandom::new(arrow_shoot_seed.unwrap_or_else(|| world_random.next_long()));
        // `rangedAttackUncertainty`: 14 less 4 per difficulty step.
        let uncertainty = (14 - 4 * difficulty) as f32;
        // Aimed a third of the way up the target's box (`getY(1/3)`).
        let mut shot = Arrow::skeleton_shot(position, self.skeleton.eye_height(), target.position, target.height, uncertainty, &mut random);
        // `setBaseDamageFromMob`: twice the power and the difficulty's
        // triangle, from the new arrow's own random (pinned by the harness).
        let seed = arrow_damage_seed.unwrap_or_else(|| world_random.next_long());
        shot.set_base_damage_from_mob(1.0, difficulty as u32, &mut LegacyRandom::new(seed));
        let _ = power;
        // `AbstractSkeleton.performRangedAttack`'s sound: 1 / (0.8..1.2).
        let pitch = 1.0 / (self.random.next_float() * 0.4 + 0.8);
        self.voices.push((Voice::Event("entity.skeleton.shoot", 1.0, pitch), position));
        Some(shot)
    }

    fn tick_bow(
        &mut self,
        world: &impl World,
        players: &[PlayerCandidate],
        arrow_shoot_seed: Option<u64>,
        arrow_damage_seed: Option<u64>,
        world_random: &mut LegacyRandom,
    ) -> Option<Arrow> {
        let position = self.skeleton.body.position;
        if players.iter().any(|player| {
            player.alive
                && !player.spectator
                && player.position.distance_squared(position) < 32.0 * 32.0
        }) {
            self.no_action_time = 0;
        }
        self.no_action_time += 1;
        let full_goal_tick = self.tick_count <= 1 || (self.tick_count + self.id as i32) % 2 == 0;
        if full_goal_tick {
            if self.target_player_id.is_some_and(|id| {
                !players
                    .iter()
                    .any(|player| player.id == id && player.alive && player.attackable)
            }) {
                self.target_player_id = None;
            }
            if self.target_player_id.is_none() {
                if self.random.next_int(5) == 0 {
                    self.target_player_id = players
                        .iter()
                        .filter(|player| player.alive && player.attackable)
                        .filter(|player| player.position.distance_squared(position) <= 35.0 * 35.0)
                        .min_by(|a, b| {
                            a.position
                                .distance_squared(position)
                                .total_cmp(&b.position.distance_squared(position))
                        })
                        .map(|player| player.id);
                }
                if self.target_player_id.is_none() {
                    // The two lower-priority golem and baby-turtle target
                    // goals still trial in the isolated no-other-target scene.
                    let _ = self.random.next_int(5);
                    let _ = self.random.next_int(5);
                }
            }
        }
        let target = players
            .iter()
            .find(|player| Some(player.id) == self.target_player_id && player.alive);
        let bow_tick = self.bow.tick(
            target.is_some(),
            true,
            self.navigation.is_done(),
            target.map_or(f64::INFINITY, |target| {
                position.distance_squared(target.position)
            }),
            true, // This isolated fixture has clear sight; world sensing follows.
            &mut self.random,
        );
        let mut arrow = None;
        if let (Some(target), Some(bow_tick)) = (target, bow_tick) {
            match bow_tick.movement {
                BowMovement::Navigate { speed } => {
                    let profile = monster_profile(&self.skeleton.body, 16.0, self.skeleton.health, 20.0, true);
                    let _ = navigate_walk_to(
                        &self.skeleton.body,
                        &mut self.navigation,
                        world,
                        &profile,
                        FluidFrame::default(),
                        target.position,
                        speed,
                        1,
                    );
                    self.look_control.set_look_at_with_limits(
                        target.position + DVec3::new(0.0, f64::from(target.eye_height), 0.0),
                        30.0,
                        30.0,
                    );
                }
                BowMovement::Strafe { forward, sideways } => {
                    self.navigation.stop();
                    self.move_control.strafe(forward, sideways);
                    let dx = target.position.x - position.x;
                    let dz = target.position.z - position.z;
                    let desired = (minecraft_atan2(dz, dx) * 57.2957763671875_f64) as f32 - 90.0;
                    self.yaw = mob_rotlerp(self.yaw, desired, 30.0);
                }
            }
            if bow_tick.shoot_power.is_some() {
                let mut random =
                    LegacyRandom::new(arrow_shoot_seed.unwrap_or_else(|| world_random.next_long()));
                let mut shot = Arrow::skeleton_shot(
                    position,
                    self.skeleton.eye_height(),
                    target.position,
                    1.8,
                    6.0,
                    &mut random,
                );
                if let Some(seed) = arrow_damage_seed {
                    shot.set_base_damage_from_mob(1.0, 2, &mut LegacyRandom::new(seed));
                }
                arrow = Some(shot);
                // AbstractSkeleton.playSound samples the shoot pitch.
                let _ = self.random.next_float();
            }
        }
        if let Some((wanted, speed)) = self.navigation.tick(
            world,
            position,
            self.skeleton.body.on_ground,
            self.skeleton.body.width,
            self.speed,
        ) {
            self.move_control.set_wanted_position(wanted, speed);
        }
        let obstacle_top = crate::control::obstacle_top(world, position);
        // `LivingEntity.applyInput`: last tick's inputs fade.
        self.sideways *= 0.98;
        self.forward *= 0.98;
        let control = self.move_control.tick(
            position,
            self.skeleton.body.on_ground,
            self.yaw,
            self.speed,
            self.forward,
            self.sideways,
            0.25_f32 as f64,
            self.skeleton.body.width,
            self.skeleton.body.step_height,
            obstacle_top,
            |_, _| true,
        );
        self.yaw = control.yaw;
        self.speed = control.speed;
        self.forward = control.forward;
        self.sideways = control.sideways;
        self.look_control.tick(
            position,
            self.skeleton.eye_height(),
            self.body_rotation.body_yaw,
            !self.navigation.is_done(),
        );
        let fluid = FluidFrame::sample(
            world,
            position,
            self.skeleton.body.width,
            self.skeleton.body.height,
        );
        let input = DVec3::new(self.sideways as f64, 0.0, self.forward as f64);
        if fluid.in_water() {
            self.skeleton.body.travel_water(world, input, self.yaw);
        } else if fluid.in_lava() {
            self.skeleton
                .body
                .travel_lava(world, input, self.yaw, fluid.lava_height);
        } else if let Some(fallen) = self.skeleton.body.travel_air(world, input, self.speed, self.yaw) {
            if let Some(damage) = fall_damage(&self.skeleton.body, world, fallen, true, &mut self.voices) {
                self.hurt(damage);
            }
        }
        let sounds = self.skeleton.kind.movement_sounds();
        play_movement(&mut self.skeleton.body, self.tick_count, &mut self.random, &mut self.voices, sounds);
        self.body_rotation.tick(
            self.yaw,
            &mut self.look_control,
            self.previous_position,
            self.skeleton.body.position,
        );
        arrow
    }
}

fn mob_rotlerp(from: f32, to: f32, max: f32) -> f32 {
    let mut diff = (to - from) % 360.0;
    if diff >= 180.0 {
        diff -= 360.0;
    }
    if diff < -180.0 {
        diff += 360.0;
    }
    from + diff.clamp(-max, max)
}

/// A monster's walk search: no fluid floating, the given follow range, and
/// `Mob.getMaxFallDistance` with a target on Normal difficulty.
fn monster_profile(body: &Body, follow_range: f32, health: f32, max_health: f32, has_target: bool) -> WalkProfile {
    let mut profile = WalkProfile::animal(body.width, body.height);
    profile.clear_malus(PathType::FireInNeighbor);
    profile.clear_malus(PathType::Fire);
    profile.can_float = false;
    profile.follow_range = follow_range;
    if has_target {
        let difficulty = 2;
        let sacrifice = ((health - max_health * 0.33) as i32 - (3 - difficulty) * 4).max(0);
        profile.max_fall_distance = (sacrifice as f32 + 3.0).floor() as i32;
    }
    profile
}

impl ZombieEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.zombie.body.position
    }

    fn navigate_to_pursuit_target(&mut self, world: &impl World, target: DVec3) -> Option<bool> {
        if self.zombie.kind == ZombieKind::Drowned {
            navigate_amphibious_to_with_accuracy(
                &self.zombie.body,
                &mut self.navigation,
                &MeasuredWaterFloorTerrain(world),
                target,
                1.0,
                0,
            )
        } else {
            let profile = monster_profile(&self.zombie.body, 35.0, self.zombie.health, 20.0, true);
            navigate_walk_to(&self.zombie.body, &mut self.navigation, world, &profile, FluidFrame::default(), target, 1.0, 0)
        }
    }

    pub fn set_random_seed(&mut self, seed: u64) {
        self.random = LegacyRandom::new(seed);
    }

    pub fn hurt(&mut self, amount: f32) -> DamageResult {
        // `hurtServer` resets the idle clock for any hit on a living mob,
        // before the damage cooldown decides.
        if self.zombie.health > 0.0 && !self.zombie.damage.dead {
            self.no_action_time = 0;
        }
        let result = self
            .zombie
            .damage
            .hurt_generic(&mut self.zombie.health, 20.0, amount);
        let position = self.position();
        self.zombie.damage.place_death(result, position, self.zombie.body.fire_ticks > 0);
        // Only a full hit plays the hurt or death sound (`tookFullDamage`).
        if result.applied && result.full {
            if !result.died {
                self.ambient_sound_time = -80;
            }
            // LivingEntity plays either hurt or death with two pitch draws;
            // only the nonfatal Mob.playHurtSound path resets ambient time.
            let voice = zombie_voice(&mut self.random, &self.zombie, result.died);
            self.voices.push((voice, self.position()));
        }
        result
    }

    fn travel_controlled(&mut self, world: &impl World, wants_to_swim: bool) {
        let fluid = FluidFrame::sample(
            world,
            self.zombie.body.position,
            self.zombie.body.width,
            self.zombie.body.height,
        );
        let input = DVec3::new(self.sideways as f64, 0.0, self.forward as f64);
        // `Drowned.updateSwimming`: under water and wanting to swim.
        self.zombie.body.swimming = self.zombie.kind == ZombieKind::Drowned && self.underwater_last_tick && wants_to_swim;
        if fluid.in_water() {
            if self.zombie.kind == ZombieKind::Drowned && self.underwater_last_tick && wants_to_swim
            {
                self.zombie
                    .body
                    .travel_drowned_swimming(world, input, self.yaw);
            } else {
                self.zombie.body.travel_water(world, input, self.yaw);
            }
        } else if fluid.in_lava() {
            self.zombie
                .body
                .travel_lava(world, input, self.yaw, fluid.lava_height);
        } else if let Some(fallen) = self.zombie.body.travel_air(world, input, self.speed, self.yaw) {
            if let Some(damage) = fall_damage(&self.zombie.body, world, fallen, true, &mut self.voices) {
                self.hurt(damage);
            }
        }
        let sounds = self.zombie.kind.movement_sounds();
        play_movement(&mut self.zombie.body, self.tick_count, &mut self.random, &mut self.voices, sounds);
    }

    /// Source: 26.3 NearestAttackableTargetGoal, MeleeAttackGoal, ZombieAttackGoal,
    /// DrownedAttackGoal, and Mob.serverAiStep. The caller isolates the relevant
    /// melee action goal while leaving the target selector registered.
    fn tick_pursuit(
        &mut self,
        world: &impl World,
        players: &[PlayerCandidate],
        villagers: &[VillagerCandidate],
        game_time: i64,
        bright_outside: bool,
    ) {
        let position = self.zombie.body.position;
        if players.iter().any(|player| {
            player.alive
                && !player.spectator
                && player.position.distance_squared(position) < 32.0 * 32.0
        }) {
            self.no_action_time = 0;
        }
        self.no_action_time += 1;
        let full_goal_tick = self.tick_count <= 1 || (self.tick_count + self.id as i32) % 2 == 0;
        if full_goal_tick {
            if let Some(target_id) = self.target_player_id {
                if !players.iter().any(|player| {
                    player.id == target_id
                        && player.alive
                        && player.attackable
                        && (self.zombie.kind != ZombieKind::Drowned
                            || !bright_outside
                            || FluidFrame::sample(world, player.position, 0.6, 1.8).in_water())
                }) {
                    self.target_player_id = None;
                }
            }
            if let Some(target_id) = self.target_villager_id {
                if !villagers
                    .iter()
                    .any(|villager| villager.id == target_id && villager.alive)
                {
                    self.target_villager_id = None;
                }
            }
            // The higher-priority player goal still trials while the villager
            // goal owns TARGET, because it can replace that lower priority.
            if self.target_player_id.is_none() {
                if self.random.next_int(5) == 0 {
                    self.target_player_id = players
                        .iter()
                        .filter(|player| player.alive && player.attackable)
                        .filter(|player| {
                            self.zombie.kind != ZombieKind::Drowned
                                || !bright_outside
                                || FluidFrame::sample(world, player.position, 0.6, 1.8).in_water()
                        })
                        .filter(|player| player.position.distance_squared(position) <= 35.0 * 35.0)
                        .min_by(|a, b| {
                            a.position
                                .distance_squared(position)
                                .total_cmp(&b.position.distance_squared(position))
                        })
                        .map(|player| player.id);
                }
                if self.target_player_id.is_some() {
                    self.target_villager_id = None;
                }
            }
            if self.target_player_id.is_none() && self.target_villager_id.is_none() {
                if self.random.next_int(5) == 0 {
                    self.target_villager_id = villagers
                        .iter()
                        .filter(|villager| villager.alive)
                        .filter(|villager| {
                            villager.position.distance_squared(position) <= 35.0 * 35.0
                        })
                        .min_by(|a, b| {
                            a.position
                                .distance_squared(position)
                                .total_cmp(&b.position.distance_squared(position))
                        })
                        .map(|villager| villager.id);
                }
                if self.target_player_id.is_none() && self.target_villager_id.is_none() {
                    // The remaining iron-golem and baby-turtle target goals
                    // each make their interval trial in this isolated scene.
                    let _ = self.random.next_int(5);
                    let _ = self.random.next_int(5);
                }
            }
        }
        let target = players
            .iter()
            .find(|player| Some(player.id) == self.target_player_id && player.alive)
            .map(|player| PursuitTarget {
                position: player.position,
                eye_height: player.eye_height,
                width: 0.6,
                height: 1.8,
                villager_id: None,
                player_id: Some(player.id),
            })
            .or_else(|| {
                villagers
                    .iter()
                    .find(|villager| Some(villager.id) == self.target_villager_id && villager.alive)
                    .map(|villager| PursuitTarget {
                        position: villager.position,
                        eye_height: villager.eye_height,
                        width: villager.width,
                        height: villager.height,
                        villager_id: Some(villager.id),
                        player_id: None,
                    })
            });
        // Drowned.okTarget is part of its melee goal (and player target goal),
        // not the villager target selector. A dry villager can remain selected
        // in daylight without allowing the attack action to start.
        let target_in_water = target.is_some_and(|target| {
            FluidFrame::sample(world, target.position, target.width, target.height).in_water()
        });
        let can_attack_target = target.is_some()
            && (self.zombie.kind != ZombieKind::Drowned || !bright_outside || target_in_water);
        if full_goal_tick
            && self.attack_goal_running
            && (!can_attack_target || self.navigation.is_done())
        {
            self.attack_goal_running = false;
            self.aggressive = false;
            self.navigation.stop();
        }
        if full_goal_tick && !self.attack_goal_running && game_time - self.last_can_use_check >= 20
        {
            self.last_can_use_check = game_time;
            if let Some(target) = target.filter(|_| can_attack_target) {
                if self.navigate_to_pursuit_target(world, target.position) == Some(true) {
                    self.attack_goal_running = true;
                    self.ticks_until_next_path_recalculation = 0;
                    self.ticks_until_next_attack = 0;
                    self.raise_arm_ticks = 0;
                }
            }
        }
        if self.attack_goal_running {
            if let Some(target) = target {
                self.look_control.set_look_at_with_limits(
                    target.position + DVec3::new(0.0, f64::from(target.eye_height), 0.0),
                    30.0,
                    30.0,
                );
                self.ticks_until_next_path_recalculation =
                    (self.ticks_until_next_path_recalculation - 1).max(0);
                if self.ticks_until_next_path_recalculation <= 0
                    && (self.pathed_target == DVec3::ZERO
                        || target.position.distance_squared(self.pathed_target) >= 1.0
                        || self.random.next_float() < 0.05)
                {
                    self.pathed_target = target.position;
                    self.ticks_until_next_path_recalculation = 4 + self.random.next_int(7) as i32;
                    if self.navigate_to_pursuit_target(world, target.position) != Some(true) {
                        self.ticks_until_next_path_recalculation += 15;
                    }
                }
                self.ticks_until_next_attack = (self.ticks_until_next_attack - 1).max(0);
                if self.ticks_until_next_attack == 0 && self.within_melee_range(target) {
                    self.ticks_until_next_attack = 20;
                    self.pending_attack_villager = target.villager_id.map(|id| (id, position));
                    self.pending_attack_player = target.player_id.map(|id| (id, position));
                }
                self.raise_arm_ticks += 1;
                self.aggressive = self.raise_arm_ticks >= 5 && self.ticks_until_next_attack < 10;
            }
        }
        let in_water = FluidFrame::sample(
            world,
            self.zombie.body.position,
            self.zombie.body.width,
            self.zombie.body.height,
        )
        .in_water();
        let navigation_step = if self.zombie.kind == ZombieKind::Drowned {
            self.navigation.tick_amphibious(
                world,
                position,
                self.zombie.body.width,
                self.zombie.body.height,
                self.speed,
                in_water,
            )
        } else {
            self.navigation.tick(
                world,
                position,
                self.zombie.body.on_ground,
                self.zombie.body.width,
                self.speed,
            )
        };
        if let Some((wanted, speed)) = navigation_step {
            self.move_control.set_wanted_position(wanted, speed);
        }
        let obstacle_top = crate::control::obstacle_top(world, position);
        // DrownedMoveControl.tick applies a small downward impulse before
        // delegating to MoveControl whenever the drowned is airborne on land.
        if self.zombie.kind == ZombieKind::Drowned
            && !self.zombie.body.on_ground
            && !(target_in_water && in_water && self.underwater_last_tick)
        {
            self.zombie.body.velocity.y -= 0.008;
        }
        let swimming_control = self.zombie.kind == ZombieKind::Drowned
            && target_in_water
            && in_water
            && self.underwater_last_tick;
        let control = if swimming_control {
            self.move_control.tick_drowned_swimming(
                position,
                self.yaw,
                self.speed,
                self.sideways,
                self.effects.movement_speed(0.23_f32 as f64),
                self.navigation.is_done(),
                target.is_some_and(|target| target.position.y > position.y),
                &mut self.zombie.body.velocity,
            )
        } else {
            self.move_control.tick(
                position,
                self.zombie.body.on_ground,
                self.yaw,
                self.speed,
                self.forward,
                self.sideways,
                self.effects.movement_speed(0.23_f32 as f64),
                self.zombie.body.width,
                self.zombie.body.step_height,
                obstacle_top,
                |_, _| true,
            )
        };
        if swimming_control {
            self.body_rotation.body_yaw = control.yaw;
        }
        self.yaw = control.yaw;
        self.speed = control.speed;
        self.forward = control.forward;
        self.sideways = control.sideways;
        self.look_control.tick(
            position,
            self.zombie.eye_height(),
            self.body_rotation.body_yaw,
            !self.navigation.is_done(),
        );
        self.travel_controlled(world, target_in_water);
        self.body_rotation.tick(
            self.yaw,
            &mut self.look_control,
            self.previous_position,
            self.zombie.body.position,
        );
    }

    fn within_melee_range(&self, target: PursuitTarget) -> bool {
        // Mob.DEFAULT_ATTACK_REACH = sqrt(2.04F) - 0.6F, then the mob box is
        // expanded horizontally and tested against the target hitbox.
        let expansion = (2.04_f32 as f64).sqrt() - 0.6_f32 as f64;
        let horizontal = expansion + f64::from(self.zombie.body.width + target.width) * 0.5;
        let dx = (self.zombie.body.position.x - target.position.x).abs();
        let dz = (self.zombie.body.position.z - target.position.z).abs();
        dx < horizontal
            && dz < horizontal
            && self.zombie.body.position.y < target.position.y + f64::from(target.height)
            && target.position.y < self.zombie.body.position.y + f64::from(self.zombie.body.height)
    }
}

impl BatEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.bat.body.position
    }

    pub fn set_random_seed(&mut self, seed: u64) {
        self.random = LegacyRandom::new(seed);
    }

    fn tick_active(&mut self, world: &impl World, players: &[PlayerCandidate]) {
        let position = self.bat.body.position;
        let pos = (
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        );
        let above = (pos.0, pos.1 + 1, pos.2);
        let conductor =
            |block: (i32, i32, i32)| world.block(block).is_some_and(|b| b.id != "minecraft:air");
        if self.bat.resting {
            if conductor(above) {
                if self.random.next_int(200) == 0 {
                    self.look_control.head_yaw = self.random.next_int(360) as f32;
                }
                if players.iter().any(|player| {
                    player.alive
                        && !player.spectator
                        && player.position.distance_squared(position) <= 16.0
                }) {
                    self.bat.resting = false;
                }
            } else {
                self.bat.resting = false;
            }
        } else {
            if let Some(target) = self.target_position {
                if world.block(target).is_some_and(|b| b.id != "minecraft:air") || target.1 <= -64 {
                    self.target_position = None;
                }
            }
            let near_target = self.target_position.is_some_and(|target| {
                let center = DVec3::new(
                    target.0 as f64 + 0.5,
                    target.1 as f64 + 0.5,
                    target.2 as f64 + 0.5,
                );
                center.distance_squared(position) < 4.0
            });
            if self.target_position.is_none() || self.random.next_int(30) == 0 || near_target {
                self.target_position = Some((
                    (position.x + self.random.next_int(7) as f64 - self.random.next_int(7) as f64)
                        .floor() as i32,
                    (position.y + self.random.next_int(6) as f64 - 2.0).floor() as i32,
                    (position.z + self.random.next_int(7) as f64 - self.random.next_int(7) as f64)
                        .floor() as i32,
                ));
            }
            let target = self.target_position.expect("selected flying target");
            let dx = target.0 as f64 + 0.5 - position.x;
            let dy = target.1 as f64 + 0.1 - position.y;
            let dz = target.2 as f64 + 0.5 - position.z;
            let movement = self.bat.body.velocity;
            let java_sign = |value: f64| if value == 0.0 { value } else { value.signum() };
            let new_movement = DVec3::new(
                movement.x + (java_sign(dx) * 0.5 - movement.x) * 0.1_f32 as f64,
                movement.y + (java_sign(dy) * 0.7_f32 as f64 - movement.y) * 0.1_f32 as f64,
                movement.z + (java_sign(dz) * 0.5 - movement.z) * 0.1_f32 as f64,
            );
            self.bat.body.velocity = new_movement;
            let desired_yaw = (crate::control::minecraft_atan2(new_movement.z, new_movement.x)
                * 57.2957763671875_f64) as f32
                - 90.0;
            let difference = (desired_yaw - self.yaw + 180.0).rem_euclid(360.0) - 180.0;
            self.yaw += difference;
            self.forward = 0.5;
            if self.random.next_int(100) == 0 && conductor(above) {
                self.bat.resting = true;
            }
        }
        // The stock WAIT MoveControl executes after customServerAiStep and
        // clears Bat's zza=0.5F before LivingEntity.travel reads the input.
        self.forward = 0.0;
        self.look_control
            .tick(position, 0.45, self.body_rotation.body_yaw, false);
        self.bat.body.travel_air(
            world,
            DVec3::new(0.0, 0.0, self.forward as f64),
            0.0,
            self.yaw,
        );
        self.body_rotation.tick(
            self.yaw,
            &mut self.look_control,
            self.previous_position,
            self.bat.body.position,
        );
    }

    pub fn hurt(&mut self, amount: f32) -> DamageResult {
        // Bat.hurtServer wakes a resting bat before applying generic damage.
        self.bat.resting = false;
        let result = self.bat.damage.hurt_generic(&mut self.bat.health, 6.0, amount);
        self.bat.damage.place_death(result, self.bat.body.position, self.bat.body.fire_ticks > 0);
        result
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum EntityKey {
    Bat(u64),
    Zombie(u64),
    Skeleton(u64),
    Creeper(u64),
    Spider(u64),
    Slime(u64),
    Enderman(u64),
    Witch(u64),
    IronGolem(u64),
    Wolf(u64),
    Arrow(u64),
    /// A thrown splash potion.
    Potion(u64),
    Villager(u64),
    Cow(u64),
    Sheep(u64),
    Pig(u64),
    Chicken(u64),
}

impl EntityKey {
    fn id(self) -> u64 {
        match self {
            Self::Bat(id)
            | Self::Zombie(id)
            | Self::Skeleton(id)
            | Self::Creeper(id)
            | Self::Spider(id)
            | Self::Slime(id)
            | Self::Enderman(id)
            | Self::Witch(id)
            | Self::IronGolem(id)
            | Self::Wolf(id)
            | Self::Arrow(id)
            | Self::Potion(id)
            | Self::Villager(id)
            | Self::Cow(id)
            | Self::Sheep(id)
            | Self::Pig(id)
            | Self::Chicken(id) => id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MobHit {
    Bat(u64),
    Zombie(u64),
    Skeleton(u64),
    Creeper(u64),
    Spider(u64),
    Slime(u64),
    Enderman(u64),
    Witch(u64),
    IronGolem(u64),
    Wolf(u64),
    Villager(u64),
    Cow(u64),
    Mooshroom(u64),
    Sheep(u64),
    Pig(u64),
    Chicken(u64),
}

impl MobHit {
    /// The hit mob's entity ID.
    pub fn id(self) -> u64 {
        match self {
            Self::Bat(id) | Self::Zombie(id) | Self::Skeleton(id) | Self::Creeper(id) | Self::Spider(id) | Self::Slime(id) | Self::Enderman(id) | Self::Witch(id) | Self::IronGolem(id) | Self::Wolf(id) | Self::Villager(id) | Self::Cow(id) | Self::Mooshroom(id) | Self::Sheep(id) | Self::Pig(id) | Self::Chicken(id) => id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageSourceKind {
    Generic,
    Magic,
    Projectile,
    /// `explosion` and `player_explosion`, which make animals panic.
    Explosion,
    /// `player_attack`, which makes animals panic.
    PlayerAttack,
    /// `mob_attack`, which makes animals panic.
    MobAttack,
    /// Fire, lava, magma and cactus (`#panic_environmental_causes`), which
    /// make animals panic.
    Hazard,
}

impl DamageSourceKind {
    /// Whether `PanicGoal` runs from it (`#panic_causes`, as the fixtures
    /// measured it: magic and explosions).
    fn panics(source: Option<Self>) -> bool {
        matches!(source, Some(Self::Magic | Self::Explosion | Self::PlayerAttack | Self::MobAttack | Self::Hazard))
    }
}

impl ChickenEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.chicken.body.position
    }

    /// `Entity.baseTick`'s fluid interaction.
    fn update_fluid(&mut self, world: &impl World) {
        let sounds = self.movement_sounds();
        self.fluid = base_tick_fluid(&mut self.chicken.body, world, self.tick_count == 1, &mut self.random, &mut self.voices, sounds);
        self.was_touching_water = self.chicken.body.touching_water;
    }

    pub fn running_goals(&self) -> Vec<&'static str> {
        self.goals
            .running_ids()
            .map(|id| CHICKEN_GOAL_NAMES[id])
            .collect()
    }

    pub fn retain_goals(&mut self, names: &[&str]) {
        let ids: Vec<_> = CHICKEN_GOAL_NAMES
            .iter()
            .enumerate()
            .filter_map(|(id, name)| names.contains(name).then_some(id))
            .collect();
        assert_eq!(ids.len(), names.len());
        self.goals.retain_ids(&ids);
    }

    pub fn set_random_seed(&mut self, seed: i64) {
        self.random = LegacyRandom::new(seed as u64);
    }

    pub fn hurt(&mut self, amount: f32) -> DamageResult {
        self.hurt_with_source(amount, DamageSourceKind::Generic)
    }

    pub fn hurt_with_source(&mut self, amount: f32, source: DamageSourceKind) -> DamageResult {
        let full_hit = self.chicken.damage.cooldown_ticks <= 10;
        if self.chicken.health > 0.0 {
            self.no_action_time = 0;
        }
        let result = self.chicken.hurt_generic(amount);
        let position = self.position();
        self.chicken.damage.place_death(result, position, self.chicken.body.fire_ticks > 0);
        if result.applied {
            self.last_damage_source = Some(source);
            self.last_damage_tick = self.tick_count + 1;
            if full_hit {
                if !result.died {
                    self.ambient_sound_time = -120;
                }
                let voice = hurt_voice(&mut self.random, result.died, self.chicken.age.baby());
                self.voices.push((voice, position));
            }
        }
        result
    }
}

impl CowEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.cow.body.position
    }

    /// `Entity.baseTick`'s fluid interaction.
    fn update_fluid(&mut self, world: &impl World) {
        let sounds = self.movement_sounds();
        self.fluid = base_tick_fluid(&mut self.cow.body, world, self.tick_count == 1, &mut self.random, &mut self.voices, sounds);
        self.was_touching_water = self.cow.body.touching_water;
    }

    pub fn hurt(&mut self, amount: f32, source: DamageSourceKind) -> DamageResult {
        let full_hit = self.cow.damage.cooldown_ticks <= 10;
        if self.cow.health > 0.0 {
            self.no_action_time = 0;
        }
        let result = self.cow.hurt_generic(amount);
        let position = self.position();
        self.cow.damage.place_death(result, position, self.cow.body.fire_ticks > 0);
        if result.applied {
            self.last_damage_source = Some(source);
            self.last_damage_tick = self.tick_count + 1;
            if full_hit {
                if !result.died {
                    self.ambient_sound_time = -self.ambient_interval();
                }
                let voice = hurt_voice(&mut self.random, result.died, self.cow.age.baby());
                self.voices.push((voice, position));
            }
            // `AbstractHorse.hurtServer`: one hurt in three rears it.
            if let Some(horse) = &mut self.horse {
                if self.random.next_int(3) == 0 {
                    horse.stand_if_possible();
                }
            }
        }
        result
    }

    pub fn running_goals(&self) -> Vec<&'static str> {
        let names: &[&str] = if self.horse.is_some() { &HORSE_GOAL_NAMES } else { &GOAL_NAMES };
        self.goals.running_ids().map(|id| names[id]).collect()
    }

    pub fn retain_goals(&mut self, names: &[&str]) {
        let all: &[&str] = if self.horse.is_some() { &HORSE_GOAL_NAMES } else { &GOAL_NAMES };
        let ids: Vec<_> = all
            .iter()
            .enumerate()
            .filter_map(|(id, name)| names.contains(name).then_some(id))
            .collect();
        assert_eq!(ids.len(), names.len());
        self.goals.retain_ids(&ids);
    }

    pub fn set_random_seed(&mut self, seed: i64) {
        self.random = LegacyRandom::new(seed as u64);
    }
}

impl SheepEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.body.position
    }

    /// `Entity.baseTick`'s fluid interaction.
    fn update_fluid(&mut self, world: &impl World) {
        self.fluid = base_tick_fluid(&mut self.body, world, self.tick_count == 1, &mut self.random, &mut self.voices, SHEEP_SOUNDS);
        self.was_touching_water = self.body.touching_water;
    }

    pub fn hurt(&mut self, amount: f32, source: DamageSourceKind) -> DamageResult {
        let full_hit = self.damage.cooldown_ticks <= 10;
        if self.health > 0.0 {
            self.no_action_time = 0;
        }
        let result = self.damage.hurt_generic(&mut self.health, 8.0, amount);
        let position = self.position();
        self.damage.place_death(result, position, self.body.fire_ticks > 0);
        if result.applied {
            self.last_damage_source = Some(source);
            self.last_damage_tick = self.tick_count + 1;
            if full_hit {
                if !result.died {
                    self.ambient_sound_time = -120;
                }
                let voice = hurt_voice(&mut self.random, result.died, self.sheep.age.baby());
                self.voices.push((voice, position));
            }
        }
        result
    }

    pub fn sync_dimensions(&mut self) {
        let baby = self.sheep.age.baby();
        self.body.width = if baby { 0.45 } else { 0.9 };
        self.body.height = if baby { 0.65 } else { 1.3 };
    }

    pub fn set_random_seed(&mut self, seed: i64) {
        self.random = LegacyRandom::new(seed as u64);
    }

    pub fn retain_goals(&mut self, names: &[&str]) {
        let ids: Vec<_> = SHEEP_GOAL_NAMES
            .iter()
            .enumerate()
            .filter_map(|(id, name)| names.contains(name).then_some(id))
            .collect();
        assert_eq!(ids.len(), names.len());
        self.goals.retain_ids(&ids);
    }

    pub fn running_goals(&self) -> Vec<&'static str> {
        self.goals
            .running_ids()
            .map(|id| SHEEP_GOAL_NAMES[id])
            .collect()
    }

    pub fn eat_head_position_scale(&self, partial_tick: f32) -> f32 {
        let ticks = self.eat_animation_ticks;
        if ticks <= 0 {
            0.0
        } else if (4..=36).contains(&ticks) {
            1.0
        } else if ticks < 4 {
            (ticks as f32 - partial_tick) / 4.0
        } else {
            -(ticks as f32 - 40.0 - partial_tick) / 4.0
        }
    }

    pub fn eat_head_angle_scale(&self, partial_tick: f32) -> f32 {
        let ticks = self.eat_animation_ticks;
        let rest = (std::f64::consts::PI / 5.0) as f32;
        if ticks > 4 && ticks <= 36 {
            let scale = (ticks as f32 - 4.0 - partial_tick) / 32.0;
            rest + 0.21991149_f32 * mth_sin(scale * 28.7_f32)
        } else if ticks > 0 {
            rest
        } else {
            0.0
        }
    }
}

impl PigEntity {
    /// Where it stands (its feet).
    pub fn position(&self) -> DVec3 {
        self.pig.body.position
    }

    /// `Entity.baseTick`'s fluid interaction.
    fn update_fluid(&mut self, world: &impl World) {
        let sounds = self.movement_sounds();
        self.fluid = base_tick_fluid(&mut self.pig.body, world, self.tick_count == 1, &mut self.random, &mut self.voices, sounds);
        self.was_touching_water = self.pig.body.touching_water;
    }

    pub fn running_goals(&self) -> Vec<&'static str> {
        self.goals
            .running_ids()
            .map(|id| PIG_GOAL_NAMES[id])
            .collect()
    }

    pub fn retain_goals(&mut self, names: &[&str]) {
        let ids: Vec<_> = PIG_GOAL_NAMES
            .iter()
            .enumerate()
            .filter_map(|(id, name)| names.contains(name).then_some(id))
            .collect();
        assert!(names.iter().all(|name| PIG_GOAL_NAMES.contains(name)));
        self.goals.retain_ids(&ids);
    }

    pub fn set_random_seed(&mut self, seed: i64) {
        self.random = LegacyRandom::new(seed as u64);
    }

    pub fn hurt(&mut self, amount: f32, source: DamageSourceKind) -> DamageResult {
        let full_hit = self.pig.damage.cooldown_ticks <= 10;
        if self.pig.health > 0.0 {
            self.no_action_time = 0;
        }
        let result = self.pig.hurt_generic(amount);
        let position = self.position();
        self.pig.damage.place_death(result, position, self.pig.body.fire_ticks > 0);
        if result.applied {
            self.last_damage_source = Some(source);
            self.last_damage_tick = self.tick_count + 1;
            if full_hit {
                if !result.died {
                    self.ambient_sound_time = -120;
                }
                let voice = hurt_voice(&mut self.random, result.died, self.pig.age.baby());
                self.voices.push((voice, position));
            }
        }
        result
    }
}

/// `Mth.sin` of a float angle.
fn mth_sin(value: f32) -> f32 {
    minecraftoss_player::mth::sin(f64::from(value))
}

/// A mob as natural spawning's census sees it ([`EntityWorld::census`]).
#[derive(Clone, Debug)]
pub struct CensusEntry {
    pub kind: &'static str,
    pub position: DVec3,
    pub width: f32,
    pub height: f32,
    pub persistent: bool,
}

#[derive(Clone)]
pub struct EntityWorld {
    bats: Vec<BatEntity>,
    zombies: Vec<ZombieEntity>,
    skeletons: Vec<SkeletonEntity>,
    creepers: Vec<CreeperEntity>,
    spiders: Vec<SpiderEntity>,
    slimes: Vec<SlimeEntity>,
    endermen: Vec<EndermanEntity>,
    witches: Vec<WitchEntity>,
    iron_golems: Vec<IronGolemEntity>,
    wolves: Vec<WolfEntity>,
    /// Where the players look, for endermen.
    player_views: Vec<(u64, crate::enderman::PlayerView)>,
    /// The players' health, effects and motion, for witches.
    player_vitals: Vec<(u64, crate::monster_ai::PlayerVitals)>,
    arrows: Vec<ArrowEntity>,
    potions: Vec<PotionEntity>,
    /// Potions that broke near players since the last
    /// `take_player_splashes`.
    player_splashes: Vec<(u64, PlayerSplash)>,
    /// Where potions broke since the last `take_potion_breaks`.
    potion_breaks: Vec<PotionBreak>,
    villagers: Vec<VillagerEntity>,
    cows: Vec<CowEntity>,
    sheep: Vec<SheepEntity>,
    pigs: Vec<PigEntity>,
    chickens: Vec<ChickenEntity>,
    order: Vec<EntityKey>,
    next_id: u64,
    game_time: i64,
    bright_outside: bool,
    mob_griefing: bool,
    recipes: Option<Arc<RecipeBook>>,
    mix_random: LegacyRandom,
    projectile_seed_random: LegacyRandom,
    arrow_shoot_seed: Option<u64>,
    arrow_damage_seed: Option<u64>,
    explosions: Vec<CreeperExplosion>,
    /// Hits on players since the last `take_player_hits`.
    player_hits: Vec<PlayerHit>,
    /// Each player's fight memory.
    player_fights: HashMap<u64, PlayerFights>,
    /// Whether arrows hit players: real players are in the entity lookup;
    /// the harness's probe players are not, so arrows pass through them.
    players_pickable: bool,
    /// The `gameplay/monsters_burn` attribute (daytime in the Overworld).
    monsters_burn: bool,
    /// `Difficulty.getId`: 0 peaceful to 3 hard.
    difficulty: i32,
    /// Sounds not tied to a living mob (a creeper's blast, gone with it).
    sounds: Vec<MobSound>,
    /// The `mob_drops` game rule.
    mob_drops: bool,
    /// The living mobs and players by entity section, for pushing.
    sections: pushing::Sections,
    /// The real players' boxes this tick, for pushing (`players_pickable`).
    pushing_players: Vec<pushing::PlayerBox>,
    /// The level's random, which brains draw from.
    level_random: LegacyRandom,
    /// `Level.getDayTime`, for villager schedules.
    day_time: i64,
    /// `RandomSupport`'s seed uniquifier (without the clock), for what
    /// vanilla seeds from `RandomSource.create()`.
    seed_uniquifier: u64,
    /// The level's points of interest (`PoiManager`).
    pub pois: crate::poi::PoiManager,
    /// A pinned random for the next golem villagers summon.
    summoned_golem_seed: Option<u64>,
    /// Pinned randoms for the next babies villagers make, in turn, and the
    /// babies made since the last `take_born_villagers`.
    born_seeds: std::collections::VecDeque<u64>,
    born_villagers: Vec<u64>,
    /// Pups born since the last `take_born_wolves`.
    born_wolves: Vec<u64>,
    /// The trade data villagers make offers from, and the level's trade
    /// sequences.
    trades: Option<Arc<crate::trading::TradeBook>>,
    pub trade_sequences: crate::trading::TradeSequences,
    /// Golems villagers summoned since the last `take_summoned_golems`.
    summoned_golems: Vec<u64>,
    /// The `max_entity_cramming` game rule.
    max_entity_cramming: i32,
    /// Entities' UUIDs where known (players under `PLAYER_TARGET` plus
    /// their ID), which gossip is kept by, and what makes the others'
    /// differ from session to session.
    uuids: HashMap<u64, u128>,
    uuid_salt: u64,
    /// Players' Hero of the Village levels, their trading screens, and the
    /// experience trades dropped.
    player_heroes: HashMap<u64, i32>,
    merchant_menus: HashMap<u64, crate::merchant::MerchantMenu>,
    trade_experience: Vec<(DVec3, i32)>,
    /// Players' main-hand items, as villagers see them.
    player_main_hands: HashMap<u64, String>,
    /// `broadcastEntityEvent` since last taken: entity, event (12 hearts,
    /// 13 anger, 14 happiness).
    entity_events: Vec<(u64, u8)>,
}

impl Default for EntityWorld {
    fn default() -> Self {
        Self {
            bats: Vec::new(),
            zombies: Vec::new(),
            skeletons: Vec::new(),
            creepers: Vec::new(),
            spiders: Vec::new(),
            slimes: Vec::new(),
            endermen: Vec::new(),
            witches: Vec::new(),
            iron_golems: Vec::new(),
            wolves: Vec::new(),
            player_views: Vec::new(),
            player_vitals: Vec::new(),
            arrows: Vec::new(),
            potions: Vec::new(),
            player_splashes: Vec::new(),
            potion_breaks: Vec::new(),
            villagers: Vec::new(),
            cows: Vec::new(),
            sheep: Vec::new(),
            pigs: Vec::new(),
            chickens: Vec::new(),
            order: Vec::new(),
            next_id: 0,
            game_time: 0,
            bright_outside: false,
            mob_griefing: true,
            recipes: None,
            mix_random: LegacyRandom::new(0),
            projectile_seed_random: LegacyRandom::new(0),
            arrow_shoot_seed: None,
            arrow_damage_seed: None,
            explosions: Vec::new(),
            player_hits: Vec::new(),
            player_fights: HashMap::new(),
            players_pickable: false,
            monsters_burn: false,
            difficulty: 2,
            sounds: Vec::new(),
            mob_drops: true,
            sections: pushing::Sections::default(),
            pushing_players: Vec::new(),
            level_random: LegacyRandom::new(0),
            day_time: 0,
            seed_uniquifier: 8_682_522_807_148_012,
            pois: crate::poi::PoiManager::new(-4, 19),
            summoned_golem_seed: None,
            summoned_golems: Vec::new(),
            born_seeds: Default::default(),
            born_villagers: Vec::new(),
            born_wolves: Vec::new(),
            trades: None,
            trade_sequences: Default::default(),
            max_entity_cramming: 24,
            uuids: HashMap::new(),
            uuid_salt: 0,
            player_heroes: HashMap::new(),
            merchant_menus: HashMap::new(),
            trade_experience: Vec::new(),
            player_main_hands: HashMap::new(),
            entity_events: Vec::new(),
        }
    }
}

impl EntityWorld {
    /// Isolated bow-equipped skeleton profile. Other registered action goals
    /// and non-player targets are not yet represented in this profile.
    /// A bow skeleton keeping only `RangedBowAttackGoal` of its goals (its
    /// target selector whole), as the goal-filtered fixtures measure it.
    pub fn spawn_skeleton_bow(&mut self, skeleton: Skeleton) -> u64 {
        self.spawn_skeleton_keeping(skeleton, &["RangedBowAttackGoal"])
    }

    pub fn spawn_skeleton(&mut self, skeleton: Skeleton, no_ai: bool) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let previous_position = skeleton.body.position;
        self.skeletons.push(SkeletonEntity {
            effects: Default::default(),
            id,
            skeleton,
            no_ai,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            previous_position,
            random: LegacyRandom::new(0),
            target_player_id: None,
            bow: SkeletonBowGoal::normal(),
            navigation: GroundNavigation::default(),
            move_control: MoveControl::default(),
            look_control: LookControl::default(),
            body_rotation: BodyRotation::default(),
            yaw: 0.0,
            speed: 0.0,
            forward: 0.0,
            sideways: 0.0,
            ai: None,
        });
        self.order.push(EntityKey::Skeleton(id));
        self.file_in_section(EntityKey::Skeleton(id));
        id
    }

    /// A bow skeleton with its whole goal and target selectors
    /// (`AbstractSkeleton.registerGoals`, the bow goal from
    /// `reassessWeaponGoal`), facing `yaw`.
    pub fn spawn_skeleton_active(&mut self, skeleton: Skeleton, yaw: f32) -> u64 {
        let mut ai = MonsterAi::of_kind(crate::monster_ai::MonsterKind::Skeleton, &skeleton.body, yaw);
        ai.state.eye_height = skeleton.eye_height();
        // `AbstractSkeleton.reassessWeaponGoal`: the kind's interval, its
        // hard one on hard.
        ai.state.bow.attack_interval = skeleton.kind.attack_interval(self.difficulty == 3);
        let id = self.spawn_skeleton(skeleton, false);
        let entity = self.skeleton_mut(id).unwrap();
        entity.yaw = yaw;
        entity.body_rotation = ai.body_rotation.clone();
        entity.look_control = ai.state.look_control.clone();
        entity.ai = Some(Box::new(ai));
        id
    }

    /// A bow skeleton on the goal framework keeping only the named goals
    /// (the harness's `entity_keep_goal`).
    pub fn spawn_skeleton_keeping(&mut self, skeleton: Skeleton, goals: &[&str]) -> u64 {
        let id = self.spawn_skeleton_active(skeleton, 0.0);
        self.skeleton_mut(id).unwrap().ai.as_deref_mut().unwrap().retain_goals(goals);
        id
    }

    pub fn skeletons(&self) -> &[SkeletonEntity] {
        &self.skeletons
    }

    pub fn skeleton_mut(&mut self, id: u64) -> Option<&mut SkeletonEntity> {
        self.skeletons.iter_mut().find(|entity| entity.id == id)
    }

    pub fn spawn_creeper(&mut self, creeper: Creeper, no_ai: bool) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let ai = CreeperAi::new(&creeper.body, creeper.yaw);
        self.creepers.push(CreeperEntity {
            effects: Default::default(),
            id,
            previous_position: creeper.body.position,
            creeper,
            no_ai,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            random: LegacyRandom::new(0),
            ai,
        });
        self.order.push(EntityKey::Creeper(id));
        self.file_in_section(EntityKey::Creeper(id));
        id
    }

    pub fn creepers(&self) -> &[CreeperEntity] {
        &self.creepers
    }

    /// A spider with its whole goal and target selectors, facing `yaw`
    /// (NoAI spiders keep still but still climb, sound and take hits).
    pub fn spawn_spider(&mut self, spider: Spider, no_ai: bool) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let mut ai = MonsterAi::of_kind(crate::monster_ai::MonsterKind::Spider, &spider.body, spider.yaw);
        ai.state.eye_height = spider.eye_height();
        ai.state.max_health = crate::spider::MAX_HEALTH;
        self.spiders.push(SpiderEntity {
            effects: Default::default(),
            id,
            previous_position: spider.body.position,
            spider,
            no_ai,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            random: LegacyRandom::new(0),
            ai: Box::new(ai),
        });
        self.order.push(EntityKey::Spider(id));
        self.file_in_section(EntityKey::Spider(id));
        id
    }

    pub fn spiders(&self) -> &[SpiderEntity] {
        &self.spiders
    }

    pub fn spider_mut(&mut self, id: u64) -> Option<&mut SpiderEntity> {
        self.spiders.iter_mut().find(|entity| entity.id == id)
    }

    pub fn creeper_mut(&mut self, id: u64) -> Option<&mut CreeperEntity> {
        self.creepers.iter_mut().find(|entity| entity.id == id)
    }

    pub fn take_creeper_explosions(&mut self) -> Vec<CreeperExplosion> {
        std::mem::take(&mut self.explosions)
    }

    /// A mob's body and eye height for a blast, in world order.
    fn blast_target(&self, key: EntityKey) -> Option<(Body, f32)> {
        Some(match key {
            EntityKey::Bat(id) => (self.bats.iter().find(|e| e.id == id)?.bat.body.clone(), 0.45),
            EntityKey::Zombie(id) => {
                let e = self.zombies.iter().find(|e| e.id == id)?;
                (e.zombie.body.clone(), e.zombie.eye_height())
            }
            EntityKey::Skeleton(id) => {
                let e = self.skeletons.iter().find(|e| e.id == id)?;
                (e.skeleton.body.clone(), e.skeleton.eye_height())
            }
            EntityKey::Creeper(id) => {
                let e = self.creepers.iter().find(|e| e.id == id)?;
                if e.creeper.exploded {
                    return None;
                }
                (e.creeper.body.clone(), e.creeper.body.height * 0.85)
            }
            EntityKey::Spider(id) => {
                let e = self.spiders.iter().find(|e| e.id == id)?;
                (e.spider.body.clone(), e.spider.eye_height())
            }
            EntityKey::Slime(id) => {
                let e = self.slimes.iter().find(|e| e.id == id)?;
                (e.slime.body.clone(), e.slime.eye_height())
            }
            EntityKey::Enderman(id) => {
                let e = self.endermen.iter().find(|e| e.id == id)?;
                (e.enderman.body.clone(), e.enderman.eye_height())
            }
            EntityKey::Witch(id) => (self.witches.iter().find(|e| e.id == id)?.witch.body.clone(), crate::witch::EYE_HEIGHT),
            EntityKey::IronGolem(id) => (self.iron_golems.iter().find(|e| e.id == id)?.golem.body.clone(), crate::iron_golem::EYE_HEIGHT),
            EntityKey::Wolf(id) => {
                let e = self.wolves.iter().find(|e| e.id == id)?;
                (e.wolf.body.clone(), e.wolf.eye_height())
            }
            EntityKey::Villager(id) => {
                let e = self.villagers.iter().find(|e| e.id == id)?;
                (e.villager.body.clone(), e.villager.eye_height())
            }
            EntityKey::Cow(id) => {
                let e = self.cows.iter().find(|e| e.id == id)?;
                let eye = match (e.cow.age.baby(), e.mooshroom.is_some()) {
                    (false, _) => 1.3,
                    (true, true) => 0.69,
                    (true, false) => 0.665,
                };
                (e.cow.body.clone(), eye)
            }
            EntityKey::Sheep(id) => {
                let e = self.sheep.iter().find(|e| e.id == id)?;
                (e.body.clone(), if e.sheep.age.baby() { 0.6175 } else { 1.235 })
            }
            EntityKey::Pig(id) => {
                let e = self.pigs.iter().find(|e| e.id == id)?;
                (e.pig.body.clone(), if e.pig.age.baby() { 0.3825 } else { 0.765 })
            }
            EntityKey::Chicken(id) => {
                let e = self.chickens.iter().find(|e| e.id == id)?;
                (e.chicken.body.clone(), if e.chicken.age.baby() { 0.28125 } else { 0.644 })
            }
            EntityKey::Arrow(_) | EntityKey::Potion(_) => return None,
        })
    }

    /// A blast's damage on a mob (`hurtServer` with `player_explosion`:
    /// armor counts, no default knockback), then its push
    /// (`pushFromExplosion`), which lands even when the hurt does not.
    fn hurt_by_blast(&mut self, key: EntityKey, damage: f32, push: DVec3, source_id: u64) -> Option<DamageResult> {
        let time = self.game_time;
        let source = DamageSourceKind::Explosion;
        let (result, body) = match key {
            EntityKey::Bat(id) => self.bats.iter_mut().find(|e| e.id == id).map(|e| (e.hurt(damage), &mut e.bat.body)),
            EntityKey::Zombie(id) => self.zombies.iter_mut().find(|e| e.id == id).map(|e| (e.hurt(damage_after_armor(damage, 2.0, 0.0)), &mut e.zombie.body)),
            EntityKey::Skeleton(id) => self.skeletons.iter_mut().find(|e| e.id == id).map(|e| (e.hurt(damage), &mut e.skeleton.body)),
            EntityKey::Creeper(id) => self.creepers.iter_mut().find(|e| e.id == id).map(|e| (e.hurt(damage), &mut e.creeper.body)),
            EntityKey::Spider(id) => self.spiders.iter_mut().find(|e| e.id == id).map(|e| (e.hurt(damage), &mut e.spider.body)),
            EntityKey::Slime(id) => self.slimes.iter_mut().find(|e| e.id == id).map(|e| (e.hurt(damage), &mut e.slime.body)),
            EntityKey::Enderman(id) => self.endermen.iter_mut().find(|e| e.id == id).map(|e| (e.hurt(damage), &mut e.enderman.body)),
            EntityKey::Witch(id) => self.witches.iter_mut().find(|e| e.id == id).map(|e| (e.hurt(damage, false, false), &mut e.witch.body)),
            EntityKey::IronGolem(id) => self.iron_golems.iter_mut().find(|e| e.id == id).map(|e| (e.hurt(damage), &mut e.golem.body)),
            EntityKey::Wolf(id) => self.wolves.iter_mut().find(|e| e.id == id).map(|e| (e.hurt_from(damage, "minecraft:player_explosion", Some(crate::monster_ai::Target::Mob(source_id)), time), &mut e.wolf.body)),
            EntityKey::Villager(id) => self.villagers.iter_mut().find(|e| e.id == id).map(|e| (e.hurt_from(damage, "minecraft:player_explosion", Some(source_id), time), &mut e.villager.body)),
            EntityKey::Cow(id) => self.cows.iter_mut().find(|e| e.id == id).map(|e| (e.hurt(damage, source), &mut e.cow.body)),
            EntityKey::Sheep(id) => self.sheep.iter_mut().find(|e| e.id == id).map(|e| (e.hurt(damage, source), &mut e.body)),
            EntityKey::Pig(id) => self.pigs.iter_mut().find(|e| e.id == id).map(|e| (e.hurt(damage, source), &mut e.pig.body)),
            EntityKey::Chicken(id) => self.chickens.iter_mut().find(|e| e.id == id).map(|e| (e.hurt_with_source(damage, source), &mut e.chicken.body)),
            EntityKey::Arrow(_) | EntityKey::Potion(_) => None,
        }?;
        body.velocity += push;
        if let (true, EntityKey::Villager(id)) = (result.applied, key) {
            self.villager_hurt_by(id, source_id);
        }
        Some(result)
    }

    /// `ServerExplosion.hurtEntities` for a creeper's blast: every mob and
    /// player in reach takes damage and a push away from the centre, both
    /// scaled by how much of it the blast sees through the terrain
    /// (`getSeenPercent`). Players get theirs as hits to apply (the
    /// harness's probe players are not in the entity lookup, so only when
    /// players are pickable).
    fn apply_explosion(&mut self, world: &impl World, source_id: u64, blast: CreeperExplosion, players: &[PlayerCandidate]) {
        let reach = blast.radius * 2.0_f32;
        if reach < 1.0e-5_f32 {
            return;
        }
        let reach = f64::from(reach);
        let impact = |body: &Body, eye_height: f32| {
            let distance = body.position.distance(blast.position) / reach;
            if distance > 1.0 {
                return None;
            }
            let (direction, exposure) = explosion_exposure(world, body, eye_height, blast.position);
            let impact = (1.0 - distance) * f64::from(exposure);
            let damage = ((impact * impact + impact) / 2.0 * 7.0 * reach + 1.0) as f32;
            Some((damage, direction * impact))
        };
        for key in self.order.clone() {
            if key.id() == source_id {
                continue;
            }
            let Some((body, eye_height)) = self.blast_target(key) else { continue };
            if let Some((damage, push)) = impact(&body, eye_height) {
                if let Some(result) = self.hurt_by_blast(key, damage, push, source_id) {
                    // `explosion(creeper, creeper)`: the creeper is the
                    // source's entity and its direct one.
                    self.credit(key.id(), result, "minecraft:creeper", true);
                    if blast.powered && result.died {
                        if let Some(death) = self.damage_state_mut(key.id()).and_then(|d| d.death.as_mut()) {
                            death.charged_creeper = Some(source_id);
                        }
                    }
                }
            }
        }
        if !self.players_pickable {
            return;
        }
        for player in players.iter().filter(|p| p.alive && !p.spectator) {
            // A standing, crouching or swimming player's box.
            let height = match player.eye_height {
                e if e >= 1.5 => 1.8,
                e if e >= 1.0 => 1.5,
                _ => 0.6,
            };
            let mut body = Body::new(player.position, 0.6, height);
            body.on_ground = true;
            if let Some((damage, knockback)) = impact(&body, player.eye_height) {
                self.player_hits.push(PlayerHit { player_id: player.id, damage, kind: PlayerHitKind::Explosion { knockback }, source: Some(source_id) });
            }
        }
    }

    pub fn arrows(&self) -> &[ArrowEntity] {
        &self.arrows
    }

    /// Controls the projectile's independent shoot random source for a
    /// deterministic comparison fixture. Normal play leaves this unset.
    pub fn set_arrow_shoot_seed(&mut self, seed: Option<u64>) {
        self.arrow_shoot_seed = seed;
    }

    pub fn set_arrow_damage_seed(&mut self, seed: Option<u64>) {
        self.arrow_damage_seed = seed;
    }

    fn spawn_arrow(&mut self, owner_id: u64, arrow: Arrow) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.arrows.push(ArrowEntity {
            id,
            owner_id,
            arrow,
            effect: None,
        });
        self.order.push(EntityKey::Arrow(id));
        id
    }

    /// An arrow already in flight (a summoned one) belonging to `owner_id`.
    pub fn spawn_owned_arrow(&mut self, owner_id: u64, arrow: Arrow) -> u64 {
        self.spawn_arrow(owner_id, arrow)
    }

    fn arrow_targets(&self, owner_id: u64, players: &[PlayerCandidate]) -> Vec<ArrowTarget> {
        // Players are pickable unless spectating (`canHitEntity`).
        let players = players.iter().filter(|p| self.players_pickable && p.alive && !p.spectator).map(|p| ArrowTarget {
            id: PLAYER_TARGET + p.id,
            min: p.position - DVec3::new(0.3, 0.0, 0.3),
            max: p.position + DVec3::new(0.3, 1.8, 0.3),
        });
        self.order
            .iter()
            .filter_map(|key| {
                let (id, body, health) = match *key {
                    EntityKey::Bat(id) => {
                        let entity = self.bats.iter().find(|entity| entity.id == id)?;
                        (id, &entity.bat.body, entity.bat.health)
                    }
                    EntityKey::Zombie(id) => {
                        let entity = self.zombies.iter().find(|entity| entity.id == id)?;
                        (id, &entity.zombie.body, entity.zombie.health)
                    }
                    EntityKey::Skeleton(id) => {
                        let entity = self.skeletons.iter().find(|entity| entity.id == id)?;
                        (id, &entity.skeleton.body, entity.skeleton.health)
                    }
                    EntityKey::Creeper(id) => {
                        let entity = self.creepers.iter().find(|entity| entity.id == id)?;
                        (id, &entity.creeper.body, entity.creeper.health)
                    }
                    EntityKey::Spider(id) => {
                        let entity = self.spiders.iter().find(|entity| entity.id == id)?;
                        (id, &entity.spider.body, entity.spider.health)
                    }
                    EntityKey::Slime(id) => {
                        let entity = self.slimes.iter().find(|entity| entity.id == id)?;
                        (id, &entity.slime.body, entity.slime.health)
                    }
                    EntityKey::Enderman(id) => {
                        let entity = self.endermen.iter().find(|entity| entity.id == id)?;
                        (id, &entity.enderman.body, entity.enderman.health)
                    }
                    EntityKey::Witch(id) => {
                        let entity = self.witches.iter().find(|entity| entity.id == id)?;
                        (id, &entity.witch.body, entity.witch.health)
                    }
                    EntityKey::IronGolem(id) => {
                        let entity = self.iron_golems.iter().find(|entity| entity.id == id)?;
                        (id, &entity.golem.body, entity.golem.health)
                    }
                    EntityKey::Wolf(id) => {
                        let entity = self.wolves.iter().find(|entity| entity.id == id)?;
                        (id, &entity.wolf.body, entity.wolf.health)
                    }
                    EntityKey::Villager(id) => {
                        let entity = self.villagers.iter().find(|entity| entity.id == id)?;
                        (id, &entity.villager.body, entity.villager.health)
                    }
                    EntityKey::Cow(id) => {
                        let entity = self.cows.iter().find(|entity| entity.id == id)?;
                        (id, &entity.cow.body, entity.cow.health)
                    }
                    EntityKey::Sheep(id) => {
                        let entity = self.sheep.iter().find(|entity| entity.id == id)?;
                        (id, &entity.body, entity.health)
                    }
                    EntityKey::Pig(id) => {
                        let entity = self.pigs.iter().find(|entity| entity.id == id)?;
                        (id, &entity.pig.body, entity.pig.health)
                    }
                    EntityKey::Chicken(id) => {
                        let entity = self.chickens.iter().find(|entity| entity.id == id)?;
                        (id, &entity.chicken.body, entity.chicken.health)
                    }
                    EntityKey::Arrow(_) | EntityKey::Potion(_) => return None,
                };
                if id == owner_id || health <= 0.0 {
                    return None;
                }
                let half_width = f64::from(body.width / 2.0);
                Some(ArrowTarget {
                    id,
                    min: body.position - DVec3::new(half_width, 0.0, half_width),
                    max: body.position + DVec3::new(half_width, f64::from(body.height), half_width),
                })
            })
            .chain(players)
            .collect()
    }

    /// The world's difficulty (`Difficulty.getId`, 0 peaceful to 3 hard).
    pub fn set_difficulty(&mut self, difficulty: i32) {
        self.difficulty = difficulty;
    }

    /// Whether monsters that burn in daylight burn now.
    pub fn set_monsters_burn(&mut self, burn: bool) {
        self.monsters_burn = burn;
    }

    /// Lets arrows hit the players (the players are real entities).
    pub fn set_players_pickable(&mut self, pickable: bool) {
        self.players_pickable = pickable;
    }

    /// The hits on players since the last call.
    pub fn take_player_hits(&mut self) -> Vec<PlayerHit> {
        std::mem::take(&mut self.player_hits)
    }

    /// A player's fight memory.
    pub fn player_fights(&self, player: u64) -> PlayerFights {
        self.player_fights.get(&player).copied().unwrap_or_default()
    }

    /// Sets a player's `tickCount` (a harness probe's never advances).
    pub fn set_player_tick_count(&mut self, player: u64, ticks: i32) {
        self.player_fights.entry(player).or_default().tick_count = ticks;
    }

    /// A ticking player's `tickCount` advances, and `LivingEntity.baseTick`
    /// forgets a dead victim, and an attacker once dead or 100 ticks on.
    pub fn tick_player(&mut self, player: u64) {
        let alive = |world: &Self, target: crate::monster_ai::Target| match target {
            crate::monster_ai::Target::Mob(id) | crate::monster_ai::Target::Villager(id) => world.mob_body(id).is_some_and(|(_, health)| health > 0.0),
            crate::monster_ai::Target::Player(_) => true,
        };
        let mut fights = self.player_fights(player);
        fights.tick_count += 1;
        if fights.hurt_mob.is_some_and(|(t, _)| !alive(self, t)) {
            fights.hurt_mob = None;
        }
        if let Some((t, stamp)) = fights.hurt_by {
            if !alive(self, t) || fights.tick_count - stamp > 100 {
                fights.hurt_by = None;
            }
        }
        self.player_fights.insert(player, fights);
    }

    /// A hit that landed on a player: its source becomes the player's
    /// `lastHurtByMob` (at its `tickCount`) and the damage its
    /// `lastDamageSource`.
    pub fn player_hurt(&mut self, player: u64, source: Option<u64>, kind: &'static str) {
        let target = source.map(|id| if self.villagers.iter().any(|e| e.id == id) { crate::monster_ai::Target::Villager(id) } else { crate::monster_ai::Target::Mob(id) });
        let time = self.game_time;
        let fights = self.player_fights.entry(player).or_default();
        if let Some(target) = target {
            fights.hurt_by = Some((target, fights.tick_count));
        }
        fights.last_damage = Some((kind, time));
    }

    fn apply_arrow_hit(&mut self, hit: ArrowImpact, owner: u64) -> Option<DamageResult> {
        let id = hit.target_id;
        let time = self.game_time;
        let source = DamageSourceKind::Projectile;
        let mut outcome = None;
        if let Some(entity) = self.bats.iter_mut().find(|entity| entity.id == id) {
            let result = entity.hurt(hit.damage);
            outcome = Some(result);
            if result.applied {
                projectile_knockback(&mut entity.bat.body, &mut entity.random, hit.velocity);
            }
        } else if let Some(entity) = self.zombies.iter_mut().find(|entity| entity.id == id) {
            let amount = damage_after_armor(hit.damage, 2.0, 0.0);
            let result = entity.hurt(amount);
            outcome = Some(result);
            if result.applied {
                projectile_knockback(&mut entity.zombie.body, &mut entity.random, hit.velocity);
            }
        } else if let Some(entity) = self.skeletons.iter_mut().find(|entity| entity.id == id) {
            let result = entity.hurt(hit.damage);
            outcome = Some(result);
            if result.applied {
                projectile_knockback(&mut entity.skeleton.body, &mut entity.random, hit.velocity);
            }
        } else if let Some(entity) = self.creepers.iter_mut().find(|entity| entity.id == id) {
            let result = entity.hurt(hit.damage);
            outcome = Some(result);
            if result.applied {
                projectile_knockback(&mut entity.creeper.body, &mut entity.random, hit.velocity);
            }
        } else if let Some(entity) = self.spiders.iter_mut().find(|entity| entity.id == id) {
            let result = entity.hurt(hit.damage);
            outcome = Some(result);
            if result.applied {
                projectile_knockback(&mut entity.spider.body, &mut entity.random, hit.velocity);
            }
        } else if let Some(entity) = self.slimes.iter_mut().find(|entity| entity.id == id) {
            let result = entity.hurt(hit.damage);
            outcome = Some(result);
            if result.applied {
                projectile_knockback(&mut entity.slime.body, &mut entity.random, hit.velocity);
            }
        } else if let Some(entity) = self.endermen.iter_mut().find(|entity| entity.id == id) {
            entity.dodge_pending = true;
            outcome = Some(DamageResult { applied: false, dealt: 0.0, died: false, full: false });
        } else if let Some(entity) = self.witches.iter_mut().find(|entity| entity.id == id) {
            let result = entity.hurt(hit.damage, false, false);
            outcome = Some(result);
            if result.applied {
                projectile_knockback(&mut entity.witch.body, &mut entity.random, hit.velocity);
            }
        } else if let Some(entity) = self.iron_golems.iter_mut().find(|entity| entity.id == id) {
            // `KNOCKBACK_RESISTANCE` 1: no knockback, and no draw for it; the
            // shooter is remembered (`setLastHurtByMob`).
            let result = entity.hurt(hit.damage);
            if result.applied {
                entity.ai.state.hurt_by = Some((crate::monster_ai::Target::Mob(owner), entity.tick_count));
            }
            outcome = Some(result);
        } else if let Some(entity) = self.villagers.iter_mut().find(|entity| entity.id == id) {
            // `arrow(this, owner)`: the shooter is the one behind it.
            let result = entity.hurt_from(hit.damage, "minecraft:arrow", Some(owner), time);
            outcome = Some(result);
            if result.applied {
                projectile_knockback(&mut entity.villager.body, &mut entity.random, hit.velocity);
            }
        } else if let Some(entity) = self.wolves.iter_mut().find(|entity| entity.id == id) {
            let time = self.game_time;
            let result = entity.hurt_from(hit.damage, "minecraft:arrow", Some(crate::monster_ai::Target::Mob(owner)), time);
            outcome = Some(result);
            if result.applied {
                projectile_knockback(&mut entity.wolf.body, &mut entity.random, hit.velocity);
            }
        } else if let Some(entity) = self.cows.iter_mut().find(|entity| entity.id == id) {
            let result = entity.hurt(hit.damage, source);
            outcome = Some(result);
            if result.applied {
                projectile_knockback(&mut entity.cow.body, &mut entity.random, hit.velocity);
            }
        } else if let Some(entity) = self.sheep.iter_mut().find(|entity| entity.id == id) {
            let result = entity.hurt(hit.damage, source);
            outcome = Some(result);
            if result.applied {
                projectile_knockback(&mut entity.body, &mut entity.random, hit.velocity);
            }
        } else if let Some(entity) = self.pigs.iter_mut().find(|entity| entity.id == id) {
            let result = entity.hurt(hit.damage, source);
            outcome = Some(result);
            if result.applied {
                projectile_knockback(&mut entity.pig.body, &mut entity.random, hit.velocity);
            }
        } else if let Some(entity) = self.chickens.iter_mut().find(|entity| entity.id == id) {
            let result = entity.hurt_with_source(hit.damage, source);
            outcome = Some(result);
            if result.applied {
                projectile_knockback(&mut entity.chicken.body, &mut entity.random, hit.velocity);
            }
        }
        if outcome.is_some_and(|r| r.applied) {
            self.villager_hurt_by(id, owner);
        }
        outcome
    }

    pub fn spawn_zombie(&mut self, zombie: Zombie, no_ai: bool) -> u64 {
        assert!(
            no_ai,
            "use spawn_zombie_pursuit for the measured active goal profile"
        );
        self.spawn_zombie_internal(zombie, true, false)
    }

    /// A zombie keeping only `ZombieAttackGoal` of its goals (its target
    /// selector whole), as the goal-filtered fixtures measure it.
    pub fn spawn_zombie_pursuit(&mut self, zombie: Zombie) -> u64 {
        self.spawn_zombie_keeping(zombie, &["ZombieAttackGoal"])
    }

    /// Isolated land combat with DrownedAttackGoal and vanilla target goals.
    /// The caller supplies a drowned body; aquatic goals remain outside this profile.
    pub fn spawn_drowned_pursuit(&mut self, mut drowned: Zombie) -> u64 {
        drowned.kind = ZombieKind::Drowned;
        drowned.body.step_height = 1.0;
        self.spawn_zombie_internal(drowned, false, true)
    }

    /// Isolates the source drowning tracker without unrelated hostile goals.
    pub fn spawn_zombie_drowning(&mut self, zombie: Zombie) -> u64 {
        self.spawn_zombie_internal(zombie, false, false)
    }

    fn spawn_zombie_internal(&mut self, zombie: Zombie, no_ai: bool, attack_only: bool) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let previous_position = zombie.body.position;
        self.zombies.push(ZombieEntity {
            effects: Default::default(),
            id,
            zombie,
            no_ai,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            underwater_last_tick: false,
            previous_position,
            random: LegacyRandom::new(0),
            attack_only,
            target_player_id: None,
            target_villager_id: None,
            pending_attack_villager: None,
            pending_attack_player: None,
            aggressive: false,
            attack_goal_running: false,
            navigation: GroundNavigation::default(),
            move_control: MoveControl::default(),
            look_control: LookControl::default(),
            body_rotation: BodyRotation::default(),
            yaw: 0.0,
            speed: 0.0,
            forward: 0.0,
            sideways: 0.0,
            last_can_use_check: 0,
            ticks_until_next_path_recalculation: 0,
            ticks_until_next_attack: 0,
            pathed_target: DVec3::ZERO,
            raise_arm_ticks: 0,
            ai: None,
        });
        self.order.push(EntityKey::Zombie(id));
        self.file_in_section(EntityKey::Zombie(id));
        id
    }

    /// A zombie on the goal framework keeping only the named goals (the
    /// harness's `entity_keep_goal`), for the goal-filtered fixtures.
    pub fn spawn_zombie_keeping(&mut self, zombie: Zombie, goals: &[&str]) -> u64 {
        let id = self.spawn_zombie_active(zombie, 0.0);
        self.zombie_mut(id).unwrap().ai.as_deref_mut().unwrap().retain_goals(goals);
        id
    }

    /// A zombie with its whole goal and target selectors
    /// (`Zombie.registerGoals`), facing `yaw`.
    pub fn spawn_zombie_active(&mut self, zombie: Zombie, yaw: f32) -> u64 {
        let mut ai = MonsterAi::of_kind(crate::monster_ai::MonsterKind::Zombie, &zombie.body, yaw);
        ai.state.eye_height = zombie.eye_height();
        ai.state.max_health = 20.0;
        let id = self.spawn_zombie_internal(zombie, false, false);
        let entity = self.zombie_mut(id).unwrap();
        entity.yaw = yaw;
        entity.body_rotation = ai.body_rotation.clone();
        entity.look_control = ai.state.look_control.clone();
        entity.ai = Some(Box::new(ai));
        id
    }

    /// `ConversionType.SINGLE`: discard the old zombie and insert a new
    /// drowned after copying its position, motion and common mob state.
    /// `Zombie.doUnderWaterConversion`: a zombie becomes a drowned (on its
    /// water profile), a husk a zombie on the goal framework.
    fn convert_drowning_zombie(&mut self, old_id: u64) -> u64 {
        let Some(old) = self.zombies.iter().find(|entity| entity.id == old_id) else { return old_id };
        if old.zombie.kind != ZombieKind::Husk {
            return self.convert_zombie_to_drowned(old_id);
        }
        let index = self.zombies.iter().position(|entity| entity.id == old_id).unwrap();
        let old = self.zombies.remove(index);
        self.order.retain(|key| !matches!(key, EntityKey::Zombie(id) if *id == old_id));
        let mut zombie = Zombie::new(old.zombie.body.position);
        zombie.set_baby(old.zombie.baby);
        zombie.body.velocity = old.zombie.body.velocity;
        zombie.body.on_ground = old.zombie.body.on_ground;
        zombie.body.fall_distance = old.zombie.body.fall_distance;
        zombie.damage = old.zombie.damage;
        zombie.persistence_required = old.zombie.persistence_required;
        zombie.can_break_doors = old.zombie.can_break_doors;
        // `ConversionParams.keepEquipment`.
        zombie.head_item = old.zombie.head_item;
        zombie.main_hand = old.zombie.main_hand.clone();
        let yaw = old.yaw;
        // A zombie off the goal framework (the drowning model) converts
        // into another.
        if old.ai.is_none() {
            return self.spawn_zombie_internal(zombie, old.no_ai, false);
        }
        self.spawn_zombie_active(zombie, yaw)
    }

    fn convert_zombie_to_drowned(&mut self, old_id: u64) -> u64 {
        let index = self
            .zombies
            .iter()
            .position(|entity| entity.id == old_id)
            .unwrap();
        let old = self.zombies.remove(index);
        self.order
            .retain(|key| !matches!(key, EntityKey::Zombie(id) if *id == old_id));
        let mut drowned = Zombie::new(old.zombie.body.position);
        drowned.kind = ZombieKind::Drowned;
        drowned.set_baby(old.zombie.baby);
        drowned.body.velocity = old.zombie.body.velocity;
        drowned.body.on_ground = old.zombie.body.on_ground;
        drowned.body.fall_distance = old.zombie.body.fall_distance;
        drowned.body.step_height = 1.0;
        drowned.damage = old.zombie.damage;
        drowned.persistence_required = old.zombie.persistence_required;
        drowned.can_break_doors = old.zombie.can_break_doors;
        // `ConversionParams.keepEquipment`.
        drowned.head_item = old.zombie.head_item;
        drowned.main_hand = old.zombie.main_hand.clone();
        self.spawn_zombie_internal(drowned, old.no_ai, false)
    }

    /// The next spawned entity takes `id` (vanilla's `ENTITY_COUNTER` in a
    /// replay: goal ticks alternate by `tickCount + getId()`).
    pub fn set_next_entity_id(&mut self, id: u64) {
        self.next_id = id - 1;
    }

    /// The ID the next entity takes.
    pub fn next_entity_id(&self) -> u64 {
        self.next_id + 1
    }

    pub fn set_game_time(&mut self, game_time: i64) {
        self.game_time = game_time;
    }

    /// Source `Level.isBrightOutside` input for daylight-sensitive mob goals.
    pub fn set_bright_outside(&mut self, bright_outside: bool) {
        self.bright_outside = bright_outside;
    }

    pub fn game_time(&self) -> i64 {
        self.game_time
    }

    pub fn spawn_villager(&mut self, villager: Villager, no_ai: bool) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let previous_position = villager.body.position;
        self.villagers.push(VillagerEntity {
            effects: Default::default(),
            id,
            villager,
            no_ai,
            yaw: 0.0,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            previous_position,
            random: LegacyRandom::new(0),
            ai: None,
            last_damage: None,
            sleeping: None,
            wake_pending: false,
            offers: None,
            last_restock: 0,
            restocks_today: 0,
            last_restock_check_day: 0,
            gossips: Arc::default(),
            last_gossip_decay: 0,
            unhappy: 0,
            trading_player: None,
            last_traded_player: None,
            held_item: None,
            inventory: Default::default(),
            food_level: 0,
            can_pick_up_loot: true,
            tracked_items: Vec::new(),
        });
        self.order.push(EntityKey::Villager(id));
        self.file_in_section(EntityKey::Villager(id));
        id
    }

    pub fn villagers(&self) -> &[VillagerEntity] {
        &self.villagers
    }

    /// Pins the random of the next baby villagers make (vanilla's is
    /// unseeded), after those already pinned.
    pub fn push_born_seed(&mut self, seed: u64) {
        self.born_seeds.push_back(seed);
    }

    /// The babies villagers made, oldest first.
    pub fn take_born_wolves(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.born_wolves)
    }

    pub fn take_born_villagers(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.born_villagers)
    }

    /// An entity's UUID (players under `PLAYER_TARGET` plus their ID), as
    /// loaded or summoned; gossip is kept by it.
    pub fn set_uuid(&mut self, id: u64, uuid: u128) {
        self.uuids.insert(id, uuid);
    }

    /// A player's UUID, by the player's ID.
    pub fn set_player_uuid(&mut self, player: u64, uuid: u128) {
        self.uuids.insert(PLAYER_TARGET + player, uuid);
    }

    /// What makes the UUIDs of mobs with none of their own differ between
    /// sessions (vanilla draws them at random); players' stay as they are.
    pub fn set_uuid_salt(&mut self, salt: u64) {
        self.uuid_salt = salt;
    }

    /// An entity's UUID where known, else one made from its ID.
    pub fn uuid_of(&self, id: u64) -> u128 {
        self.uuids.get(&id).copied().unwrap_or_else(|| fallback_uuid(if id >= PLAYER_TARGET { id } else { id ^ self.uuid_salt }))
    }

    /// `Villager.setLastHurtByMob` for a hit that landed with a living
    /// entity behind it: `VILLAGER_HURT`, 25 minor negative gossip about
    /// the attacker. Nothing for others than villagers.
    pub fn villager_hurt_by(&mut self, villager: u64, attacker: u64) {
        let uuid = self.uuid_of(attacker);
        if let Some(entity) = self.villager_mut(villager) {
            Arc::make_mut(&mut entity.gossips).add(uuid, crate::gossip::GossipType::MinorNegative, 25);
            // A living villager hurt by a player shows its anger.
            if entity.villager.health > 0.0 && attacker >= PLAYER_TARGET {
                self.entity_events.push((villager, 13));
            }
        }
    }

    /// A player's main-hand item, as villagers see it.
    pub fn set_player_main_hand(&mut self, player: u64, item: Option<&str>) {
        match item.filter(|i| *i != "minecraft:air") {
            Some(item) => {
                self.player_main_hands.insert(player, item.to_owned());
            }
            None => {
                self.player_main_hands.remove(&player);
            }
        }
    }

    /// Entity events (`broadcastEntityEvent`) since last taken.
    pub fn take_entity_events(&mut self) -> Vec<(u64, u8)> {
        std::mem::take(&mut self.entity_events)
    }

    pub fn villager_mut(&mut self, id: u64) -> Option<&mut VillagerEntity> {
        self.villagers.iter_mut().find(|entity| entity.id == id)
    }

    /// The iron golems and monsters as goals see them (the targets besides
    /// players and villagers: golems for the monsters that hunt them,
    /// monsters for golems, and whoever hurt a mob), in world order.
    pub(crate) fn mob_candidates(&self) -> Vec<crate::monster_ai::MobCandidate> {
        use crate::monster_ai::MobCandidate;
        let mut out = Vec::new();
        let mut add = |id: u64, kind: &'static str, body: &Body, eye_height: f32, alive: bool| {
            out.push(MobCandidate { id, position: body.position, eye_height, width: body.width, height: body.height, alive, kind });
        };
        for e in &self.iron_golems {
            add(e.id, "minecraft:iron_golem", &e.golem.body, crate::iron_golem::EYE_HEIGHT, e.golem.health > 0.0);
        }
        for e in &self.zombies {
            add(e.id, e.zombie.kind.type_id(), &e.zombie.body, e.zombie.eye_height(), e.zombie.health > 0.0);
        }
        for e in &self.skeletons {
            add(e.id, e.skeleton.kind.type_id(), &e.skeleton.body, e.skeleton.eye_height(), e.skeleton.health > 0.0);
        }
        for e in self.creepers.iter().filter(|e| !e.creeper.exploded) {
            add(e.id, "minecraft:creeper", &e.creeper.body, e.creeper.body.height * 0.85, e.creeper.health > 0.0);
        }
        for e in &self.spiders {
            add(e.id, "minecraft:spider", &e.spider.body, e.spider.eye_height(), e.spider.health > 0.0);
        }
        for e in &self.slimes {
            add(e.id, "minecraft:slime", &e.slime.body, e.slime.eye_height(), e.slime.health > 0.0);
        }
        for e in &self.endermen {
            add(e.id, "minecraft:enderman", &e.enderman.body, e.enderman.eye_height(), e.enderman.health > 0.0);
        }
        for e in &self.witches {
            add(e.id, "minecraft:witch", &e.witch.body, crate::witch::EYE_HEIGHT, e.witch.health > 0.0);
        }
        for e in &self.wolves {
            add(e.id, "minecraft:wolf", &e.wolf.body, e.wolf.eye_height(), e.wolf.health > 0.0);
        }
        for e in &self.sheep {
            let baby = e.sheep.age.baby();
            add(e.id, "minecraft:sheep", &e.body, if baby { 0.6175 } else { 1.235 }, e.health > 0.0);
        }
        for e in &self.cows {
            let baby = e.cow.age.baby();
            let kind = if e.mooshroom.is_some() { "minecraft:mooshroom" } else { "minecraft:cow" };
            add(e.id, kind, &e.cow.body, if baby { 0.665 } else { 1.3 }, e.cow.health > 0.0);
        }
        for e in &self.pigs {
            let baby = e.pig.age.baby();
            add(e.id, "minecraft:pig", &e.pig.body, if baby { 0.3825 } else { 0.765 }, e.pig.health > 0.0);
        }
        for e in &self.chickens {
            let baby = e.chicken.age.baby();
            add(e.id, "minecraft:chicken", &e.chicken.body, if baby { 0.28125 } else { 0.644 }, e.chicken.health > 0.0);
        }
        out
    }

    /// `Mob.doHurtTarget` on another mob (`mob_attack` from `attacker`
    /// standing at `from`): the victim's `hurtServer` (its armor, the damage
    /// cooldown, the attacker remembered for its `HurtByTargetGoal`, and on
    /// a full hit the default knockback away from `from`, which knockback
    /// resistance cancels), then `lift` upwards on a hit that took
    /// (`IronGolem.doHurtTarget`). An enderman hurt by a living attacker
    /// does not teleport.
    pub(super) fn mob_hits_mob(&mut self, attacker: u64, victim: u64, damage: f32, from: DVec3, lift: f64) -> Option<DamageResult> {
        use crate::monster_ai::Target;
        let attacker_kind = self.entity_type(attacker)?;
        let by = Target::Mob(attacker);
        let time = self.game_time;
        let (result, resists) = if let Some(e) = self.iron_golems.iter_mut().find(|e| e.id == victim) {
            let hit = e.hurt(damage);
            if hit.applied {
                e.ai.state.hurt_by = Some((by, e.tick_count));
            }
            (hit, true)
        } else if let Some(e) = self.zombies.iter_mut().find(|e| e.id == victim) {
            // `Zombie.createAttributes`: 2 armor.
            let hit = e.hurt(damage_after_armor(damage, 2.0, 0.0));
            if hit.applied {
                if let Some(ai) = e.ai.as_deref_mut() {
                    ai.state.hurt_by = Some((by, e.tick_count));
                }
            }
            (hit, false)
        } else if let Some(e) = self.skeletons.iter_mut().find(|e| e.id == victim) {
            let hit = e.hurt(damage);
            if hit.applied {
                if let Some(ai) = e.ai.as_deref_mut() {
                    ai.state.hurt_by = Some((by, e.tick_count));
                }
            }
            (hit, false)
        } else if let Some(e) = self.creepers.iter_mut().find(|e| e.id == victim && !e.creeper.exploded) {
            let hit = e.hurt(damage);
            if hit.applied {
                e.ai.state.hurt_by = Some((by, e.tick_count));
            }
            (hit, false)
        } else if let Some(e) = self.spiders.iter_mut().find(|e| e.id == victim) {
            let hit = e.hurt(damage);
            if hit.applied {
                e.ai.state.hurt_by = Some((by, e.tick_count));
            }
            (hit, false)
        } else if let Some(e) = self.slimes.iter_mut().find(|e| e.id == victim) {
            let hit = e.hurt(damage);
            if hit.applied {
                e.ai.state.hurt_by = Some((by, e.tick_count));
            }
            (hit, false)
        } else if let Some(e) = self.endermen.iter_mut().find(|e| e.id == victim) {
            let hit = e.hurt(damage);
            if hit.applied {
                e.ai.state.hurt_by = Some((by, e.tick_count));
            }
            (hit, false)
        } else if let Some(e) = self.witches.iter_mut().find(|e| e.id == victim) {
            let hit = e.hurt(damage, false, false);
            if hit.applied {
                e.ai.state.hurt_by = Some((by, e.tick_count));
            }
            (hit, false)
        } else if let Some(e) = self.villagers.iter_mut().find(|e| e.id == victim) {
            let hit = e.hurt_from(damage, "minecraft:mob_attack", Some(attacker), time);
            if hit.applied {
                self.villager_hurt_by(victim, attacker);
            }
            (hit, false)
        } else if let Some(e) = self.wolves.iter_mut().find(|e| e.id == victim) {
            (e.hurt_from(damage, "minecraft:mob_attack", Some(by), time), false)
        } else if let Some(e) = self.sheep.iter_mut().find(|e| e.id == victim) {
            (e.hurt(damage, DamageSourceKind::MobAttack), false)
        } else if let Some(e) = self.cows.iter_mut().find(|e| e.id == victim) {
            (e.hurt(damage, DamageSourceKind::MobAttack), false)
        } else if let Some(e) = self.pigs.iter_mut().find(|e| e.id == victim) {
            (e.hurt(damage, DamageSourceKind::MobAttack), false)
        } else if let Some(e) = self.chickens.iter_mut().find(|e| e.id == victim) {
            (e.hurt_with_source(damage, DamageSourceKind::MobAttack), false)
        } else {
            return None;
        };
        self.credit(victim, result, attacker_kind, true);
        if result.applied && result.full && !resists {
            if let Some((body, _)) = self.mob_body(victim) {
                let (xd, zd) = (from.x - body.position.x, from.z - body.position.z);
                self.knock_back(victim, f64::from(0.4_f32), xd, zd);
            }
        }
        if result.applied && lift > 0.0 && !resists {
            if let Some(body) = self.body_mut(victim) {
                body.velocity.y += lift;
            }
        }
        Some(result)
    }

    /// `HurtByTargetGoal.alertOthers` for a zombie: the other zombies within
    /// follow range around it (ten blocks up and down) with no target turn
    /// on the attacker.
    fn alert_zombies(&mut self, zombie_id: u64, attacker: crate::monster_ai::Target) {
        let Some(source) = self.zombies.iter().find(|e| e.id == zombie_id) else { return };
        let Some(range) = source.ai.as_ref().map(|ai| ai.state.follow_range) else { return };
        let p = source.zombie.body.position;
        // `AABB.unitCubeFromLowerCorner(position).inflate(within, 10, within)`.
        let (min, max) = (p - DVec3::new(range, 10.0, range), p + DVec3::new(1.0 + range, 11.0, 1.0 + range));
        // `getEntitiesOfClass(mob.getClass())`: a zombie's class covers every
        // kind; the others only their own.
        let kind = source.zombie.kind;
        for other in &mut self.zombies {
            if other.id == zombie_id || other.zombie.health <= 0.0 || (kind != ZombieKind::Zombie && other.zombie.kind != kind) {
                continue;
            }
            let Some(ai) = other.ai.as_deref_mut() else { continue };
            let b = &other.zombie.body;
            let half = f64::from(b.width / 2.0);
            let q = b.position;
            let touches = q.x - half < max.x && q.x + half > min.x && q.y < max.y && q.y + f64::from(b.height) > min.y && q.z - half < max.z && q.z + half > min.z;
            if touches && ai.state.target().is_none() {
                ai.state.target = Some(attacker);
            }
        }
    }

    pub fn zombies(&self) -> &[ZombieEntity] {
        &self.zombies
    }

    pub fn zombie_mut(&mut self, id: u64) -> Option<&mut ZombieEntity> {
        self.zombies.iter_mut().find(|entity| entity.id == id)
    }

    pub fn spawn_bat(&mut self, bat: Bat, no_ai: bool) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let previous_position = bat.body.position;
        self.bats.push(BatEntity {
            effects: Default::default(),
            id,
            bat,
            no_ai,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            previous_position,
            yaw: 0.0,
            forward: 0.0,
            random: LegacyRandom::new(0),
            target_position: None,
            look_control: LookControl::new(0.0),
            body_rotation: BodyRotation::new(0.0),
        });
        self.order.push(EntityKey::Bat(id));
        id
    }

    pub fn bats(&self) -> &[BatEntity] {
        &self.bats
    }

    pub fn bat_mut(&mut self, id: u64) -> Option<&mut BatEntity> {
        self.bats.iter_mut().find(|entity| entity.id == id)
    }

    pub fn set_mob_griefing(&mut self, enabled: bool) {
        self.mob_griefing = enabled;
    }

    /// The `mob_griefing` game rule.
    pub fn mob_griefing(&self) -> bool {
        self.mob_griefing
    }

    pub fn set_mob_drops(&mut self, enabled: bool) {
        self.mob_drops = enabled;
    }

    /// The `mob_drops` game rule.
    pub fn mob_drops(&self) -> bool {
        self.mob_drops
    }

    /// The deaths since the last call, in the order they happened, as the
    /// dead mobs' drops see them. With `mob_drops` off nothing drops. A
    /// player's kill may shake a skeleton's bow loose
    /// (`Mob.dropCustomDeathLoot`: 8.5% and a random wear), rolled here from
    /// the dead mob's random, which nothing draws from once it has died.
    pub fn take_deaths(&mut self) -> Vec<MobDeath> {
        let mut deaths = Vec::new();
        let plain = EntityLootContext::default();
        let mob_drops = self.mob_drops;
        // Animals, bats and villagers drop their loot only as adults
        // (`LivingEntity.shouldDropLoot`); monsters as babies too.
        let adult = |baby: bool, table: &'static str| (!baby).then_some(table);
        // `dropExperience`: only a kill within a player's memory leaves
        // experience, from monsters at any age and other mobs as adults
        // (`shouldDropExperience`).
        let earns = |death: &MobDeath, monster: bool| mob_drops && death.context.killed_by_player && (monster || !death.context.baby);
        // `Animal.getBaseExperienceReward`: 1 to 3, from the mob's random.
        let animal = |death: &mut MobDeath, random: &mut LegacyRandom| {
            if earns(death, false) {
                death.experience = 1 + random.next_int(3) as i32;
            }
        };
        for e in &mut self.bats {
            deaths.extend(death_of(e.id, &mut e.bat.damage, "minecraft:bat", Some("minecraft:bat"), plain));
        }
        for e in &mut self.zombies {
            let kind = e.zombie.kind.type_id();
            let Some((order, mut death)) = death_of(e.id, &mut e.zombie.damage, kind, Some(kind), EntityLootContext { baby: e.zombie.baby, ..plain }) else { continue };
            // `Monster`'s 5, two and a half times for a baby
            // (`Zombie.getBaseExperienceReward`).
            if earns(&death, true) {
                death.experience = if e.zombie.baby { 12 } else { 5 };
            }
            deaths.push((order, death));
        }
        for e in &mut self.skeletons {
            let kind = e.skeleton.kind.type_id();
            let Some((order, mut death)) = death_of(e.id, &mut e.skeleton.damage, kind, Some(kind), plain) else { continue };
            // The main hand's drop chance is drawn for a player's kill (any
            // kill when preserved); an unpreserved bow keeps a random share
            // of its wear: `max - nextInt(1 + nextInt(max(max - 3, 1)))`.
            let chance = e.skeleton.bow_drop_chance;
            let preserve = chance > 1.0;
            if mob_drops && chance != 0.0 && e.skeleton.holds_bow && (death.context.killed_by_player || preserve) && e.random.next_float() < chance {
                let mut bow = ItemStack::new("minecraft:bow", 1);
                bow.max = 1;
                if !preserve {
                    let spread = e.random.next_int(BOW_DURABILITY - 3);
                    let wear = BOW_DURABILITY - e.random.next_int(1 + spread);
                    bow.components = Some(serde_json::json!({ "minecraft:damage": wear }));
                }
                death.equipment.push(bow);
                e.skeleton.holds_bow = false;
            }
            // `Mob.getBaseExperienceReward`: 5, and 1 to 3 more for a bow
            // still held that is not preserved.
            if earns(&death, true) {
                let bow = if e.skeleton.holds_bow && chance <= 1.0 { 1 + e.random.next_int(3) as i32 } else { 0 };
                death.experience = 5 + bow;
            }
            deaths.push((order, death));
        }
        for e in &mut self.creepers {
            let Some((order, mut death)) = death_of(e.id, &mut e.creeper.damage, "minecraft:creeper", Some("minecraft:creeper"), plain) else { continue };
            if earns(&death, true) {
                death.experience = 5;
            }
            deaths.push((order, death));
        }
        for e in &mut self.endermen {
            let carried = e.ai.state.enderman.carried.take();
            let Some((order, mut death)) = death_of(e.id, &mut e.enderman.damage, "minecraft:enderman", Some("minecraft:enderman"), plain) else {
                e.ai.state.enderman.carried = carried;
                continue;
            };
            if earns(&death, true) {
                death.experience = 5;
            }
            // `dropCustomDeathLoot`: the carried block's drops for a
            // silk-touch axe, which for every holdable block is itself.
            if let Some(block) = carried {
                death.equipment.push(ItemStack::new(&block.id, 1));
            }
            deaths.push((order, death));
        }
        for e in &mut self.witches {
            let held = e.witch.drinking.take();
            let Some((order, mut death)) = death_of(e.id, &mut e.witch.damage, "minecraft:witch", Some("minecraft:witch"), plain) else {
                e.witch.drinking = held;
                continue;
            };
            // `dropCustomDeathLoot`: the potion it was drinking, for a
            // player's kill, 8.5% of the time (`DropChances.DEFAULT`).
            if let Some(potion) = held {
                if mob_drops && death.context.killed_by_player && e.random.next_float() < 0.085 {
                    let mut stack = ItemStack::new("minecraft:potion", 1);
                    stack.components = Some(serde_json::json!({ "minecraft:potion_contents": { "potion": potion.id() } }));
                    death.equipment.push(stack);
                }
            }
            if earns(&death, true) {
                death.experience = 5;
            }
            deaths.push((order, death));
        }
        for e in &mut self.iron_golems {
            // `Mob.xpReward` stays 0 for golems.
            deaths.extend(death_of(e.id, &mut e.golem.damage, "minecraft:iron_golem", Some("minecraft:iron_golem"), plain));
        }
        for e in &mut self.slimes {
            let context = EntityLootContext { cube_size: Some(e.slime.size), ..plain };
            let Some((order, mut death)) = death_of(e.id, &mut e.slime.damage, "minecraft:slime", Some("minecraft:slime"), context) else { continue };
            // `Slime.setSize`: `xpReward` is the size.
            if earns(&death, true) {
                death.experience = e.slime.size;
            }
            deaths.push((order, death));
        }
        for e in &mut self.spiders {
            let Some((order, mut death)) = death_of(e.id, &mut e.spider.damage, "minecraft:spider", Some("minecraft:spider"), plain) else { continue };
            if earns(&death, true) {
                death.experience = 5;
            }
            deaths.push((order, death));
        }
        for e in &mut self.villagers {
            // Villagers leave no experience (`xpReward` 0).
            let table = adult(e.villager.age.baby(), "minecraft:villager");
            deaths.extend(death_of(e.id, &mut e.villager.damage, "minecraft:villager", table, EntityLootContext { baby: e.villager.age.baby(), ..plain }));
        }
        for e in &mut self.cows {
            let kind = if e.mooshroom.is_some() { "minecraft:mooshroom" } else { "minecraft:cow" };
            let baby = e.cow.age.baby();
            let Some((order, mut death)) = death_of(e.id, &mut e.cow.damage, kind, adult(baby, kind), EntityLootContext { baby, ..plain }) else { continue };
            animal(&mut death, &mut e.random);
            deaths.push((order, death));
        }
        for e in &mut self.wolves {
            let baby = e.wolf.baby();
            let Some((order, mut death)) = death_of(e.id, &mut e.wolf.damage, "minecraft:wolf", adult(baby, "minecraft:wolf"), EntityLootContext { baby, ..plain }) else { continue };
            animal(&mut death, &mut e.random);
            deaths.push((order, death));
        }
        for e in &mut self.sheep {
            let baby = e.sheep.age.baby();
            let context = EntityLootContext { baby, sheep_color: Some(e.sheep.wool.data() & 15), sheep_sheared: e.sheep.wool.sheared(), ..plain };
            let Some((order, mut death)) = death_of(e.id, &mut e.damage, "minecraft:sheep", adult(baby, "minecraft:sheep"), context) else { continue };
            animal(&mut death, &mut e.random);
            deaths.push((order, death));
        }
        for e in &mut self.pigs {
            let baby = e.pig.age.baby();
            let Some((order, mut death)) = death_of(e.id, &mut e.pig.damage, "minecraft:pig", adult(baby, "minecraft:pig"), EntityLootContext { baby, ..plain }) else { continue };
            animal(&mut death, &mut e.random);
            deaths.push((order, death));
        }
        for e in &mut self.chickens {
            let baby = e.chicken.age.baby();
            let Some((order, mut death)) = death_of(e.id, &mut e.chicken.damage, "minecraft:chicken", adult(baby, "minecraft:chicken"), EntityLootContext { baby, ..plain }) else { continue };
            animal(&mut death, &mut e.random);
            deaths.push((order, death));
        }
        deaths.sort_by_key(|&(order, _)| order);
        if !mob_drops {
            return Vec::new();
        }
        deaths.into_iter().map(|(_, death)| death).collect()
    }

    /// An entity's type, by ID, while it is in the world (dying included).
    pub fn entity_type(&self, id: u64) -> Option<&'static str> {
        if self.bats.iter().any(|e| e.id == id) {
            return Some("minecraft:bat");
        }
        if let Some(e) = self.zombies.iter().find(|e| e.id == id) {
            return Some(e.zombie.kind.type_id());
        }
        if let Some(e) = self.skeletons.iter().find(|e| e.id == id) {
            return Some(e.skeleton.kind.type_id());
        }
        if self.creepers.iter().any(|e| e.id == id) {
            return Some("minecraft:creeper");
        }
        if self.slimes.iter().any(|e| e.id == id) {
            return Some("minecraft:slime");
        }
        if self.endermen.iter().any(|e| e.id == id) {
            return Some("minecraft:enderman");
        }
        if self.witches.iter().any(|e| e.id == id) {
            return Some("minecraft:witch");
        }
        if self.iron_golems.iter().any(|e| e.id == id) {
            return Some("minecraft:iron_golem");
        }
        if self.wolves.iter().any(|e| e.id == id) {
            return Some("minecraft:wolf");
        }
        if self.potions.iter().any(|e| e.id == id) {
            return Some("minecraft:splash_potion");
        }
        if self.spiders.iter().any(|e| e.id == id) {
            return Some("minecraft:spider");
        }
        if self.arrows.iter().any(|e| e.id == id) {
            return Some("minecraft:arrow");
        }
        if self.villagers.iter().any(|e| e.id == id) {
            return Some("minecraft:villager");
        }
        if let Some(e) = self.cows.iter().find(|e| e.id == id) {
            return Some(if e.mooshroom.is_some() { "minecraft:mooshroom" } else { "minecraft:cow" });
        }
        if self.sheep.iter().any(|e| e.id == id) {
            return Some("minecraft:sheep");
        }
        if self.pigs.iter().any(|e| e.id == id) {
            return Some("minecraft:pig");
        }
        if self.chickens.iter().any(|e| e.id == id) {
            return Some("minecraft:chicken");
        }
        None
    }

    /// Every mob's body.
    fn bodies_mut(&mut self) -> impl Iterator<Item = &mut Body> {
        self.bats.iter_mut().map(|e| &mut e.bat.body)
            .chain(self.zombies.iter_mut().map(|e| &mut e.zombie.body))
            .chain(self.skeletons.iter_mut().map(|e| &mut e.skeleton.body))
            .chain(self.creepers.iter_mut().map(|e| &mut e.creeper.body))
            .chain(self.spiders.iter_mut().map(|e| &mut e.spider.body))
            .chain(self.slimes.iter_mut().map(|e| &mut e.slime.body))
            .chain(self.endermen.iter_mut().map(|e| &mut e.enderman.body))
            .chain(self.witches.iter_mut().map(|e| &mut e.witch.body))
            .chain(self.iron_golems.iter_mut().map(|e| &mut e.golem.body))
            .chain(self.wolves.iter_mut().map(|e| &mut e.wolf.body))
            .chain(self.villagers.iter_mut().map(|e| &mut e.villager.body))
            .chain(self.cows.iter_mut().map(|e| &mut e.cow.body))
            .chain(self.sheep.iter_mut().map(|e| &mut e.body))
            .chain(self.pigs.iter_mut().map(|e| &mut e.pig.body))
            .chain(self.chickens.iter_mut().map(|e| &mut e.chicken.body))
    }

    /// A mob's body, by ID (a loaded entity's motion and ground contact).
    pub fn body_mut(&mut self, id: u64) -> Option<&mut Body> {
        if let Some(e) = self.bats.iter_mut().find(|e| e.id == id) {
            Some(&mut e.bat.body)
        } else if let Some(e) = self.zombies.iter_mut().find(|e| e.id == id) {
            Some(&mut e.zombie.body)
        } else if let Some(e) = self.skeletons.iter_mut().find(|e| e.id == id) {
            Some(&mut e.skeleton.body)
        } else if let Some(e) = self.creepers.iter_mut().find(|e| e.id == id) {
            Some(&mut e.creeper.body)
        } else if let Some(e) = self.spiders.iter_mut().find(|e| e.id == id) {
            Some(&mut e.spider.body)
        } else if let Some(e) = self.slimes.iter_mut().find(|e| e.id == id) {
            Some(&mut e.slime.body)
        } else if let Some(e) = self.endermen.iter_mut().find(|e| e.id == id) {
            Some(&mut e.enderman.body)
        } else if let Some(e) = self.witches.iter_mut().find(|e| e.id == id) {
            Some(&mut e.witch.body)
        } else if let Some(e) = self.iron_golems.iter_mut().find(|e| e.id == id) {
            Some(&mut e.golem.body)
        } else if let Some(e) = self.wolves.iter_mut().find(|e| e.id == id) {
            Some(&mut e.wolf.body)
        } else if let Some(e) = self.villagers.iter_mut().find(|e| e.id == id) {
            Some(&mut e.villager.body)
        } else if let Some(e) = self.cows.iter_mut().find(|e| e.id == id) {
            Some(&mut e.cow.body)
        } else if let Some(e) = self.sheep.iter_mut().find(|e| e.id == id) {
            Some(&mut e.body)
        } else if let Some(e) = self.pigs.iter_mut().find(|e| e.id == id) {
            Some(&mut e.pig.body)
        } else if let Some(e) = self.chickens.iter_mut().find(|e| e.id == id) {
            Some(&mut e.chicken.body)
        } else {
            None
        }
    }

    /// A mob's damage state, by ID, to read.
    pub fn damage_state(&self, id: u64) -> Option<&DamageState> {
        None.or_else(|| self.bats.iter().find(|e| e.id == id).map(|e| &e.bat.damage))
            .or_else(|| self.zombies.iter().find(|e| e.id == id).map(|e| &e.zombie.damage))
            .or_else(|| self.skeletons.iter().find(|e| e.id == id).map(|e| &e.skeleton.damage))
            .or_else(|| self.creepers.iter().find(|e| e.id == id).map(|e| &e.creeper.damage))
            .or_else(|| self.spiders.iter().find(|e| e.id == id).map(|e| &e.spider.damage))
            .or_else(|| self.slimes.iter().find(|e| e.id == id).map(|e| &e.slime.damage))
            .or_else(|| self.endermen.iter().find(|e| e.id == id).map(|e| &e.enderman.damage))
            .or_else(|| self.witches.iter().find(|e| e.id == id).map(|e| &e.witch.damage))
            .or_else(|| self.iron_golems.iter().find(|e| e.id == id).map(|e| &e.golem.damage))
            .or_else(|| self.wolves.iter().find(|e| e.id == id).map(|e| &e.wolf.damage))
            .or_else(|| self.villagers.iter().find(|e| e.id == id).map(|e| &e.villager.damage))
            .or_else(|| self.cows.iter().find(|e| e.id == id).map(|e| &e.cow.damage))
            .or_else(|| self.sheep.iter().find(|e| e.id == id).map(|e| &e.damage))
            .or_else(|| self.pigs.iter().find(|e| e.id == id).map(|e| &e.pig.damage))
            .or_else(|| self.chickens.iter().find(|e| e.id == id).map(|e| &e.chicken.damage))
    }

    /// A mob's damage state, by ID.
    fn damage_state_mut(&mut self, id: u64) -> Option<&mut DamageState> {
        if let Some(e) = self.bats.iter_mut().find(|e| e.id == id) {
            Some(&mut e.bat.damage)
        } else if let Some(e) = self.zombies.iter_mut().find(|e| e.id == id) {
            Some(&mut e.zombie.damage)
        } else if let Some(e) = self.skeletons.iter_mut().find(|e| e.id == id) {
            Some(&mut e.skeleton.damage)
        } else if let Some(e) = self.creepers.iter_mut().find(|e| e.id == id) {
            Some(&mut e.creeper.damage)
        } else if let Some(e) = self.spiders.iter_mut().find(|e| e.id == id) {
            Some(&mut e.spider.damage)
        } else if let Some(e) = self.slimes.iter_mut().find(|e| e.id == id) {
            Some(&mut e.slime.damage)
        } else if let Some(e) = self.endermen.iter_mut().find(|e| e.id == id) {
            Some(&mut e.enderman.damage)
        } else if let Some(e) = self.witches.iter_mut().find(|e| e.id == id) {
            Some(&mut e.witch.damage)
        } else if let Some(e) = self.iron_golems.iter_mut().find(|e| e.id == id) {
            Some(&mut e.golem.damage)
        } else if let Some(e) = self.wolves.iter_mut().find(|e| e.id == id) {
            Some(&mut e.wolf.damage)
        } else if let Some(e) = self.villagers.iter_mut().find(|e| e.id == id) {
            Some(&mut e.villager.damage)
        } else if let Some(e) = self.cows.iter_mut().find(|e| e.id == id) {
            Some(&mut e.cow.damage)
        } else if let Some(e) = self.sheep.iter_mut().find(|e| e.id == id) {
            Some(&mut e.damage)
        } else if let Some(e) = self.pigs.iter_mut().find(|e| e.id == id) {
            Some(&mut e.pig.damage)
        } else if let Some(e) = self.chickens.iter_mut().find(|e| e.id == id) {
            Some(&mut e.chicken.damage)
        } else {
            None
        }
    }

    /// Names the source of an applied hit on a mob (see
    /// [`DamageState::credit`]).
    fn credit(&mut self, id: u64, result: DamageResult, attacker: &'static str, direct: bool) {
        if let Some(damage) = self.damage_state_mut(id) {
            damage.credit(result, attacker, direct);
        }
    }

    /// The sounds mobs made since the last call, in world order by kind.
    pub fn take_sounds(&mut self) -> Vec<MobSound> {
        let mut out = std::mem::take(&mut self.sounds);
        for e in &mut self.zombies {
            let family = e.zombie.kind.sound_family();
            resolve_voices(&mut out, &mut e.voices, family, 1.0, "hostile_volume");
        }
        for e in &mut self.skeletons {
            resolve_voices(&mut out, &mut e.voices, e.skeleton.kind.sound_family(), 1.0, "hostile_volume");
        }
        for e in &mut self.creepers {
            resolve_voices(&mut out, &mut e.voices, "creeper", 1.0, "hostile_volume");
        }
        for e in &mut self.slimes {
            resolve_voices(&mut out, &mut e.voices, "slime", 1.0, "hostile_volume");
        }
        for e in &mut self.endermen {
            resolve_voices(&mut out, &mut e.voices, "enderman", 1.0, "hostile_volume");
        }
        for e in &mut self.witches {
            resolve_voices(&mut out, &mut e.voices, "witch", 1.0, "hostile_volume");
        }
        for e in &mut self.spiders {
            resolve_voices(&mut out, &mut e.voices, "spider", 1.0, "hostile_volume");
        }
        for e in &mut self.villagers {
            resolve_voices(&mut out, &mut e.voices, "villager", 1.0, "friendly_volume");
        }
        for e in &mut self.iron_golems {
            resolve_voices(&mut out, &mut e.voices, "iron_golem", 1.0, "friendly_volume");
        }
        for e in &mut self.wolves {
            resolve_voices(&mut out, &mut e.voices, "wolf", crate::wolf::SOUND_VOLUME, "friendly_volume");
        }
        for e in &mut self.cows {
            if let Some(horse) = &e.horse {
                resolve_voices(&mut out, &mut e.voices, horse.kind.sound_family(e.cow.age.baby()), crate::horse::SOUND_VOLUME, "neutral_volume");
                continue;
            }
            // Mooshrooms keep `AbstractCow`'s classic sound set.
            let family = if e.mooshroom.is_some() {
                "cow"
            } else if e.cow.sound_variant == crate::cow::CowSoundVariant::Moody {
                "cow_moody"
            } else {
                "cow"
            };
            // `Cow.getSoundVolume`.
            resolve_voices(&mut out, &mut e.voices, family, 0.4, "friendly_volume");
        }
        for e in &mut self.sheep {
            resolve_voices(&mut out, &mut e.voices, "sheep", 1.0, "friendly_volume");
        }
        for e in &mut self.pigs {
            // `Pig.getSoundSet`: a baby's own sounds whatever its variant.
            let family = match e.pig.sound_variant {
                _ if e.pig.age.baby() => "baby_pig",
                crate::pig::PigSoundVariant::Classic => "pig",
                crate::pig::PigSoundVariant::Mini => "pig_mini",
                crate::pig::PigSoundVariant::Big => "pig_big",
            };
            resolve_voices(&mut out, &mut e.voices, family, 1.0, "friendly_volume");
        }
        for e in &mut self.chickens {
            // `Chicken.getSoundSet`: a baby's own sounds whatever its variant.
            let family = match e.chicken.sound_variant {
                _ if e.chicken.age.baby() => "baby_chicken",
                crate::chicken::ChickenSoundVariant::Classic => "chicken",
                crate::chicken::ChickenSoundVariant::Picky => "chicken_picky",
            };
            resolve_voices(&mut out, &mut e.voices, family, 1.0, "friendly_volume");
        }
        for e in &mut self.bats {
            resolve_voices(&mut out, &mut e.voices, "bat", 0.1, "ambient_volume");
        }
        out
    }

    /// Every living entity's ID, in world order (arrows excluded).
    pub fn mob_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.order.iter().filter(|key| !matches!(key, EntityKey::Arrow(_))).map(|key| key.id())
    }
    pub fn set_recipe_book(&mut self, recipes: Arc<RecipeBook>) {
        self.recipes = Some(recipes);
    }
    pub fn spawn_cow(&mut self, cow: Cow, no_ai: bool) -> u64 {
        self.spawn_cow_with_kind(cow, None, no_ai)
    }

    pub fn spawn_mooshroom(&mut self, mooshroom: MushroomCow, no_ai: bool) -> u64 {
        self.spawn_cow_with_kind(mooshroom.cow, Some(mooshroom.state), no_ai)
    }

    /// A horse or donkey: a farm animal of its size and health with the
    /// horse goal set.
    pub fn spawn_horse(&mut self, mut cow: Cow, horse: crate::horse::HorseState, no_ai: bool) -> u64 {
        cow.dimensions = horse.kind.dimensions();
        cow.max_health = horse.max_health;
        cow.body.step_height = 1.0;
        let id = self.spawn_cow_with_kind(cow, None, no_ai);
        let entity = self.cows.iter_mut().find(|e| e.id == id).expect("just spawned");
        entity.horse = Some(horse);
        entity.goals = registered_horse_goals();
        id
    }

    fn spawn_cow_with_kind(
        &mut self,
        mut cow: Cow,
        mooshroom: Option<MushroomCowState>,
        no_ai: bool,
    ) -> u64 {
        self.next_id += 1;
        cow.sync_dimensions();
        let id = self.next_id;
        let previous_position = cow.body.position;
        let previous_yaw = cow.yaw;
        self.cows.push(CowEntity {
            effects: Default::default(),
            id,
            cow,
            mooshroom,
            horse: None,
            previous_position,
            previous_yaw,
            no_ai,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            fluid: FluidFrame::default(),
            jumping: false,
            no_jump_delay: 0,
            was_touching_water: false,
            last_damage_source: None,
            last_damage_tick: 0,
            follow_parent: FollowParentState::default(),
            breed: BreedState::default(),
            tempt: TemptState::default(),
            random_look: RandomLookState::default(),
            look_at_player: LookAtPlayerState::default(),
            stroll: StrollState::default(),
            panic: PanicState::default(),
            random: LegacyRandom::new(0),
            look_control: LookControl::new(previous_yaw),
            body_rotation: BodyRotation::new(previous_yaw),
            goals: registered_goals(),
        });
        self.order.push(EntityKey::Cow(id));
        self.file_in_section(EntityKey::Cow(id));
        id
    }

    pub fn spawn_sheep_no_ai(&mut self, sheep: Sheep, position: DVec3) -> u64 {
        self.spawn_sheep(sheep, position, true)
    }

    pub fn spawn_pig_no_ai(&mut self, pig: Pig) -> u64 {
        self.spawn_pig(pig, true)
    }

    pub fn spawn_pig(&mut self, mut pig: Pig, no_ai: bool) -> u64 {
        self.next_id += 1;
        pig.sync_dimensions();
        let id = self.next_id;
        let previous_position = pig.body.position;
        let previous_yaw = pig.yaw;
        self.pigs.push(PigEntity {
            effects: Default::default(),
            id,
            pig,
            previous_position,
            previous_yaw,
            no_ai,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            last_damage_source: None,
            last_damage_tick: 0,
            fluid: FluidFrame::default(),
            was_touching_water: false,
            jumping: false,
            no_jump_delay: 0,
            follow_parent: FollowParentState::default(),
            breed: BreedState::default(),
            steering: SteeringState::default(),
            panic: PanicState::default(),
            stroll: StrollState::default(),
            tempt_stick: TemptState::default(),
            tempt_food: TemptState::default(),
            random_look: RandomLookState::default(),
            look_at_player: LookAtPlayerState::default(),
            random: LegacyRandom::new(0),
            look_control: LookControl::new(previous_yaw),
            body_rotation: BodyRotation::new(previous_yaw),
            goals: registered_pig_goals(),
        });
        self.order.push(EntityKey::Pig(id));
        self.file_in_section(EntityKey::Pig(id));
        id
    }

    pub fn pigs(&self) -> &[PigEntity] {
        &self.pigs
    }

    pub fn pig_mut(&mut self, id: u64) -> Option<&mut PigEntity> {
        self.pigs.iter_mut().find(|entity| entity.id == id)
    }

    pub fn spawn_chicken(&mut self, mut chicken: Chicken, no_ai: bool) -> u64 {
        self.next_id += 1;
        chicken.sync_dimensions();
        let id = self.next_id;
        let previous_position = chicken.body.position;
        let previous_yaw = chicken.yaw;
        self.chickens.push(ChickenEntity {
            effects: Default::default(),
            id,
            chicken,
            previous_position,
            previous_yaw,
            no_ai,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            last_damage_source: None,
            last_damage_tick: 0,
            fluid: FluidFrame::default(),
            was_touching_water: false,
            jumping: false,
            no_jump_delay: 0,
            follow_parent: FollowParentState::default(),
            breed: BreedState::default(),
            panic: PanicState::default(),
            stroll: StrollState::default(),
            tempt: TemptState::default(),
            random_look: RandomLookState::default(),
            look_at_player: LookAtPlayerState::default(),
            random: LegacyRandom::new(0),
            look_control: LookControl::new(previous_yaw),
            body_rotation: BodyRotation::new(previous_yaw),
            eggs_laid: 0,
            goals: registered_chicken_goals(),
        });
        self.order.push(EntityKey::Chicken(id));
        self.file_in_section(EntityKey::Chicken(id));
        id
    }

    pub fn chickens(&self) -> &[ChickenEntity] {
        &self.chickens
    }

    pub fn chicken_mut(&mut self, id: u64) -> Option<&mut ChickenEntity> {
        self.chickens.iter_mut().find(|entity| entity.id == id)
    }

    pub fn spawn_sheep(&mut self, sheep: Sheep, position: DVec3, no_ai: bool) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let baby = sheep.age.baby();
        self.sheep.push(SheepEntity {
            effects: Default::default(),
            id,
            sheep,
            body: Body::new(
                position,
                if baby { 0.45 } else { 0.9 },
                if baby { 0.65 } else { 1.3 },
            ),
            health: 8.0,
            damage: DamageState::default(),
            no_ai,
            tick_count: 0,
            ambient_sound_time: 0,
            voices: Vec::new(),
            no_action_time: 0,
            last_damage_source: None,
            last_damage_tick: 0,
            fluid: FluidFrame::default(),
            jumping: false,
            no_jump_delay: 0,
            was_touching_water: false,
            eat_animation_ticks: 0,
            random: LegacyRandom::new(0),
            previous_position: position,
            previous_yaw: 0.0,
            navigation: GroundNavigation::default(),
            move_control: MoveControl::default(),
            look_control: LookControl::new(0.0),
            body_rotation: BodyRotation::new(0.0),
            yaw: 0.0,
            speed: 0.0,
            forward: 0.0,
            sideways: 0.0,
            breed: BreedState::default(),
            tempt: TemptState::default(),
            follow_parent: FollowParentState::default(),
            random_look: RandomLookState::default(),
            look_at_player: LookAtPlayerState::default(),
            stroll: StrollState::default(),
            panic: PanicState::default(),
            goals: registered_sheep_goals(),
        });
        self.order.push(EntityKey::Sheep(id));
        self.file_in_section(EntityKey::Sheep(id));
        id
    }

    pub fn sheep(&self) -> &[SheepEntity] {
        &self.sheep
    }

    pub fn sheep_mut(&mut self, id: u64) -> Option<&mut SheepEntity> {
        self.sheep.iter_mut().find(|entity| entity.id == id)
    }

    pub fn cows(&self) -> &[CowEntity] {
        &self.cows
    }

    pub fn cow_mut(&mut self, id: u64) -> Option<&mut CowEntity> {
        self.cows.iter_mut().find(|entity| entity.id == id)
    }

    pub fn mooshroom_mut(&mut self, id: u64) -> Option<&mut CowEntity> {
        self.cows
            .iter_mut()
            .find(|entity| entity.id == id && entity.mooshroom.is_some())
    }

    /// Adult shearing converts the mob to an ordinary cow and emits the
    /// variant's five individual mushroom drops at the caller's world layer.
    pub fn shear_mooshroom(&mut self, id: u64) -> Option<crate::mooshroom::MushroomShearing> {
        let entity = self.mooshroom_mut(id)?;
        let shearing = entity.mooshroom.as_ref()?.shear(&entity.cow)?;
        entity.mooshroom = None;
        Some(shearing)
    }

    /// NoAI suppresses travel in the pinned version but not age/love aiStep.
    /// Isolated cow goals feed navigation and controls before ordered travel.
    pub fn tick(&mut self, world: &mut impl World) {
        self.tick_with_players(world, &[]);
    }

    pub fn tick_with_players(&mut self, world: &mut impl World, players: &[PlayerCandidate]) {
        self.tick_with_players_where(world, players, &|_| true);
    }

    /// Every entity's ID and feet position.
    fn positions(&self) -> impl Iterator<Item = (u64, DVec3)> + '_ {
        self.bats
            .iter()
            .map(|e| (e.id, e.bat.body.position))
            .chain(self.zombies.iter().map(|e| (e.id, e.zombie.body.position)))
            .chain(self.skeletons.iter().map(|e| (e.id, e.skeleton.body.position)))
            .chain(self.creepers.iter().map(|e| (e.id, e.creeper.body.position)))
            .chain(self.spiders.iter().map(|e| (e.id, e.spider.body.position)))
            .chain(self.slimes.iter().map(|e| (e.id, e.slime.body.position)))
            .chain(self.endermen.iter().map(|e| (e.id, e.enderman.body.position)))
            .chain(self.witches.iter().map(|e| (e.id, e.witch.body.position)))
            .chain(self.iron_golems.iter().map(|e| (e.id, e.golem.body.position)))
            .chain(self.wolves.iter().map(|e| (e.id, e.wolf.body.position)))
            .chain(self.arrows.iter().map(|e| (e.id, e.arrow.position)))
            .chain(self.potions.iter().map(|e| (e.id, e.potion.position)))
            .chain(self.villagers.iter().map(|e| (e.id, e.villager.body.position)))
            .chain(self.cows.iter().map(|e| (e.id, e.cow.body.position)))
            .chain(self.sheep.iter().map(|e| (e.id, e.body.position)))
            .chain(self.pigs.iter().map(|e| (e.id, e.pig.body.position)))
            .chain(self.chickens.iter().map(|e| (e.id, e.chicken.body.position)))
    }

    /// The burning mobs (`displayFireAnimation`), for their flames: where
    /// each stood last tick and stands now, and its box's width and height.
    pub fn burning(&self) -> Vec<(DVec3, DVec3, f32, f32)> {
        let mut out = Vec::new();
        let mut add = |previous: DVec3, body: &Body| {
            if body.fire_ticks > 0 {
                out.push((previous, body.position, body.width, body.height));
            }
        };
        self.bats.iter().for_each(|e| add(e.previous_position, &e.bat.body));
        self.zombies.iter().for_each(|e| add(e.previous_position, &e.zombie.body));
        self.skeletons.iter().for_each(|e| add(e.previous_position, &e.skeleton.body));
        self.creepers.iter().filter(|e| !e.creeper.exploded).for_each(|e| add(e.previous_position, &e.creeper.body));
        self.spiders.iter().for_each(|e| add(e.previous_position, &e.spider.body));
        self.slimes.iter().for_each(|e| add(e.previous_position, &e.slime.body));
        self.endermen.iter().for_each(|e| add(e.previous_position, &e.enderman.body));
        self.witches.iter().for_each(|e| add(e.previous_position, &e.witch.body));
        self.iron_golems.iter().for_each(|e| add(e.previous_position, &e.golem.body));
        self.wolves.iter().for_each(|e| add(e.previous_position, &e.wolf.body));
        self.villagers.iter().for_each(|e| add(e.previous_position, &e.villager.body));
        self.cows.iter().for_each(|e| add(e.previous_position, &e.cow.body));
        self.sheep.iter().for_each(|e| add(e.previous_position, &e.body));
        self.pigs.iter().for_each(|e| add(e.previous_position, &e.pig.body));
        self.chickens.iter().for_each(|e| add(e.previous_position, &e.chicken.body));
        out
    }

    /// A copy holding only the entities whose feet position `keep` accepts
    /// (what a client tracking them sees); the others are not copied.
    pub fn clone_where(&self, keep: impl Fn(DVec3) -> bool) -> Self {
        let bats: Vec<BatEntity> = self.bats.iter().filter(|e| keep(e.bat.body.position)).cloned().collect();
        let zombies: Vec<ZombieEntity> = self.zombies.iter().filter(|e| keep(e.zombie.body.position)).cloned().collect();
        let skeletons: Vec<SkeletonEntity> = self.skeletons.iter().filter(|e| keep(e.skeleton.body.position)).cloned().collect();
        let creepers: Vec<CreeperEntity> = self.creepers.iter().filter(|e| keep(e.creeper.body.position)).cloned().collect();
        let spiders: Vec<SpiderEntity> = self.spiders.iter().filter(|e| keep(e.spider.body.position)).cloned().collect();
        let slimes: Vec<SlimeEntity> = self.slimes.iter().filter(|e| keep(e.slime.body.position)).cloned().collect();
        let endermen: Vec<EndermanEntity> = self.endermen.iter().filter(|e| keep(e.enderman.body.position)).cloned().collect();
        let witches: Vec<WitchEntity> = self.witches.iter().filter(|e| keep(e.witch.body.position)).cloned().collect();
        let iron_golems: Vec<IronGolemEntity> = self.iron_golems.iter().filter(|e| keep(e.golem.body.position)).cloned().collect();
        let wolves: Vec<WolfEntity> = self.wolves.iter().filter(|e| keep(e.wolf.body.position)).cloned().collect();
        let arrows: Vec<ArrowEntity> = self.arrows.iter().filter(|e| keep(e.arrow.position)).cloned().collect();
        let potions: Vec<PotionEntity> = self.potions.iter().filter(|e| keep(e.potion.position)).cloned().collect();
        let villagers: Vec<VillagerEntity> = self.villagers.iter().filter(|e| keep(e.villager.body.position)).cloned().collect();
        let cows: Vec<CowEntity> = self.cows.iter().filter(|e| keep(e.cow.body.position)).cloned().collect();
        let sheep: Vec<SheepEntity> = self.sheep.iter().filter(|e| keep(e.body.position)).cloned().collect();
        let pigs: Vec<PigEntity> = self.pigs.iter().filter(|e| keep(e.pig.body.position)).cloned().collect();
        let chickens: Vec<ChickenEntity> = self.chickens.iter().filter(|e| keep(e.chicken.body.position)).cloned().collect();
        let mut copy = Self {
            bats,
            zombies,
            skeletons,
            creepers,
            spiders,
            slimes,
            endermen,
            witches,
            iron_golems,
            wolves,
            player_views: self.player_views.clone(),
            player_vitals: self.player_vitals.clone(),
            arrows,
            potions,
            player_splashes: Vec::new(),
            potion_breaks: Vec::new(),
            villagers,
            cows,
            sheep,
            pigs,
            chickens,
            order: Vec::new(),
            next_id: self.next_id,
            game_time: self.game_time,
            bright_outside: self.bright_outside,
            mob_griefing: self.mob_griefing,
            recipes: self.recipes.clone(),
            mix_random: self.mix_random.clone(),
            projectile_seed_random: self.projectile_seed_random.clone(),
            arrow_shoot_seed: self.arrow_shoot_seed,
            arrow_damage_seed: self.arrow_damage_seed,
            explosions: Vec::new(),
            player_hits: Vec::new(),
            player_fights: HashMap::new(),
            players_pickable: false,
            monsters_burn: false,
            difficulty: 2,
            sounds: Vec::new(),
            mob_drops: self.mob_drops,
            sections: self.sections.clone(),
            pushing_players: self.pushing_players.clone(),
            level_random: self.level_random.clone(),
            day_time: self.day_time,
            seed_uniquifier: self.seed_uniquifier,
            pois: self.pois.clone(),
            summoned_golem_seed: self.summoned_golem_seed,
            summoned_golems: Vec::new(),
            born_seeds: self.born_seeds.clone(),
            born_villagers: Vec::new(),
            born_wolves: Vec::new(),
            trades: self.trades.clone(),
            trade_sequences: self.trade_sequences.clone(),
            max_entity_cramming: self.max_entity_cramming,
            uuids: self.uuids.clone(),
            uuid_salt: self.uuid_salt,
            player_heroes: self.player_heroes.clone(),
            merchant_menus: HashMap::new(),
            trade_experience: Vec::new(),
            player_main_hands: self.player_main_hands.clone(),
            entity_events: Vec::new(),
        };
        let kept: HashSet<u64> = copy.positions().map(|(id, _)| id).collect();
        copy.order = self.order.iter().copied().filter(|key| kept.contains(&key.id())).collect();
        copy.sections.retain_mobs(|id| kept.contains(&id));
        copy
    }

    /// How many entities the world holds.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Every mob with its type, feet, box size and persistence
    /// (`isPersistenceRequired`), for natural spawning's mob caps and
    /// obstruction test.
    pub fn census(&self) -> Vec<CensusEntry> {
        let entry = |kind: &'static str, body: &crate::movement::Body, persistent: bool| CensusEntry {
            kind,
            position: body.position,
            width: body.width,
            height: body.height,
            persistent,
        };
        let mut out = Vec::with_capacity(self.order.len());
        out.extend(self.bats.iter().map(|e| entry("minecraft:bat", &e.bat.body, e.bat.persistence_required)));
        out.extend(self.zombies.iter().map(|e| {
            let kind = e.zombie.kind.type_id();
            entry(kind, &e.zombie.body, e.zombie.persistence_required)
        }));
        out.extend(self.skeletons.iter().map(|e| entry(e.skeleton.kind.type_id(), &e.skeleton.body, e.skeleton.persistence_required)));
        out.extend(self.creepers.iter().map(|e| entry("minecraft:creeper", &e.creeper.body, e.creeper.persistence_required)));
        out.extend(self.spiders.iter().map(|e| entry("minecraft:spider", &e.spider.body, e.spider.persistence_required)));
        out.extend(self.slimes.iter().map(|e| entry("minecraft:slime", &e.slime.body, e.slime.persistence_required)));
        // `requiresCustomPersistence`: carrying a block keeps it.
        out.extend(self.endermen.iter().map(|e| entry("minecraft:enderman", &e.enderman.body, e.enderman.persistence_required || e.carried().is_some())));
        out.extend(self.witches.iter().map(|e| entry("minecraft:witch", &e.witch.body, e.witch.persistence_required)));
        out.extend(self.iron_golems.iter().map(|e| entry("minecraft:iron_golem", &e.golem.body, e.golem.persistence_required)));
        out.extend(self.wolves.iter().map(|e| entry("minecraft:wolf", &e.wolf.body, e.wolf.persistence_required || e.wolf.tame)));
        out.extend(self.villagers.iter().map(|e| entry("minecraft:villager", &e.villager.body, e.villager.persistence_required)));
        out.extend(self.cows.iter().map(|e| {
            let kind = if e.mooshroom.is_some() { "minecraft:mooshroom" } else { "minecraft:cow" };
            entry(kind, &e.cow.body, e.cow.persistence_required)
        }));
        out.extend(self.sheep.iter().map(|e| entry("minecraft:sheep", &e.body, e.sheep.persistence_required)));
        out.extend(self.pigs.iter().map(|e| entry("minecraft:pig", &e.pig.body, e.pig.persistence_required)));
        out.extend(self.chickens.iter().map(|e| entry("minecraft:chicken", &e.chicken.body, e.chicken.persistence_required)));
        out
    }

    /// `Mob.checkDespawn`, which `ServerLevel.tick` runs for every entity
    /// in its tick list (`listed`: loaded, entity-ticking chunks) just
    /// before the entity ticks; only each mob's own random is drawn, so it
    /// runs as a pass before the ticks. In peaceful, monsters go. Otherwise,
    /// against the nearest non-spectator player: a mob that may go
    /// (`removeWhenFarAway`) and is not persistent goes beyond its
    /// category's despawn distance, or one time in 800 once idle for 600
    /// ticks beyond 32 blocks; within 32 blocks its idle time restarts.
    /// Returns how many were removed.
    pub fn check_despawn(&mut self, players: &[DVec3], peaceful: bool, listed: &dyn Fn(DVec3) -> bool) -> usize {
        // Despawn distance, `removeWhenFarAway`, `isAllowedInPeaceful`.
        const MONSTER: (f64, bool, bool) = (128.0, true, false);
        const CREATURE: (f64, bool, bool) = (128.0, false, true);
        const AMBIENT: (f64, bool, bool) = (128.0, true, true);
        const VILLAGER: (f64, bool, bool) = (128.0, false, true);
        let check = |position: DVec3, persistent: bool, rule: (f64, bool, bool), no_action_time: &mut i32, random: &mut LegacyRandom| -> bool {
            if !listed(position) {
                return false;
            }
            let (despawn, removable, peaceful_ok) = rule;
            if peaceful && !peaceful_ok {
                return true;
            }
            let Some(dist) = players.iter().map(|p| p.distance_squared(position)).min_by(f64::total_cmp) else { return false };
            let far = !persistent && dist > despawn * despawn && removable;
            if !persistent && *no_action_time > 600 && random.next_int(800) == 0 && dist > 32.0 * 32.0 && removable {
                return true;
            } else if dist < 32.0 * 32.0 {
                *no_action_time = 0;
            }
            far
        };
        let before = self.order.len();
        self.bats.retain_mut(|e| !check(e.bat.body.position, e.bat.persistence_required, AMBIENT, &mut e.no_action_time, &mut e.random));
        self.zombies.retain_mut(|e| !check(e.zombie.body.position, e.zombie.persistence_required, MONSTER, &mut e.no_action_time, &mut e.random));
        self.skeletons.retain_mut(|e| !check(e.skeleton.body.position, e.skeleton.persistence_required, MONSTER, &mut e.no_action_time, &mut e.random));
        self.creepers.retain_mut(|e| !check(e.creeper.body.position, e.creeper.persistence_required, MONSTER, &mut e.no_action_time, &mut e.random));
        self.spiders.retain_mut(|e| !check(e.spider.body.position, e.spider.persistence_required, MONSTER, &mut e.no_action_time, &mut e.random));
        self.slimes.retain_mut(|e| !check(e.slime.body.position, e.slime.persistence_required, MONSTER, &mut e.no_action_time, &mut e.random));
        self.endermen.retain_mut(|e| {
            let persistent = e.enderman.persistence_required || e.ai.state.enderman.carried.is_some();
            !check(e.enderman.body.position, persistent, MONSTER, &mut e.no_action_time, &mut e.random)
        });
        self.witches.retain_mut(|e| !check(e.witch.body.position, e.witch.persistence_required, MONSTER, &mut e.no_action_time, &mut e.random));
        // `AbstractGolem.removeWhenFarAway` is false, yet the idle roll draws.
        self.iron_golems.retain_mut(|e| !check(e.golem.body.position, e.golem.persistence_required, VILLAGER, &mut e.no_action_time, &mut e.random));
        // `TamableAnimal`: a tame wolf never goes.
        self.wolves.retain_mut(|e| !check(e.wolf.body.position, e.wolf.persistence_required || e.wolf.tame, CREATURE, &mut e.no_action_time, &mut e.random));
        self.villagers.retain_mut(|e| !check(e.villager.body.position, e.villager.persistence_required, VILLAGER, &mut e.no_action_time, &mut e.random));
        self.cows.retain_mut(|e| !check(e.cow.body.position, e.cow.persistence_required, CREATURE, &mut e.no_action_time, &mut e.random));
        self.sheep.retain_mut(|e| !check(e.body.position, e.sheep.persistence_required, CREATURE, &mut e.no_action_time, &mut e.random));
        self.pigs.retain_mut(|e| !check(e.pig.body.position, e.pig.persistence_required, CREATURE, &mut e.no_action_time, &mut e.random));
        self.chickens.retain_mut(|e| !check(e.chicken.body.position, e.chicken.persistence_required, CREATURE, &mut e.no_action_time, &mut e.random));
        let alive: HashSet<u64> = self.positions().map(|(id, _)| id).collect();
        self.order.retain(|key| alive.contains(&key.id()));
        before - self.order.len()
    }

    /// Removes the entities whose feet position `remove` accepts (their
    /// chunk unloaded). Returns how many were removed.
    pub fn remove_where(&mut self, remove: impl Fn(DVec3) -> bool) -> usize {
        let before = self.order.len();
        self.bats.retain(|e| !remove(e.bat.body.position));
        self.zombies.retain(|e| !remove(e.zombie.body.position));
        self.skeletons.retain(|e| !remove(e.skeleton.body.position));
        self.creepers.retain(|e| !remove(e.creeper.body.position));
        self.spiders.retain(|e| !remove(e.spider.body.position));
        self.slimes.retain(|e| !remove(e.slime.body.position));
        self.endermen.retain(|e| !remove(e.enderman.body.position));
        self.witches.retain(|e| !remove(e.witch.body.position));
        self.iron_golems.retain(|e| !remove(e.golem.body.position));
        self.wolves.retain(|e| !remove(e.wolf.body.position));
        self.arrows.retain(|e| !remove(e.arrow.position));
        self.potions.retain(|e| !remove(e.potion.position));
        self.villagers.retain(|e| !remove(e.villager.body.position));
        self.cows.retain(|e| !remove(e.cow.body.position));
        self.sheep.retain(|e| !remove(e.body.position));
        self.pigs.retain(|e| !remove(e.pig.body.position));
        self.chickens.retain(|e| !remove(e.chicken.body.position));
        let alive: HashSet<u64> = self.positions().map(|(id, _)| id).collect();
        self.order.retain(|key| alive.contains(&key.id()));
        before - self.order.len()
    }

    /// `tick_with_players` for the entities whose feet position `ticks`
    /// accepts, as `ServerLevel.tick` skips entities outside the
    /// entity-ticking range. The others keep their state untouched.
    pub fn tick_with_players_where(&mut self, world: &mut impl World, players: &[PlayerCandidate], ticks: &dyn Fn(DVec3) -> bool) {
        // The chunk source ticks the trackers before any entity: each
        // sends what the last tick flagged.
        for body in self.bodies_mut() {
            body.needs_sync = false;
        }
        self.game_time += 1;
        let game_time = self.game_time;
        self.set_pushing_players(players);
        let ticking: HashSet<u64> = self.positions().filter(|&(_, position)| ticks(position)).map(|(id, _)| id).collect();
        // Snapshot membership: newborns join the world immediately but start
        // ticking on the next game tick, in global insertion order.
        let mut order = self.order.clone();
        order.retain(|key| ticking.contains(&key.id()));
        let cow_indices: HashMap<_, _> = self
            .cows
            .iter()
            .enumerate()
            .map(|(index, entity)| (entity.id, index))
            .collect();
        let sheep_indices: HashMap<_, _> = self
            .sheep
            .iter()
            .enumerate()
            .map(|(index, entity)| (entity.id, index))
            .collect();
        let pig_indices: HashMap<_, _> = self
            .pigs
            .iter()
            .enumerate()
            .map(|(index, entity)| (entity.id, index))
            .collect();
        let chicken_indices: HashMap<_, _> = self
            .chickens
            .iter()
            .enumerate()
            .map(|(index, entity)| (entity.id, index))
            .collect();
        for key in order {
            let index = match key {
                EntityKey::Bat(id) => {
                    let entity = self.bat_mut(id).unwrap();
                    entity.previous_position = entity.bat.body.position;
                    entity.tick_count += 1;
                    hazards::burn(entity, &*world, game_time);
                    hazards::suffocate(entity, &*world, 0.45, game_time);
                    let removed = entity.bat.damage.tick();
                    // `tickEffects`, after the hurt and death clocks.
                    for work in entity.effects.tick(false) {
                        match work.resolve(entity.bat.health, 6.0) {
                            Some(EffectWork::Heal(amount)) => heal(&mut entity.bat.health, 6.0, amount),
                            Some(EffectWork::HurtMagic(amount)) => {
                                entity.hurt(amount);
                            }
                            _ => {}
                        }
                    }
                    if entity.bat.health > 0.0 {
                        if (entity.random.next_int(1000) as i32) < entity.ambient_sound_time {
                            entity.ambient_sound_time = -80;
                            if entity.bat.resting {
                                let _ = entity.random.next_int(4);
                            }
                        } else {
                            entity.ambient_sound_time += 1;
                        }
                    }
                    if !removed {
                        entity.bat.body.trim_small_velocity();
                        if !entity.no_ai {
                            if players.iter().any(|player| {
                                player.alive
                                    && !player.spectator
                                    && player.position.distance_squared(entity.bat.body.position)
                                        < 32.0 * 32.0
                            }) {
                                entity.no_action_time = 0;
                            }
                            entity.no_action_time += 1;
                            entity.tick_active(world, players);
                        }
                        let old = entity.previous_position;
                        hazards::blocks_act(entity, &*world, old, game_time);
                        entity.bat.tick_after_living();
                    }
                    continue;
                }
                EntityKey::Zombie(id) => {
                    let villagers: Vec<_> = self
                        .villagers
                        .iter()
                        .map(|entity| VillagerCandidate {
                            id: entity.id,
                            position: entity.villager.body.position,
                            eye_height: entity.villager.eye_height(),
                            width: entity.villager.body.width,
                            height: entity.villager.body.height,
                            alive: entity.villager.health > 0.0,
                        })
                        .collect();
                    let bright_outside = self.bright_outside;
                    let monsters_burn = self.monsters_burn;
                    let (difficulty, mob_griefing) = (self.difficulty, self.mob_griefing);
                    let mobs = self.mob_candidates();
                    // The level's points of interest, lent to its goals.
                    let mut lent_pois = Some(std::mem::take(&mut self.pois));
                    let mut alert = None;
                    let mut pending_mob = None;
                    let (pending_attack, pending_player, conversion_due, stepped) = {
                        let entity = self.zombie_mut(id).unwrap();
                        entity.previous_position = entity.zombie.body.position;
                        entity.tick_count += 1;
                        let sounds = entity.zombie.kind.movement_sounds();
                        base_tick_fluid(&mut entity.zombie.body, world, entity.tick_count == 1, &mut entity.random, &mut entity.voices, sounds);
                        hazards::burn(entity, &*world, game_time);
                        let eye = entity.zombie.eye_height();
                        hazards::suffocate(entity, &*world, eye, game_time);
                        // Undead breathe under water.
                        if entity.zombie.health > 0.0 {
                            let eye = entity.zombie.eye_height();
                            breathe(&mut entity.zombie.body, &*world, eye, true);
                        }
                        let removed = entity.zombie.damage.tick();
                        // `tickEffects`, after the hurt and death clocks.
                        for work in entity.effects.tick(true) {
                            match work.resolve(entity.zombie.health, 20.0) {
                                Some(EffectWork::Heal(amount)) => heal(&mut entity.zombie.health, 20.0, amount),
                                Some(EffectWork::HurtMagic(amount)) => {
                                    entity.hurt(amount);
                                }
                                _ => {}
                            }
                        }
                        if entity.zombie.health > 0.0 {
                            if (entity.random.next_int(1000) as i32) < entity.ambient_sound_time {
                                entity.ambient_sound_time = -80;
                                let (a, b) = (entity.random.next_float(), entity.random.next_float());
                                let base = if entity.zombie.baby { entity.zombie.kind.baby_voice() } else { 1.0 };
                                let voice = Voice::Ambient((a - b) * 0.2 + base);
                            entity.voices.push((voice, entity.position()));
                            } else {
                                entity.ambient_sound_time += 1;
                            }
                        }
                        if !removed {
                            // `LivingEntity.aiStep` while dying: no input, still moving.
                            if entity.zombie.health <= 0.0 && !entity.no_ai {
                                let sounds = entity.zombie.kind.movement_sounds();
                                let _ = dying_travel(&mut entity.zombie.body, &*world, entity.yaw, entity.tick_count, &mut entity.random, &mut entity.voices, sounds, Some(true));
                            } else {
                                entity.zombie.body.trim_small_velocity();
                            }
                            if entity.no_ai {
                                monster_idle_without_ai(world, players, entity.zombie.body.position, entity.zombie.eye_height(), &mut entity.no_action_time);
                            }
                            if entity.zombie.health > 0.0 && !entity.no_ai && entity.ai.is_some() {
                                let mob_villagers: Vec<crate::monster_ai::MobCandidate> = villagers
                                    .iter()
                                    .map(|v| crate::monster_ai::MobCandidate { id: v.id, position: v.position, eye_height: v.eye_height, width: v.width, height: v.height, alive: v.alive, kind: "minecraft:villager" })
                                    .collect();
                                // `Zombie.createAttributes`: 0.23 speed, half again for babies.
                                let speed = entity.effects.movement_speed(f64::from(0.23_f32) * if entity.zombie.baby { 1.5 } else { 1.0 });
                                let step = MonsterStep { players, villagers: &mob_villagers, game_time, difficulty, movement_speed: speed, sounds: entity.zombie.kind.movement_sounds() };
                                let ai = entity.ai.as_deref_mut().unwrap();
                                ai.state.mob_griefing = mob_griefing;
                                ai.state.bright_outside = bright_outside;
                                ai.state.mobs = mobs;
                                ai.state.pois = lent_pois.take();
                                ai.state.can_break_doors = entity.zombie.can_break_doors;
                                // The goals hit before the zombie moves.
                                let position = entity.zombie.body.position;
                                let landed = monster_ai_step(ai, &mut entity.zombie.body, entity.zombie.health, &mut entity.random, &mut entity.voices, &mut entity.no_action_time, entity.previous_position, entity.tick_count, entity.id, world, &step);
                                lent_pois = ai.state.pois.take();
                                // The renderer, the census and the gates read the zombie's own fields.
                                entity.yaw = ai.yaw;
                                entity.speed = ai.speed;
                                entity.forward = ai.forward;
                                entity.sideways = ai.sideways;
                                entity.body_rotation = ai.body_rotation.clone();
                                entity.look_control = ai.state.look_control.clone();
                                entity.navigation = ai.state.navigation.clone();
                                entity.move_control = ai.move_control.clone();
                                entity.aggressive = ai.state.melee.aggressive;
                                entity.attack_goal_running = ai.running_goals().contains(&"ZombieAttackGoal");
                                entity.target_player_id = match ai.state.target() {
                                    Some(crate::monster_ai::TargetInfo { target: crate::monster_ai::Target::Player(id), .. }) => Some(id),
                                    _ => None,
                                };
                                entity.target_villager_id = match ai.state.target() {
                                    Some(crate::monster_ai::TargetInfo { target: crate::monster_ai::Target::Villager(id), .. }) => Some(id),
                                    _ => None,
                                };
                                alert = ai.state.alert.take();
                                // `Mob.doHurtTarget`: 3 attack damage; a burning bare-handed
                                // zombie may set the target alight (the roll is drawn; the fire
                                // is not passed on yet).
                                if let Some(target) = ai.state.attack.take() {
                                    match target {
                                        crate::monster_ai::Target::Player(id) => entity.pending_attack_player = Some((id, position)),
                                        crate::monster_ai::Target::Villager(id) => entity.pending_attack_villager = Some((id, position)),
                                        crate::monster_ai::Target::Mob(id) => pending_mob = Some((id, position)),
                                    }
                                    if entity.zombie.body.fire_ticks > 0 {
                                        let _ = entity.random.next_float();
                                    }
                                }
                                if let Some(damage) = landed.and_then(|fallen| fall_damage(&entity.zombie.body, &*world, fallen, true, &mut entity.voices)) {
                                    entity.hurt(damage);
                                }
                            } else if entity.zombie.health > 0.0 && !entity.no_ai {
                                if entity.attack_only {
                                    entity.tick_pursuit(
                                        world,
                                        players,
                                        &villagers,
                                        game_time,
                                        bright_outside,
                                    );
                                } else {
                                    entity.travel_controlled(world, false);
                                }
                            }
                            // `applyEffectsFromBlocks` after the move; pushing
                            // and `Mob.aiStep`'s sun follow.
                            let old = entity.previous_position;
                            hazards::blocks_act(entity, &*world, old, game_time);
                        }
                        let eye_in_water = FluidFrame::eye_in_water(
                            world,
                            entity.zombie.body.position,
                            entity.zombie.eye_height(),
                        );
                        entity.underwater_last_tick = eye_in_water;
                        let conversion_due =
                            entity.zombie.tick_drowning(eye_in_water, entity.no_ai);
                        (entity.pending_attack_villager.take(), entity.pending_attack_player.take(), conversion_due, !removed)
                    };
                    if let Some((player_id, attacker)) = pending_player {
                        // `Zombie.createAttributes`: 3 attack damage; an
                        // empty-handed husk's bite adds 140 ticks of hunger
                        // per whole step of the regional difficulty where it
                        // stands (`Husk.doHurtTarget`).
                        let difficulty = self.difficulty;
                        let hunger_ticks = self
                            .zombies
                            .iter()
                            .find(|e| e.id == id && e.zombie.kind == ZombieKind::Husk && e.zombie.main_hand.is_none())
                            .map_or(0, |e| {
                                let p = e.zombie.body.position;
                                let (clock, inhabited, moon) = world.difficulty_inputs((p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32));
                                140 * regional_difficulty(difficulty, clock, inhabited, moon) as i32
                            });
                        self.player_hits.push(PlayerHit { player_id, damage: 3.0, kind: PlayerHitKind::Melee { attacker, hunger_ticks, lift: 0.0 }, source: Some(id) });
                    }
                    if let Some((victim_id, attacker_position)) = pending_attack {
                        if let Some(victim) = self.villager_mut(victim_id) {
                            let result = victim.hurt_from(3.0, "minecraft:mob_attack", Some(id), game_time);
                            if result.applied {
                                victim.knockback_from(attacker_position);
                                self.villager_hurt_by(victim_id, id);
                            }
                        }
                    }
                    self.pois = lent_pois.expect("the points of interest come back");
                    if let Some((victim, attacker_position)) = pending_mob {
                        self.mob_hits_mob(id, victim, 3.0, attacker_position, 0.0);
                    }
                    if stepped {
                        self.push_entities(EntityKey::Zombie(id), &*world, game_time, ticks);
                        let entity = self.zombie_mut(id).unwrap();
                        if entity.zombie.health > 0.0 {
                            let eye = entity.zombie.eye_height();
                            let burns = monsters_burn && entity.zombie.kind.burns_in_daylight();
                            burn_undead(world, &mut entity.random, &mut entity.zombie.body, eye, burns, entity.zombie.head_item);
                        }
                    }
                    if let Some(attacker) = alert {
                        self.alert_zombies(id, attacker);
                    }
                    if conversion_due {
                        self.convert_drowning_zombie(id);
                    }
                    continue;
                }
                EntityKey::Skeleton(id) => {
                    let index = self
                        .skeletons
                        .iter()
                        .position(|entity| entity.id == id)
                        .unwrap();
                    let arrow_shoot_seed = self.arrow_shoot_seed;
                    let arrow_damage_seed = self.arrow_damage_seed;
                    let monsters_burn = self.monsters_burn;
                    let (difficulty, bright_outside) = (self.difficulty, self.bright_outside);
                    let mobs = self.mob_candidates();
                    let (fired, stepped) = {
                        let entity = &mut self.skeletons[index];
                        if let Some(ai) = entity.ai.as_deref_mut() {
                            ai.state.mobs = mobs;
                        }
                        entity.previous_position = entity.skeleton.body.position;
                        entity.tick_count += 1;
                        let sounds = entity.skeleton.kind.movement_sounds();
                        base_tick_fluid(&mut entity.skeleton.body, world, entity.tick_count == 1, &mut entity.random, &mut entity.voices, sounds);
                        hazards::burn(entity, &*world, game_time);
                        let eye = entity.skeleton.eye_height();
                        hazards::suffocate(entity, &*world, eye, game_time);
                        // Undead breathe under water.
                        if entity.skeleton.health > 0.0 {
                            let eye = entity.skeleton.eye_height();
                            breathe(&mut entity.skeleton.body, &*world, eye, true);
                        }
                        let removed = entity.skeleton.damage.tick();
                        // `tickEffects`, after the hurt and death clocks.
                        for work in entity.effects.tick(true) {
                            match work.resolve(entity.skeleton.health, entity.skeleton.kind.max_health()) {
                                Some(EffectWork::Heal(amount)) => heal(&mut entity.skeleton.health, entity.skeleton.kind.max_health(), amount),
                                Some(EffectWork::HurtMagic(amount)) => {
                                    entity.hurt(amount);
                                }
                                _ => {}
                            }
                        }
                        if entity.skeleton.health > 0.0 {
                            if (entity.random.next_int(1000) as i32) < entity.ambient_sound_time {
                                entity.ambient_sound_time = -80;
                                let voice = Voice::Ambient(voice_pitch(&mut entity.random, false));
                            entity.voices.push((voice, entity.position()));
                            } else {
                                entity.ambient_sound_time += 1;
                            }
                        }
                        if removed {
                            (None, false)
                        } else if entity.skeleton.health <= 0.0 && !entity.no_ai {
                            let sounds = entity.skeleton.kind.movement_sounds();
                            let _ = dying_travel(&mut entity.skeleton.body, &*world, entity.yaw, entity.tick_count, &mut entity.random, &mut entity.voices, sounds, Some(true));
                            let old = entity.previous_position;
                            hazards::blocks_act(entity, &*world, old, game_time);
                            (None, true)
                        } else {
                            entity.skeleton.body.trim_small_velocity();
                            let fired = if entity.skeleton.health > 0.0 && !entity.no_ai && entity.ai.is_some() {
                                entity.tick_monster_ai(world, players, game_time, difficulty, bright_outside, arrow_shoot_seed, arrow_damage_seed, &mut self.projectile_seed_random)
                            } else if entity.skeleton.health > 0.0 && !entity.no_ai {
                                entity.tick_bow(
                                    world,
                                    players,
                                    arrow_shoot_seed,
                                    arrow_damage_seed,
                                    &mut self.projectile_seed_random,
                                )
                            } else {
                                if entity.no_ai {
                                    monster_idle_without_ai(world, players, entity.skeleton.body.position, entity.skeleton.eye_height(), &mut entity.no_action_time);
                                }
                                None
                            };
                            let old = entity.previous_position;
                            hazards::blocks_act(entity, &*world, old, game_time);
                            (fired, true)
                        }
                    };
                    if stepped {
                        // Pushing, then `Mob.aiStep`'s sun.
                        self.push_entities(EntityKey::Skeleton(id), &*world, game_time, ticks);
                        let entity = &mut self.skeletons[index];
                        if entity.skeleton.health > 0.0 {
                            let eye = entity.skeleton.eye_height();
                            let burns = monsters_burn && entity.skeleton.kind.burns_in_daylight();
                            burn_undead(world, &mut entity.random, &mut entity.skeleton.body, eye, burns, entity.skeleton.head_item);
                        }
                    }
                    if let Some(arrow) = fired {
                        // `getArrow`: a stray's, bogged's or parched's arrow
                        // carries its effect.
                        let effect = self.skeletons.iter().find(|e| e.id == id).and_then(|e| e.skeleton.kind.arrow_effect());
                        let arrow_id = self.spawn_arrow(id, arrow);
                        if let Some(entity) = self.arrows.iter_mut().find(|e| e.id == arrow_id) {
                            entity.effect = effect;
                        }
                    }
                    continue;
                }
                EntityKey::Creeper(id) => {
                    let difficulty = self.difficulty;
                    let mut stepped = false;
                    let mobs = self.mob_candidates();
                    let explosion = {
                        let entity = self.creeper_mut(id).unwrap();
                        entity.ai.state.mobs = mobs;
                        entity.previous_position = entity.creeper.body.position;
                        entity.tick_count += 1;
                        // `Creeper.tick`: the fuse's first step hisses.
                        let c = &entity.creeper;
                        if c.health > 0.0 && !c.exploded && c.swell == 0 && (c.swell_dir > 0 || c.ignited) {
                            let at = entity.creeper.body.position;
                            entity.voices.push((Voice::Event("entity.creeper.primed", 1.0, 0.5), at));
                        }
                        let explosion = entity.creeper.tick_fuse();
                        if explosion.is_none() {
                            base_tick_fluid(&mut entity.creeper.body, world, entity.tick_count == 1, &mut entity.random, &mut entity.voices, CREEPER_SOUNDS);
                            hazards::burn(entity, &*world, game_time);
                            let eye = entity.creeper.body.height * 0.85;
                            hazards::suffocate(entity, &*world, eye, game_time);
                            if entity.creeper.health > 0.0 && breathe(&mut entity.creeper.body, &*world, eye, false) {
                                entity.hurt(2.0);
                            }
                            let removed = entity.creeper.damage.tick();
                            // `LivingEntity.baseTick` forgets an attacker after 100 ticks.
                            if entity.ai.state.hurt_by.is_some_and(|(_, when)| entity.tick_count - when > 100) {
                                entity.ai.state.hurt_by = None;
                            }
                            // `tickEffects`, after the hurt and death clocks.
                            for work in entity.effects.tick(false) {
                                match work.resolve(entity.creeper.health, 20.0) {
                                    Some(EffectWork::Heal(amount)) => heal(&mut entity.creeper.health, 20.0, amount),
                                    Some(EffectWork::HurtMagic(amount)) => {
                                        entity.hurt(amount);
                                    }
                                    _ => {}
                                }
                            }
                            if entity.creeper.health > 0.0 {
                                // `Creeper` has no ambient sound, so `makeSound`
                                // draws no pitch.
                                if (entity.random.next_int(1000) as i32) < entity.ambient_sound_time {
                                    entity.ambient_sound_time = -80;
                                } else {
                                    entity.ambient_sound_time += 1;
                                }
                            }
                            if !removed {
                                if entity.creeper.health <= 0.0 && !entity.no_ai {
                                    let _ = dying_travel(&mut entity.creeper.body, &*world, entity.ai.yaw, entity.tick_count, &mut entity.random, &mut entity.voices, CREEPER_SOUNDS, Some(true));
                                }
                                if entity.creeper.health > 0.0 && !entity.no_ai {
                                    entity.tick_ai(world, players, game_time, difficulty);
                                } else if entity.no_ai {
                                    entity.creeper.body.trim_small_velocity();
                                    if entity.no_ai {
                                        let eye = entity.creeper.body.height * 0.85;
                                        monster_idle_without_ai(world, players, entity.creeper.body.position, eye, &mut entity.no_action_time);
                                    }
                                }
                                let old = entity.previous_position;
                                hazards::blocks_act(entity, &*world, old, game_time);
                                stepped = true;
                            }
                        }
                        explosion
                    };
                    if stepped {
                        self.push_entities(EntityKey::Creeper(id), &*world, game_time, ticks);
                    }
                    if let Some(explosion) = explosion {
                        self.apply_explosion(&*world, id, explosion, players);
                        self.explosions.push(explosion);
                    }
                    continue;
                }
                EntityKey::Spider(id) => {
                    let difficulty = self.difficulty;
                    let mobs = self.mob_candidates();
                    let (hit, stepped) = {
                        let entity = self.spider_mut(id).unwrap();
                        entity.ai.state.mobs = mobs;
                        entity.previous_position = entity.spider.body.position;
                        entity.tick_count += 1;
                        base_tick_fluid(&mut entity.spider.body, world, entity.tick_count == 1, &mut entity.random, &mut entity.voices, SPIDER_SOUNDS);
                        hazards::burn(entity, &*world, game_time);
                        let eye = entity.spider.eye_height();
                        hazards::suffocate(entity, &*world, eye, game_time);
                        if entity.spider.health > 0.0 && breathe(&mut entity.spider.body, &*world, eye, false) {
                            entity.hurt(2.0);
                        }
                        let removed = entity.spider.damage.tick();
                        // `LivingEntity.baseTick` forgets an attacker after 100 ticks.
                        if entity.ai.state.hurt_by.is_some_and(|(_, when)| entity.tick_count - when > 100) {
                            entity.ai.state.hurt_by = None;
                        }
                        // `tickEffects`, after the hurt and death clocks.
                        for work in entity.effects.tick(false) {
                            match work.resolve(entity.spider.health, crate::spider::MAX_HEALTH) {
                                Some(EffectWork::Heal(amount)) => heal(&mut entity.spider.health, crate::spider::MAX_HEALTH, amount),
                                Some(EffectWork::HurtMagic(amount)) => {
                                    entity.hurt(amount);
                                }
                                _ => {}
                            }
                        }
                        if entity.spider.health > 0.0 {
                            if (entity.random.next_int(1000) as i32) < entity.ambient_sound_time {
                                entity.ambient_sound_time = -80;
                                let voice = Voice::Ambient(voice_pitch(&mut entity.random, false));
                                entity.voices.push((voice, entity.position()));
                            } else {
                                entity.ambient_sound_time += 1;
                            }
                        }
                        let mut hit = None;
                        if !removed {
                            if entity.spider.health <= 0.0 && !entity.no_ai {
                                let _ = dying_travel(&mut entity.spider.body, &*world, entity.spider.yaw, entity.tick_count, &mut entity.random, &mut entity.voices, SPIDER_SOUNDS, Some(true));
                            }
                            if entity.spider.health > 0.0 && !entity.no_ai {
                                hit = entity.tick_ai(world, players, game_time, difficulty);
                            } else if entity.no_ai {
                                entity.spider.body.trim_small_velocity();
                                if entity.no_ai {
                                    monster_idle_without_ai(world, players, entity.spider.body.position, entity.spider.eye_height(), &mut entity.no_action_time);
                                }
                            }
                            let old = entity.previous_position;
                            hazards::blocks_act(entity, &*world, old, game_time);
                        }
                        (hit, !removed)
                    };
                    if stepped {
                        self.push_entities(EntityKey::Spider(id), &*world, game_time, ticks);
                    }
                    // `Spider.tick`: after moving (and pushing), it climbs if
                    // it met a wall.
                    let entity = self.spider_mut(id).unwrap();
                    entity.spider.body.climbing = entity.spider.body.horizontal_collision;
                    if let Some((crate::monster_ai::Target::Player(player_id), attacker)) = hit {
                        self.player_hits.push(PlayerHit { player_id, damage: crate::spider::ATTACK_DAMAGE, kind: PlayerHitKind::Melee { attacker, hunger_ticks: 0, lift: 0.0 }, source: Some(id) });
                    }
                    // A bite on a mob (an iron golem, or whatever hurt it).
                    if let Some((crate::monster_ai::Target::Mob(victim), attacker)) = hit {
                        self.mob_hits_mob(id, victim, crate::spider::ATTACK_DAMAGE, attacker, 0.0);
                    }
                    continue;
                }
                EntityKey::Slime(id) => {
                    self.tick_slime(id, world, players, ticks);
                    continue;
                }
                EntityKey::Enderman(id) => {
                    self.tick_enderman(id, world, players, ticks);
                    continue;
                }
                EntityKey::Witch(id) => {
                    self.tick_witch(id, world, players, ticks);
                    continue;
                }
                EntityKey::IronGolem(id) => {
                    self.tick_iron_golem(id, world, players, ticks);
                    continue;
                }
                EntityKey::Wolf(id) => {
                    self.tick_wolf(id, world, players, ticks);
                    continue;
                }
                EntityKey::Potion(id) => {
                    self.tick_potion(id, world, players);
                    continue;
                }
                EntityKey::Arrow(id) => {
                    let owner_id = self
                        .arrows
                        .iter()
                        .find(|entity| entity.id == id)
                        .unwrap()
                        .owner_id;
                    let targets = self.arrow_targets(owner_id, players);
                    let hit = self
                        .arrows
                        .iter_mut()
                        .find(|entity| entity.id == id)
                        .and_then(|entity| entity.arrow.tick_world(world, &targets));
                    if let Some(hit) = hit {
                        let accepted = if hit.target_id >= PLAYER_TARGET {
                            // `Player.hurtServer` turns arrows away from
                            // players who cannot be hurt (creative).
                            let player = players.iter().find(|p| p.id == hit.target_id - PLAYER_TARGET);
                            let accepted = player.is_some_and(|p| p.attackable);
                            if accepted {
                                let effect = self.arrows.iter().find(|entity| entity.id == id).and_then(|entity| entity.effect);
                                self.player_hits.push(PlayerHit {
                                    player_id: hit.target_id - PLAYER_TARGET,
                                    damage: hit.damage,
                                    kind: PlayerHitKind::Arrow { velocity: hit.velocity, effect },
                                    source: Some(owner_id),
                                });
                            }
                            accepted
                        } else {
                            // `arrow(this, owner != null ? owner : this)`.
                            let attacker = self.entity_type(owner_id).unwrap_or("minecraft:arrow");
                            let target = hit.target_id;
                            let result = self.apply_arrow_hit(hit, owner_id);
                            if let Some(result) = result {
                                self.credit(target, result, attacker, true);
                            }
                            result.is_some_and(|r| r.applied)
                        };
                        self.arrows
                            .iter_mut()
                            .find(|entity| entity.id == id)
                            .unwrap()
                            .arrow
                            .resolve_entity_hit(accepted);
                    }
                    // `entity.arrow.hit`, from the arrow (`SoundSource.NEUTRAL`).
                    if let Some((position, pitch)) = self.arrows.iter_mut().find(|entity| entity.id == id).and_then(|entity| entity.arrow.take_hit_sound()) {
                        self.sounds.push(MobSound { event: "entity.arrow.hit".to_owned(), position, volume: 1.0, pitch, category: "friendly_volume" });
                    }
                    continue;
                }
                EntityKey::Villager(id) => {
                    self.tick_villager(id, world, players, ticks);
                    continue;
                }
                EntityKey::Cow(id) => cow_indices[&id],
                EntityKey::Chicken(id) => {
                    let index = chicken_indices[&id];
                    let candidates: Vec<_> = self
                        .chickens
                        .iter()
                        .map(|entity| CowCandidate {
                            id: entity.id,
                            position: entity.chicken.body.position,
                            width: entity.chicken.body.width,
                            height: entity.chicken.body.height,
                            age: entity.chicken.age.ticks,
                            alive: entity.chicken.health > 0.0,
                            in_love: entity.chicken.in_love,
                            panicking: entity.panic.running,
                        })
                        .collect();
                    let mut breeding_partner = None;
                    let entity = &mut self.chickens[index];
                    entity.tick_count += 1;
                    entity.update_fluid(world);
                    hazards::burn(entity, &*world, game_time);
                    let eye = if entity.chicken.age.baby() { 0.28125 } else { 0.644 };
                    hazards::suffocate(entity, &*world, eye, game_time);
                    let water_breathing = entity.effects.has(MobEffect::WaterBreathing);
                    if entity.chicken.health > 0.0 && breathe(&mut entity.chicken.body, &*world, eye, water_breathing) {
                        entity.hurt_with_source(2.0, DamageSourceKind::Generic);
                    }
                    let removed = entity.chicken.damage.tick();
                    // `tickEffects`, after the hurt and death clocks.
                    for work in entity.effects.tick(false) {
                        match work.resolve(entity.chicken.health, 4.0) {
                            Some(EffectWork::Heal(amount)) => heal(&mut entity.chicken.health, 4.0, amount),
                            Some(EffectWork::HurtMagic(amount)) => {
                                entity.hurt_with_source(amount, DamageSourceKind::Magic);
                            }
                            _ => {}
                        }
                    }
                    if entity.chicken.health > 0.0 {
                        if (entity.random.next_int(1000) as i32) < entity.ambient_sound_time {
                            entity.ambient_sound_time = -120;
                            let voice = Voice::Ambient(voice_pitch(&mut entity.random, entity.chicken.age.baby()));
                            entity.voices.push((voice, entity.position()));
                        } else {
                            entity.ambient_sound_time += 1;
                        }
                    }
                    if removed {
                        continue;
                    }
                    if entity.chicken.health <= 0.0 {
                        if !entity.no_ai {
                            let (yaw, baby) = (entity.chicken.yaw, entity.chicken.age.baby());
                            let _ = dying_travel(&mut entity.chicken.body, &*world, yaw, entity.tick_count, &mut entity.random, &mut entity.voices, chicken_sounds(baby), None);
                            entity.chicken.ai_step(&mut entity.random);
                        }
                        let old = entity.previous_position;
                        hazards::blocks_act(entity, &*world, old, game_time);
                        self.push_entities(EntityKey::Chicken(id), &*world, game_time, ticks);
                        continue;
                    }
                    if entity.no_ai {
                        // `LivingEntity.aiStep` still settles small motion.
                        entity.chicken.body.trim_small_velocity();
                    }
                    if !entity.no_ai {
                        entity.previous_position = entity.chicken.body.position;
                        entity.previous_yaw = entity.chicken.yaw;
                        entity.chicken.body.trim_small_velocity();
                        if players.iter().any(|player| {
                            player.alive
                                && !player.spectator
                                && player
                                    .position
                                    .distance_squared(entity.chicken.body.position)
                                    < 32.0 * 32.0
                        }) {
                            entity.no_action_time = 0;
                        }
                        entity.no_action_time += 1;
                        let full_goal_tick = entity.tick_count <= 1
                            || (entity.tick_count + entity.id as i32) % 2 == 0;
                        let last_damage_panic = DamageSourceKind::panics(entity.last_damage_source)
                            && entity.tick_count - entity.last_damage_tick <= 40;
                        let on_fire = entity.chicken.body.fire_ticks > 0;
                        let mut context = ChickenGoalContext {
                            child: candidates[index],
                            candidates,
                            follow_parent: std::mem::take(&mut entity.follow_parent),
                            breed: std::mem::take(&mut entity.breed),
                            panic: std::mem::take(&mut entity.panic),
                            stroll: std::mem::take(&mut entity.stroll),
                            last_damage_panic,
                            on_fire,
                            no_action_time: entity.no_action_time,
                            navigation_done: entity.chicken.navigation.is_done(),
                            tempt: std::mem::take(&mut entity.tempt),
                            random_look: std::mem::take(&mut entity.random_look),
                            look_at_player: std::mem::take(&mut entity.look_at_player),
                            players: players.to_vec(),
                            random: std::mem::take(&mut entity.random),
                            fluid: entity.fluid,
                            walk: entity.chicken.walk_profile(),
                            effects: Vec::new(),
                        };
                        if full_goal_tick {
                            entity.goals.tick(&mut context, &*world);
                        } else {
                            entity.goals.tick_running(&mut context, false);
                        }
                        entity.follow_parent = context.follow_parent;
                        entity.breed = context.breed;
                        entity.panic = context.panic;
                        entity.stroll = context.stroll;
                        entity.tempt = context.tempt;
                        entity.random_look = context.random_look;
                        entity.look_at_player = context.look_at_player;
                        entity.random = context.random;
                        let mut jump_queued = false;
                        for effect in context.effects {
                            match effect {
                                ChickenGoalEffect::Navigate { target, speed } => {
                                    let fluid = entity.fluid;
                                    let _ = entity.chicken.navigate_to(&*world, fluid, target, speed);
                                }
                                ChickenGoalEffect::NavigateToEntity { target, speed } => {
                                    let fluid = entity.fluid;
                                    let _ = entity.chicken.navigate_to_entity(&*world, fluid, target, speed);
                                }
                                ChickenGoalEffect::LookAt {
                                    target,
                                    y_max,
                                    x_max,
                                } => {
                                    entity
                                        .look_control
                                        .set_look_at_with_limits(target, y_max, x_max);
                                }
                                ChickenGoalEffect::StopNavigation => {
                                    entity.chicken.navigation.stop()
                                }
                                ChickenGoalEffect::Jump => jump_queued = true,
                                ChickenGoalEffect::Breed { partner_id } => {
                                    breeding_partner = Some(partner_id)
                                }
                            }
                        }
                        let chicken = &mut entity.chicken;
                        let position = chicken.body.position;
                        let (can_update, surface) = crate::navigation::ground_view(world, &chicken.body, entity.fluid, true);
                        if let Some((target, speed)) = chicken.navigation.tick_in(
                            world,
                            position,
                            can_update,
                            surface,
                            chicken.body.width,
                            chicken.speed,
                        ) {
                            chicken.move_control.set_wanted_position(target, speed);
                        }
                        let obstacle_top = crate::control::obstacle_top(world, position);
                        let control = chicken.move_control.tick(
                            position,
                            chicken.body.on_ground,
                            chicken.yaw,
                            chicken.speed,
                            chicken.forward,
                            chicken.sideways,
                            entity.effects.movement_speed(0.25),
                            chicken.body.width,
                            chicken.body.step_height,
                            obstacle_top,
                            |_, _| true,
                        );
                        chicken.yaw = control.yaw;
                        chicken.speed = control.speed;
                        chicken.forward = control.forward;
                        chicken.sideways = control.sideways;
                        entity.jumping = jump_queued || control.jump;
                        // `LivingEntity.aiStep` after the AI: a jump swims up in
                        // liquid, or leaves the ground at most every ten ticks
                        // (`getFluidJumpThreshold`: 0 below an eye height of 0.4).
                        if entity.no_jump_delay > 0 {
                            entity.no_jump_delay -= 1;
                        }
                        let jump_threshold = if chicken.age.baby() { 0.0 } else { 0.4 };
                        chicken.body.living_jump(&*world, entity.fluid, entity.jumping, &mut entity.no_jump_delay, jump_threshold);
                        let eye_height = if chicken.age.baby() { 0.28125 } else { 0.644 };
                        entity.look_control.tick(
                            chicken.body.position,
                            eye_height,
                            entity.body_rotation.body_yaw,
                            !chicken.navigation.is_done(),
                        );
                        let input =
                            DVec3::new(chicken.sideways as f64, 0.0, chicken.forward as f64);
                        let was_water = entity.fluid.in_water();
                        if was_water {
                            chicken.body.travel_water(world, input, chicken.yaw);
                        } else if entity.fluid.in_lava() {
                            chicken.body.travel_lava(
                                world,
                                input,
                                chicken.yaw,
                                entity.fluid.lava_height,
                            );
                        } else {
                            chicken
                                .body
                                .travel_air(world, input, chicken.speed, chicken.yaw);
                        }
                        play_movement(&mut chicken.body, entity.tick_count, &mut entity.random, &mut entity.voices, chicken_sounds(chicken.age.baby()));
                        entity.body_rotation.tick(
                            chicken.yaw,
                            &mut entity.look_control,
                            entity.previous_position,
                            chicken.body.position,
                        );
                        if !was_water {
                            entity.fluid = FluidFrame::sample(world, chicken.body.position, chicken.body.width, chicken.body.height);
                            entity.was_touching_water = chicken.body.touching_water;
                        }
                    }
                    {
                        // `applyEffectsFromBlocks` after the move.
                        let entity = &mut self.chickens[index];
                        let old = entity.previous_position;
                        hazards::blocks_act(entity, &*world, old, game_time);
                    }
                    self.push_entities(EntityKey::Chicken(id), &*world, game_time, ticks);
                    if let Some(partner_id) = breeding_partner {
                        if let Some(partner_index) =
                            self.chickens.iter().position(|e| e.id == partner_id)
                        {
                            let first_variant = self.chickens[index].chicken.variant;
                            let second_variant = self.chickens[partner_index].chicken.variant;
                            let first_chosen = self.chickens[index].random.next_boolean();
                            let position = self.chickens[index].chicken.body.position;
                            self.chickens[index].chicken.age.set(6000);
                            self.chickens[index].chicken.in_love = 0;
                            self.chickens[partner_index].chicken.age.set(6000);
                            self.chickens[partner_index].chicken.in_love = 0;
                            let mut offspring = Chicken::new(position);
                            offspring.age.set(Age::BABY_START);
                            offspring.variant = if first_chosen {
                                first_variant
                            } else {
                                second_variant
                            };
                            self.spawn_chicken(offspring, false);
                        }
                    }
                    let entity = &mut self.chickens[index];
                    let was_baby = entity.chicken.age.baby();
                    entity.chicken.age.tick(true);
                    if was_baby != entity.chicken.age.baby() {
                        entity.chicken.sync_dimensions();
                    }
                    if entity.chicken.age.ticks != 0 {
                        entity.chicken.in_love = 0;
                    } else if entity.chicken.in_love > 0 {
                        entity.chicken.in_love -= 1;
                        if entity.chicken.in_love % 10 == 0 {
                            for _ in 0..3 {
                                let _ = entity.random.next_gaussian();
                            }
                            for _ in 0..3 {
                                let _ = entity.random.next_double();
                            }
                        }
                    }
                    if entity.chicken.ai_step(&mut entity.random) {
                        entity.eggs_laid += 1;
                    }
                    continue;
                }
                EntityKey::Pig(id) => {
                    let index = pig_indices[&id];
                    let candidates: Vec<_> = self
                        .pigs
                        .iter()
                        .map(|entity| CowCandidate {
                            id: entity.id,
                            position: entity.pig.body.position,
                            width: entity.pig.body.width,
                            height: entity.pig.body.height,
                            age: entity.pig.age.ticks,
                            alive: entity.pig.health > 0.0,
                            in_love: entity.pig.in_love,
                            panicking: entity.panic.running,
                        })
                        .collect();
                    let mut breeding_partner = None;
                    let entity = &mut self.pigs[index];
                    entity.tick_count += 1;
                    entity.update_fluid(world);
                    hazards::burn(entity, &*world, game_time);
                    let eye = if entity.pig.age.baby() { 0.3825 } else { 0.765 };
                    hazards::suffocate(entity, &*world, eye, game_time);
                    let water_breathing = entity.effects.has(MobEffect::WaterBreathing);
                    if entity.pig.health > 0.0 && breathe(&mut entity.pig.body, &*world, eye, water_breathing) {
                        entity.hurt(2.0, DamageSourceKind::Generic);
                    }
                    let removed = entity.pig.damage.tick();
                    // `tickEffects`, after the hurt and death clocks.
                    for work in entity.effects.tick(false) {
                        match work.resolve(entity.pig.health, 10.0) {
                            Some(EffectWork::Heal(amount)) => heal(&mut entity.pig.health, 10.0, amount),
                            Some(EffectWork::HurtMagic(amount)) => {
                                entity.hurt(amount, DamageSourceKind::Magic);
                            }
                            _ => {}
                        }
                    }
                    if entity.pig.health > 0.0 {
                        if (entity.random.next_int(1000) as i32) < entity.ambient_sound_time {
                            entity.ambient_sound_time = -120;
                            let voice = Voice::Ambient(voice_pitch(&mut entity.random, entity.pig.age.baby()));
                            entity.voices.push((voice, entity.position()));
                        } else {
                            entity.ambient_sound_time += 1;
                        }
                    }
                    if removed {
                        continue;
                    }
                    if entity.pig.health <= 0.0 {
                        if !entity.no_ai {
                            let (yaw, baby) = (entity.pig.yaw, entity.pig.age.baby());
                            dying_travel(&mut entity.pig.body, &*world, yaw, entity.tick_count, &mut entity.random, &mut entity.voices, pig_sounds(baby), Some(false));
                        }
                        let old = entity.previous_position;
                        hazards::blocks_act(entity, &*world, old, game_time);
                        self.push_entities(EntityKey::Pig(id), &*world, game_time, ticks);
                        continue;
                    }
                    if entity.no_ai {
                        // `LivingEntity.aiStep` still settles small motion.
                        entity.pig.body.trim_small_velocity();
                    }
                    if !entity.no_ai {
                        entity.previous_position = entity.pig.body.position;
                        entity.previous_yaw = entity.pig.yaw;
                        entity.pig.body.trim_small_velocity();
                        if players.iter().any(|player| {
                            player.alive
                                && !player.spectator
                                && player.position.distance_squared(entity.pig.body.position)
                                    < 32.0 * 32.0
                        }) {
                            entity.no_action_time = 0;
                        }
                        entity.no_action_time += 1;
                        let full_goal_tick = entity.tick_count <= 1
                            || (entity.tick_count + entity.id as i32) % 2 == 0;
                        let last_damage_panic = DamageSourceKind::panics(entity.last_damage_source)
                            && entity.tick_count - entity.last_damage_tick <= 40;
                        let on_fire = entity.pig.body.fire_ticks > 0;
                        let mut context = PigGoalContext {
                            child: candidates[index],
                            candidates,
                            follow_parent: std::mem::take(&mut entity.follow_parent),
                            breed: std::mem::take(&mut entity.breed),
                            panic: std::mem::take(&mut entity.panic),
                            stroll: std::mem::take(&mut entity.stroll),
                            last_damage_panic,
                            on_fire,
                            no_action_time: entity.no_action_time,
                            navigation_done: entity.pig.navigation.is_done(),
                            tempt_stick: std::mem::take(&mut entity.tempt_stick),
                            tempt_food: std::mem::take(&mut entity.tempt_food),
                            random_look: std::mem::take(&mut entity.random_look),
                            look_at_player: std::mem::take(&mut entity.look_at_player),
                            players: players.to_vec(),
                            random: std::mem::take(&mut entity.random),
                            fluid: entity.fluid,
                            walk: entity.pig.walk_profile(),
                            effects: Vec::new(),
                        };
                        if full_goal_tick {
                            entity.goals.tick(&mut context, &*world);
                        } else {
                            entity.goals.tick_running(&mut context, false);
                        }
                        entity.follow_parent = context.follow_parent;
                        entity.breed = context.breed;
                        entity.panic = context.panic;
                        entity.stroll = context.stroll;
                        entity.tempt_stick = context.tempt_stick;
                        entity.tempt_food = context.tempt_food;
                        entity.random_look = context.random_look;
                        entity.look_at_player = context.look_at_player;
                        entity.random = context.random;
                        let mut jump_queued = false;
                        for effect in context.effects {
                            match effect {
                                PigGoalEffect::Navigate { target, speed } => {
                                    let fluid = entity.fluid;
                                    let _ = entity.pig.navigate_to(&*world, fluid, target, speed);
                                }
                                PigGoalEffect::NavigateToEntity { target, speed } => {
                                    let fluid = entity.fluid;
                                    let _ = entity.pig.navigate_to_entity(&*world, fluid, target, speed);
                                }
                                PigGoalEffect::LookAt {
                                    target,
                                    y_max,
                                    x_max,
                                } => {
                                    entity
                                        .look_control
                                        .set_look_at_with_limits(target, y_max, x_max);
                                }
                                PigGoalEffect::StopNavigation => entity.pig.navigation.stop(),
                                PigGoalEffect::Jump => jump_queued = true,
                                PigGoalEffect::Breed { partner_id } => {
                                    breeding_partner = Some(partner_id)
                                }
                            }
                        }
                        let pig = &mut entity.pig;
                        let position = pig.body.position;
                        let (can_update, surface) = crate::navigation::ground_view(world, &pig.body, entity.fluid, true);
                        if let Some((target, speed)) = pig.navigation.tick_in(
                            world,
                            position,
                            can_update,
                            surface,
                            pig.body.width,
                            pig.speed,
                        ) {
                            pig.move_control.set_wanted_position(target, speed);
                        }
                        let obstacle_top = crate::control::obstacle_top(world, position);
                        let control = pig.move_control.tick(
                            position,
                            pig.body.on_ground,
                            pig.yaw,
                            pig.speed,
                            pig.forward,
                            pig.sideways,
                            entity.effects.movement_speed(0.25),
                            pig.body.width,
                            pig.body.step_height,
                            obstacle_top,
                            |_, _| true,
                        );
                        pig.yaw = control.yaw;
                        pig.speed = control.speed;
                        pig.forward = control.forward;
                        pig.sideways = control.sideways;
                        entity.jumping = jump_queued || control.jump;
                        // `LivingEntity.aiStep` after the AI: a jump swims up in
                        // liquid, or leaves the ground at most every ten ticks
                        // (`getFluidJumpThreshold`: 0 below an eye height of 0.4).
                        if entity.no_jump_delay > 0 {
                            entity.no_jump_delay -= 1;
                        }
                        let jump_threshold = if pig.age.baby() { 0.0 } else { 0.4 };
                        pig.body.living_jump(&*world, entity.fluid, entity.jumping, &mut entity.no_jump_delay, jump_threshold);
                        let eye_height = if pig.age.baby() { 0.3825 } else { 0.765 };
                        entity.look_control.tick(
                            pig.body.position,
                            eye_height,
                            entity.body_rotation.body_yaw,
                            !pig.navigation.is_done(),
                        );
                        let input = DVec3::new(pig.sideways as f64, 0.0, pig.forward as f64);
                        let was_water = entity.fluid.in_water();
                        let mut fall = None;
                        if was_water {
                            pig.body.travel_water(world, input, pig.yaw);
                        } else if entity.fluid.in_lava() {
                            pig.body
                                .travel_lava(world, input, pig.yaw, entity.fluid.lava_height);
                        } else if let Some(fallen) = pig.body.travel_air(world, input, pig.speed, pig.yaw) {
                            fall = fall_damage(&pig.body, &*world, fallen, false, &mut entity.voices);
                        }
                        play_movement(&mut pig.body, entity.tick_count, &mut entity.random, &mut entity.voices, pig_sounds(pig.age.baby()));
                        entity.body_rotation.tick(
                            pig.yaw,
                            &mut entity.look_control,
                            entity.previous_position,
                            pig.body.position,
                        );
                        if !was_water {
                            entity.fluid = FluidFrame::sample(world, pig.body.position, pig.body.width, pig.body.height);
                            entity.was_touching_water = pig.body.touching_water;
                        }
                        if let Some(damage) = fall {
                            entity.hurt(damage, DamageSourceKind::Generic);
                        }
                    }
                    {
                        // `applyEffectsFromBlocks` after the move.
                        let entity = &mut self.pigs[index];
                        let old = entity.previous_position;
                        hazards::blocks_act(entity, &*world, old, game_time);
                    }
                    self.push_entities(EntityKey::Pig(id), &*world, game_time, ticks);
                    if let Some(partner_id) = breeding_partner {
                        if let Some(partner_index) =
                            self.pigs.iter().position(|e| e.id == partner_id)
                        {
                            let first_variant = self.pigs[index].pig.variant;
                            let second_variant = self.pigs[partner_index].pig.variant;
                            let first_chosen = self.pigs[index].random.next_boolean();
                            let position = self.pigs[index].pig.body.position;
                            self.pigs[index].pig.age.set(6000);
                            self.pigs[index].pig.in_love = 0;
                            self.pigs[partner_index].pig.age.set(6000);
                            self.pigs[partner_index].pig.in_love = 0;
                            let mut offspring = Pig::new(position);
                            offspring.age.set(Age::BABY_START);
                            offspring.variant = if first_chosen {
                                first_variant
                            } else {
                                second_variant
                            };
                            self.spawn_pig(offspring, false);
                        }
                    }
                    let entity = &mut self.pigs[index];
                    let was_baby = entity.pig.age.baby();
                    entity.pig.age.tick(true);
                    if was_baby != entity.pig.age.baby() {
                        entity.pig.sync_dimensions();
                    }
                    if entity.pig.age.ticks != 0 {
                        entity.pig.in_love = 0;
                    } else if entity.pig.in_love > 0 {
                        entity.pig.in_love -= 1;
                        if entity.pig.in_love % 10 == 0 {
                            for _ in 0..3 {
                                let _ = entity.random.next_gaussian();
                            }
                            for _ in 0..3 {
                                let _ = entity.random.next_double();
                            }
                        }
                    }
                    continue;
                }
                EntityKey::Sheep(id) => {
                    let index = sheep_indices[&id];
                    let candidates: Vec<_> = self
                        .sheep
                        .iter()
                        .map(|entity| CowCandidate {
                            id: entity.id,
                            position: entity.body.position,
                            width: entity.body.width,
                            height: entity.body.height,
                            age: entity.sheep.age.ticks,
                            alive: entity.health > 0.0,
                            in_love: entity.sheep.in_love,
                            panicking: entity.panic.running,
                        })
                        .collect();
                    let mut breeding_partner = None;
                    let entity = &mut self.sheep[index];
                    entity.tick_count += 1;
                    entity.update_fluid(world);
                    hazards::burn(entity, &*world, game_time);
                    let eye = if entity.sheep.age.baby() { 0.6175 } else { 1.235 };
                    hazards::suffocate(entity, &*world, eye, game_time);
                    let water_breathing = entity.effects.has(MobEffect::WaterBreathing);
                    if entity.health > 0.0 && breathe(&mut entity.body, &*world, eye, water_breathing) {
                        entity.hurt(2.0, DamageSourceKind::Generic);
                    }
                    let removed = entity.damage.tick();
                    // `tickEffects`, after the hurt and death clocks.
                    for work in entity.effects.tick(false) {
                        match work.resolve(entity.health, 8.0) {
                            Some(EffectWork::Heal(amount)) => heal(&mut entity.health, 8.0, amount),
                            Some(EffectWork::HurtMagic(amount)) => {
                                entity.hurt(amount, DamageSourceKind::Magic);
                            }
                            _ => {}
                        }
                    }
                    if entity.health > 0.0 {
                        if (entity.random.next_int(1000) as i32) < entity.ambient_sound_time {
                            entity.ambient_sound_time = -120;
                            let voice = Voice::Ambient(voice_pitch(&mut entity.random, entity.sheep.age.baby()));
                            entity.voices.push((voice, entity.position()));
                        } else {
                            entity.ambient_sound_time += 1;
                        }
                    }
                    if removed {
                        continue;
                    }
                    if entity.health <= 0.0 {
                        if !entity.no_ai {
                            let yaw = entity.yaw;
                            dying_travel(&mut entity.body, &*world, yaw, entity.tick_count, &mut entity.random, &mut entity.voices, SHEEP_SOUNDS, Some(false));
                        }
                        let old = entity.previous_position;
                        hazards::blocks_act(entity, &*world, old, game_time);
                        self.push_entities(EntityKey::Sheep(id), &*world, game_time, ticks);
                        continue;
                    }
                    if entity.no_ai {
                        // `LivingEntity.aiStep` still settles small motion.
                        entity.body.trim_small_velocity();
                    }
                    if entity.health > 0.0 && !entity.no_ai {
                        entity.previous_position = entity.body.position;
                        entity.previous_yaw = entity.yaw;
                        entity.body.trim_small_velocity();
                        if players.iter().any(|player| {
                            player.alive
                                && !player.spectator
                                && player.position.distance_squared(entity.body.position)
                                    < 32.0 * 32.0
                        }) {
                            entity.no_action_time = 0;
                        }
                        entity.no_action_time += 1;
                        let pos = (
                            entity.body.position.x.floor() as i32,
                            entity.body.position.y.floor() as i32,
                            entity.body.position.z.floor() as i32,
                        );
                        let below = (pos.0, pos.1 - 1, pos.2);
                        let edible_here = world
                            .block(pos)
                            .is_some_and(|block| edible_for_sheep(&block.id));
                        let grass_below = world
                            .block(below)
                            .is_some_and(|block| block.id == "minecraft:grass_block");
                        let full_goal_tick = entity.tick_count <= 1
                            || (entity.tick_count + entity.id as i32) % 2 == 0;
                        let last_damage_panic = DamageSourceKind::panics(entity.last_damage_source)
                            && entity.tick_count - entity.last_damage_tick <= 40;
                        let on_fire = entity.body.fire_ticks > 0;
                        let mut context = SheepGoalContext {
                            baby: entity.sheep.age.baby(),
                            edible_here,
                            grass_below,
                            random: std::mem::take(&mut entity.random),
                            eat_animation_ticks: entity.eat_animation_ticks,
                            ate: false,
                            child: candidates[index],
                            candidates,
                            players: players.to_vec(),
                            breed: std::mem::take(&mut entity.breed),
                            tempt: std::mem::take(&mut entity.tempt),
                            follow_parent: std::mem::take(&mut entity.follow_parent),
                            random_look: std::mem::take(&mut entity.random_look),
                            look_at_player: std::mem::take(&mut entity.look_at_player),
                            stroll: std::mem::take(&mut entity.stroll),
                            panic: std::mem::take(&mut entity.panic),
                            last_damage_panic,
                            on_fire,
                            fluid: entity.fluid,
                            walk: WalkProfile::animal(entity.body.width, entity.body.height),
                            no_action_time: entity.no_action_time,
                            navigation_done: entity.navigation.is_done(),
                            effects: Vec::new(),
                        };
                        if full_goal_tick {
                            entity.goals.tick(&mut context, &*world);
                        } else {
                            entity.goals.tick_running(&mut context, false);
                        }
                        entity.random = context.random;
                        entity.breed = context.breed;
                        entity.tempt = context.tempt;
                        entity.follow_parent = context.follow_parent;
                        entity.random_look = context.random_look;
                        entity.look_at_player = context.look_at_player;
                        entity.stroll = context.stroll;
                        entity.panic = context.panic;
                        entity.eat_animation_ticks = context.eat_animation_ticks;
                        if context.ate {
                            if self.mob_griefing {
                                if edible_here {
                                    world.set_block(pos, None);
                                } else if grass_below {
                                    world.set_block(below, Some(Block::new("minecraft:dirt")));
                                }
                            }
                            entity.sheep.ate();
                        }
                        let mut jump_queued = false;
                        for effect in context.effects {
                            match effect {
                                SheepGoalEffect::Navigate { target, speed } => {
                                    let profile = WalkProfile::animal(entity.body.width, entity.body.height);
                                    let _ = navigate_walk_to(
                                        &entity.body,
                                        &mut entity.navigation,
                                        &*world,
                                        &profile,
                                        entity.fluid,
                                        target,
                                        speed,
                                        1,
                                    );
                                }
                                SheepGoalEffect::NavigateToEntity { target, speed } => {
                                    let profile = WalkProfile::animal(entity.body.width, entity.body.height);
                                    let _ = crate::navigation::navigate_walk_to_entity(
                                        &entity.body,
                                        &mut entity.navigation,
                                        &*world,
                                        &profile,
                                        entity.fluid,
                                        target,
                                        speed,
                                        1,
                                    );
                                }
                                SheepGoalEffect::StopNavigation => entity.navigation.stop(),
                                SheepGoalEffect::Jump => jump_queued = true,
                                SheepGoalEffect::LookAt {
                                    target,
                                    y_max,
                                    x_max,
                                } => entity
                                    .look_control
                                    .set_look_at_with_limits(target, y_max, x_max),
                                SheepGoalEffect::Breed { partner_id } => {
                                    breeding_partner = Some(partner_id)
                                }
                            }
                        }
                        let position = entity.body.position;
                        let (can_update, surface) = crate::navigation::ground_view(world, &entity.body, entity.fluid, true);
                        if let Some((target, speed)) = entity.navigation.tick_in(
                            world,
                            position,
                            can_update,
                            surface,
                            entity.body.width,
                            entity.speed,
                        ) {
                            entity.move_control.set_wanted_position(target, speed);
                        }
                        let obstacle_top = crate::control::obstacle_top(world, position);
                        let control = entity.move_control.tick(
                            position,
                            entity.body.on_ground,
                            entity.yaw,
                            entity.speed,
                            entity.forward,
                            entity.sideways,
                            entity.effects.movement_speed(0.23),
                            entity.body.width,
                            entity.body.step_height,
                            obstacle_top,
                            |_, _| true,
                        );
                        entity.yaw = control.yaw;
                        entity.speed = control.speed;
                        entity.forward = control.forward;
                        entity.sideways = control.sideways;
                        entity.jumping = jump_queued || control.jump;
                        // `LivingEntity.aiStep` after the AI: a jump swims up in
                        // liquid, or leaves the ground at most every ten ticks
                        // (`getFluidJumpThreshold`: 0 below an eye height of 0.4).
                        if entity.no_jump_delay > 0 {
                            entity.no_jump_delay -= 1;
                        }
                        let jump_threshold = 0.4;
                        entity.body.living_jump(&*world, entity.fluid, entity.jumping, &mut entity.no_jump_delay, jump_threshold);
                        let eye_height = if entity.sheep.age.baby() {
                            0.6175
                        } else {
                            1.235
                        };
                        entity.look_control.tick(
                            entity.body.position,
                            eye_height,
                            entity.body_rotation.body_yaw,
                            !entity.navigation.is_done(),
                        );
                        let input = DVec3::new(entity.sideways as f64, 0.0, entity.forward as f64);
                        let was_water = entity.fluid.in_water();
                        let mut fall = None;
                        if was_water {
                            entity.body.travel_water(world, input, entity.yaw);
                        } else if entity.fluid.in_lava() {
                            entity.body.travel_lava(
                                world,
                                input,
                                entity.yaw,
                                entity.fluid.lava_height,
                            );
                        } else if let Some(fallen) = entity.body.travel_air(world, input, entity.speed, entity.yaw) {
                            fall = fall_damage(&entity.body, &*world, fallen, false, &mut entity.voices);
                        }
                        play_movement(&mut entity.body, entity.tick_count, &mut entity.random, &mut entity.voices, SHEEP_SOUNDS);
                        entity.body_rotation.tick(
                            entity.yaw,
                            &mut entity.look_control,
                            entity.previous_position,
                            entity.body.position,
                        );
                        if !was_water {
                            entity.fluid = FluidFrame::sample(world, entity.body.position, entity.body.width, entity.body.height);
                            entity.was_touching_water = entity.body.touching_water;
                        }
                        if let Some(damage) = fall {
                            entity.hurt(damage, DamageSourceKind::Generic);
                        }
                    }
                    {
                        // `applyEffectsFromBlocks` after the move.
                        let entity = &mut self.sheep[index];
                        let old = entity.previous_position;
                        hazards::blocks_act(entity, &*world, old, game_time);
                    }
                    self.push_entities(EntityKey::Sheep(id), &*world, game_time, ticks);
                    if let Some(partner_id) = breeding_partner {
                        if let Some(partner_index) =
                            self.sheep.iter().position(|e| e.id == partner_id)
                        {
                            self.sheep[index].sheep.age.set(6000);
                            self.sheep[index].sheep.in_love = 0;
                            self.sheep[partner_index].sheep.age.set(6000);
                            self.sheep[partner_index].sheep.in_love = 0;
                            let mut offspring = Sheep::default();
                            offspring.age.set(Age::BABY_START);
                            let first = self.sheep[index].sheep.wool.data() & 15;
                            let second = self.sheep[partner_index].sheep.wool.data() & 15;
                            let mixed = self
                                .recipes
                                .as_ref()
                                .and_then(|recipes| {
                                    let dye = |color: u8| {
                                        Some(ItemStack::new(
                                            format!(
                                                "minecraft:{}_dye",
                                                crate::sheep::DYE_NAMES[color as usize]
                                            ),
                                            1,
                                        ))
                                    };
                                    let grid = [dye(first), dye(second)];
                                    recipes
                                        .matching(&grid, 2, 1)
                                        .and_then(|result| {
                                            crate::sheep::DYE_NAMES.iter().position(|color| {
                                                result.id == format!("minecraft:{color}_dye")
                                            })
                                        })
                                        .map(|index| index as u8)
                                })
                                .unwrap_or_else(|| {
                                    if self.mix_random.next_boolean() {
                                        first
                                    } else {
                                        second
                                    }
                                });
                            offspring.wool.set_color(mixed);
                            let position = self.sheep[index].body.position;
                            self.spawn_sheep(offspring, position, false);
                        }
                    }
                    let entity = &mut self.sheep[index];
                    entity.sheep.age.tick(entity.health > 0.0);
                    if entity.sheep.age.ticks != 0 {
                        entity.sheep.in_love = 0;
                    } else if entity.sheep.in_love > 0 {
                        entity.sheep.in_love -= 1;
                        if entity.sheep.in_love % 10 == 0 {
                            for _ in 0..3 {
                                let _ = entity.random.next_gaussian();
                            }
                            for _ in 0..3 {
                                let _ = entity.random.next_double();
                            }
                        }
                    }
                    entity.sync_dimensions();
                    continue;
                }
            };
            let current_mooshroom = self.cows[index].mooshroom.is_some();
            let current_horse = self.cows[index].horse.as_ref().map(|h| h.kind);
            let candidates: Vec<_> = self
                .cows
                .iter()
                .map(|entity| CowCandidate {
                    id: entity.id,
                    position: entity.cow.body.position,
                    width: entity.cow.body.width,
                    height: entity.cow.body.height,
                    age: entity.cow.age.ticks,
                    alive: entity.cow.health > 0.0
                        && entity.mooshroom.is_some() == current_mooshroom
                        && entity.horse.as_ref().map(|h| h.kind) == current_horse,
                    in_love: entity.cow.in_love,
                    panicking: entity.panic.running,
                })
                .collect();
            let mut breeding_partner = None;
            let entity = &mut self.cows[index];
            entity.tick_count += 1;
            entity.update_fluid(world);
            hazards::burn(entity, &*world, game_time);
            let eye = entity.eye_height();
            hazards::suffocate(entity, &*world, eye, game_time);
            let water_breathing = entity.effects.has(MobEffect::WaterBreathing);
            if entity.cow.health > 0.0 && breathe(&mut entity.cow.body, &*world, eye, water_breathing) {
                entity.hurt(2.0, DamageSourceKind::Generic);
            }
            let removed = entity.cow.damage.tick();
            // `tickEffects`, after the hurt and death clocks.
            for work in entity.effects.tick(false) {
                match work.resolve(entity.cow.health, 10.0) {
                    Some(EffectWork::Heal(amount)) => heal(&mut entity.cow.health, 10.0, amount),
                    Some(EffectWork::HurtMagic(amount)) => {
                        entity.hurt(amount, DamageSourceKind::Magic);
                    }
                    _ => {}
                }
            }
            // Mob.baseTick consumes its ambient-sound trial while alive.
            if entity.cow.health > 0.0 {
                if (entity.random.next_int(1000) as i32) < entity.ambient_sound_time {
                    entity.ambient_sound_time = -entity.ambient_interval();
                    // LivingEntity.getVoicePitch consumes two floats for the cow sound.
                    let baby = entity.cow.age.baby();
                    let voice = Voice::Ambient(voice_pitch(&mut entity.random, baby));
                            entity.voices.push((voice, entity.position()));
                } else {
                    entity.ambient_sound_time += 1;
                }
            }
            entity.previous_position = entity.cow.body.position;
            entity.previous_yaw = entity.cow.yaw;
            if removed {
                continue;
            }
            if entity.cow.health <= 0.0 {
                if !entity.no_ai {
                    let sounds = cow_sounds(entity.mooshroom.is_some(), entity.cow.sound_variant);
                    let yaw = entity.cow.yaw;
                    dying_travel(&mut entity.cow.body, &*world, yaw, entity.tick_count, &mut entity.random, &mut entity.voices, sounds, Some(false));
                }
                let old = entity.previous_position;
                hazards::blocks_act(entity, &*world, old, game_time);
                let id = self.cows[index].id;
                self.push_entities(EntityKey::Cow(id), &*world, game_time, ticks);
                continue;
            }
            // `AbstractHorse.aiStep` opens with its tail's roll.
            if let Some(horse) = &mut entity.horse {
                if entity.random.next_int(200) == 0 {
                    horse.tail_counter = 1;
                }
            }
            // `isImmobile` (grazing or rearing): `LivingEntity.aiStep` clears
            // the inputs and skips the whole server AI.
            let immobile = entity.horse.as_ref().is_some_and(|h| h.immobile());
            if entity.no_ai {
                // `LivingEntity.aiStep` still settles small motion.
                entity.cow.body.trim_small_velocity();
            }
            if !entity.no_ai && immobile {
                entity.cow.body.trim_small_velocity();
                if players.iter().any(|player| player.alive && !player.spectator && player.position.distance_squared(entity.cow.body.position) < 32.0 * 32.0) {
                    entity.no_action_time = 0;
                }
                let cow = &mut entity.cow;
                cow.forward = 0.0;
                cow.sideways = 0.0;
                entity.jumping = false;
                entity.no_jump_delay = 0;
                let input = DVec3::ZERO;
                let was_water = entity.fluid.in_water();
                let mut fall = None;
                if was_water {
                    cow.body.travel_water(world, input, cow.yaw);
                } else if entity.fluid.in_lava() {
                    cow.body.travel_lava(world, input, cow.yaw, entity.fluid.lava_height);
                } else if let Some(fallen) = cow.body.travel_air(world, input, cow.speed, cow.yaw) {
                    fall = fall_damage(&cow.body, &*world, fallen, false, &mut entity.voices);
                }
                let sounds = if let Some(horse) = &entity.horse {
                    let family = horse.kind.sound_family(cow.age.baby());
                    MovementSounds { horse_step: Some((if family == "baby_horse" { "entity.baby_horse.step" } else { "entity.horse.step" }, "entity.horse.step_wood")), ..MovementSounds::creature(None) }
                } else {
                    cow_sounds(false, cow.sound_variant)
                };
                play_movement(&mut cow.body, entity.tick_count, &mut entity.random, &mut entity.voices, sounds);
                entity.body_rotation.tick(cow.yaw, &mut entity.look_control, entity.previous_position, cow.body.position);
                if !was_water {
                    entity.fluid = FluidFrame::sample(world, cow.body.position, cow.body.width, cow.body.height);
                    entity.was_touching_water = cow.body.touching_water;
                }
                if let Some(damage) = fall {
                    entity.hurt(damage, DamageSourceKind::Generic);
                }
            }
            if !entity.no_ai && !immobile {
                entity.cow.body.trim_small_velocity();
                // Mob.checkDespawn clears inactivity near a player before serverAiStep.
                if players.iter().any(|player| {
                    player.alive
                        && !player.spectator
                        && player.position.distance_squared(entity.cow.body.position) < 32.0 * 32.0
                }) {
                    entity.no_action_time = 0;
                }
                entity.no_action_time += 1;
                let cow = &mut entity.cow;
                let position = cow.body.position;
                let full_goal_tick =
                    entity.tick_count <= 1 || (entity.tick_count + entity.id as i32) % 2 == 0;
                let last_damage_panic = DamageSourceKind::panics(entity.last_damage_source)
                    && entity.tick_count - entity.last_damage_tick <= 40;
                let on_fire = cow.body.fire_ticks > 0;
                let species = entity.horse.as_ref().map_or(Species::COW, |h| Species::horse(h.kind));
                let stand = entity.horse.as_ref().map(|h| StandState { next_stand: h.next_stand, immobile: h.immobile() });
                let mut context = CowGoalContext {
                    species,
                    stand,
                    child: candidates[index],
                    candidates,
                    players: players.to_vec(),
                    breed: std::mem::take(&mut entity.breed),
                    tempt: std::mem::take(&mut entity.tempt),
                    follow_parent: std::mem::take(&mut entity.follow_parent),
                    random_look: std::mem::take(&mut entity.random_look),
                    look_at_player: std::mem::take(&mut entity.look_at_player),
                    stroll: std::mem::take(&mut entity.stroll),
                    panic: std::mem::take(&mut entity.panic),
                    last_damage_panic,
                    on_fire,
                    fluid: entity.fluid,
                    walk: cow.walk_profile(),
                    no_action_time: entity.no_action_time,
                    navigation_done: cow.navigation.is_done(),
                    random: std::mem::take(&mut entity.random),
                    effects: Vec::new(),
                };
                if full_goal_tick {
                    entity.goals.tick(&mut context, &*world);
                } else {
                    entity.goals.tick_running(&mut context, false);
                }
                entity.breed = context.breed;
                entity.tempt = context.tempt;
                entity.follow_parent = context.follow_parent;
                entity.random_look = context.random_look;
                entity.look_at_player = context.look_at_player;
                entity.stroll = context.stroll;
                entity.panic = context.panic;
                entity.random = context.random;
                if let (Some(horse), Some(stand)) = (&mut entity.horse, context.stand) {
                    horse.next_stand = stand.next_stand;
                }
                let mut jump_queued = false;
                for effect in context.effects {
                    match effect {
                        CowGoalEffect::Navigate { target, speed } => {
                            let _ = cow.navigate_to(&*world, entity.fluid, target, speed);
                        }
                        CowGoalEffect::NavigateToEntity { target, speed } => {
                            let _ = cow.navigate_to_entity(&*world, entity.fluid, target, speed);
                        }
                        CowGoalEffect::StopNavigation => cow.navigation.stop(),
                        CowGoalEffect::LookAt {
                            target,
                            y_max,
                            x_max,
                        } => entity
                            .look_control
                            .set_look_at_with_limits(target, y_max, x_max),
                        CowGoalEffect::Breed { partner_id } => {
                            breeding_partner = Some(partner_id);
                        }
                        CowGoalEffect::Jump => jump_queued = true,
                        CowGoalEffect::Stand => {
                            // `RandomStandGoal.start`: rear, and the ambient
                            // stand sound (`playSound(sound)`: volume and
                            // pitch 1).
                            if let Some(horse) = &mut entity.horse {
                                horse.stand_if_possible();
                                let family = horse.kind.sound_family(cow.age.baby());
                                let event = match family {
                                    "baby_horse" => "entity.baby_horse.ambient",
                                    "donkey" => "entity.donkey.ambient",
                                    _ => "entity.horse.ambient",
                                };
                                entity.voices.push((Voice::Event(event, 1.0, 1.0), cow.body.position));
                            }
                        }
                    }
                }
                let (can_update, surface) = crate::navigation::ground_view(world, &cow.body, entity.fluid, true);
                if let Some((target, speed)) =
                    cow.navigation
                        .tick_in(world, position, can_update, surface, cow.body.width, cow.speed)
                {
                    cow.move_control.set_wanted_position(target, speed);
                }
                let obstacle_top = crate::control::obstacle_top(world, position);
                let control = cow.move_control.tick(
                    position,
                    cow.body.on_ground,
                    cow.yaw,
                    cow.speed,
                    cow.forward,
                    cow.sideways,
                    entity.effects.movement_speed(entity.horse.as_ref().map_or(0.2, |h| h.movement_speed)),
                    cow.body.width,
                    cow.body.step_height,
                    obstacle_top,
                    |_, _| true,
                );
                cow.yaw = control.yaw;
                cow.speed = control.speed;
                cow.forward = control.forward;
                cow.sideways = control.sideways;
                entity.jumping = jump_queued || control.jump;
                // `LivingEntity.aiStep` after the AI: a jump swims up in
                // liquid, or leaves the ground at most every ten ticks
                // (`getFluidJumpThreshold`: 0 below an eye height of 0.4).
                if entity.no_jump_delay > 0 {
                    entity.no_jump_delay -= 1;
                }
                let jump_threshold = 0.4;
                cow.body.living_jump(&*world, entity.fluid, entity.jumping, &mut entity.no_jump_delay, jump_threshold);
                let eye_height = match (&entity.horse, cow.age.baby(), entity.mooshroom.is_some()) {
                    (Some(horse), baby, _) => horse.kind.eye_height(baby),
                    (None, false, _) => 1.3,
                    (None, true, true) => 0.69,
                    (None, true, false) => 0.665,
                };
                entity.look_control.tick(
                    cow.body.position,
                    eye_height,
                    entity.body_rotation.body_yaw,
                    !cow.navigation.is_done(),
                );
                let input = DVec3::new(cow.sideways as f64, 0.0, cow.forward as f64);
                let was_water = entity.fluid.in_water();
                let mut fall = None;
                if was_water {
                    cow.body.travel_water(world, input, cow.yaw);
                } else if entity.fluid.in_lava() {
                    cow.body
                        .travel_lava(world, input, cow.yaw, entity.fluid.lava_height);
                } else if let Some(fallen) = cow.body.travel_air(world, input, cow.speed, cow.yaw) {
                    fall = fall_damage(&cow.body, &*world, fallen, false, &mut entity.voices);
                }
                let sounds = if let Some(horse) = &entity.horse {
                    let family = horse.kind.sound_family(cow.age.baby());
                    MovementSounds { horse_step: Some((if family == "baby_horse" { "entity.baby_horse.step" } else { "entity.horse.step" }, "entity.horse.step_wood")), ..MovementSounds::creature(None) }
                } else {
                    cow_sounds(entity.mooshroom.is_some(), cow.sound_variant)
                };
                play_movement(&mut cow.body, entity.tick_count, &mut entity.random, &mut entity.voices, sounds);
                entity.body_rotation.tick(
                    cow.yaw,
                    &mut entity.look_control,
                    entity.previous_position,
                    cow.body.position,
                );
                // `LivingEntity.checkFallDamage` refreshed the water state
                // during the move.
                if !was_water {
                    entity.fluid = FluidFrame::sample(world, cow.body.position, cow.body.width, cow.body.height);
                    entity.was_touching_water = cow.body.touching_water;
                }
                if let Some(damage) = fall {
                    entity.hurt(damage, DamageSourceKind::Generic);
                }
            }
            {
                // `applyEffectsFromBlocks` after the move.
                let entity = &mut self.cows[index];
                let old = entity.previous_position;
                hazards::blocks_act(entity, &*world, old, game_time);
            }
            let id = self.cows[index].id;
            self.push_entities(EntityKey::Cow(id), &*world, game_time, ticks);
            if let Some(partner_id) = breeding_partner {
                if let Some(partner_index) = self.cows.iter().position(|e| e.id == partner_id) {
                    let parent_variant = self.cows[index]
                        .mooshroom
                        .as_ref()
                        .map(|state| state.variant);
                    let mate_variant = self.cows[partner_index]
                        .mooshroom
                        .as_ref()
                        .map(|state| state.variant);
                    let offspring_variant = match (parent_variant, mate_variant) {
                        (Some(first), Some(second)) => {
                            let random = &mut self.cows[index].random;
                            if first == second && random.next_int(1024) == 0 {
                                Some(if first == MushroomVariant::Brown {
                                    MushroomVariant::Red
                                } else {
                                    MushroomVariant::Brown
                                })
                            } else {
                                Some(if random.next_boolean() { first } else { second })
                            }
                        }
                        _ => None,
                    };
                    self.cows[index].cow.age.set(6000);
                    self.cows[index].cow.in_love = 0;
                    self.cows[partner_index].cow.age.set(6000);
                    self.cows[partner_index].cow.in_love = 0;
                    let mut offspring = Cow::new(self.cows[index].cow.body.position);
                    offspring.age.set(Age::BABY_START);
                    if let Some(variant) = offspring_variant {
                        self.spawn_mooshroom(
                            MushroomCow {
                                cow: offspring,
                                state: MushroomCowState {
                                    variant,
                                    stew_effects: None,
                                    last_lightning_bolt_uuid: None,
                                },
                            },
                            false,
                        );
                    } else {
                        self.spawn_cow(offspring, false);
                    }
                }
            }
            let (_, hearts) = self.cows[index].cow.tick_age_and_love();
            if hearts {
                let random = &mut self.cows[index].random;
                for _ in 0..3 {
                    let _ = random.next_gaussian();
                }
                for _ in 0..3 {
                    let _ = random.next_double();
                }
            }
            // The rest of `AbstractHorse.aiStep` (a one-in-900 heal, then
            // grazing on grass: a one-in-300 start, fifty ticks long), and of
            // `AbstractHorse.tick` (its counters and animations).
            let entity = &mut self.cows[index];
            if let Some(horse) = &mut entity.horse {
                if entity.cow.health > 0.0 {
                    if entity.random.next_int(900) == 0 && entity.cow.damage.death_ticks == 0 {
                        heal(&mut entity.cow.health, entity.cow.max_health, 1.0);
                    }
                    let p = entity.cow.body.position;
                    let below = (p.x.floor() as i32, p.y.floor() as i32 - 1, p.z.floor() as i32);
                    if !horse.eating && entity.random.next_int(300) == 0 && world.block(below).is_some_and(|b| b.id == "minecraft:grass_block") {
                        horse.eating = true;
                    }
                    if horse.eating {
                        horse.eating_counter += 1;
                        if horse.eating_counter > 50 {
                            horse.eating_counter = 0;
                            horse.eating = false;
                        }
                    }
                }
                horse.tick_animation();
            }
        }
        self.bats
            .retain(|entity| entity.bat.damage.death_ticks < 20);
        self.zombies
            .retain(|entity| entity.zombie.damage.death_ticks < 20);
        self.skeletons
            .retain(|entity| entity.skeleton.damage.death_ticks < 20);
        self.creepers
            .retain(|entity| !entity.creeper.exploded && entity.creeper.damage.death_ticks < 20);
        self.spiders.retain(|entity| entity.spider.damage.death_ticks < 20);
        self.split_dead_slimes();
        self.slimes.retain(|entity| entity.slime.damage.death_ticks < 20);
        self.endermen.retain(|entity| entity.enderman.damage.death_ticks < 20);
        self.witches.retain(|entity| entity.witch.damage.death_ticks < 20);
        self.iron_golems.retain(|entity| entity.golem.damage.death_ticks < 20);
        self.wolves.retain(|entity| entity.wolf.damage.death_ticks < 20);
        self.potions.retain(|entity| entity.potion.alive);
        self.villagers
            .retain(|entity| entity.villager.damage.death_ticks < 20);
        self.cows
            .retain(|entity| entity.cow.damage.death_ticks < 20);
        self.sheep.retain(|entity| entity.damage.death_ticks < 20);
        self.pigs
            .retain(|entity| entity.pig.damage.death_ticks < 20);
        self.chickens
            .retain(|entity| entity.chicken.damage.death_ticks < 20);
        self.arrows.retain(|entity| entity.arrow.alive);
        let alive_ids: HashSet<_> = self
            .bats
            .iter()
            .map(|entity| entity.id)
            .chain(self.cows.iter().map(|entity| entity.id))
            .chain(self.zombies.iter().map(|entity| entity.id))
            .chain(self.skeletons.iter().map(|entity| entity.id))
            .chain(self.creepers.iter().map(|entity| entity.id))
            .chain(self.spiders.iter().map(|entity| entity.id))
            .chain(self.slimes.iter().map(|entity| entity.id))
            .chain(self.endermen.iter().map(|entity| entity.id))
            .chain(self.witches.iter().map(|entity| entity.id))
            .chain(self.iron_golems.iter().map(|entity| entity.id))
            .chain(self.wolves.iter().map(|entity| entity.id))
            .chain(self.arrows.iter().map(|entity| entity.id))
            .chain(self.potions.iter().map(|entity| entity.id))
            .chain(self.villagers.iter().map(|entity| entity.id))
            .chain(self.sheep.iter().map(|entity| entity.id))
            .chain(self.pigs.iter().map(|entity| entity.id))
            .chain(self.chickens.iter().map(|entity| entity.id))
            .collect();
        self.sections.retain_mobs(|id| alive_ids.contains(&id));
        self.order.retain(|key| match key {
            EntityKey::Bat(id)
            | EntityKey::Zombie(id)
            | EntityKey::Skeleton(id)
            | EntityKey::Creeper(id)
            | EntityKey::Spider(id)
            | EntityKey::Slime(id)
            | EntityKey::Enderman(id)
            | EntityKey::Witch(id)
            | EntityKey::IronGolem(id)
            | EntityKey::Wolf(id)
            | EntityKey::Arrow(id)
            | EntityKey::Potion(id)
            | EntityKey::Villager(id)
            | EntityKey::Cow(id)
            | EntityKey::Sheep(id)
            | EntityKey::Pig(id)
            | EntityKey::Chicken(id) => alive_ids.contains(id),
        });
    }

    /// Returns the nearest entity AABB hit in front of the eye, using the
    /// stable insertion order to break equal-distance ties.
    pub fn cow_on_ray(&self, eye: DVec3, look: DVec3, reach: f64) -> Option<(u64, f64)> {
        let mut nearest = None;
        for entity in &self.cows {
            if entity.cow.health <= 0.0 || entity.mooshroom.is_some() {
                continue;
            }
            let body = &entity.cow.body;
            let half = body.width as f64 / 2.0;
            let min = body.position + DVec3::new(-half, 0.0, -half);
            let max = body.position + DVec3::new(half, body.height as f64, half);
            if let Some(distance) = ray_box(eye, look, min, max, reach) {
                if nearest.is_none_or(|(_, best)| distance < best) {
                    nearest = Some((entity.id, distance));
                }
            }
        }
        nearest
    }

    /// The nearest living mob AABB across species, with spawn order for ties.
    pub fn mob_on_ray(&self, eye: DVec3, look: DVec3, reach: f64) -> Option<(MobHit, f64)> {
        let mut nearest = None;
        for key in &self.order {
            let (hit, body) = match *key {
                EntityKey::Bat(id) => {
                    let Some(entity) = self.bats.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.bat.health <= 0.0 {
                        continue;
                    }
                    (MobHit::Bat(id), &entity.bat.body)
                }
                EntityKey::Zombie(id) => {
                    let Some(entity) = self.zombies.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.zombie.health <= 0.0 {
                        continue;
                    }
                    (MobHit::Zombie(id), &entity.zombie.body)
                }
                EntityKey::Skeleton(id) => {
                    let Some(entity) = self.skeletons.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.skeleton.health <= 0.0 {
                        continue;
                    }
                    (MobHit::Skeleton(id), &entity.skeleton.body)
                }
                EntityKey::Creeper(id) => {
                    let Some(entity) = self.creepers.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.creeper.health <= 0.0 || entity.creeper.exploded {
                        continue;
                    }
                    (MobHit::Creeper(id), &entity.creeper.body)
                }
                EntityKey::Spider(id) => {
                    let Some(entity) = self.spiders.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.spider.health <= 0.0 {
                        continue;
                    }
                    (MobHit::Spider(id), &entity.spider.body)
                }
                EntityKey::Slime(id) => {
                    let Some(entity) = self.slimes.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.slime.health <= 0.0 {
                        continue;
                    }
                    (MobHit::Slime(id), &entity.slime.body)
                }
                EntityKey::Enderman(id) => {
                    let Some(entity) = self.endermen.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.enderman.health <= 0.0 {
                        continue;
                    }
                    (MobHit::Enderman(id), &entity.enderman.body)
                }
                EntityKey::Witch(id) => {
                    let Some(entity) = self.witches.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.witch.health <= 0.0 {
                        continue;
                    }
                    (MobHit::Witch(id), &entity.witch.body)
                }
                EntityKey::IronGolem(id) => {
                    let Some(entity) = self.iron_golems.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.golem.health <= 0.0 {
                        continue;
                    }
                    (MobHit::IronGolem(id), &entity.golem.body)
                }
                EntityKey::Wolf(id) => {
                    let Some(entity) = self.wolves.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.wolf.health <= 0.0 {
                        continue;
                    }
                    (MobHit::Wolf(id), &entity.wolf.body)
                }
                EntityKey::Arrow(_) | EntityKey::Potion(_) => continue,
                EntityKey::Villager(id) => {
                    let Some(entity) = self.villagers.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.villager.health <= 0.0 {
                        continue;
                    }
                    (MobHit::Villager(id), &entity.villager.body)
                }
                EntityKey::Cow(id) => {
                    let Some(entity) = self.cows.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.cow.health <= 0.0 {
                        continue;
                    }
                    (
                        if entity.mooshroom.is_some() {
                            MobHit::Mooshroom(id)
                        } else {
                            MobHit::Cow(id)
                        },
                        &entity.cow.body,
                    )
                }
                EntityKey::Sheep(id) => {
                    let Some(entity) = self.sheep.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.health <= 0.0 {
                        continue;
                    }
                    (MobHit::Sheep(id), &entity.body)
                }
                EntityKey::Pig(id) => {
                    let Some(entity) = self.pigs.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.pig.health <= 0.0 {
                        continue;
                    }
                    (MobHit::Pig(id), &entity.pig.body)
                }
                EntityKey::Chicken(id) => {
                    let Some(entity) = self.chickens.iter().find(|entity| entity.id == id) else {
                        continue;
                    };
                    if entity.chicken.health <= 0.0 {
                        continue;
                    }
                    (MobHit::Chicken(id), &entity.chicken.body)
                }
            };
            let half = f64::from(body.width) / 2.0;
            let min = body.position + DVec3::new(-half, 0.0, -half);
            let max = body.position + DVec3::new(half, f64::from(body.height), half);
            if let Some(distance) = ray_box(eye, look, min, max, reach) {
                if nearest.is_none_or(|(_, best)| distance < best) {
                    nearest = Some((hit, distance));
                }
            }
        }
        nearest
    }
}

fn ray_box(eye: DVec3, look: DVec3, min: DVec3, max: DVec3, reach: f64) -> Option<f64> {
    let mut entry: f64 = 0.0;
    let mut exit = reach;
    for axis in 0..3 {
        if look[axis].abs() < 1e-12 {
            if eye[axis] < min[axis] || eye[axis] > max[axis] {
                return None;
            }
        } else {
            let a = (min[axis] - eye[axis]) / look[axis];
            let b = (max[axis] - eye[axis]) / look[axis];
            entry = entry.max(a.min(b));
            exit = exit.min(a.max(b));
            if entry > exit {
                return None;
            }
        }
    }
    Some(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EmptyWorld;
    impl World for EmptyWorld {
        fn block(&self, _: minecraftoss_player::Pos) -> Option<minecraftoss_player::Block> {
            None
        }
        fn set_block(
            &mut self,
            _: minecraftoss_player::Pos,
            _: Option<minecraftoss_player::Block>,
        ) {
        }
    }

    /// A skeleton's arrow kills a creeper: the death names the skeleton
    /// (a disc for the creeper's loot); a player's kill remembers the
    /// player, and a baby zombie leaves its loot and 12 experience.
    #[test]
    fn deaths_name_their_killer() {
        let mut world = EntityWorld::default();
        let mut creeper = Creeper::new(DVec3::new(0.5, 1.0, 4.5));
        creeper.health = 1.0;
        let creeper = world.spawn_creeper(creeper, true);
        let archer = world.spawn_skeleton(Skeleton::new(DVec3::new(0.5, 1.0, -2.5)), true);
        world.spawn_owned_arrow(archer, Arrow::in_flight(DVec3::new(0.5, 1.5, 1.5), DVec3::Z, LegacyRandom::new(0)));
        for _ in 0..4 {
            world.tick(&mut EmptyWorld);
        }
        let deaths = world.take_deaths();
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].id, creeper);
        assert_eq!(deaths[0].table, Some("minecraft:creeper"));
        assert_eq!(deaths[0].context.attacker, Some("minecraft:skeleton"));
        assert!(!deaths[0].context.killed_by_player);
        assert_eq!(deaths[0].experience, 0);
        assert!(world.take_deaths().is_empty(), "reported once");

        let mut baby = Zombie::new(DVec3::new(8.5, 1.0, 0.5));
        baby.set_baby(true);
        baby.health = 1.0;
        let baby = world.spawn_zombie(baby, true);
        let attack = PlayerAttack {
            player_id: 7,
            position: DVec3::new(8.5, 1.0, 2.5),
            yaw: 180.0,
            attack_damage: 9.0,
            strength: 1.0,
            sprinting: false,
            can_critical: false,
            can_sweep: false,
        };
        assert!(world.player_attack(&attack, baby).died);
        let deaths = world.take_deaths();
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].table, Some("minecraft:zombie"), "monsters drop loot as babies too");
        assert!(deaths[0].context.killed_by_player && deaths[0].context.baby);
        assert_eq!(deaths[0].context.attacker, Some("minecraft:player"));
        assert_eq!(deaths[0].experience, 12);
    }

    #[test]
    fn drowned_conversion_replaces_id_and_copies_common_mob_state() {
        let mut world = EntityWorld::default();
        let mut zombie = Zombie::new(DVec3::new(2.5, 1.0, 4.5));
        zombie.health = 13.0;
        zombie.set_baby(true);
        zombie.body.velocity = DVec3::new(0.0, -0.005, 0.0);
        zombie.body.on_ground = true;
        zombie.persistence_required = true;
        zombie.can_break_doors = true;
        let original_id = world.spawn_zombie_drowning(zombie);
        let drowned_id = world.convert_zombie_to_drowned(original_id);
        assert_ne!(drowned_id, original_id);
        assert!(world.zombie_mut(original_id).is_none());
        let entity = world.zombie_mut(drowned_id).unwrap();
        assert_eq!(entity.zombie.kind, ZombieKind::Drowned);
        assert_eq!(entity.zombie.health, 20.0);
        assert!(entity.zombie.baby);
        assert!(entity.zombie.persistence_required);
        assert!(entity.zombie.can_break_doors);
        assert_eq!(entity.zombie.body.velocity, DVec3::new(0.0, -0.005, 0.0));
        assert!(entity.zombie.body.on_ground);
        assert_eq!(entity.zombie.body.step_height, 1.0);
        assert_eq!(entity.tick_count, 0);
        assert_eq!(world.order.len(), 1);
        assert!(matches!(world.order[0], EntityKey::Zombie(id) if id == drowned_id));
    }

    #[test]
    fn zombie_targets_survival_candidate_but_ignores_creative_candidate() {
        let player = PlayerCandidate {
            id: 7,
            position: DVec3::new(8.5, 1.0, 4.5),
            eye_height: 1.62,
            main_hand_cow_food: false,
            offhand_cow_food: false,
            main_hand_pig_food: false,
            offhand_pig_food: false,
            main_hand_chicken_food: false,
            offhand_chicken_food: false,
            main_hand_carrot_on_a_stick: false,
            offhand_carrot_on_a_stick: false,
            main_hand_wolf_interest: false,
            offhand_wolf_interest: false,
            main_hand_horse_tempt: false,
            offhand_horse_tempt: false,
            alive: true,
            spectator: false,
            attackable: false,
        };
        for attackable in [false, true] {
            let mut world = EntityWorld::default();
            let id = world.spawn_zombie_pursuit(Zombie::new(DVec3::new(2.5, 1.0, 4.5)));
            world.zombie_mut(id).unwrap().set_random_seed(59);
            let mut player = player;
            player.attackable = attackable;
            world.tick_with_players(&mut EmptyWorld, &[player]);
            assert_eq!(world.zombies()[0].target_player_id, attackable.then_some(7));
        }
    }

    #[test]
    fn ray_selects_nearest_cow_within_reach() {
        let mut world = EntityWorld::default();
        let near = world.spawn_cow(Cow::new(DVec3::new(0.0, 1.0, 3.0)), true);
        world.spawn_cow(Cow::new(DVec3::new(0.0, 1.0, 6.0)), true);
        assert_eq!(
            world
                .cow_on_ray(DVec3::new(0.0, 2.0, 0.0), DVec3::Z, 5.0)
                .unwrap()
                .0,
            near
        );
        assert!(world
            .cow_on_ray(DVec3::new(2.0, 2.0, 0.0), DVec3::Z, 5.0)
            .is_none());
    }

    #[test]
    fn mob_ray_selects_nearest_species_and_spawn_order_for_ties() {
        let mut world = EntityWorld::default();
        let back = world.spawn_cow(Cow::new(DVec3::new(0.0, 1.0, 4.0)), true);
        let front = world.spawn_sheep_no_ai(Sheep::default(), DVec3::new(0.0, 1.0, 3.0));
        assert_eq!(
            world
                .mob_on_ray(DVec3::new(0.0, 2.0, 0.0), DVec3::Z, 5.0)
                .unwrap()
                .0,
            MobHit::Sheep(front)
        );
        world.sheep_mut(front).unwrap().health = 0.0;
        assert_eq!(
            world
                .mob_on_ray(DVec3::new(0.0, 2.0, 0.0), DVec3::Z, 5.0)
                .unwrap()
                .0,
            MobHit::Cow(back)
        );
    }

    #[test]
    fn mob_ray_targets_living_chicken_and_ignores_dead_one() {
        let mut world = EntityWorld::default();
        let chicken = world.spawn_chicken(Chicken::new(DVec3::new(0.0, 1.0, 2.0)), true);
        let cow = world.spawn_cow(Cow::new(DVec3::new(0.0, 1.0, 4.0)), true);
        let eye = DVec3::new(0.0, 1.35, 0.0);
        assert_eq!(
            world.mob_on_ray(eye, DVec3::Z, 5.0).unwrap().0,
            MobHit::Chicken(chicken)
        );
        world.chicken_mut(chicken).unwrap().chicken.health = 0.0;
        assert_eq!(
            world.mob_on_ray(eye, DVec3::Z, 5.0).unwrap().0,
            MobHit::Cow(cow)
        );
    }

    #[test]
    fn skeleton_participates_in_shared_ray_and_dead_cleanup() {
        let mut world = EntityWorld::default();
        let skeleton = world.spawn_skeleton(Skeleton::new(DVec3::new(0.0, 1.0, 2.0)), true);
        let zombie = world.spawn_zombie(Zombie::new(DVec3::new(0.0, 1.0, 4.0)), true);
        let eye = DVec3::new(0.0, 2.0, 0.0);
        assert_eq!(
            world.mob_on_ray(eye, DVec3::Z, 5.0).unwrap().0,
            MobHit::Skeleton(skeleton)
        );
        assert!(world.skeleton_mut(skeleton).unwrap().hurt(21.0).died);
        assert_eq!(
            world.mob_on_ray(eye, DVec3::Z, 5.0).unwrap().0,
            MobHit::Zombie(zombie)
        );
        for _ in 0..21 {
            world.tick(&mut EmptyWorld);
        }
        assert!(world.skeletons().is_empty());
    }

    #[test]
    fn bat_hit_wakes_and_ray_skips_dead_bat() {
        let mut world = EntityWorld::default();
        let bat = world.spawn_bat(Bat::new(DVec3::new(0.0, 1.0, 2.0)), true);
        let eye = DVec3::new(0.0, 1.45, 0.0);
        assert_eq!(
            world.mob_on_ray(eye, DVec3::Z, 5.0).unwrap().0,
            MobHit::Bat(bat)
        );
        let entity = world.bat_mut(bat).unwrap();
        assert!(entity.bat.resting);
        assert!(entity.hurt(7.0).died);
        assert!(!entity.bat.resting);
        assert!(world.mob_on_ray(eye, DVec3::Z, 5.0).is_none());
    }

    #[test]
    fn zombie_ray_uses_body_and_skips_dead_zombie() {
        let mut world = EntityWorld::default();
        let zombie = world.spawn_zombie(Zombie::new(DVec3::new(0.0, 1.0, 2.0)), true);
        let eye = DVec3::new(0.0, 1.74, 0.0);
        assert_eq!(
            world.mob_on_ray(eye, DVec3::Z, 5.0).unwrap().0,
            MobHit::Zombie(zombie)
        );
        assert!(world.zombie_mut(zombie).unwrap().hurt(21.0).died);
        assert!(world.mob_on_ray(eye, DVec3::Z, 5.0).is_none());
    }

    #[test]
    fn villager_ray_uses_body_and_skips_dead_villager() {
        let mut world = EntityWorld::default();
        let villager = world.spawn_villager(Villager::new(DVec3::new(0.0, 1.0, 2.0)), true);
        let eye = DVec3::new(0.0, 1.62, 0.0);
        assert_eq!(
            world.mob_on_ray(eye, DVec3::Z, 5.0).unwrap().0,
            MobHit::Villager(villager)
        );
        assert!(world.villager_mut(villager).unwrap().hurt(21.0).died);
        assert!(world.mob_on_ray(eye, DVec3::Z, 5.0).is_none());
    }

    #[test]
    fn mooshroom_keeps_its_species_in_shared_cow_ray_order() {
        let mut world = EntityWorld::default();
        let mooshroom = world.spawn_mooshroom(
            MushroomCow::new(DVec3::new(0.0, 1.0, 2.0), MushroomVariant::Brown),
            true,
        );
        let cow = world.spawn_cow(Cow::new(DVec3::new(0.0, 1.0, 4.0)), true);
        let eye = DVec3::new(0.0, 2.0, 0.0);
        assert_eq!(
            world.mob_on_ray(eye, DVec3::Z, 5.0).unwrap().0,
            MobHit::Mooshroom(mooshroom)
        );
        assert_eq!(world.cow_on_ray(eye, DVec3::Z, 5.0).unwrap().0, cow);
        world.mooshroom_mut(mooshroom).unwrap().cow.health = 0.0;
        assert_eq!(
            world.mob_on_ray(eye, DVec3::Z, 5.0).unwrap().0,
            MobHit::Cow(cow)
        );
    }

    #[test]
    fn shearing_converts_only_adult_mooshrooms_to_cows() {
        let mut world = EntityWorld::default();
        let adult = world.spawn_mooshroom(
            MushroomCow::new(DVec3::new(0.0, 1.0, 2.0), MushroomVariant::Brown),
            true,
        );
        let baby = world.spawn_mooshroom(
            MushroomCow::new(DVec3::new(0.0, 1.0, 4.0), MushroomVariant::Red),
            true,
        );
        world.mooshroom_mut(baby).unwrap().cow.age.ticks = -1200;
        assert!(world.shear_mooshroom(baby).is_none());
        let shearing = world.shear_mooshroom(adult).unwrap();
        assert_eq!(shearing.drop_item, "minecraft:brown_mushroom");
        assert_eq!((shearing.drop_count, shearing.tool_damage), (5, 1));
        assert!(world.mooshroom_mut(adult).is_none());
        assert!(world.shear_mooshroom(adult).is_none());
        assert!(world.mooshroom_mut(baby).is_some());
        assert_eq!(
            world
                .mob_on_ray(DVec3::new(0.0, 2.0, 0.0), DVec3::Z, 5.0)
                .unwrap()
                .0,
            MobHit::Cow(adult)
        );
    }

    #[test]
    fn no_ai_still_ages_without_moving() {
        let mut world = EntityWorld::default();
        let id = world.spawn_cow(Cow::new(DVec3::new(0.5, 3.0, 0.5)), true);
        world.cow_mut(id).unwrap().cow.age.ticks = -20;
        world.tick(&mut EmptyWorld);
        let cow = &world.cows()[0];
        assert_eq!(cow.cow.age.ticks, -19);
        assert_eq!(cow.cow.body.position, DVec3::new(0.5, 3.0, 0.5));
    }
}
