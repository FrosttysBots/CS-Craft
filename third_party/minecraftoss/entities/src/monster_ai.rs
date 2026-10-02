//! Monsters on 26.3's goal framework, over one shared context: the
//! creeper's goals (`Creeper.registerGoals`: float, swell, avoid ocelots and
//! cats, the melee approach, a water-avoiding stroll, looking at players and
//! around), the zombie's (`Zombie.registerGoals`/`addBehaviourGoals`:
//! the turtle-egg goal, looking, the spear goal, `ZombieAttackGoal`, moving
//! through villages, the stroll), the skeleton's and the spider's
//! (`Spider.registerGoals`: float, avoid armadillos, `LeapAtTargetGoal`, the
//! melee attack that gives up in the light, the stroll, looking; targets
//! only in the dark), with their target selectors
//! (`HurtByTargetGoal`, `NearestAttackableTargetGoal` for players,
//! villagers, iron golems and baby turtles, through `TargetGoal` and
//! `TargetingConditions`). Targets are players or villagers. Navigation and
//! the look control live in the context while the goals run, so paths are
//! created when vanilla creates them. Sources: the goal classes named here,
//! `Mob.getTarget`/`asValidTarget`, `Mob.getMaxFallDistance`,
//! `Creeper.getMaxFallDistance` and `Mob.serverAiStep` in the pinned 26.3
//! common JAR.
use crate::control::MoveControl;
use crate::enderman::PlayerView;
use crate::fluid::FluidFrame;
use crate::goals::{Control, Controls, Goal, GoalSelector};
use crate::look::{BodyRotation, LookAtPlayerState, LookControl, RandomLookState};
use crate::movement::Body;
use crate::navigation::{navigate_walk_to, navigate_walk_to_entity, plan_walk_path, GroundNavigation, PlannedPath};
use crate::sight::line_of_sight;
use crate::stroll::StrollState;
use crate::tempt::PlayerCandidate;
use crate::walk_path::{WalkProfile, WalkTarget};
use glam::DVec3;
use minecraftoss_player::path_type::PathType;
use minecraftoss_player::rng::LegacyRandom;
use minecraftoss_player::World;

/// Which monster the context drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonsterKind {
    Creeper,
    Zombie,
    Skeleton,
    Spider,
    /// `Slime` (`AbstractCubeMob`'s goals over its cube move control).
    Slime,
    /// `Enderman`: a neutral mob that the stare of a player sets off.
    Enderman,
    /// `Witch`: a raider that throws potions from range.
    Witch,
    /// `IronGolem`: a village's guard (`golem_ai`).
    IronGolem,
    /// `Wolf`: a tamable animal that hunts (`wolf_ai`).
    Wolf,
}

impl MonsterKind {
    /// The running goals' class names, by registration.
    pub fn goal_names(self) -> &'static [&'static str] {
        match self {
            Self::Creeper => &CREEPER_GOAL_NAMES,
            Self::Zombie => &ZOMBIE_GOAL_NAMES,
            Self::Skeleton => &SKELETON_GOAL_NAMES,
            Self::Spider => &SPIDER_GOAL_NAMES,
            Self::Slime => &SLIME_GOAL_NAMES,
            Self::Enderman => &ENDERMAN_GOAL_NAMES,
            Self::Witch => &WITCH_GOAL_NAMES,
            Self::IronGolem => &crate::golem_ai::GOAL_NAMES,
            Self::Wolf => &crate::wolf_ai::GOAL_NAMES,
        }
    }

    pub fn target_goal_names(self) -> &'static [&'static str] {
        match self {
            Self::Creeper => &CREEPER_TARGET_NAMES,
            Self::Zombie => &ZOMBIE_TARGET_NAMES,
            Self::Skeleton => &SKELETON_TARGET_NAMES,
            Self::Spider => &SPIDER_TARGET_NAMES,
            Self::Slime => &SLIME_TARGET_NAMES,
            Self::Enderman => &ENDERMAN_TARGET_NAMES,
            Self::Witch => &WITCH_TARGET_NAMES,
            Self::IronGolem => &crate::golem_ai::TARGET_NAMES,
            Self::Wolf => &crate::wolf_ai::TARGET_NAMES,
        }
    }
}

pub const CREEPER_GOAL_NAMES: [&str; 8] = [
    "FloatGoal",
    "SwellGoal",
    "AvoidEntityGoal",
    "AvoidEntityGoal",
    "MeleeAttackGoal",
    "WaterAvoidingRandomStrollGoal",
    "LookAtPlayerGoal",
    "RandomLookAroundGoal",
];
pub const CREEPER_TARGET_NAMES: [&str; 2] = ["NearestAttackableTargetGoal", "HurtByTargetGoal"];
pub const ZOMBIE_GOAL_NAMES: [&str; 7] = [
    "ZombieAttackTurtleEggGoal",
    "LookAtPlayerGoal",
    "RandomLookAroundGoal",
    "SpearUseGoal",
    "ZombieAttackGoal",
    "MoveThroughVillageGoal",
    "WaterAvoidingRandomStrollGoal",
];
pub const ZOMBIE_TARGET_NAMES: [&str; 5] = [
    "HurtByTargetGoal",
    "NearestAttackableTargetGoal",
    "NearestAttackableTargetGoal",
    "NearestAttackableTargetGoal",
    "NearestAttackableTargetGoal",
];

pub const SKELETON_GOAL_NAMES: [&str; 7] = [
    "RestrictSunGoal",
    "FleeSunGoal",
    "AvoidEntityGoal",
    "WaterAvoidingRandomStrollGoal",
    "LookAtPlayerGoal",
    "RandomLookAroundGoal",
    "RangedBowAttackGoal",
];
pub const SKELETON_TARGET_NAMES: [&str; 4] = [
    "HurtByTargetGoal",
    "NearestAttackableTargetGoal",
    "NearestAttackableTargetGoal",
    "NearestAttackableTargetGoal",
];

pub const SPIDER_GOAL_NAMES: [&str; 7] = [
    "FloatGoal",
    "AvoidEntityGoal",
    "LeapAtTargetGoal",
    "SpiderAttackGoal",
    "WaterAvoidingRandomStrollGoal",
    "LookAtPlayerGoal",
    "RandomLookAroundGoal",
];
pub const SPIDER_TARGET_NAMES: [&str; 3] = ["HurtByTargetGoal", "SpiderTargetGoal", "SpiderTargetGoal"];

/// `AbstractCubeMob.registerGoals`, then `Slime.addBehaviourGoals`.
pub const SLIME_GOAL_NAMES: [&str; 4] = ["CubeMobFloatGoal", "CubeMobRandomDirectionGoal", "CubeMobKeepOnJumpingGoal", "CubeMobAttackGoal"];
pub const SLIME_TARGET_NAMES: [&str; 2] = ["NearestAttackableTargetGoal", "NearestAttackableTargetGoal"];
pub const ENDERMAN_GOAL_NAMES: [&str; 8] = [
    "FloatGoal",
    "EndermanFreezeWhenLookedAt",
    "MeleeAttackGoal",
    "WaterAvoidingRandomStrollGoal",
    "LookAtPlayerGoal",
    "RandomLookAroundGoal",
    "EndermanLeaveBlockGoal",
    "EndermanTakeBlockGoal",
];
pub const ENDERMAN_TARGET_NAMES: [&str; 4] = ["EndermanLookForPlayerGoal", "HurtByTargetGoal", "NearestAttackableTargetGoal", "ResetUniversalAngerTargetGoal"];
/// `PatrollingMonster.registerGoals`, `Raider.registerGoals`, then
/// `Witch.registerGoals`.
pub const WITCH_GOAL_NAMES: [&str; 10] = [
    "LongDistancePatrolGoal",
    "ObtainRaidLeaderBannerGoal",
    "PathfindToRaidGoal",
    "RaiderMoveThroughVillageGoal",
    "RaiderCelebration",
    "FloatGoal",
    "RangedAttackGoal",
    "WaterAvoidingRandomStrollGoal",
    "LookAtPlayerGoal",
    "RandomLookAroundGoal",
];
pub const WITCH_TARGET_NAMES: [&str; 3] = ["HurtByTargetGoal", "NearestHealableRaiderTargetGoal", "NearestAttackableWitchTargetGoal"];

/// A witch's own state for its goals.
#[derive(Clone, Debug)]
pub struct WitchState {
    /// `isDrinkingPotion`: a ranged attack throws nothing.
    pub drinking: bool,
    /// `RangedAttackGoal.target`, `attackTime` and `seeTime`.
    pub ranged_target: Option<Target>,
    pub attack_time: i32,
    pub see_time: i32,
    /// The potion thrown this tick and the throw sound's pitch.
    pub throw: Option<(crate::witch::Throw, f32)>,
}

impl Default for WitchState {
    fn default() -> Self {
        Self { drinking: false, ranged_target: None, attack_time: -1, see_time: 0, throw: None }
    }
}

/// What a monster sees of a player besides where it is: health, effects
/// and motion (a witch picks its potion by them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerVitals {
    pub health: f32,
    pub velocity: DVec3,
    pub slowed: bool,
    pub poisoned: bool,
    pub weakened: bool,
}

impl Default for PlayerVitals {
    fn default() -> Self {
        Self { health: 20.0, velocity: DVec3::ZERO, slowed: false, poisoned: false, weakened: false }
    }
}

/// An enderman's own state (`Enderman`'s synced data and fields, and
/// `NeutralMob`'s anger).
#[derive(Clone, Debug)]
pub struct EndermanState {
    /// `DATA_CARRY_STATE`.
    pub carried: Option<minecraftoss_player::Block>,
    /// `DATA_CREEPY`: it has a target, and screams.
    pub creepy: bool,
    /// `DATA_STARED_AT`: a stare set it off.
    pub stared_at: bool,
    /// `targetChangeTime`: its tick count when it last took a target.
    pub target_change_time: i32,
    /// `persistentAngerEndTime` (game time; -1 for none).
    pub anger_end_time: i64,
    /// `persistentAngerTarget`.
    pub anger_target: Option<Target>,
    /// Where each teleport this tick landed.
    pub teleports: Vec<DVec3>,
    /// A block the goals took or put down this tick, for the world to
    /// change (none for air).
    pub block_change: Option<((i32, i32, i32), Option<minecraftoss_player::Block>)>,
}

impl Default for EndermanState {
    fn default() -> Self {
        Self {
            carried: None,
            creepy: false,
            stared_at: false,
            target_change_time: 0,
            anger_end_time: -1,
            anger_target: None,
            teleports: Vec::new(),
            block_change: None,
        }
    }
}

/// `AbstractCubeMob.CubeMobMoveControl`: the facing it turns to, the
/// countdown to its next hop, and a hop's speed while the goals ask for
/// one (`MoveControl.Operation.MOVE_TO`).
#[derive(Clone, Copy, Debug, Default)]
pub struct CubeMove {
    pub y_rot: f32,
    pub jump_delay: i32,
    pub aggressive: bool,
    pub speed_modifier: f64,
    pub move_to: bool,
}

impl CubeMove {
    pub fn set_direction(&mut self, y_rot: f32, aggressive: bool) {
        self.y_rot = y_rot;
        self.aggressive = aggressive;
    }

    pub fn set_wanted_movement(&mut self, speed_modifier: f64) {
        self.speed_modifier = speed_modifier;
        self.move_to = true;
    }
}

/// `Monster.createMonsterAttributes`: `FOLLOW_RANGE`'s base.
pub const FOLLOW_RANGE: f64 = 16.0;
/// `Zombie.createAttributes`: `FOLLOW_RANGE`'s base.
pub const ZOMBIE_FOLLOW_RANGE: f64 = 35.0;
/// `Creeper.createAttributes`: `MOVEMENT_SPEED`.
pub const MOVEMENT_SPEED: f64 = 0.25;

/// A mob a monster may target besides players (a villager, an iron golem,
/// a monster that hurt it).
#[derive(Clone, Copy, Debug)]
pub struct MobCandidate {
    pub id: u64,
    pub position: DVec3,
    pub eye_height: f32,
    pub width: f32,
    pub height: f32,
    pub alive: bool,
    /// Its entity type (`minecraft:villager` for the villager list).
    pub kind: &'static str,
}

/// A monster's target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Player(u64),
    Villager(u64),
    /// Any other mob: an iron golem, or a monster a golem hunts.
    Mob(u64),
}

/// `Enemy`: the monsters here (and slimes), which iron golems hunt all
/// but creepers of.
pub fn is_enemy(kind: &str) -> bool {
    matches!(
        kind,
        "minecraft:zombie"
            | "minecraft:husk"
            | "minecraft:drowned"
            | "minecraft:zombie_villager"
            | "minecraft:skeleton"
            | "minecraft:stray"
            | "minecraft:bogged"
            | "minecraft:parched"
            | "minecraft:creeper"
            | "minecraft:spider"
            | "minecraft:slime"
            | "minecraft:enderman"
            | "minecraft:witch"
    )
}

/// A target as the goals see it.
#[derive(Clone, Copy, Debug)]
pub struct TargetInfo {
    pub target: Target,
    pub position: DVec3,
    pub eye_height: f32,
    pub width: f32,
    pub height: f32,
    pub alive: bool,
    /// For players: attackable (not creative) and not spectating.
    player_ok: bool,
}

/// What the target selectors look for in `NearestAttackableTargetGoal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Prey {
    Player,
    Villager,
    /// Baby turtles on land (and endermites): none live here, so the goals
    /// only roll their interval.
    Absent,
    /// Iron golems.
    Golem,
    /// The iron golem's: a player it is angry at (`NeutralMob.isAngryAt`).
    AngryPlayer,
    /// The iron golem's: an `Enemy` other than a creeper.
    Enemy,
    /// The wolf's prey (`Wolf.PREY_SELECTOR`): sheep, rabbits and foxes.
    WolfPrey,
    /// `AbstractSkeleton`s.
    Skeleton,
}

/// `MeleeAttackGoal`'s state (and `ZombieAttackGoal`'s raised arms).
#[derive(Clone, Debug, Default)]
pub struct MeleeState {
    last_can_use_check: i64,
    /// The path `canUse` created, for `start` to follow.
    path: Option<PlannedPath>,
    pathed_target: DVec3,
    ticks_until_recalculation: i32,
    ticks_until_attack: i32,
    raise_arm_ticks: i32,
    /// `Mob.isAggressive` (a zombie raises its arms).
    pub aggressive: bool,
    /// Attacks swung (`swingForAttack`), each a swing packet for clients.
    pub swings: u32,
}

/// `RangedBowAttackGoal`'s state, with the bow's draw (`LivingEntity`'s
/// item use: `isUsingItem`, `getTicksUsingItem`).
#[derive(Clone, Debug)]
pub struct BowState {
    pub attack_interval: i32,
    pub attack_time: i32,
    pub see_time: i32,
    pub strafing_time: i32,
    pub strafing_clockwise: bool,
    pub strafing_backwards: bool,
    pub using_item: bool,
    pub ticks_using_item: i32,
}

impl Default for BowState {
    fn default() -> Self {
        // `AbstractSkeleton.getAttackInterval`: 40 ticks (20 on hard).
        Self { attack_interval: 40, attack_time: -1, see_time: 0, strafing_time: -1, strafing_clockwise: false, strafing_backwards: false, using_item: false, ticks_using_item: 0 }
    }
}

/// What the monster's goals read and change for one tick.
#[derive(Clone)]
pub struct MonsterGoalContext {
    pub kind: MonsterKind,
    pub body: Body,
    pub eye_height: f32,
    pub health: f32,
    pub max_health: f32,
    pub game_time: i64,
    /// `Difficulty.getId` (0 is peaceful: `canAttack` refuses players).
    pub difficulty: i32,
    pub mob_griefing: bool,
    /// `Level.isBrightOutside`.
    pub bright_outside: bool,
    /// `FOLLOW_RANGE` with its modifiers: how far it looks for and keeps
    /// a target, and its longest path.
    pub follow_range: f64,
    pub players: Vec<PlayerCandidate>,
    pub villagers: Vec<MobCandidate>,
    /// The other mobs its targets may be: iron golems and monsters.
    pub mobs: Vec<MobCandidate>,
    /// `Mob.target` as last set; [`MonsterGoalContext::target`] filters it
    /// as `getTarget` does.
    pub target: Option<Target>,
    /// Who last hurt it (a player or a mob) and when, in its ticks
    /// (`getLastHurtByMob`, `getLastHurtByMobTimestamp`).
    pub hurt_by: Option<(Target, i32)>,
    pub fluid: FluidFrame,
    pub walk: WalkProfile,
    pub no_action_time: i32,
    /// The creeper's fuse direction.
    pub swell_dir: i32,
    pub navigation: GroundNavigation,
    pub look_control: LookControl,
    /// The jump control's request (`FloatGoal`).
    pub jump: bool,
    pub random: LegacyRandom,
    pub stroll: StrollState,
    pub look_at_player: LookAtPlayerState,
    pub random_look: RandomLookState,
    pub melee: MeleeState,
    /// `RemoveBlockGoal.nextStartTick` for the turtle-egg goal.
    pub turtle_egg_next_start: i32,
    /// A melee hit to land this tick (`Mob.doHurtTarget`).
    pub attack: Option<Target>,
    /// `HurtByTargetGoal.alertOthers`: the attacker to set on nearby
    /// monsters of its kind with no target.
    pub alert: Option<Target>,
    /// An iron golem's hit this tick: its damage, drawn as it struck
    /// (`IronGolem.doHurtTarget`).
    pub attack_damage: Option<f32>,
    /// `Sensing`: sight of each target, worked out once per tick.
    pub seen: Vec<(Target, bool)>,
    /// The body's facing (`yRot`), which `Mob.lookAt` turns directly.
    pub yaw: f32,
    /// A strafe for the move control (`MoveControl.strafe`).
    pub strafe: Option<(f32, f32)>,
    /// An arrow to loose at a target with a bow's power (`performRangedAttack`).
    pub shoot: Option<(Target, f32)>,
    /// Burning (`isOnFire`).
    pub on_fire: bool,
    /// Something is worn on the head (sun protection).
    pub helmet: bool,
    pub bow: BowState,
    /// A velocity a goal set directly this tick (`LeapAtTargetGoal`).
    pub leap: Option<DVec3>,
    /// A cube mob's move control, which its goals steer.
    pub cube: CubeMove,
    /// A cube mob of size 1 (`isTiny`): it deals no damage.
    pub tiny: bool,
    /// Its tick count (`tickCount`).
    pub tick_count: i32,
    /// Where the players look.
    pub views: Vec<(u64, PlayerView)>,
    pub enderman: EndermanState,
    /// The players' health, effects and motion.
    pub vitals: Vec<(u64, PlayerVitals)>,
    pub witch: WitchState,
    pub golem: crate::golem_ai::GolemState,
    /// A wolf's own state (`wolf_ai`).
    pub wolf: crate::wolf_ai::WolfState,
    /// The level's points of interest and random, lent while it ticks (to
    /// iron golems and zombies).
    pub pois: Option<crate::poi::PoiManager>,
    pub level_random: Option<LegacyRandom>,
    /// `Zombie.canBreakDoors`: the doors `MoveThroughVillageGoal` may path
    /// through.
    pub can_break_doors: bool,
}

impl MonsterGoalContext {
    fn player(&self, id: u64) -> Option<PlayerCandidate> {
        self.players.iter().copied().find(|p| p.id == id)
    }

    /// A target's current state, if it is still in the world.
    pub fn info(&self, target: Target) -> Option<TargetInfo> {
        match target {
            Target::Player(id) => self.player(id).map(|p| TargetInfo {
                target,
                position: p.position,
                eye_height: p.eye_height,
                width: 0.6,
                height: 1.8,
                alive: p.alive,
                player_ok: p.attackable && !p.spectator,
            }),
            Target::Villager(id) => self.villagers.iter().find(|v| v.id == id).map(|v| TargetInfo {
                target,
                position: v.position,
                eye_height: v.eye_height,
                width: v.width,
                height: v.height,
                alive: v.alive,
                player_ok: true,
            }),
            Target::Mob(id) => self.mobs.iter().find(|m| m.id == id).map(|m| TargetInfo {
                target,
                position: m.position,
                eye_height: m.eye_height,
                width: m.width,
                height: m.height,
                alive: m.alive,
                player_ok: true,
            }),
        }
    }

    /// A mob target's entity type.
    pub(crate) fn kind_of(&self, target: Target) -> Option<&'static str> {
        match target {
            Target::Mob(id) => self.mobs.iter().find(|m| m.id == id).map(|m| m.kind),
            Target::Villager(_) => Some("minecraft:villager"),
            Target::Player(_) => Some("minecraft:player"),
        }
    }

    pub(crate) fn eye(&self) -> DVec3 {
        self.body.position + DVec3::new(0.0, f64::from(self.eye_height), 0.0)
    }

    /// `getLightLevelDependentMagicValue() >= 0.5` at the eyes' block: too
    /// light for a spider.
    fn in_light(&self, world: &dyn World) -> bool {
        let eye = self.eye();
        world.light_path_cost((eye.x.floor() as i32, eye.y.floor() as i32, eye.z.floor() as i32)) >= 0.0
    }

    /// `Sensing.hasLineOfSight`, cached for the tick.
    pub(crate) fn sees(&mut self, world: &dyn World, target: TargetInfo) -> bool {
        if let Some(&(_, seen)) = self.seen.iter().find(|(t, _)| *t == target.target) {
            return seen;
        }
        let seen = line_of_sight(world, self.eye(), target.position + DVec3::new(0.0, f64::from(target.eye_height), 0.0));
        self.seen.push((target.target, seen));
        seen
    }

    /// `Mob.canAttack`: a living target that can be an enemy; players not in
    /// peaceful and not invulnerable (creative) or spectating. `asValidTarget`
    /// asks the same.
    pub(crate) fn can_attack(&self, target: TargetInfo) -> bool {
        let player = matches!(target.target, Target::Player(_));
        // `IronGolem.canAttack`: a golem a player built spares players, and
        // none goes for creepers.
        if self.kind == MonsterKind::IronGolem && ((player && self.golem.player_created) || self.kind_of(target.target) == Some("minecraft:creeper")) {
            return false;
        }
        // `TamableAnimal.canAttack`: never its owner.
        if self.kind == MonsterKind::Wolf && matches!(target.target, Target::Player(id) if self.wolf.owner == Some(id)) {
            return false;
        }
        target.alive && target.player_ok && !(player && self.difficulty == 0)
    }

    /// `Mob.getTarget`: the target while it is still a valid one.
    pub fn target(&self) -> Option<TargetInfo> {
        self.target.and_then(|t| self.info(t)).filter(|&t| self.can_attack(t))
    }

    /// `Mob.getMaxHeadXRot`: 40 degrees, a sitting wolf's 20.
    pub(crate) fn max_head_x_rot(&self) -> f32 {
        if self.kind == MonsterKind::Wolf && self.wolf.sitting {
            20.0
        } else {
            40.0
        }
    }

    /// `Mob.setTarget` through `asValidTarget`.
    pub(crate) fn set_target(&mut self, target: Option<Target>) {
        self.target = target.filter(|&t| self.info(t).is_some_and(|info| self.can_attack(info)));
        self.target_set(target.is_some());
    }

    /// `Mob.setTarget(null)`.
    pub(crate) fn clear_target(&mut self) {
        self.target = None;
        self.target_set(false);
    }

    /// `Enderman.setTarget`'s side, on every call: a target (again) marks
    /// the time and makes it creepy; none calms it.
    fn target_set(&mut self, some: bool) {
        if self.kind != MonsterKind::Enderman {
            return;
        }
        let state = &mut self.enderman;
        if some {
            state.target_change_time = self.tick_count;
            state.creepy = true;
        } else {
            state.target_change_time = 0;
            state.creepy = false;
            state.stared_at = false;
        }
    }

    /// Where a player looks (level, south, bare-headed when unknown).
    fn view(&self, id: u64) -> PlayerView {
        self.views.iter().find(|(p, _)| *p == id).map_or_else(PlayerView::default, |&(_, view)| view)
    }

    /// `Enderman.isBeingStaredBy`.
    pub(crate) fn stared_by(&self, world: &dyn World, player: u64) -> bool {
        let Some(p) = self.player(player) else { return false };
        let view = self.view(player);
        let gaze = self.body.position.y + f64::from(self.eye_height);
        !view.disguised && crate::enderman::is_looking_at_me(world, p.position, p.eye_height, view, self.body.position, gaze)
    }

    /// `NeutralMob.isAngryAt` (universal anger is off).
    fn angry_at(&self, target: TargetInfo) -> bool {
        self.can_attack(target) && self.enderman.anger_target == Some(target.target)
    }

    /// `NeutralMob.isAngry`.
    pub(crate) fn angry(&self) -> bool {
        self.enderman.anger_end_time > 0 && self.enderman.anger_end_time - self.game_time > 0
    }

    /// `Enderman.teleport()` from the goals (`randomTeleport` stops the
    /// navigation).
    pub(crate) fn teleport(&mut self, world: &dyn World) -> bool {
        let moved = crate::enderman::teleport(&mut self.body, &mut self.random, world);
        if moved {
            self.navigation.stop();
            self.enderman.teleports.push(self.body.position);
        }
        moved
    }

    /// `Enderman.teleportTowards`.
    fn teleport_towards(&mut self, world: &dyn World, target: TargetInfo) -> bool {
        let eye = target.position.y + f64::from(target.eye_height);
        let moved = crate::enderman::teleport_towards(&mut self.body, &mut self.random, world, target.position, eye);
        if moved {
            self.navigation.stop();
            self.enderman.teleports.push(self.body.position);
        }
        moved
    }

    /// `NeutralMob.stopBeingAngry`.
    fn stop_being_angry(&mut self) {
        self.hurt_by = None;
        self.enderman.anger_target = None;
        self.clear_target();
        self.enderman.anger_end_time = -1;
    }

    /// The walk profile with the fall a path may take: three blocks, or
    /// with a target more (`Creeper.getMaxFallDistance`: as far as it can
    /// fall and keep one health; `Mob.getMaxFallDistance`: what health above
    /// a third it can spare, less four per difficulty step below hard).
    pub(crate) fn profile(&self) -> WalkProfile {
        let mut profile = self.walk.clone();
        if self.follow_range != self.walk.follow_range as f64 {
            profile.max_path_length = Some(self.follow_range as f32);
        }
        let allowed = match (self.kind, self.target().is_some()) {
            (_, false) => 0.0,
            (MonsterKind::Creeper, true) => self.health - 1.0,
            (MonsterKind::Zombie | MonsterKind::Skeleton | MonsterKind::Spider | MonsterKind::Slime | MonsterKind::Enderman | MonsterKind::Witch | MonsterKind::IronGolem | MonsterKind::Wolf, true) => {
                let sacrifice = (self.health - self.max_health * 0.33) as i32 - (3 - self.difficulty) * 4;
                sacrifice.max(0) as f32
            }
        };
        profile.max_fall_distance = (allowed + 3.0).floor() as i32;
        profile
    }

    /// `Mob.isWithinMeleeAttackRange`: the reach-inflated box meets the
    /// target's.
    pub(crate) fn within_melee_range(&self, target: TargetInfo) -> bool {
        let reach = f64::from(2.04_f32).sqrt() - f64::from(0.6_f32);
        let half = f64::from(self.body.width / 2.0);
        let p = self.body.position;
        let (min_x, max_x) = (p.x - half - reach, p.x + half + reach);
        let (min_z, max_z) = (p.z - half - reach, p.z + half + reach);
        let (min_y, max_y) = (p.y, p.y + f64::from(self.body.height));
        let t = target.position;
        let target_half = f64::from(target.width / 2.0);
        min_x < t.x + target_half
            && max_x > t.x - target_half
            && min_y < t.y + f64::from(target.height)
            && max_y > t.y
            && min_z < t.z + target_half
            && max_z > t.z - target_half
    }
}

/// A monster's AI between ticks: its goal and target selectors and the
/// state they drive.
#[derive(Clone)]
pub struct MonsterAi {
    goals: GoalSelector<MonsterGoalContext>,
    targets: GoalSelector<MonsterGoalContext>,
    pub state: MonsterGoalContext,
    pub move_control: MoveControl,
    pub body_rotation: BodyRotation,
    pub yaw: f32,
    pub speed: f32,
    pub forward: f32,
    pub sideways: f32,
    pub no_jump_delay: i32,
    /// `LivingEntity.jumping` as the jump control last set it.
    pub jumping: bool,
}

/// The creeper's AI (the name the creeper code and gates use).
pub type CreeperAi = MonsterAi;

impl MonsterAi {
    /// A creeper's goals over its body.
    pub fn new(body: &Body, yaw: f32) -> Self {
        Self::of_kind(MonsterKind::Creeper, body, yaw)
    }

    pub fn of_kind(kind: MonsterKind, body: &Body, yaw: f32) -> Self {
        // `Monster`: none of an animal's fire maluses; the creeper's
        // `FloatGoal` lets its navigation float; it walks towards darkness.
        let mut walk = WalkProfile::animal(body.width, body.height);
        if kind != MonsterKind::Wolf {
            walk.clear_malus(PathType::FireInNeighbor);
            walk.clear_malus(PathType::Fire);
        }
        // `FloatGoal` lets the navigation float.
        walk.can_float = matches!(kind, MonsterKind::Creeper | MonsterKind::Spider | MonsterKind::Enderman | MonsterKind::Witch | MonsterKind::Wolf);
        let follow_range = match kind {
            MonsterKind::Creeper | MonsterKind::Skeleton | MonsterKind::Spider | MonsterKind::Slime | MonsterKind::Witch | MonsterKind::IronGolem => FOLLOW_RANGE,
            MonsterKind::Zombie => ZOMBIE_FOLLOW_RANGE,
            MonsterKind::Enderman => crate::enderman::FOLLOW_RANGE,
            MonsterKind::Wolf => crate::wolf::FOLLOW_RANGE,
        };
        walk.follow_range = follow_range as f32;
        // An `Animal` keeps its own walk values (grass is best).
        if kind != MonsterKind::Wolf {
            walk.walk_target = WalkTarget::Monster;
        }
        if kind == MonsterKind::Wolf {
            // `Wolf`: powder snow is out of bounds.
            walk.set_malus(PathType::PowderSnow, -1.0);
            walk.set_malus(PathType::OnTopOfPowderSnow, -1.0);
        }
        if kind == MonsterKind::Enderman {
            // `setPathfindingMalus(WATER, -1)`, the one-block step, and
            // `getWalkTargetValue` 0.
            walk.set_malus(PathType::Water, -1.0);
            walk.max_up_step = 1.0;
            walk.walk_target = WalkTarget::Zero;
        }
        if kind == MonsterKind::IronGolem {
            // A `PathfinderMob` with a one-block step (`STEP_HEIGHT`) whose
            // spots are all alike (`getWalkTargetValue` 0).
            walk.max_up_step = crate::iron_golem::STEP_HEIGHT;
            walk.walk_target = WalkTarget::Zero;
        }
        let (goals, targets, eye_height) = match kind {
            MonsterKind::Creeper => (creeper_goals(), creeper_targets(), body.height * 0.85),
            MonsterKind::Zombie => (zombie_goals(), zombie_targets(), 1.74),
            MonsterKind::Skeleton => (skeleton_goals(), skeleton_targets(), 1.74),
            MonsterKind::Spider => (spider_goals(), spider_targets(), 0.65),
            MonsterKind::Slime => (slime_goals(), slime_targets(), 0.325),
            MonsterKind::Enderman => (enderman_goals(), enderman_targets(), crate::enderman::EYE_HEIGHT),
            MonsterKind::Witch => (witch_goals(), witch_targets(), crate::witch::EYE_HEIGHT),
            MonsterKind::IronGolem => (crate::golem_ai::golem_goals(), crate::golem_ai::golem_targets(), crate::iron_golem::EYE_HEIGHT),
            MonsterKind::Wolf => (crate::wolf_ai::wolf_goals(), crate::wolf_ai::wolf_targets(), crate::wolf::EYE_HEIGHT),
        };
        let mut navigation = GroundNavigation::default();
        navigation.wall_climber = kind == MonsterKind::Spider;
        Self {
            goals,
            targets,
            state: MonsterGoalContext {
                kind,
                body: body.clone(),
                eye_height,
                health: 20.0,
                max_health: 20.0,
                game_time: 0,
                difficulty: 2,
                mob_griefing: true,
                bright_outside: false,
                follow_range,
                players: Vec::new(),
                villagers: Vec::new(),
                mobs: Vec::new(),
                target: None,
                hurt_by: None,
                fluid: FluidFrame::default(),
                walk,
                no_action_time: 0,
                swell_dir: -1,
                navigation,
                look_control: LookControl::new(yaw),
                jump: false,
                random: LegacyRandom::new(0),
                stroll: StrollState::default(),
                look_at_player: LookAtPlayerState::default(),
                random_look: RandomLookState::default(),
                melee: MeleeState::default(),
                turtle_egg_next_start: 0,
                attack: None,
                alert: None,
                attack_damage: None,
                seen: Vec::new(),
                yaw,
                strafe: None,
                shoot: None,
                on_fire: false,
                helmet: false,
                bow: BowState::default(),
                leap: None,
                cube: CubeMove::default(),
                tiny: false,
                tick_count: 0,
                views: Vec::new(),
                enderman: EndermanState::default(),
                vitals: Vec::new(),
                witch: WitchState::default(),
                golem: Default::default(),
                wolf: Default::default(),
                pois: None,
                level_random: None,
                can_break_doors: false,
            },
            move_control: MoveControl::default(),
            body_rotation: BodyRotation::new(yaw),
            yaw,
            speed: 0.0,
            forward: 0.0,
            sideways: 0.0,
            no_jump_delay: 0,
            jumping: false,
        }
    }

    /// The selectors' part of `Mob.serverAiStep`: every goal on full ticks
    /// (odd id-based tick counts run only the every-tick goals), targets
    /// first. The sensing cache starts empty.
    pub fn tick_goals(&mut self, world: &dyn World, full: bool) {
        self.state.seen.clear();
        self.state.jump = false;
        self.state.yaw = self.yaw;
        // `LivingEntity.updatingUsingItem` runs before `aiStep`.
        if self.state.bow.using_item {
            self.state.bow.ticks_using_item += 1;
        }
        if full {
            self.targets.tick(&mut self.state, world);
            self.goals.tick(&mut self.state, world);
        } else {
            self.targets.tick_running_world(&mut self.state, world, false);
            self.goals.tick_running_world(&mut self.state, world, false);
        }
        self.yaw = self.state.yaw;
        if let Some((forward, sideways)) = self.state.strafe.take() {
            self.move_control.strafe(forward, sideways);
        }
    }

    /// Keeps only the named goals (the harness's `entity_keep_goal`),
    /// before the first tick.
    pub fn retain_goals(&mut self, names: &[&str]) {
        let ids: Vec<usize> = self.state.kind.goal_names().iter().enumerate().filter_map(|(id, name)| names.contains(name).then_some(id)).collect();
        self.goals.retain_ids(&ids);
    }

    /// The running goals' names, for comparison with a vanilla trace.
    pub fn running_goals(&self) -> Vec<&'static str> {
        let names = self.state.kind.goal_names();
        self.goals.running_ids().map(|id| names[id]).collect()
    }

    /// The running target goals' names.
    pub fn running_targets(&self) -> Vec<&'static str> {
        let names = self.state.kind.target_goal_names();
        self.targets.running_ids().map(|id| names[id]).collect()
    }
}

pub fn creeper_goals() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(1, FloatGoal);
    selector.add(2, SwellGoal { target: None });
    selector.add(3, NeverGoal { controls: &[Control::Move] });
    selector.add(3, NeverGoal { controls: &[Control::Move] });
    selector.add(4, MeleeAttackGoal { hits: false, raises_arms: false, follow_unseen: false, gives_up_in_light: false });
    selector.add(5, StrollGoal { speed: 0.8, probability: 0.001 });
    selector.add(6, LookAtPlayerGoal { range: 8.0 });
    selector.add(6, RandomLookGoal);
    selector
}

pub fn creeper_targets() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(1, NearestTargetGoal { interval: 5, prey: Prey::Player, must_see: true, max_dy: None, dark_only: false, candidate: None, unseen_ticks: 0, wild_only: false });
    selector.add(2, HurtByTargetGoal { timestamp: 0, unseen_ticks: 0, target_mob: None, alert_others: false });
    selector
}

pub fn spider_goals() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(1, FloatGoal);
    // `AvoidEntityGoal` for armadillos: none live here.
    selector.add(2, NeverGoal { controls: &[Control::Move] });
    selector.add(3, LeapGoal { yd: 0.4, target: None });
    selector.add(4, MeleeAttackGoal { hits: true, raises_arms: false, follow_unseen: true, gives_up_in_light: true });
    selector.add(5, StrollGoal { speed: 0.8, probability: 0.001 });
    selector.add(6, LookAtPlayerGoal { range: 8.0 });
    selector.add(6, RandomLookGoal);
    selector
}

pub fn spider_targets() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(1, HurtByTargetGoal { timestamp: 0, unseen_ticks: 0, target_mob: None, alert_others: false });
    selector.add(2, NearestTargetGoal { interval: 5, prey: Prey::Player, must_see: true, max_dy: None, dark_only: true, candidate: None, unseen_ticks: 0, wild_only: false });
    selector.add(3, NearestTargetGoal { interval: 5, prey: Prey::Golem, must_see: true, max_dy: None, dark_only: true, candidate: None, unseen_ticks: 0, wild_only: false });
    selector
}

/// `LeapAtTargetGoal(mob, yd)`: on the ground two to four blocks from its
/// target, one tick in three it springs at it, and holds the move and jump
/// controls until it lands.
#[derive(Clone)]
pub(crate) struct LeapGoal {
    yd: f32,
    target: Option<Target>,
}

impl LeapGoal {
    /// `LeapAtTargetGoal(mob, yd)`.
    pub(crate) fn new(yd: f32) -> Self {
        Self { yd, target: None }
    }
}
impl Goal<MonsterGoalContext> for LeapGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Jump, Control::Move])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        let Some(target) = ctx.target() else { return false };
        self.target = Some(target.target);
        let distance = ctx.body.position.distance_squared(target.position);
        if !(4.0..=16.0).contains(&distance) || !ctx.body.on_ground {
            return false;
        }
        // `reducedTickDelay(5)`.
        ctx.random.next_int(3) == 0
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        !ctx.body.on_ground
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        let Some(target) = self.target.and_then(|t| ctx.info(t)) else { return };
        let movement = ctx.body.velocity;
        let mut delta = DVec3::new(target.position.x - ctx.body.position.x, 0.0, target.position.z - ctx.body.position.z);
        if delta.length_squared() > 1.0e-7 {
            // `Vec3.normalize` divides by the length.
            let length = delta.length();
            let unit = if length < f64::from(1.0e-5_f32) { DVec3::ZERO } else { DVec3::new(delta.x / length, delta.y / length, delta.z / length) };
            delta = unit * 0.4 + movement * 0.2;
        }
        ctx.leap = Some(DVec3::new(delta.x, f64::from(self.yd), delta.z));
    }
}

pub fn slime_goals() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(1, CubeFloatGoal);
    selector.add(4, CubeRandomDirectionGoal { chosen_degrees: 0.0, next_randomize_time: 0 });
    selector.add(5, CubeKeepOnJumpingGoal);
    selector.add(2, CubeAttackGoal { grow_tired_timer: 0 });
    selector
}

/// `Slime.addTargetingGoals`: players within four blocks up or down, and
/// iron golems (none live here, but the goal still rolls).
pub fn slime_targets() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(1, NearestTargetGoal { interval: 5, prey: Prey::Player, must_see: true, dark_only: false, max_dy: Some(4.0), candidate: None, unseen_ticks: 0, wild_only: false });
    selector.add(3, NearestTargetGoal { interval: 5, prey: Prey::Golem, must_see: true, dark_only: false, max_dy: None, candidate: None, unseen_ticks: 0, wild_only: false });
    selector
}

/// `CubeMobFloatGoal`: in a liquid it hops (eight times in ten) and swims
/// on at 1.2.
#[derive(Clone)]
struct CubeFloatGoal;
impl Goal<MonsterGoalContext> for CubeFloatGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Jump, Control::Move])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        ctx.fluid.in_water() || ctx.fluid.in_lava()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, ctx: &mut MonsterGoalContext) {
        if ctx.random.next_float() < 0.8 {
            ctx.jump = true;
        }
        ctx.cube.set_wanted_movement(1.2);
    }
}

/// `CubeMobRandomDirectionGoal`: with no target, on the ground or in a
/// liquid, a new random heading every 20 to 49 full ticks.
#[derive(Clone)]
struct CubeRandomDirectionGoal {
    chosen_degrees: f32,
    next_randomize_time: i32,
}
impl Goal<MonsterGoalContext> for CubeRandomDirectionGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Look])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        ctx.target().is_none() && (ctx.body.on_ground || ctx.fluid.in_water() || ctx.fluid.in_lava())
    }
    fn tick(&mut self, ctx: &mut MonsterGoalContext) {
        self.next_randomize_time -= 1;
        if self.next_randomize_time <= 0 {
            // `adjustedTickDelay`: halved (rounded up) for a goal that does
            // not tick every tick.
            let delay = 40 + ctx.random.next_int(60) as i32;
            self.next_randomize_time = (delay + 1) / 2;
            self.chosen_degrees = ctx.random.next_int(360) as f32;
        }
        ctx.cube.set_direction(self.chosen_degrees, false);
    }
}

/// `CubeMobKeepOnJumpingGoal`: unless riding, it keeps hopping.
#[derive(Clone)]
struct CubeKeepOnJumpingGoal;
impl Goal<MonsterGoalContext> for CubeKeepOnJumpingGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Jump, Control::Move])
    }
    fn can_start(&mut self, _ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        true
    }
    fn tick(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.cube.set_wanted_movement(1.0);
    }
}

/// `CubeMobAttackGoal`: faces its target (`Mob.lookAt`, 10° a tick) and
/// hops at it, a third as long between hops when it can hurt; it tires
/// after 150 ticks.
#[derive(Clone)]
struct CubeAttackGoal {
    grow_tired_timer: i32,
}
impl Goal<MonsterGoalContext> for CubeAttackGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Look])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        ctx.target().is_some()
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        if ctx.target().is_none() {
            return false;
        }
        self.grow_tired_timer -= 1;
        self.grow_tired_timer > 0
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn start(&mut self, _ctx: &mut MonsterGoalContext) {
        // `reducedTickDelay(300)`.
        self.grow_tired_timer = 150;
    }
    fn tick(&mut self, ctx: &mut MonsterGoalContext) {
        if let Some(target) = ctx.target() {
            look_at(ctx, target, 10.0, 10.0);
        }
        let aggressive = !ctx.tiny;
        ctx.cube.set_direction(ctx.yaw, aggressive);
    }
}

pub fn skeleton_goals() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(2, RestrictSunGoal);
    selector.add(3, FleeSunGoal { wanted: None });
    selector.add(3, AvoidEntityGoal::new("minecraft:wolf", 6.0, 1.0, 1.2));
    selector.add(5, StrollGoal { speed: 1.0, probability: 0.001 });
    selector.add(6, LookAtPlayerGoal { range: 8.0 });
    selector.add(6, RandomLookGoal);
    // `reassessWeaponGoal` adds the bow goal last.
    selector.add(4, BowGoal);
    selector
}

pub fn skeleton_targets() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(1, HurtByTargetGoal { timestamp: 0, unseen_ticks: 0, target_mob: None, alert_others: false });
    selector.add(2, NearestTargetGoal { interval: 5, prey: Prey::Player, must_see: true, max_dy: None, dark_only: false, candidate: None, unseen_ticks: 0, wild_only: false });
    selector.add(3, NearestTargetGoal { interval: 5, prey: Prey::Golem, must_see: true, max_dy: None, dark_only: false, candidate: None, unseen_ticks: 0, wild_only: false });
    selector.add(3, NearestTargetGoal { interval: 5, prey: Prey::Absent, must_see: true, max_dy: None, dark_only: false, candidate: None, unseen_ticks: 0, wild_only: false });
    selector
}

/// `RestrictSunGoal`: in daylight, bareheaded, the mob's paths keep out of
/// the sun (`setAvoidSun`). It takes no controls.
#[derive(Clone)]
struct RestrictSunGoal;
impl Goal<MonsterGoalContext> for RestrictSunGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        ctx.bright_outside && !ctx.helmet
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.navigation.avoid_sun = true;
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.navigation.avoid_sun = false;
    }
}

/// `FleeSunGoal(mob, 1.0)`: burning in daylight under open sky with no
/// target and nothing on its head, the mob runs for shade: one of ten
/// random spots within ten blocks (three up or down) out of the sky that
/// it likes (`getWalkTargetValue` below zero: darker than middling).
#[derive(Clone)]
struct FleeSunGoal {
    wanted: Option<DVec3>,
}
impl Goal<MonsterGoalContext> for FleeSunGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        let p = ctx.body.position;
        let feet = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        if ctx.target.is_some() || !ctx.bright_outside || !ctx.on_fire || !world.can_see_sky(feet) || ctx.helmet {
            return false;
        }
        self.wanted = None;
        for _ in 0..10 {
            let x = feet.0 + ctx.random.next_int(20) as i32 - 10;
            let y = feet.1 + ctx.random.next_int(6) as i32 - 3;
            let z = feet.2 + ctx.random.next_int(20) as i32 - 10;
            // `Monster.getWalkTargetValue`: the negated light cost.
            if !world.can_see_sky((x, y, z)) && -world.light_path_cost((x, y, z)) < 0.0 {
                self.wanted = Some(DVec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5));
                break;
            }
        }
        self.wanted.is_some()
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        !ctx.navigation.is_done()
    }
    fn start_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        if let Some(target) = self.wanted {
            let profile = ctx.profile();
            let _ = navigate_walk_to(&ctx.body, &mut ctx.navigation, world, &profile, ctx.fluid, target, 1.0, 1);
        }
    }
}

/// `AvoidEntityGoal(mob, class, maxDist, walk, sprint)`: the nearest mob of
/// `kind` in its box grown by `max_dist` across and 3 up and down that it
/// could fight in sight within `max_dist` (`forCombat().range(maxDist)`)
/// sends it to a spot up to 16 blocks off within a quarter turn of straight
/// away (`DefaultRandomPos.getPosAway`), when that spot is farther from the
/// mob than it stands; it goes there at the walk speed, sprinting while
/// within 7 blocks, until its path ends.
#[derive(Clone)]
pub(crate) struct AvoidEntityGoal {
    kind: &'static str,
    max_dist: f64,
    walk: f64,
    sprint: f64,
    to_avoid: Option<Target>,
    path: Option<PlannedPath>,
}
impl AvoidEntityGoal {
    pub(crate) fn new(kind: &'static str, max_dist: f64, walk: f64, sprint: f64) -> Self {
        Self { kind, max_dist, walk, sprint, to_avoid: None, path: None }
    }
}
impl Goal<MonsterGoalContext> for AvoidEntityGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        let body = ctx.body.clone();
        let half = f64::from(body.width / 2.0);
        let p = body.position;
        let (min, max) = (
            DVec3::new(p.x - half - self.max_dist, p.y - 3.0, p.z - half - self.max_dist),
            DVec3::new(p.x + half + self.max_dist, p.y + f64::from(body.height) + 3.0, p.z + half + self.max_dist),
        );
        // `getNearestEntity`: the first of the nearest.
        let range = self.max_dist.max(2.0);
        let mut best: Option<(f64, TargetInfo)> = None;
        for m in ctx.mobs.clone() {
            if m.kind != self.kind {
                continue;
            }
            let mh = f64::from(m.width / 2.0);
            let q = m.position;
            let inside = q.x - mh < max.x && q.x + mh > min.x && q.y < max.y && q.y + f64::from(m.height) > min.y && q.z - mh < max.z && q.z + mh > min.z;
            if !inside {
                continue;
            }
            let Some(info) = ctx.info(Target::Mob(m.id)) else { continue };
            let distance = p.distance_squared(info.position);
            if !ctx.can_attack(info) || distance > range * range || !ctx.sees(world, info) {
                continue;
            }
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, info));
            }
        }
        let Some((_, avoid)) = best else { return false };
        let profile = ctx.profile();
        let Some(pos) = crate::stroll::default_random_position_away(world, &profile, p, 16, 7, avoid.position, &mut ctx.random) else { return false };
        if avoid.position.distance_squared(pos) < avoid.position.distance_squared(p) {
            return false;
        }
        // `createPath(x, y, z, 0)`.
        self.path = plan_walk_path(&body, &mut ctx.navigation, world, &profile, ctx.fluid, pos, 0);
        self.to_avoid = Some(avoid.target);
        self.path.is_some()
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        !ctx.navigation.is_done()
    }
    fn start_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        let position = ctx.body.position;
        ctx.navigation.move_to_in(world, self.path.take(), self.walk, position);
    }
    fn stop(&mut self, _ctx: &mut MonsterGoalContext) {
        self.to_avoid = None;
    }
    fn tick(&mut self, ctx: &mut MonsterGoalContext) {
        let Some(avoid) = self.to_avoid.and_then(|t| ctx.info(t)) else { return };
        ctx.navigation.speed_modifier = if ctx.body.position.distance_squared(avoid.position) < 49.0 { self.sprint } else { self.walk };
    }
}

/// `RangedBowAttackGoal(skeleton, 1.0, interval, 15)`: closes in until it
/// has seen the target for a second within fifteen blocks, then strafes
/// (switching direction now and then, backing off close in, pressing in
/// far out), draws the bow for a second and looses with the draw's power.
#[derive(Clone)]
struct BowGoal;
impl Goal<MonsterGoalContext> for BowGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        ctx.target().is_some()
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        ctx.target().is_some() || !ctx.navigation.is_done()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.melee.aggressive = true;
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.melee.aggressive = false;
        ctx.bow.see_time = 0;
        ctx.bow.attack_time = -1;
        ctx.bow.using_item = false;
        ctx.bow.ticks_using_item = 0;
    }
    fn tick_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        let Some(target) = ctx.target() else { return };
        const RADIUS_SQR: f32 = 15.0 * 15.0;
        let distance = ctx.body.position.distance_squared(target.position);
        let sees = ctx.sees(world, target);
        if sees != (ctx.bow.see_time > 0) {
            ctx.bow.see_time = 0;
        }
        ctx.bow.see_time += if sees { 1 } else { -1 };
        if !(distance > f64::from(RADIUS_SQR)) && ctx.bow.see_time >= 20 {
            ctx.navigation.stop();
            ctx.bow.strafing_time += 1;
        } else {
            let profile = ctx.profile();
            let _ = navigate_walk_to_entity(&ctx.body, &mut ctx.navigation, world, &profile, ctx.fluid, target.position, 1.0, 1);
            ctx.bow.strafing_time = -1;
        }
        if ctx.bow.strafing_time >= 20 {
            if ctx.random.next_float() < 0.3 {
                ctx.bow.strafing_clockwise = !ctx.bow.strafing_clockwise;
            }
            if ctx.random.next_float() < 0.3 {
                ctx.bow.strafing_backwards = !ctx.bow.strafing_backwards;
            }
            ctx.bow.strafing_time = 0;
        }
        let eye = target.position + DVec3::new(0.0, f64::from(target.eye_height), 0.0);
        if ctx.bow.strafing_time > -1 {
            if distance > f64::from(RADIUS_SQR * 0.75) {
                ctx.bow.strafing_backwards = false;
            } else if distance < f64::from(RADIUS_SQR * 0.25) {
                ctx.bow.strafing_backwards = true;
            }
            ctx.strafe = Some((if ctx.bow.strafing_backwards { -0.5 } else { 0.5 }, if ctx.bow.strafing_clockwise { 0.5 } else { -0.5 }));
            look_at(ctx, target, 30.0, 30.0);
        } else {
            ctx.look_control.set_look_at_with_limits(eye, 30.0, 30.0);
        }
        if ctx.bow.using_item {
            if !sees && ctx.bow.see_time < -60 {
                ctx.bow.using_item = false;
                ctx.bow.ticks_using_item = 0;
            } else if sees && ctx.bow.ticks_using_item >= 20 {
                // `BowItem.getPowerForTime`.
                let f = ctx.bow.ticks_using_item as f32 / 20.0;
                let power = ((f * f + f * 2.0) / 3.0).min(1.0);
                ctx.bow.using_item = false;
                ctx.bow.ticks_using_item = 0;
                ctx.shoot = Some((target.target, power));
                ctx.bow.attack_time = ctx.bow.attack_interval;
            }
        } else {
            ctx.bow.attack_time -= 1;
            if ctx.bow.attack_time <= 0 && ctx.bow.see_time >= -60 {
                ctx.bow.using_item = true;
                ctx.bow.ticks_using_item = 0;
            }
        }
    }
}

/// `Mob.lookAt(entity, yMax, xMax)`: the body and pitch turn straight at
/// the target's eyes, by at most the given steps.
fn look_at(ctx: &mut MonsterGoalContext, target: TargetInfo, y_max: f32, x_max: f32) {
    let p = ctx.body.position;
    let xd = target.position.x - p.x;
    let zd = target.position.z - p.z;
    let yd = (target.position.y + f64::from(target.eye_height)) - (p.y + f64::from(ctx.eye_height));
    let sd = (xd * xd + zd * zd).sqrt();
    // `Mth.RAD_TO_DEG`: 180 / π as a float.
    const RAD_TO_DEG: f64 = 57.2957763671875;
    let y_rot = (crate::control::minecraft_atan2(zd, xd) * RAD_TO_DEG) as f32 - 90.0;
    let x_rot = (-(crate::control::minecraft_atan2(yd, sd) * RAD_TO_DEG)) as f32;
    ctx.look_control.pitch = mob_rotlerp(ctx.look_control.pitch, x_rot, x_max);
    ctx.yaw = mob_rotlerp(ctx.yaw, y_rot, y_max);
}

/// `Mob.rotlerp`: turn by at most `max`, the shorter way round.
pub(crate) fn mob_rotlerp(from: f32, to: f32, max: f32) -> f32 {
    let mut diff = (to - from) % 360.0;
    if diff >= 180.0 {
        diff -= 360.0;
    }
    if diff < -180.0 {
        diff += 360.0;
    }
    from + diff.clamp(-max, max)
}

pub fn zombie_goals() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(4, TurtleEggGoal);
    selector.add(8, LookAtPlayerGoal { range: 8.0 });
    selector.add(8, RandomLookGoal);
    // `SpearUseGoal` needs a kinetic weapon in hand.
    selector.add(2, NeverGoal { controls: &[Control::Move, Control::Look] });
    selector.add(3, MeleeAttackGoal { hits: true, raises_arms: true, follow_unseen: false, gives_up_in_light: false });
    selector.add(6, VillageGoal::default());
    selector.add(7, StrollGoal { speed: 1.0, probability: 0.001 });
    selector
}

pub fn zombie_targets() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(1, HurtByTargetGoal { timestamp: 0, unseen_ticks: 0, target_mob: None, alert_others: true });
    selector.add(2, NearestTargetGoal { interval: 5, prey: Prey::Player, must_see: true, max_dy: None, dark_only: false, candidate: None, unseen_ticks: 0, wild_only: false });
    selector.add(3, NearestTargetGoal { interval: 5, prey: Prey::Villager, must_see: false, max_dy: None, dark_only: false, candidate: None, unseen_ticks: 0, wild_only: false });
    selector.add(3, NearestTargetGoal { interval: 5, prey: Prey::Golem, must_see: true, max_dy: None, dark_only: false, candidate: None, unseen_ticks: 0, wild_only: false });
    selector.add(5, NearestTargetGoal { interval: 5, prey: Prey::Absent, must_see: true, max_dy: None, dark_only: false, candidate: None, unseen_ticks: 0, wild_only: false });
    selector
}

#[derive(Clone)]
pub(crate) struct FloatGoal;
impl Goal<MonsterGoalContext> for FloatGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Jump])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        ctx.fluid.floatable_height() > 0.4 || ctx.fluid.in_lava()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, ctx: &mut MonsterGoalContext) {
        if ctx.random.next_float() < 0.8 {
            ctx.jump = true;
        }
    }
}

/// `SwellGoal`: close to its target the creeper stops and swells while it
/// sees the target within seven blocks. It keeps the target it started
/// with, even one that turns creative.
#[derive(Clone)]
struct SwellGoal {
    target: Option<Target>,
}
impl Goal<MonsterGoalContext> for SwellGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        ctx.swell_dir > 0 || ctx.target().is_some_and(|t| ctx.body.position.distance_squared(t.position) < 9.0)
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.navigation.stop();
        self.target = ctx.target().map(|t| t.target);
    }
    fn stop(&mut self, _ctx: &mut MonsterGoalContext) {
        self.target = None;
    }
    fn tick_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        ctx.swell_dir = match self.target.and_then(|t| ctx.info(t)).filter(|t| t.alive) {
            Some(target) if ctx.body.position.distance_squared(target.position) > 49.0 => -1,
            Some(target) if !ctx.sees(world, target) => -1,
            Some(_) => 1,
            None => -1,
        };
    }
}

/// A goal that never starts here: the creeper's `AvoidEntityGoal`s (no
/// ocelots or cats live here) and the zombie's `SpearUseGoal` (no kinetic
/// weapon in hand).
#[derive(Clone)]
pub(crate) struct NeverGoal {
    pub(crate) controls: &'static [Control],
}
impl Goal<MonsterGoalContext> for NeverGoal {
    fn controls(&self) -> Controls {
        Controls::new(self.controls)
    }
    fn can_start(&mut self, _ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        false
    }
}

/// `MoveThroughVillageGoal(zombie, 1.0, true, 4, canBreakDoors)`: at night
/// within six sections of a village, a land spot 15 blocks about is weighed
/// by the first claimed village point of interest within ten of it that the
/// zombie has not visited (nearer to it is better; none outside villages);
/// it paths to the one nearest the chosen spot (only through doors it can
/// break), else to a free spot on the way, stopping short of the first
/// wooden door on the path. It leaves its navigation able to path through
/// doors ever after (`setCanOpenDoors(true)`). It keeps on until it comes
/// within its width and four blocks of the point, which it then counts as
/// visited (the last sixteen are kept).
#[derive(Clone, Default)]
pub(crate) struct VillageGoal {
    path: Option<PlannedPath>,
    poi: (i32, i32, i32),
    visited: Vec<(i32, i32, i32)>,
}

impl VillageGoal {
    /// `BlockPos.closerToCenterThan`.
    fn poi_within(&self, position: DVec3, distance: f64) -> bool {
        let center = DVec3::new(f64::from(self.poi.0) + 0.5, f64::from(self.poi.1) + 0.5, f64::from(self.poi.2) + 0.5);
        center.distance_squared(position) < distance * distance
    }
}

impl Goal<MonsterGoalContext> for VillageGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        if self.visited.len() > 15 {
            self.visited.remove(0);
        }
        if ctx.bright_outside {
            return false;
        }
        let p = ctx.body.position;
        let pos = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        let Some(pois) = ctx.pois.as_mut() else { return false };
        // `isCloseToVillage(pos, 6)`.
        if pois.sections_to_village((pos.0 >> 4, pos.1 >> 4, pos.2 >> 4)) > 6 {
            return false;
        }
        let visited = self.visited.clone();
        let not_visited = |poi: (i32, i32, i32)| !visited.contains(&poi);
        let village = |kind: crate::poi::PoiType| kind.village();
        let land = crate::stroll::land_random_position_by(world, &ctx.walk, p, 15, 7, &mut ctx.random, |spot| {
            if !pois.is_village(spot) {
                return f64::NEG_INFINITY;
            }
            pois.find_first(&village, &not_visited, spot, 10, crate::poi::Occupancy::IsOccupied).map_or(f64::NEG_INFINITY, |poi| -crate::poi::dist_sqr(poi, pos))
        });
        let Some(land) = land else { return false };
        let spot = (land.x.floor() as i32, land.y.floor() as i32, land.z.floor() as i32);
        let Some(poi) = pois.find_first(&village, &not_visited, spot, 10, crate::poi::Occupancy::IsOccupied) else { return false };
        self.poi = poi;
        let block = |(x, y, z): (i32, i32, i32)| DVec3::new(f64::from(x), f64::from(y), f64::from(z));
        let mut breaking = ctx.profile();
        breaking.can_open_doors = ctx.can_break_doors;
        let mut path = plan_walk_path(&ctx.body, &mut ctx.navigation, world, &breaking, ctx.fluid, block(poi), 0);
        ctx.walk.can_open_doors = true;
        if path.is_none() {
            let towards = DVec3::new(f64::from(poi.0) + 0.5, f64::from(poi.1), f64::from(poi.2) + 0.5);
            let Some(step) = crate::stroll::default_random_position_towards(world, &ctx.walk, p, 10, 7, towards, std::f32::consts::FRAC_PI_2, &mut ctx.random) else { return false };
            let mut breaking = ctx.profile();
            breaking.can_open_doors = ctx.can_break_doors;
            path = plan_walk_path(&ctx.body, &mut ctx.navigation, world, &breaking, ctx.fluid, step, 0);
            ctx.walk.can_open_doors = true;
            if path.is_none() {
                return false;
            }
        }
        // `DoorBlock.isWoodenDoor` above a node: stop there.
        let door = path.as_ref().and_then(|planned| {
            planned.nodes().iter().copied().find(|&(x, y, z)| world.block((x, y + 1, z)).is_some_and(|b| b.id.ends_with("_door") && b.id != "minecraft:iron_door"))
        });
        if let Some(node) = door {
            let profile = ctx.profile();
            path = plan_walk_path(&ctx.body, &mut ctx.navigation, world, &profile, ctx.fluid, block(node), 0);
        }
        self.path = path;
        self.path.is_some()
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        !ctx.navigation.is_done() && !self.poi_within(ctx.body.position, f64::from(ctx.body.width) + 4.0)
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        let path = self.path.take();
        ctx.navigation.move_to(path, 1.0, ctx.body.position);
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        if ctx.navigation.is_done() || self.poi_within(ctx.body.position, 4.0) {
            self.visited.push(self.poi);
        }
    }
}

/// `Zombie.ZombieAttackTurtleEggGoal` (a `RemoveBlockGoal`): while mobs
/// may grief, a look for turtle eggs every ten to twenty seconds. Finding
/// one would send the zombie to trample it; that walk is not ported, so it
/// only keeps its timer.
#[derive(Clone)]
struct TurtleEggGoal;
impl Goal<MonsterGoalContext> for TurtleEggGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Jump])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        if !ctx.mob_griefing {
            return false;
        }
        if ctx.turtle_egg_next_start > 0 {
            ctx.turtle_egg_next_start -= 1;
            return false;
        }
        if find_turtle_egg(world, ctx.body.position) {
            ctx.turtle_egg_next_start = 10;
        } else {
            // `MoveToBlockGoal.nextStartTick`: reducedTickDelay(200 + nextInt(200)).
            ctx.turtle_egg_next_start = (200 + ctx.random.next_int(200) as i32 + 1) / 2;
        }
        false
    }
}

/// `MoveToBlockGoal.findNearestBlock` for `RemoveBlockGoal`'s 24-block,
/// three-high search: a turtle egg in the column rings below each level.
fn find_turtle_egg(world: &dyn World, position: DVec3) -> bool {
    let (mx, my, mz) = (position.x.floor() as i32, position.y.floor() as i32, position.z.floor() as i32);
    let mut y = 0;
    while y <= 3 {
        for r in 0..24 {
            let mut x = 0;
            while x <= r {
                let mut z = if x < r && x > -r { r } else { 0 };
                while z <= r {
                    if world.block((mx + x, my + y - 1, mz + z)).is_some_and(|b| b.id == "minecraft:turtle_egg") {
                        return true;
                    }
                    z = if z > 0 { -z } else { 1 - z };
                }
                x = if x > 0 { -x } else { 1 - x };
            }
        }
        y = if y > 0 { -y } else { 1 - y };
    }
    false
}

/// `MeleeAttackGoal(mob, 1.0, false)` (the creeper's approach),
/// `ZombieAttackGoal`, which raises its arms as the next swing nears, and
/// `Spider.SpiderAttackGoal` (`MeleeAttackGoal(spider, 1.0, true)`: it
/// follows a target out of sight, and in the light gives up one tick in a
/// hundred). `Creeper.doHurtTarget` deals nothing; the others' hits land as
/// `attack`.
#[derive(Clone)]
pub(crate) struct MeleeAttackGoal {
    pub(crate) hits: bool,
    pub(crate) raises_arms: bool,
    pub(crate) follow_unseen: bool,
    pub(crate) gives_up_in_light: bool,
}
impl Goal<MonsterGoalContext> for MeleeAttackGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        if ctx.game_time - ctx.melee.last_can_use_check < 20 {
            return false;
        }
        ctx.melee.last_can_use_check = ctx.game_time;
        let Some(target) = ctx.target() else { return false };
        let profile = ctx.profile();
        ctx.melee.path = plan_walk_path(&ctx.body, &mut ctx.navigation, world, &profile, ctx.fluid, target.position, 0);
        ctx.melee.path.is_some() || ctx.within_melee_range(target)
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        if self.gives_up_in_light && ctx.in_light(world) && ctx.random.next_int(100) == 0 {
            ctx.set_target(None);
            return false;
        }
        // `followingTargetEvenIfNotSeen` keeps on while the target lives.
        ctx.target().is_some() && (self.follow_unseen || !ctx.navigation.is_done())
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        let path = ctx.melee.path.take();
        ctx.navigation.move_to(path, 1.0, ctx.body.position);
        ctx.melee.aggressive = true;
        ctx.melee.ticks_until_recalculation = 0;
        ctx.melee.ticks_until_attack = 0;
        ctx.melee.raise_arm_ticks = 0;
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        // `getTarget` already drops creative and spectating players, so the
        // target is never cleared here.
        ctx.melee.aggressive = false;
        ctx.navigation.stop();
    }
    fn tick_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        if let Some(target) = ctx.target() {
            let eye = target.position + DVec3::new(0.0, f64::from(target.eye_height), 0.0);
            ctx.look_control.set_look_at_with_limits(eye, 30.0, 30.0);
            ctx.melee.ticks_until_recalculation = (ctx.melee.ticks_until_recalculation - 1).max(0);
            if (self.follow_unseen || ctx.sees(world, target)) && ctx.melee.ticks_until_recalculation <= 0 {
                let pathed = ctx.melee.pathed_target;
                let recalculate = pathed == DVec3::ZERO || target.position.distance_squared(pathed) >= 1.0 || ctx.random.next_float() < 0.05;
                if recalculate {
                    ctx.melee.pathed_target = target.position;
                    ctx.melee.ticks_until_recalculation = 4 + ctx.random.next_int(7) as i32;
                    let distance = ctx.body.position.distance_squared(target.position);
                    if distance > 1024.0 {
                        ctx.melee.ticks_until_recalculation += 10;
                    } else if distance > 256.0 {
                        ctx.melee.ticks_until_recalculation += 5;
                    }
                    let profile = ctx.profile();
                    if !navigate_walk_to_entity(&ctx.body, &mut ctx.navigation, world, &profile, ctx.fluid, target.position, 1.0, 0) {
                        ctx.melee.ticks_until_recalculation += 15;
                    }
                }
            }
            ctx.melee.ticks_until_attack = (ctx.melee.ticks_until_attack - 1).max(0);
            if ctx.melee.ticks_until_attack <= 0 && ctx.within_melee_range(target) && ctx.sees(world, target) {
                ctx.melee.ticks_until_attack = 20;
                ctx.melee.swings = ctx.melee.swings.wrapping_add(1);
                if self.hits {
                    ctx.attack = Some(target.target);
                    // `IronGolem.doHurtTarget`: half its attack plus up to
                    // all of it, from its random as it strikes.
                    if ctx.kind == MonsterKind::IronGolem {
                        let attack = crate::iron_golem::ATTACK_DAMAGE;
                        ctx.attack_damage = Some(attack / 2.0 + ctx.random.next_int(attack as u32) as f32);
                    }
                }
            }
        }
        if self.raises_arms {
            ctx.melee.raise_arm_ticks += 1;
            ctx.melee.aggressive = ctx.melee.raise_arm_ticks >= 5 && ctx.melee.ticks_until_attack < 10;
        }
    }
}

/// `WaterAvoidingRandomStrollGoal(mob, speed, probability)`.
#[derive(Clone)]
pub(crate) struct StrollGoal {
    speed: f64,
    probability: f32,
}

impl StrollGoal {
    pub(crate) fn new(speed: f64, probability: f32) -> Self {
        Self { speed, probability }
    }
}
impl Goal<MonsterGoalContext> for StrollGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        let in_water = ctx.fluid.in_water();
        ctx.stroll.can_use_with(ctx.body.position, ctx.no_action_time, in_water, world, &ctx.walk, &mut ctx.random, self.probability)
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        !ctx.navigation.is_done()
    }
    fn start_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        if let Some(target) = ctx.stroll.wanted {
            let profile = ctx.profile();
            let _ = navigate_walk_to(&ctx.body, &mut ctx.navigation, world, &profile, ctx.fluid, target, self.speed, 1);
        }
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.navigation.stop();
    }
}

/// `LookAtPlayerGoal(mob, Player.class, range)`.
#[derive(Clone)]
pub(crate) struct LookAtPlayerGoal {
    pub(crate) range: f64,
}
impl Goal<MonsterGoalContext> for LookAtPlayerGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Look])
    }
    /// `canUse` (`getNearestPlayer` with non-combat conditions, which
    /// still ask for sight): the visible player within range of the feet
    /// nearest the eyes.
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        if ctx.random.next_float() >= 0.02 {
            return false;
        }
        let (position, eye) = (ctx.body.position, ctx.eye());
        let mut best: Option<(f64, u64)> = None;
        for player in ctx.players.clone() {
            if !player.alive || player.spectator || position.distance_squared(player.position) > self.range * self.range {
                continue;
            }
            let Some(info) = ctx.info(Target::Player(player.id)) else { continue };
            if !ctx.sees(world, info) {
                continue;
            }
            let distance = player.position.distance_squared(eye);
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, player.id));
            }
        }
        ctx.look_at_player.target_id = best.map(|(_, id)| id);
        best.is_some()
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        ctx.look_at_player.can_continue_within(ctx.body.position, &ctx.players, self.range)
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.look_at_player.start(&mut ctx.random);
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.look_at_player.stop();
    }
    fn tick(&mut self, ctx: &mut MonsterGoalContext) {
        if let Some(target) = ctx.look_at_player.tick(&ctx.players) {
            let x_max = ctx.max_head_x_rot();
            ctx.look_control.set_look_at_with_limits(target, 10.0, x_max);
        }
    }
}

/// `RandomLookAroundGoal`.
#[derive(Clone)]
pub(crate) struct RandomLookGoal;
impl Goal<MonsterGoalContext> for RandomLookGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        RandomLookState::can_use(&mut ctx.random)
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        ctx.random_look.can_continue()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.random_look.start(&mut ctx.random);
    }
    fn tick(&mut self, ctx: &mut MonsterGoalContext) {
        let target = ctx.random_look.tick(ctx.body.position, ctx.eye_height);
        let x_max = ctx.max_head_x_rot();
        ctx.look_control.set_look_at_with_limits(target, 10.0, x_max);
    }
}

/// `Enderman.registerGoals`: float, freeze while a target stares, the melee
/// attack, the stroll (any land spot: its walk values are all 0), looking at
/// players and around, and putting down and picking up blocks.
/// `PatrollingMonster`'s and `Raider`'s goals (none can start outside a
/// patrol or raid, and none draws), then the witch's own.
pub fn witch_goals() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(4, NeverGoal { controls: &[Control::Move] });
    selector.add(1, NeverGoal { controls: &[Control::Move] });
    selector.add(3, NeverGoal { controls: &[Control::Move] });
    selector.add(4, NeverGoal { controls: &[Control::Move] });
    selector.add(5, NeverGoal { controls: &[Control::Move] });
    selector.add(1, FloatGoal);
    selector.add(2, RangedAttackGoal { speed: 1.0, interval_min: crate::witch::ATTACK_INTERVAL, interval_max: crate::witch::ATTACK_INTERVAL, radius: crate::witch::ATTACK_RADIUS });
    selector.add(2, StrollGoal { speed: 1.0, probability: 0.001 });
    selector.add(3, LookAtPlayerGoal { range: 8.0 });
    selector.add(3, RandomLookGoal);
    selector
}

/// `HurtByTargetGoal` (raiders' hits ignored), the raider-healing goal
/// (a coin flip, then nothing without a raid) and players.
pub fn witch_targets() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(1, HurtByTargetGoal { timestamp: 0, unseen_ticks: 0, target_mob: None, alert_others: false });
    selector.add(2, HealRaidersGoal);
    selector.add(3, NearestTargetGoal { interval: 5, prey: Prey::Player, must_see: true, max_dy: None, dark_only: false, candidate: None, unseen_ticks: 0, wild_only: false });
    selector
}

/// `NearestHealableRaiderTargetGoal`: off its cooldown (always, outside a
/// raid) it flips a coin, then finds no raid.
#[derive(Clone)]
struct HealRaidersGoal;
impl Goal<MonsterGoalContext> for HealRaidersGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Target])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        let _ = ctx.random.next_boolean();
        false
    }
}

/// `RangedAttackGoal(mob, speed, min, max, radius)`: walks at its target
/// until it has seen it for 5 ticks within the radius, looks at it, and
/// attacks every `min` to `max` ticks (by distance) while in sight.
#[derive(Clone)]
struct RangedAttackGoal {
    speed: f64,
    interval_min: i32,
    interval_max: i32,
    radius: f32,
}
impl Goal<MonsterGoalContext> for RangedAttackGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        match ctx.target() {
            Some(target) if target.alive => {
                ctx.witch.ranged_target = Some(target.target);
                true
            }
            _ => false,
        }
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        self.can_start(ctx, world) || (ctx.witch.ranged_target.and_then(|t| ctx.info(t)).is_some_and(|t| t.alive) && !ctx.navigation.is_done())
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.witch.ranged_target = None;
        ctx.witch.see_time = 0;
        ctx.witch.attack_time = -1;
    }
    fn tick_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        let Some(target) = ctx.witch.ranged_target.and_then(|t| ctx.info(t)) else { return };
        let distance_sq = ctx.body.position.distance_squared(target.position);
        let sees = ctx.sees(world, target);
        if sees {
            ctx.witch.see_time += 1;
        } else {
            ctx.witch.see_time = 0;
        }
        let radius_sq = self.radius * self.radius;
        if !(distance_sq > f64::from(radius_sq)) && ctx.witch.see_time >= 5 {
            ctx.navigation.stop();
        } else {
            let profile = ctx.profile();
            let _ = navigate_walk_to_entity(&ctx.body, &mut ctx.navigation, world, &profile, ctx.fluid, target.position, self.speed, 1);
        }
        let eye = target.position + DVec3::new(0.0, f64::from(target.eye_height), 0.0);
        ctx.look_control.set_look_at_with_limits(eye, 30.0, 30.0);
        ctx.witch.attack_time -= 1;
        if ctx.witch.attack_time == 0 {
            if !sees {
                return;
            }
            let dist = distance_sq.sqrt() as f32 / self.radius;
            // The power (`clamp(dist, 0.1, 1)`) only matters to arrows.
            perform_ranged_attack(ctx, target);
            ctx.witch.attack_time = (dist * (self.interval_max - self.interval_min) as f32 + self.interval_min as f32).floor() as i32;
        } else if ctx.witch.attack_time < 0 {
            let delta = distance_sq.sqrt() / f64::from(self.radius);
            let (min, max) = (f64::from(self.interval_min), f64::from(self.interval_max));
            ctx.witch.attack_time = (min + delta * (max - min)).floor() as i32;
        }
    }
}

/// `Witch.performRangedAttack`: while drinking it throws nothing; else a
/// splash potion for the target, and the throw sound's pitch from its
/// random.
fn perform_ranged_attack(ctx: &mut MonsterGoalContext, target: TargetInfo) {
    if ctx.witch.drinking {
        return;
    }
    let vitals = match target.target {
        Target::Player(id) => ctx.vitals.iter().find(|(p, _)| *p == id).map_or_else(PlayerVitals::default, |&(_, v)| v),
        Target::Villager(_) | Target::Mob(_) => PlayerVitals::default(),
    };
    let aim = crate::witch::ThrowTarget {
        position: target.position,
        eye_height: target.eye_height,
        velocity: vitals.velocity,
        health: vitals.health,
        slowed: vitals.slowed,
        poisoned: vitals.poisoned,
        weakened: vitals.weakened,
    };
    let throw = crate::witch::aim(ctx.body.position, aim, &mut ctx.random);
    let pitch = 0.8 + ctx.random.next_float() * 0.4;
    ctx.witch.throw = Some((throw, pitch));
}

pub fn enderman_goals() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(0, FloatGoal);
    selector.add(1, FreezeGoal { target: None });
    selector.add(2, MeleeAttackGoal { hits: true, raises_arms: false, follow_unseen: false, gives_up_in_light: false });
    selector.add(7, StrollGoal { speed: 1.0, probability: 0.0 });
    selector.add(8, LookAtPlayerGoal { range: 8.0 });
    selector.add(8, RandomLookGoal);
    selector.add(10, LeaveBlockGoal);
    selector.add(11, TakeBlockGoal);
    selector
}

/// Its targets: a player who stares or angered it
/// (`EndermanLookForPlayerGoal`), whoever hurt it, endermites (none live
/// here: the goal only rolls), and `ResetUniversalAngerTargetGoal`
/// (universal anger is off).
pub fn enderman_targets() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(1, LookForPlayerGoal::default());
    selector.add(2, HurtByTargetGoal { timestamp: 0, unseen_ticks: 0, target_mob: None, alert_others: false });
    selector.add(3, NearestTargetGoal { interval: 5, prey: Prey::Absent, must_see: true, max_dy: None, dark_only: false, candidate: None, unseen_ticks: 0, wild_only: false });
    selector.add(4, NeverGoal { controls: &[] });
    selector
}

/// `EndermanFreezeWhenLookedAt`: while its target, a player within 16
/// blocks, stares at it, it stands still and looks back.
#[derive(Clone)]
struct FreezeGoal {
    target: Option<Target>,
}
impl Goal<MonsterGoalContext> for FreezeGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Jump, Control::Move])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        let target = ctx.target();
        self.target = target.map(|t| t.target);
        let Some(target @ TargetInfo { target: Target::Player(id), .. }) = target else { return false };
        target.position.distance_squared(ctx.body.position) <= 256.0 && ctx.stared_by(world, id)
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.navigation.stop();
    }
    fn tick(&mut self, ctx: &mut MonsterGoalContext) {
        if let Some(target) = self.target.and_then(|t| ctx.info(t)) {
            ctx.look_control.set_look_at(target.position + DVec3::new(0.0, f64::from(target.eye_height), 0.0));
        }
    }
}

/// `EndermanLeaveBlockGoal`: carrying a block where mobs may grief, one
/// time in a thousand it tries a spot about its feet, and sets the block
/// down on air over a full block that is not bedrock.
#[derive(Clone)]
struct LeaveBlockGoal;
impl Goal<MonsterGoalContext> for LeaveBlockGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        ctx.enderman.carried.is_some() && ctx.mob_griefing && ctx.random.next_int(1000) == 0
    }
    fn tick_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        let p = ctx.body.position;
        let x = (p.x - 1.0 + ctx.random.next_double() * 2.0).floor() as i32;
        let y = (p.y + ctx.random.next_double() * 2.0).floor() as i32;
        let z = (p.z - 1.0 + ctx.random.next_double() * 2.0).floor() as i32;
        let Some(carried) = ctx.enderman.carried.clone() else { return };
        let below = (x, y - 1, z);
        let below_block = world.block(below);
        // `canPlaceBlock` (`Block.updateFromNeighbourShapes` leaves these
        // blocks as they are; no entity stands there in these worlds).
        let air = |b: &Option<minecraftoss_player::Block>| b.as_ref().is_none_or(|b| matches!(b.id.as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"));
        let full_below = world.collision_boxes(below) == [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]];
        if air(&world.block((x, y, z))) && !air(&below_block) && below_block.as_ref().is_none_or(|b| b.id != "minecraft:bedrock") && full_below {
            ctx.enderman.block_change = Some(((x, y, z), Some(carried)));
            ctx.enderman.carried = None;
        }
    }
}

/// `EndermanTakeBlockGoal`: carrying nothing where mobs may grief, one time
/// in ten it picks a spot within two blocks, and takes the block there if
/// it is `#enderman_holdable` and the ray from its own column meets it
/// first (the ray reads collision shapes here).
#[derive(Clone)]
struct TakeBlockGoal;
impl Goal<MonsterGoalContext> for TakeBlockGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        ctx.enderman.carried.is_none() && ctx.mob_griefing && ctx.random.next_int(10) == 0
    }
    fn tick_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        let p = ctx.body.position;
        let x = (p.x - 2.0 + ctx.random.next_double() * 4.0).floor() as i32;
        let y = (p.y + ctx.random.next_double() * 3.0).floor() as i32;
        let z = (p.z - 2.0 + ctx.random.next_double() * 4.0).floor() as i32;
        let pos = (x, y, z);
        let from = DVec3::new(p.x.floor() + 0.5, f64::from(y) + 0.5, p.z.floor() + 0.5);
        let to = DVec3::new(f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5);
        // A miss reports the block at its end, which is the spot.
        let reachable = crate::sight::first_hit(world, from, to).is_none_or(|hit| hit == pos);
        if reachable && world.block_in_tag(pos, "minecraft:enderman_holdable") {
            if let Some(block) = world.block(pos) {
                ctx.enderman.block_change = Some((pos, None));
                // `getBlock().defaultBlockState()`.
                ctx.enderman.carried = Some(minecraftoss_player::Block::new(&block.id));
            }
        }
    }
}

/// `EndermanLookForPlayerGoal` over `NearestAttackableTargetGoal<Player>`:
/// the nearest player within follow range and sight who stares at it or
/// angered it becomes pending; it looks back while that lasts, and three
/// goal ticks on the player is its target. Then a staring target nearer
/// than four blocks makes it teleport away, and a target farther than 16
/// makes it teleport nearer every 15 goal ticks.
#[derive(Clone, Default)]
struct LookForPlayerGoal {
    pending: Option<u64>,
    target: Option<Target>,
    aggro_time: i32,
    teleport_time: i32,
    unseen_ticks: i32,
}

impl LookForPlayerGoal {
    /// `isAngerInducing`: it stares at the enderman, or angered it.
    fn anger_inducing(ctx: &MonsterGoalContext, world: &dyn World, id: u64) -> bool {
        let Some(info) = ctx.info(Target::Player(id)) else { return false };
        ctx.stared_by(world, id) || ctx.angry_at(info)
    }
}

impl Goal<MonsterGoalContext> for LookForPlayerGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Target])
    }
    /// `getNearestPlayer` with `startAggroTargetConditions`: combat targets
    /// within follow range of its feet, in sight, anger-inducing; the one
    /// nearest its feet. No interval roll.
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        let range = ctx.follow_range.max(2.0);
        let mut best: Option<(f64, u64)> = None;
        for player in ctx.players.clone() {
            let Some(info) = ctx.info(Target::Player(player.id)) else { continue };
            if !Self::anger_inducing(ctx, world, player.id) || !ctx.can_attack(info) {
                continue;
            }
            let distance = ctx.body.position.distance_squared(info.position);
            if distance > range * range || !ctx.sees(world, info) {
                continue;
            }
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, player.id));
            }
        }
        self.pending = best.map(|(_, id)| id);
        self.pending.is_some()
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        if let Some(id) = self.pending {
            if !Self::anger_inducing(ctx, world, id) {
                return false;
            }
            // `Mob.lookAt(pendingTarget, 10, 10)`.
            if let Some(info) = ctx.info(Target::Player(id)) {
                look_at(ctx, info, 10.0, 10.0);
            }
            return true;
        }
        // `continueAggroTargetConditions`: a combat target, sight ignored.
        if let Some(info) = self.target.and_then(|t| ctx.info(t)) {
            if ctx.can_attack(info) {
                return true;
            }
        }
        continue_target(ctx, world, self.target, &mut self.unseen_ticks, None)
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        // `adjustedTickDelay(5)`.
        self.aggro_time = 3;
        self.teleport_time = 0;
        ctx.enderman.stared_at = true;
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        self.pending = None;
        ctx.clear_target();
        self.target = None;
    }
    fn tick_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        if ctx.target().is_none() {
            self.target = None;
        }
        if let Some(id) = self.pending {
            self.aggro_time -= 1;
            if self.aggro_time <= 0 {
                self.target = Some(Target::Player(id));
                self.pending = None;
                // `NearestAttackableTargetGoal.start`.
                ctx.set_target(self.target);
                self.unseen_ticks = 0;
            }
            return;
        }
        let Some(target) = self.target.and_then(|t| ctx.info(t)) else { return };
        let Target::Player(id) = target.target else { return };
        let distance = target.position.distance_squared(ctx.body.position);
        if ctx.stared_by(world, id) {
            if distance < 16.0 {
                ctx.teleport(world);
            }
            self.teleport_time = 0;
        } else if distance > 256.0 {
            let due = self.teleport_time >= 15;
            self.teleport_time += 1;
            if due && ctx.teleport_towards(world, target) {
                self.teleport_time = 0;
            }
        }
    }
}

/// `Enderman.aiStep`'s `updatePersistentAnger(level, true)`: a target keeps
/// it angry at that target for another 20 to 39 seconds each tick (a draw
/// from its random); anger that has run out with no target ends; so does
/// anger at a player in creative or spectator mode or in peaceful.
pub fn update_enderman_anger(ctx: &mut MonsterGoalContext) {
    let held = ctx.enderman.anger_target;
    // `getTargetUnchecked`: a mob it was angry at, now dying (or gone).
    if let Some(Target::Mob(id)) = ctx.target.filter(|&t| held == Some(t)) {
        if ctx.info(Target::Mob(id)).is_none_or(|m| !m.alive) {
            ctx.stop_being_angry();
            return;
        }
    }
    if let Some(target) = ctx.target() {
        let new_target = held != Some(target.target);
        if new_target {
            ctx.enderman.anger_target = Some(target.target);
        }
        // `startPersistentAngerTimer`: `TimeUtil.rangeOfSeconds(20, 39)`.
        let ticks = ctx.random.next_int(381) as i64 + 400;
        ctx.enderman.anger_end_time = ctx.game_time + ticks;
    }
    let target = ctx.target();
    let valid_player = target.is_some_and(|t| matches!(t.target, Target::Player(_)) && t.player_ok && ctx.difficulty != 0);
    if held.is_some() && !ctx.angry() && (target.is_none() || !valid_player) {
        ctx.stop_being_angry();
    }
    if let Some(Target::Player(id)) = held {
        if let Some(info) = ctx.info(Target::Player(id)) {
            if !info.player_ok || ctx.difficulty == 0 {
                ctx.stop_being_angry();
            }
        }
    }
}

/// `Enderman.customServerAiStep`: in daylight, ten seconds after it last
/// took a target, in light over 0.5 under open sky, it may drop its target
/// and teleport (a float from its random).
pub fn enderman_daylight_step(ctx: &mut MonsterGoalContext, world: &dyn World) {
    if !ctx.bright_outside || ctx.tick_count < ctx.enderman.target_change_time + 600 {
        return;
    }
    let eye = ctx.eye();
    let light = world.light_path_cost((eye.x.floor() as i32, eye.y.floor() as i32, eye.z.floor() as i32)) + 0.5;
    let p = ctx.body.position;
    let feet = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
    if light > 0.5 && world.can_see_sky(feet) && ctx.random.next_float() * 30.0 < (light - 0.4) * 2.0 {
        ctx.clear_target();
        ctx.teleport(world);
    }
}

/// `NearestAttackableTargetGoal(mob, prey, mustSee)`: one try in five, the
/// target nearest its eyes that it may attack, within follow range of its
/// feet and in sight (sight is asked on finding whatever `mustSee` says;
/// `mustSee` only keeps a target in view).
#[derive(Clone)]
pub(crate) struct NearestTargetGoal {
    /// `randomInterval` after `reducedTickDelay`: one roll in this many.
    interval: u32,
    prey: Prey,
    must_see: bool,
    /// `Spider.SpiderTargetGoal`: too light, it does not look (or roll).
    dark_only: bool,
    /// The slime's selector: only targets within this many blocks up or
    /// down.
    max_dy: Option<f64>,
    candidate: Option<Target>,
    unseen_ticks: i32,
    /// `NonTameRandomTargetGoal`: only while wild, and it keeps its target
    /// only while its targeting conditions still hold (in sight, in range).
    wild_only: bool,
}
impl NearestTargetGoal {
    /// `NearestAttackableTargetGoal(mob, prey, interval, mustSee, false, selector)`,
    /// `interval` already reduced.
    pub(crate) fn every(prey: Prey, must_see: bool, interval: u32) -> Self {
        Self { interval, prey, must_see, dark_only: false, max_dy: None, candidate: None, unseen_ticks: 0, wild_only: false }
    }

    /// As a `NonTameRandomTargetGoal`.
    pub(crate) fn wild_only(mut self) -> Self {
        self.wild_only = true;
        self
    }
}

impl Goal<MonsterGoalContext> for NearestTargetGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Target])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        if self.wild_only && ctx.wolf.tame {
            return false;
        }
        if self.dark_only && ctx.in_light(world) {
            return false;
        }
        if ctx.random.next_int(self.interval) != 0 {
            return false;
        }
        let (range, eye) = (ctx.follow_range.max(2.0), ctx.eye());
        let candidates: Vec<Target> = match self.prey {
            Prey::Player => ctx.players.iter().map(|p| Target::Player(p.id)).collect(),
            Prey::Villager => ctx.villagers.iter().map(|v| Target::Villager(v.id)).collect(),
            Prey::Absent => Vec::new(),
            Prey::Golem => ctx.mobs.iter().filter(|m| m.kind == "minecraft:iron_golem").map(|m| Target::Mob(m.id)).collect(),
            Prey::Enemy => ctx.mobs.iter().filter(|m| is_enemy(m.kind) && m.kind != "minecraft:creeper").map(|m| Target::Mob(m.id)).collect(),
            Prey::AngryPlayer => ctx.players.iter().map(|p| Target::Player(p.id)).filter(|&t| ctx.enderman.anger_target == Some(t)).collect(),
            Prey::WolfPrey => ctx.mobs.iter().filter(|m| matches!(m.kind, "minecraft:sheep" | "minecraft:rabbit" | "minecraft:fox")).map(|m| Target::Mob(m.id)).collect(),
            Prey::Skeleton => ctx.mobs.iter().filter(|m| matches!(m.kind, "minecraft:skeleton" | "minecraft:stray" | "minecraft:bogged" | "minecraft:parched" | "minecraft:wither_skeleton")).map(|m| Target::Mob(m.id)).collect(),
        };
        let mut best: Option<(f64, Target)> = None;
        for target in candidates {
            let Some(info) = ctx.info(target) else { continue };
            if !ctx.can_attack(info) || ctx.body.position.distance_squared(info.position) > range * range || !ctx.sees(world, info) {
                continue;
            }
            if self.max_dy.is_some_and(|dy| (info.position.y - ctx.body.position.y).abs() > dy) {
                continue;
            }
            let distance = info.position.distance_squared(eye);
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, target));
            }
        }
        self.candidate = best.map(|(_, target)| target);
        self.candidate.is_some()
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        if self.wild_only {
            // `targetConditions.test(mob, target)` on the target it found:
            // alive, attackable, within follow range of its feet (2 at
            // least), and in sight.
            let Some(info) = self.candidate.and_then(|t| ctx.info(t)) else { return false };
            let range = ctx.follow_range.max(2.0);
            return ctx.can_attack(info) && ctx.body.position.distance_squared(info.position) <= range * range && ctx.sees(world, info);
        }
        continue_target(ctx, world, None, &mut self.unseen_ticks, self.must_see.then_some(30))
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.set_target(self.candidate);
        self.unseen_ticks = 0;
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.clear_target();
    }
}

/// `TargetGoal.canContinueToUse`: the target (or the one the goal
/// remembers) can still be attacked, is within follow range, and, when the
/// goal must see it, has not been out of sight for more than `memory` goal
/// ticks; it stays the target.
pub fn continue_target(ctx: &mut MonsterGoalContext, world: &dyn World, remembered: Option<Target>, unseen_ticks: &mut i32, memory: Option<i32>) -> bool {
    let target = ctx.target().map(|t| t.target).or(remembered);
    let Some(target) = target.and_then(|t| ctx.info(t)) else { return false };
    if !ctx.can_attack(target) {
        return false;
    }
    if ctx.body.position.distance_squared(target.position) > ctx.follow_range * ctx.follow_range {
        return false;
    }
    if let Some(memory) = memory {
        if ctx.sees(world, target) {
            *unseen_ticks = 0;
        } else {
            *unseen_ticks += 1;
            if *unseen_ticks > memory {
                return false;
            }
        }
    }
    ctx.set_target(Some(target.target));
    true
}

/// `HurtByTargetGoal`: whoever last hurt it (a player or a mob it may
/// attack: `HURT_BY_TARGETING` asks no sight) becomes its target,
/// remembered for 150 goal ticks out of sight. The zombie's also alerts
/// nearby zombies (`setAlertOthers`).
#[derive(Clone)]
pub(crate) struct HurtByTargetGoal {
    pub(crate) timestamp: i32,
    pub(crate) unseen_ticks: i32,
    pub(crate) target_mob: Option<Target>,
    pub(crate) alert_others: bool,
}
impl Goal<MonsterGoalContext> for HurtByTargetGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Target])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        let Some((attacker, when)) = ctx.hurt_by else { return false };
        when != self.timestamp && ctx.info(attacker).is_some_and(|p| ctx.can_attack(p))
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        continue_target(ctx, world, self.target_mob, &mut self.unseen_ticks, Some(150))
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        if let Some((attacker, when)) = ctx.hurt_by {
            ctx.set_target(Some(attacker));
            self.target_mob = ctx.target;
            self.timestamp = when;
            if self.alert_others {
                ctx.alert = Some(attacker);
            }
        }
        self.unseen_ticks = 0;
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.clear_target();
        self.target_mob = None;
    }
}
