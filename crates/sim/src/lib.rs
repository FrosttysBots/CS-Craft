//! Fixed-tick simulation.
//!
//! One funnel for authority, prediction and replay:
//! `World` + per-player [`TickInput`] → [`World::step`] → [`Snapshot`] and
//! [`GameEvent`]s. The step is deterministic for the same inputs (all
//! randomness is seeded from the tick, player and shot), so a client can
//! predict by running the very same code the server runs.
//!
//! Two rule sets ([`GameMode`]): deathmatch (respawn with a fixed loadout)
//! and competitive (rounds, money, buy zones, the bomb; see [`rules`]).
//!
//! The tick rate matches CS:GO's 64-tick matchmaking servers.

pub mod bots;
pub mod objectives;
pub mod rng;
pub mod rules;
pub mod weapon;
pub mod utility;
pub mod tactics;

use collision::{contents, CollisionWorld};
use glam::Vec3;
use movement::{buttons, MoveConfig, MoveEvents, MoveInput, PlayerMove};
pub use objectives::Objectives;
use rng::Rng;
pub use rules::{BuyError, GameMode, Phase, Purchase, RoundEndReason, RoundState, RulesConfig};
use std::sync::Arc;
use weapon::{AccuracyContext, HitGroup, WeaponState, C4};

pub const TICK_RATE: u32 = 64;
pub const TICK_DT: f32 = 1.0 / TICK_RATE as f32;
/// Seconds before a dead player respawns (deathmatch rules).
pub const RESPAWN_DELAY: f32 = 2.0;
/// Knife reach, units.
pub const MELEE_RANGE: f32 = 48.0;
/// The "attacker" of world damage (the bomb).
pub const WORLD: PlayerId = 0;
/// `weapon_slot` value that switches to the previously used weapon.
pub const SLOT_LAST: u8 = 10;
/// How close a defuser must be to the bomb, units.
pub const DEFUSE_RANGE: f32 = 72.0;
/// Pickup radius for items on the ground, units.
pub const PICKUP_RANGE: f32 = 40.0;
/// Bomb blast: damage at the centre and radius (`500 * 3.5`).
pub const BOMB_DAMAGE: f32 = 500.0;
pub const BOMB_RADIUS: f32 = 1750.0;

pub type PlayerId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Team {
    Terrorist,
    CounterTerrorist,
    Spectator,
}

impl Team {
    pub fn enemy_of(self, other: Team) -> bool {
        self != other && self != Team::Spectator && other != Team::Spectator
    }

    pub fn other(self) -> Team {
        match self {
            Team::Terrorist => Team::CounterTerrorist,
            Team::CounterTerrorist => Team::Terrorist,
            Team::Spectator => Team::Spectator,
        }
    }
}

/// Input for one player for one tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TickInput {
    /// Pitch, yaw, roll in degrees.
    pub view_angles: Vec3,
    /// -450..450
    pub forward_move: f32,
    /// -450..450, positive is right.
    pub side_move: f32,
    /// `movement::buttons` bits (JUMP, DUCK, WALK, ATTACK, RELOAD, USE...).
    pub buttons: u32,
    /// Inventory slot to switch to (1 primary, 2 pistol, 3 knife, 5 bomb;
    /// pressing the current slot again cycles within it), [`SLOT_LAST`] for
    /// the previous weapon, 0 for none.
    pub weapon_slot: u8,
    /// Something to buy this tick.
    pub buy: Purchase,
    /// Drop the active weapon.
    pub drop: bool,
    /// Select a carried grenade by item definition; never grants an item.
    pub utility: u32,
}

#[derive(Clone, Debug)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    pub team: Team,
    pub is_bot: bool,
    pub movement: PlayerMove,
    pub view_angles: Vec3,
    pub health: i32,
    pub armor: i32,
    pub helmet: bool,
    pub defuse_kit: bool,
    pub money: i32,
    pub alive: bool,
    pub respawn_at: f32,
    pub flash_until: f32,
    /// Ordered by inventory slot.
    pub weapons: Vec<WeaponState>,
    pub active: usize,
    pub last_active: usize,
    pub kills: u32,
    pub deaths: u32,
    pub shots: u64,
    pub in_buy_zone: bool,
    /// Sim time a bomb plant started, while planting.
    pub plant_start: Option<f32>,
    pub last_input: TickInput,
    pub last_events: MoveEvents,
}

impl Player {
    pub fn active_weapon_mut(&mut self) -> Option<&mut WeaponState> {
        self.weapons.get_mut(self.active)
    }
    pub fn active_weapon(&self) -> Option<&WeaponState> {
        self.weapons.get(self.active)
    }

    pub fn eye(&self) -> Vec3 {
        self.movement.eye_position()
    }

    /// Recoil offset the camera shows (degrees, pitch/yaw).
    pub fn view_punch(&self) -> [f32; 2] {
        let p = self.active_weapon().map_or([0.0, 0.0], WeaponState::aim_punch);
        let k = weapon::RECOIL_SCALE * weapon::VIEW_PUNCH_FRACTION;
        [p[0] * k, p[1] * k]
    }

    /// The hull the player occupies (mins, maxs) in world space.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let (mins, maxs) = self.movement.hull();
        (self.movement.origin + mins, self.movement.origin + maxs)
    }

    pub fn has(&self, class_name: &str) -> bool {
        self.weapons.iter().any(|w| w.def.class_name == class_name)
    }

    pub fn has_bomb(&self) -> bool {
        self.has(C4)
    }

    /// Index of the first weapon in an inventory slot.
    pub fn weapon_in_slot(&self, slot: u8) -> Option<usize> {
        self.weapons.iter().position(|w| weapon::slot(&w.def) == slot)
    }

    /// The weapon to hold by default: primary, else pistol, else knife.
    pub fn best_weapon(&self) -> usize {
        (1..=5).find_map(|s| self.weapon_in_slot(s)).unwrap_or(0)
    }

    /// Adds a weapon in slot order, keeping the active selection.
    fn give(&mut self, w: WeaponState) -> usize {
        let slot = weapon::slot(&w.def);
        let at = self.weapons.iter().position(|x| weapon::slot(&x.def) > slot).unwrap_or(self.weapons.len());
        self.weapons.insert(at, w);
        if self.weapons.len() > 1 {
            if self.active >= at {
                self.active += 1;
            }
            if self.last_active >= at {
                self.last_active += 1;
            }
        }
        at
    }

    /// Removes a weapon; if it was held, switches to the best remaining.
    fn take(&mut self, index: usize, time: f32) -> WeaponState {
        let mut w = self.weapons.remove(index);
        // A dropped gun cannot finish its owner's reload on the ground or
        // in another player's hands.
        w.reload_end = None;
        w.reset_zoom(time);
        let was_active = self.active == index;
        if self.active > index {
            self.active -= 1;
        }
        if self.last_active > index {
            self.last_active -= 1;
        }
        self.last_active = self.last_active.min(self.weapons.len().saturating_sub(1));
        if was_active {
            self.active = 0;
            let best = self.best_weapon();
            self.select(best, time);
        }
        self.active = self.active.min(self.weapons.len().saturating_sub(1));
        w
    }

    fn select(&mut self, index: usize, time: f32) {
        if index >= self.weapons.len() {
            return;
        }
        if let Some(old) = self.weapons.get_mut(self.active) {
            old.reset_zoom(time);
            old.reload_end = None;
        }
        if index != self.active {
            self.last_active = self.active;
        }
        self.active = index;
        self.plant_start = None;
        let w = &mut self.weapons[index];
        w.reload_end = None;
        w.reset_zoom(time);
        w.next_attack = w.next_attack.max(time + weapon::deploy_time(&w.def));
    }
}

/// A spawn point from the map.
#[derive(Clone, Copy, Debug)]
pub struct SpawnPoint {
    pub origin: Vec3,
    pub yaw: f32,
    pub team: Team,
}

#[derive(Clone, Debug)]
pub enum GroundKind {
    Weapon(WeaponState),
    DefuseKit,
}

/// Something lying on the ground that players pick up by walking over it.
#[derive(Clone, Debug)]
pub struct GroundItem {
    pub id: u32,
    pub kind: GroundKind,
    pub pos: Vec3,
    pub yaw: f32,
    pub dropped_by: PlayerId,
    /// The dropper cannot pick it back up before this time.
    pub pickup_after: f32,
}

impl GroundItem {
    pub fn class_name(&self) -> &str {
        match &self.kind {
            GroundKind::Weapon(w) => &w.def.class_name,
            GroundKind::DefuseKit => "item_defuser",
        }
    }
}

/// The ticking bomb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlantedBomb {
    pub pos: Vec3,
    /// Index into [`Objectives::bomb_sites`].
    pub site: usize,
    pub planter: PlayerId,
    pub planted_at: f32,
    pub explode_at: f32,
    /// The CT defusing it and when the defuse completes.
    pub defuser: Option<(PlayerId, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeleeHit { Miss, World, Player }

/// Things that happened during a tick, for effects, sounds and the HUD.
#[derive(Clone, Debug, PartialEq)]
pub enum GameEvent {
    StatTrakKill { owner:PlayerId, serial:u64 },
    DecoyShot { player: PlayerId, pos: Vec3 },
    UtilityThrown { player: PlayerId, kind: utility::Kind, pos: Vec3 },
    UtilityDetonated { player: PlayerId, kind: utility::Kind, pos: Vec3 },
    Melee { player: PlayerId, weapon: String, origin: Vec3, secondary: bool, hit: MeleeHit },
    Scope { player: PlayerId, weapon: String, origin: Vec3, level: u8 },
    Fire { player: PlayerId, weapon: String, origin: Vec3 },
    Tracer { player: PlayerId, start: Vec3, end: Vec3 },
    Impact { pos: Vec3, normal: Vec3, surface: u32 },
    Damage { attacker: PlayerId, victim: PlayerId, health: i32, group: HitGroup, pos: Vec3 },
    /// `attacker` is [`WORLD`] for the bomb.
    Kill { attacker: PlayerId, victim: PlayerId, weapon: String, headshot: bool },
    Reload { player: PlayerId },
    Jump { player: PlayerId },
    Land { player: PlayerId, speed: f32 },
    RoundStart { round: u32 },
    FreezeEnd,
    RoundEnd { reason: RoundEndReason },
    Halftime,
    MatchEnd { winner: Option<Team> },
    Purchase { player: PlayerId, item: Purchase },
    Pickup { player: PlayerId, item: String },
    Dropped { player: PlayerId, item: String, pos: Vec3 },
    PlantStart { player: PlayerId },
    BombPlanted { player: PlayerId, site: char, pos: Vec3 },
    DefuseStart { player: PlayerId, kit: bool },
    BombDefused { player: PlayerId },
    BombExploded { pos: Vec3 },
}

pub struct World {
    pub collision: Arc<CollisionWorld>,
    pub catalog: Arc<weapons::Catalog>,
    pub config: MoveConfig,
    pub mode: GameMode,
    pub rules: RulesConfig,
    pub round: RoundState,
    pub objectives: Objectives,
    pub players: Vec<Player>,
    pub spawns: Vec<SpawnPoint>,
    pub items: Vec<GroundItem>,
    pub projectiles: Vec<utility::Projectile>,
    pub utility_fields: Vec<utility::Field>,
    pub bomb: Option<PlantedBomb>,
    pub tick: u64,
    /// Events from the most recent step.
    pub events: Vec<GameEvent>,
    next_id: PlayerId,
    next_item: u32,
}

/// Everything a renderer or a remote client needs about one tick.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub tick: u64,
    pub players: Vec<PlayerSnapshot>,
}

#[derive(Clone, Copy, Debug)]
pub struct PlayerSnapshot {
    pub id: PlayerId,
    pub origin: Vec3,
    pub velocity: Vec3,
    pub view_angles: Vec3,
    pub view_offset: f32,
    pub on_ground: bool,
    pub ducked: bool,
    pub alive: bool,
    pub health: i32,
}

/// Slab test: entry distance of a ray into an axis-aligned box.
fn ray_box(origin: Vec3, dir: Vec3, mins: Vec3, maxs: Vec3) -> Option<f32> {
    let mut t0 = 0.0f32;
    let mut t1 = f32::MAX;
    for a in 0..3 {
        if dir[a].abs() < 1e-8 {
            if origin[a] < mins[a] || origin[a] > maxs[a] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / dir[a];
        let (mut near, mut far) = ((mins[a] - origin[a]) * inv, (maxs[a] - origin[a]) * inv);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

impl World {
    pub fn new(collision: Arc<CollisionWorld>) -> Self {
        Self::with_catalog(collision, Arc::new(weapons::Catalog::default()))
    }

    pub fn with_catalog(collision: Arc<CollisionWorld>, catalog: Arc<weapons::Catalog>) -> Self {
        let rules = RulesConfig::default();
        Self {
            collision,
            catalog,
            config: MoveConfig::default(),
            mode: GameMode::Deathmatch,
            round: RoundState::new(&rules, 0.0),
            rules,
            objectives: Objectives::default(),
            players: Vec::new(),
            spawns: Vec::new(),
            items: Vec::new(),
            projectiles: Vec::new(),
            utility_fields: Vec::new(),
            bomb: None,
            tick: 0,
            events: Vec::new(),
            next_id: 1,
            next_item: 1,
        }
    }

    pub fn time(&self) -> f32 {
        self.tick as f32 * TICK_DT
    }

    pub fn competitive(&self) -> bool {
        self.mode == GameMode::Competitive
    }

    /// Reads `info_player_terrorist` / `info_player_counterterrorist`.
    pub fn load_spawns(&mut self, map: &bsp::Bsp) {
        self.spawns.clear();
        for (class, team) in [
            ("info_player_terrorist", Team::Terrorist),
            ("info_player_counterterrorist", Team::CounterTerrorist),
        ] {
            for ent in map.entities_of_class(class) {
                let Some(origin) = ent.str("origin").and_then(bsp::parse_origin) else {
                    continue;
                };
                let yaw = ent.str("angles").map(kv_angles).map(|a| a.y).unwrap_or(0.0);
                self.spawns.push(SpawnPoint { origin, yaw, team });
            }
        }
    }

    /// Spawns plus buy zones and bomb sites. `overview` is the map's radar
    /// overview file text, used to name the sites.
    pub fn load_map(&mut self, map: &bsp::Bsp, overview: Option<&str>) {
        self.load_spawns(map);
        self.objectives = Objectives::from_bsp(map, &self.spawns, overview);
    }

    /// Switches rule sets. Competitive starts a fresh match: call it after
    /// adding the players so the bomb goes to one of the Terrorists.
    pub fn set_mode(&mut self, mode: GameMode, rules: RulesConfig) {
        self.mode = mode;
        self.rules = rules;
        if mode == GameMode::Special { self.players.retain(|p|!p.is_bot); for p in &mut self.players {p.money=0;} }
        match mode {
            GameMode::Competitive => self.start_match(),
            GameMode::Deathmatch | GameMode::Special => {
                self.items.clear();
        self.projectiles.clear();
        self.utility_fields.clear();
        for p in &mut self.players { p.flash_until=0.0; }
                self.bomb = None;
                let ids: Vec<PlayerId> = self.players.iter().map(|p| p.id).collect();
                for id in ids {
                    self.respawn(id);
                }
            }
        }
    }

    /// Drops a point onto the floor below it (as the game does for spawns).
    pub fn drop_to_floor(&self, origin: Vec3, ducked: bool) -> Vec3 {
        let (mins, maxs) = if ducked {
            (movement::DUCK_MINS, movement::DUCK_MAXS)
        } else {
            (movement::STAND_MINS, movement::STAND_MAXS)
        };
        let start = origin + Vec3::new(0.0, 0.0, 2.0);
        let tr = self
            .collision
            .trace_box(start, origin - Vec3::new(0.0, 0.0, 256.0), mins, maxs, contents::MASK_PLAYERSOLID);
        if tr.start_solid || tr.fraction == 1.0 { origin } else { tr.end }
    }

    pub fn add_player(&mut self, name: &str, team: Team) -> PlayerId {
        self.add_player_ex(name, team, false)
    }

    pub fn add_bot(&mut self, name: &str, team: Team) -> PlayerId {
        self.add_player_ex(name, team, true)
    }

    fn add_player_ex(&mut self, name: &str, team: Team, is_bot: bool) -> PlayerId {
        let id = self.next_id;
        self.next_id += 1;
        self.players.push(Player {
            id,
            name: name.to_string(),
            team,
            is_bot,
            movement: PlayerMove::default(),
            view_angles: Vec3::ZERO,
            health: 100,
            armor: 0,
            helmet: false,
            defuse_kit: false,
            money: if self.mode == GameMode::Special { 0 } else { self.rules.start_money },
            alive: false,
            respawn_at: 0.0,
            flash_until: 0.0,
            weapons: Vec::new(),
            active: 0,
            last_active: 0,
            kills: 0,
            deaths: 0,
            shots: 0,
            in_buy_zone: false,
            plant_start: None,
            last_input: TickInput::default(),
            last_events: MoveEvents::default(),
        });
        match self.mode {
            GameMode::Deathmatch | GameMode::Special => self.respawn(id),
            // Joining mid-round waits for the next one.
            GameMode::Competitive if self.round.phase == Phase::Freeze => {
                self.spawn_for_round(id, false);
                let bomb_out = self.bomb.is_some() || self.players.iter().any(Player::has_bomb) || self.items.iter().any(|i| i.class_name() == C4);
                if team == Team::Terrorist && !bomb_out {
                    self.give_weapon(id, C4);
                }
            }
            GameMode::Competitive => {}
        }
        id
    }

    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    pub fn player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }

    fn index_of(&self, id: PlayerId) -> Option<usize> {
        self.players.iter().position(|p| p.id == id)
    }

    fn weapon(&self, class_name: &str) -> Option<WeaponState> {
        self.catalog.get(class_name).map(|d| WeaponState::new(Arc::new(d.clone())))
    }

    fn give_weapon(&mut self, id: PlayerId, class_name: &str) {
        if let Some(w) = self.weapon(class_name) {
            if let Some(p) = self.player_mut(id) {
                p.give(w);
            }
        }
    }

    /// Deathmatch loadout: a rifle, a pistol and a knife.
    fn loadout(&self, team: Team) -> Vec<WeaponState> {
        let names: &[&str] = match team {
            Team::Terrorist => &["weapon_ak47", "weapon_glock", "weapon_knife_t"],
            _ => &["weapon_m4a1_silencer", "weapon_usp_silencer", "weapon_knife"],
        };
        names.iter().filter_map(|n| self.weapon(n)).collect()
    }

    /// Competitive starting inventory: knife and the default pistol.
    fn default_inventory(&self, team: Team) -> Vec<WeaponState> {
        let (pistol, knife) = match team {
            Team::Terrorist => (&self.rules.t_default_secondary, &self.rules.t_default_melee),
            _ => (&self.rules.ct_default_secondary, &self.rules.ct_default_melee),
        };
        [pistol, knife].iter().filter_map(|n| self.weapon(n)).collect()
    }

    /// A free spawn point for a team (not within 40 units of a live player).
    fn pick_spawn(&self, id: PlayerId, team: Team) -> SpawnPoint {
        let taken: Vec<Vec3> = self
            .players
            .iter()
            .filter(|p| p.id != id && p.alive)
            .map(|p| p.movement.origin)
            .collect();
        let offset = (self.tick as usize + id as usize * 7 + self.round.round as usize * 3) % self.spawns.len().max(1);
        let team_spawns: Vec<SpawnPoint> = self.spawns.iter().copied().filter(|s| s.team == team).collect();
        (0..team_spawns.len())
            .map(|i| team_spawns[(i + offset) % team_spawns.len()])
            .find(|s| taken.iter().all(|t| t.distance(s.origin) > 40.0))
            .or_else(|| team_spawns.first().copied())
            .or_else(|| self.spawns.first().copied())
            .unwrap_or(SpawnPoint { origin: Vec3::ZERO, yaw: 0.0, team })
    }

    fn place(&mut self, id: PlayerId) {
        let Some(team) = self.player(id).map(|p| p.team) else { return };
        let spawn = self.pick_spawn(id, team);
        let origin = self.drop_to_floor(spawn.origin, false);
        if let Some(p) = self.player_mut(id) {
            p.movement = PlayerMove::spawn_at(origin);
            p.view_angles = Vec3::new(0.0, spawn.yaw, 0.0);
            p.health = 100;
            p.flash_until = 0.0;
            p.alive = true;
            p.plant_start = None;
        }
    }

    /// Deathmatch respawn: a free spawn point, fully equipped.
    pub fn respawn(&mut self, id: PlayerId) {
        let Some(team) = self.player(id).map(|p| p.team) else { return };
        if team == Team::Spectator {
            return;
        }
        self.place(id);
        let special = self.mode == GameMode::Special;
        let loadout = if special {
            self.default_inventory(team)
        } else { self.loadout(team) };
        if let Some(p) = self.player_mut(id) {
            p.armor = if special { 0 } else { 100 };
            p.helmet = !special;
            p.defuse_kit = false;
            p.weapons = loadout;
            p.active = 0;
            p.last_active = 0;
        }
    }

    /// Round start placement. Survivors (`keep`) keep their equipment.
    fn spawn_for_round(&mut self, id: PlayerId, keep: bool) {
        let Some(team) = self.player(id).map(|p| p.team) else { return };
        if team == Team::Spectator {
            return;
        }
        self.place(id);
        let fresh = self.default_inventory(team);
        let now = self.time();
        if let Some(p) = self.player_mut(id) {
            if !keep {
                p.weapons = fresh;
                p.armor = 0;
                p.helmet = false;
                p.defuse_kit = false;
            }
            for w in &mut p.weapons {
                w.reset_zoom(now);
                w.reload_end = None;
                w.recoil_index = 0.0;
                w.accuracy_penalty = 0.0;
            }
            p.active = p.best_weapon();
            p.last_active = p.weapon_in_slot(3).unwrap_or(p.active);
        }
    }

    fn start_match(&mut self) {
        self.round = RoundState::new(&self.rules, self.time());
        for p in &mut self.players {
            p.money = self.rules.start_money;
            p.kills = 0;
            p.deaths = 0;
        }
        self.start_round(false);
    }

    fn start_round(&mut self, keep_survivors: bool) {
        let now = self.time();
        self.items.clear();
        self.projectiles.clear();
        self.utility_fields.clear();
        for p in &mut self.players { p.flash_until=0.0; }
        self.bomb = None;
        self.round.phase = Phase::Freeze;
        self.round.phase_start = now;
        self.round.live_start = now + self.rules.freeze_time;
        self.round.bomb_planted = false;
        self.round.last_result = None;
        let survivors: Vec<PlayerId> = if keep_survivors {
            self.players.iter().filter(|p| p.alive).map(|p| p.id).collect()
        } else {
            Vec::new()
        };
        for p in &mut self.players {
            p.alive = false;
            // A bomb carried over from last round is handed out afresh.
            if let Some(i) = p.weapons.iter().position(|w| w.def.class_name == C4) {
                p.weapons.remove(i);
                p.active = 0;
                p.last_active = 0;
            }
        }
        let ids: Vec<PlayerId> = self.players.iter().map(|p| p.id).collect();
        for id in ids {
            self.spawn_for_round(id, survivors.contains(&id));
        }
        // The bomb goes to a random Terrorist.
        let ts: Vec<PlayerId> = self.players.iter().filter(|p| p.team == Team::Terrorist && p.alive).map(|p| p.id).collect();
        if !ts.is_empty() {
            let mut rng = Rng::from_parts(&[self.round.round as u64, self.tick, 0xC4]);
            let carrier = ts[(rng.next_u64() % ts.len() as u64) as usize];
            self.give_weapon(carrier, C4);
        }
        self.events.push(GameEvent::RoundStart { round: self.round.round });
    }

    fn next_round(&mut self) {
        let halftime = self.rules.halftime && self.round.round == self.rules.max_rounds / 2;
        self.round.round += 1;
        if halftime {
            for p in &mut self.players {
                p.team = p.team.other();
                p.money = self.rules.start_money;
            }
            let r = &mut self.round;
            std::mem::swap(&mut r.score_t, &mut r.score_ct);
            r.losses_t = self.rules.starting_losses;
            r.losses_ct = self.rules.starting_losses;
            self.events.push(GameEvent::Halftime);
        }
        self.start_round(!halftime);
    }

    fn end_round(&mut self, reason: RoundEndReason) {
        let now = self.time();
        self.round.phase = Phase::Over;
        self.round.phase_start = now;
        self.round.last_result = Some(reason);
        let winner = reason.winner();
        match winner {
            Some(Team::Terrorist) => {
                self.round.score_t += 1;
                self.round.losses_t = self.round.losses_t.saturating_sub(1);
                self.round.losses_ct = (self.round.losses_ct + 1).min(5);
            }
            Some(Team::CounterTerrorist) => {
                self.round.score_ct += 1;
                self.round.losses_ct = self.round.losses_ct.saturating_sub(1);
                self.round.losses_t = (self.round.losses_t + 1).min(5);
            }
            _ => {}
        }
        let win_cash = match reason {
            RoundEndReason::TargetBombed => self.rules.win_bomb,
            RoundEndReason::BombDefused => self.rules.win_defuse,
            RoundEndReason::TargetSaved => self.rules.win_time,
            RoundEndReason::TerroristsEliminated | RoundEndReason::CounterTerroristsEliminated => self.rules.win_elimination,
            RoundEndReason::Draw => 0,
        };
        let planted = self.round.bomb_planted;
        let (losses_t, losses_ct) = (self.round.losses_t, self.round.losses_ct);
        for p in &mut self.players {
            if p.team == Team::Spectator {
                continue;
            }
            let cash = if Some(p.team) == winner {
                win_cash
            } else if winner.is_none() {
                0
            } else if reason == RoundEndReason::TargetSaved && p.team == Team::Terrorist && p.alive {
                // Terrorists who hide out the clock get nothing.
                0
            } else {
                let losses = if p.team == Team::Terrorist { losses_t } else { losses_ct };
                let mut c = self.rules.loss_bonus(losses);
                if p.team == Team::Terrorist && planted {
                    c += self.rules.planted_but_defused;
                }
                c
            };
            p.money = (p.money + cash).min(self.rules.max_money);
        }
        self.events.push(GameEvent::RoundEnd { reason });

        let to_win = self.rules.rounds_to_win();
        let (t, ct) = (self.round.score_t, self.round.score_ct);
        let clinched = self.rules.can_clinch && (t >= to_win || ct >= to_win);
        if clinched || self.round.round >= self.rules.max_rounds {
            let winner = match t.cmp(&ct) {
                std::cmp::Ordering::Greater => Some(Team::Terrorist),
                std::cmp::Ordering::Less => Some(Team::CounterTerrorist),
                std::cmp::Ordering::Equal => None,
            };
            self.round.phase = Phase::MatchOver;
            self.round.match_winner = winner;
            self.events.push(GameEvent::MatchEnd { winner });
        }
    }

    /// Timers that run before players move: freeze time, round clock,
    /// bomb, next round.
    fn update_round(&mut self, time: f32) {
        if let Some(b) = self.bomb {
            if time >= b.explode_at {
                self.explode(b.pos);
                if self.round.phase == Phase::Live {
                    self.end_round(RoundEndReason::TargetBombed);
                }
            } else if let Some((defuser, _)) = b.defuser {
                if !self.player(defuser).is_some_and(|p| p.alive) {
                    if let Some(b) = &mut self.bomb {
                        b.defuser = None;
                    }
                }
            }
        }
        let r = &self.round;
        match r.phase {
            Phase::Freeze if time >= r.phase_start + self.rules.freeze_time => {
                self.round.phase = Phase::Live;
                self.round.live_start = time;
                self.events.push(GameEvent::FreezeEnd);
            }
            Phase::Live if self.bomb.is_none() && time >= r.live_start + self.rules.round_time => {
                self.end_round(RoundEndReason::TargetSaved);
            }
            Phase::Over if time >= r.phase_start + self.rules.restart_delay => self.next_round(),
            Phase::MatchOver if time >= r.phase_start + self.rules.restart_delay * 2.0 => self.start_match(),
            _ => {}
        }
    }

    fn check_elimination(&mut self) {
        if self.round.phase != Phase::Live {
            return;
        }
        let count = |team: Team, alive_only: bool| {
            self.players.iter().filter(|p| p.team == team && (p.alive || !alive_only)).count()
        };
        if count(Team::CounterTerrorist, false) > 0 && count(Team::CounterTerrorist, true) == 0 {
            self.end_round(RoundEndReason::CounterTerroristsEliminated);
        } else if count(Team::Terrorist, false) > 0 && count(Team::Terrorist, true) == 0 && self.bomb.is_none() {
            self.end_round(RoundEndReason::TerroristsEliminated);
        }
    }

    fn explode(&mut self, pos: Vec3) {
        self.bomb = None;
        self.events.push(GameEvent::BombExploded { pos });
        let sigma = BOMB_RADIUS / 3.0;
        for i in 0..self.players.len() {
            let p = &self.players[i];
            if !p.alive {
                continue;
            }
            let d = (p.movement.origin + Vec3::Z * 36.0).distance(pos);
            let dmg = BOMB_DAMAGE * (-(d * d) / (2.0 * sigma * sigma)).exp();
            if dmg < 1.0 {
                continue;
            }
            let (health_dmg, armor_dmg) = if p.armor > 0 {
                let absorbed = (dmg * 0.5).min(p.armor as f32 * 2.0);
                (dmg - absorbed, absorbed * 0.5)
            } else {
                (dmg, 0.0)
            };
            let health_dmg = health_dmg.round().max(1.0) as i32;
            let p = &mut self.players[i];
            p.health -= health_dmg;
            p.armor = (p.armor - armor_dmg as i32).max(0);
            let victim = p.id;
            let at = p.movement.origin;
            self.events.push(GameEvent::Damage {
                attacker: WORLD,
                victim,
                health: health_dmg,
                group: HitGroup::Chest,
                pos: at,
            });
            if self.players[i].health <= 0 {
                self.kill(i, None, "planted_c4", false);
            }
        }
    }

    fn new_item(&mut self, kind: GroundKind, pos: Vec3, yaw: f32, dropped_by: PlayerId, pickup_after: f32) {
        let id = self.next_item;
        self.next_item += 1;
        self.items.push(GroundItem { id, kind, pos, yaw, dropped_by, pickup_after });
    }

    /// Where an item thrown by a player lands: a little ahead, on the floor.
    fn throw_point(&self, p: &Player, distance: f32) -> Vec3 {
        let (fwd, _, _) = movement::angle_vectors(Vec3::new(0.0, p.view_angles.y, 0.0));
        let start = p.movement.origin + Vec3::Z * 36.0;
        let tr = self.collision.trace_ray(start, start + fwd * distance, contents::MASK_PLAYERSOLID);
        let ahead = start + (tr.end - start) * 0.8;
        let down = self.collision.trace_ray(ahead, ahead - Vec3::Z * 512.0, contents::MASK_PLAYERSOLID);
        down.end + Vec3::Z * 1.0
    }

    /// Drops one inventory item on the ground.
    fn drop_weapon(&mut self, i: usize, index: usize, distance: f32) {
        let time = self.time();
        let pos = self.throw_point(&self.players[i], distance);
        let p = &mut self.players[i];
        let id = p.id;
        let yaw = p.view_angles.y;
        let w = p.take(index, time);
        let name = w.def.class_name.clone();
        self.new_item(GroundKind::Weapon(w), pos, yaw, id, time + 1.0);
        self.events.push(GameEvent::Dropped { player: id, item: name, pos });
    }

    /// What a dying player leaves behind: best gun, the bomb, a kit.
    fn death_drops(&mut self, i: usize) {
        if !self.competitive() {
            return;
        }
        let gun = self.players[i].weapon_in_slot(1).or_else(|| self.players[i].weapon_in_slot(2));
        let bomb = self.players[i].weapons.iter().position(|w| w.def.class_name == C4);
        let mut drops: Vec<usize> = gun.into_iter().chain(bomb).collect();
        drops.sort_unstable_by(|a, b| b.cmp(a));
        for (n, index) in drops.into_iter().enumerate() {
            self.drop_weapon(i, index, 8.0 + n as f32 * 20.0);
        }
        if std::mem::take(&mut self.players[i].defuse_kit) {
            let p = &self.players[i];
            let pos = self.throw_point(p, 24.0);
            let (id, yaw, time) = (p.id, p.view_angles.y, self.time());
            self.new_item(GroundKind::DefuseKit, pos, yaw, id, time);
        }
    }

    fn pickups(&mut self, i: usize, time: f32) {
        let p = &self.players[i];
        let origin = p.movement.origin;
        let mut taken = None;
        for (n, item) in self.items.iter().enumerate() {
            let d = item.pos - origin;
            if d.truncate().length() > PICKUP_RANGE || d.z < -24.0 || d.z > 80.0 {
                continue;
            }
            if item.dropped_by == p.id && time < item.pickup_after {
                continue;
            }
            let ok = match &item.kind {
                GroundKind::DefuseKit => p.team == Team::CounterTerrorist && !p.defuse_kit,
                GroundKind::Weapon(w) if w.def.class_name == C4 => p.team == Team::Terrorist,
                GroundKind::Weapon(w) => p.weapon_in_slot(weapon::slot(&w.def)).is_none() && !p.has(&w.def.class_name),
            };
            if ok {
                // Nearby items on the other side of a wall are not reachable.
                let trace = self.collision.trace_ray(p.eye(), item.pos + Vec3::Z * 4.0, contents::MASK_PLAYERSOLID);
                if trace.hit() {
                    continue;
                }
                taken = Some(n);
                break;
            }
        }
        let Some(n) = taken else { return };
        let item = self.items.remove(n);
        let name = item.class_name().to_string();
        let p = &mut self.players[i];
        match item.kind {
            GroundKind::DefuseKit => p.defuse_kit = true,
            GroundKind::Weapon(w) => {
                let had_gun = p.weapon_in_slot(1).is_some() || p.weapon_in_slot(2).is_some();
                let at = p.give(w);
                if !had_gun && weapon::slot(&p.weapons[at].def) <= 2 {
                    p.select(at, time);
                }
            }
        }
        let id = p.id;
        self.events.push(GameEvent::Pickup { player: id, item: name });
    }

    /// What a purchase would cost, or why it is not allowed right now.
    pub fn buy_cost(&self, id: PlayerId, item: Purchase) -> Result<i32, BuyError> {
        let time = self.time();
        let p = self.player(id).ok_or(BuyError::Unknown)?;
        if !p.alive || (self.mode != GameMode::Special && (!self.competitive() || !self.round.can_buy(&self.rules, time))) {
            return Err(BuyError::NotAllowedNow);
        }
        if self.mode != GameMode::Special && !p.in_buy_zone {
            return Err(BuyError::NotInBuyZone);
        }
        let cost = match item {
            Purchase::None => return Err(BuyError::Unknown),
            Purchase::Weapon(index) => {
                let def = self.catalog.weapons.iter().find(|w| w.def_index == index).ok_or(BuyError::Unknown)?;
                if def.class == weapons::WeaponClass::Knife || def.class_name == C4 || def.price <= 0 {
                    return Err(BuyError::Unknown);
                }
                let side_ok = match def.side {
                    weapons::Side::Both => true,
                    weapons::Side::Terrorist => p.team == Team::Terrorist,
                    weapons::Side::CounterTerrorist => p.team == Team::CounterTerrorist,
                };
                if !side_ok {
                    return Err(BuyError::WrongTeam);
                }
                if if def.class == weapons::WeaponClass::Grenade {
                    let count=p.weapons.iter().filter(|w| w.def.class_name==def.class_name).count();
                    let total=p.weapons.iter().filter(|w| w.def.class==weapons::WeaponClass::Grenade).count();
                    total>=4 || count>=if def.class_name=="weapon_flashbang" {2} else {1}
                        || (utility::Kind::from_name(&def.class_name).is_some_and(|k| k.fire()) && p.weapons.iter().any(|w| utility::Kind::from_name(&w.def.class_name).is_some_and(|k| k.fire())))
                } else { p.has(&def.class_name) } {
                    return Err(BuyError::AlreadyOwned);
                }
                def.price
            }
            Purchase::Kevlar => {
                if p.armor >= 100 {
                    return Err(BuyError::AlreadyOwned);
                }
                rules::KEVLAR_PRICE
            }
            Purchase::KevlarHelmet => {
                let c = if p.armor < 100 { rules::KEVLAR_PRICE } else { 0 } + if p.helmet { 0 } else { rules::HELMET_PRICE };
                if c == 0 {
                    return Err(BuyError::AlreadyOwned);
                }
                c
            }
            Purchase::DefuseKit => {
                if p.team != Team::CounterTerrorist {
                    return Err(BuyError::WrongTeam);
                }
                if p.defuse_kit {
                    return Err(BuyError::AlreadyOwned);
                }
                rules::DEFUSE_KIT_PRICE
            }
        };
        if cost > p.money {
            return Err(BuyError::NotEnoughMoney);
        }
        Ok(cost)
    }

    /// Applies a purchase if the rules allow it.
    pub fn buy(&mut self, id: PlayerId, item: Purchase) -> Result<(), BuyError> {
        let time = self.time();
        let cost = self.buy_cost(id, item)?;
        let i = self.index_of(id).ok_or(BuyError::Unknown)?;
        self.players[i].money -= cost;
        match item {
            Purchase::Weapon(index) => {
                let def = self.catalog.weapons.iter().find(|w| w.def_index == index).cloned().ok_or(BuyError::Unknown)?;
                let slot = weapon::slot(&def);
                if slot <= 2 {
                    if let Some(old) = self.players[i].weapon_in_slot(slot) {
                        self.drop_weapon(i, old, 16.0);
                    }
                }
                let p = &mut self.players[i];
                let at = p.give(WeaponState::new(Arc::new(def)));
                if slot == 1 || (slot == 2 && p.weapon_in_slot(1).is_none()) {
                    p.select(at, time);
                }
            }
            Purchase::Kevlar => self.players[i].armor = 100,
            Purchase::KevlarHelmet => {
                self.players[i].armor = 100;
                self.players[i].helmet = true;
            }
            Purchase::DefuseKit => self.players[i].defuse_kit = true,
            Purchase::None => {}
        }
        self.events.push(GameEvent::Purchase { player: id, item });
        Ok(())
    }

    /// Bomb planting. Returns true while the player is planting (movement
    /// is locked).
    fn update_plant(&mut self, i: usize, input: &TickInput, time: f32) -> bool {
        let p = &self.players[i];
        let holding = p.active_weapon().is_some_and(|w| w.def.class_name == C4);
        let attack = input.buttons & buttons::ATTACK != 0;
        let (mn, mx) = p.bounds();
        let site = self.objectives.site_at(mn, mx);
        let able = self.competitive() && self.round.phase == Phase::Live && holding && attack && p.movement.on_ground;
        let (Some(site), true) = (site, able) else {
            self.players[i].plant_start = None;
            return false;
        };
        let id = p.id;
        match p.plant_start {
            None => {
                self.players[i].plant_start = Some(time);
                self.events.push(GameEvent::PlantStart { player: id });
            }
            Some(start) if time - start >= self.rules.plant_time => {
                let p = &mut self.players[i];
                p.plant_start = None;
                let index = p.active;
                p.take(index, time);
                p.money = (p.money + self.rules.player_planted).min(self.rules.max_money);
                let pos = p.movement.origin + Vec3::Z * 1.0;
                self.bomb = Some(PlantedBomb {
                    pos,
                    site,
                    planter: id,
                    planted_at: time,
                    explode_at: time + self.rules.c4_timer,
                    defuser: None,
                });
                self.round.bomb_planted = true;
                let name = self.objectives.bomb_sites[site].name;
                self.events.push(GameEvent::BombPlanted { player: id, site: name, pos });
                return false;
            }
            Some(_) => {}
        }
        true
    }

    /// Bomb defusing. Returns true while the player is defusing.
    fn update_defuse(&mut self, i: usize, input: &TickInput, time: f32) -> bool {
        let Some(bomb) = self.bomb else { return false };
        let p = &self.players[i];
        if p.team != Team::CounterTerrorist || self.round.phase != Phase::Live {
            return false;
        }
        let id = p.id;
        let to = bomb.pos - p.eye();
        let near = to.length() <= DEFUSE_RANGE + 40.0 && (bomb.pos - p.movement.origin).truncate().length() <= DEFUSE_RANGE;
        let holding = input.buttons & buttons::USE != 0
            && near
            && p.movement.on_ground
            && !self.collision.trace_ray(p.eye(), bomb.pos + Vec3::Z * 4.0, contents::MASK_PLAYERSOLID).hit();
        match bomb.defuser {
            Some((d, done)) if d == id => {
                if !holding {
                    if let Some(b) = &mut self.bomb {
                        b.defuser = None;
                    }
                    return false;
                }
                if time >= done {
                    self.bomb = None;
                    let p = &mut self.players[i];
                    p.money = (p.money + self.rules.player_defused).min(self.rules.max_money);
                    self.events.push(GameEvent::BombDefused { player: id });
                    self.end_round(RoundEndReason::BombDefused);
                    return false;
                }
                true
            }
            None if holding => {
                let (fwd, _, _) = movement::angle_vectors(input.view_angles);
                if fwd.dot(to.normalize_or_zero()) < 0.5 {
                    return false;
                }
                let kit = p.defuse_kit;
                let done = time + self.rules.defuse_duration(kit);
                if let Some(b) = &mut self.bomb {
                    b.defuser = Some((id, done));
                }
                self.events.push(GameEvent::DefuseStart { player: id, kit });
                true
            }
            _ => false,
        }
    }

    /// Advances the world by one tick. Players without an input this tick
    /// repeat their last one (as a server does for a late packet), minus
    /// one-shot buttons and commands.
    pub fn step(&mut self, inputs: &[(PlayerId, TickInput)]) {
        self.events.clear();
        let time = self.time();
        let collision = self.collision.clone();

        match self.mode {
            GameMode::Deathmatch | GameMode::Special => {
                let due: Vec<PlayerId> = self
                    .players
                    .iter()
                    .filter(|p| !p.alive && p.team != Team::Spectator && time >= p.respawn_at)
                    .map(|p| p.id)
                    .collect();
                for id in due {
                    self.respawn(id);
                }
            }
            GameMode::Competitive => self.update_round(time),
        }
        let frozen = self.competitive() && self.round.phase == Phase::Freeze;

        for i in 0..self.players.len() {
            if !self.players[i].alive {
                continue;
            }
            let id = self.players[i].id;
            let mut input = inputs.iter().find(|(pid, _)| *pid == id).map(|(_, inp)| *inp).unwrap_or(TickInput {
                buttons: self.players[i].last_input.buttons & !(buttons::JUMP | buttons::RELOAD),
                weapon_slot: 0,
                buy: Purchase::None,
                drop: false,
                utility: 0,
                ..self.players[i].last_input
            });

            // Buying and dropping.
            {
                let (mn, mx) = self.players[i].bounds();
                let team = self.players[i].team;
                self.players[i].in_buy_zone = self.objectives.in_buy_zone(team, mn, mx);
            }
            if input.buy != Purchase::None {
                let _ = self.buy(id, input.buy);
            }
            if input.drop {
                let p = &self.players[i];
                if let Some(w) = p.active_weapon() {
                    if w.def.class != weapons::WeaponClass::Knife {
                        let index = p.active;
                        self.drop_weapon(i, index, 48.0);
                    }
                }
            }

            // Weapon switching and reloads.
            {
                let p = &mut self.players[i];
                let slot = input.weapon_slot;
                if input.utility!=0 {
                    if let Some(index)=p.weapons.iter().position(|w| w.def.def_index==input.utility && w.def.class==weapons::WeaponClass::Grenade) {
                        if index!=p.active { p.select(index,time); }
                    }
                }
                if input.utility!=0 { /* selected above */ } else if slot == SLOT_LAST {
                    let last = p.last_active;
                    if last != p.active {
                        p.select(last, time);
                    }
                } else if slot >= 1 {
                    let in_slot: Vec<usize> = (0..p.weapons.len()).filter(|&w| weapon::slot(&p.weapons[w].def) == slot).collect();
                    let pressed = p.last_input.weapon_slot != slot;
                    let target = match in_slot.iter().position(|&w| w == p.active) {
                        Some(k) if pressed && in_slot.len() > 1 => Some(in_slot[(k + 1) % in_slot.len()]),
                        Some(_) => None,
                        None => in_slot.first().copied(),
                    };
                    if let Some(t) = target {
                        p.select(t, time);
                    }
                }
                let pressed_reload = input.buttons & buttons::RELOAD != 0;
                if let Some(w) = p.weapons.get_mut(p.active) {
                    if pressed_reload && w.can_reload() {
                        w.reset_zoom(time);
                        w.begin_reload(time);
                        self.events.push(GameEvent::Reload { player: id });
                    }
                }
            }

            // Objectives lock movement while planting or defusing.
            let busy = self.update_plant(i, &input, time) || self.update_defuse(i, &input, time);
            if !self.players[i].alive {
                continue;
            }
            if frozen || busy {
                input.forward_move = 0.0;
                input.side_move = 0.0;
                input.buttons &= !buttons::JUMP;
            }

            // Movement.
            let max_speed = self.players[i].active_weapon().map_or(250.0, |w| w.max_speed());
            let move_input = MoveInput {
                view_angles: input.view_angles,
                forward_move: input.forward_move,
                side_move: input.side_move,
                buttons: input.buttons,
                weapon_max_speed: max_speed,
            };
            {
                let p = &mut self.players[i];
                p.view_angles = input.view_angles;
                // Minecraft fluids use the same collision sweeps as CS movement.
                let swimming=collision.dynamic.as_ref().is_some_and(|d|d.point_contents(p.movement.origin+Vec3::Z*32.,contents::WATER)!=0);
                if swimming {
                    let mut config=self.config.clone();config.gravity=0.;config.air_accelerate=8.;config.air_max_wishspeed=160.;config.max_speed=160.;
                    p.movement.velocity*=0.97;
                    let vertical=if input.buttons&buttons::JUMP!=0{140.}else if input.buttons&buttons::DUCK!=0{-100.}else{-input.view_angles.x.to_radians().sin()*input.forward_move*0.3-12.};
                    p.movement.velocity.z=vertical;
                    p.last_events=movement::player_move(&mut p.movement,&move_input,&config,collision.as_ref(),TICK_DT);
                } else {
                    p.last_events = movement::player_move(&mut p.movement, &move_input, &self.config, collision.as_ref(), TICK_DT);
                }
                if p.last_events.jumped {
                    self.events.push(GameEvent::Jump { player: id });
                }
                if let Some(speed) = p.last_events.landed {
                    self.events.push(GameEvent::Land { player: id, speed });
                }
            }

            // Firing.
            let attack = input.buttons & buttons::ATTACK != 0 && !frozen && !busy;
            let pressed = attack && self.players[i].last_input.buttons & buttons::ATTACK == 0;
            let secondary = input.buttons & buttons::ATTACK2 != 0 && !frozen && !busy;
            let secondary_pressed = secondary && self.players[i].last_input.buttons & buttons::ATTACK2 == 0;
            let mut fire = false;
            let mut stab = false;
            {
                let p = &mut self.players[i];
                let ducked = p.movement.ducked;
                let eye = p.eye();
                if let Some(w) = p.weapons.get_mut(p.active) {
                    if secondary_pressed && w.zoom_levels() > 0 && w.reload_end.is_none()
                        && time >= w.next_attack && time >= w.next_secondary_attack {
                        w.resume_zoom = None;
                        w.set_zoom((w.zoom_level + 1) % (w.zoom_levels() + 1), time);
                        w.next_secondary_attack = time + 0.3;
                        self.events.push(GameEvent::Scope { player: id, weapon: w.def.class_name.clone(), origin: eye, level: w.zoom_level });
                    }
                    stab = secondary && w.is_melee();
                    let can_fire = w.def.class_name != C4;
                    // Pulling the trigger stops a shell-by-shell reload once
                    // a shell is in, as in the game.
                    if attack && (pressed || w.def.full_auto) && w.reload_end.is_some() && w.shell_reload().is_some() && w.clip > 0 {
                        w.reload_end = None;
                    }
                    if can_fire && (attack || stab) && w.reload_end.is_none() && time >= w.next_attack && (w.def.full_auto || pressed || w.is_melee()) {
                        if w.uses_ammo() && w.clip <= 0 {
                            if w.can_reload() {
                                w.reset_zoom(time);
                                w.begin_reload(time);
                                self.events.push(GameEvent::Reload { player: id });
                            }
                        } else {
                            fire = true;
                        }
                    }
                    weapon::think(w, time, TICK_DT, ducked, fire);
                }
            }
            if fire {
                if self.players[i].active_weapon().is_some_and(|w| w.def.class==weapons::WeaponClass::Grenade) {
                    self.throw_grenade(i);
                } else if self.players[i].active_weapon().is_some_and(|w| w.is_melee()) {
                    self.melee_attack(i, time, stab);
                } else { self.fire(i, time); }
            }
            if self.players[i].alive && !self.items.is_empty() {
                self.pickups(i, time);
            }
            self.players[i].last_input = input;
        }
        self.step_utility();
        if self.competitive() {
            self.check_elimination();
        }
        self.tick += 1;
    }

    fn melee_attack(&mut self, shooter: usize, time: f32, secondary: bool) {
        let p = &self.players[shooter];
        let id = p.id;
        let team = p.team;
        let eye = p.eye();
        let (dir, _, _) = movement::angle_vectors(p.view_angles);
        let w = p.active_weapon().unwrap();
        let mut def = (*w.def).clone();
        let first = w.last_melee_attack.is_none_or(|t| time - t > 0.9);
        let range = if secondary { 32.0 } else { MELEE_RANGE };
        let tr = self.collision.trace_ray(eye, eye + dir * range, contents::MASK_SHOT);
        let mut best = None;
        for (j, other) in self.players.iter().enumerate() {
            if j == shooter || !other.alive || !team.enemy_of(other.team) { continue; }
            let (mn, mx) = other.bounds();
            if let Some(t) = ray_box(eye, dir, mn, mx) {
                if t <= tr.fraction * range && best.is_none_or(|(_, bt)| t < bt) { best = Some((j, t)); }
            }
        }
        let external = self.collision.dynamic.as_ref().and_then(|d| d.hit_target(eye, tr.end))
            .filter(|(_, fraction)| best.is_none_or(|(_, t)| fraction * tr.fraction * range < t));
        let hit = if best.is_some() || external.is_some() { MeleeHit::Player } else if tr.fraction < 1.0 { MeleeHit::World } else { MeleeHit::Miss };
        self.events.push(GameEvent::Melee { player: id, weapon: def.class_name.clone(), origin: eye, secondary, hit });
        if let Some((key, fraction)) = external {
            if let Some(dynamic) = &self.collision.dynamic {
                dynamic.damage_target_at(key, if secondary { 65.0 } else if first { 40.0 } else { 25.0 }, eye, eye + (tr.end-eye)*fraction);
            }
        } else if let Some((victim, distance)) = best {
            let v = &self.players[victim];
            let (forward, _, _) = movement::angle_vectors(Vec3::new(0.0, v.view_angles.y, 0.0));
            let toward = (v.movement.origin - self.players[shooter].movement.origin).with_z(0.0).normalize_or_zero();
            let behind = forward.dot(toward) > 0.475;
            def.damage = match (secondary, behind, first) {
                (true, true, _) => 180.0, (true, false, _) => 65.0,
                (false, true, _) => 90.0, (false, false, true) => 40.0, _ => 25.0,
            };
            def.armor_ratio = 1.7;
            def.range_modifier = 1.0;
            self.apply_damage(shooter, victim, &def, distance, HitGroup::Chest, eye + dir * distance);
        }
        if external.is_none() && best.is_none() && tr.hit() {
            if let Some(dynamic) = &self.collision.dynamic {
                dynamic.damage_block(tr.end, tr.normal, if secondary { 30.0 } else { 15.0 });
            }
        }
        let active = self.players[shooter].active;
        let w = &mut self.players[shooter].weapons[active];
        w.last_melee_attack = Some(time);
        w.next_attack = time + match (secondary, hit == MeleeHit::Miss) {
            (true, true) => 1.0, (true, false) => 1.1, (false, true) => 0.4, (false, false) => 0.5,
        };
    }

    fn fire(&mut self, shooter: usize, time: f32) {
        let tick = self.tick;
        let p = &self.players[shooter];
        let id = p.id;
        let team = p.team;
        let eye = p.eye();
        let Some(w) = p.weapons.get(p.active) else { return };
        let def = w.def.clone();
        let punch = w.aim_punch();
        let ctx = AccuracyContext {
            speed: p.movement.speed_2d(),
            on_ground: p.movement.on_ground,
            ducked: p.movement.ducked,
        };
        let inacc = weapon::inaccuracy(w, ctx) * 0.001;
        let spread = w.alt_stat("spread", def.accuracy.spread) * 0.001;
        let angles = p.view_angles
            + Vec3::new(punch[0] * weapon::RECOIL_SCALE, punch[1] * weapon::RECOIL_SCALE, 0.0);
        let (fwd, right, up) = movement::angle_vectors(angles);
        let shot = p.shots;
        let range = def.range.max(1.0);

        self.events.push(GameEvent::Fire {
            player: id,
            weapon: def.class_name.clone(),
            origin: eye,
        });

        let mut rng = Rng::from_parts(&[tick, id as u64, shot]);
        for _ in 0..def.bullets.max(1) {
            let r1 = rng.next_f32();
            let a1 = rng.range(0.0, std::f32::consts::TAU);
            let r2 = rng.next_f32();
            let a2 = rng.range(0.0, std::f32::consts::TAU);
            let ox = a1.cos() * r1 * inacc + a2.cos() * r2 * spread;
            let oy = a1.sin() * r1 * inacc + a2.sin() * r2 * spread;
            let dir = (fwd + right * ox + up * oy).normalize();
            let end = eye + dir * range;
            let tr = self.collision.trace_ray(eye, end, contents::MASK_SHOT);
            let world_dist = tr.fraction * range;

            // Nearest enemy (or anyone, with friendly fire off → enemies) hull.
            let mut best: Option<(usize, f32)> = None;
            for (j, other) in self.players.iter().enumerate() {
                if j == shooter || !other.alive || !team.enemy_of(other.team) {
                    continue;
                }
                let (mn, mx) = other.bounds();
                if let Some(t) = ray_box(eye, dir, mn, mx) {
                    if t <= world_dist && best.is_none_or(|(_, bt)| t < bt) {
                        best = Some((j, t));
                    }
                }
            }
            if let Some(dynamic)=&self.collision.dynamic {
                if let Some((key,fraction))=dynamic.hit_target(eye,tr.end) {
                    let distance=fraction*world_dist;
                    if best.is_none_or(|(_,t)|distance<t) {
                        let hit=eye+dir*distance;
                        dynamic.damage_target_at(key,def.damage as f32 * def.range_modifier.powf(distance/500.0),eye,hit);
                        self.events.push(GameEvent::Tracer{player:id,start:eye,end:hit});
                        continue;
                    }
                }
            }
            match best {
                Some((j, t)) => {
                    let hit = eye + dir * t;
                    self.events.push(GameEvent::Tracer { player: id, start: eye, end: hit });
                    let victim = &self.players[j];
                    let (mn, mx) = victim.bounds();
                    let frac = ((hit.z - mn.z) / (mx.z - mn.z)).clamp(0.0, 1.0);
                    let (_, vright, _) = movement::angle_vectors(Vec3::new(0.0, victim.view_angles.y, 0.0));
                    let center = (mn + mx) * 0.5;
                    let side = ((hit - center).dot(vright) / 16.0).clamp(-1.0, 1.0);
                    let group = HitGroup::from_height(frac, side);
                    self.apply_damage(shooter, j, &def, t, group, hit);
                }
                None => {
                    self.events.push(GameEvent::Tracer { player: id, start: eye, end: tr.end });
                    if tr.fraction < 1.0 && !self.collision.is_sky(tr.surface) {
                        if let Some(dynamic)=&self.collision.dynamic {dynamic.damage_block(tr.end,tr.normal,def.damage as f32 * def.range_modifier.powf(world_dist/500.0));}
                        self.events.push(GameEvent::Impact {
                            pos: tr.end,
                            normal: tr.normal,
                            surface: tr.surface,
                        });
                    }
                }
            }
        }

        let p = &mut self.players[shooter];
        p.shots += 1;
        if let Some(w) = p.weapons.get_mut(p.active) {
            if w.uses_ammo() {
                w.clip -= 1;
            }
            w.next_attack = time + w.alt_stat("cycletime", def.cycle_time).max(0.01);
            w.accuracy_penalty += w.alt_stat("inaccuracy fire", def.accuracy.inaccuracy_fire);
            w.recoil_index += 1.0;
            if w.zoom_level > 0 && def.attr_f32("unzoom after shot").is_some_and(|v| v != 0.0) {
                w.resume_zoom = (w.clip > 0).then_some(w.zoom_level);
                w.set_zoom(0, time);
            }
        }
    }

    fn apply_damage(&mut self, attacker: usize, victim: usize, def: &weapons::WeaponDefinition, distance: f32, group: HitGroup, pos: Vec3) {
        let v = &self.players[victim];
        let (health_dmg, armor_dmg) = weapon::damage(def, distance, group, v.armor, v.helmet);
        let attacker_id = self.players[attacker].id;
        let v = &mut self.players[victim];
        v.health -= health_dmg.max(1);
        v.armor = (v.armor - armor_dmg).max(0);
        let victim_id = v.id;
        self.events.push(GameEvent::Damage {
            attacker: attacker_id,
            victim: victim_id,
            health: health_dmg,
            group,
            pos,
        });
        if self.players[victim].health <= 0 {
            self.kill(victim, Some(attacker), &def.class_name, group == HitGroup::Head);
            if self.competitive() && self.players[attacker].team.enemy_of(self.players[victim].team) {
                let award = (def.kill_award as f32 * self.rules.kill_award_factor) as i32;
                let a = &mut self.players[attacker];
                a.money = (a.money + award).min(self.rules.max_money);
            }
        }
    }

    /// Damage from the host world's creatures or environmental hazards.
    pub fn damage_environment(&mut self, id: PlayerId, amount: i32, cause: &str) {
        let Some(victim) = self.players.iter().position(|p| p.id == id && p.alive) else { return };
        let amount = amount.max(0);
        if amount == 0 { return; }
        self.players[victim].health -= amount;
        self.events.push(GameEvent::Damage { attacker: WORLD, victim: id, health: amount, group: HitGroup::Chest, pos: self.players[victim].eye() });
        if self.players[victim].health <= 0 { self.kill(victim, None, cause, false); }
    }

    fn kill(&mut self, victim: usize, attacker: Option<usize>, weapon: &str, headshot: bool) {
        let now = self.time();
        let v = &mut self.players[victim];
        v.health = 0;
        v.alive = false;
        for w in &mut v.weapons { w.reset_zoom(now); }
        v.deaths += 1;
        v.plant_start = None;
        v.respawn_at = now + RESPAWN_DELAY;
        let victim_id = v.id;
        let attacker_id = match attacker {
            Some(a) => {
                self.players[a].kills += 1;
                self.players[a].id
            }
            None => WORLD,
        };
        if let Some(a)=attacker {
            let enemy=self.players[a].team!=self.players[victim].team;
            if enemy {
                if let Some(w)=self.players[a].active_weapon_mut() {
                    if w.def.class_name==weapon && w.cosmetic_owner==Some(attacker_id) {
                        if let Some(c)=w.cosmetic.as_mut() {
                            if let Some(count)=c.stattrak.as_mut() {
                                *count=count.saturating_add(1);
                                self.events.push(GameEvent::StatTrakKill{owner:attacker_id,serial:c.serial});
                            }
                        }
                    }
                }
            }
        }
        self.events.push(GameEvent::Kill {
            attacker: attacker_id,
            victim: victim_id,
            weapon: weapon.to_string(),
            headshot,
        });
        self.death_drops(victim);
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            tick: self.tick,
            players: self
                .players
                .iter()
                .map(|p| PlayerSnapshot {
                    id: p.id,
                    origin: p.movement.origin,
                    velocity: p.movement.velocity,
                    view_angles: p.view_angles,
                    view_offset: p.movement.view_offset(),
                    on_ground: p.movement.on_ground,
                    ducked: p.movement.ducked,
                    alive: p.alive,
                    health: p.health,
                })
                .collect(),
        }
    }

    /// True if nothing solid blocks the line between two points.
    pub fn visible(&self, from: Vec3, to: Vec3) -> bool {
        let tr = self.collision.trace_ray(from, to, contents::MASK_SHOT);
        tr.fraction >= 1.0 || self.collision.is_sky(tr.surface)
    }

    /// The player carrying the bomb, if any.
    pub fn bomb_carrier(&self) -> Option<&Player> {
        self.players.iter().find(|p| p.alive && p.has_bomb())
    }

    /// The bomb lying on the ground, if any.
    pub fn dropped_bomb(&self) -> Option<&GroundItem> {
        self.items.iter().find(|i| i.class_name() == C4)
    }
}

fn kv_angles(s: &str) -> Vec3 {
    let v: Vec<f32> = s.split_whitespace().filter_map(|t| t.parse().ok()).collect();
    Vec3::new(
        v.first().copied().unwrap_or(0.0),
        v.get(1).copied().unwrap_or(0.0),
        v.get(2).copied().unwrap_or(0.0),
    )
}

#[cfg(test)]
mod tests;
