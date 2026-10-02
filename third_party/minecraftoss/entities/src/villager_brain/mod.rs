//! The villager's brain (pinned 26.3 `Brain`, `Villager.BRAIN_PROVIDER`,
//! `VillagerGoalPackages` and the sensors and behaviors they use).
//!
//! A brain remembers (`memory.rs`), senses on its own clocks, and each tick
//! tries to start every stopped behavior of its active activities, then
//! ticks every running one. Behaviors are grouped by priority; within a
//! priority vanilla keeps one `HashMap` of activities, so they run in that
//! map's order (see [`MAP_ORDER`]), and within an activity in the order the
//! package lists them. Class behaviors draw their running time from the
//! level's random as they start; `RunOne` gates shuffle their children with
//! a random of their own (seeded, in the harness, from the mob's).
//!
//! Points of interest come from the entity world's [`PoiManager`]: an
//! unemployed villager claims beds, meeting points and job sites
//! (`AcquirePoi`), babies find beds to jump on, and strolls lean towards
//! villages. The core, idle, play, rest and panic activities run; work
//! and meet, sleeping, professions and doors are not ported yet.
mod behaviors;
mod memory;

pub use behaviors::{door_halves, rise_from_bed};
pub use memory::{Memories, Observer, Slot, Tracker, Visible, WalkTarget};

use crate::fluid::FluidFrame;
use crate::movement::Body;
use crate::navigation::{plan_walk_path_to_any, GroundNavigation};
use crate::poi::{JavaPosSet, Occupancy, PoiManager, PoiType};
use crate::villager::Profession;
use crate::walk_path::WalkProfile;
use glam::DVec3;
use minecraftoss_player::{rng::LegacyRandom, World};
use std::collections::HashMap;

/// `Activity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Activity {
    Core,
    Idle,
    Work,
    Play,
    Rest,
    Meet,
    Panic,
    PreRaid,
    Raid,
    Hide,
}

impl Activity {
    pub fn name(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Idle => "idle",
            Self::Work => "work",
            Self::Play => "play",
            Self::Rest => "rest",
            Self::Meet => "meet",
            Self::Panic => "panic",
            Self::PreRaid => "pre_raid",
            Self::Raid => "raid",
            Self::Hide => "hide",
        }
    }
}

/// How a priority's `HashMap<Activity, _>` iterates: by `String.hashCode`
/// of the name spread into 16 buckets (play 0, core and rest 1, hide 2, meet
/// 4, idle and pre_raid 5, work 6, panic and raid 15), then in the order
/// `BRAIN_PROVIDER` adds them.
const MAP_ORDER: [Activity; 10] = [
    Activity::Play,
    Activity::Core,
    Activity::Rest,
    Activity::Hide,
    Activity::Meet,
    Activity::Idle,
    Activity::PreRaid,
    Activity::Work,
    Activity::Panic,
    Activity::Raid,
];

/// `Behavior.Status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Stopped,
    Running,
}

/// Another living entity as a villager's brain sees it this tick.
#[derive(Clone, Debug)]
pub struct Seen {
    pub id: u64,
    /// The entity type (`minecraft:villager`).
    pub kind: &'static str,
    /// `MobCategory`, by name (`creature`, `monster`, `misc`, ...).
    pub category: &'static str,
    pub position: DVec3,
    pub eye_height: f32,
    pub width: f32,
    pub height: f32,
    pub alive: bool,
    pub baby: bool,
    pub spectator: bool,
    /// For a villager, whom its brain means to interact with.
    pub interaction_target: Option<u64>,
    /// `isSleeping`.
    pub sleeping: bool,
    /// A villager's `canBreed`, and the food points in its inventory
    /// (`wantsMoreFood` under 12).
    pub can_breed: bool,
    pub food_points: i32,
    /// A villager's profession, job site, whether it has a potential job
    /// site, and its trading experience.
    pub profession: Option<Profession>,
    pub job_site: Option<(i32, i32, i32)>,
    pub potential_job_site: bool,
    pub xp: i32,
    /// A villager's path, while under way: the nodes it moves from and to.
    pub path_step: Option<((i32, i32, i32), (i32, i32, i32))>,
    /// A villager's `lastGossipTime`, and whether it wants a golem
    /// (`wantsToSpawnGolem`).
    pub last_gossip_time: i64,
    pub wants_golem: bool,
    /// A villager's gossip, which others hear as they gossip with it.
    pub gossips: Option<std::sync::Arc<crate::gossip::Gossips>>,
    /// A player's main-hand item.
    pub main_hand: Option<String>,
}

/// A villager's offers as its behaviors read them (`getOffers`, making
/// them from its trade set the first time).
pub struct OffersAccess<'a> {
    pub offers: &'a mut Option<Vec<crate::trading::MerchantOffer>>,
    pub book: Option<&'a crate::trading::TradeBook>,
    pub sequences: &'a mut crate::trading::TradeSequences,
    /// Its type and level.
    pub kind: &'a str,
    pub level: i32,
}

impl OffersAccess<'_> {
    fn reborrow(&mut self) -> OffersAccess<'_> {
        OffersAccess { offers: &mut *self.offers, book: self.book, sequences: &mut *self.sequences, kind: self.kind, level: self.level }
    }

    /// `getOffers` for a villager of `profession`.
    pub fn get(&mut self, profession: Profession) -> &[crate::trading::MerchantOffer] {
        if self.offers.is_none() {
            let Some(book) = self.book else { return &[] };
            *self.offers = Some(book.villager_offers(self.kind, profession.id(), self.level, self.sequences));
        }
        self.offers.as_deref().unwrap_or_default()
    }
}

/// Item entities join the entities a brain knows of past this many IDs
/// (players come at `u64::MAX / 2`).
pub const ITEM_TARGET: u64 = u64::MAX / 4;

/// An item entity as a brain knows it (tracked like an entity).
pub fn seen_item(item: &minecraftoss_player::WorldItem) -> Seen {
    Seen {
        id: ITEM_TARGET + item.id as u64,
        kind: "minecraft:item",
        category: "misc",
        position: item.position,
        eye_height: 0.2125,
        width: 0.25,
        height: 0.25,
        alive: true,
        baby: false,
        spectator: false,
        interaction_target: None,
        sleeping: false,
        can_breed: false,
        food_points: 0,
        profession: None,
        job_site: None,
        potential_job_site: false,
        xp: 0,
        path_step: None,
        last_gossip_time: 0,
        wants_golem: false,
        gossips: None,
        main_hand: None,
    }
}

/// A villager's own state its behaviors act on.
pub struct Me<'a> {
    pub id: u64,
    /// Its body (sleeping moves it).
    pub body: &'a mut Body,
    /// `getSleepingPos`.
    pub sleeping: &'a mut Option<(i32, i32, i32)>,
    /// `getYRot` (getting up turns it to the bed).
    pub yaw: &'a mut f32,
    /// `VillagerData.profession` (a new one rebuilds the brain).
    pub profession: &'a mut Profession,
    /// Trading experience and level.
    pub xp: i32,
    pub level: i32,
    /// Sounds it makes (`makeSound`): event and pitch, at full volume.
    pub sounds: &'a mut Vec<(&'static str, f32)>,
    /// Doors it opens or closes (`DoorBlock.setOpen`), with the sound's
    /// pitch drawn as it did: set once its brain has ticked, before it moves.
    pub doors: &'a mut Vec<((i32, i32, i32), bool, f32)>,
    pub fluid: FluidFrame,
    pub eye_height: f32,
    pub baby: bool,
    pub navigation: &'a mut GroundNavigation,
    pub paths: &'a mut PathObjects,
    pub profile: &'a WalkProfile,
    pub look_at: &'a mut Option<DVec3>,
    pub jump: &'a mut bool,
    pub random: &'a mut LegacyRandom,
    /// `Sensing`: this tick's line-of-sight answers.
    pub sight: &'a mut HashMap<u64, bool>,
    pub last_gossip_time: i64,
    pub no_ai: bool,
    /// Its gossip (`Villager.gossips`).
    pub gossips: &'a mut std::sync::Arc<crate::gossip::Gossips>,
    /// The player it trades with (`getTradingPlayer`), as its memories name
    /// players; `hurtTime`; the item in its hand (what it shows a player);
    /// and its offers.
    pub trading_player: Option<u64>,
    pub hurt_time: i32,
    pub held_item: &'a mut Option<crate::trading::TradeItem>,
    pub offers: OffersAccess<'a>,
    /// Its inventory (what it wants to pick up depends on the room left),
    /// food level and age (breeding eats and ages it).
    pub inventory: &'a mut crate::villager_inventory::VillagerInventory,
    pub food_level: &'a mut i32,
    pub age: &'a mut crate::age::Age,
    /// `Mob.canPickUpLoot`.
    pub can_pick_up_loot: bool,
    /// The `mob_griefing` game rule, and its `tickCount`.
    pub mob_griefing: bool,
    pub tick_count: i32,
    /// Blocks its behaviors set this tick, in order: they stand at once for
    /// its brain (`Ctx::block_at`) and reach the world once it has ticked.
    pub blocks: &'a mut Vec<((i32, i32, i32), Option<minecraftoss_player::Block>)>,
}

impl Me<'_> {
    /// `Villager.canBreed`.
    pub fn can_breed(&self) -> bool {
        crate::villager_inventory::can_breed(*self.food_level, self.inventory, self.sleeping.is_some(), self.age.ticks)
    }
}

/// A baby two villagers made: where, of which type, and the bed it was
/// given (`VillagerMakeLove.breed`).
#[derive(Clone, Debug)]
pub struct Birth {
    pub parent: u64,
    pub partner: u64,
    pub at: DVec3,
    /// Its type, or none for the partner's.
    pub kind: Option<String>,
    pub bed: (i32, i32, i32),
}

/// A change a behavior makes to another villager's brain, applied once
/// this villager's tick is done (vanilla writes it at once; nothing reads
/// it in between).
#[derive(Clone, Debug)]
pub enum Remote {
    Look { villager: u64, target: Tracker },
    Walk { villager: u64, target: WalkTarget },
    Gossip { villager: u64, time: i64 },
    /// A bed's `OCCUPIED` set as its sleeper lies down or gets up.
    Bed { pos: (i32, i32, i32), occupied: bool },
    /// Another villager is sent to a job site it gave up.
    PotentialJobSite { villager: u64, pos: (i32, i32, i32) },
    /// Another villager loses its job site to a rival.
    EraseJobSite { villager: u64 },
    /// An iron golem it summoned.
    SpawnGolem(crate::golem_spawn::SummonedGolem),
    /// It worked at its job site: the restock check (`shouldRestock`,
    /// making its offers if it has none) and restock.
    WorkedAtPoi { villager: u64 },
    /// Another villager saw the golem appear (`GolemSensor.golemDetected`).
    GolemDetected { villager: u64 },
    /// Another villager eats and digests (`eatAndDigestFood`).
    Eat { villager: u64 },
    /// A baby is born: the partner rests from breeding too.
    Birth(Birth),
    /// `Level.broadcastEntityEvent`.
    EntityEvent { entity: u64, event: u8 },
    /// `Block.popResource`: an item entity at `position` (its place drawn
    /// from the level random as the block broke).
    PopItem { stack: minecraftoss_player::inventory::ItemStack, position: DVec3 },
    /// A sound the level plays at a position (`Level.playSound(null, ...)`),
    /// heard with the villager's.
    Sound { villager: u64, event: &'static str, position: DVec3 },
    /// `scheduleTick` for the block at `pos`.
    ScheduleTick { pos: (i32, i32, i32), delay: i32 },
    /// `BehaviorUtils.throwItem`: an item entity thrown from where it
    /// stood (its pickup delay the default ten ticks).
    ThrowItem { stack: minecraftoss_player::inventory::ItemStack, position: DVec3, velocity: DVec3 },
}

/// What a behavior works with.
pub struct Ctx<'a> {
    pub world: &'a dyn World,
    pub mem: &'a mut Memories,
    pub activities: &'a mut Activities,
    pub me: Me<'a>,
    pub others: &'a [Seen],
    pub level_random: &'a mut LegacyRandom,
    pub time: i64,
    pub day_time: i64,
    pub remote: &'a mut Vec<Remote>,
    /// The brain's stand-in for vanilla's unseeded randoms
    /// (`Collections.shuffle`'s shared `Random`).
    pub unseeded: &'a mut LegacyRandom,
    /// The level's points of interest (`ServerLevel.getPoiManager`).
    pub pois: &'a mut PoiManager,
    /// A new profession asks for a new brain (`Villager.refreshBrain`).
    pub refresh: bool,
}

impl Ctx<'_> {
    pub fn observer(&self) -> Observer {
        Observer { id: self.me.id, position: self.me.body.position, eye_height: self.me.eye_height }
    }

    pub fn seen(&self, id: u64) -> Option<&Seen> {
        self.others.iter().find(|s| s.id == id)
    }

    /// `NearestVisibleLivingEntities.contains(entity)` from the memory.
    pub fn sees(&mut self, id: u64) -> bool {
        let observer = self.observer();
        let (others, world) = (self.others, self.world);
        match self.mem.visible_mobs.get_mut() {
            Some(visible) => visible.contains(id, &observer, others, world, self.me.sight),
            None => false,
        }
    }

    /// `findClosest` over the visible mobs.
    /// `NearestVisibleLivingEntities.contains(predicate)` from the memory.
    pub fn sees_where(&mut self, filter: impl FnMut(&Seen) -> bool) -> bool {
        let observer = self.observer();
        let (others, world) = (self.others, self.world);
        match self.mem.visible_mobs.get_mut() {
            Some(visible) => visible.contains_where(&observer, others, world, self.me.sight, filter),
            None => false,
        }
    }

    pub fn closest_visible(&mut self, filter: impl FnMut(&Seen) -> bool) -> Option<u64> {
        let observer = self.observer();
        let (others, world) = (self.others, self.world);
        self.mem.visible_mobs.get_mut()?.find_closest(&observer, others, world, self.me.sight, filter)
    }

    /// The block at `pos` as its brain sees it: with the blocks its
    /// behaviors set this tick.
    pub fn block_at(&self, pos: (i32, i32, i32)) -> Option<minecraftoss_player::Block> {
        match self.me.blocks.iter().rev().find(|(p, _)| *p == pos) {
            Some((_, block)) => block.clone(),
            None => self.world.block(pos),
        }
    }

    /// `Entity.distanceToSqr` from this villager's feet.
    pub fn distance_squared(&self, id: u64) -> Option<f64> {
        self.seen(id).map(|s| s.position.distance_squared(self.me.body.position))
    }
}

/// The paths a villager's navigation has held, by identity: vanilla keeps
/// `Path` objects in memories and compares them by reference.
#[derive(Clone, Debug, Default)]
pub struct PathObjects {
    next_id: u64,
    /// The navigation's current path object.
    pub current: Option<u64>,
    /// Each path's target, node count and the node it was on when last
    /// current (for observing an old one).
    pub known: HashMap<u64, (minecraftoss_player::Pos, usize, usize)>,
}

impl PathObjects {
    pub fn fresh(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}

/// The brain's activities (`Brain.activeActivities`, `defaultActivity`,
/// the schedule).
#[derive(Clone, Debug)]
pub struct Activities {
    pub active: Vec<Activity>,
    pub default: Activity,
    pub baby: bool,
    last_schedule_update: i64,
}

impl Activities {
    fn new(baby: bool) -> Self {
        Self { active: vec![Activity::Core, Activity::Idle], default: Activity::Idle, baby, last_schedule_update: -9999 }
    }

    pub fn is_active(&self, activity: Activity) -> bool {
        self.active.contains(&activity)
    }

    /// `activityRequirementsAreMet`: work needs a job site, meeting a
    /// meeting point; activities the brain lacks are never met.
    fn requirements_met(&self, activity: Activity, memories: &Memories) -> bool {
        match activity {
            Activity::Work => !self.baby && memories.job_site.present(),
            Activity::Play => self.baby,
            Activity::Meet => memories.meeting_point.present(),
            _ => true,
        }
    }

    /// `setActiveActivity`: the core and this one.
    fn set_active(&mut self, activity: Activity) {
        if !self.is_active(activity) {
            self.active = vec![Activity::Core, activity];
        }
    }

    /// `setActiveActivityIfPossible`.
    pub fn set_active_if_possible(&mut self, activity: Activity, memories: &Memories) {
        if self.requirements_met(activity, memories) {
            self.set_active(activity);
        } else {
            self.set_active(self.default);
        }
    }

    /// `updateActivityFromSchedule`: at most once a second, the villager
    /// timeline's activity for the day time.
    pub fn update_from_schedule(&mut self, time: i64, day_time: i64, memories: &Memories) {
        if time - self.last_schedule_update > 20 {
            self.last_schedule_update = time;
            let scheduled = scheduled_activity(self.baby, day_time);
            if !self.is_active(scheduled) {
                self.set_active_if_possible(scheduled, memories);
            }
        }
    }
}

/// `minecraft:timeline/villager_schedule`: the adult's and the baby's
/// activity by the time of day (the last keyframe at or before it).
fn scheduled_activity(baby: bool, day_time: i64) -> Activity {
    let t = day_time.rem_euclid(24000);
    let keys: &[(i64, Activity)] = if baby {
        &[(10, Activity::Idle), (3000, Activity::Play), (6000, Activity::Idle), (10000, Activity::Play), (12000, Activity::Rest)]
    } else {
        &[(10, Activity::Idle), (2000, Activity::Work), (9000, Activity::Meet), (11000, Activity::Idle), (12000, Activity::Rest)]
    };
    keys.iter().rev().find(|&&(at, _)| t >= at).map_or(Activity::Rest, |&(_, activity)| activity)
}

/// `BehaviorControl`.
pub trait Behavior: Send + Sync {
    fn status(&self) -> Status;
    fn try_start(&mut self, ctx: &mut Ctx, time: i64) -> bool;
    fn tick_or_stop(&mut self, ctx: &mut Ctx, time: i64);
    fn do_stop(&mut self, ctx: &mut Ctx, time: i64);
    /// How the harness names a running one (a gate lists its running
    /// children).
    fn describe(&self) -> String;
    /// A gate's shuffling random, its child gates' in order after it.
    fn shuffles(&mut self) -> Vec<&mut LegacyRandom> {
        Vec::new()
    }
    fn clone_box(&self) -> Box<dyn Behavior>;
}

impl Clone for Box<dyn Behavior> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// One behavior of the brain with its priority and activity, and the
/// index it has within them (for seeding).
#[derive(Clone)]
struct Entry {
    priority: i32,
    activity: Activity,
    index: usize,
    behavior: Box<dyn Behavior>,
}

/// `Sensor`: a scan every `rate` ticks.
#[derive(Clone, Debug)]
pub struct Sensor {
    pub kind: SensorKind,
    pub rate: i32,
    pub time_to_tick: i64,
    /// `NearestBedSensor`'s batch cache, tries and last update.
    pub bed: BedSearch,
}

/// `NearestBedSensor`'s state: beds tried lately (until when), and this
/// scan's tries.
#[derive(Clone, Debug, Default)]
pub struct BedSearch {
    cache: Vec<((i32, i32, i32), i64)>,
    tried: i32,
    last_update: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensorKind {
    NearestLivingEntities,
    NearestPlayers,
    NearestItems,
    NearestBed,
    HurtBy,
    VillagerHostiles,
    VillagerBabies,
    SecondaryPois,
    GolemDetected,
}

impl SensorKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::NearestLivingEntities => "minecraft:nearest_living_entities",
            Self::NearestPlayers => "minecraft:nearest_players",
            Self::NearestItems => "minecraft:nearest_items",
            Self::NearestBed => "minecraft:nearest_bed",
            Self::HurtBy => "minecraft:hurt_by",
            Self::VillagerHostiles => "minecraft:villager_hostiles",
            Self::VillagerBabies => "minecraft:villager_babies",
            Self::SecondaryPois => "minecraft:secondary_pois",
            Self::GolemDetected => "minecraft:golem_detected",
        }
    }
}

/// The villager's brain.
#[derive(Clone)]
pub struct Brain {
    pub memories: Memories,
    pub sensors: Vec<Sensor>,
    pub activities: Activities,
    table: Vec<Entry>,
    unseeded: LegacyRandom,
    /// What it was built for.
    baby: bool,
    /// Times rebuilt, and the game time of the last rebuild.
    pub generation: u32,
    pub rebuilt_at: Option<i64>,
}

/// `VillagerHostilesSensor.ACCEPTABLE_DISTANCE_FROM_HOSTILES`.
pub fn hostile_distance(kind: &str) -> Option<f32> {
    Some(match kind {
        "minecraft:drowned" | "minecraft:husk" | "minecraft:vex" | "minecraft:zombie" | "minecraft:zombie_villager" => 8.0,
        "minecraft:evoker" | "minecraft:illusioner" | "minecraft:ravager" => 12.0,
        "minecraft:pillager" => 15.0,
        "minecraft:vindicator" | "minecraft:zoglin" => 10.0,
        _ => return None,
    })
}

impl Brain {
    /// `BRAIN_PROVIDER.makeBrain` and `registerBrainGoals`: the sensors
    /// (each waiting `random.nextInt(rate)` from the given random), the
    /// packages, and the schedule's activity for now.
    pub fn new(baby: bool, profession: Profession, random: &mut LegacyRandom, time: i64, day_time: i64) -> Self {
        Self::with_memories(baby, profession, random, time, day_time, Memories::default())
    }

    /// `BRAIN_PROVIDER.makeBrain` with the given memories, then
    /// `registerBrainGoals`' schedule update.
    fn with_memories(baby: bool, profession: Profession, random: &mut LegacyRandom, time: i64, day_time: i64, memories: Memories) -> Self {
        use SensorKind::*;
        let sensors = [(NearestLivingEntities, 20), (NearestPlayers, 20), (NearestItems, 20), (NearestBed, 20), (HurtBy, 20), (VillagerHostiles, 20), (VillagerBabies, 20), (SecondaryPois, 40), (GolemDetected, 200)]
            .into_iter()
            .map(|(kind, rate)| Sensor { kind, rate, time_to_tick: i64::from(random.next_int(rate as u32)), bed: BedSearch::default() })
            .collect();
        let mut table = Vec::new();
        for (activity, package) in behaviors::packages(baby, profession) {
            let mut indices: HashMap<i32, usize> = HashMap::new();
            for (priority, behavior) in package {
                let index = indices.entry(priority).or_default();
                table.push(Entry { priority, activity, index: *index, behavior });
                *index += 1;
            }
        }
        // `TreeMap` by priority, then the activity map's order, then the
        // package's.
        table.sort_by_key(|e| (e.priority, MAP_ORDER.iter().position(|&a| a == e.activity).unwrap()));
        let mut brain = Self { memories, sensors, activities: Activities::new(baby), table, unseeded: LegacyRandom::new(0), baby, generation: 0, rebuilt_at: None };
        brain.activities.update_from_schedule(time, day_time, &brain.memories);
        brain
    }

    /// `Villager.refreshBrain` (after its running behaviors stopped): a new
    /// brain for its age and profession, keeping the memories that pack,
    /// its sensors' first scans drawn from the villager's random; its
    /// shuffles take the old brain's unseeded random's next seeds.
    fn rebuild(&mut self, baby: bool, profession: Profession, random: &mut LegacyRandom, time: i64, day_time: i64) {
        let memories = self.memories.persistent();
        let mut unseeded = self.unseeded.clone();
        let generation = self.generation + 1;
        *self = Self::with_memories(baby, profession, random, time, day_time, memories);
        self.seed_unseeded(|| unseeded.next_long());
        self.generation = generation;
        self.rebuilt_at = Some(time);
    }

    /// A rebuild outside a brain tick (`ageBoundaryReached`): the running
    /// behaviors stop, then the brain is rebuilt.
    pub fn refresh(&mut self, mut ctx_parts: CtxParts, baby: bool) {
        let Self { memories, activities, table, unseeded, .. } = self;
        let mut ctx = ctx_parts.ctx(memories, activities, unseeded);
        let time = ctx.time;
        for entry in table.iter_mut().filter(|e| e.behavior.status() == Status::Running) {
            entry.behavior.do_stop(&mut ctx, time);
        }
        drop(ctx);
        let profession = *ctx_parts.me.profession;
        let (time, day_time) = (ctx_parts.time, ctx_parts.day_time);
        self.rebuild(baby, profession, ctx_parts.me.random, time, day_time);
    }

    /// The harness's reseeding of the shuffling lists alone (a rebuilt
    /// brain's), each by its key as in [`Brain::reseed`].
    pub fn reseed_lists(&mut self, seed: u64) {
        for entry in &mut self.table {
            let key = format!("{}/{}/{}", entry.activity.name(), entry.priority, entry.index);
            for random in entry.behavior.shuffles() {
                *random = LegacyRandom::new(seed ^ java_hash(&key) as i64 as u64);
            }
        }
    }

    /// The harness's reseeding: each shuffling list takes
    /// `seed ^ "activity/priority/index[/child...]".hashCode()`, and sensor
    /// `i` waits `LegacyRandom(seed ^ "sensor/i".hashCode()).nextInt(rate)`.
    pub fn reseed(&mut self, seed: u64) {
        for entry in &mut self.table {
            let key = format!("{}/{}/{}", entry.activity.name(), entry.priority, entry.index);
            for random in entry.behavior.shuffles() {
                *random = LegacyRandom::new(seed ^ java_hash(&key) as i64 as u64);
            }
        }
        for (i, sensor) in self.sensors.iter_mut().enumerate() {
            let mut random = LegacyRandom::new(seed ^ java_hash(&format!("sensor/{i}")) as i64 as u64);
            sensor.time_to_tick = i64::from(random.next_int(sensor.rate as u32));
        }
        self.unseeded = LegacyRandom::new(seed ^ java_hash("unseeded") as i64 as u64);
    }

    /// Vanilla seeds each shuffling list (`RandomSource.create()`) and the
    /// shared `Collections.shuffle` random from the clock; this seeds them
    /// from `seeds` (the world's seed uniquifier chain) so no two brains
    /// shuffle alike and runs repeat.
    pub fn seed_unseeded(&mut self, mut seeds: impl FnMut() -> u64) {
        for entry in &mut self.table {
            for random in entry.behavior.shuffles() {
                *random = LegacyRandom::new(seeds());
            }
        }
        self.unseeded = LegacyRandom::new(seeds());
    }

    /// `getRunningBehaviors`, named as the harness names them.
    pub fn running(&self) -> Vec<String> {
        self.table.iter().filter(|e| e.behavior.status() == Status::Running).map(|e| e.behavior.describe()).collect()
    }

    /// `Brain.tick`: memories age, sensors scan (over `nearby`, see
    /// [`sense`]; `hurt` is the last damage within 40 ticks and who dealt
    /// it), stopped behaviors of the active activities try to start,
    /// running ones tick.
    pub fn tick(&mut self, mut ctx_parts: CtxParts, nearby: &[u64], hurt: Option<(String, Option<u64>)>) {
        self.memories.tick();
        sense(&mut self.memories, &mut self.sensors, &mut ctx_parts, nearby, hurt);
        let Self { memories, activities, table, unseeded, .. } = self;
        let mut ctx = ctx_parts.ctx(memories, activities, unseeded);
        for i in 0..table.len() {
            if ctx.activities.is_active(table[i].activity) && table[i].behavior.status() == Status::Stopped {
                let time = ctx.time;
                table[i].behavior.try_start(&mut ctx, time);
                if ctx.refresh {
                    // `refreshBrain` inside the trigger: `stopAll`, then a
                    // new brain. The old brain's tick runs on unseen (its
                    // later behaviors touch only its discarded memories).
                    for entry in table.iter_mut().filter(|e| e.behavior.status() == Status::Running) {
                        entry.behavior.do_stop(&mut ctx, time);
                    }
                    drop(ctx);
                    let profession = *ctx_parts.me.profession;
                    let (baby, day_time) = (self.baby, ctx_parts.day_time);
                    self.rebuild(baby, profession, ctx_parts.me.random, time, day_time);
                    return;
                }
            }
        }
        let running: Vec<usize> = (0..table.len()).filter(|&i| table[i].behavior.status() == Status::Running).collect();
        for i in running {
            let time = ctx.time;
            table[i].behavior.tick_or_stop(&mut ctx, time);
        }
    }
}

/// The parts of a [`Ctx`] besides the brain's own.
pub struct CtxParts<'a> {
    pub world: &'a dyn World,
    pub me: Me<'a>,
    pub others: &'a [Seen],
    pub level_random: &'a mut LegacyRandom,
    pub time: i64,
    pub day_time: i64,
    pub remote: &'a mut Vec<Remote>,
    pub pois: &'a mut PoiManager,
}

impl<'a> CtxParts<'a> {
    fn ctx<'b>(&'b mut self, mem: &'b mut Memories, activities: &'b mut Activities, unseeded: &'b mut LegacyRandom) -> Ctx<'b>
    where
        'a: 'b,
    {
        Ctx {
            unseeded,
            pois: &mut *self.pois,
            refresh: false,
            world: self.world,
            mem,
            activities,
            me: Me {
                id: self.me.id,
                body: &mut *self.me.body,
                sleeping: &mut *self.me.sleeping,
                yaw: &mut *self.me.yaw,
                profession: &mut *self.me.profession,
                xp: self.me.xp,
                level: self.me.level,
                sounds: &mut *self.me.sounds,
                doors: &mut *self.me.doors,
                fluid: self.me.fluid,
                eye_height: self.me.eye_height,
                baby: self.me.baby,
                navigation: &mut *self.me.navigation,
                paths: &mut *self.me.paths,
                profile: self.me.profile,
                look_at: &mut *self.me.look_at,
                jump: &mut *self.me.jump,
                random: &mut *self.me.random,
                sight: &mut *self.me.sight,
                last_gossip_time: self.me.last_gossip_time,
                no_ai: self.me.no_ai,
                gossips: &mut *self.me.gossips,
                trading_player: self.me.trading_player,
                hurt_time: self.me.hurt_time,
                held_item: &mut *self.me.held_item,
                offers: self.me.offers.reborrow(),
                inventory: &mut *self.me.inventory,
                food_level: &mut *self.me.food_level,
                age: &mut *self.me.age,
                can_pick_up_loot: self.me.can_pick_up_loot,
                mob_griefing: self.me.mob_griefing,
                tick_count: self.me.tick_count,
                blocks: &mut *self.me.blocks,
            },
            others: self.others,
            level_random: &mut *self.level_random,
            time: self.time,
            day_time: self.day_time,
            remote: &mut *self.remote,
        }
    }
}

/// `NearestBedSensor.doTick` for a baby: up to four beds within 48 not
/// tried lately (each then kept 40 ticks past this scan's update time,
/// drawn from the level's random), and the nearest bed a path reaches;
/// when none does, it forgets the beds whose time has passed.
fn nearest_bed(memories: &mut Memories, search: &mut BedSearch, parts: &mut CtxParts) {
    search.tried = 0;
    search.last_update = parts.time + i64::from(parts.level_random.next_int(20));
    let here = block_of(parts.me.body.position);
    let mut targets = JavaPosSet::default();
    for record in parts.pois.in_range(&|t| t == PoiType::Home, here, 48, Occupancy::Any) {
        let pos = record.pos;
        if search.cache.iter().any(|&(p, _)| p == pos) {
            continue;
        }
        search.tried += 1;
        if search.tried >= 5 {
            continue;
        }
        search.cache.push((pos, search.last_update + 40));
        targets.insert(pos);
    }
    let targets: Vec<(i32, i32, i32)> = targets.iter().collect();
    let me = &mut parts.me;
    let path = plan_walk_path_to_any(me.body, me.navigation, parts.world, me.profile, me.fluid, &targets, PoiType::Home.valid_range());
    match path.filter(|p| p.reached()).and_then(|p| p.target()) {
        Some(target) => {
            if parts.pois.kind(target).is_some() {
                memories.nearest_bed.set(target);
            }
        }
        None => {
            if search.tried < 5 {
                let last = search.last_update;
                search.cache.retain(|&(_, until)| until >= last);
            }
        }
    }
}

/// `Entity.blockPosition`.
pub fn block_of(position: DVec3) -> (i32, i32, i32) {
    (position.x.floor() as i32, position.y.floor() as i32, position.z.floor() as i32)
}

/// `String.hashCode`.
pub fn java_hash(text: &str) -> i32 {
    text.encode_utf16().fold(0_i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

/// The sensors' scans (`Sensor.tick`, each `doTick`) for a villager at
/// `me`, over `nearby` (the living entities the level's entity sections
/// hold within the search box, in their order; the villager itself left
/// out).
pub fn sense(memories: &mut Memories, sensors: &mut [Sensor], parts: &mut CtxParts, nearby: &[u64], hurt: Option<(String, Option<u64>)>) {
    for sensor in sensors.iter_mut() {
        sensor.time_to_tick -= 1;
        if sensor.time_to_tick > 0 {
            continue;
        }
        sensor.time_to_tick = i64::from(sensor.rate);
        let me = parts.me.body.position;
        match sensor.kind {
            SensorKind::NearestLivingEntities => {
                // Alive and in the box inflated by the follow range (16),
                // nearest first (a stable sort).
                let half = f64::from(parts.me.body.width) / 2.0;
                let (min, max) = (me - DVec3::new(half + 16.0, 16.0, half + 16.0), me + DVec3::new(half + 16.0, f64::from(parts.me.body.height) + 16.0, half + 16.0));
                let mut found: Vec<(u64, f64)> = nearby
                    .iter()
                    .filter_map(|&id| parts.others.iter().find(|s| s.id == id))
                    .filter(|s| s.alive && {
                        let h = f64::from(s.width) / 2.0;
                        s.position.x - h < max.x && s.position.x + h > min.x && s.position.y < max.y && s.position.y + f64::from(s.height) > min.y && s.position.z - h < max.z && s.position.z + h > min.z
                    })
                    .map(|s| (s.id, s.position.distance_squared(me)))
                    .collect();
                found.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                memories.set_mobs(found.into_iter().map(|(id, _)| id).collect());
            }
            SensorKind::NearestPlayers => {
                // The harness's probes are not in the level's player list
                // for brains here; real players come with the world.
                let mut players: Vec<(u64, f64)> = parts
                    .others
                    .iter()
                    .filter(|s| s.kind == "minecraft:player" && !s.spectator && s.position.distance_squared(me) < 16.0 * 16.0)
                    .map(|s| (s.id, s.position.distance_squared(me)))
                    .collect();
                players.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                let players: Vec<u64> = players.into_iter().map(|(id, _)| id).collect();
                memories.set_players(players.clone());
                let observer = Observer { id: parts.me.id, position: me, eye_height: parts.me.eye_height };
                let mut visible = Visible::new(players.clone());
                let seen: Vec<u64> = visible.find_all(&observer, parts.others, parts.world, parts.me.sight, |_| true);
                memories.nearest_visible_player.set_or_erase(seen.first().copied());
                memories.set_attackable_players(seen.clone());
                memories.nearest_visible_attackable_player.set_or_erase(seen.first().copied());
            }
            SensorKind::NearestItems => {
                // `NearestItemSensor`: the items in its box grown by 32, 16
                // and 32, nearest first (a stable sort); the first it wants,
                // within 32 and in sight.
                let body = &parts.me.body;
                let half = f64::from(body.width) / 2.0;
                let (min, max) = (me - DVec3::new(half + 32.0, 16.0, half + 32.0), me + DVec3::new(half + 32.0, f64::from(body.height) + 16.0, half + 32.0));
                let mut items = parts.world.items_in(min, max);
                items.sort_by(|a, b| a.position.distance_squared(me).partial_cmp(&b.position.distance_squared(me)).unwrap_or(std::cmp::Ordering::Equal));
                let eye = me + DVec3::Y * f64::from(parts.me.eye_height);
                let profession = *parts.me.profession;
                let found = items.into_iter().find(|item| {
                    crate::villager_inventory::wants_to_pick_up(parts.me.inventory, profession, &item.item, item.components.as_ref())
                        && item.position.distance_squared(me) < 32.0 * 32.0
                        && crate::sight::line_of_sight(parts.world, eye, item.position + DVec3::Y * 0.2125)
                });
                memories.nearest_visible_wanted_item.set_or_erase(found.map(|item| ITEM_TARGET + item.id as u64));
            }
            SensorKind::SecondaryPois => {
                // `SecondaryPoiSensor`: its profession's fields within 4
                // across and 2 up or down (x, then y, then z).
                let profession = *parts.me.profession;
                let at = block_of(me);
                let mut sites = Vec::new();
                for x in -4..=4 {
                    for y in -2..=2 {
                        for z in -4..=4 {
                            let pos = (at.0 + x, at.1 + y, at.2 + z);
                            if parts.world.block(pos).is_some_and(|b| profession.secondary_poi(&b.id)) {
                                sites.push(pos);
                            }
                        }
                    }
                }
                memories.secondary_job_site.set_or_erase((!sites.is_empty()).then_some(sites));
            }
            SensorKind::NearestBed => {
                if parts.me.baby {
                    nearest_bed(memories, &mut sensor.bed, parts);
                }
            }
            SensorKind::HurtBy => {
                match &hurt {
                    Some((kind, attacker)) => {
                        memories.hurt_by.set(kind.clone());
                        if let Some(attacker) = *attacker {
                            memories.hurt_by_entity.set(attacker);
                        }
                    }
                    None => memories.hurt_by.erase(),
                }
                // A dead or departed attacker is forgotten.
                if let Some(&attacker) = memories.hurt_by_entity.get() {
                    if !parts.others.iter().any(|s| s.id == attacker && s.alive) {
                        memories.hurt_by_entity.erase();
                    }
                }
            }
            SensorKind::VillagerHostiles => {
                let observer = Observer { id: parts.me.id, position: me, eye_height: parts.me.eye_height };
                let hostile = memories.visible_mobs.get_mut().and_then(|visible| {
                    visible.find_closest(&observer, parts.others, parts.world, parts.me.sight, |s| {
                        hostile_distance(s.kind).is_some_and(|d| s.position.distance_squared(me) <= f64::from(d * d))
                    })
                });
                memories.nearest_hostile.set_or_erase(hostile);
            }
            SensorKind::VillagerBabies => {
                let observer = Observer { id: parts.me.id, position: me, eye_height: parts.me.eye_height };
                let babies = memories
                    .visible_mobs
                    .get_mut()
                    .map(|visible| visible.find_all(&observer, parts.others, parts.world, parts.me.sight, |s| s.kind == "minecraft:villager" && s.baby))
                    .unwrap_or_default();
                memories.set_babies(babies);
            }
            SensorKind::GolemDetected => {
                let golem = memories.mobs.get().is_some_and(|ids| ids.iter().any(|&id| parts.others.iter().any(|s| s.id == id && s.kind == "minecraft:iron_golem")));
                if golem {
                    memories.golem_detected_recently.set_for((), 599);
                }
            }
        }
    }
}
