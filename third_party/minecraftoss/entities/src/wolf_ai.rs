//! The wolf's goals on the monster goal framework (26.3 `Wolf.registerGoals`):
//! floating, panicking at fire, lava, frost, cacti and lightning
//! (`TamableAnimalPanicGoal` with `#panic_environmental_causes`), sitting
//! when told, keeping from llamas, leaping at and biting its target,
//! following its owner, breeding, strolling, begging from a player who
//! holds a bone or meat, and looking at players and around; and its
//! targets: whoever hurt its owner or whom its owner hurt, whoever hurt it
//! (the pack joins in), players it is angry at, sheep, rabbits and foxes
//! while wild (and baby turtles on land), and skeletons. Sources: `Wolf`,
//! `TamableAnimal`, `BegGoal`, `LeapAtTargetGoal`, `PanicGoal`,
//! `NonTameRandomTargetGoal` and `NearestAttackableTargetGoal` in the
//! pinned 26.3 common JAR.
use crate::goals::{Control, Controls, Goal, GoalSelector};
use crate::monster_ai::{FloatGoal, HurtByTargetGoal, LeapGoal, LookAtPlayerGoal, MeleeAttackGoal, MonsterGoalContext, NearestTargetGoal, NeverGoal, Prey, RandomLookGoal, StrollGoal, Target};
use crate::navigation::{navigate_walk_to, navigate_walk_to_entity};
use glam::DVec3;
use minecraftoss_player::path_type::PathType;
use minecraftoss_player::World;

pub const GOAL_NAMES: [&str; 12] = [
    "FloatGoal",
    "TamableAnimalPanicGoal",
    "SitWhenOrderedToGoal",
    "WolfAvoidEntityGoal",
    "LeapAtTargetGoal",
    "MeleeAttackGoal",
    "FollowOwnerGoal",
    "BreedGoal",
    "WaterAvoidingRandomStrollGoal",
    "BegGoal",
    "LookAtPlayerGoal",
    "RandomLookAroundGoal",
];
pub const TARGET_NAMES: [&str; 8] = [
    "OwnerHurtByTargetGoal",
    "OwnerHurtTargetGoal",
    "HurtByTargetGoal",
    "NearestAttackableTargetGoal",
    "NonTameRandomTargetGoal",
    "NonTameRandomTargetGoal",
    "NearestAttackableTargetGoal",
    "ResetUniversalAngerTargetGoal",
];

/// A wolf's own state for its goals.
#[derive(Clone, Debug, Default)]
pub struct WolfState {
    /// `TamableAnimal.isTame` and its owner (the ID of the player it
    /// belongs to, while that player is in the level: `getOwner`).
    pub tame: bool,
    pub owner: Option<u64>,
    /// The owner's fight memory (`getLastHurtByMob` and
    /// `getLastHurtMob` with their stamps from its `tickCount`), and whether
    /// its last damage, within 100 ticks, calls for retaliation
    /// (`getLastDamageSource(100)`, not `#no_wolf_retaliation`).
    pub owner_hurt_by: Option<(Target, i32)>,
    pub owner_hurt_mob: Option<(Target, i32)>,
    pub owner_damage_recent: bool,
    /// The tame wolves with the same owner, which it never turns on.
    pub pack_mates: Vec<u64>,
    /// In love (`Animal.isInLove`), and the other wolves as `BreedGoal`
    /// looks for a partner among them.
    pub in_love: bool,
    pub mates: Vec<WolfMate>,
    /// `BreedGoal`: its partner, the ticks it has courted, and the partner
    /// it breeds with this tick.
    pub breed_partner: Option<u64>,
    pub love_time: i32,
    pub breed_with: Option<u64>,
    /// It teleported to its owner this tick (`snapTo`: it has not moved).
    pub teleported: bool,
    /// `FollowOwnerGoal`: ticks to the next path, and the water malus it
    /// set aside while following.
    pub follow_recalc: i32,
    pub follow_water_cost: f32,
    pub ordered_to_sit: bool,
    /// `isInSittingPose` (its look rises only 20 degrees).
    pub sitting: bool,
    /// `DATA_INTERESTED_ID`, set while it begs.
    pub interested: bool,
    /// The player it begs from and the ticks left looking.
    pub beg: Option<(u64, i32)>,
    /// The type of the damage it took within the last 40 ticks
    /// (`getLastDamageSource`).
    pub last_damage: Option<&'static str>,
    pub panic: crate::panic::PanicState,
}

/// `BreedGoal(1.0)`: a wolf in love looks for the nearest wolf it can mate
/// with (`Wolf.canMate`: both tame and in love, the partner not sitting)
/// that is not panicking, within its box grown by 8
/// (`TargetingConditions.forNonCombat().range(8)`, sight not needed), goes
/// to it and looks at it, and after 30 goal ticks within three blocks
/// breeds; it gives up after 60 ticks or when the partner dies, falls out
/// of love or panics.
#[derive(Clone)]
struct BreedGoal {
    speed: f64,
}
impl BreedGoal {
    fn partner(ctx: &MonsterGoalContext) -> Option<WolfMate> {
        let id = ctx.wolf.breed_partner?;
        ctx.wolf.mates.iter().find(|m| m.id == id).copied()
    }
}
impl Goal<MonsterGoalContext> for BreedGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        if !ctx.wolf.in_love {
            return false;
        }
        let body = &ctx.body;
        let half = f64::from(body.width / 2.0);
        let p = body.position;
        let (min, max) = (DVec3::new(p.x - half - 8.0, p.y - 8.0, p.z - half - 8.0), DVec3::new(p.x + half + 8.0, p.y + f64::from(body.height) + 8.0, p.z + half + 8.0));
        let mut best: Option<(f64, u64)> = None;
        for m in &ctx.wolf.mates {
            let mh = f64::from(m.width / 2.0);
            let q = m.position;
            let inside = q.x - mh < max.x && q.x + mh > min.x && q.y < max.y && q.y + f64::from(m.height) > min.y && q.z - mh < max.z && q.z + mh > min.z;
            let distance = p.distance_squared(q);
            // `forNonCombat().range(8)`: alive and within 8 blocks.
            if !inside || !m.alive || distance > 64.0 {
                continue;
            }
            let can_mate = ctx.wolf.tame && m.tame && !m.sitting && ctx.wolf.in_love && m.in_love;
            if can_mate && !m.panicking && best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, m.id));
            }
        }
        ctx.wolf.breed_partner = best.map(|(_, id)| id);
        best.is_some()
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        Self::partner(ctx).is_some_and(|m| m.alive && m.in_love && !m.panicking) && ctx.wolf.love_time < 60
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.wolf.breed_partner = None;
        ctx.wolf.love_time = 0;
    }
    fn tick_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        let Some(partner) = Self::partner(ctx) else { return };
        // `setLookAt(partner, 10, getMaxHeadXRot())`: its eyes (0.68 up, a
        // pup's half).
        let eye = if partner.height < crate::wolf::HEIGHT { crate::wolf::BABY_EYE_HEIGHT } else { crate::wolf::EYE_HEIGHT };
        let x_max = ctx.max_head_x_rot();
        ctx.look_control.set_look_at_with_limits(partner.position + DVec3::Y * f64::from(eye), 10.0, x_max);
        let profile = ctx.profile();
        let _ = navigate_walk_to_entity(&ctx.body, &mut ctx.navigation, world, &profile, ctx.fluid, partner.position, self.speed, 1);
        ctx.wolf.love_time += 1;
        // `adjustedTickDelay(60)`.
        if ctx.wolf.love_time >= 30 && ctx.body.position.distance_squared(partner.position) < 9.0 {
            ctx.wolf.breed_with = Some(partner.id);
        }
    }
}

/// Another wolf as `BreedGoal` sees it.
#[derive(Clone, Copy, Debug)]
pub struct WolfMate {
    pub id: u64,
    pub position: DVec3,
    pub width: f32,
    pub height: f32,
    pub alive: bool,
    pub tame: bool,
    pub sitting: bool,
    pub in_love: bool,
    /// `isPanicking`: its panic goal runs.
    pub panicking: bool,
}

/// `#minecraft:panic_environmental_causes`.
fn environmental(kind: &str) -> bool {
    matches!(
        kind,
        "minecraft:cactus" | "minecraft:freeze" | "minecraft:hot_floor" | "minecraft:sulfur_cube_hot" | "minecraft:in_fire" | "minecraft:lava" | "minecraft:lightning_bolt" | "minecraft:on_fire"
    )
}

pub fn wolf_goals() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(1, FloatGoal);
    selector.add(1, PanicGoal { speed: 1.5 });
    selector.add(2, SitGoal);
    // `WolfAvoidEntityGoal`: no llamas live here.
    selector.add(3, NeverGoal { controls: &[Control::Move] });
    selector.add(4, LeapGoal::new(0.4));
    selector.add(5, MeleeAttackGoal { hits: true, raises_arms: false, follow_unseen: true, gives_up_in_light: false });
    selector.add(6, FollowOwnerGoal { speed: 1.0, start: 10.0, stop: 2.0 });
    selector.add(7, BreedGoal { speed: 1.0 });
    selector.add(8, StrollGoal::new(1.0, 0.001));
    selector.add(9, BegGoal { range: 8.0 });
    selector.add(10, LookAtPlayerGoal { range: 8.0 });
    selector.add(10, RandomLookGoal);
    selector
}

/// The universal-anger reset needs the game rule, which is off. `NearestAttackableTargetGoal`'s interval of 10
/// becomes one roll in 5 (`reducedTickDelay`).
pub fn wolf_targets() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(1, OwnerHurtGoal { by_owner: false, timestamp: 0, unseen_ticks: 0 });
    selector.add(2, OwnerHurtGoal { by_owner: true, timestamp: 0, unseen_ticks: 0 });
    selector.add(3, HurtByTargetGoal { timestamp: 0, unseen_ticks: 0, target_mob: None, alert_others: true });
    selector.add(4, NearestTargetGoal::every(Prey::AngryPlayer, true, 5));
    selector.add(5, NearestTargetGoal::every(Prey::WolfPrey, false, 5).wild_only());
    selector.add(6, NearestTargetGoal::every(Prey::Absent, false, 5).wild_only());
    selector.add(7, NearestTargetGoal::every(Prey::Skeleton, false, 5));
    selector.add(8, NeverGoal { controls: &[] });
    selector
}

/// `TamableAnimalPanicGoal(1.5, #panic_environmental_causes)`: hurt by
/// fire, lava, frost, a cactus or lightning within the last 40 ticks, it
/// runs for water when burning (else anywhere five across and four up or
/// down, `DefaultRandomPos`) until its path is done. A tame one free to go
/// to its owner teleports to it first when 12 blocks or more away.
#[derive(Clone)]
struct PanicGoal {
    speed: f64,
}
impl Goal<MonsterGoalContext> for PanicGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        let panics = ctx.wolf.last_damage.is_some_and(environmental);
        let profile = ctx.profile();
        let (on_fire, position) = (ctx.on_fire, ctx.body.position);
        ctx.wolf.panic.can_use(panics, on_fire, position, world, &profile, &mut ctx.random)
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        !ctx.navigation.is_done()
    }
    fn start_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        if let Some(target) = ctx.wolf.panic.wanted {
            let profile = ctx.profile();
            let _ = navigate_walk_to(&ctx.body, &mut ctx.navigation, world, &profile, ctx.fluid, target, self.speed, 1);
        }
        ctx.wolf.panic.running = true;
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.wolf.panic.running = false;
    }
    fn tick_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        if let Some(owner) = owner_of(ctx) {
            if !unable_to_move_to_owner(ctx) && ctx.body.position.distance_squared(owner.position) >= 144.0 {
                teleport_to_owner(ctx, world, owner.position);
            }
        }
    }
}

/// `OwnerHurtByTargetGoal` (`by_owner` false: whoever last hurt its owner,
/// the damage fresh within 100 ticks and one it may retaliate for) and
/// `OwnerHurtTargetGoal` (`by_owner`: whatever its owner last hurt): a tame
/// wolf not told to sit takes on a new one (its stamp differs from the one
/// it last took) that it can attack in sight (`TargetingConditions.DEFAULT`)
/// and wants to (`Wolf.wantsToAttack`), and keeps it as `TargetGoal` does.
#[derive(Clone)]
struct OwnerHurtGoal {
    by_owner: bool,
    timestamp: i32,
    unseen_ticks: i32,
}
impl OwnerHurtGoal {
    fn candidate(&self, ctx: &MonsterGoalContext) -> Option<(Target, i32)> {
        if self.by_owner {
            ctx.wolf.owner_hurt_mob
        } else if ctx.wolf.owner_damage_recent {
            ctx.wolf.owner_hurt_by
        } else {
            None
        }
    }
}
impl Goal<MonsterGoalContext> for OwnerHurtGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Target])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        if !ctx.wolf.tame || ctx.wolf.ordered_to_sit || owner_of(ctx).is_none() {
            return false;
        }
        let Some((target, stamp)) = self.candidate(ctx) else { return false };
        stamp != self.timestamp && ctx.info(target).is_some_and(|info| ctx.can_attack(info) && ctx.sees(world, info)) && wants_to_attack(ctx, target)
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        crate::monster_ai::continue_target(ctx, world, None, &mut self.unseen_ticks, None)
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        if let Some((target, stamp)) = self.candidate(ctx) {
            ctx.set_target(Some(target));
            self.timestamp = stamp;
        }
        self.unseen_ticks = 0;
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.clear_target();
    }
}

/// `Wolf.wantsToAttack`: never creepers or ghasts, nor wolves of its own
/// owner.
fn wants_to_attack(ctx: &MonsterGoalContext, target: Target) -> bool {
    match ctx.kind_of(target) {
        Some("minecraft:creeper" | "minecraft:ghast") => false,
        Some("minecraft:wolf") => !matches!(target, Target::Mob(id) if ctx.wolf.pack_mates.contains(&id)),
        _ => true,
    }
}

/// `TamableAnimal.getOwner`: its owner, while in the level.
fn owner_of(ctx: &MonsterGoalContext) -> Option<crate::tempt::PlayerCandidate> {
    let owner = ctx.wolf.owner?;
    ctx.players.iter().find(|p| p.id == owner).copied()
}

/// `TamableAnimal.unableToMoveToOwner`: told to sit, or its owner a
/// spectator (it rides nothing and wears no lead here).
fn unable_to_move_to_owner(ctx: &MonsterGoalContext) -> bool {
    ctx.wolf.ordered_to_sit || owner_of(ctx).is_some_and(|o| o.spectator)
}

/// `TamableAnimal.teleportToAroundBlockPos(owner.blockPosition())`: ten
/// tries at a spot three blocks about the owner's, at least two out on one
/// axis and a block up or down, until one is walkable, not on leaves and
/// clear for its box moved there; it lands at the block's centre, its path
/// dropped.
fn teleport_to_owner(ctx: &mut MonsterGoalContext, world: &dyn World, owner: DVec3) {
    let target = (owner.x.floor() as i32, owner.y.floor() as i32, owner.z.floor() as i32);
    for _ in 0..10 {
        let xd = ctx.random.next_int(7) as i32 - 3;
        let zd = ctx.random.next_int(7) as i32 - 3;
        if xd.abs() < 2 && zd.abs() < 2 {
            continue;
        }
        let yd = ctx.random.next_int(3) as i32 - 1;
        let pos = (target.0 + xd, target.1 + yd, target.2 + zd);
        if crate::walk_path::path_type_static(world, pos) != PathType::Walkable {
            continue;
        }
        if world.block((pos.0, pos.1 - 1, pos.2)).is_some_and(|b| b.id.ends_with("_leaves")) {
            continue;
        }
        let p = ctx.body.position;
        let here = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        let delta = DVec3::new(f64::from(pos.0 - here.0), f64::from(pos.1 - here.1), f64::from(pos.2 - here.2));
        let half = f64::from(ctx.body.width / 2.0);
        let min = DVec3::new(p.x - half, p.y, p.z - half) + delta;
        let max = DVec3::new(p.x + half, p.y + f64::from(ctx.body.height), p.z + half) + delta;
        if !minecraftoss_player::collision::no_block_collision(world, min, max) {
            continue;
        }
        ctx.body.position = DVec3::new(f64::from(pos.0) + 0.5, f64::from(pos.1), f64::from(pos.2) + 0.5);
        ctx.navigation.stop();
        ctx.wolf.teleported = true;
        return;
    }
}

/// `SitWhenOrderedToGoal`: a tame wolf on the ground and out of water sits
/// when told to, or whenever its owner is not in the level; not beside an
/// owner something has just hurt.
#[derive(Clone)]
struct SitGoal;
impl Goal<MonsterGoalContext> for SitGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Jump, Control::Move])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        let ordered = ctx.wolf.ordered_to_sit;
        if !ordered && !ctx.wolf.tame || ctx.fluid.in_water() || !ctx.body.on_ground {
            return false;
        }
        let Some(owner) = owner_of(ctx) else { return true };
        if ctx.body.position.distance_squared(owner.position) < 144.0 && ctx.wolf.owner_hurt_by.is_some() {
            false
        } else {
            ordered
        }
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        ctx.wolf.ordered_to_sit
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.navigation.stop();
        ctx.wolf.sitting = true;
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.wolf.sitting = false;
    }
}

/// `FollowOwnerGoal(1.0, 10, 2)`: ten blocks or more from its owner and
/// free to go, it heads for the owner (water costing nothing on the way),
/// looking at the owner, a new path every 5 goal ticks, or teleports when 12
/// blocks or more away; it stops within two blocks or when its path ends.
#[derive(Clone)]
struct FollowOwnerGoal {
    speed: f64,
    start: f64,
    stop: f64,
}
impl Goal<MonsterGoalContext> for FollowOwnerGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        let Some(owner) = owner_of(ctx) else { return false };
        !unable_to_move_to_owner(ctx) && ctx.body.position.distance_squared(owner.position) >= self.start * self.start
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        if ctx.navigation.is_done() || unable_to_move_to_owner(ctx) {
            return false;
        }
        owner_of(ctx).is_some_and(|owner| ctx.body.position.distance_squared(owner.position) > self.stop * self.stop)
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.wolf.follow_recalc = 0;
        ctx.wolf.follow_water_cost = ctx.walk.malus(PathType::Water);
        ctx.walk.set_malus(PathType::Water, 0.0);
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.navigation.stop();
        let cost = ctx.wolf.follow_water_cost;
        ctx.walk.set_malus(PathType::Water, cost);
    }
    fn tick_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        let Some(owner) = owner_of(ctx) else { return };
        let far = ctx.body.position.distance_squared(owner.position) >= 144.0;
        if !far {
            let eyes = owner.position + DVec3::Y * f64::from(owner.eye_height);
            let x_max = ctx.max_head_x_rot();
            ctx.look_control.set_look_at_with_limits(eyes, 10.0, x_max);
        }
        ctx.wolf.follow_recalc -= 1;
        if ctx.wolf.follow_recalc <= 0 {
            // `adjustedTickDelay(10)`.
            ctx.wolf.follow_recalc = 5;
            if far {
                teleport_to_owner(ctx, world, owner.position);
            } else {
                let profile = ctx.profile();
                let _ = navigate_walk_to_entity(&ctx.body, &mut ctx.navigation, world, &profile, ctx.fluid, owner.position, self.speed, 1);
            }
        }
    }
}

/// `BegGoal(8)`: the nearest player it sees within 8 blocks of its feet
/// (`getNearestPlayer` with non-combat conditions), if that player holds a
/// bone or wolf food, draws its interest: it looks at the player's eyes for
/// 20 to 39 goal ticks while the player stays near and keeps holding it.
#[derive(Clone)]
struct BegGoal {
    range: f64,
}
impl BegGoal {
    fn holding(ctx: &MonsterGoalContext, player: u64) -> bool {
        ctx.players.iter().any(|p| p.id == player && (p.main_hand_wolf_interest || p.offhand_wolf_interest))
    }
}
impl Goal<MonsterGoalContext> for BegGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Look])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        let position = ctx.body.position;
        let mut best: Option<(f64, u64)> = None;
        for player in ctx.players.clone() {
            if !player.alive || player.spectator || position.distance_squared(player.position) > self.range * self.range {
                continue;
            }
            let Some(info) = ctx.info(Target::Player(player.id)) else { continue };
            if !ctx.sees(world, info) {
                continue;
            }
            let distance = player.position.distance_squared(position);
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, player.id));
            }
        }
        let Some((_, player)) = best else { return false };
        ctx.wolf.beg = Some((player, 0));
        Self::holding(ctx, player)
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        let Some((player, look_time)) = ctx.wolf.beg else { return false };
        let Some(candidate) = ctx.players.iter().find(|p| p.id == player) else { return false };
        if !candidate.alive || ctx.body.position.distance_squared(candidate.position) > self.range * self.range {
            return false;
        }
        look_time > 0 && Self::holding(ctx, player)
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.wolf.interested = true;
        // `adjustedTickDelay(40 + nextInt(40))`: halved, rounding up.
        let ticks = 40 + ctx.random.next_int(40) as i32;
        if let Some(beg) = ctx.wolf.beg.as_mut() {
            beg.1 = (ticks + 1) / 2;
        }
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.wolf.interested = false;
        ctx.wolf.beg = None;
    }
    fn tick(&mut self, ctx: &mut MonsterGoalContext) {
        let Some((player, _)) = ctx.wolf.beg else { return };
        if let Some(p) = ctx.players.iter().find(|p| p.id == player) {
            let eyes = p.position + DVec3::Y * f64::from(p.eye_height);
            let x_max = if ctx.wolf.sitting { 20.0 } else { 40.0 };
            ctx.look_control.set_look_at_with_limits(eyes, 10.0, x_max);
        }
        if let Some(beg) = ctx.wolf.beg.as_mut() {
            beg.1 -= 1;
        }
    }
}
