//! The villager's behaviors (pinned 26.3 `VillagerGoalPackages` and the
//! behavior classes it lists), for villagers with no points of interest:
//! the core, idle, play and panic activities.
use super::{Activity, Behavior, Ctx, Remote, Status, Tracker, WalkTarget};
use crate::navigation::plan_walk_path;
use super::{block_of, Memories, Seen, Slot};
use crate::navigation::plan_walk_path_to_any;
use crate::poi::{dist_sqr, JavaPosSet, Occupancy, PoiManager, PoiType};
use crate::stroll::{default_random_position_towards, land_random_position, land_random_position_by};
use crate::villager::Profession;
use minecraftoss_player::Pos;
use minecraftoss_player::collision::colliding_blocks;
use glam::DVec3;
use minecraftoss_player::rng::LegacyRandom;

type Package = Vec<(i32, Box<dyn Behavior>)>;

/// `Villager.BRAIN_PROVIDER`'s activities in the order it adds them, each
/// with its package.
pub(super) fn packages(baby: bool, profession: Profession) -> Vec<(Activity, Package)> {
    let mut out = Vec::new();
    if baby {
        out.push((Activity::Play, play()));
    } else {
        out.push((Activity::Work, work(profession)));
    }
    out.push((Activity::Core, core(profession)));
    out.push((Activity::Meet, meet()));
    out.push((Activity::Rest, rest()));
    out.push((Activity::Idle, idle()));
    out.push((Activity::Panic, panic()));
    out
}

/// `getMeetPackage`: at the meeting point, strolling about it or
/// socializing with a villager near the bell, walking back when away.
fn meet() -> Package {
    vec![
        (
            2,
            boxed(TriggerGate::new(vec![
                (boxed(StrollAroundPoi { memory: PoiMemory::MeetingPoint, speed: 0.4, max_distance: 40, next_ok: 0 }), 2),
                (boxed(SocializeAtBell), 2),
            ])),
        ),
        (10, boxed(ShowTradesToPlayer::default())),
        (10, boxed(SetLookAndInteractWithPlayer)),
        (2, boxed(SetWalkTargetFromBlockMemory { memory: PoiMemory::MeetingPoint, speed: 0.5, close_enough: 6, too_far: 100, too_long: 200 })),
        (3, boxed(Never::new())), // GiveGiftToHero: no hero.
        (3, boxed(ValidateNearbyPoi { kinds: PoiKinds::Meeting, memory: PoiMemory::MeetingPoint })),
        (3, boxed(Gate::ordered(Erase::InteractionTarget, vec![(boxed(TradeWithVillager::default()), 1)]))),
        (5, boxed(full_look())),
        (99, boxed(UpdateActivityFromSchedule)),
    ]
}

/// `getRestPackage`: home to bed and sleep; without a home, to the nearest
/// bed, towards a village, about indoors, or nowhere.
fn rest() -> Package {
    vec![
        (2, boxed(SetWalkTargetFromBlockMemory { memory: PoiMemory::Home, speed: 0.5, close_enough: 1, too_far: 150, too_long: 1200 })),
        (3, boxed(ValidateNearbyPoi { kinds: PoiKinds::Home, memory: PoiMemory::Home })),
        (3, boxed(SleepInBed::default())),
        (
            5,
            boxed(Gate::run_one(
                Entry::NoHome,
                vec![
                    (boxed(SetClosestHomeAsWalkTarget::new(0.5)), 1),
                    (boxed(InsideBrownianWalk { speed: 0.5 }), 4),
                    (boxed(GoToClosestVillage { speed: 0.5, close_enough: 4 }), 2),
                    (boxed(DoNothing::new(20, 40)), 2),
                ],
            )),
        ),
        (5, boxed(minimal_look())),
        (99, boxed(UpdateActivityFromSchedule)),
    ]
}

fn boxed(b: impl Behavior + Clone + 'static) -> Box<dyn Behavior> {
    Box::new(b)
}

/// `getWorkPackage`: at the job site, working (its sound, then a restock
/// check), strolling about or to it, or to the fields; walking back when
/// away. Farmers' harvesting and bone meal need farmland and bone meal.
fn work(profession: Profession) -> Package {
    let farmer = profession == Profession::Farmer;
    vec![
        (5, boxed(minimal_look())),
        (
            5,
            boxed(Gate::run_one(
                Entry::None,
                vec![
                    (boxed(WorkAtPoi { composter: farmer, ..WorkAtPoi::default() }), 7),
                    (boxed(StrollAroundPoi { memory: PoiMemory::JobSite, speed: 0.4, max_distance: 4, next_ok: 0 }), 2),
                    (boxed(StrollToPoi { memory: PoiMemory::JobSite, speed: 0.4, close_enough: 1, max_distance: 10, next_ok: 0 }), 5),
                    (boxed(StrollToPoiList { speed: 0.5, close_enough: 1, max_distance: 6, next_ok: 0 }), 5),
                    (boxed(HarvestFarmland::default()), if farmer { 2 } else { 5 }),
                    (boxed(UseBonemeal::default()), if farmer { 4 } else { 7 }),
                ],
            )),
        ),
        (10, boxed(ShowTradesToPlayer::default())),
        (10, boxed(SetLookAndInteractWithPlayer)),
        (2, boxed(SetWalkTargetFromBlockMemory { memory: PoiMemory::JobSite, speed: 0.5, close_enough: 9, too_far: 100, too_long: 1200 })),
        (3, boxed(Never::new())), // GiveGiftToHero: no hero.
        (99, boxed(UpdateActivityFromSchedule)),
    ]
}

/// `getCorePackage` for a villager of `profession`.
fn core(profession: Profession) -> Package {
    vec![
        (0, boxed(Swim::new(0.8))),
        (0, boxed(InteractWithDoor::default())),
        (0, boxed(LookAtTargetSink::new(45, 90))),
        (0, boxed(PanicTrigger::default())),
        (0, boxed(WakeUp)),
        (0, boxed(Never::new())), // ReactToBell: no bell heard.
        (0, boxed(SetRaidStatus::default())),
        (0, boxed(ValidateNearbyPoi { kinds: PoiKinds::Held(profession), memory: PoiMemory::JobSite })),
        (0, boxed(ValidateNearbyPoi { kinds: PoiKinds::Acquirable(profession), memory: PoiMemory::PotentialJobSite })),
        (1, boxed(MoveToTargetSink::new(150, 250))),
        (2, boxed(PoiCompetitorScan)),
        (3, boxed(LookAndFollowTradingPlayerSink { speed: 0.5, running: false })),
        (5, boxed(GoToWantedItem)),
        (6, boxed(AcquirePoi::new(PoiKinds::Acquirable(profession), PoiMemory::PotentialJobSite, PoiMemory::JobSite, true, false))),
        (7, boxed(GoToPotentialJobSite::new(0.5))),
        (8, boxed(YieldJobSite { speed: 0.5 })),
        (10, boxed(AcquirePoi::new(PoiKinds::Home, PoiMemory::Home, PoiMemory::Home, false, true))),
        (10, boxed(AcquirePoi::new(PoiKinds::Meeting, PoiMemory::MeetingPoint, PoiMemory::MeetingPoint, true, false))),
        (10, boxed(AssignProfessionFromJobSite)),
        (10, boxed(ResetProfession)),
    ]
}

/// `getIdlePackage`.
fn idle() -> Package {
    vec![
        (
            2,
            boxed(Gate::run_one(
                Entry::None,
                vec![
                    (boxed(InteractWith::villager(0.5, false)), 2),
                    (boxed(InteractWith::villager(0.5, true)), 1),
                    (boxed(InteractWith::cat(0.5)), 1),
                    (boxed(VillageBoundRandomStroll::new(0.5, 10, 7)), 1),
                    (boxed(SetWalkTargetFromLookTarget::new(0.5, 2)), 1),
                    (boxed(JumpOnBed::new(0.5)), 1),
                    (boxed(DoNothing::new(30, 60)), 1),
                ],
            )),
        ),
        (3, boxed(Never::new())), // GiveGiftToHero: no hero.
        (3, boxed(SetLookAndInteractWithPlayer)),
        (3, boxed(ShowTradesToPlayer::default())),
        (3, boxed(Gate::ordered(Erase::InteractionTarget, vec![(boxed(TradeWithVillager::default()), 1)]))),
        (3, boxed(Gate::ordered(Erase::BreedTarget, vec![(boxed(VillagerMakeLove::default()), 1)]))),
        (5, boxed(full_look())),
        (99, boxed(UpdateActivityFromSchedule)),
    ]
}

/// `getPlayPackage`.
fn play() -> Package {
    vec![
        (0, boxed(MoveToTargetSink::new(80, 120))),
        (5, boxed(full_look())),
        (5, boxed(PlayTagWithOtherKids)),
        (
            5,
            boxed(Gate::run_one(
                Entry::NoBabiesSeen,
                vec![
                    (boxed(InteractWith::villager(0.5, false)), 2),
                    (boxed(InteractWith::cat(0.5)), 1),
                    (boxed(VillageBoundRandomStroll::new(0.5, 10, 7)), 1),
                    (boxed(SetWalkTargetFromLookTarget::new(0.5, 2)), 1),
                    (boxed(JumpOnBed::new(0.5)), 2),
                    (boxed(DoNothing::new(20, 40)), 2),
                ],
            )),
        ),
        (99, boxed(UpdateActivityFromSchedule)),
    ]
}

/// `getPanicPackage`.
fn panic() -> Package {
    let runaway = 0.5_f32 * 1.5;
    vec![
        (0, boxed(VillagerCalmDown)),
        (1, boxed(WalkAwayFrom { from: Flee::Hostile, speed: runaway })),
        (1, boxed(WalkAwayFrom { from: Flee::HurtBy, speed: runaway })),
        (3, boxed(VillageBoundRandomStroll::new(runaway, 2, 2))),
        (5, boxed(minimal_look())),
    ]
}

/// `getFullLookBehavior`.
fn full_look() -> Gate {
    Gate::run_one(
        Entry::None,
        vec![
            (boxed(SetEntityLookTarget(LookFor::Kind("minecraft:cat"))), 8),
            (boxed(SetEntityLookTarget(LookFor::Kind("minecraft:villager"))), 2),
            (boxed(SetEntityLookTarget(LookFor::Kind("minecraft:player"))), 2),
            (boxed(SetEntityLookTarget(LookFor::Category("creature"))), 1),
            (boxed(SetEntityLookTarget(LookFor::Category("water_creature"))), 1),
            (boxed(SetEntityLookTarget(LookFor::Category("axolotls"))), 1),
            (boxed(SetEntityLookTarget(LookFor::Category("underground_water_creature"))), 1),
            (boxed(SetEntityLookTarget(LookFor::Category("water_ambient"))), 1),
            (boxed(SetEntityLookTarget(LookFor::Category("monster"))), 1),
            (boxed(DoNothing::new(30, 60)), 2),
        ],
    )
}

/// `getMinimalLookBehavior`.
fn minimal_look() -> Gate {
    Gate::run_one(
        Entry::None,
        vec![
            (boxed(SetEntityLookTarget(LookFor::Kind("minecraft:villager"))), 2),
            (boxed(SetEntityLookTarget(LookFor::Kind("minecraft:player"))), 2),
            (boxed(DoNothing::new(30, 60)), 8),
        ],
    )
}

/// `Behavior.tryStart`'s running time: `min + level.random.nextInt(max + 1 - min)`.
fn duration(ctx: &mut Ctx, min: i32, max: i32) -> i64 {
    // Java's int arithmetic: `Integer.MAX_VALUE + 1 - Integer.MAX_VALUE` is 1.
    i64::from(min) + i64::from(ctx.level_random.next_int(max.wrapping_add(1).wrapping_sub(min) as u32))
}

/// A behavior whose conditions cannot hold here (it needs a point of
/// interest, a player, an item or a bed this villager does not have).
#[derive(Clone)]
struct Never;

impl Never {
    fn new() -> Self {
        Self
    }
}

impl Behavior for Never {
    fn status(&self) -> Status {
        Status::Stopped
    }
    fn try_start(&mut self, _: &mut Ctx, _: i64) -> bool {
        false
    }
    fn tick_or_stop(&mut self, _: &mut Ctx, _: i64) {}
    fn do_stop(&mut self, _: &mut Ctx, _: i64) {}
    fn describe(&self) -> String {
        "Never".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// A one-shot behavior (`OneShot`): it runs when its trigger fires and
/// stops at its next tick.
macro_rules! one_shot {
    ($name:ident) => {
        impl Behavior for $name {
            fn status(&self) -> Status {
                Status::Stopped
            }
            fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
                self.trigger(ctx, time)
            }
            fn tick_or_stop(&mut self, _: &mut Ctx, _: i64) {}
            fn do_stop(&mut self, _: &mut Ctx, _: i64) {}
            fn describe(&self) -> String {
                stringify!($name).into()
            }
            fn clone_box(&self) -> Box<dyn Behavior> {
                Box::new(self.clone())
            }
        }
    };
}

// A one-shot stops within the tick it starts in, so it never shows as
// running between ticks; its brief running only matters to a gate, which
// asks whether it started ([`Gate`] treats a fired one-shot as a child
// that ran and stopped).

/// `Swim(0.8)`.
#[derive(Clone)]
struct Swim {
    chance: f32,
    status: Status,
    end: i64,
}

impl Swim {
    fn new(chance: f32) -> Self {
        Self { chance, status: Status::Stopped, end: 0 }
    }

    /// `isInFluidDeeperThan(getFluidJumpThreshold(), #entity_floatable) || isInLava`.
    fn applies(ctx: &Ctx) -> bool {
        let threshold = if ctx.me.eye_height < 0.4 { 0.0 } else { 0.4 };
        ctx.me.fluid.water_height > threshold || ctx.me.fluid.in_lava()
    }
}

impl Behavior for Swim {
    fn status(&self) -> Status {
        self.status
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        if !Self::applies(ctx) {
            return false;
        }
        self.status = Status::Running;
        self.end = time + duration(ctx, 60, 60);
        true
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        if time <= self.end && Self::applies(ctx) {
            if ctx.me.random.next_float() < self.chance {
                *ctx.me.jump = true;
            }
        } else {
            self.do_stop(ctx, time);
        }
    }
    fn do_stop(&mut self, _: &mut Ctx, _: i64) {
        self.status = Status::Stopped;
    }
    fn describe(&self) -> String {
        "Swim".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `LookAtTargetSink(45, 90)`: while the look target stays in sight, the
/// head turns to it; stopping forgets it.
#[derive(Clone)]
struct LookAtTargetSink {
    min: i32,
    max: i32,
    status: Status,
    end: i64,
}

impl LookAtTargetSink {
    fn new(min: i32, max: i32) -> Self {
        Self { min, max, status: Status::Stopped, end: 0 }
    }

    /// `PositionTracker.isVisibleBy`: an entity that is not living (an
    /// item) always is.
    fn visible(ctx: &mut Ctx) -> bool {
        match ctx.mem.look_target.get().copied() {
            Some(Tracker::Entity { id, .. }) if ctx.seen(id).is_some_and(|s| s.kind == "minecraft:item") => true,
            Some(Tracker::Entity { id, .. }) => ctx.seen(id).is_some_and(|s| s.alive) && ctx.sees(id),
            Some(Tracker::Block(_)) => true,
            None => false,
        }
    }
}

impl Behavior for LookAtTargetSink {
    fn status(&self) -> Status {
        self.status
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        if !ctx.mem.look_target.present() {
            return false;
        }
        self.status = Status::Running;
        self.end = time + duration(ctx, self.min, self.max);
        true
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        if time <= self.end && Self::visible(ctx) {
            if let Some(target) = ctx.mem.look_target.get().copied().and_then(|t| t.position(ctx.others)) {
                *ctx.me.look_at = Some(target);
            }
        } else {
            self.do_stop(ctx, time);
        }
    }
    fn do_stop(&mut self, ctx: &mut Ctx, _: i64) {
        self.status = Status::Stopped;
        ctx.mem.look_target.erase();
    }
    fn describe(&self) -> String {
        "LookAtTargetSink".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `VillagerPanicTrigger`: it starts every tick (drawing its time), turns
/// to panic when hurt or near a hostile, and stops unless still so.
#[derive(Clone, Default)]
struct PanicTrigger {
    running: bool,
    end: i64,
}

impl PanicTrigger {
    fn afraid(ctx: &Ctx) -> bool {
        ctx.mem.hurt_by.present() || ctx.mem.nearest_hostile.present()
    }
}

impl Behavior for PanicTrigger {
    fn status(&self) -> Status {
        if self.running {
            Status::Running
        } else {
            Status::Stopped
        }
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        self.running = true;
        self.end = time + duration(ctx, 60, 60);
        if Self::afraid(ctx) {
            if !ctx.activities.is_active(Activity::Panic) {
                ctx.mem.path.erase();
                ctx.mem.walk_target.erase();
                ctx.mem.look_target.erase();
                ctx.mem.breed_target.erase();
                ctx.mem.interaction_target.erase();
            }
            ctx.activities.set_active_if_possible(Activity::Panic, ctx.mem);
        }
        true
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        if time <= self.end && Self::afraid(ctx) {
            // Every hundred ticks three frightened villagers that want a
            // golem summon one.
            if time % 100 == 0 {
                spawn_golem_if_needed(ctx, time, 3);
            }
        } else {
            self.do_stop(ctx, time);
        }
    }
    fn do_stop(&mut self, _: &mut Ctx, _: i64) {
        self.running = false;
    }
    fn describe(&self) -> String {
        "VillagerPanicTrigger".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `Villager.spawnGolemIfNeeded`: a villager that wants a golem, with at
/// least `needed` villagers wanting one within its box grown by ten (itself
/// among them, five counted at most), tries to summon one
/// (`SpawnUtil.trySpawnMob`); if it appears, every villager about
/// detects it.
fn spawn_golem_if_needed(ctx: &mut Ctx, time: i64, needed: usize) {
    let wants = ctx.mem.last_slept.get().is_some_and(|&slept| time - slept < 24_000) && !ctx.mem.golem_detected_recently.present();
    if !wants {
        return;
    }
    let p = ctx.me.body.position;
    let half = f64::from(ctx.me.body.width) / 2.0;
    let (min, max) = (p - DVec3::new(half + 10.0, 10.0, half + 10.0), p + DVec3::new(half + 10.0, f64::from(ctx.me.body.height) + 10.0, half + 10.0));
    let meets = |s: &Seen, min: DVec3, max: DVec3| {
        let h = f64::from(s.width) / 2.0;
        let q = s.position;
        q.x - h < max.x && q.x + h > min.x && q.y < max.y && q.y + f64::from(s.height) > min.y && q.z - h < max.z && q.z + h > min.z
    };
    let nearby: Vec<u64> = ctx.others.iter().filter(|s| s.kind == "minecraft:villager" && meets(s, min, max)).map(|s| s.id).collect();
    let wanting = 1 + ctx.others.iter().filter(|s| nearby.contains(&s.id) && s.wants_golem).count();
    if wanting.min(5) < needed {
        return;
    }
    // `isUnobstructed`: no living thing's box, its own included, in the way.
    let me = (p - DVec3::new(half, 0.0, half), p + DVec3::new(half, f64::from(ctx.me.body.height), half));
    let others = ctx.others;
    let blocked = |min: DVec3, max: DVec3| {
        let overlap = |(a, b): (DVec3, DVec3)| a.x < max.x && b.x > min.x && a.y < max.y && b.y > min.y && a.z < max.z && b.z > min.z;
        overlap(me)
            || others.iter().filter(|s| s.kind != "minecraft:item").any(|s| {
                let h = f64::from(s.width) / 2.0;
                overlap((s.position - DVec3::new(h, 0.0, h), s.position + DVec3::new(h, f64::from(s.height), h)))
            })
    };
    let start = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
    if let Some(golem) = crate::golem_spawn::try_spawn_golem(ctx.world, start, ctx.level_random, blocked) {
        ctx.remote.push(Remote::SpawnGolem(golem));
        ctx.mem.golem_detected_recently.set_for((), 599);
        for villager in nearby {
            ctx.remote.push(Remote::GolemDetected { villager });
        }
    }
}

/// `SetRaidStatus`: a one in twenty chance each tick of looking for a raid.
#[derive(Clone, Default)]
struct SetRaidStatus;

impl SetRaidStatus {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        ctx.level_random.next_int(20) == 0
    }
}
one_shot!(SetRaidStatus);

/// A memory of a point of interest (a `GlobalPos`; this dimension's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PoiMemory {
    Home,
    JobSite,
    PotentialJobSite,
    MeetingPoint,
}

impl PoiMemory {
    fn slot(self, mem: &mut Memories) -> &mut Slot<Pos> {
        match self {
            Self::Home => &mut mem.home,
            Self::JobSite => &mut mem.job_site,
            Self::PotentialJobSite => &mut mem.potential_job_site,
            Self::MeetingPoint => &mut mem.meeting_point,
        }
    }

    fn get(self, mem: &Memories) -> Option<Pos> {
        match self {
            Self::Home => mem.home.get(),
            Self::JobSite => mem.job_site.get(),
            Self::PotentialJobSite => mem.potential_job_site.get(),
            Self::MeetingPoint => mem.meeting_point.get(),
        }
        .copied()
    }
}

/// The point-of-interest types a behavior asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PoiKinds {
    Home,
    Meeting,
    /// `heldJobSite` of a profession (`PoiType.NONE` for the unemployed).
    Held(Profession),
    /// `acquirableJobSite` of a profession (any job site unemployed).
    Acquirable(Profession),
}

impl PoiKinds {
    fn matches(self, kind: PoiType) -> bool {
        match self {
            Self::Home => kind == PoiType::Home,
            Self::Meeting => kind == PoiType::Meeting,
            Self::Held(p) => p.holds(kind),
            Self::Acquirable(p) => p.can_acquire(kind),
        }
    }
}

/// `VillagerGoalPackages.validateBedPoi`: a bed nobody sleeps in.
fn bed_free(world: &dyn minecraftoss_player::World, pos: Pos) -> bool {
    world.block(pos).is_some_and(|b| is_bed(&b.id) && b.property("occupied") != Some("true"))
}

/// `#minecraft:beds` (`villagers_can_sleep_on_bed`, `villager_babies_can_jump_on_bed`).
fn is_bed(id: &str) -> bool {
    id.strip_prefix("minecraft:").and_then(|n| n.strip_suffix("_bed")).is_some_and(|color| {
        matches!(
            color,
            "white" | "orange" | "magenta" | "light_blue" | "yellow" | "lime" | "pink" | "gray" | "light_gray" | "cyan" | "purple" | "blue" | "brown" | "green" | "red" | "black"
        )
    })
}

/// `AcquirePoi.JitteredLinearRetry`: when a spot may be tried again, the
/// wait growing by 40 to 79 ticks a try (to at most 400), from the level's
/// random.
#[derive(Clone, Copy, Debug)]
struct Retry {
    previous: i64,
    next: i64,
    delay: i32,
}

impl Retry {
    fn new(random: &mut LegacyRandom, time: i64) -> Self {
        let mut retry = Self { previous: 0, next: 0, delay: 0 };
        retry.mark(random, time);
        retry
    }

    fn mark(&mut self, random: &mut LegacyRandom, time: i64) {
        self.previous = time;
        self.delay = (self.delay + random.next_int(40) as i32 + 40).min(400);
        self.next = time + i64::from(self.delay);
    }
}

/// `AcquirePoi`: with its memory (and the one it validates) empty, every
/// 20 to 39 ticks (the first wait up to 19, both from the level's random)
/// it takes the five nearest points of interest within 48 blocks that have
/// room and are not waiting out a retry, keeps the valid ones, and paths
/// to them together; reaching one, it takes a ticket there and remembers
/// it, otherwise each tried spot waits before its next try.
#[derive(Clone)]
struct AcquirePoi {
    kinds: PoiKinds,
    acquire: PoiMemory,
    validate: PoiMemory,
    only_adults: bool,
    /// `validateBedPoi`.
    bed: bool,
    next_start: i64,
    retries: Vec<(Pos, Retry)>,
}

impl AcquirePoi {
    fn new(kinds: PoiKinds, acquire: PoiMemory, validate: PoiMemory, only_adults: bool, bed: bool) -> Self {
        Self { kinds, acquire, validate, only_adults, bed, next_start: 0, retries: Vec::new() }
    }

    fn trigger(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        if self.acquire.get(ctx.mem).is_some() || self.validate.get(ctx.mem).is_some() {
            return false;
        }
        if self.only_adults && ctx.me.baby {
            return false;
        }
        if self.next_start == 0 {
            self.next_start = ctx.time + i64::from(ctx.level_random.next_int(20));
            return false;
        }
        if ctx.time < self.next_start {
            return false;
        }
        self.next_start = time + 20 + i64::from(ctx.level_random.next_int(20));
        self.retries.retain(|(_, r)| time - r.previous < 400);
        let here = block_of(ctx.me.body.position);
        let kinds = self.kinds;
        let mut candidates: Vec<(PoiType, Pos)> = Vec::new();
        for record in ctx.pois.in_range(&|t| kinds.matches(t), here, 48, Occupancy::HasSpace) {
            let pass = match self.retries.iter_mut().find(|(p, _)| *p == record.pos) {
                None => true,
                Some((_, retry)) if time >= retry.next => {
                    retry.mark(ctx.level_random, time);
                    true
                }
                Some(_) => false,
            };
            if pass {
                candidates.push((record.kind, record.pos));
            }
        }
        candidates.sort_by(|a, b| dist_sqr(a.1, here).total_cmp(&dist_sqr(b.1, here)));
        candidates.truncate(5);
        let world = ctx.world;
        if self.bed {
            candidates.retain(|&(_, pos)| bed_free(world, pos));
        }
        let mut range = 1;
        let mut set = JavaPosSet::default();
        for &(kind, pos) in &candidates {
            range = range.max(kind.valid_range());
            set.insert(pos);
        }
        let targets: Vec<Pos> = set.iter().collect();
        let me = &mut ctx.me;
        let path = plan_walk_path_to_any(me.body, me.navigation, world, me.profile, me.fluid, &targets, range);
        match path.filter(|p| p.reached()).and_then(|p| p.target()) {
            Some(target) => {
                if ctx.pois.kind(target).is_some() {
                    ctx.pois.take(&|t| kinds.matches(t), |_, p| p == target, target, 1);
                    self.acquire.slot(ctx.mem).set(target);
                    self.retries.clear();
                }
            }
            None => {
                // Vanilla goes through its set of (type, position) pairs,
                // whose order follows identity hashes; the positions' set
                // order stands in.
                for pos in targets {
                    if !self.retries.iter().any(|(p, _)| *p == pos) {
                        let retry = Retry::new(ctx.level_random, time);
                        self.retries.push((pos, retry));
                    }
                }
            }
        }
        true
    }
}
one_shot!(AcquirePoi);

/// `ValidateNearbyPoi`: a remembered point of interest within 16 blocks
/// (of its centre) that is gone or no longer of its kind is forgotten, and
/// so is a bed someone else sleeps in (its ticket freed unless a villager
/// sleeps there).
#[derive(Clone)]
struct ValidateNearbyPoi {
    kinds: PoiKinds,
    memory: PoiMemory,
}

impl ValidateNearbyPoi {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        let Some(pos) = self.memory.get(ctx.mem) else { return false };
        let me = ctx.me.body.position;
        let center = DVec3::new(f64::from(pos.0) + 0.5, f64::from(pos.1) + 0.5, f64::from(pos.2) + 0.5);
        if center.distance_squared(me) >= 16.0 * 16.0 {
            return false;
        }
        let kinds = self.kinds;
        if !ctx.pois.exists(pos, |t| kinds.matches(t)) {
            self.memory.slot(ctx.mem).erase();
        } else if ctx.world.block(pos).is_some_and(|b| is_bed(&b.id) && b.property("occupied") == Some("true")) {
            // `bedIsOccupied` (it is not the sleeper), and unless a
            // villager sleeps in it (`bedIsOccupiedByVillager`: a sleeping
            // villager's box meets the block) the bed's ticket goes free.
            if ctx.me.sleeping.is_none() {
                self.memory.slot(ctx.mem).erase();
                let meets = |s: &Seen| {
                    let h = f64::from(s.width) / 2.0;
                    let (x, y, z) = (f64::from(pos.0), f64::from(pos.1), f64::from(pos.2));
                    s.position.x - h < x + 1.0 && s.position.x + h > x && s.position.y < y + 1.0 && s.position.y + f64::from(s.height) > y && s.position.z - h < z + 1.0 && s.position.z + h > z
                };
                if !ctx.others.iter().any(|s| s.kind == "minecraft:villager" && s.sleeping && meets(s)) {
                    ctx.pois.release(pos);
                }
            }
        }
        true
    }
}
one_shot!(ValidateNearbyPoi);

/// `JumpOnBed`: a baby that knows a bed and has nowhere to walk goes to
/// it (100 ticks to get there) and jumps on it three to six times, five
/// ticks apart.
#[derive(Clone)]
struct JumpOnBed {
    speed: f32,
    running: bool,
    target: Option<Pos>,
    time_to_reach: i32,
    jumps: i32,
    cooldown: i32,
}

impl JumpOnBed {
    fn new(speed: f32) -> Self {
        Self { speed, running: false, target: None, time_to_reach: 0, jumps: 0, cooldown: 0 }
    }

    fn jumpable(world: &dyn minecraftoss_player::World, pos: Pos) -> bool {
        world.block(pos).is_some_and(|b| is_bed(&b.id))
    }

    fn on_or_over_bed(ctx: &Ctx) -> bool {
        let at = block_of(ctx.me.body.position);
        Self::jumpable(ctx.world, at) || Self::jumpable(ctx.world, (at.0, at.1 - 1, at.2))
    }
}

impl Behavior for JumpOnBed {
    fn status(&self) -> Status {
        if self.running {
            Status::Running
        } else {
            Status::Stopped
        }
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        let Some(bed) = ctx.mem.nearest_bed.get().copied() else { return false };
        if ctx.mem.walk_target.present() || !ctx.me.baby {
            return false;
        }
        self.running = true;
        let _ = time + duration(ctx, 60, 60);
        self.target = Some(bed);
        self.time_to_reach = 100;
        self.jumps = 3 + ctx.level_random.next_int(4) as i32;
        self.cooldown = 0;
        ctx.mem.walk_target.set(WalkTarget { target: Tracker::Block(bed), speed: self.speed, close_enough: 0 });
        true
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        let over = Self::on_or_over_bed(ctx);
        let tired = (!over && self.time_to_reach <= 0) || (over && self.jumps <= 0);
        let usable = ctx.me.baby && self.target.is_some_and(|bed| Self::jumpable(ctx.world, bed)) && !tired;
        if !usable {
            self.do_stop(ctx, time);
            return;
        }
        if !over {
            self.time_to_reach -= 1;
        } else if self.cooldown > 0 {
            self.cooldown -= 1;
        } else if Self::jumpable(ctx.world, block_of(ctx.me.body.position)) {
            *ctx.me.jump = true;
            self.jumps -= 1;
            self.cooldown = 5;
        }
    }
    fn do_stop(&mut self, _: &mut Ctx, _: i64) {
        self.running = false;
        self.target = None;
        self.time_to_reach = 0;
        self.jumps = 0;
        self.cooldown = 0;
    }
    fn describe(&self) -> String {
        "JumpOnBed".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `BehaviorUtils.findSectionClosestToVillage`.
fn closest_village_section(pois: &mut PoiManager, center: Pos, radius: i32) -> Pos {
    pois.closest_village_section(center, radius)
}

/// `MoveToTargetSink`: walks the path to the walk target, forgetting it
/// once reached, and noting since when it cannot be reached.
#[derive(Clone)]
struct MoveToTargetSink {
    min: i32,
    max: i32,
    status: Status,
    end: i64,
    cooldown: i32,
    /// The path object this holds, and its planned nodes.
    path: Option<(u64, crate::navigation::PlannedPath)>,
    last_target: Option<minecraftoss_player::Pos>,
    speed: f32,
}

impl MoveToTargetSink {
    fn new(min: i32, max: i32) -> Self {
        Self { min, max, status: Status::Stopped, end: 0, cooldown: 0, path: None, last_target: None, speed: 0.0 }
    }

    /// `reachedTarget`: within the close-enough Manhattan distance.
    fn reached(ctx: &Ctx, walk: &WalkTarget) -> bool {
        let p = ctx.me.body.position;
        let me = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        walk.target.block(ctx.others).is_some_and(|(x, y, z)| (x - me.0).abs() + (y - me.1).abs() + (z - me.2).abs() <= walk.close_enough)
    }

    /// `navigation.createPath(target, 0)`: the current path object when it
    /// already heads there, else a new one.
    fn create_path(ctx: &mut Ctx, target: DVec3) -> Option<(u64, crate::navigation::PlannedPath)> {
        let before = ctx.me.navigation.target_pos;
        let current = ctx.me.paths.current;
        let planned = plan_walk_path(ctx.me.body, ctx.me.navigation, ctx.world, ctx.me.profile, ctx.me.fluid, target, 0)?;
        let same = !ctx.me.navigation.is_done() && current.is_some() && before == ctx.me.navigation.target_pos && planned.nodes() == ctx.me.navigation.nodes.as_slice();
        let id = match (same, current) {
            (true, Some(id)) => id,
            _ => ctx.me.paths.fresh(),
        };
        let target = ctx.me.navigation.target_pos.unwrap_or_default();
        ctx.me.paths.known.insert(id, (target, planned.nodes().len(), planned.next()));
        Some((id, planned))
    }

    /// `tryComputePath`.
    fn compute(&mut self, ctx: &mut Ctx, walk: &WalkTarget, time: i64) -> bool {
        let Some(target) = walk.target.block(ctx.others) else { return false };
        let at = DVec3::new(f64::from(target.0), f64::from(target.1), f64::from(target.2));
        self.path = Self::create_path(ctx, at);
        self.speed = walk.speed;
        if Self::reached(ctx, walk) {
            ctx.mem.cant_reach_walk_target_since.erase();
        } else {
            let can_reach = self.path.as_ref().is_some_and(|(_, p)| p.reached());
            if can_reach {
                ctx.mem.cant_reach_walk_target_since.erase();
            } else if !ctx.mem.cant_reach_walk_target_since.present() {
                ctx.mem.cant_reach_walk_target_since.set(time);
            }
            if self.path.is_some() {
                return true;
            }
            // `DefaultRandomPos.getPosTowards(body, 10, 7, bottom centre, pi/2)`.
            let towards = DVec3::new(f64::from(target.0) + 0.5, f64::from(target.1), f64::from(target.2) + 0.5);
            let step = default_random_position_towards(ctx.world, ctx.me.profile, ctx.me.body.position, 10, 7, towards, std::f64::consts::FRAC_PI_2 as f32, ctx.me.random);
            if let Some(step) = step {
                self.path = Self::create_path(ctx, step);
                return self.path.is_some();
            }
        }
        false
    }

    /// `navigation.moveTo(path, speed)`, keeping the navigation's own path
    /// object when the nodes are the same.
    fn start(&mut self, ctx: &mut Ctx) {
        let Some((id, planned)) = self.path.clone() else {
            ctx.me.navigation.stop();
            ctx.me.paths.current = None;
            ctx.mem.path.erase();
            return;
        };
        ctx.mem.path.set(id);
        let same = ctx.me.paths.current.is_some() && ctx.me.navigation.nodes.as_slice() == planned.nodes();
        if !same {
            ctx.me.paths.current = Some(id);
        }
        let speed = f64::from(self.speed);
        ctx.me.navigation.move_to(Some(planned), speed, ctx.me.body.position);
    }
}

impl Behavior for MoveToTargetSink {
    fn status(&self) -> Status {
        self.status
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        if ctx.mem.path.present() || !ctx.mem.walk_target.present() {
            return false;
        }
        if self.cooldown > 0 {
            self.cooldown -= 1;
            return false;
        }
        let walk = *ctx.mem.walk_target.get().unwrap();
        let reached = Self::reached(ctx, &walk);
        if !reached && self.compute(ctx, &walk, ctx.time) {
            self.last_target = walk.target.block(ctx.others);
            self.status = Status::Running;
            self.end = time + duration(ctx, self.min, self.max);
            self.start(ctx);
            return true;
        }
        ctx.mem.walk_target.erase();
        if reached {
            ctx.mem.cant_reach_walk_target_since.erase();
        }
        false
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        let still = time <= self.end
            && self.path.is_some()
            && self.last_target.is_some()
            && match ctx.mem.walk_target.get().copied() {
                Some(walk) => !ctx.me.navigation.is_done() && !Self::reached(ctx, &walk),
                None => false,
            };
        if !still {
            self.do_stop(ctx, time);
            return;
        }
        // The navigation's path object, when it changed.
        let current = ctx.me.paths.current;
        if self.path.as_ref().map(|(id, _)| *id) != current {
            match current {
                Some(id) => {
                    ctx.mem.path.set(id);
                    let planned = crate::navigation::PlannedPath::of(ctx.me.navigation);
                    self.path = Some((id, planned));
                }
                None => {
                    ctx.mem.path.erase();
                    self.path = None;
                }
            }
        }
        if self.path.is_some() {
            if let (Some(last), Some(walk)) = (self.last_target, ctx.mem.walk_target.get().copied()) {
                if let Some(now) = walk.target.block(ctx.others) {
                    let (dx, dy, dz) = (f64::from(now.0 - last.0), f64::from(now.1 - last.1), f64::from(now.2 - last.2));
                    if dx * dx + dy * dy + dz * dz > 4.0 && self.compute(ctx, &walk, ctx.time) {
                        self.last_target = Some(now);
                        self.start(ctx);
                    }
                }
            }
        }
    }
    fn do_stop(&mut self, ctx: &mut Ctx, _: i64) {
        self.status = Status::Stopped;
        if let Some(walk) = ctx.mem.walk_target.get().copied() {
            if !Self::reached(ctx, &walk) && ctx.me.navigation.is_stuck() {
                self.cooldown = ctx.level_random.next_int(40) as i32;
            }
        }
        ctx.me.navigation.stop();
        ctx.me.paths.current = None;
        ctx.mem.walk_target.erase();
        ctx.mem.path.erase();
        self.path = None;
    }
    fn describe(&self) -> String {
        "MoveToTargetSink".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// A gate's entry condition.
#[derive(Clone, Copy)]
enum Entry {
    None,
    /// `VISIBLE_VILLAGER_BABIES` absent.
    NoBabiesSeen,
    /// `HOME` absent.
    NoHome,
}

/// What a gate forgets when it stops.
#[derive(Clone, Copy)]
enum Erase {
    Nothing,
    InteractionTarget,
    BreedTarget,
}

/// `GateBehavior` running one child (`RUN_ONE`), shuffled (`RunOne`) or in
/// order; it starts whenever its entry holds and stops once no child runs.
#[derive(Clone)]
struct Gate {
    entry: Entry,
    erase: Erase,
    shuffled: bool,
    /// The children with their weights, in the list's current order
    /// (shuffles sort it in place).
    children: Vec<(Box<dyn Behavior>, i32)>,
    random: LegacyRandom,
    running: bool,
    /// A child one-shot that fired this tick (running until the gate's tick).
    fired: bool,
}

impl Gate {
    fn run_one(entry: Entry, children: Vec<(Box<dyn Behavior>, i32)>) -> Self {
        Self { entry, erase: Erase::Nothing, shuffled: true, children, random: LegacyRandom::new(0), running: false, fired: false }
    }

    fn ordered(erase: Erase, children: Vec<(Box<dyn Behavior>, i32)>) -> Self {
        Self { entry: Entry::None, erase, shuffled: false, children, random: LegacyRandom::new(0), running: false, fired: false }
    }

    /// `ShufflingList.shuffle`: each entry weighs `-pow(nextFloat, 1/weight)`,
    /// then a stable sort by weight.
    fn shuffle(&mut self) {
        let mut keyed: Vec<(f64, (Box<dyn Behavior>, i32))> = std::mem::take(&mut self.children)
            .into_iter()
            .map(|(b, w)| (-f64::from(self.random.next_float()).powf(f64::from(1.0_f32 / w as f32)), (b, w)))
            .collect();
        keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        self.children = keyed.into_iter().map(|(_, c)| c).collect();
    }

    fn any_running(&self) -> bool {
        self.fired || self.children.iter().any(|(b, _)| b.status() == Status::Running)
    }
}

impl Behavior for Gate {
    fn status(&self) -> Status {
        if self.running {
            Status::Running
        } else {
            Status::Stopped
        }
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        if matches!(self.entry, Entry::NoBabiesSeen) && ctx.mem.visible_villager_babies.present() {
            return false;
        }
        if matches!(self.entry, Entry::NoHome) && ctx.mem.home.present() {
            return false;
        }
        self.running = true;
        if self.shuffled {
            self.shuffle();
        }
        // `RUN_ONE`: the first stopped child that starts.
        for (child, _) in &mut self.children {
            if child.status() == Status::Stopped && child.try_start(ctx, time) {
                // A one-shot stays "stopped" here; it ran.
                if child.status() == Status::Stopped {
                    self.fired = true;
                }
                break;
            }
        }
        true
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        // A fired one-shot stops at its tick.
        self.fired = false;
        for (child, _) in &mut self.children {
            if child.status() == Status::Running {
                child.tick_or_stop(ctx, time);
            }
        }
        if !self.any_running() {
            self.do_stop(ctx, time);
        }
    }
    fn do_stop(&mut self, ctx: &mut Ctx, time: i64) {
        self.running = false;
        self.fired = false;
        for (child, _) in &mut self.children {
            if child.status() == Status::Running {
                child.do_stop(ctx, time);
            }
        }
        match self.erase {
            Erase::Nothing => {}
            Erase::InteractionTarget => ctx.mem.interaction_target.erase(),
            Erase::BreedTarget => ctx.mem.breed_target.erase(),
        }
    }
    fn describe(&self) -> String {
        let mut names: Vec<String> = self.children.iter().filter(|(b, _)| b.status() == Status::Running).map(|(b, _)| b.describe()).collect();
        names.sort();
        names.dedup();
        format!("{}:{}", if self.shuffled { "RunOne" } else { "GateBehavior" }, names.join(","))
    }
    fn shuffles(&mut self) -> Vec<&mut LegacyRandom> {
        vec![&mut self.random]
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `DoNothing(min, max)`.
#[derive(Clone)]
struct DoNothing {
    min: i32,
    max: i32,
    status: Status,
    end: i64,
}

impl DoNothing {
    fn new(min: i32, max: i32) -> Self {
        Self { min, max, status: Status::Stopped, end: 0 }
    }
}

impl Behavior for DoNothing {
    fn status(&self) -> Status {
        self.status
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        self.status = Status::Running;
        self.end = time + duration(ctx, self.min, self.max);
        true
    }
    fn tick_or_stop(&mut self, _: &mut Ctx, time: i64) {
        if time > self.end {
            self.status = Status::Stopped;
        }
    }
    fn do_stop(&mut self, _: &mut Ctx, _: i64) {
        self.status = Status::Stopped;
    }
    fn describe(&self) -> String {
        "DoNothing".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `InteractWith`: toward the nearest seen villager (or cat) within 8; to
/// breed (`AgeableMob::canBreed` for both), one that can breed, as the
/// breed target.
#[derive(Clone)]
struct InteractWith {
    kind: &'static str,
    speed: f32,
    breed: bool,
}

impl InteractWith {
    fn villager(speed: f32, breed: bool) -> Self {
        Self { kind: "minecraft:villager", speed, breed }
    }

    fn cat(speed: f32) -> Self {
        Self { kind: "minecraft:cat", speed, breed: false }
    }

    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        if ctx.mem.walk_target.present() || !ctx.mem.visible_mobs.present() {
            return false;
        }
        let (kind, breed) = (self.kind, self.breed);
        if breed && !ctx.me.can_breed() {
            return false;
        }
        if ctx.closest_visible(|s| s.kind == kind && (!breed || s.can_breed)).is_none() {
            return false;
        }
        let me = ctx.me.body.position;
        if let Some(id) = ctx.closest_visible(|s| s.position.distance_squared(me) <= 64.0 && s.kind == kind && (!breed || s.can_breed)) {
            if breed {
                ctx.mem.breed_target.set(id);
            } else {
                ctx.mem.interaction_target.set(id);
            }
            ctx.mem.look_target.set(Tracker::Entity { id, eyes: true });
            ctx.mem.walk_target.set(WalkTarget { target: Tracker::Entity { id, eyes: false }, speed: self.speed, close_enough: 2 });
        }
        true
    }
}
one_shot!(InteractWith);

/// `VillageBoundRandomStroll`: in a village a land position within reach;
/// outside one, a position towards the nearest village section within two
/// that is nearer one (a quarter turn either way), or with none a land
/// position (none clears the walk target).
#[derive(Clone)]
struct VillageBoundRandomStroll {
    speed: f32,
    horizontal: u32,
    vertical: u32,
}

impl VillageBoundRandomStroll {
    fn new(speed: f32, horizontal: u32, vertical: u32) -> Self {
        Self { speed, horizontal, vertical }
    }

    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        if ctx.mem.walk_target.present() {
            return false;
        }
        let position = ctx.me.body.position;
        let here = block_of(position);
        let section = (here.0 >> 4, here.1 >> 4, here.2 >> 4);
        let toward = if ctx.pois.is_village(here) { None } else { Some(closest_village_section(ctx.pois, section, 2)).filter(|&s| s != section) };
        let at = match toward {
            Some((sx, sy, sz)) => {
                let center = DVec3::new(f64::from(sx * 16 + 8) + 0.5, f64::from(sy * 16 + 8), f64::from(sz * 16 + 8) + 0.5);
                default_random_position_towards(ctx.world, ctx.me.profile, position, self.horizontal, self.vertical, center, std::f32::consts::FRAC_PI_2, ctx.me.random)
            }
            None => land_random_position(ctx.world, ctx.me.profile, position, self.horizontal, self.vertical, ctx.me.random),
        };
        ctx.mem.walk_target.set_or_erase(at.map(|p| WalkTarget::at(p, self.speed, 0)));
        true
    }
}
one_shot!(VillageBoundRandomStroll);

/// `SetWalkTargetFromLookTarget(speed, 2)`.
#[derive(Clone)]
struct SetWalkTargetFromLookTarget {
    speed: f32,
    close_enough: i32,
}

impl SetWalkTargetFromLookTarget {
    fn new(speed: f32, close_enough: i32) -> Self {
        Self { speed, close_enough }
    }

    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        if ctx.mem.walk_target.present() {
            return false;
        }
        let Some(&look) = ctx.mem.look_target.get() else { return false };
        ctx.mem.walk_target.set(WalkTarget { target: look, speed: self.speed, close_enough: self.close_enough });
        true
    }
}
one_shot!(SetWalkTargetFromLookTarget);

/// What `SetEntityLookTarget` looks for.
#[derive(Clone, Copy)]
enum LookFor {
    Kind(&'static str),
    Category(&'static str),
}

/// `SetEntityLookTarget(type or category, 8)`.
#[derive(Clone)]
struct SetEntityLookTarget(LookFor);

impl SetEntityLookTarget {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        if ctx.mem.look_target.present() || !ctx.mem.visible_mobs.present() {
            return false;
        }
        let me = ctx.me.body.position;
        let look = self.0;
        let target = ctx.closest_visible(|s| {
            let matches = match look {
                LookFor::Kind(kind) => s.kind == kind,
                LookFor::Category(category) => s.category == category,
            };
            matches && s.position.distance_squared(me) <= f64::from(8.0_f32 * 8.0)
        });
        let Some(id) = target else { return false };
        ctx.mem.look_target.set(Tracker::Entity { id, eyes: true });
        true
    }
}
one_shot!(SetEntityLookTarget);

/// `GoToWantedItem.create(0.5, false, 4)`: with nowhere to walk, to the
/// wanted item it sees within four blocks, watching it, if it can pick
/// things up (no villager sets the pickup cooldown).
#[derive(Clone)]
struct GoToWantedItem;

impl GoToWantedItem {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        if ctx.mem.walk_target.present() || !ctx.me.can_pick_up_loot {
            return false;
        }
        let Some(&item) = ctx.mem.nearest_visible_wanted_item.get() else { return false };
        let me = ctx.me.body.position;
        if !ctx.seen(item).is_some_and(|s| s.position.distance_squared(me) < 4.0 * 4.0) {
            return false;
        }
        ctx.mem.look_target.set(Tracker::Entity { id: item, eyes: true });
        ctx.mem.walk_target.set(WalkTarget { target: Tracker::Entity { id: item, eyes: false }, speed: 0.5, close_enough: 0 });
        true
    }
}
one_shot!(GoToWantedItem);

/// `SetLookAndInteract(PLAYER, 4)`.
#[derive(Clone)]
struct SetLookAndInteractWithPlayer;

impl SetLookAndInteractWithPlayer {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        if ctx.mem.interaction_target.present() || !ctx.mem.visible_mobs.present() {
            return false;
        }
        let me = ctx.me.body.position;
        let Some(id) = ctx.closest_visible(|s| s.position.distance_squared(me) <= 16.0 && s.kind == "minecraft:player") else { return false };
        ctx.mem.interaction_target.set(id);
        ctx.mem.look_target.set(Tracker::Entity { id, eyes: true });
        true
    }
}
one_shot!(SetLookAndInteractWithPlayer);

/// `ShowTradesToPlayer(400, 1600)`: to a player it means to interact with
/// close by, it looks at them and holds up in turn (every two seconds)
/// the results of its offers that the player's main-hand item pays for,
/// looking for 45 seconds while it has some to show and 2 while not.
#[derive(Clone, Default)]
struct ShowTradesToPlayer {
    running: bool,
    end: i64,
    /// `playerItemStack`: the item it last saw in the player's hand (none
    /// before it looked).
    player_item: Option<Option<String>>,
    display: Vec<crate::trading::TradeItem>,
    cycle: i32,
    index: usize,
    look_time: i32,
}

impl ShowTradesToPlayer {
    /// `checkExtraStartConditions`: a living player target within 17
    /// (squared), an adult.
    fn target(ctx: &mut Ctx) -> Option<u64> {
        let id = *ctx.mem.interaction_target.get()?;
        let seen = ctx.seen(id)?;
        (seen.kind == "minecraft:player" && seen.alive && !ctx.me.baby && seen.position.distance_squared(ctx.me.body.position) <= 17.0).then_some(id)
    }

    fn look_at(ctx: &mut Ctx, id: u64) {
        ctx.mem.look_target.set(Tracker::Entity { id, eyes: true });
    }

    /// `clearHeldItem`.
    fn clear_held(ctx: &mut Ctx) {
        *ctx.me.held_item = None;
    }

    /// `findItemsToDisplay`: on a new item in the player's hand, the
    /// offers it pays for (not out of stock), and the first shown.
    fn find_items(&mut self, ctx: &mut Ctx, player: u64) {
        let current = ctx.seen(player).and_then(|s| s.main_hand.clone());
        let same = self.player_item.as_ref().is_some_and(|seen| *seen == current);
        if same {
            return;
        }
        self.player_item = Some(current.clone());
        self.display.clear();
        let Some(item) = current else { return };
        let profession = *ctx.me.profession;
        for offer in ctx.me.offers.get(profession) {
            let pays = offer.buy.id == item || offer.buy_b.as_ref().is_some_and(|b| b.id == item);
            if !offer.out_of_stock() && pays {
                self.display.push(offer.sell.clone());
            }
        }
        if let Some(first) = self.display.first() {
            self.look_time = 900;
            *ctx.me.held_item = Some(first.clone());
        }
    }
}

impl Behavior for ShowTradesToPlayer {
    fn status(&self) -> Status {
        if self.running { Status::Running } else { Status::Stopped }
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        if !ctx.mem.interaction_target.present() {
            return false;
        }
        let Some(player) = Self::target(ctx) else { return false };
        self.running = true;
        self.end = time + duration(ctx, 400, 1600);
        Self::look_at(ctx, player);
        self.cycle = 0;
        self.index = 0;
        self.look_time = 40;
        true
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        let target = if time <= self.end && self.look_time > 0 { Self::target(ctx) } else { None };
        let Some(player) = target else {
            self.do_stop(ctx, time);
            return;
        };
        Self::look_at(ctx, player);
        self.find_items(ctx, player);
        if self.display.is_empty() {
            Self::clear_held(ctx);
            self.look_time = self.look_time.min(40);
        } else if self.display.len() >= 2 {
            // `displayCyclingItems`.
            self.cycle += 1;
            if self.cycle >= 40 {
                self.index += 1;
                self.cycle = 0;
                if self.index > self.display.len() - 1 {
                    self.index = 0;
                }
                *ctx.me.held_item = Some(self.display[self.index].clone());
            }
        }
        self.look_time -= 1;
    }
    fn do_stop(&mut self, ctx: &mut Ctx, _: i64) {
        self.running = false;
        ctx.mem.interaction_target.erase();
        Self::clear_held(ctx);
        self.player_item = None;
    }
    fn describe(&self) -> String {
        "ShowTradesToPlayer".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `LookAndFollowTradingPlayerSink(0.5)`: while trading with a player
/// within 4 blocks, out of water and not just hurt, it walks to within two
/// blocks of them and looks at them; stopping forgets both targets.
#[derive(Clone)]
struct LookAndFollowTradingPlayerSink {
    speed: f32,
    running: bool,
}

impl LookAndFollowTradingPlayerSink {
    fn trader(ctx: &mut Ctx) -> Option<u64> {
        let player = ctx.me.trading_player?;
        let near = ctx.distance_squared(player).is_some_and(|d| d <= 16.0);
        (!ctx.me.fluid.in_water() && ctx.me.hurt_time <= 0 && near).then_some(player)
    }

    fn follow(&self, ctx: &mut Ctx, player: u64) {
        ctx.mem.walk_target.set(WalkTarget { target: Tracker::Entity { id: player, eyes: false }, speed: self.speed, close_enough: 2 });
        ctx.mem.look_target.set(Tracker::Entity { id: player, eyes: true });
    }
}

impl Behavior for LookAndFollowTradingPlayerSink {
    fn status(&self) -> Status {
        if self.running { Status::Running } else { Status::Stopped }
    }
    fn try_start(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        let Some(player) = Self::trader(ctx) else { return false };
        self.running = true;
        // Its duration is `Integer.MAX_VALUE` either way, drawn all the same.
        let _ = duration(ctx, i32::MAX, i32::MAX);
        self.follow(ctx, player);
        true
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        match Self::trader(ctx) {
            Some(player) => self.follow(ctx, player),
            None => self.do_stop(ctx, time),
        }
    }
    fn do_stop(&mut self, ctx: &mut Ctx, _: i64) {
        self.running = false;
        ctx.mem.walk_target.erase();
        ctx.mem.look_target.erase();
    }
    fn describe(&self) -> String {
        "LookAndFollowTradingPlayerSink".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `TradeWithVillager`: with a seen villager to interact with, the two
/// look at and walk to each other and, when close, gossip and share: food
/// when it has plenty (24 points) and the other has little (under 12) or it
/// farms, a farmer's wheat past half a stack, and what the other's
/// profession asks for and its own does not.
#[derive(Clone, Default)]
struct TradeWithVillager {
    running: bool,
    end: i64,
    /// `figureOutWhatIAmWillingToTrade`, as it started.
    trades: Vec<&'static str>,
}

impl TradeWithVillager {
    /// `BehaviorUtils.targetIsValid(brain, INTERACTION_TARGET, VILLAGER)`.
    fn valid(ctx: &mut Ctx) -> Option<u64> {
        let id = *ctx.mem.interaction_target.get()?;
        let seen = ctx.seen(id)?;
        (seen.kind == "minecraft:villager" && seen.alive).then_some(id).filter(|&id| ctx.sees(id))
    }

    /// `lockGazeAndWalkToEachOther(me, target, 0.5, 2)`.
    fn lock_gaze(ctx: &mut Ctx, target: u64) {
        let me = ctx.me.id;
        ctx.mem.look_target.set(Tracker::Entity { id: target, eyes: true });
        ctx.remote.push(Remote::Look { villager: target, target: Tracker::Entity { id: me, eyes: true } });
        ctx.mem.look_target.set(Tracker::Entity { id: target, eyes: true });
        ctx.mem.walk_target.set(WalkTarget { target: Tracker::Entity { id: target, eyes: true }, speed: 0.5, close_enough: 2 });
        ctx.remote.push(Remote::Look { villager: target, target: Tracker::Entity { id: me, eyes: true } });
        ctx.remote.push(Remote::Walk { villager: target, target: WalkTarget { target: Tracker::Entity { id: me, eyes: true }, speed: 0.5, close_enough: 2 } });
    }

    /// `throwHalfStack` towards the target (`BehaviorUtils.throwItem`: from
    /// its eyes less 0.3, at 0.3 along each axis of the way to the target's
    /// feet).
    fn throw_half_stack(ctx: &mut Ctx, target: u64, matches: impl Fn(&minecraftoss_player::inventory::ItemStack) -> bool) {
        let Some(stack) = ctx.me.inventory.take_half_stack(matches) else { return };
        let Some(to) = ctx.seen(target).map(|s| s.position) else { return };
        let me = ctx.me.body.position;
        let position = DVec3::new(me.x, me.y + f64::from(ctx.me.eye_height) - f64::from(0.3_f32), me.z);
        // `Vec3.normalize` divides by the length (none below 1.0E-5F).
        let way = to - me;
        let length = way.length();
        let unit = if length < f64::from(1.0e-5_f32) { DVec3::ZERO } else { DVec3::new(way.x / length, way.y / length, way.z / length) };
        let velocity = unit * f64::from(0.3_f32);
        ctx.remote.push(Remote::ThrowItem { stack, position, velocity });
    }
}

impl Behavior for TradeWithVillager {
    fn status(&self) -> Status {
        if self.running {
            Status::Running
        } else {
            Status::Stopped
        }
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        if !ctx.mem.interaction_target.present() || !ctx.mem.visible_mobs.present() {
            return false;
        }
        let Some(target) = Self::valid(ctx) else { return false };
        self.running = true;
        self.end = time + duration(ctx, 60, 60);
        Self::lock_gaze(ctx, target);
        let theirs = ctx.seen(target).and_then(|s| s.profession).map_or(&[][..], |p| p.requested_items());
        let mine = ctx.me.profession.requested_items();
        self.trades = theirs.iter().copied().filter(|item| !mine.contains(item)).collect();
        true
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        let valid = if time <= self.end { Self::valid(ctx) } else { None };
        let Some(target) = valid else {
            self.do_stop(ctx, time);
            return;
        };
        if ctx.distance_squared(target).is_some_and(|d| d <= 5.0) {
            Self::lock_gaze(ctx, target);
            // `Villager.gossip`: at most once in 1200 ticks for either; it
            // hears the other's gossip (`transferFrom`, from its own
            // random), both note the time, and five villagers that want a
            // golem summon one.
            let theirs = ctx.seen(target).map_or(0, |s| s.last_gossip_time);
            let free = |last: i64| time < last || time >= last + 1200;
            if free(ctx.me.last_gossip_time) && free(theirs) {
                if let Some(source) = ctx.seen(target).and_then(|s| s.gossips.clone()) {
                    if !source.is_empty() {
                        std::sync::Arc::make_mut(ctx.me.gossips).transfer_from(&source, ctx.me.random, 10);
                    }
                }
                ctx.me.last_gossip_time = time;
                ctx.remote.push(Remote::Gossip { villager: ctx.me.id, time });
                ctx.remote.push(Remote::Gossip { villager: target, time });
                spawn_golem_if_needed(ctx, time, 5);
            }
            let farmer = *ctx.me.profession == Profession::Farmer;
            let wants_food = ctx.seen(target).is_some_and(|s| s.food_points < 12);
            if ctx.me.inventory.food_points() >= 24 && (farmer || wants_food) {
                Self::throw_half_stack(ctx, target, |s| crate::villager_inventory::villager_food(&s.id, s.components.as_ref()).is_some());
            }
            if farmer && ctx.me.inventory.count("minecraft:wheat") > 32 {
                Self::throw_half_stack(ctx, target, |s| s.id == "minecraft:wheat");
            }
            let trades = self.trades.clone();
            if !trades.is_empty() && ctx.me.inventory.slots.iter().flatten().any(|s| trades.contains(&s.id.as_str())) {
                Self::throw_half_stack(ctx, target, |s| trades.contains(&s.id.as_str()));
            }
        }
    }
    fn do_stop(&mut self, ctx: &mut Ctx, _: i64) {
        self.running = false;
        ctx.mem.interaction_target.erase();
    }
    fn describe(&self) -> String {
        "TradeWithVillager".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `VillagerMakeLove` (350 ticks): with a breed target it sees that can
/// breed as it can, the two look at and walk to each other, hearts showing
/// now and then while close; after 275 to 324 ticks, if still close, both
/// eat and digest, and with a home bed free that it can reach it takes the
/// bed for a baby (both then rest from breeding), else both look angry.
#[derive(Clone, Default)]
struct VillagerMakeLove {
    running: bool,
    end: i64,
    birth: i64,
}

impl VillagerMakeLove {
    /// `isBreedingPossible`: a villager breed target it sees alive, and both
    /// can breed.
    fn possible(ctx: &mut Ctx) -> Option<u64> {
        let id = *ctx.mem.breed_target.get()?;
        let seen = ctx.seen(id)?;
        if seen.kind != "minecraft:villager" || !seen.alive || !ctx.sees(id) {
            return None;
        }
        let target_can = ctx.seen(id).is_some_and(|s| s.can_breed);
        (ctx.me.can_breed() && target_can).then_some(id)
    }

    fn events(ctx: &mut Ctx, target: u64, event: u8) {
        ctx.remote.push(Remote::EntityEvent { entity: target, event });
        ctx.remote.push(Remote::EntityEvent { entity: ctx.me.id, event });
    }

    /// `takeVacantBed`: the first home with a ticket free within 48 blocks
    /// whose bed it can reach (`createPath(pos, 1).canReach()`), taken.
    fn take_vacant_bed(ctx: &mut Ctx) -> Option<Pos> {
        let here = block_of(ctx.me.body.position);
        let is_home = |t: PoiType| t == PoiType::Home;
        for record in ctx.pois.in_range(&is_home, here, 48, Occupancy::HasSpace) {
            let world = ctx.world;
            let me = &mut ctx.me;
            let path = plan_walk_path_to_any(me.body, me.navigation, world, me.profile, me.fluid, &[record.pos], record.kind.valid_range());
            if path.is_some_and(|p| p.reached()) {
                ctx.pois.acquire(record.pos);
                return Some(record.pos);
            }
        }
        None
    }

    /// `tryToGiveBirth`.
    fn give_birth(ctx: &mut Ctx, target: u64) {
        let Some(bed) = Self::take_vacant_bed(ctx) else {
            Self::events(ctx, target, 13);
            return;
        };
        // `getBreedOffspring`: the baby's type from the biome where it
        // stands, its own or the partner's.
        let roll = ctx.me.random.next_double();
        let at = ctx.me.body.position;
        let kind = if roll < 0.5 {
            let biome = ctx.world.biome(block_of(at));
            Some(crate::villager::type_for_biome(biome.as_deref().unwrap_or("minecraft:plains")).to_owned())
        } else if roll < 0.75 {
            Some(ctx.me.offers.kind.to_owned())
        } else {
            None
        };
        ctx.me.age.set(6000);
        ctx.remote.push(Remote::Birth(super::Birth { parent: ctx.me.id, partner: target, at, kind, bed }));
    }
}

impl Behavior for VillagerMakeLove {
    fn status(&self) -> Status {
        if self.running {
            Status::Running
        } else {
            Status::Stopped
        }
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        if !ctx.mem.breed_target.present() || !ctx.mem.visible_mobs.present() {
            return false;
        }
        let Some(target) = Self::possible(ctx) else { return false };
        self.running = true;
        self.end = time + duration(ctx, 350, 350);
        TradeWithVillager::lock_gaze(ctx, target);
        Self::events(ctx, target, 18);
        self.birth = time + 275 + i64::from(ctx.me.random.next_int(50));
        true
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        let target = if time <= self.end && time <= self.birth { Self::possible(ctx) } else { None };
        let Some(target) = target else {
            self.do_stop(ctx, time);
            return;
        };
        if ctx.distance_squared(target).is_some_and(|d| d <= 5.0) {
            TradeWithVillager::lock_gaze(ctx, target);
            if time >= self.birth {
                crate::villager_inventory::eat_and_digest(ctx.me.food_level, ctx.me.inventory);
                ctx.remote.push(Remote::Eat { villager: target });
                Self::give_birth(ctx, target);
            } else if ctx.me.random.next_int(35) == 0 {
                Self::events(ctx, target, 12);
            }
        }
    }
    fn do_stop(&mut self, ctx: &mut Ctx, _: i64) {
        self.running = false;
        ctx.mem.breed_target.erase();
    }
    fn describe(&self) -> String {
        "VillagerMakeLove".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `WorkAtComposter.useWorkstation`: at its composter it bakes bread, then
/// composts.
fn use_composter(ctx: &mut Ctx) {
    let Some(site) = ctx.mem.job_site.get().copied() else { return };
    let Some(composter) = ctx.block_at(site).filter(|b| b.id == "minecraft:composter") else { return };
    make_bread(ctx);
    compost_items(ctx, site, composter);
}

/// `makeBread`: with 36 loaves or fewer, three wheat to a loaf, up to
/// three loaves (`removeItemType`, then `addItem`; what does not fit is
/// dropped at its feet half a block up, `spawnAtLocation`).
fn make_bread(ctx: &mut Ctx) {
    let inventory = &mut *ctx.me.inventory;
    if inventory.count("minecraft:bread") > 36 {
        return;
    }
    let loaves = (inventory.count("minecraft:wheat") / 3).min(3);
    if loaves == 0 {
        return;
    }
    inventory.remove_type("minecraft:wheat", loaves * 3);
    let bread = minecraftoss_player::inventory::ItemStack { max: 64, ..minecraftoss_player::inventory::ItemStack::new("minecraft:bread", loaves as u8) };
    if let Some(left) = inventory.add(bread) {
        let p = ctx.me.body.position;
        ctx.remote.push(Remote::PopItem { stack: left, position: DVec3::new(p.x, p.y + f64::from(0.5_f32), p.z) });
    }
}

/// The composter's `LEVEL`.
fn compost_level(block: &minecraftoss_player::Block) -> i32 {
    block.property("level").and_then(|l| l.parse().ok()).unwrap_or(0)
}

/// `compostItems`: a full composter (8) gives its bone meal first; then,
/// from its last slot back, the wheat and beetroot seeds past ten of each
/// kind go in one by one, 20 at most, until it is ready to fill (7).
fn compost_items(ctx: &mut Ctx, site: Pos, composter: minecraftoss_player::Block) {
    let mut state = composter;
    if compost_level(&state) == 8 {
        state = extract_produce(ctx, site, state);
    }
    let mut left = 20;
    let mut seen = [0i32; 2];
    let mut temp = state;
    for slot in (0..crate::villager_inventory::SLOTS).rev() {
        if left <= 0 {
            break;
        }
        let Some(stack) = ctx.me.inventory.slots[slot].as_ref() else { continue };
        let kind = match stack.id.as_str() {
            "minecraft:wheat_seeds" => 0,
            "minecraft:beetroot_seeds" => 1,
            _ => continue,
        };
        let size = i32::from(stack.count);
        seen[kind] += size;
        let using = (seen[kind] - 10).min(left).min(size);
        if using <= 0 {
            continue;
        }
        left -= using;
        for _ in 0..using {
            temp = insert_item(ctx, temp, site, slot);
            if compost_level(&temp) == 7 {
                return;
            }
        }
    }
}

/// `ComposterBlock.insertItem` for seeds (`compostable/low`): below 7 a
/// seed goes in, an empty composter taking a layer for sure, others one
/// time in 0.3 (`nextInt(100)` under 30 from the level random); at 7 it is
/// ready 20 ticks later.
fn insert_item(ctx: &mut Ctx, state: minecraftoss_player::Block, site: Pos, slot: usize) -> minecraftoss_player::Block {
    let level = compost_level(&state);
    if level >= 7 {
        return state;
    }
    let layers = if level == 0 { 1 } else { i32::from(ctx.level_random.next_int(100) < 30) };
    let next = if layers > 0 {
        let filled = (level + layers).clamp(0, 7);
        let next = state.clone().with("level", &filled.to_string());
        ctx.me.blocks.push((site, Some(next.clone())));
        if filled == 7 {
            ctx.remote.push(Remote::ScheduleTick { pos: site, delay: 20 });
        }
        next
    } else {
        state
    };
    ctx.me.inventory.remove(slot, 1);
    next
}

/// `ComposterBlock.extractProduce`: bone meal popped out above it (placed
/// by the level random), emptied, with its sound.
fn extract_produce(ctx: &mut Ctx, site: Pos, state: minecraftoss_player::Block) -> minecraftoss_player::Block {
    let dx = (ctx.level_random.next_float() - 0.5) * 0.7;
    let dz = (ctx.level_random.next_float() - 0.5) * 0.7;
    let position = DVec3::new(f64::from(site.0) + 0.5 + f64::from(dx), f64::from(site.1) + 1.01, f64::from(site.2) + 0.5 + f64::from(dz));
    let stack = minecraftoss_player::inventory::ItemStack { max: 64, ..minecraftoss_player::inventory::ItemStack::new("minecraft:bone_meal", 1) };
    ctx.remote.push(Remote::PopItem { stack, position });
    let empty = state.with("level", "0");
    ctx.me.blocks.push((site, Some(empty.clone())));
    let center = DVec3::new(f64::from(site.0) + 0.5, f64::from(site.1) + 0.5, f64::from(site.2) + 0.5);
    ctx.remote.push(Remote::Sound { villager: ctx.me.id, event: "block.composter.empty", position: center });
    empty
}

/// `BlockState.isAir`.
fn air(block: Option<&minecraftoss_player::Block>) -> bool {
    block.is_none_or(|b| matches!(b.id.as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"))
}

/// `BlockPos.closerToCenterThan(position, distance)`.
fn closer_to_center(pos: Pos, position: DVec3, distance: f64) -> bool {
    let center = DVec3::new(f64::from(pos.0) + 0.5, f64::from(pos.1) + 0.5, f64::from(pos.2) + 0.5);
    center.distance_squared(position) < distance * distance
}

/// `HarvestFarmland` (60 ticks; a farmer's, with mob griefing): with its
/// fields remembered and nowhere to look or walk, it picks at random (the
/// level random) one of the blocks about it (its position give or take one
/// on each axis) where a crop is ripe or farmland lies bare and walks
/// there; within a block of its centre it harvests a ripe crop
/// (`destroyBlock`: the loot popped out), sows bare farmland with the first
/// seed it carries, and moves on (20 ticks later) from a crop still growing.
/// It rests 40 ticks after.
#[derive(Clone, Default)]
struct HarvestFarmland {
    running: bool,
    end: i64,
    above: Option<Pos>,
    next_ok: i64,
    worked: i32,
    valid: Vec<Pos>,
}

impl HarvestFarmland {
    /// `validPos`.
    fn valid_pos(ctx: &Ctx, pos: Pos) -> bool {
        let block = ctx.block_at(pos);
        let below = ctx.block_at((pos.0, pos.1 - 1, pos.2));
        block.as_ref().is_some_and(crate::crops::ripe) || (air(block.as_ref()) && below.is_some_and(|b| b.id == "minecraft:farmland"))
    }

    /// `getValidFarmland`.
    fn pick(&self, ctx: &mut Ctx) -> Option<Pos> {
        (!self.valid.is_empty()).then(|| self.valid[ctx.level_random.next_int(self.valid.len() as u32) as usize])
    }

    fn walk_there(ctx: &mut Ctx, pos: Pos) {
        ctx.mem.walk_target.set(WalkTarget { target: Tracker::Block(pos), speed: 0.5, close_enough: 1 });
        ctx.mem.look_target.set(Tracker::Block(pos));
    }

    /// `Level.destroyBlock(pos, true, villager)`: the crop's loot, each stack
    /// popped out about its centre (`Block.popResource`, placed by the level
    /// random), and air.
    fn harvest(ctx: &mut Ctx, pos: Pos, crop: &minecraftoss_player::Block) {
        for stack in ctx.world.block_drops(pos, crop) {
            let mut jitter = || ctx.level_random.next_double() * 0.5 - 0.25;
            let x = f64::from(pos.0) + 0.5 + jitter();
            let y = f64::from(pos.1) + 0.5 + jitter() - 0.125;
            let z = f64::from(pos.2) + 0.5 + jitter();
            if stack.count > 0 {
                ctx.remote.push(Remote::PopItem { stack, position: DVec3::new(x, y, z) });
            }
        }
        ctx.me.blocks.push((pos, None));
    }

    /// Sows the first plantable seed it carries (`BlockItem`'s crop), with
    /// the planting sound at the block's corner.
    fn sow(ctx: &mut Ctx, pos: Pos) {
        for slot in 0..crate::villager_inventory::SLOTS {
            let Some(stack) = ctx.me.inventory.slots[slot].as_ref().filter(|s| s.count > 0) else { continue };
            if !crate::villager_inventory::plantable_seed(&stack.id) {
                continue;
            }
            let Some(crop) = crate::crops::planted(&stack.id) else { continue };
            ctx.me.blocks.push((pos, Some(crop)));
            let position = DVec3::new(f64::from(pos.0), f64::from(pos.1), f64::from(pos.2));
            ctx.remote.push(Remote::Sound { villager: ctx.me.id, event: "item.crop.plant", position });
            ctx.me.inventory.remove(slot, 1);
            break;
        }
    }
}

impl Behavior for HarvestFarmland {
    fn status(&self) -> Status {
        if self.running {
            Status::Running
        } else {
            Status::Stopped
        }
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        if ctx.mem.look_target.present() || ctx.mem.walk_target.present() || !ctx.mem.secondary_job_site.present() {
            return false;
        }
        if !ctx.me.mob_griefing || *ctx.me.profession != Profession::Farmer {
            return false;
        }
        let p = ctx.me.body.position;
        self.valid.clear();
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    let pos = ((p.x + f64::from(x)).floor() as i32, (p.y + f64::from(y)).floor() as i32, (p.z + f64::from(z)).floor() as i32);
                    if Self::valid_pos(ctx, pos) {
                        self.valid.push(pos);
                    }
                }
            }
        }
        self.above = self.pick(ctx);
        let Some(above) = self.above else { return false };
        self.running = true;
        self.end = time + duration(ctx, 60, 60);
        if time > self.next_ok {
            ctx.mem.look_target.set(Tracker::Block(above));
            ctx.mem.walk_target.set(WalkTarget { target: Tracker::Block(above), speed: 0.5, close_enough: 1 });
        }
        true
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        if time > self.end || self.worked >= 200 {
            self.do_stop(ctx, time);
            return;
        }
        if self.above.is_some_and(|pos| !closer_to_center(pos, ctx.me.body.position, 1.0)) {
            return;
        }
        if let Some(pos) = self.above.filter(|_| time > self.next_ok) {
            let state = ctx.block_at(pos);
            let below = ctx.block_at((pos.0, pos.1 - 1, pos.2));
            if let Some(crop) = state.as_ref().filter(|b| crate::crops::ripe(b)) {
                Self::harvest(ctx, pos, crop);
            }
            if air(state.as_ref()) && below.is_some_and(|b| b.id == "minecraft:farmland") && ctx.me.inventory.has_farm_seeds() {
                Self::sow(ctx, pos);
            }
            if state.as_ref().is_some_and(crate::crops::growing) {
                if let Some(index) = self.valid.iter().position(|&p| p == pos) {
                    self.valid.remove(index);
                }
                self.above = self.pick(ctx);
                if let Some(next) = self.above {
                    self.next_ok = time + 20;
                    Self::walk_there(ctx, next);
                }
            }
        }
        self.worked += 1;
    }
    fn do_stop(&mut self, ctx: &mut Ctx, time: i64) {
        self.running = false;
        ctx.mem.look_target.erase();
        ctx.mem.walk_target.erase();
        self.worked = 0;
        self.next_ok = time + 40;
    }
    fn describe(&self) -> String {
        "HarvestFarmland".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `UseBonemeal` (60 ticks): every tenth of its ticks, 160 after its last
/// session, with bone meal and nowhere to look or walk, it picks a growing
/// crop about it (reservoir sampling from the level random), holds bone
/// meal, and once within a block of it grows it (`BoneMealItem.growCrop`)
/// and moves on to another, 40 ticks apart, for up to 80 working ticks.
#[derive(Clone, Default)]
struct UseBonemeal {
    running: bool,
    end: i64,
    next_cycle: i64,
    last_session: i32,
    worked: i32,
    crop: Option<Pos>,
}

impl UseBonemeal {
    /// `pickNextTarget`.
    fn pick(ctx: &mut Ctx) -> Option<Pos> {
        let at = block_of(ctx.me.body.position);
        let (mut result, mut count) = (None, 0u32);
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    let pos = (at.0 + x, at.1 + y, at.2 + z);
                    if ctx.block_at(pos).as_ref().is_some_and(crate::crops::growing) {
                        count += 1;
                        if ctx.level_random.next_int(count) == 0 {
                            result = Some(pos);
                        }
                    }
                }
            }
        }
        result
    }

    /// `setCurrentCropAsTarget`.
    fn target(&self, ctx: &mut Ctx) {
        if let Some(pos) = self.crop {
            ctx.mem.look_target.set(Tracker::Block(pos));
            ctx.mem.walk_target.set(WalkTarget { target: Tracker::Block(pos), speed: 0.5, close_enough: 1 });
        }
    }
}

impl Behavior for UseBonemeal {
    fn status(&self) -> Status {
        if self.running {
            Status::Running
        } else {
            Status::Stopped
        }
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        if ctx.mem.look_target.present() || ctx.mem.walk_target.present() {
            return false;
        }
        let ticks = ctx.me.tick_count;
        if ticks % 10 != 0 || !(self.last_session == 0 || self.last_session + 160 <= ticks) {
            return false;
        }
        if ctx.me.inventory.count("minecraft:bone_meal") <= 0 {
            return false;
        }
        self.crop = Self::pick(ctx);
        if self.crop.is_none() {
            return false;
        }
        self.running = true;
        self.end = time + duration(ctx, 60, 60);
        self.target(ctx);
        *ctx.me.held_item = Some(crate::trading::TradeItem { id: "minecraft:bone_meal".to_owned(), count: 1, components: Default::default() });
        self.next_cycle = time;
        self.worked = 0;
        true
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        if time > self.end || !(self.worked < 80 && self.crop.is_some()) {
            self.do_stop(ctx, time);
            return;
        }
        let Some(pos) = self.crop else { return };
        if time < self.next_cycle || !closer_to_center(pos, ctx.me.body.position, 1.0) {
            return;
        }
        let slot = (0..crate::villager_inventory::SLOTS).find(|&i| ctx.me.inventory.slots[i].as_ref().is_some_and(|s| s.id == "minecraft:bone_meal" && s.count > 0));
        // `BoneMealItem.growCrop`: a crop still growing takes it.
        let grown = match (slot, ctx.block_at(pos)) {
            (Some(slot), Some(crop)) if crate::crops::growing(&crop) => {
                let next = crate::crops::bonemealed(&crop, ctx.level_random);
                ctx.me.blocks.push((pos, Some(next)));
                ctx.me.inventory.remove(slot, 1);
                true
            }
            _ => false,
        };
        if grown {
            self.crop = Self::pick(ctx);
            self.target(ctx);
            self.next_cycle = time + 40;
        }
        self.worked += 1;
    }
    fn do_stop(&mut self, ctx: &mut Ctx, _: i64) {
        self.running = false;
        *ctx.me.held_item = None;
        self.last_session = ctx.me.tick_count;
    }
    fn describe(&self) -> String {
        "UseBonemeal".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `UpdateActivityFromSchedule`.
#[derive(Clone)]
struct UpdateActivityFromSchedule;

impl UpdateActivityFromSchedule {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        ctx.activities.update_from_schedule(ctx.time, ctx.day_time, ctx.mem);
        true
    }
}
one_shot!(UpdateActivityFromSchedule);

/// `PlayTagWithOtherKids`: a one in ten chance each tick of chasing or
/// fleeing another seen baby.
#[derive(Clone)]
struct PlayTagWithOtherKids;

impl PlayTagWithOtherKids {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        if !ctx.mem.visible_villager_babies.present() || ctx.mem.walk_target.present() {
            return false;
        }
        if ctx.level_random.next_int(10) != 0 {
            return false;
        }
        let friends = ctx.mem.visible_villager_babies.get().cloned().unwrap_or_default();
        let me = ctx.me.id;
        let chasing_me = friends.iter().any(|&f| ctx.seen(f).is_some_and(|s| s.interaction_target == Some(me)));
        if !chasing_me {
            // `findSomeoneBeingChased`: the friend chased by fewest (1 to 5).
            let mut counts: Vec<(u64, i32)> = Vec::new();
            for &f in &friends {
                if let Some(target) = ctx.seen(f).and_then(|s| s.interaction_target) {
                    match counts.iter_mut().find(|(id, _)| *id == target) {
                        Some((_, n)) => *n += 1,
                        None => counts.push((target, 1)),
                    }
                }
            }
            counts.sort_by_key(|&(_, n)| n);
            let chased = counts.into_iter().find(|&(_, n)| n > 0 && n <= 5).map(|(id, _)| id);
            if let Some(kid) = chased.or_else(|| friends.first().copied()) {
                ctx.mem.interaction_target.set(kid);
                ctx.mem.look_target.set(Tracker::Entity { id: kid, eyes: true });
                ctx.mem.walk_target.set(WalkTarget { target: Tracker::Entity { id: kid, eyes: false }, speed: 0.6, close_enough: 1 });
            }
            true
        } else {
            // Flee: ten tries at a position in a village (there is none).
            for _ in 0..10 {
                let _ = land_random_position(ctx.world, ctx.me.profile, ctx.me.body.position, 20, 8, ctx.me.random);
            }
            true
        }
    }
}
one_shot!(PlayTagWithOtherKids);

/// `VillagerCalmDown`: unless hurt within the last 40 ticks, near a
/// hostile, or within 6 blocks of its attacker, it forgets the hurt and
/// goes back to its schedule (throttled as ever).
#[derive(Clone)]
struct VillagerCalmDown;

impl VillagerCalmDown {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        let me = ctx.me.body.position;
        // An attacker no longer in the world counts as far away.
        let attacker_near = ctx.mem.hurt_by_entity.get().is_some_and(|&id| ctx.seen(id).is_some_and(|s| s.position.distance_squared(me) <= 36.0));
        let scared = ctx.mem.hurt_by.present() || ctx.mem.nearest_hostile.present() || attacker_near;
        if !scared {
            ctx.mem.hurt_by.erase();
            ctx.mem.hurt_by_entity.erase();
            ctx.activities.update_from_schedule(ctx.time, ctx.day_time, ctx.mem);
        }
        true
    }
}
one_shot!(VillagerCalmDown);

/// Whom a panicking villager runs from.
#[derive(Clone, Copy)]
enum Flee {
    Hostile,
    HurtBy,
}

/// `SetWalkTargetAwayFrom.entity(memory, speed, 6, false)`.
#[derive(Clone)]
struct WalkAwayFrom {
    from: Flee,
    speed: f32,
}

impl WalkAwayFrom {
    /// Without interrupting a walk: nothing while it has a walk target;
    /// otherwise, within 6 blocks of whom it flees, the first of ten
    /// tries at a land spot up to 16 away (7 up or down) within a quarter
    /// turn of straight away.
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        let from = match self.from {
            Flee::Hostile => ctx.mem.nearest_hostile.get().copied(),
            Flee::HurtBy => ctx.mem.hurt_by_entity.get().copied(),
        };
        let Some(from) = from else { return false };
        if ctx.mem.walk_target.present() {
            return false;
        }
        // One no longer in the world is out of reach.
        let Some(avoid) = ctx.seen(from).map(|s| s.position) else { return false };
        if ctx.me.body.position.distance_squared(avoid) >= 36.0 {
            return false;
        }
        for _ in 0..10 {
            if let Some(at) = crate::stroll::land_random_position_away(ctx.world, ctx.me.profile, ctx.me.body.position, 16, 7, avoid, ctx.me.random) {
                ctx.mem.walk_target.set(WalkTarget::at(at, self.speed, 0));
                break;
            }
        }
        true
    }
}
one_shot!(WalkAwayFrom);

/// `InsideBrownianWalk`: under a roof, off to one of the 27 blocks
/// around its own (`betweenClosed` order, then `Collections.shuffle` on
/// the unseeded random) that is out of the sky and has a full top to
/// stand on, while its own box is clear (vanilla asks that of the body as
/// it stands, not of the spot). The walk target is that block itself.
#[derive(Clone)]
struct InsideBrownianWalk {
    speed: f32,
}

impl InsideBrownianWalk {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        if ctx.mem.walk_target.present() {
            return false;
        }
        let p = ctx.me.body.position;
        let at = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        if ctx.world.can_see_sky(at) {
            return false;
        }
        let mut poses = Vec::with_capacity(27);
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    poses.push((at.0 + dx, at.1 + dy, at.2 + dz));
                }
            }
        }
        // `Collections.shuffle(list, rnd)`.
        for i in (2..=poses.len()).rev() {
            let j = ctx.unseeded.next_int(i as u32) as usize;
            poses.swap(i - 1, j);
        }
        let body = &*ctx.me.body;
        let half = f64::from(body.width) / 2.0;
        let (min, max) = (body.position - DVec3::new(half, 0.0, half), body.position + DVec3::new(half, f64::from(body.height), half));
        let target = poses.into_iter().find(|&pos| {
            !ctx.world.can_see_sky(pos)
                && (ctx.world.min_y()..=ctx.world.max_y()).contains(&pos.1)
                && full_top(&ctx.world.collision_boxes(pos))
                && colliding_blocks(ctx.world, min, max).is_empty()
        });
        if let Some(pos) = target {
            ctx.mem.walk_target.set(WalkTarget { target: Tracker::Block(pos), speed: self.speed, close_enough: 0 });
        }
        true
    }
}

/// `Block.isFaceFull(shape, UP)`: the boxes reaching the top of the
/// block cover all of it.
fn full_top(boxes: &[[f64; 6]]) -> bool {
    let top: Vec<&[f64; 6]> = boxes.iter().filter(|b| b[4] >= 1.0).collect();
    let cuts = |lo: usize, hi: usize| {
        let mut cuts: Vec<f64> = top.iter().flat_map(|b| [b[lo], b[hi]]).chain([0.0, 1.0]).filter(|c| (0.0..=1.0).contains(c)).collect();
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        cuts
    };
    let (xs, zs) = (cuts(0, 3), cuts(2, 5));
    xs.windows(2).all(|x| {
        let mx = (x[0] + x[1]) / 2.0;
        zs.windows(2).all(|z| {
            let mz = (z[0] + z[1]) / 2.0;
            top.iter().any(|b| b[0] <= mx && mx <= b[3] && b[2] <= mz && mz <= b[5])
        })
    })
}
one_shot!(InsideBrownianWalk);

/// `GoToClosestVillage`: outside a village, of five tries at a land spot
/// (15 across, 7 up or down, the best of ten by nearness to a village) the
/// first nearer a village than here, or else the last as near.
#[derive(Clone)]
struct GoToClosestVillage {
    speed: f32,
    close_enough: i32,
}

impl GoToClosestVillage {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        if ctx.mem.walk_target.present() {
            return false;
        }
        let here = block_of(ctx.me.body.position);
        if ctx.pois.is_village(here) {
            return false;
        }
        let section = |(x, y, z): Pos| (x >> 4, y >> 4, z >> 4);
        let distance = ctx.pois.sections_to_village(section(here));
        let pois: &PoiManager = ctx.pois;
        // Settled just above; the tries change nothing.
        let mut target = None;
        for _ in 0..5 {
            let found = land_random_position_by(ctx.world, ctx.me.profile, ctx.me.body.position, 15, 7, ctx.me.random, |p| -f64::from(pois.village_level(section(p))));
            if let Some(at) = found {
                let d = pois.village_level(section(block_of(at)));
                if d < distance {
                    target = Some(at);
                    break;
                }
                if d == distance {
                    target = Some(at);
                }
            }
        }
        if let Some(at) = target {
            ctx.mem.walk_target.set(WalkTarget::at(at, self.speed, self.close_enough));
        }
        true
    }
}
one_shot!(GoToClosestVillage);

/// `Villager.POI_MEMORIES`: the types a remembered spot must still be to
/// give its ticket back.
pub(super) fn memory_kinds_of(memory: PoiMemory, profession: Profession) -> PoiKinds {
    match memory {
        PoiMemory::Home => PoiKinds::Home,
        PoiMemory::JobSite => PoiKinds::Held(profession),
        PoiMemory::PotentialJobSite => PoiKinds::Acquirable(Profession::None),
        PoiMemory::MeetingPoint => PoiKinds::Meeting,
    }
}

/// `Villager.releasePoi`: the remembered spot's ticket back, when it is
/// still of the memory's kind.
fn release_poi(ctx: &mut Ctx, memory: PoiMemory) {
    let Some(pos) = memory.get(ctx.mem) else { return };
    let kinds = memory_kinds_of(memory, *ctx.me.profession);
    if ctx.pois.kind(pos).is_some_and(|t| kinds.matches(t)) {
        ctx.pois.release(pos);
    }
}

/// `SetWalkTargetFromBlockMemory`: with no walk target, towards the
/// remembered spot: more than `too_far` away (Manhattan), the first of up
/// to a thousand tries at a spot towards it (15 across, 7 up or down, a
/// quarter turn) within reach; beyond `close_enough`, the spot itself. A
/// spot unreachable for longer than `too_long` is given up (its ticket
/// back), as is one of another dimension.
#[derive(Clone)]
struct SetWalkTargetFromBlockMemory {
    memory: PoiMemory,
    speed: f32,
    close_enough: i32,
    too_far: i32,
    too_long: i64,
}

impl SetWalkTargetFromBlockMemory {
    fn trigger(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        if ctx.mem.walk_target.present() {
            return false;
        }
        let Some(target) = self.memory.get(ctx.mem) else { return false };
        let unreachable_since = ctx.mem.cant_reach_walk_target_since.get().copied();
        let manhattan = |a: Pos, b: Pos| (a.0 - b.0).abs() + (a.1 - b.1).abs() + (a.2 - b.2).abs();
        if unreachable_since.is_none_or(|since| ctx.time - since <= self.too_long) {
            let here = block_of(ctx.me.body.position);
            if manhattan(target, here) > self.too_far {
                let towards = DVec3::new(f64::from(target.0) + 0.5, f64::from(target.1), f64::from(target.2) + 0.5);
                let mut found = None;
                for _ in 0..1000 {
                    let at = default_random_position_towards(ctx.world, ctx.me.profile, ctx.me.body.position, 15, 7, towards, std::f32::consts::FRAC_PI_2, ctx.me.random);
                    if let Some(at) = at.filter(|&p| manhattan(block_of(p), here) <= self.too_far) {
                        found = Some(at);
                        break;
                    }
                }
                match found {
                    Some(at) => ctx.mem.walk_target.set(WalkTarget::at(at, self.speed, self.close_enough)),
                    None => {
                        release_poi(ctx, self.memory);
                        self.memory.slot(ctx.mem).erase();
                        ctx.mem.cant_reach_walk_target_since.set(time);
                    }
                }
            } else if manhattan(target, here) > self.close_enough {
                ctx.mem.walk_target.set(WalkTarget { target: Tracker::Block(target), speed: self.speed, close_enough: self.close_enough });
            }
        } else {
            release_poi(ctx, self.memory);
            self.memory.slot(ctx.mem).erase();
            ctx.mem.cant_reach_walk_target_since.set(time);
        }
        true
    }
}
one_shot!(SetWalkTargetFromBlockMemory);

/// `SetClosestHomeAsWalkTarget`: without a home or walk target, at most
/// every 20 ticks, when a bed lies within 48 blocks but not within two,
/// the next update is drawn (up to 19 ticks later, from the level's
/// random), up to four beds not tried in the last 40 ticks get a path
/// together, and the one reached becomes the walk target.
#[derive(Clone)]
struct SetClosestHomeAsWalkTarget {
    speed: f32,
    cache: Vec<(Pos, i64)>,
    last_update: i64,
}

impl SetClosestHomeAsWalkTarget {
    fn new(speed: f32) -> Self {
        Self { speed, cache: Vec::new(), last_update: 0 }
    }

    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        if ctx.mem.walk_target.present() || ctx.mem.home.present() {
            return false;
        }
        if ctx.time - self.last_update < 20 {
            return false;
        }
        let here = block_of(ctx.me.body.position);
        let is_home = |t: PoiType| t == PoiType::Home;
        let Some(closest) = ctx.pois.find_closest(&is_home, here, 48, Occupancy::Any) else { return false };
        if dist_sqr(closest, here) <= 4.0 {
            return false;
        }
        let mut tried = 0;
        self.last_update = ctx.time + i64::from(ctx.level_random.next_int(20));
        let mut set = JavaPosSet::default();
        for record in ctx.pois.in_range(&is_home, here, 48, Occupancy::Any) {
            let pos = record.pos;
            if self.cache.iter().any(|&(p, _)| p == pos) {
                continue;
            }
            tried += 1;
            if tried >= 5 {
                continue;
            }
            self.cache.push((pos, self.last_update + 40));
            set.insert(pos);
        }
        let targets: Vec<Pos> = set.iter().collect();
        let world = ctx.world;
        let me = &mut ctx.me;
        let path = plan_walk_path_to_any(me.body, me.navigation, world, me.profile, me.fluid, &targets, PoiType::Home.valid_range());
        match path.filter(|p| p.reached()).and_then(|p| p.target()) {
            Some(target) => {
                if ctx.pois.kind(target).is_some() {
                    ctx.mem.walk_target.set(WalkTarget { target: Tracker::Block(target), speed: self.speed, close_enough: 1 });
                }
            }
            None => {
                if tried < 5 {
                    let last = self.last_update;
                    self.cache.retain(|&(_, until)| until >= last);
                }
            }
        }
        true
    }
}
one_shot!(SetClosestHomeAsWalkTarget);

/// `SleepInBed`: at home (within two blocks of the bed's centre), not
/// woken in the last hundred ticks and the bed free, it lies down (from 40
/// ticks after it last got up); it sleeps while resting, above the bed and
/// within 1.14 of its centre.
#[derive(Clone, Default)]
struct SleepInBed {
    running: bool,
    next_ok_start: i64,
}

impl Behavior for SleepInBed {
    fn status(&self) -> Status {
        if self.running {
            Status::Running
        } else {
            Status::Stopped
        }
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        let Some(bed) = ctx.mem.home.get().copied() else { return false };
        if let Some(&woken) = ctx.mem.last_woken.get() {
            let since = ctx.time - woken;
            if since > 0 && since < 100 {
                return false;
            }
        }
        let center = DVec3::new(f64::from(bed.0) + 0.5, f64::from(bed.1) + 0.5, f64::from(bed.2) + 0.5);
        if center.distance_squared(ctx.me.body.position) >= 4.0 || !bed_free(ctx.world, bed) {
            return false;
        }
        self.running = true;
        let _ = duration(ctx, 60, 60);
        if time > self.next_ok_start {
            if let Some(doors) = ctx.mem.doors_to_close.get().cloned() {
                let remaining = close_doors(ctx, None, None, doors);
                ctx.mem.doors_to_close.set(remaining);
            }
            if start_sleeping(ctx, bed) {
                ctx.mem.last_slept.set(time);
            }
            ctx.mem.walk_target.erase();
            ctx.mem.cant_reach_walk_target_since.erase();
        }
        true
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        let usable = ctx.mem.home.get().copied().is_some_and(|bed| {
            let center = DVec3::new(f64::from(bed.0) + 0.5, f64::from(bed.1) + 0.5, f64::from(bed.2) + 0.5);
            ctx.activities.is_active(Activity::Rest) && ctx.me.body.position.y > f64::from(bed.1) + 0.4 && center.distance_squared(ctx.me.body.position) < 1.14 * 1.14
        });
        if !usable {
            self.do_stop(ctx, time);
        }
    }
    fn do_stop(&mut self, ctx: &mut Ctx, time: i64) {
        self.running = false;
        if ctx.me.sleeping.is_some() {
            stop_sleeping(ctx);
            self.next_ok_start = time + 40;
        }
    }
    fn describe(&self) -> String {
        "SleepInBed".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `WakeUp`: out of rest, a sleeper gets up.
#[derive(Clone)]
struct WakeUp;

impl WakeUp {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        if !ctx.activities.is_active(Activity::Rest) && ctx.me.sleeping.is_some() {
            stop_sleeping(ctx);
            true
        } else {
            false
        }
    }
}
one_shot!(WakeUp);

/// `LivingEntity.startSleeping` on the bed at `bed`: at its centre, a
/// sixteenth and a half above its top, lying down (0.2 by 0.2), still, and
/// the bed occupied (once the villager's tick is done).
pub(super) fn start_sleeping(ctx: &mut Ctx, bed: Pos) -> bool {
    let Some(height) = ctx.world.block(bed).filter(|b| is_bed(&b.id)).map(|_| 0.5625) else { return false };
    let body = &mut *ctx.me.body;
    body.position = DVec3::new(f64::from(bed.0) + 0.5, f64::from(bed.1) + height + 0.125, f64::from(bed.2) + 0.5);
    ctx.remote.push(Remote::Bed { pos: bed, occupied: true });
    body.width = 0.2;
    body.height = 0.2;
    *ctx.me.sleeping = Some(bed);
    body.velocity = DVec3::ZERO;
    true
}

/// Getting up from the bed at `bed` facing `facing`: the stand-up spot
/// (`findStandUpPosition`, else above the bed) and the yaw that faces the
/// bed from it.
pub fn rise_from_bed(world: &dyn minecraftoss_player::World, bed: Pos, facing: &str, yaw: f32) -> (DVec3, f32) {
    let stand = stand_up_position(world, bed, facing, yaw).unwrap_or(DVec3::new(f64::from(bed.0) + 0.5, f64::from(bed.1 + 1) + 0.1, f64::from(bed.2) + 0.5));
    let bottom = DVec3::new(f64::from(bed.0) + 0.5, f64::from(bed.1), f64::from(bed.2) + 0.5);
    // `Vec3.normalize`, then the folded 180 / pi float constant.
    let d = bottom - stand;
    let length = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
    let look = if length < f64::from(1.0e-5_f32) { DVec3::ZERO } else { DVec3::new(d.x / length, d.y / length, d.z / length) };
    let degrees = minecraftoss_player::mth::atan2(look.z, look.x) * 57.295_776_367_187_5 - 90.0;
    (stand, wrap_degrees_f64(degrees) as f32)
}

/// `LivingEntity.stopSleeping` (and `Villager.stopSleeping`'s wake time).
pub(super) fn stop_sleeping(ctx: &mut Ctx) {
    if let Some(bed) = ctx.me.sleeping.take() {
        if let Some(block) = ctx.world.block(bed).filter(|b| is_bed(&b.id)) {
            ctx.remote.push(Remote::Bed { pos: bed, occupied: false });
            let facing = block.property("facing").unwrap_or("north");
            let (stand, yaw) = rise_from_bed(ctx.world, bed, facing, *ctx.me.yaw);
            *ctx.me.yaw = yaw;
            ctx.me.body.position = stand;
        }
    }
    // The standing size (`Villager.set_age`: 26.3 babies are 0.49 by 0.98).
    let (width, height) = if ctx.me.baby { (0.49, 0.98) } else { (0.6, 1.95) };
    ctx.me.body.width = width;
    ctx.me.body.height = height;
    ctx.mem.last_woken.set(ctx.time);
}

/// `Mth.wrapDegrees(double)`.
fn wrap_degrees_f64(degrees: f64) -> f64 {
    let mut d = degrees % 360.0;
    if d >= 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d
}

/// `AbstractBedBlock.findStandUpPosition` for a villager: the first safe
/// spot beside the bed (the side away from where it faces first), then the
/// bed itself and its other half, then the same allowing danger.
fn stand_up_position(world: &dyn minecraftoss_player::World, bed: Pos, facing: &str, yaw: f32) -> Option<DVec3> {
    let step = |d: &str| match d {
        "north" => (0, -1),
        "south" => (0, 1),
        "west" => (-1, 0),
        _ => (1, 0),
    };
    let clockwise = |d: &str| match d {
        "north" => "east",
        "east" => "south",
        "south" => "west",
        _ => "north",
    };
    let opposite = |d: &str| match d {
        "north" => "south",
        "south" => "north",
        "west" => "east",
        _ => "west",
    };
    let forward = step(facing);
    let right = clockwise(facing);
    // `Direction.isFacingAngle`: the side it faces is left for last.
    let (sin, cos) = minecraftoss_player::minecraft_sin_cos(f64::from(yaw));
    let r = step(right);
    let facing_right = r.0 as f32 * -(sin as f32) + r.1 as f32 * (cos as f32) > 0.0;
    let side = step(if facing_right { opposite(right) } else { right });
    let below_is_bed = world.block((bed.0, bed.1 - 1, bed.2)).is_some_and(|b| is_bed(&b.id));
    let surround = [
        (side.0, side.1),
        (side.0 - forward.0, side.1 - forward.1),
        (side.0 - forward.0 * 2, side.1 - forward.1 * 2),
        (-forward.0 * 2, -forward.1 * 2),
        (-side.0 - forward.0 * 2, -side.1 - forward.1 * 2),
        (-side.0 - forward.0, -side.1 - forward.1),
        (-side.0, -side.1),
        (-side.0 + forward.0, -side.1 + forward.1),
        (forward.0, forward.1),
        (side.0 + forward.0, side.1 + forward.1),
    ];
    let above = [(0, 0), (-forward.0, -forward.1)];
    let at = |base: Pos, offsets: &[(i32, i32)], careful: bool| offsets.iter().find_map(|&(dx, dz)| safe_dismount(world, (base.0 + dx, base.1, base.2 + dz), careful));
    if below_is_bed {
        let below = (bed.0, bed.1 - 1, bed.2);
        return at(bed, &surround, true)
            .or_else(|| at(below, &surround, true))
            .or_else(|| at(bed, &above, true))
            .or_else(|| at(bed, &surround, false))
            .or_else(|| at(below, &surround, false))
            .or_else(|| at(bed, &above, false));
    }
    let all: Vec<(i32, i32)> = surround.iter().chain(above.iter()).copied().collect();
    at(bed, &all, true).or_else(|| at(bed, &all, false))
}

/// `DismountHelper.findSafeDismountLocation` for a villager (its type's
/// 0.6 by 1.95 box, whatever its age).
fn safe_dismount(world: &dyn minecraftoss_player::World, pos: Pos, careful: bool) -> Option<DVec3> {
    let dangerous = |p: Pos| {
        world.block(p).is_some_and(|b| {
            matches!(b.id.as_str(), "minecraft:fire" | "minecraft:soul_fire" | "minecraft:magma_block" | "minecraft:lava" | "minecraft:wither_rose" | "minecraft:sweet_berry_bush" | "minecraft:cactus" | "minecraft:powder_snow")
                || (b.id.ends_with("campfire") && b.property("lit") == Some("true"))
                || b.id == "minecraft:lava_cauldron"
        })
    };
    if careful && dangerous(pos) {
        return None;
    }
    // `nonClimbableShape`: climbable blocks and open trapdoors have none.
    let shape_top = |p: Pos| -> Option<f64> {
        let block = world.block(p)?;
        if matches!(block.id.as_str(), "minecraft:ladder" | "minecraft:vine" | "minecraft:scaffolding" | "minecraft:twisting_vines" | "minecraft:twisting_vines_plant" | "minecraft:weeping_vines" | "minecraft:weeping_vines_plant" | "minecraft:cave_vines" | "minecraft:cave_vines_plant")
            || (block.id.ends_with("_trapdoor") && block.property("open") == Some("true"))
        {
            return None;
        }
        world.collision_boxes(p).iter().map(|b| b[4]).reduce(f64::max)
    };
    // `getBlockFloorHeight`.
    let floor = match shape_top(pos) {
        Some(top) => top,
        None => match shape_top((pos.0, pos.1 - 1, pos.2)) {
            Some(below) if below >= 1.0 => below - 1.0,
            _ => f64::NEG_INFINITY,
        },
    };
    if floor.is_infinite() || floor >= 1.0 {
        return None;
    }
    if careful && floor <= 0.0 && dangerous((pos.0, pos.1 - 1, pos.2)) {
        return None;
    }
    let position = DVec3::new(f64::from(pos.0) + 0.5, f64::from(pos.1) + floor, f64::from(pos.2) + 0.5);
    let (min, max) = (position - DVec3::new(0.3, 0.0, 0.3), position + DVec3::new(0.3, f64::from(1.95_f32), 0.3));
    colliding_blocks(world, min, max).is_empty().then_some(position)
}

/// `TriggerGate.triggerOneShuffled`: each try, a fresh shuffle, then the
/// children in turn until one triggers.
#[derive(Clone)]
struct TriggerGate {
    children: Vec<(Box<dyn Behavior>, i32)>,
    random: LegacyRandom,
}

impl TriggerGate {
    fn new(children: Vec<(Box<dyn Behavior>, i32)>) -> Self {
        Self { children, random: LegacyRandom::new(0) }
    }

    fn trigger(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        // `ShufflingList.shuffle`.
        let mut keyed: Vec<(f64, (Box<dyn Behavior>, i32))> = std::mem::take(&mut self.children)
            .into_iter()
            .map(|(b, w)| (-f64::from(self.random.next_float()).powf(f64::from(1.0_f32 / w as f32)), (b, w)))
            .collect();
        keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        self.children = keyed.into_iter().map(|(_, c)| c).collect();
        for (child, _) in &mut self.children {
            if child.try_start(ctx, time) {
                break;
            }
        }
        true
    }
}

impl Behavior for TriggerGate {
    fn status(&self) -> Status {
        Status::Stopped
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        self.trigger(ctx, time)
    }
    fn tick_or_stop(&mut self, _: &mut Ctx, _: i64) {}
    fn do_stop(&mut self, _: &mut Ctx, _: i64) {}
    fn describe(&self) -> String {
        "TriggerGate".into()
    }
    fn shuffles(&mut self) -> Vec<&mut LegacyRandom> {
        vec![&mut self.random]
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `StrollAroundPoi`: within `max_distance` of the spot, every 180 ticks a
/// land spot (8 across, 6 up or down), or none; it counts as done in
/// between.
#[derive(Clone)]
struct StrollAroundPoi {
    memory: PoiMemory,
    speed: f32,
    max_distance: i32,
    next_ok: i64,
}

impl StrollAroundPoi {
    fn trigger(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        let Some(pos) = self.memory.get(ctx.mem) else { return false };
        let center = DVec3::new(f64::from(pos.0) + 0.5, f64::from(pos.1) + 0.5, f64::from(pos.2) + 0.5);
        let reach = f64::from(self.max_distance);
        if center.distance_squared(ctx.me.body.position) >= reach * reach {
            return false;
        }
        if time <= self.next_ok {
            return true;
        }
        let at = land_random_position(ctx.world, ctx.me.profile, ctx.me.body.position, 8, 6, ctx.me.random);
        ctx.mem.walk_target.set_or_erase(at.map(|p| WalkTarget::at(p, self.speed, 1)));
        self.next_ok = time + 180;
        true
    }
}
one_shot!(StrollAroundPoi);

/// `SocializeAtBell`: one try in a hundred (from the level's random),
/// within four of the bell with a villager in sight, the nearest villager
/// within √32 becomes whom it interacts with, looks at and walks to.
#[derive(Clone)]
struct SocializeAtBell;

impl SocializeAtBell {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        let Some(bell) = ctx.mem.meeting_point.get().copied() else { return false };
        if !ctx.mem.visible_mobs.present() || ctx.mem.interaction_target.present() {
            return false;
        }
        if ctx.level_random.next_int(100) != 0 {
            return false;
        }
        let center = DVec3::new(f64::from(bell.0) + 0.5, f64::from(bell.1) + 0.5, f64::from(bell.2) + 0.5);
        if center.distance_squared(ctx.me.body.position) >= 16.0 {
            return false;
        }
        let me = ctx.me.body.position;
        if !ctx.sees_where(|s| s.kind == "minecraft:villager") {
            return false;
        }
        if let Some(id) = ctx.closest_visible(|s| s.kind == "minecraft:villager" && s.position.distance_squared(me) <= 32.0) {
            let eyes = ctx.seen(id).map_or(0.0, |s| s.eye_height);
            ctx.mem.interaction_target.set(id);
            ctx.mem.look_target.set(Tracker::Entity { id, eyes: true });
            ctx.mem.walk_target.set(WalkTarget { target: Tracker::Entity { id, eyes: false }, speed: 0.3, close_enough: 1 });
            let _ = eyes;
        }
        true
    }
}
one_shot!(SocializeAtBell);

/// `GoToPotentialJobSite`: while idling, working or playing (or with no
/// other activity), for up to 1200 ticks it walks to and looks at its
/// potential job site; on stopping it gives the site's ticket back and
/// forgets it.
#[derive(Clone)]
struct GoToPotentialJobSite {
    speed: f32,
    running: bool,
    end: i64,
}

impl GoToPotentialJobSite {
    fn new(speed: f32) -> Self {
        Self { speed, running: false, end: 0 }
    }
}

impl Behavior for GoToPotentialJobSite {
    fn status(&self) -> Status {
        if self.running {
            Status::Running
        } else {
            Status::Stopped
        }
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        if !ctx.mem.potential_job_site.present() {
            return false;
        }
        let other = ctx.activities.active.iter().copied().find(|&a| a != Activity::Core);
        if !other.is_none_or(|a| matches!(a, Activity::Idle | Activity::Work | Activity::Play)) {
            return false;
        }
        self.running = true;
        self.end = time + duration(ctx, 1200, 1200);
        true
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        let Some(pos) = ctx.mem.potential_job_site.get().copied().filter(|_| time <= self.end) else {
            self.do_stop(ctx, time);
            return;
        };
        // `BehaviorUtils.setWalkAndLookTargetMemories`.
        ctx.mem.look_target.set(Tracker::Block(pos));
        ctx.mem.walk_target.set(WalkTarget { target: Tracker::Block(pos), speed: self.speed, close_enough: 1 });
    }
    fn do_stop(&mut self, ctx: &mut Ctx, _: i64) {
        self.running = false;
        if let Some(pos) = ctx.mem.potential_job_site.get().copied() {
            if ctx.pois.kind(pos).is_some() {
                ctx.pois.release(pos);
            }
        }
        ctx.mem.potential_job_site.erase();
    }
    fn describe(&self) -> String {
        "GoToPotentialJobSite".into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `AssignProfessionFromJobSite`: within two blocks of its potential job
/// site's centre, that becomes its job site; an unemployed villager takes
/// the site's profession (the first in the registry that works there) and
/// its brain is rebuilt for it.
#[derive(Clone)]
struct AssignProfessionFromJobSite;

impl AssignProfessionFromJobSite {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        let Some(pos) = ctx.mem.potential_job_site.get().copied() else { return false };
        let center = DVec3::new(f64::from(pos.0) + 0.5, f64::from(pos.1) + 0.5, f64::from(pos.2) + 0.5);
        if center.distance_squared(ctx.me.body.position) >= 4.0 {
            return false;
        }
        ctx.mem.potential_job_site.erase();
        ctx.mem.job_site.set(pos);
        if *ctx.me.profession != Profession::None {
            return true;
        }
        if let Some(profession) = ctx.pois.kind(pos).and_then(Profession::of_job_site) {
            *ctx.me.profession = profession;
            ctx.refresh = true;
        }
        true
    }
}
one_shot!(AssignProfessionFromJobSite);

/// `ResetProfession`: without a job site, a villager of any profession but
/// a nitwit's that never traded loses it, and its brain is rebuilt.
#[derive(Clone)]
struct ResetProfession;

impl ResetProfession {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        if ctx.mem.job_site.present() {
            return false;
        }
        let profession = *ctx.me.profession;
        let can_be_fired = profession != Profession::None && profession != Profession::Nitwit;
        if can_be_fired && ctx.me.xp == 0 && ctx.me.level <= 1 {
            *ctx.me.profession = Profession::None;
            ctx.refresh = true;
            true
        } else {
            false
        }
    }
}
one_shot!(ResetProfession);

/// `YieldJobSite`: an unemployed adult holding a potential job site gives
/// it up to the first living villager it knows of that wants it more (one
/// of that profession without a job site that can reach it, or whose job
/// site it is), sending it there unless it has one.
#[derive(Clone)]
struct YieldJobSite {
    speed: f32,
}

impl YieldJobSite {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        let Some(pos) = ctx.mem.potential_job_site.get().copied() else { return false };
        if ctx.mem.job_site.present() || !ctx.mem.mobs.present() {
            return false;
        }
        if ctx.me.baby || *ctx.me.profession != Profession::None {
            return false;
        }
        let Some(kind) = ctx.pois.kind(pos) else { return true };
        let nearby: Vec<u64> = ctx.mem.mobs.get().cloned().unwrap_or_default();
        let me = ctx.me.id;
        let mut taker = None;
        for id in nearby {
            let Some(v) = ctx.seen(id).filter(|s| s.kind == "minecraft:villager" && s.id != me && s.alive) else { continue };
            if v.potential_job_site {
                continue;
            }
            let Some(profession) = v.profession.filter(|p| p.holds(kind)) else { continue };
            let _ = profession;
            let wants = match v.job_site {
                Some(site) => site == pos,
                // `canReachPos` with its navigation (here a fresh one from
                // where it stands).
                None => {
                    let walker = crate::walk_path::Walker { position: v.position, on_ground: true, in_floatable_fluid: false };
                    crate::walk_path::find_walk_path_to_any(ctx.world, ctx.me.profile, walker, &[pos], kind.valid_range()).is_some_and(|p| p.reached)
                }
            };
            if wants {
                taker = Some((id, v.job_site.is_none()));
                break;
            }
        }
        if let Some((id, send)) = taker {
            ctx.mem.walk_target.erase();
            ctx.mem.look_target.erase();
            ctx.mem.potential_job_site.erase();
            if send {
                ctx.remote.push(Remote::Look { villager: id, target: Tracker::Block(pos) });
                ctx.remote.push(Remote::Walk { villager: id, target: WalkTarget { target: Tracker::Block(pos), speed: self.speed, close_enough: 1 } });
                ctx.remote.push(Remote::PotentialJobSite { villager: id, pos });
            }
        }
        true
    }
}
one_shot!(YieldJobSite);

/// `PoiCompetitorScan`: among it and the living villagers it knows of
/// holding its job site for that site's profession, the most experienced
/// (the later on a tie) keeps it; each loser forgets it.
#[derive(Clone)]
struct PoiCompetitorScan;

impl PoiCompetitorScan {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        let Some(pos) = ctx.mem.job_site.get().copied() else { return false };
        if !ctx.mem.mobs.present() {
            return false;
        }
        if let Some(kind) = ctx.pois.kind(pos) {
            let me = ctx.me.id;
            let nearby: Vec<u64> = ctx.mem.mobs.get().cloned().unwrap_or_default();
            // `reduce(body, selectWinner)`.
            let mut winner: (u64, i32) = (me, ctx.me.xp);
            for id in nearby {
                let Some(v) = ctx.seen(id).filter(|s| s.kind == "minecraft:villager" && s.id != me && s.alive) else { continue };
                if v.job_site != Some(pos) || !v.profession.is_some_and(|p| p.holds(kind)) {
                    continue;
                }
                let (loser, next) = if winner.1 > v.xp { (id, winner) } else { (winner.0, (id, v.xp)) };
                if loser == me {
                    ctx.mem.job_site.erase();
                } else {
                    ctx.remote.push(Remote::EraseJobSite { villager: loser });
                }
                winner = next;
            }
        }
        true
    }
}
one_shot!(PoiCompetitorScan);

/// `WorkAtPoi`: every 300 ticks at best, on a coin toss from the level's
/// random, within 1.73 of its job site's centre it works there: it looks
/// at the site and makes its profession's work sound (the
/// restock check after needs trades that were used).
#[derive(Clone, Default)]
struct WorkAtPoi {
    running: bool,
    end: i64,
    last_check: i64,
    /// A farmer's `WorkAtComposter` (its bread and composting need a
    /// harvest in its inventory).
    composter: bool,
}

impl WorkAtPoi {
    fn near(ctx: &Ctx) -> Option<Pos> {
        let pos = ctx.mem.job_site.get().copied()?;
        let center = DVec3::new(f64::from(pos.0) + 0.5, f64::from(pos.1) + 0.5, f64::from(pos.2) + 0.5);
        (center.distance_squared(ctx.me.body.position) < 1.73 * 1.73).then_some(pos)
    }
}

impl Behavior for WorkAtPoi {
    fn status(&self) -> Status {
        if self.running {
            Status::Running
        } else {
            Status::Stopped
        }
    }
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        if !ctx.mem.job_site.present() {
            return false;
        }
        if ctx.time - self.last_check < 300 {
            return false;
        }
        if ctx.level_random.next_int(2) != 0 {
            return false;
        }
        self.last_check = ctx.time;
        let Some(site) = Self::near(ctx) else { return false };
        self.running = true;
        self.end = time + duration(ctx, 60, 60);
        // `LAST_WORKED_AT_POI` is not among a villager's memories: the set
        // is dropped.
        ctx.mem.look_target.set(Tracker::Block(site));
        // `playWorkSound`: `makeSound` at the villager's pitch.
        if let Some(event) = ctx.me.profession.work_sound() {
            let random = &mut *ctx.me.random;
            let pitch = if ctx.me.baby { (random.next_float() - random.next_float()) * 0.2 + 1.5 } else { (random.next_float() - random.next_float()) * 0.2 + 1.0 };
            ctx.me.sounds.push((event, pitch));
        }
        // A farmer's `WorkAtComposter.useWorkstation`.
        if self.composter {
            use_composter(ctx);
        }
        // `shouldRestock` and `restock`, after its brain has ticked (they
        // touch only its offers and the trade sets' sequences).
        ctx.remote.push(Remote::WorkedAtPoi { villager: ctx.me.id });
        true
    }
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64) {
        if time > self.end || Self::near(ctx).is_none() {
            self.do_stop(ctx, time);
        }
    }
    fn do_stop(&mut self, _: &mut Ctx, _: i64) {
        self.running = false;
    }
    fn describe(&self) -> String {
        if self.composter { "WorkAtComposter" } else { "WorkAtPoi" }.into()
    }
    fn clone_box(&self) -> Box<dyn Behavior> {
        Box::new(self.clone())
    }
}

/// `StrollToPoi`: within `max_distance` of the spot, every 80 ticks it
/// walks to it; it counts as done in between.
#[derive(Clone)]
struct StrollToPoi {
    memory: PoiMemory,
    speed: f32,
    close_enough: i32,
    max_distance: i32,
    next_ok: i64,
}

impl StrollToPoi {
    fn trigger(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        let Some(pos) = self.memory.get(ctx.mem) else { return false };
        let center = DVec3::new(f64::from(pos.0) + 0.5, f64::from(pos.1) + 0.5, f64::from(pos.2) + 0.5);
        let reach = f64::from(self.max_distance);
        if center.distance_squared(ctx.me.body.position) >= reach * reach {
            return false;
        }
        if time <= self.next_ok {
            return true;
        }
        ctx.mem.walk_target.set(WalkTarget { target: Tracker::Block(pos), speed: self.speed, close_enough: self.close_enough });
        self.next_ok = time + 80;
        true
    }
}
one_shot!(StrollToPoi);

/// `StrollToPoiList(SECONDARY_JOB_SITE, ..., JOB_SITE)`: one of the fields
/// by the level's random; within `max_distance` of the job site, every 100
/// ticks it walks there.
#[derive(Clone)]
struct StrollToPoiList {
    speed: f32,
    close_enough: i32,
    max_distance: i32,
    next_ok: i64,
}

impl StrollToPoiList {
    fn trigger(&mut self, ctx: &mut Ctx, time: i64) -> bool {
        let Some(list) = ctx.mem.secondary_job_site.get().cloned() else { return false };
        let Some(site) = ctx.mem.job_site.get().copied() else { return false };
        if list.is_empty() {
            return false;
        }
        let target = list[ctx.level_random.next_int(list.len() as u32) as usize];
        let center = DVec3::new(f64::from(site.0) + 0.5, f64::from(site.1) + 0.5, f64::from(site.2) + 0.5);
        let reach = f64::from(self.max_distance);
        if center.distance_squared(ctx.me.body.position) >= reach * reach {
            return false;
        }
        if time > self.next_ok {
            ctx.mem.walk_target.set(WalkTarget { target: Tracker::Block(target), speed: self.speed, close_enough: self.close_enough });
            self.next_ok = time + 100;
        }
        true
    }
}
one_shot!(StrollToPoiList);

/// `#minecraft:mob_interactable_doors`: the wooden and copper doors.
fn mob_door(id: &str) -> bool {
    let Some(name) = id.strip_prefix("minecraft:").and_then(|n| n.strip_suffix("_door")) else { return false };
    matches!(
        name,
        "oak" | "spruce" | "birch" | "jungle" | "acacia" | "dark_oak" | "pale_oak" | "crimson" | "warped" | "mangrove" | "bamboo" | "cherry" | "poplar"
            | "copper" | "exposed_copper" | "weathered_copper" | "oxidized_copper" | "waxed_copper" | "waxed_exposed_copper" | "waxed_weathered_copper" | "waxed_oxidized_copper"
    )
}

/// `DoorBlock.setOpen`: the door's new state (applied once the villager's
/// brain has ticked) and its sound's pitch, drawn now from the level's
/// random. Each half's change (`ServerLevel.sendBlockUpdated`) may have
/// its own navigation recompute a path that passes near it, over the
/// world as the change leaves it.
fn set_door(ctx: &mut Ctx, pos: Pos, open: bool) {
    let pitch = ctx.level_random.next_float() * 0.1 + 0.9;
    let mut changed: Vec<(Pos, Option<minecraftoss_player::Block>)> = Vec::new();
    for &(p, o, _) in ctx.me.doors.iter() {
        changed.extend(door_halves(ctx.world, p, o).into_iter().map(|(h, b)| (h, Some(b))));
    }
    let halves = door_halves(ctx.world, pos, open);
    ctx.me.doors.push((pos, open, pitch));
    for (half, block) in halves {
        changed.push((half, Some(block)));
        if ctx.me.navigation.should_recompute(half, ctx.me.body.position) {
            let view = crate::overlay::Overlay { base: ctx.world, changed: changed.clone() };
            if ctx.me.navigation.recompute(&view, ctx.me.body, ctx.me.profile, ctx.me.fluid, ctx.time) {
                let id = ctx.me.paths.fresh();
                ctx.me.paths.current = Some(id);
                let nav = &*ctx.me.navigation;
                ctx.me.paths.known.insert(id, (nav.target_pos.unwrap_or_default(), nav.nodes.len(), nav.next));
            }
        }
    }
}

/// A door's halves as `setOpen` leaves them, the one at `pos` first.
pub fn door_halves(world: &dyn minecraftoss_player::World, pos: Pos, open: bool) -> Vec<(Pos, minecraftoss_player::Block)> {
    let Some(door) = world.block(pos).filter(|b| b.id.ends_with("_door")) else { return Vec::new() };
    let value = if open { "true" } else { "false" };
    let other = if door.property("half") == Some("upper") { (pos.0, pos.1 - 1, pos.2) } else { (pos.0, pos.1 + 1, pos.2) };
    let mut out = vec![(pos, door.clone().with("open", value))];
    if let Some(half) = world.block(other).filter(|b| b.id == door.id && b.property("half") != door.property("half")) {
        out.push((other, half.with("open", value)));
    }
    out
}

/// Whether the door at `pos` is open, with this tick's own changes.
fn door_open(ctx: &Ctx, pos: Pos) -> Option<bool> {
    if let Some(&(_, open, _)) = ctx.me.doors.iter().rev().find(|(p, _, _)| *p == pos) {
        return Some(open);
    }
    let block = ctx.world.block(pos).filter(|b| mob_door(&b.id))?;
    Some(block.property("open") == Some("true"))
}

/// `InteractWithDoor.closeDoorsThatIHaveOpenedOrPassedThrough`: of the
/// doors it remembers, besides the ones it steps from and to, those more
/// than 3 from it, gone, shut or with another villager within two coming
/// through are forgotten; the rest it shuts. Returns those still kept.
fn close_doors(ctx: &mut Ctx, from: Option<Pos>, to: Option<Pos>, doors: Vec<Pos>) -> Vec<Pos> {
    let me = ctx.me.body.position;
    let mut kept = Vec::new();
    for door in doors {
        if Some(door) == from || Some(door) == to {
            kept.push(door);
            continue;
        }
        let center = DVec3::new(f64::from(door.0) + 0.5, f64::from(door.1) + 0.5, f64::from(door.2) + 0.5);
        if center.distance_squared(me) >= 9.0 {
            continue;
        }
        if door_open(ctx, door) != Some(true) {
            continue;
        }
        // `areOtherMobsComingThroughDoor`: villagers it knows of within two
        // of the door stepping from or to it.
        let nearby = ctx.mem.mobs.get().cloned().unwrap_or_default();
        let coming = nearby.iter().filter_map(|&id| ctx.seen(id)).any(|s| {
            s.kind == "minecraft:villager"
                && center.distance_squared(s.position) < 4.0
                && s.path_step.is_some_and(|(a, b)| a == door || b == door)
        });
        if coming {
            continue;
        }
        set_door(ctx, door, false);
    }
    kept
}

/// `InteractWithDoor`: along its path (begun, not done), at most once per
/// node (and then after 20 tries), it opens a shut door it steps from or
/// to, remembering it, then shuts the doors behind it.
#[derive(Clone, Default)]
struct InteractWithDoor {
    last_node: Option<Pos>,
    cooldown: i32,
}

impl InteractWithDoor {
    fn trigger(&mut self, ctx: &mut Ctx, _: i64) -> bool {
        if !ctx.mem.path.present() {
            return false;
        }
        let nav = &*ctx.me.navigation;
        if nav.next == 0 || nav.next >= nav.nodes.len() {
            return false;
        }
        let (from, to) = (nav.nodes[nav.next - 1], nav.nodes[nav.next]);
        if self.last_node == Some(to) {
            self.cooldown = 20;
        } else {
            self.cooldown -= 1;
            if self.cooldown > 0 {
                return false;
            }
        }
        self.last_node = Some(to);
        let mut doors = ctx.mem.doors_to_close.get().cloned();
        if let Some(open) = door_open(ctx, from) {
            if !open {
                set_door(ctx, from, true);
            }
            let set = doors.get_or_insert_with(Vec::new);
            if !set.contains(&from) {
                set.push(from);
            }
        }
        if door_open(ctx, to) == Some(false) {
            set_door(ctx, to, true);
            let set = doors.get_or_insert_with(Vec::new);
            if !set.contains(&to) {
                set.push(to);
            }
        }
        if let Some(set) = doors {
            let kept = close_doors(ctx, Some(from), Some(to), set);
            ctx.mem.doors_to_close.set(kept);
        }
        true
    }
}
one_shot!(InteractWithDoor);
