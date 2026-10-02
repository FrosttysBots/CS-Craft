//! The iron golem's goals on the monster goal framework
//! (`IronGolem.registerGoals`): the melee attack, moving towards its
//! target, back to a village when outside one, strolling about the
//! village, offering villagers a poppy, looking at players and around;
//! and its targets: players the villagers about think badly of, whoever
//! hurt it, players it is angry at, and monsters other than creepers. Its village goals read the level's points of
//! interest and draw from the level's random, which the world lends the
//! context while it ticks. Sources: `GolemRandomStrollInVillageGoal`,
//! `MoveBackToVillageGoal`, `OfferFlowerGoal`, `MoveTowardsTargetGoal`,
//! `RandomStrollGoal`, `DefendVillageTargetGoal` and
//! `ResetUniversalAngerTargetGoal` in the pinned 26.3 common JAR.
use crate::goals::{Control, Controls, Goal, GoalSelector};
use crate::monster_ai::{continue_target, HurtByTargetGoal, LookAtPlayerGoal, MeleeAttackGoal, MonsterGoalContext, NearestTargetGoal, NeverGoal, Prey, RandomLookGoal, Target};
use crate::navigation::navigate_walk_to;
use crate::poi::Occupancy;
use crate::stroll::{default_random_position_towards, land_random_position, land_random_position_towards};
use glam::DVec3;
use minecraftoss_player::World;

pub const GOAL_NAMES: [&str; 7] = [
    "MeleeAttackGoal",
    "MoveTowardsTargetGoal",
    "MoveBackToVillageGoal",
    "GolemRandomStrollInVillageGoal",
    "OfferFlowerGoal",
    "LookAtPlayerGoal",
    "RandomLookAroundGoal",
];
pub const TARGET_NAMES: [&str; 5] = [
    "DefendVillageTargetGoal",
    "HurtByTargetGoal",
    "NearestAttackableTargetGoal",
    "NearestAttackableTargetGoal",
    "ResetUniversalAngerTargetGoal",
];

/// A villager as a golem's goals see it.
#[derive(Clone, Debug)]
pub struct GolemVillager {
    pub id: u64,
    pub position: DVec3,
    pub width: f32,
    pub height: f32,
    pub alive: bool,
    /// `Villager.wantsToSpawnGolem`: it slept within a day and has not
    /// seen a golem lately.
    pub wants_golem: bool,
    /// `getPlayerReputation` for each player it has heard of, by player ID.
    pub reputations: Vec<(u64, i32)>,
}

/// An iron golem's own state for its goals.
#[derive(Clone, Debug, Default)]
pub struct GolemState {
    /// The villagers of the entity sections about it, in their order.
    pub villagers: Vec<GolemVillager>,
    /// `offerFlowerTick`: ticks left holding out a poppy.
    pub offer_flower_tick: i32,
    /// `PlayerCreated`: it never attacks players (`IronGolem.canAttack`).
    pub player_created: bool,
}

pub fn golem_goals() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(1, MeleeAttackGoal { hits: true, raises_arms: false, follow_unseen: true, gives_up_in_light: false });
    selector.add(2, MoveTowardsTargetGoal { target: None, wanted: DVec3::ZERO });
    selector.add(2, MoveBackToVillageGoal { wanted: DVec3::ZERO });
    selector.add(4, VillageStrollGoal { wanted: DVec3::ZERO });
    selector.add(5, OfferFlowerGoal { entity: None, tick: 0 });
    selector.add(7, LookAtPlayerGoal { range: 6.0 });
    selector.add(8, RandomLookGoal);
    selector
}

/// The universal-anger reset needs the game rule, which is off.
pub fn golem_targets() -> GoalSelector<MonsterGoalContext> {
    let mut selector = GoalSelector::default();
    selector.add(1, DefendVillageTargetGoal { potential: None, unseen_ticks: 0 });
    selector.add(2, HurtByTargetGoal { timestamp: 0, unseen_ticks: 0, target_mob: None, alert_others: false });
    selector.add(3, NearestTargetGoal::every(Prey::AngryPlayer, true, 5));
    selector.add(3, NearestTargetGoal::every(Prey::Enemy, false, 3));
    selector.add(4, NeverGoal { controls: &[] });
    selector
}

/// `DefendVillageTargetGoal`: of the villagers in its box grown by 10, 8
/// and 10 that it could fight (`TargetingConditions.forCombat().range(64)`:
/// within 64 and in sight), a player in that box it could fight whom one of
/// them thinks badly of (a reputation of -100 or worse; the last such pair
/// wins) becomes its target, unless spectating or in creative. Vanilla never
/// clears that candidate, so a golem that once had one keeps coming back
/// to it.
#[derive(Clone)]
struct DefendVillageTargetGoal {
    potential: Option<Target>,
    unseen_ticks: i32,
}

impl DefendVillageTargetGoal {
    /// `TargetingConditions.forCombat().range(64).test(golem, target)`.
    fn fightable(ctx: &mut MonsterGoalContext, world: &dyn World, target: Target) -> bool {
        let Some(info) = ctx.info(target) else { return false };
        if !ctx.can_attack(info) || ctx.body.position.distance_squared(info.position) > 64.0 * 64.0 {
            return false;
        }
        ctx.sees(world, info)
    }
}

impl Goal<MonsterGoalContext> for DefendVillageTargetGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Target])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        let half = f64::from(ctx.body.width) / 2.0;
        let p = ctx.body.position;
        let (min, max) = (DVec3::new(p.x - half - 10.0, p.y - 8.0, p.z - half - 10.0), DVec3::new(p.x + half + 10.0, p.y + f64::from(ctx.body.height) + 8.0, p.z + half + 10.0));
        let inside = |at: DVec3, width: f32, height: f32| {
            let h = f64::from(width) / 2.0;
            at.x - h < max.x && at.x + h > min.x && at.y < max.y && at.y + f64::from(height) > min.y && at.z - h < max.z && at.z + h > min.z
        };
        let villagers: Vec<GolemVillager> = ctx.golem.villagers.iter().filter(|v| inside(v.position, v.width, v.height)).cloned().collect();
        let villagers: Vec<GolemVillager> = villagers.into_iter().filter(|v| Self::fightable(ctx, world, Target::Villager(v.id))).collect();
        let players: Vec<u64> = ctx.players.iter().filter(|pl| inside(pl.position, 0.6, 1.8)).map(|pl| pl.id).collect();
        let players: Vec<u64> = players.into_iter().filter(|&id| Self::fightable(ctx, world, Target::Player(id))).collect();
        for villager in &villagers {
            for &player in &players {
                if villager.reputations.iter().any(|&(id, reputation)| id == player && reputation <= -100) {
                    self.potential = Some(Target::Player(player));
                }
            }
        }
        match self.potential {
            Some(Target::Player(id)) => ctx.players.iter().find(|pl| pl.id == id).is_some_and(|pl| !pl.spectator && pl.attackable),
            Some(_) => true,
            None => false,
        }
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        continue_target(ctx, world, None, &mut self.unseen_ticks, None)
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.set_target(self.potential);
        self.unseen_ticks = 0;
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.clear_target();
    }
}

fn block_pos(p: DVec3) -> (i32, i32, i32) {
    (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
}

fn section_of((x, y, z): (i32, i32, i32)) -> (i32, i32, i32) {
    (x >> 4, y >> 4, z >> 4)
}

/// `Vec3.atBottomCenterOf(SectionPos.center())`.
fn section_bottom_center((x, y, z): (i32, i32, i32)) -> DVec3 {
    DVec3::new(f64::from(x * 16 + 8) + 0.5, f64::from(y * 16 + 8), f64::from(z * 16 + 8) + 0.5)
}

/// `GolemRandomStrollInVillageGoal(golem, 0.6)`: a `RandomStrollGoal` that
/// rolls one in 120 (`reducedTickDelay(240)`) of its random for a spot the
/// level's random steers.
#[derive(Clone)]
struct VillageStrollGoal {
    wanted: DVec3,
}

impl Goal<MonsterGoalContext> for VillageStrollGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        if ctx.random.next_int(120) != 0 {
            return false;
        }
        match village_stroll_position(ctx, world) {
            Some(pos) => {
                self.wanted = pos;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        !ctx.navigation.is_done()
    }
    fn start_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        let profile = ctx.profile();
        let _ = navigate_walk_to(&ctx.body, &mut ctx.navigation, world, &profile, ctx.fluid, self.wanted, 0.6, 1);
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.navigation.stop();
    }
}

/// `GolemRandomStrollInVillageGoal.getPosition`: three times in ten any
/// land spot; otherwise, seven times in ten towards a villager that wants
/// a golem and failing that a claimed point of interest, the other three
/// the other way round; failing both, any land spot.
fn village_stroll_position(ctx: &mut MonsterGoalContext, world: &dyn World) -> Option<DVec3> {
    let first = ctx.level_random.as_mut().expect("the level's random is lent").next_float();
    if first < 0.3 {
        return land_random_position(world, &ctx.walk, ctx.body.position, 10, 7, &mut ctx.random);
    }
    let second = ctx.level_random.as_mut().expect("the level's random is lent").next_float();
    let target = if second < 0.7 {
        towards_villager(ctx, world).or_else(|| towards_poi(ctx, world))
    } else {
        towards_poi(ctx, world).or_else(|| towards_villager(ctx, world))
    };
    target.or_else(|| land_random_position(world, &ctx.walk, ctx.body.position, 10, 7, &mut ctx.random))
}

/// `getPositionTowardsVillagerWhoWantsGolem`: of the villagers whose box
/// meets its own grown by 32 and who want a golem, one the level's random
/// picks.
fn towards_villager(ctx: &mut MonsterGoalContext, world: &dyn World) -> Option<DVec3> {
    let p = ctx.body.position;
    let half = f64::from(ctx.body.width) / 2.0;
    let (min, max) = (DVec3::new(p.x - half - 32.0, p.y - 32.0, p.z - half - 32.0), DVec3::new(p.x + half + 32.0, p.y + f64::from(ctx.body.height) + 32.0, p.z + half + 32.0));
    let wanting: Vec<DVec3> = ctx
        .golem
        .villagers
        .iter()
        .filter(|v| {
            let h = f64::from(v.width) / 2.0;
            let q = v.position;
            v.wants_golem && q.x - h < max.x && q.x + h > min.x && q.y < max.y && q.y + f64::from(v.height) > min.y && q.z - h < max.z && q.z + h > min.z
        })
        .map(|v| v.position)
        .collect();
    if wanting.is_empty() {
        return None;
    }
    let index = ctx.level_random.as_mut().expect("the level's random is lent").next_int(wanting.len() as u32) as usize;
    land_random_position_towards(world, &ctx.walk, p, 10, 7, wanting[index], &mut ctx.random)
}

/// `getPositionTowardsPoi`: a village-centre section within two of its
/// own (x fastest, then y, then z) and a claimed point of interest within
/// eight blocks of that section's centre, each picked by the level's
/// random.
fn towards_poi(ctx: &mut MonsterGoalContext, world: &dyn World) -> Option<DVec3> {
    let here = section_of(block_pos(ctx.body.position));
    let pois = ctx.pois.as_mut().expect("the points of interest are lent");
    let mut centres = Vec::new();
    for dz in -2..=2 {
        for dy in -2..=2 {
            for dx in -2..=2 {
                let section = (here.0 + dx, here.1 + dy, here.2 + dz);
                if pois.sections_to_village(section) == 0 {
                    centres.push(section);
                }
            }
        }
    }
    let random = ctx.level_random.as_mut().expect("the level's random is lent");
    if centres.is_empty() {
        return None;
    }
    let section = centres[random.next_int(centres.len() as u32) as usize];
    let center = (section.0 * 16 + 8, section.1 * 16 + 8, section.2 * 16 + 8);
    let claimed: Vec<(i32, i32, i32)> = pois.in_range(&|_| true, center, 8, Occupancy::IsOccupied).into_iter().map(|r| r.pos).collect();
    if claimed.is_empty() {
        return None;
    }
    let (x, y, z) = claimed[random.next_int(claimed.len() as u32) as usize];
    let towards = DVec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5);
    land_random_position_towards(world, &ctx.walk, ctx.body.position, 10, 7, towards, &mut ctx.random)
}

/// `MoveBackToVillageGoal(golem, 0.6, false)`: outside a village
/// (`isVillage`: over a section from one), one roll in five
/// (`reducedTickDelay(10)`) for a free spot towards the centre of the
/// section within two nearest a village, if that is nearer than its own.
#[derive(Clone)]
struct MoveBackToVillageGoal {
    wanted: DVec3,
}

impl Goal<MonsterGoalContext> for MoveBackToVillageGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        let pos = block_pos(ctx.body.position);
        let pois = ctx.pois.as_mut().expect("the points of interest are lent");
        if pois.is_village(pos) {
            return false;
        }
        if ctx.random.next_int(5) != 0 {
            return false;
        }
        let here = section_of(pos);
        let pois = ctx.pois.as_mut().expect("the points of interest are lent");
        let best = pois.closest_village_section(here, 2);
        if best == here {
            return false;
        }
        let towards = section_bottom_center(best);
        match default_random_position_towards(world, &ctx.walk, ctx.body.position, 10, 7, towards, std::f32::consts::FRAC_PI_2, &mut ctx.random) {
            Some(at) => {
                self.wanted = at;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        !ctx.navigation.is_done()
    }
    fn start_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        let profile = ctx.profile();
        let _ = navigate_walk_to(&ctx.body, &mut ctx.navigation, world, &profile, ctx.fluid, self.wanted, 0.6, 1);
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        ctx.navigation.stop();
    }
}

/// `MoveTowardsTargetGoal(golem, 0.9, 32)`: a target within 32 blocks
/// sends it to a free spot on the way (16 across, 7 up or down); it keeps
/// on while it walks and that target lives within 32 blocks.
#[derive(Clone)]
struct MoveTowardsTargetGoal {
    target: Option<Target>,
    wanted: DVec3,
}

impl Goal<MonsterGoalContext> for MoveTowardsTargetGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        let Some(target) = ctx.target() else {
            self.target = None;
            return false;
        };
        self.target = Some(target.target);
        if target.position.distance_squared(ctx.body.position) > 32.0 * 32.0 {
            return false;
        }
        match default_random_position_towards(world, &ctx.walk, ctx.body.position, 16, 7, target.position, std::f32::consts::FRAC_PI_2, &mut ctx.random) {
            Some(at) => {
                self.wanted = at;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        let Some(target) = self.target.and_then(|t| ctx.info(t)) else { return false };
        !ctx.navigation.is_done() && target.alive && target.position.distance_squared(ctx.body.position) < 32.0 * 32.0
    }
    fn start_world(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) {
        let profile = ctx.profile();
        let _ = navigate_walk_to(&ctx.body, &mut ctx.navigation, world, &profile, ctx.fluid, self.wanted, 0.9, 1);
    }
    fn stop(&mut self, _ctx: &mut MonsterGoalContext) {
        self.target = None;
    }
}

/// `OfferFlowerGoal`: by day, one roll in 8000 for the nearest villager
/// in sight within six blocks (in its box grown by 6, 2 and 6); it holds
/// out a poppy for 400 ticks (`adjustedTickDelay`: 200 goal ticks),
/// looking at the villager.
#[derive(Clone)]
struct OfferFlowerGoal {
    entity: Option<u64>,
    tick: i32,
}

impl Goal<MonsterGoalContext> for OfferFlowerGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut MonsterGoalContext, world: &dyn World) -> bool {
        if !ctx.bright_outside {
            return false;
        }
        if ctx.random.next_int(8000) != 0 {
            return false;
        }
        let p = ctx.body.position;
        let half = f64::from(ctx.body.width) / 2.0;
        let (min, max) = (DVec3::new(p.x - half - 6.0, p.y - 2.0, p.z - half - 6.0), DVec3::new(p.x + half + 6.0, p.y + f64::from(ctx.body.height) + 2.0, p.z + half + 6.0));
        let mut best: Option<(f64, u64)> = None;
        for v in ctx.golem.villagers.clone() {
            let h = f64::from(v.width) / 2.0;
            let q = v.position;
            let inside = q.x - h < max.x && q.x + h > min.x && q.y < max.y && q.y + f64::from(v.height) > min.y && q.z - h < max.z && q.z + h > min.z;
            // `TargetingConditions.forNonCombat().range(6)`: living, within
            // six of its feet, in sight.
            if !inside || !v.alive || p.distance_squared(q) > 36.0 {
                continue;
            }
            let Some(info) = ctx.info(Target::Villager(v.id)) else { continue };
            if !ctx.sees(world, info) {
                continue;
            }
            let distance = q.distance_squared(p);
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, v.id));
            }
        }
        self.entity = best.map(|(_, id)| id);
        self.entity.is_some()
    }
    fn can_continue(&mut self, _ctx: &mut MonsterGoalContext, _world: &dyn World) -> bool {
        self.tick > 0
    }
    fn start(&mut self, ctx: &mut MonsterGoalContext) {
        self.tick = crate::iron_golem::OFFER_TICKS / 2;
        ctx.golem.offer_flower_tick = crate::iron_golem::OFFER_TICKS;
    }
    fn stop(&mut self, ctx: &mut MonsterGoalContext) {
        // No copper golem takes the poppy here.
        ctx.golem.offer_flower_tick = 0;
        self.entity = None;
    }
    fn tick(&mut self, ctx: &mut MonsterGoalContext) {
        if let Some(target) = self.entity.and_then(|id| ctx.info(Target::Villager(id))) {
            let eye = target.position + DVec3::new(0.0, f64::from(target.eye_height), 0.0);
            ctx.look_control.set_look_at_with_limits(eye, 30.0, 30.0);
        }
        self.tick -= 1;
    }
}
