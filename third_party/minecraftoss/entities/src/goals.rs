//! Ordered goal arbitration, source-informed by 26.3 GoalSelector/WrappedGoal.
//! Goals own behavior state; the context supplies the shared mob/world services.

use minecraftoss_player::World;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Control {
    Move,
    Look,
    Jump,
    Target,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls(u8);
impl Controls {
    pub fn new(flags: &[Control]) -> Self {
        Self(
            flags
                .iter()
                .fold(0, |bits, flag| bits | (1 << *flag as usize)),
        )
    }
    fn iter(self) -> impl Iterator<Item = usize> {
        (0..4).filter(move |flag| self.0 & (1 << flag) != 0)
    }
    fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

/// Goals are plain data (their state lives in the mob), so mobs can move
/// to the server thread.
pub trait Goal<C>: GoalClone<C> + Send {
    fn controls(&self) -> Controls;
    /// `canUse`; `world` is the level as the mob sees it now.
    fn can_start(&mut self, context: &mut C, world: &dyn World) -> bool;
    fn can_continue(&mut self, context: &mut C, world: &dyn World) -> bool {
        self.can_start(context, world)
    }
    fn interruptible(&self) -> bool {
        true
    }
    fn every_tick(&self) -> bool {
        false
    }
    fn start(&mut self, _context: &mut C) {}
    fn stop(&mut self, _context: &mut C) {}
    fn tick(&mut self, _context: &mut C) {}
    /// `start` for goals that read the world (a path to follow).
    fn start_world(&mut self, context: &mut C, _world: &dyn World) {
        self.start(context);
    }
    /// `tick` for goals that read the world (sight, paths).
    fn tick_world(&mut self, context: &mut C, _world: &dyn World) {
        self.tick(context);
    }
}

/// Copies a boxed goal (goals keep their state in the mob, so a copy of a
/// selector behaves as the original from the same mob state).
pub trait GoalClone<C> {
    fn clone_box(&self) -> Box<dyn Goal<C>>;
}

impl<C, T: Goal<C> + Clone + 'static> GoalClone<C> for T {
    fn clone_box(&self) -> Box<dyn Goal<C>> {
        Box::new(self.clone())
    }
}

struct Entry<C> {
    priority: i32,
    enabled: bool,
    running: bool,
    goal: Box<dyn Goal<C>>,
}

impl<C> Clone for Entry<C> {
    fn clone(&self) -> Self {
        Self { priority: self.priority, enabled: self.enabled, running: self.running, goal: self.goal.clone_box() }
    }
}

pub struct GoalSelector<C> {
    entries: Vec<Entry<C>>,
    owners: [Option<usize>; 4],
    disabled: Controls,
}

impl<C> Clone for GoalSelector<C> {
    fn clone(&self) -> Self {
        Self { entries: self.entries.clone(), owners: self.owners, disabled: self.disabled }
    }
}
impl<C> Default for GoalSelector<C> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            owners: [None; 4],
            disabled: Controls::default(),
        }
    }
}
impl<C> GoalSelector<C> {
    /// Registration order is preserved, even when priorities are out of order.
    pub fn add(&mut self, priority: i32, goal: impl Goal<C> + 'static) -> usize {
        let id = self.entries.len();
        self.entries.push(Entry {
            priority,
            enabled: true,
            running: false,
            goal: Box::new(goal),
        });
        id
    }
    pub fn set_enabled(&mut self, control: Control, enabled: bool) {
        let bit = 1 << control as usize;
        if enabled {
            self.disabled.0 &= !bit;
        } else {
            self.disabled.0 |= bit;
        }
    }
    pub fn running(&self, id: usize) -> bool {
        self.entries[id].running
    }
    pub fn enabled(&self, id: usize) -> bool {
        self.entries[id].enabled
    }
    pub fn running_ids(&self) -> impl Iterator<Item = usize> + '_ {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(id, entry)| entry.running.then_some(id))
    }
    /// Apply a harness goal filter before the first tick of an entity.
    pub fn retain_ids(&mut self, keep: &[usize]) {
        assert!(self.entries.iter().all(|entry| !entry.running));
        for (id, entry) in self.entries.iter_mut().enumerate() {
            entry.enabled = keep.contains(&id);
        }
    }
    fn stop(&mut self, id: usize, context: &mut C) {
        if self.entries[id].running {
            self.entries[id].running = false;
            self.entries[id].goal.stop(context);
        }
    }
    pub fn tick(&mut self, context: &mut C, world: &dyn World) {
        for id in 0..self.entries.len() {
            if self.entries[id].running
                && (!self.entries[id].enabled
                    || self.entries[id].goal.controls().intersects(self.disabled)
                    || !self.entries[id].goal.can_continue(context, world))
            {
                self.stop(id, context);
            }
        }
        for owner in &mut self.owners {
            if owner.is_some_and(|id| !self.entries[id].running) {
                *owner = None;
            }
        }
        for id in 0..self.entries.len() {
            let entry = &self.entries[id];
            let controls = entry.goal.controls();
            if !entry.enabled || entry.running || controls.intersects(self.disabled) {
                continue;
            }
            let replaceable = controls.iter().all(|flag| {
                self.owners[flag].map_or(entry.priority < i32::MAX, |owner| {
                    self.entries[owner].goal.interruptible()
                        && entry.priority < self.entries[owner].priority
                })
            });
            if !replaceable || !self.entries[id].goal.can_start(context, world) {
                continue;
            }
            for flag in controls.iter() {
                if let Some(owner) = self.owners[flag] {
                    self.stop(owner, context);
                }
                self.owners[flag] = Some(id);
            }
            self.entries[id].running = true;
            self.entries[id].goal.start_world(context, world);
        }
        self.tick_running_world(context, world, true);
    }
    /// `tickRunningGoals` for goals that read the world.
    pub fn tick_running_world(&mut self, context: &mut C, world: &dyn World, all: bool) {
        for entry in &mut self.entries {
            if entry.running && (all || entry.goal.every_tick()) {
                entry.goal.tick_world(context, world);
            }
        }
    }
    pub fn tick_running(&mut self, context: &mut C, all: bool) {
        for entry in &mut self.entries {
            if entry.running && (all || entry.goal.every_tick()) {
                entry.goal.tick(context);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NoWorld;
    impl World for NoWorld {
        fn block(&self, _pos: minecraftoss_player::Pos) -> Option<minecraftoss_player::Block> {
            None
        }
        fn set_block(&mut self, _pos: minecraftoss_player::Pos, _block: Option<minecraftoss_player::Block>) {}
    }
    #[derive(Default)]
    struct Context {
        enabled: [bool; 3],
        events: Vec<String>,
    }
    #[derive(Clone)]
    struct Probe {
        id: usize,
        flags: Controls,
        every: bool,
    }
    impl Goal<Context> for Probe {
        fn controls(&self) -> Controls {
            self.flags
        }
        fn can_start(&mut self, c: &mut Context, _world: &dyn World) -> bool {
            c.events.push(format!("use{}", self.id));
            c.enabled[self.id]
        }
        fn can_continue(&mut self, c: &mut Context, _world: &dyn World) -> bool {
            c.events.push(format!("continue{}", self.id));
            c.enabled[self.id]
        }
        fn start(&mut self, c: &mut Context) {
            c.events.push(format!("start{}", self.id));
        }
        fn stop(&mut self, c: &mut Context) {
            c.events.push(format!("stop{}", self.id));
        }
        fn tick(&mut self, c: &mut Context) {
            c.events.push(format!("tick{}", self.id));
        }
        fn every_tick(&self) -> bool {
            self.every
        }
    }
    #[test]
    fn preemption_keeps_other_flag_locked_until_next_cleanup() {
        let mut s = GoalSelector::default();
        s.add(
            2,
            Probe {
                id: 0,
                flags: Controls::new(&[Control::Move, Control::Look]),
                every: false,
            },
        );
        s.add(
            1,
            Probe {
                id: 1,
                flags: Controls::new(&[Control::Move]),
                every: true,
            },
        );
        s.add(
            3,
            Probe {
                id: 2,
                flags: Controls::new(&[Control::Look]),
                every: false,
            },
        );
        let mut c = Context {
            enabled: [true, false, true],
            ..Context::default()
        };
        s.tick(&mut c, &NoWorld);
        assert_eq!(c.events, ["use0", "start0", "use1", "tick0"]);
        c.events.clear();
        c.enabled[1] = true;
        s.tick(&mut c, &NoWorld);
        assert_eq!(c.events, ["continue0", "use1", "stop0", "start1", "tick1"]);
        c.events.clear();
        s.tick(&mut c, &NoWorld);
        assert_eq!(c.events, ["continue1", "use2", "start2", "tick1", "tick2"]);
        c.events.clear();
        s.tick_running(&mut c, false);
        assert_eq!(c.events, ["tick1"]);
        c.events.clear();
        s.set_enabled(Control::Move, false);
        s.tick(&mut c, &NoWorld);
        assert_eq!(c.events, ["stop1", "continue2", "tick2"]);
    }
}
