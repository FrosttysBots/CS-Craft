//! Sheep EatBlockGoal, source-informed by pinned 26.3 EatBlockGoal and Sheep.
//! The shared selector owns scheduling; this goal owns only its countdown.
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
pub const EAT_BLOCK_GOAL: usize = 5;
pub const STROLL_GOAL: usize = 6;
pub const LOOK_AT_PLAYER_GOAL: usize = 7;
pub const RANDOM_LOOK_GOAL: usize = 8;
pub const GOAL_NAMES: [&str; 9] = [
    "FloatGoal",
    "PanicGoal",
    "BreedGoal",
    "TemptGoal",
    "FollowParentGoal",
    "EatBlockGoal",
    "WaterAvoidingRandomStrollGoal",
    "LookAtPlayerGoal",
    "RandomLookAroundGoal",
];

pub enum SheepGoalEffect {
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
    Jump,
    LookAt {
        target: DVec3,
        y_max: f32,
        x_max: f32,
    },
    Breed {
        partner_id: u64,
    },
}

pub struct SheepGoalContext {
    pub baby: bool,
    pub edible_here: bool,
    pub grass_below: bool,
    pub random: LegacyRandom,
    pub eat_animation_ticks: i32,
    pub ate: bool,
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
    pub effects: Vec<SheepGoalEffect>,
}

pub fn registered_sheep_goals() -> GoalSelector<SheepGoalContext> {
    let mut goals = GoalSelector::default();
    assert_eq!(goals.add(0, SheepFloatGoal), FLOAT_GOAL);
    assert_eq!(goals.add(1, SheepPanicGoal), PANIC_GOAL);
    assert_eq!(goals.add(2, SheepBreedGoal), BREED_GOAL);
    assert_eq!(goals.add(3, SheepTemptGoal), TEMPT_GOAL);
    assert_eq!(goals.add(4, SheepFollowParentGoal), FOLLOW_PARENT_GOAL);
    assert_eq!(goals.add(5, EatBlockGoal { remaining: 0 }), EAT_BLOCK_GOAL);
    assert_eq!(goals.add(6, SheepStrollGoal), STROLL_GOAL);
    assert_eq!(goals.add(7, SheepLookAtPlayerGoal), LOOK_AT_PLAYER_GOAL);
    assert_eq!(goals.add(8, SheepRandomLookGoal), RANDOM_LOOK_GOAL);
    goals
}

#[derive(Clone)]
struct SheepFloatGoal;

impl Goal<SheepGoalContext> for SheepFloatGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Jump])
    }
    fn can_start(&mut self, ctx: &mut SheepGoalContext, _world: &dyn World) -> bool {
        ctx.fluid.floatable_height() > 0.4 || ctx.fluid.in_lava()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, ctx: &mut SheepGoalContext) {
        if ctx.random.next_float() < 0.8 {
            ctx.effects.push(SheepGoalEffect::Jump);
        }
    }
}

#[derive(Clone)]
struct SheepPanicGoal;

impl Goal<SheepGoalContext> for SheepPanicGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut SheepGoalContext, world: &dyn World) -> bool {
        ctx.panic.can_use(
            ctx.last_damage_panic,
            ctx.on_fire,
            ctx.child.position,
            world,
            &ctx.walk,
            &mut ctx.random,
        )
    }
    fn can_continue(&mut self, ctx: &mut SheepGoalContext, _world: &dyn World) -> bool {
        !ctx.navigation_done
    }
    fn start(&mut self, ctx: &mut SheepGoalContext) {
        ctx.panic.running = true;
        if let Some(target) = ctx.panic.wanted {
            ctx.effects.push(SheepGoalEffect::Navigate {
                target,
                speed: 1.25,
            });
        }
    }
    fn stop(&mut self, ctx: &mut SheepGoalContext) {
        ctx.panic.running = false;
    }
}

#[derive(Clone)]
struct SheepStrollGoal;

impl Goal<SheepGoalContext> for SheepStrollGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut SheepGoalContext, world: &dyn World) -> bool {
        ctx.stroll.can_use(
            ctx.child.position,
            ctx.no_action_time,
            ctx.fluid.in_water(),
            world,
            &ctx.walk,
            &mut ctx.random,
        )
    }
    fn can_continue(&mut self, ctx: &mut SheepGoalContext, _world: &dyn World) -> bool {
        !ctx.navigation_done
    }
    fn start(&mut self, ctx: &mut SheepGoalContext) {
        if let Some(target) = ctx.stroll.wanted {
            ctx.effects
                .push(SheepGoalEffect::Navigate { target, speed: 1.0 });
        }
    }
    fn stop(&mut self, ctx: &mut SheepGoalContext) {
        ctx.effects.push(SheepGoalEffect::StopNavigation);
    }
}

#[derive(Clone)]
struct SheepLookAtPlayerGoal;

impl Goal<SheepGoalContext> for SheepLookAtPlayerGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Look])
    }
    fn can_start(&mut self, ctx: &mut SheepGoalContext, world: &dyn World) -> bool {
        let eye_height: f32 = if ctx.child.age < 0 { 0.6175 } else { 1.235 };
        ctx.look_at_player
            .can_use(world, ctx.child.position, eye_height, &ctx.players, &mut ctx.random)
    }
    fn can_continue(&mut self, ctx: &mut SheepGoalContext, _world: &dyn World) -> bool {
        ctx.look_at_player
            .can_continue(ctx.child.position, &ctx.players)
    }
    fn start(&mut self, ctx: &mut SheepGoalContext) {
        ctx.look_at_player.start(&mut ctx.random);
    }
    fn stop(&mut self, ctx: &mut SheepGoalContext) {
        ctx.look_at_player.stop();
    }
    fn tick(&mut self, ctx: &mut SheepGoalContext) {
        if let Some(target) = ctx.look_at_player.tick(&ctx.players) {
            ctx.effects.push(SheepGoalEffect::LookAt {
                target,
                y_max: 10.0,
                x_max: 40.0,
            });
        }
    }
}

#[derive(Clone)]
struct SheepRandomLookGoal;

impl Goal<SheepGoalContext> for SheepRandomLookGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut SheepGoalContext, _world: &dyn World) -> bool {
        RandomLookState::can_use(&mut ctx.random)
    }
    fn can_continue(&mut self, ctx: &mut SheepGoalContext, _world: &dyn World) -> bool {
        ctx.random_look.can_continue()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn start(&mut self, ctx: &mut SheepGoalContext) {
        ctx.random_look.start(&mut ctx.random);
    }
    fn tick(&mut self, ctx: &mut SheepGoalContext) {
        let eye_height = if ctx.child.age < 0 { 0.6175 } else { 1.235 };
        ctx.effects.push(SheepGoalEffect::LookAt {
            target: ctx.random_look.tick(ctx.child.position, eye_height),
            y_max: 10.0,
            x_max: 40.0,
        });
    }
}

#[derive(Clone)]
struct SheepTemptGoal;

impl Goal<SheepGoalContext> for SheepTemptGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut SheepGoalContext, _world: &dyn World) -> bool {
        ctx.tempt.can_use(ctx.child, &ctx.players)
    }
    fn can_continue(&mut self, ctx: &mut SheepGoalContext, _world: &dyn World) -> bool {
        ctx.tempt.can_use(ctx.child, &ctx.players)
    }
    fn start(&mut self, ctx: &mut SheepGoalContext) {
        ctx.tempt.start();
    }
    fn stop(&mut self, ctx: &mut SheepGoalContext) {
        ctx.tempt.stop();
        ctx.effects.push(SheepGoalEffect::StopNavigation);
    }
    fn tick(&mut self, ctx: &mut SheepGoalContext) {
        if let Some(player) = ctx
            .players
            .iter()
            .find(|p| Some(p.id) == ctx.tempt.player_id)
        {
            ctx.effects.push(SheepGoalEffect::LookAt {
                target: player.position + DVec3::new(0.0, f64::from(player.eye_height), 0.0),
                y_max: 95.0,
                x_max: 40.0,
            });
        }
        match ctx.tempt.tick(ctx.child, &ctx.players) {
            Some(TemptAction::Navigate(target)) => ctx
                .effects
                .push(SheepGoalEffect::NavigateToEntity { target, speed: 1.1 }),
            Some(TemptAction::StopNavigation) => ctx.effects.push(SheepGoalEffect::StopNavigation),
            None => {}
        }
    }
}

#[derive(Clone)]
struct SheepFollowParentGoal;

impl Goal<SheepGoalContext> for SheepFollowParentGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[])
    }
    fn can_start(&mut self, ctx: &mut SheepGoalContext, _world: &dyn World) -> bool {
        ctx.follow_parent.can_use(ctx.child, &ctx.candidates)
    }
    fn can_continue(&mut self, ctx: &mut SheepGoalContext, _world: &dyn World) -> bool {
        ctx.follow_parent.can_continue(ctx.child, &ctx.candidates)
    }
    fn start(&mut self, ctx: &mut SheepGoalContext) {
        ctx.follow_parent.start();
    }
    fn stop(&mut self, ctx: &mut SheepGoalContext) {
        ctx.follow_parent.stop();
    }
    fn tick(&mut self, ctx: &mut SheepGoalContext) {
        if let Some(target) = ctx.follow_parent.tick(&ctx.candidates) {
            ctx.effects
                .push(SheepGoalEffect::NavigateToEntity { target, speed: 1.1 });
        }
    }
}

#[derive(Clone)]
struct SheepBreedGoal;

impl Goal<SheepGoalContext> for SheepBreedGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut SheepGoalContext, _world: &dyn World) -> bool {
        ctx.breed.can_use(ctx.child, &ctx.candidates)
    }
    fn can_continue(&mut self, ctx: &mut SheepGoalContext, _world: &dyn World) -> bool {
        ctx.breed.can_continue(&ctx.candidates)
    }
    fn start(&mut self, ctx: &mut SheepGoalContext) {
        ctx.breed.start();
    }
    fn stop(&mut self, ctx: &mut SheepGoalContext) {
        ctx.breed.stop();
    }
    fn tick(&mut self, ctx: &mut SheepGoalContext) {
        if let Some((target, partner)) = ctx.breed.tick(ctx.child, &ctx.candidates) {
            if let Some(candidate) = ctx
                .candidates
                .iter()
                .find(|p| Some(p.id) == ctx.breed.partner_id)
            {
                let eye_height: f32 = if candidate.age < 0 { 0.6175 } else { 1.235 };
                ctx.effects.push(SheepGoalEffect::LookAt {
                    target: candidate.position + DVec3::new(0.0, f64::from(eye_height), 0.0),
                    y_max: 10.0,
                    x_max: 40.0,
                });
            }
            ctx.effects
                .push(SheepGoalEffect::NavigateToEntity { target, speed: 1.0 });
            if let Some(partner_id) = partner {
                ctx.effects.push(SheepGoalEffect::Breed { partner_id });
            }
        }
    }
}

#[derive(Clone)]
struct EatBlockGoal {
    remaining: i32,
}

impl Goal<SheepGoalContext> for EatBlockGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look, Control::Jump])
    }

    fn can_start(&mut self, ctx: &mut SheepGoalContext, _world: &dyn World) -> bool {
        let delay = if ctx.baby { 50 } else { 1000 };
        ctx.random.next_int(delay) == 0 && (ctx.edible_here || ctx.grass_below)
    }

    fn can_continue(&mut self, _: &mut SheepGoalContext, _world: &dyn World) -> bool {
        self.remaining > 0
    }

    fn start(&mut self, ctx: &mut SheepGoalContext) {
        // Goal.adjustedTickDelay halves values for goals without every-tick updates.
        self.remaining = 20;
        ctx.eat_animation_ticks = self.remaining;
    }

    fn stop(&mut self, ctx: &mut SheepGoalContext) {
        self.remaining = 0;
        ctx.eat_animation_ticks = 0;
    }

    fn tick(&mut self, ctx: &mut SheepGoalContext) {
        self.remaining = (self.remaining - 1).max(0);
        ctx.eat_animation_ticks = self.remaining;
        if self.remaining == 2 && (ctx.edible_here || ctx.grass_below) {
            ctx.ate = true;
        }
    }
}

pub fn edible_for_sheep(block_id: &str) -> bool {
    // data/minecraft/tags/block/edible_for_sheep.json in the pinned common JAR.
    matches!(
        block_id,
        "minecraft:short_grass"
            | "minecraft:short_dry_grass"
            | "minecraft:tall_dry_grass"
            | "minecraft:fern"
    )
}
