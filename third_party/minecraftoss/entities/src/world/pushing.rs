//! Mobs pushing one another (pinned 26.3 `LivingEntity.pushEntities`,
//! `LivingEntity.doPush`, `Entity.push`, `Level.getPushableEntities`,
//! `EntitySelector.pushableBy` and `EntitySectionStorage.getEntities`).
//! After its move each living mob finds the pushable entities its box
//! overlaps, in the order the level's entity sections hold them; each is
//! pushed away from it and it away from each, by at most a twentieth of a
//! block a tick, less the farther apart their centres are, unless they
//! stand within a hundredth of a block of each other. A mob among more than
//! `max_entity_cramming - 1` others takes 6 cramming damage a quarter of
//! the time. Bats neither push nor are pushed; the dead, climbing spiders
//! and mobs outside the entity-ticking range are not pushed, though the
//! dying still push others. Real players are entities too: mobs are pushed
//! away from them in their own push and again in the player's
//! (`ServerPlayer.doTick`, after the level's entities).
use super::*;
use crate::inside_blocks::Hazard;
use hazards::Exposed;
use std::collections::BTreeMap;

/// `SectionPos.asLong`.
fn section_key(x: i32, y: i32, z: i32) -> i64 {
    let (x, y, z) = (i64::from(x) as u64, i64::from(y) as u64, i64::from(z) as u64);
    ((x & 0x3F_FFFF) << 42 | (z & 0x3F_FFFF) << 20 | (y & 0xF_FFFF)) as i64
}

/// `SectionPos.asLong(blockPosition())`.
fn section_at(position: DVec3) -> i64 {
    let coord = |v: f64| v.floor() as i32 >> 4;
    section_key(coord(position.x), coord(position.y), coord(position.z))
}

/// An entity in a section: a mob, or a player (real players are entities;
/// the harness's probes are not).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SectionEntry {
    Mob(EntityKey),
    Player(u64),
}

/// A player's box for pushing: `Player.getDefaultDimensions` for its pose,
/// told by its eyes (standing 0.6 by 1.8, crouching 1.5 tall, swimming,
/// crawling or gliding 0.6, asleep 0.2 by 0.2).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlayerBox {
    pub id: u64,
    pub position: DVec3,
    pub width: f32,
    pub height: f32,
    /// Alive and not a spectator (`isPushable`, `NO_SPECTATORS`).
    pub pushable: bool,
}

impl PlayerBox {
    fn of(player: &crate::tempt::PlayerCandidate) -> Self {
        let (width, height) = player_size(player.eye_height);
        Self { id: player.id, position: player.position, width, height, pushable: player.alive && !player.spectator }
    }
}

/// A player's width and height for the pose its eyes tell.
fn player_size(eye_height: f32) -> (f32, f32) {
    match eye_height {
        e if e >= 1.5 => (0.6, 1.8),
        e if e >= 1.0 => (0.6, 1.5),
        e if e >= 0.3 => (0.6, 0.6),
        _ => (0.2, 0.2),
    }
}

/// `EntitySectionStorage` for pushing: each 16-block section's entities in
/// the order they came into it (`ClassInstanceMultiMap`), the sections
/// keyed as vanilla sorts them.
#[derive(Clone, Default)]
pub(crate) struct Sections {
    lists: BTreeMap<i64, Vec<SectionEntry>>,
    filed: HashMap<SectionEntry, i64>,
}

impl Sections {
    /// `EntityCallbacks.onMove`: an entity that moved into another section
    /// leaves its old one and joins the end of the new one.
    pub(crate) fn file(&mut self, entry: SectionEntry, position: DVec3) {
        let section = section_at(position);
        match self.filed.get(&entry) {
            Some(&old) if old == section => return,
            Some(&old) => self.leave(entry, old),
            None => {}
        }
        self.lists.entry(section).or_default().push(entry);
        self.filed.insert(entry, section);
    }

    /// Forgets the entries `keep` refuses (`onRemove`).
    pub(crate) fn retain(&mut self, keep: impl Fn(&SectionEntry) -> bool) {
        self.filed.retain(|entry, _| keep(entry));
        self.lists.retain(|_, list| {
            list.retain(&keep);
            !list.is_empty()
        });
    }

    /// Forgets every mob `keep` refuses; players stay.
    pub(crate) fn retain_mobs(&mut self, keep: impl Fn(u64) -> bool) {
        self.retain(|entry| match entry {
            SectionEntry::Mob(key) => keep(key.id()),
            SectionEntry::Player(_) => true,
        });
    }

    fn leave(&mut self, entry: SectionEntry, section: i64) {
        if let Some(list) = self.lists.get_mut(&section) {
            list.retain(|other| *other != entry);
            if list.is_empty() {
                self.lists.remove(&section);
            }
        }
    }

    /// The living entities of the sections a box query reaches, in their
    /// order: mobs by ID, players by `PLAYER_TARGET` plus theirs.
    pub(crate) fn living_around(&self, min: DVec3, max: DVec3) -> Vec<u64> {
        self.around(min, max)
            .into_iter()
            .map(|entry| match entry {
                SectionEntry::Mob(key) => key.id(),
                SectionEntry::Player(id) => PLAYER_TARGET + id,
            })
            .collect()
    }

    /// The entities of the sections a box query reaches
    /// (`forEachAccessibleNonEmptySection`: two blocks further sideways,
    /// four below), a column of sections at a time from low x, each column
    /// in its keys' order.
    fn around(&self, min: DVec3, max: DVec3) -> Vec<SectionEntry> {
        let coord = |v: f64| v.floor() as i32 >> 4;
        let (x0, y0, z0) = (coord(min.x - 2.0), coord(min.y - 4.0), coord(min.z - 2.0));
        let (x1, y1, z1) = (coord(max.x + 2.0), coord(max.y + 0.0), coord(max.z + 2.0));
        let mut out = Vec::new();
        for x in x0..=x1 {
            for (&key, list) in self.lists.range(section_key(x, 0, 0)..=section_key(x, -1, -1)) {
                // `SectionPos.y` and `SectionPos.z`.
                let (y, z) = ((key << 44 >> 44) as i32, (key << 22 >> 42) as i32);
                if (y0..=y1).contains(&y) && (z0..=z1).contains(&z) {
                    out.extend_from_slice(list);
                }
            }
        }
        out
    }
}

impl EntityWorld {
    /// A living mob's body and whether it lives (an exploded creeper is
    /// gone); none for bats, projectiles and the removed.
    fn push_body(&mut self, key: EntityKey) -> Option<(&mut Body, bool)> {
        match key {
            EntityKey::Zombie(id) => self.zombies.iter_mut().find(|e| e.id == id).map(|e| (&mut e.zombie.body, e.zombie.health > 0.0)),
            EntityKey::Skeleton(id) => self.skeletons.iter_mut().find(|e| e.id == id).map(|e| (&mut e.skeleton.body, e.skeleton.health > 0.0)),
            EntityKey::Creeper(id) => self
                .creepers
                .iter_mut()
                .find(|e| e.id == id)
                .map(|e| (&mut e.creeper.body, e.creeper.health > 0.0 && !e.creeper.exploded)),
            EntityKey::Spider(id) => self.spiders.iter_mut().find(|e| e.id == id).map(|e| (&mut e.spider.body, e.spider.health > 0.0)),
            EntityKey::Slime(id) => self.slimes.iter_mut().find(|e| e.id == id).map(|e| (&mut e.slime.body, e.slime.health > 0.0)),
            EntityKey::Enderman(id) => self.endermen.iter_mut().find(|e| e.id == id).map(|e| (&mut e.enderman.body, e.enderman.health > 0.0)),
            EntityKey::Witch(id) => self.witches.iter_mut().find(|e| e.id == id).map(|e| (&mut e.witch.body, e.witch.health > 0.0)),
            EntityKey::IronGolem(id) => self.iron_golems.iter_mut().find(|e| e.id == id).map(|e| (&mut e.golem.body, e.golem.health > 0.0)),
            EntityKey::Wolf(id) => self.wolves.iter_mut().find(|e| e.id == id).map(|e| (&mut e.wolf.body, e.wolf.health > 0.0)),
            EntityKey::Villager(id) => self.villagers.iter_mut().find(|e| e.id == id).map(|e| (&mut e.villager.body, e.villager.health > 0.0)),
            EntityKey::Cow(id) => self.cows.iter_mut().find(|e| e.id == id).map(|e| (&mut e.cow.body, e.cow.health > 0.0)),
            EntityKey::Sheep(id) => self.sheep.iter_mut().find(|e| e.id == id).map(|e| (&mut e.body, e.health > 0.0)),
            EntityKey::Pig(id) => self.pigs.iter_mut().find(|e| e.id == id).map(|e| (&mut e.pig.body, e.pig.health > 0.0)),
            EntityKey::Chicken(id) => self.chickens.iter_mut().find(|e| e.id == id).map(|e| (&mut e.chicken.body, e.chicken.health > 0.0)),
            EntityKey::Bat(_) | EntityKey::Arrow(_) | EntityKey::Potion(_) => None,
        }
    }

    /// The mob as the hazards see it, to take cramming damage.
    fn exposed_mut(&mut self, key: EntityKey) -> Option<&mut dyn Exposed> {
        match key {
            EntityKey::Zombie(id) => self.zombies.iter_mut().find(|e| e.id == id).map(|e| e as &mut dyn Exposed),
            EntityKey::Skeleton(id) => self.skeletons.iter_mut().find(|e| e.id == id).map(|e| e as &mut dyn Exposed),
            EntityKey::Creeper(id) => self.creepers.iter_mut().find(|e| e.id == id).map(|e| e as &mut dyn Exposed),
            EntityKey::Spider(id) => self.spiders.iter_mut().find(|e| e.id == id).map(|e| e as &mut dyn Exposed),
            EntityKey::Slime(id) => self.slimes.iter_mut().find(|e| e.id == id).map(|e| e as &mut dyn Exposed),
            EntityKey::Enderman(id) => self.endermen.iter_mut().find(|e| e.id == id).map(|e| e as &mut dyn Exposed),
            EntityKey::Witch(id) => self.witches.iter_mut().find(|e| e.id == id).map(|e| e as &mut dyn Exposed),
            EntityKey::IronGolem(id) => self.iron_golems.iter_mut().find(|e| e.id == id).map(|e| e as &mut dyn Exposed),
            EntityKey::Wolf(id) => self.wolves.iter_mut().find(|e| e.id == id).map(|e| e as &mut dyn Exposed),
            EntityKey::Villager(id) => self.villagers.iter_mut().find(|e| e.id == id).map(|e| e as &mut dyn Exposed),
            EntityKey::Cow(id) => self.cows.iter_mut().find(|e| e.id == id).map(|e| e as &mut dyn Exposed),
            EntityKey::Sheep(id) => self.sheep.iter_mut().find(|e| e.id == id).map(|e| e as &mut dyn Exposed),
            EntityKey::Pig(id) => self.pigs.iter_mut().find(|e| e.id == id).map(|e| e as &mut dyn Exposed),
            EntityKey::Chicken(id) => self.chickens.iter_mut().find(|e| e.id == id).map(|e| e as &mut dyn Exposed),
            EntityKey::Bat(_) | EntityKey::Arrow(_) | EntityKey::Potion(_) => None,
        }
    }

    /// Files a new or moved mob in its entity section.
    pub(super) fn file_in_section(&mut self, key: EntityKey) {
        if let Some((body, _)) = self.push_body(key) {
            let position = body.position;
            self.sections.file(SectionEntry::Mob(key), position);
        }
    }

    /// Where a section entry stands, how big it is, and whether it can be
    /// pushed (`isPushable`: alive, not climbing, ticking).
    fn entry_box(&mut self, entry: SectionEntry, ticks: &dyn Fn(DVec3) -> bool) -> Option<(DVec3, f32, f32, bool)> {
        match entry {
            SectionEntry::Mob(key) => {
                let (body, alive) = self.push_body(key)?;
                let p = body.position;
                Some((p, body.width, body.height, alive && !body.climbing && ticks(p)))
            }
            SectionEntry::Player(id) => {
                let player = self.pushing_players.iter().find(|p| p.id == id)?;
                Some((player.position, player.width, player.height, player.pushable))
            }
        }
    }

    /// The pushable entities but `except` whose boxes overlap `min`..`max`
    /// (`getPushableEntities`), in section order, with where they stand.
    fn pushable_around(&mut self, except: SectionEntry, min: DVec3, max: DVec3, ticks: &dyn Fn(DVec3) -> bool) -> Vec<(SectionEntry, DVec3)> {
        let mut pushable = Vec::new();
        for other in self.sections.around(min, max) {
            if other == except {
                continue;
            }
            let Some((p, width, height, can_be_pushed)) = self.entry_box(other, ticks) else { continue };
            let half = f64::from(width / 2.0);
            let overlaps = p.x - half < max.x
                && p.x + half > min.x
                && p.y < max.y
                && p.y + f64::from(height) > min.y
                && p.z - half < max.z
                && p.z + half > min.z;
            if overlaps && can_be_pushed {
                pushable.push((other, p));
            }
        }
        pushable
    }

    /// The real players this tick, filed where they stand (their moves came
    /// in during the last tick's connection tick).
    pub(super) fn set_pushing_players(&mut self, players: &[crate::tempt::PlayerCandidate]) {
        if !self.players_pickable {
            return;
        }
        self.pushing_players = players.iter().map(PlayerBox::of).collect();
        let present: HashSet<u64> = self.pushing_players.iter().map(|p| p.id).collect();
        self.sections.retain(|entry| match entry {
            SectionEntry::Player(id) => present.contains(id),
            SectionEntry::Mob(_) => true,
        });
        for player in self.pushing_players.clone() {
            self.sections.file(SectionEntry::Player(player.id), player.position);
        }
    }

    /// `ServerPlayer.doTick`'s `pushEntities`, after the level's entities
    /// tick: each real player pushes the mobs it overlaps away from it (its
    /// own motion is its client's). Spectators and the dead push nothing.
    /// `isSleeping` for a mob.
    fn sleeping(&self, key: EntityKey) -> bool {
        matches!(key, EntityKey::Villager(id) if self.villagers.iter().any(|e| e.id == id && e.sleeping.is_some()))
    }

    pub fn push_from_players(&mut self, players: &[crate::tempt::PlayerCandidate], ticks: &dyn Fn(DVec3) -> bool) {
        if !self.players_pickable {
            return;
        }
        // The players moved (`ServerGamePacketListenerImpl`) before their
        // tick.
        self.set_pushing_players(players);
        for player in self.pushing_players.clone() {
            if !player.pushable {
                continue;
            }
            let half = f64::from(player.width / 2.0);
            let (min, max) = (player.position - DVec3::new(half, 0.0, half), player.position + DVec3::new(half, f64::from(player.height), half));
            for (other, other_position) in self.pushable_around(SectionEntry::Player(player.id), min, max, ticks) {
                let SectionEntry::Mob(key) = other else { continue };
                if self.sleeping(key) {
                    continue;
                }
                if let Some((xa, za)) = push_step(player.position, other_position) {
                    if let Some((body, _)) = self.push_body(key) {
                        body.velocity += DVec3::new(-xa, 0.0, -za);
                        body.needs_sync = true;
                    }
                }
            }
        }
    }

    /// The client's mobs pushing its own player: on the client
    /// `pushableBy` finds only the local player, so each mob but a bat
    /// whose box overlaps the player's pushes it away in its tick. Returns
    /// the impulses in the order the mobs tick.
    pub fn pushes_on_local_player(&self, position: DVec3, eye_height: f32) -> Vec<DVec3> {
        let (width, height) = player_size(eye_height);
        let half = f64::from(width / 2.0);
        let (min, max) = (position - DVec3::new(half, 0.0, half), position + DVec3::new(half, f64::from(height), half));
        let mut pushes = Vec::new();
        for &key in &self.order {
            let body = match key {
                EntityKey::Zombie(id) => self.zombies.iter().find(|e| e.id == id).map(|e| &e.zombie.body),
                EntityKey::Skeleton(id) => self.skeletons.iter().find(|e| e.id == id).map(|e| &e.skeleton.body),
                EntityKey::Creeper(id) => self.creepers.iter().find(|e| e.id == id && !e.creeper.exploded).map(|e| &e.creeper.body),
                EntityKey::Spider(id) => self.spiders.iter().find(|e| e.id == id).map(|e| &e.spider.body),
                EntityKey::Slime(id) => self.slimes.iter().find(|e| e.id == id).map(|e| &e.slime.body),
                EntityKey::Enderman(id) => self.endermen.iter().find(|e| e.id == id).map(|e| &e.enderman.body),
                EntityKey::Witch(id) => self.witches.iter().find(|e| e.id == id).map(|e| &e.witch.body),
                EntityKey::IronGolem(id) => self.iron_golems.iter().find(|e| e.id == id).map(|e| &e.golem.body),
                EntityKey::Wolf(id) => self.wolves.iter().find(|e| e.id == id).map(|e| &e.wolf.body),
                EntityKey::Villager(id) => self.villagers.iter().find(|e| e.id == id).map(|e| &e.villager.body),
                EntityKey::Cow(id) => self.cows.iter().find(|e| e.id == id).map(|e| &e.cow.body),
                EntityKey::Sheep(id) => self.sheep.iter().find(|e| e.id == id).map(|e| &e.body),
                EntityKey::Pig(id) => self.pigs.iter().find(|e| e.id == id).map(|e| &e.pig.body),
                EntityKey::Chicken(id) => self.chickens.iter().find(|e| e.id == id).map(|e| &e.chicken.body),
                EntityKey::Bat(_) | EntityKey::Arrow(_) | EntityKey::Potion(_) => None,
            };
            let Some(body) = body else { continue };
            let p = body.position;
            let half = f64::from(body.width / 2.0);
            let overlaps = p.x - half < max.x
                && p.x + half > min.x
                && p.y < max.y
                && p.y + f64::from(body.height) > min.y
                && p.z - half < max.z
                && p.z + half > min.z;
            if overlaps {
                // `player.push(mob)`: the player away from the mob.
                if let Some((xa, za)) = push_step(p, position) {
                    pushes.push(DVec3::new(-xa, 0.0, -za));
                }
            }
        }
        pushes
    }

    /// `LivingEntity.pushEntities` for the mob `key` after its move (and
    /// its `applyEffectsFromBlocks`). `ticks` tells whether a position is
    /// in the entity-ticking range (`isPositionEntityTicking`).
    pub(super) fn push_entities(&mut self, key: EntityKey, world: &dyn World, game_time: i64, ticks: &dyn Fn(DVec3) -> bool) {
        // The move filed it wherever it went.
        self.file_in_section(key);
        let Some((body, _)) = self.push_body(key) else { return };
        let position = body.position;
        let half = f64::from(body.width / 2.0);
        let (min, max) = (position - DVec3::new(half, 0.0, half), position + DVec3::new(half, f64::from(body.height), half));
        let pushable = self.pushable_around(SectionEntry::Mob(key), min, max, ticks);
        if pushable.is_empty() {
            return;
        }
        let limit = self.max_entity_cramming;
        if limit > 0 && pushable.len() as i32 > limit - 1 {
            if let Some(mob) = self.exposed_mut(key) {
                // No passengers: every one counts.
                if mob.random().next_int(4) == 0 {
                    mob.hurt_hazard(Hazard::Cramming, 6.0, world, game_time);
                }
            }
        }
        for (other, other_position) in pushable {
            // `IronGolem.doPush`: one push in twenty on an enemy other than a
            // creeper makes it the golem's target.
            if let (EntityKey::IronGolem(golem), SectionEntry::Mob(other_key)) = (key, other) {
                let enemy = self.entity_type(other_key.id()).is_some_and(|kind| crate::monster_ai::is_enemy(kind) && kind != "minecraft:creeper");
                if let Some(e) = self.iron_golems.iter_mut().find(|e| e.id == golem).filter(|_| enemy) {
                    if e.random.next_int(20) == 0 {
                        e.ai.state.set_target(Some(crate::monster_ai::Target::Mob(other_key.id())));
                    }
                }
            }
            // `other.push(pusher)`: a player's own motion is its client's,
            // and a sleeper pushes back nothing (`LivingEntity.push`).
            if let SectionEntry::Mob(other) = other {
                if self.sleeping(other) {
                    continue;
                }
            }
            let Some((xa, za)) = push_step(position, other_position) else { continue };
            if let SectionEntry::Mob(other) = other {
                if let Some((body, _)) = self.push_body(other) {
                    body.velocity += DVec3::new(-xa, 0.0, -za);
                    body.needs_sync = true;
                }
            }
            if let Some((body, alive)) = self.push_body(key) {
                if alive && !body.climbing {
                    body.velocity += DVec3::new(xa, 0.0, za);
                    body.needs_sync = true;
                }
            }
        }
    }
}

/// `Entity.push(Entity)`'s impulse on the pusher at `pusher`, away from
/// the pushed at `pushed` (the pushed takes the opposite): none when they
/// stand within a hundredth of a block on both axes, at most a twentieth.
pub fn push_step(pusher: DVec3, pushed: DVec3) -> Option<(f64, f64)> {
    let mut xa = pusher.x - pushed.x;
    let mut za = pusher.z - pushed.z;
    let mut distance = xa.abs().max(za.abs());
    if distance < f64::from(0.01_f32) {
        return None;
    }
    distance = distance.sqrt();
    xa /= distance;
    za /= distance;
    let scale = (1.0 / distance).min(1.0);
    xa *= scale;
    za *= scale;
    Some((xa * f64::from(0.05_f32), za * f64::from(0.05_f32)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_keys_decode_as_vanilla_does() {
        for (x, y, z) in [(0, 0, 0), (-1, -4, -1), (5, 19, -7), (-300, 3, 12)] {
            let key = section_key(x, y, z);
            assert_eq!(((key >> 42) as i32, (key << 44 >> 44) as i32, (key << 22 >> 42) as i32), (x, y, z));
        }
        // Within a column, positive z sorts before negative z.
        assert!(section_key(0, 0, 3) < section_key(0, 0, -1));
    }

    #[test]
    fn sections_keep_the_order_entities_came_in() {
        let mut sections = Sections::default();
        let cow = |id| SectionEntry::Mob(EntityKey::Cow(id));
        sections.file(cow(1), DVec3::new(15.5, 1.0, 7.5));
        sections.file(cow(2), DVec3::new(16.5, 1.0, 7.5));
        sections.file(cow(3), DVec3::new(15.2, 1.0, 7.5));
        sections.file(cow(1), DVec3::new(16.1, 1.0, 7.5));
        // Section x = 0 first, then x = 1, which cow 1 joined last.
        let found = sections.around(DVec3::new(15.0, 1.0, 7.0), DVec3::new(16.0, 2.0, 8.0));
        assert!(found == vec![cow(3), cow(2), cow(1)]);
    }

    fn walker(position: DVec3) -> crate::tempt::PlayerCandidate {
        crate::tempt::PlayerCandidate {
            id: 7,
            position,
            eye_height: 1.62,
            main_hand_cow_food: false,
            offhand_cow_food: false,
            main_hand_pig_food: false,
            offhand_pig_food: false,
            main_hand_chicken_food: false,
            offhand_chicken_food: false,
            main_hand_carrot_on_a_stick: false,
            offhand_carrot_on_a_stick: false,
            main_hand_wolf_interest: false,
            offhand_wolf_interest: false,
            main_hand_horse_tempt: false,
            offhand_horse_tempt: false,
            alive: true,
            spectator: false,
            attackable: true,
        }
    }

    #[test]
    fn players_push_the_mobs_they_walk_into() {
        let mut world = EntityWorld::default();
        world.set_players_pickable(true);
        let id = world.spawn_cow(crate::cow::Cow::new(DVec3::new(0.8, 64.0, 0.5)), true);
        let player = walker(DVec3::new(0.5, 64.0, 0.5));
        world.push_from_players(&[player], &|_| true);
        let cow = world.cows().iter().find(|e| e.id == id).unwrap();
        assert!(cow.cow.body.velocity.x > 0.0, "the cow goes away from the player");
        // The client's cow pushes its player the other way.
        let pushes = world.pushes_on_local_player(player.position, player.eye_height);
        assert!(pushes.len() == 1 && pushes[0].x < 0.0);
        // A spectator pushes nothing.
        let mut ghost = player;
        ghost.spectator = true;
        let before = world.cows()[0].cow.body.velocity;
        world.push_from_players(&[ghost], &|_| true);
        assert_eq!(world.cows()[0].cow.body.velocity, before);
    }

    #[test]
    fn a_push_is_a_twentieth_at_most() {
        let (xa, za) = push_step(DVec3::new(0.3, 0.0, 0.0), DVec3::ZERO).unwrap();
        assert!(xa > 0.0 && za == 0.0 && xa <= f64::from(0.05_f32));
        assert!(push_step(DVec3::new(0.005, 0.0, 0.009), DVec3::ZERO).is_none());
    }
}
