//! The villager brain's memories (pinned 26.3 `MemoryModuleType`,
//! `MemorySlot`, `WalkTarget`, `EntityTracker`, `BlockPosTracker` and
//! `NearestVisibleLivingEntities`). Every memory the villager brain registers
//! has a slot; one with a lifetime counts down each brain tick and empties
//! when it runs out.
use super::Seen;
use glam::DVec3;
use minecraftoss_player::{Pos, World};
use std::collections::HashMap;

/// `MemorySlot`: a value and its time to live (`Long.MAX_VALUE`: never
/// expires).
#[derive(Clone, Debug)]
pub struct Slot<T> {
    value: Option<T>,
    ttl: i64,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self { value: None, ttl: i64::MAX }
    }
}

impl<T> Slot<T> {
    pub fn get(&self) -> Option<&T> {
        self.value.as_ref()
    }

    pub fn get_mut(&mut self) -> Option<&mut T> {
        self.value.as_mut()
    }

    pub fn present(&self) -> bool {
        self.value.is_some()
    }

    pub fn set(&mut self, value: T) {
        self.value = Some(value);
        self.ttl = i64::MAX;
    }

    pub fn set_for(&mut self, value: T, ttl: i64) {
        self.value = Some(value);
        self.ttl = ttl;
    }

    /// `setMemory(type, Optional)`: a value, or none.
    pub fn set_or_erase(&mut self, value: Option<T>) {
        match value {
            Some(value) => self.set(value),
            None => self.erase(),
        }
    }

    pub fn erase(&mut self) {
        self.value = None;
        self.ttl = i64::MAX;
    }

    /// The time to live, when it can expire.
    pub fn ttl(&self) -> Option<i64> {
        (self.ttl != i64::MAX).then_some(self.ttl)
    }

    /// `MemorySlot.tick`.
    fn tick(&mut self) {
        if self.value.is_some() && self.ttl != i64::MAX {
            if self.ttl <= 0 {
                self.erase();
            } else {
                self.ttl -= 1;
            }
        }
    }
}

/// A slot for a list, emptied when set to an empty one
/// (`Brain.isEmptyCollection`).
fn set_list<T>(slot: &mut Slot<Vec<T>>, list: Vec<T>) {
    if list.is_empty() {
        slot.erase();
    } else {
        slot.set(list);
    }
}

/// `PositionTracker`: an entity, at its feet or its eyes, or a block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tracker {
    /// `EntityTracker(entity, trackEyeHeight)`.
    Entity { id: u64, eyes: bool },
    /// `BlockPosTracker`: the block's centre.
    Block(Pos),
}

impl Tracker {
    /// `currentPosition`; none when the entity is gone.
    pub fn position(self, others: &[Seen]) -> Option<DVec3> {
        match self {
            Self::Entity { id, eyes } => others.iter().find(|s| s.id == id).map(|s| if eyes { s.position + DVec3::Y * f64::from(s.eye_height) } else { s.position }),
            Self::Block((x, y, z)) => Some(DVec3::new(f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5)),
        }
    }

    /// `currentBlockPosition` (an entity's feet block).
    pub fn block(self, others: &[Seen]) -> Option<Pos> {
        match self {
            Self::Entity { id, .. } => others.iter().find(|s| s.id == id).map(|s| (s.position.x.floor() as i32, s.position.y.floor() as i32, s.position.z.floor() as i32)),
            Self::Block(pos) => Some(pos),
        }
    }
}

/// `WalkTarget`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WalkTarget {
    pub target: Tracker,
    pub speed: f32,
    pub close_enough: i32,
}

impl WalkTarget {
    /// `new WalkTarget(Vec3, ...)`: the block containing the position.
    pub fn at(position: DVec3, speed: f32, close_enough: i32) -> Self {
        Self { target: Tracker::Block((position.x.floor() as i32, position.y.floor() as i32, position.z.floor() as i32)), speed, close_enough }
    }
}

/// `NearestVisibleLivingEntities`: the nearby living entities, nearest
/// first, each one's visibility (`Sensor.isEntityTargetable`) worked out
/// the first time it is asked and kept until the next scan.
#[derive(Clone, Debug, Default)]
pub struct Visible {
    pub ids: Vec<u64>,
    seen: HashMap<u64, bool>,
}

impl Visible {
    pub fn new(ids: Vec<u64>) -> Self {
        Self { ids, seen: HashMap::new() }
    }

    /// Whether the observer sees `id`, asked once per scan.
    fn sees(&mut self, id: u64, observer: &Observer, others: &[Seen], world: &dyn World, sight: &mut HashMap<u64, bool>) -> bool {
        if let Some(&known) = self.seen.get(&id) {
            return known;
        }
        let result = targetable(observer, id, others, world, sight);
        self.seen.insert(id, result);
        result
    }

    /// `findClosest`: the first nearby entity the filter takes that is seen.
    pub fn find_closest(&mut self, observer: &Observer, others: &[Seen], world: &dyn World, sight: &mut HashMap<u64, bool>, mut filter: impl FnMut(&Seen) -> bool) -> Option<u64> {
        for index in 0..self.ids.len() {
            let id = self.ids[index];
            let Some(seen) = others.iter().find(|s| s.id == id) else { continue };
            if filter(seen) && self.sees(id, observer, others, world, sight) {
                return Some(id);
            }
        }
        None
    }

    /// `findAll`, eagerly.
    pub fn find_all(&mut self, observer: &Observer, others: &[Seen], world: &dyn World, sight: &mut HashMap<u64, bool>, mut filter: impl FnMut(&Seen) -> bool) -> Vec<u64> {
        let mut out = Vec::new();
        for index in 0..self.ids.len() {
            let id = self.ids[index];
            let Some(seen) = others.iter().find(|s| s.id == id) else { continue };
            if filter(seen) && self.sees(id, observer, others, world, sight) {
                out.push(id);
            }
        }
        out
    }

    /// `contains(entity)`.
    pub fn contains(&mut self, id: u64, observer: &Observer, others: &[Seen], world: &dyn World, sight: &mut HashMap<u64, bool>) -> bool {
        self.ids.contains(&id) && self.sees(id, observer, others, world, sight)
    }

    /// `contains(predicate)`.
    pub fn contains_where(&mut self, observer: &Observer, others: &[Seen], world: &dyn World, sight: &mut HashMap<u64, bool>, filter: impl FnMut(&Seen) -> bool) -> bool {
        self.find_closest(observer, others, world, sight, filter).is_some()
    }
}

/// The brain's own body as its sensors and tests see it.
#[derive(Clone, Copy, Debug)]
pub struct Observer {
    pub id: u64,
    pub position: DVec3,
    pub eye_height: f32,
}

/// `Sensor.isEntityTargetable` (non-combat, within 16 blocks, the target's
/// visibility at full, with line of sight through `Sensing`).
fn targetable(observer: &Observer, id: u64, others: &[Seen], world: &dyn World, sight: &mut HashMap<u64, bool>) -> bool {
    let Some(target) = others.iter().find(|s| s.id == id) else { return false };
    if target.id == observer.id || !target.alive || target.spectator {
        return false;
    }
    let range = 16.0_f64.max(2.0);
    if observer.position.distance_squared(target.position) > range * range {
        return false;
    }
    // `Sensing.hasLineOfSight`: once a tick per target.
    *sight.entry(id).or_insert_with(|| {
        let from = observer.position + DVec3::Y * f64::from(observer.eye_height);
        let to = target.position + DVec3::Y * f64::from(target.eye_height);
        crate::sight::line_of_sight(world, from, to)
    })
}

/// The villager's registered memories (`Villager.BRAIN_PROVIDER`'s sensors
/// and behaviors).
#[derive(Clone, Debug, Default)]
pub struct Memories {
    /// `nearest_living_entities` (observed as `mobs`).
    pub mobs: Slot<Vec<u64>>,
    /// `nearest_visible_living_entities` (`visible_mobs`).
    pub visible_mobs: Slot<Visible>,
    pub nearest_players: Slot<Vec<u64>>,
    pub nearest_visible_player: Slot<u64>,
    pub nearest_visible_attackable_player: Slot<u64>,
    pub nearest_visible_attackable_players: Slot<Vec<u64>>,
    pub nearest_visible_wanted_item: Slot<u64>,
    pub nearest_bed: Slot<Pos>,
    pub hurt_by: Slot<String>,
    pub hurt_by_entity: Slot<u64>,
    pub nearest_hostile: Slot<u64>,
    pub visible_villager_babies: Slot<Vec<u64>>,
    pub secondary_job_site: Slot<Vec<Pos>>,
    pub golem_detected_recently: Slot<()>,
    pub walk_target: Slot<WalkTarget>,
    pub look_target: Slot<Tracker>,
    /// `path`: which path object (see `PathState`).
    pub path: Slot<u64>,
    pub cant_reach_walk_target_since: Slot<i64>,
    pub interaction_target: Slot<u64>,
    pub breed_target: Slot<u64>,
    pub home: Slot<Pos>,
    pub job_site: Slot<Pos>,
    pub potential_job_site: Slot<Pos>,
    pub meeting_point: Slot<Pos>,
    pub heard_bell_time: Slot<i64>,
    pub last_slept: Slot<i64>,
    pub last_woken: Slot<i64>,
    pub last_worked_at_poi: Slot<i64>,
    pub doors_to_close: Slot<Vec<Pos>>,
    pub item_pickup_cooldown_ticks: Slot<i32>,
}

impl Memories {
    /// `Brain.pack` for a villager: only the memories with codecs survive a
    /// rebuilt brain (its points of interest and clocks).
    pub fn persistent(&self) -> Memories {
        Memories {
            home: self.home.clone(),
            job_site: self.job_site.clone(),
            potential_job_site: self.potential_job_site.clone(),
            meeting_point: self.meeting_point.clone(),
            golem_detected_recently: self.golem_detected_recently.clone(),
            last_slept: self.last_slept.clone(),
            last_woken: self.last_woken.clone(),
            last_worked_at_poi: self.last_worked_at_poi.clone(),
            ..Memories::default()
        }
    }

    /// `forgetOutdatedMemories`: every slot with a lifetime counts down.
    pub fn tick(&mut self) {
        self.mobs.tick();
        self.visible_mobs.tick();
        self.nearest_players.tick();
        self.nearest_visible_player.tick();
        self.nearest_visible_attackable_player.tick();
        self.nearest_visible_attackable_players.tick();
        self.nearest_visible_wanted_item.tick();
        self.nearest_bed.tick();
        self.hurt_by.tick();
        self.hurt_by_entity.tick();
        self.nearest_hostile.tick();
        self.visible_villager_babies.tick();
        self.secondary_job_site.tick();
        self.golem_detected_recently.tick();
        self.walk_target.tick();
        self.look_target.tick();
        self.path.tick();
        self.cant_reach_walk_target_since.tick();
        self.interaction_target.tick();
        self.breed_target.tick();
        self.home.tick();
        self.job_site.tick();
        self.potential_job_site.tick();
        self.meeting_point.tick();
        self.heard_bell_time.tick();
        self.last_slept.tick();
        self.last_woken.tick();
        self.last_worked_at_poi.tick();
        self.doors_to_close.tick();
        self.item_pickup_cooldown_ticks.tick();
    }

    pub fn set_mobs(&mut self, ids: Vec<u64>) {
        set_list(&mut self.mobs, ids.clone());
        // `new NearestVisibleLivingEntities(...)`, even for none.
        self.visible_mobs.set(Visible::new(ids));
    }

    pub fn set_babies(&mut self, ids: Vec<u64>) {
        set_list(&mut self.visible_villager_babies, ids);
    }

    pub fn set_players(&mut self, ids: Vec<u64>) {
        set_list(&mut self.nearest_players, ids);
    }

    pub fn set_attackable_players(&mut self, ids: Vec<u64>) {
        set_list(&mut self.nearest_visible_attackable_players, ids);
    }
}
