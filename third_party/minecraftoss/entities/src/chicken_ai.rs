//! Chicken goal adapters into the shared source-informed GoalSelector.
//! Registration order, priorities and speeds follow Chicken.registerGoals.
use minecraftoss_player::World;
use crate::breed::BreedState;
use crate::fluid::FluidFrame;
use crate::follow_parent::{CowCandidate, FollowParentState};
use crate::goals::{Control, Controls, Goal, GoalSelector};
use crate::look::{LookAtPlayerState, RandomLookState};
use crate::panic::PanicState;
use crate::stroll::StrollState;
use crate::tempt::{PlayerCandidate, TemptAction, TemptState};
use glam::DVec3;
use minecraftoss_player::rng::LegacyRandom;

pub const FLOAT_GOAL: usize = 0;
pub const PANIC_GOAL: usize = 1;
pub const BREED_GOAL: usize = 2;
pub const TEMPT_GOAL: usize = 3;
pub const FOLLOW_PARENT_GOAL: usize = 4;
pub const STROLL_GOAL: usize = 5;
pub const LOOK_AT_PLAYER_GOAL: usize = 6;
pub const RANDOM_LOOK_GOAL: usize = 7;
pub const GOAL_NAMES: [&str; 8] = [
    "FloatGoal",
    "PanicGoal",
    "BreedGoal",
    "TemptGoal",
    "FollowParentGoal",
    "WaterAvoidingRandomStrollGoal",
    "LookAtPlayerGoal",
    "RandomLookAroundGoal",
];

pub struct ChickenGoalContext {
    pub child: CowCandidate,
    pub candidates: Vec<CowCandidate>,
    pub players: Vec<PlayerCandidate>,
    pub breed: BreedState,
    pub tempt: TemptState,
    pub follow_parent: FollowParentState,
    pub random_look: RandomLookState,
    pub look_at_player: LookAtPlayerState,
    pub stroll: StrollState,
    pub panic: PanicState,
    pub last_damage_panic: bool,
    /// `isOnFire`: panicking, it runs for water first.
    pub on_fire: bool,
    pub fluid: FluidFrame,
    /// The mob's walk search settings (stroll and panic positions).
    pub walk: crate::walk_path::WalkProfile,
    pub no_action_time: i32,
    pub navigation_done: bool,
    pub random: LegacyRandom,
    pub effects: Vec<ChickenGoalEffect>,
}

pub enum ChickenGoalEffect {
    Navigate {
        target: DVec3,
        speed: f64,
    },
    /// `PathNavigation.moveTo(entity, speed)`: toward a mob or player,
    /// keeping the current path when none can be made (in the air).
    NavigateToEntity {
        target: DVec3,
        speed: f64,
    },
    StopNavigation,
    LookAt {
        target: DVec3,
        y_max: f32,
        x_max: f32,
    },
    Breed {
        partner_id: u64,
    },
    Jump,
}

pub fn registered_chicken_goals() -> GoalSelector<ChickenGoalContext> {
    let mut selector = GoalSelector::default();
    assert_eq!(selector.add(0, FloatGoal), FLOAT_GOAL);
    assert_eq!(selector.add(1, PanicGoal), PANIC_GOAL);
    assert_eq!(selector.add(2, BreedGoal), BREED_GOAL);
    assert_eq!(selector.add(3, TemptGoal), TEMPT_GOAL);
    assert_eq!(selector.add(4, FollowParentGoal), FOLLOW_PARENT_GOAL);
    assert_eq!(selector.add(5, StrollGoal), STROLL_GOAL);
    assert_eq!(selector.add(6, LookAtPlayerGoal), LOOK_AT_PLAYER_GOAL);
    assert_eq!(selector.add(7, RandomLookGoal), RANDOM_LOOK_GOAL);
    selector
}

#[derive(Clone)]
struct FloatGoal;
impl Goal<ChickenGoalContext> for FloatGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Jump])
    }
    fn can_start(&mut self, ctx: &mut ChickenGoalContext, _world: &dyn World) -> bool {
        ctx.fluid.floatable_height() > 0.4 || ctx.fluid.in_lava()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, ctx: &mut ChickenGoalContext) {
        if ctx.random.next_float() < 0.8 {
            ctx.effects.push(ChickenGoalEffect::Jump);
        }
    }
}

#[derive(Clone)]
struct PanicGoal;
impl Goal<ChickenGoalContext> for PanicGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut ChickenGoalContext, world: &dyn World) -> bool {
        ctx.panic.can_use(
            ctx.last_damage_panic,
            ctx.on_fire,
            ctx.child.position,
            world,
            &ctx.walk,
            &mut ctx.random,
        )
    }
    fn can_continue(&mut self, ctx: &mut ChickenGoalContext, _world: &dyn World) -> bool {
        !ctx.navigation_done
    }
    fn start(&mut self, ctx: &mut ChickenGoalContext) {
        ctx.panic.running = true;
        if let Some(target) = ctx.panic.wanted {
            ctx.effects
                .push(ChickenGoalEffect::Navigate { target, speed: 1.4 });
        }
    }
    fn stop(&mut self, ctx: &mut ChickenGoalContext) {
        ctx.panic.running = false;
    }
}

#[derive(Clone)]
struct StrollGoal;
impl Goal<ChickenGoalContext> for StrollGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut ChickenGoalContext, world: &dyn World) -> bool {
        ctx.stroll.can_use(
            ctx.child.position,
            ctx.no_action_time,
            ctx.fluid.in_water(),
            world,
            &ctx.walk,
            &mut ctx.random,
        )
    }
    fn can_continue(&mut self, ctx: &mut ChickenGoalContext, _world: &dyn World) -> bool {
        !ctx.navigation_done
    }
    fn start(&mut self, ctx: &mut ChickenGoalContext) {
        if let Some(target) = ctx.stroll.wanted {
            ctx.effects
                .push(ChickenGoalEffect::Navigate { target, speed: 1.0 });
        }
    }
    fn stop(&mut self, ctx: &mut ChickenGoalContext) {
        ctx.effects.push(ChickenGoalEffect::StopNavigation);
    }
}

#[derive(Clone)]
struct LookAtPlayerGoal;
impl Goal<ChickenGoalContext> for LookAtPlayerGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Look])
    }
    fn can_start(&mut self, ctx: &mut ChickenGoalContext, world: &dyn World) -> bool {
        let eye_height: f32 = if ctx.child.age < 0 { 0.28125 } else { 0.644 };
        ctx.look_at_player
            .can_use(world, ctx.child.position, eye_height, &ctx.players, &mut ctx.random)
    }
    fn can_continue(&mut self, ctx: &mut ChickenGoalContext, _world: &dyn World) -> bool {
        ctx.look_at_player
            .can_continue(ctx.child.position, &ctx.players)
    }
    fn start(&mut self, ctx: &mut ChickenGoalContext) {
        ctx.look_at_player.start(&mut ctx.random);
    }
    fn stop(&mut self, ctx: &mut ChickenGoalContext) {
        ctx.look_at_player.stop();
    }
    fn tick(&mut self, ctx: &mut ChickenGoalContext) {
        if let Some(target) = ctx.look_at_player.tick(&ctx.players) {
            ctx.effects.push(ChickenGoalEffect::LookAt {
                target,
                y_max: 10.0,
                x_max: 40.0,
            });
        }
    }
}

#[derive(Clone)]
struct RandomLookGoal;
impl Goal<ChickenGoalContext> for RandomLookGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut ChickenGoalContext, _world: &dyn World) -> bool {
        RandomLookState::can_use(&mut ctx.random)
    }
    fn can_continue(&mut self, ctx: &mut ChickenGoalContext, _world: &dyn World) -> bool {
        ctx.random_look.can_continue()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn start(&mut self, ctx: &mut ChickenGoalContext) {
        ctx.random_look.start(&mut ctx.random);
    }
    fn tick(&mut self, ctx: &mut ChickenGoalContext) {
        let eye_height = if ctx.child.age < 0 { 0.28125 } else { 0.644 };
        ctx.effects.push(ChickenGoalEffect::LookAt {
            target: ctx.random_look.tick(ctx.child.position, eye_height),
            y_max: 10.0,
            x_max: 40.0,
        });
    }
}

#[derive(Clone)]
struct TemptGoal;
impl Goal<ChickenGoalContext> for TemptGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut ChickenGoalContext, _world: &dyn World) -> bool {
        ctx.tempt
            .can_use_matching(ctx.child, &ctx.players, |player| {
                player.main_hand_chicken_food || player.offhand_chicken_food
            })
    }
    fn can_continue(&mut self, ctx: &mut ChickenGoalContext, _world: &dyn World) -> bool {
        // Chicken passes canScare=false, so TemptGoal calls canUse again.
        ctx.tempt
            .can_use_matching(ctx.child, &ctx.players, |player| {
                player.main_hand_chicken_food || player.offhand_chicken_food
            })
    }
    fn start(&mut self, ctx: &mut ChickenGoalContext) {
        ctx.tempt.start();
    }
    fn stop(&mut self, ctx: &mut ChickenGoalContext) {
        ctx.tempt.stop();
        ctx.effects.push(ChickenGoalEffect::StopNavigation);
    }
    fn tick(&mut self, ctx: &mut ChickenGoalContext) {
        if let Some(player) = ctx
            .players
            .iter()
            .find(|p| Some(p.id) == ctx.tempt.player_id)
        {
            ctx.effects.push(ChickenGoalEffect::LookAt {
                target: player.position + DVec3::new(0.0, f64::from(player.eye_height), 0.0),
                y_max: 95.0,
                x_max: 40.0,
            });
        }
        match ctx.tempt.tick(ctx.child, &ctx.players) {
            Some(TemptAction::Navigate(target)) => ctx
                .effects
                .push(ChickenGoalEffect::NavigateToEntity { target, speed: 1.0 }),
            Some(TemptAction::StopNavigation) => {
                ctx.effects.push(ChickenGoalEffect::StopNavigation)
            }
            None => {}
        }
    }
}

#[derive(Clone)]
struct BreedGoal;
impl Goal<ChickenGoalContext> for BreedGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut ChickenGoalContext, _world: &dyn World) -> bool {
        ctx.breed.can_use(ctx.child, &ctx.candidates)
    }
    fn can_continue(&mut self, ctx: &mut ChickenGoalContext, _world: &dyn World) -> bool {
        ctx.breed.can_continue(&ctx.candidates)
    }
    fn start(&mut self, ctx: &mut ChickenGoalContext) {
        ctx.breed.start();
    }
    fn stop(&mut self, ctx: &mut ChickenGoalContext) {
        ctx.breed.stop();
    }
    fn tick(&mut self, ctx: &mut ChickenGoalContext) {
        if let Some((target, partner)) = ctx.breed.tick(ctx.child, &ctx.candidates) {
            if let Some(candidate) = ctx
                .candidates
                .iter()
                .find(|p| Some(p.id) == ctx.breed.partner_id)
            {
                let eye_height: f32 = if candidate.age < 0 { 0.28125 } else { 0.644 };
                ctx.effects.push(ChickenGoalEffect::LookAt {
                    target: candidate.position + DVec3::new(0.0, f64::from(eye_height), 0.0),
                    y_max: 10.0,
                    x_max: 40.0,
                });
            }
            ctx.effects
                .push(ChickenGoalEffect::NavigateToEntity { target, speed: 1.0 });
            if let Some(partner_id) = partner {
                ctx.effects.push(ChickenGoalEffect::Breed { partner_id });
            }
        }
    }
}

#[derive(Clone)]
struct FollowParentGoal;
impl Goal<ChickenGoalContext> for FollowParentGoal {
    fn controls(&self) -> Controls {
        // The pinned FollowParentGoal constructor does not set any flags.
        Controls::new(&[])
    }
    fn can_start(&mut self, ctx: &mut ChickenGoalContext, _world: &dyn World) -> bool {
        ctx.follow_parent.can_use(ctx.child, &ctx.candidates)
    }
    fn can_continue(&mut self, ctx: &mut ChickenGoalContext, _world: &dyn World) -> bool {
        ctx.follow_parent.can_continue(ctx.child, &ctx.candidates)
    }
    fn start(&mut self, ctx: &mut ChickenGoalContext) {
        ctx.follow_parent.start();
    }
    fn stop(&mut self, ctx: &mut ChickenGoalContext) {
        ctx.follow_parent.stop();
    }
    fn tick(&mut self, ctx: &mut ChickenGoalContext) {
        if let Some(target) = ctx.follow_parent.tick(&ctx.candidates) {
            ctx.effects
                .push(ChickenGoalEffect::NavigateToEntity { target, speed: 1.1 });
        }
    }
}
