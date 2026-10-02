//! Pig-specific goal registration over shared 26.3 goal state and selector behavior.
use minecraftoss_player::World;
use crate::{
    breed::BreedState,
    fluid::FluidFrame,
    follow_parent::{CowCandidate, FollowParentState},
    goals::{Control, Controls, Goal, GoalSelector},
    look::{LookAtPlayerState, RandomLookState},
    panic::PanicState,
    stroll::StrollState,
    tempt::{PlayerCandidate, TemptAction, TemptState},
};
use glam::DVec3;
use minecraftoss_player::rng::LegacyRandom;

pub const PANIC_GOAL: usize = 1;
pub const STROLL_GOAL: usize = 6;
pub const GOAL_NAMES: [&str; 9] = [
    "FloatGoal",
    "PanicGoal",
    "BreedGoal",
    "TemptGoal",
    "TemptGoal",
    "FollowParentGoal",
    "WaterAvoidingRandomStrollGoal",
    "LookAtPlayerGoal",
    "RandomLookAroundGoal",
];

pub struct PigGoalContext {
    pub child: CowCandidate,
    pub candidates: Vec<CowCandidate>,
    pub follow_parent: FollowParentState,
    pub breed: BreedState,
    pub panic: PanicState,
    pub stroll: StrollState,
    pub last_damage_panic: bool,
    /// `isOnFire`: panicking, it runs for water first.
    pub on_fire: bool,
    pub no_action_time: i32,
    pub navigation_done: bool,
    pub tempt_stick: TemptState,
    pub tempt_food: TemptState,
    pub random_look: RandomLookState,
    pub look_at_player: LookAtPlayerState,
    pub players: Vec<PlayerCandidate>,
    pub random: LegacyRandom,
    pub fluid: FluidFrame,
    /// The mob's walk search settings (stroll and panic positions).
    pub walk: crate::walk_path::WalkProfile,
    pub effects: Vec<PigGoalEffect>,
}

pub enum PigGoalEffect {
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
    LookAt {
        target: DVec3,
        y_max: f32,
        x_max: f32,
    },
    StopNavigation,
    Jump,
    Breed {
        partner_id: u64,
    },
}

pub fn registered_pig_goals() -> GoalSelector<PigGoalContext> {
    let mut goals = GoalSelector::default();
    // Preserve source priority and insertion order, including the two
    // same-priority TemptGoal instances with distinct item predicates.
    assert_eq!(goals.add(0, FloatGoal), 0);
    assert_eq!(goals.add(1, PanicGoal), PANIC_GOAL);
    assert_eq!(goals.add(3, BreedGoal), 2);
    assert_eq!(goals.add(4, PigTemptGoal(PigTemptKind::Stick)), 3);
    assert_eq!(goals.add(4, PigTemptGoal(PigTemptKind::Food)), 4);
    assert_eq!(goals.add(5, FollowParentGoal), 5);
    assert_eq!(goals.add(6, StrollGoal), STROLL_GOAL);
    assert_eq!(goals.add(7, LookAtPlayerGoal), 7);
    assert_eq!(goals.add(8, RandomLookGoal), 8);
    goals
}

#[derive(Clone)]
struct BreedGoal;
impl Goal<PigGoalContext> for BreedGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut PigGoalContext, _world: &dyn World) -> bool {
        ctx.breed.can_use(ctx.child, &ctx.candidates)
    }
    fn can_continue(&mut self, ctx: &mut PigGoalContext, _world: &dyn World) -> bool {
        ctx.breed.can_continue(&ctx.candidates)
    }
    fn start(&mut self, ctx: &mut PigGoalContext) {
        ctx.breed.start();
    }
    fn stop(&mut self, ctx: &mut PigGoalContext) {
        ctx.breed.stop();
    }
    fn tick(&mut self, ctx: &mut PigGoalContext) {
        if let Some((target, partner)) = ctx.breed.tick(ctx.child, &ctx.candidates) {
            if let Some(candidate) = ctx
                .candidates
                .iter()
                .find(|candidate| Some(candidate.id) == ctx.breed.partner_id)
            {
                let eye_height: f32 = if candidate.age < 0 { 0.3825 } else { 0.765 };
                ctx.effects.push(PigGoalEffect::LookAt {
                    target: candidate.position + DVec3::new(0.0, f64::from(eye_height), 0.0),
                    y_max: 10.0,
                    x_max: 40.0,
                });
            }
            ctx.effects
                .push(PigGoalEffect::NavigateToEntity { target, speed: 1.0 });
            if let Some(partner_id) = partner {
                ctx.effects.push(PigGoalEffect::Breed { partner_id });
            }
        }
    }
}

#[derive(Clone)]
struct StrollGoal;
impl Goal<PigGoalContext> for StrollGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut PigGoalContext, world: &dyn World) -> bool {
        ctx.stroll.can_use(
            ctx.child.position,
            ctx.no_action_time,
            ctx.fluid.in_water(),
            world,
            &ctx.walk,
            &mut ctx.random,
        )
    }
    fn can_continue(&mut self, ctx: &mut PigGoalContext, _world: &dyn World) -> bool {
        !ctx.navigation_done
    }
    fn start(&mut self, ctx: &mut PigGoalContext) {
        if let Some(target) = ctx.stroll.wanted {
            ctx.effects
                .push(PigGoalEffect::Navigate { target, speed: 1.0 });
        }
    }
    fn stop(&mut self, ctx: &mut PigGoalContext) {
        ctx.effects.push(PigGoalEffect::StopNavigation);
    }
}

#[derive(Clone)]
struct PanicGoal;
impl Goal<PigGoalContext> for PanicGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move])
    }
    fn can_start(&mut self, ctx: &mut PigGoalContext, world: &dyn World) -> bool {
        ctx.panic.can_use(
            ctx.last_damage_panic,
            ctx.on_fire,
            ctx.child.position,
            world,
            &ctx.walk,
            &mut ctx.random,
        )
    }
    fn can_continue(&mut self, ctx: &mut PigGoalContext, _world: &dyn World) -> bool {
        !ctx.navigation_done
    }
    fn start(&mut self, ctx: &mut PigGoalContext) {
        ctx.panic.running = true;
        if let Some(target) = ctx.panic.wanted {
            ctx.effects.push(PigGoalEffect::Navigate {
                target,
                speed: 1.25,
            });
        }
    }
    fn stop(&mut self, ctx: &mut PigGoalContext) {
        ctx.panic.running = false;
    }
}

#[derive(Clone, Copy)]
enum PigTemptKind {
    Stick,
    Food,
}

#[derive(Clone)]
struct PigTemptGoal(PigTemptKind);
impl PigTemptGoal {
    fn state<'a>(&self, ctx: &'a PigGoalContext) -> &'a TemptState {
        match self.0 {
            PigTemptKind::Stick => &ctx.tempt_stick,
            PigTemptKind::Food => &ctx.tempt_food,
        }
    }
    fn state_mut<'a>(&self, ctx: &'a mut PigGoalContext) -> &'a mut TemptState {
        match self.0 {
            PigTemptKind::Stick => &mut ctx.tempt_stick,
            PigTemptKind::Food => &mut ctx.tempt_food,
        }
    }
    fn can_use(&self, ctx: &mut PigGoalContext) -> bool {
        let child = ctx.child;
        let players = &ctx.players;
        match self.0 {
            PigTemptKind::Stick => ctx.tempt_stick.can_use_matching(child, players, |player| {
                player.main_hand_carrot_on_a_stick || player.offhand_carrot_on_a_stick
            }),
            PigTemptKind::Food => ctx.tempt_food.can_use_matching(child, players, |player| {
                player.main_hand_pig_food || player.offhand_pig_food
            }),
        }
    }
}
impl Goal<PigGoalContext> for PigTemptGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut PigGoalContext, _world: &dyn World) -> bool {
        self.can_use(ctx)
    }
    fn can_continue(&mut self, ctx: &mut PigGoalContext, _world: &dyn World) -> bool {
        self.can_use(ctx)
    }
    fn start(&mut self, ctx: &mut PigGoalContext) {
        self.state_mut(ctx).start();
    }
    fn stop(&mut self, ctx: &mut PigGoalContext) {
        self.state_mut(ctx).stop();
        ctx.effects.push(PigGoalEffect::StopNavigation);
    }
    fn tick(&mut self, ctx: &mut PigGoalContext) {
        if let Some(player) = ctx
            .players
            .iter()
            .find(|player| Some(player.id) == self.state(ctx).player_id)
        {
            ctx.effects.push(PigGoalEffect::LookAt {
                target: player.position + DVec3::new(0.0, f64::from(player.eye_height), 0.0),
                y_max: 95.0,
                x_max: 40.0,
            });
        }
        match self.state(ctx).tick(ctx.child, &ctx.players) {
            Some(TemptAction::Navigate(target)) => ctx
                .effects
                .push(PigGoalEffect::NavigateToEntity { target, speed: 1.2 }),
            Some(TemptAction::StopNavigation) => ctx.effects.push(PigGoalEffect::StopNavigation),
            None => {}
        }
    }
}

#[derive(Clone)]
struct LookAtPlayerGoal;
impl Goal<PigGoalContext> for LookAtPlayerGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Look])
    }
    fn can_start(&mut self, ctx: &mut PigGoalContext, world: &dyn World) -> bool {
        let eye_height: f32 = if ctx.child.age < 0 { 0.3825 } else { 0.765 };
        ctx.look_at_player
            .can_use(world, ctx.child.position, eye_height, &ctx.players, &mut ctx.random)
    }
    fn can_continue(&mut self, ctx: &mut PigGoalContext, _world: &dyn World) -> bool {
        ctx.look_at_player
            .can_continue(ctx.child.position, &ctx.players)
    }
    fn start(&mut self, ctx: &mut PigGoalContext) {
        ctx.look_at_player.start(&mut ctx.random);
    }
    fn stop(&mut self, ctx: &mut PigGoalContext) {
        ctx.look_at_player.stop();
    }
    fn tick(&mut self, ctx: &mut PigGoalContext) {
        if let Some(target) = ctx.look_at_player.tick(&ctx.players) {
            ctx.effects.push(PigGoalEffect::LookAt {
                target,
                y_max: 10.0,
                x_max: 40.0,
            });
        }
    }
}

#[derive(Clone)]
struct FloatGoal;
impl Goal<PigGoalContext> for FloatGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Jump])
    }
    fn can_start(&mut self, ctx: &mut PigGoalContext, _world: &dyn World) -> bool {
        ctx.fluid.floatable_height() > 0.4 || ctx.fluid.in_lava()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, ctx: &mut PigGoalContext) {
        if ctx.random.next_float() < 0.8 {
            ctx.effects.push(PigGoalEffect::Jump);
        }
    }
}

#[derive(Clone)]
struct RandomLookGoal;
impl Goal<PigGoalContext> for RandomLookGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[Control::Move, Control::Look])
    }
    fn can_start(&mut self, ctx: &mut PigGoalContext, _world: &dyn World) -> bool {
        RandomLookState::can_use(&mut ctx.random)
    }
    fn can_continue(&mut self, ctx: &mut PigGoalContext, _world: &dyn World) -> bool {
        ctx.random_look.can_continue()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn start(&mut self, ctx: &mut PigGoalContext) {
        ctx.random_look.start(&mut ctx.random);
    }
    fn tick(&mut self, ctx: &mut PigGoalContext) {
        let eye_height = if ctx.child.age < 0 { 0.3825 } else { 0.765 };
        ctx.effects.push(PigGoalEffect::LookAt {
            target: ctx.random_look.tick(ctx.child.position, eye_height),
            y_max: 10.0,
            x_max: 40.0,
        });
    }
}

#[derive(Clone)]
struct FollowParentGoal;
impl Goal<PigGoalContext> for FollowParentGoal {
    fn controls(&self) -> Controls {
        Controls::new(&[])
    }
    fn can_start(&mut self, ctx: &mut PigGoalContext, _world: &dyn World) -> bool {
        ctx.follow_parent.can_use(ctx.child, &ctx.candidates)
    }
    fn can_continue(&mut self, ctx: &mut PigGoalContext, _world: &dyn World) -> bool {
        ctx.follow_parent.can_continue(ctx.child, &ctx.candidates)
    }
    fn start(&mut self, ctx: &mut PigGoalContext) {
        ctx.follow_parent.start();
    }
    fn stop(&mut self, ctx: &mut PigGoalContext) {
        ctx.follow_parent.stop();
    }
    fn tick(&mut self, ctx: &mut PigGoalContext) {
        if let Some(target) = ctx.follow_parent.tick(&ctx.candidates) {
            ctx.effects
                .push(PigGoalEffect::NavigateToEntity { target, speed: 1.1 });
        }
    }
}
