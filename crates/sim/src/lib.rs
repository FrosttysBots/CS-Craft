//! Fixed-tick simulation.
//!
//! One funnel for authority, prediction and replay:
//! `World` + per-player `TickInput` → [`World::step`] → [`Snapshot`].
//! The step is deterministic for the same inputs, so a client can predict
//! by running the very same code the server runs.
//!
//! The tick rate matches CS:GO's 64-tick matchmaking servers.

use collision::CollisionWorld;
use glam::Vec3;
use movement::{MoveConfig, MoveEvents, MoveInput, PlayerMove};
use std::sync::Arc;

pub const TICK_RATE: u32 = 64;
pub const TICK_DT: f32 = 1.0 / TICK_RATE as f32;

pub type PlayerId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Team {
    Terrorist,
    CounterTerrorist,
    Spectator,
}

/// Input for one player for one tick.
pub type TickInput = MoveInput;

#[derive(Clone, Debug)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    pub team: Team,
    pub movement: PlayerMove,
    pub view_angles: Vec3,
    pub health: i32,
    pub armor: i32,
    pub alive: bool,
    pub last_input: TickInput,
    pub last_events: MoveEvents,
}

/// A spawn point from the map.
#[derive(Clone, Copy, Debug)]
pub struct SpawnPoint {
    pub origin: Vec3,
    pub yaw: f32,
    pub team: Team,
}

pub struct World {
    pub collision: Arc<CollisionWorld>,
    pub config: MoveConfig,
    pub players: Vec<Player>,
    pub spawns: Vec<SpawnPoint>,
    pub tick: u64,
    next_id: PlayerId,
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

impl World {
    pub fn new(collision: Arc<CollisionWorld>) -> Self {
        Self {
            collision,
            config: MoveConfig::default(),
            players: Vec::new(),
            spawns: Vec::new(),
            tick: 0,
            next_id: 1,
        }
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
                let yaw = ent
                    .str("angles")
                    .map(kv_angles)
                    .map(|a| a.y)
                    .unwrap_or(0.0);
                self.spawns.push(SpawnPoint { origin, yaw, team });
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
            .trace_box(start, origin - Vec3::new(0.0, 0.0, 256.0), mins, maxs, collision::contents::MASK_PLAYERSOLID);
        if tr.start_solid || tr.fraction == 1.0 {
            origin
        } else {
            tr.end
        }
    }

    pub fn add_player(&mut self, name: &str, team: Team) -> PlayerId {
        let id = self.next_id;
        self.next_id += 1;
        self.players.push(Player {
            id,
            name: name.to_string(),
            team,
            movement: PlayerMove::default(),
            view_angles: Vec3::ZERO,
            health: 100,
            armor: 0,
            alive: false,
            last_input: TickInput::default(),
            last_events: MoveEvents::default(),
        });
        self.respawn(id);
        id
    }

    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    pub fn player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }

    /// Puts a player on a free spawn point for their team.
    pub fn respawn(&mut self, id: PlayerId) {
        let Some(team) = self.player(id).map(|p| p.team) else { return };
        let taken: Vec<Vec3> = self
            .players
            .iter()
            .filter(|p| p.id != id && p.alive)
            .map(|p| p.movement.origin)
            .collect();
        let spawn = self
            .spawns
            .iter()
            .filter(|s| s.team == team)
            .find(|s| taken.iter().all(|t| t.distance(s.origin) > 40.0))
            .or_else(|| self.spawns.iter().find(|s| s.team == team))
            .or_else(|| self.spawns.first())
            .copied()
            .unwrap_or(SpawnPoint {
                origin: Vec3::ZERO,
                yaw: 0.0,
                team,
            });
        let origin = self.drop_to_floor(spawn.origin, false);
        if let Some(p) = self.player_mut(id) {
            p.movement = PlayerMove::spawn_at(origin);
            p.view_angles = Vec3::new(0.0, spawn.yaw, 0.0);
            p.health = 100;
            p.alive = true;
        }
    }

    /// Advances the world by one tick. Players without an input this tick
    /// repeat their last one (as a server does for a late packet), minus
    /// one-shot buttons.
    pub fn step(&mut self, inputs: &[(PlayerId, TickInput)]) {
        let collision = self.collision.clone();
        for p in &mut self.players {
            if !p.alive {
                continue;
            }
            let input = inputs
                .iter()
                .find(|(id, _)| *id == p.id)
                .map(|(_, i)| *i)
                .unwrap_or(TickInput {
                    buttons: p.last_input.buttons & !movement::buttons::JUMP,
                    ..p.last_input
                });
            p.view_angles = input.view_angles;
            p.last_events = movement::player_move(&mut p.movement, &input, &self.config, collision.as_ref(), TICK_DT);
            p.last_input = input;
        }
        self.tick += 1;
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
}

fn kv_angles(s: &str) -> Vec3 {
    let v: Vec<f32> = s.split_whitespace().filter_map(|t| t.parse().ok()).collect();
    Vec3::new(
        v.first().copied().unwrap_or(0.0),
        v.get(1).copied().unwrap_or(0.0),
        v.get(2).copied().unwrap_or(0.0),
    )
}
