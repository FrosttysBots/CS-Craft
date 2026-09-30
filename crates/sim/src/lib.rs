//! Fixed-tick simulation.
//!
//! One funnel for authority, prediction and replay:
//! `World` + per-player [`TickInput`] → [`World::step`] → [`Snapshot`] and
//! [`GameEvent`]s. The step is deterministic for the same inputs (all
//! randomness is seeded from the tick, player and shot), so a client can
//! predict by running the very same code the server runs.
//!
//! The tick rate matches CS:GO's 64-tick matchmaking servers.

pub mod bots;
pub mod rng;
pub mod weapon;

use collision::{contents, CollisionWorld};
use glam::Vec3;
use movement::{buttons, MoveConfig, MoveEvents, MoveInput, PlayerMove};
use rng::Rng;
use std::sync::Arc;
use weapon::{AccuracyContext, HitGroup, WeaponState};

pub const TICK_RATE: u32 = 64;
pub const TICK_DT: f32 = 1.0 / TICK_RATE as f32;
/// Seconds before a dead player respawns (deathmatch rules).
pub const RESPAWN_DELAY: f32 = 2.0;
/// Knife reach, units.
pub const MELEE_RANGE: f32 = 48.0;

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
    /// `movement::buttons` bits (JUMP, DUCK, WALK, ATTACK, RELOAD...).
    pub buttons: u32,
    /// 1-based inventory slot to switch to, 0 for none.
    pub weapon_slot: u8,
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
    pub alive: bool,
    pub respawn_at: f32,
    pub weapons: Vec<WeaponState>,
    pub active: usize,
    pub kills: u32,
    pub deaths: u32,
    pub shots: u64,
    pub last_input: TickInput,
    pub last_events: MoveEvents,
}

impl Player {
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
}

/// A spawn point from the map.
#[derive(Clone, Copy, Debug)]
pub struct SpawnPoint {
    pub origin: Vec3,
    pub yaw: f32,
    pub team: Team,
}

/// Things that happened during a tick, for effects, sounds and the HUD.
#[derive(Clone, Debug, PartialEq)]
pub enum GameEvent {
    Fire { player: PlayerId, weapon: String, origin: Vec3 },
    Tracer { start: Vec3, end: Vec3 },
    Impact { pos: Vec3, normal: Vec3, surface: u32 },
    Damage { attacker: PlayerId, victim: PlayerId, health: i32, group: HitGroup, pos: Vec3 },
    Kill { attacker: PlayerId, victim: PlayerId, weapon: String, headshot: bool },
    Reload { player: PlayerId },
    Jump { player: PlayerId },
    Land { player: PlayerId, speed: f32 },
}

pub struct World {
    pub collision: Arc<CollisionWorld>,
    pub catalog: Arc<weapons::Catalog>,
    pub config: MoveConfig,
    pub players: Vec<Player>,
    pub spawns: Vec<SpawnPoint>,
    pub tick: u64,
    /// Events from the most recent step.
    pub events: Vec<GameEvent>,
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
        Self {
            collision,
            catalog,
            config: MoveConfig::default(),
            players: Vec::new(),
            spawns: Vec::new(),
            tick: 0,
            events: Vec::new(),
            next_id: 1,
        }
    }

    pub fn time(&self) -> f32 {
        self.tick as f32 * TICK_DT
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
            alive: false,
            respawn_at: 0.0,
            weapons: Vec::new(),
            active: 0,
            kills: 0,
            deaths: 0,
            shots: 0,
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

    /// Deathmatch loadout: a rifle, a pistol and a knife.
    fn loadout(&self, team: Team) -> Vec<WeaponState> {
        let names: &[&str] = match team {
            Team::Terrorist => &["weapon_ak47", "weapon_glock", "weapon_knife_t"],
            _ => &["weapon_m4a1_silencer", "weapon_usp_silencer", "weapon_knife"],
        };
        names
            .iter()
            .filter_map(|n| self.catalog.get(n))
            .map(|d| WeaponState::new(Arc::new(d.clone())))
            .collect()
    }

    /// Puts a player on a free spawn point for their team, fully equipped.
    pub fn respawn(&mut self, id: PlayerId) {
        let Some(team) = self.player(id).map(|p| p.team) else { return };
        let taken: Vec<Vec3> = self
            .players
            .iter()
            .filter(|p| p.id != id && p.alive)
            .map(|p| p.movement.origin)
            .collect();
        let offset = (self.tick as usize + id as usize * 7) % self.spawns.len().max(1);
        let team_spawns: Vec<SpawnPoint> = self.spawns.iter().copied().filter(|s| s.team == team).collect();
        let spawn = (0..team_spawns.len())
            .map(|i| team_spawns[(i + offset) % team_spawns.len()])
            .find(|s| taken.iter().all(|t| t.distance(s.origin) > 40.0))
            .or_else(|| team_spawns.first().copied())
            .or_else(|| self.spawns.first().copied())
            .unwrap_or(SpawnPoint { origin: Vec3::ZERO, yaw: 0.0, team });
        let origin = self.drop_to_floor(spawn.origin, false);
        let loadout = self.loadout(team);
        if let Some(p) = self.player_mut(id) {
            p.movement = PlayerMove::spawn_at(origin);
            p.view_angles = Vec3::new(0.0, spawn.yaw, 0.0);
            p.health = 100;
            p.armor = 100;
            p.helmet = true;
            p.alive = true;
            p.weapons = loadout;
            p.active = 0;
        }
    }

    /// Advances the world by one tick. Players without an input this tick
    /// repeat their last one (as a server does for a late packet), minus
    /// one-shot buttons.
    pub fn step(&mut self, inputs: &[(PlayerId, TickInput)]) {
        self.events.clear();
        let time = self.time();
        let collision = self.collision.clone();

        // Respawns.
        let due: Vec<PlayerId> = self
            .players
            .iter()
            .filter(|p| !p.alive && p.team != Team::Spectator && time >= p.respawn_at)
            .map(|p| p.id)
            .collect();
        for id in due {
            self.respawn(id);
        }

        for i in 0..self.players.len() {
            if !self.players[i].alive {
                continue;
            }
            let id = self.players[i].id;
            let input = inputs.iter().find(|(pid, _)| *pid == id).map(|(_, inp)| *inp).unwrap_or(TickInput {
                buttons: self.players[i].last_input.buttons & !(buttons::JUMP | buttons::RELOAD),
                weapon_slot: 0,
                ..self.players[i].last_input
            });

            // Weapon switching and reloads.
            {
                let p = &mut self.players[i];
                let slot = input.weapon_slot as usize;
                if slot >= 1 && slot <= p.weapons.len() && slot - 1 != p.active {
                    p.active = slot - 1;
                    let w = &mut p.weapons[p.active];
                    w.reload_end = None;
                    w.next_attack = w.next_attack.max(time + weapon::deploy_time(&w.def));
                }
                let pressed_reload = input.buttons & buttons::RELOAD != 0;
                if let Some(w) = p.weapons.get_mut(p.active) {
                    if pressed_reload && w.can_reload() {
                        w.reload_end = Some(time + weapon::reload_time(&w.def));
                        self.events.push(GameEvent::Reload { player: id });
                    }
                }
            }

            // Movement.
            let max_speed = self.players[i].active_weapon().map_or(250.0, |w| w.def.max_player_speed);
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
                p.last_events = movement::player_move(&mut p.movement, &move_input, &self.config, collision.as_ref(), TICK_DT);
                if p.last_events.jumped {
                    self.events.push(GameEvent::Jump { player: id });
                }
                if let Some(speed) = p.last_events.landed {
                    self.events.push(GameEvent::Land { player: id, speed });
                }
            }

            // Firing.
            let attack = input.buttons & buttons::ATTACK != 0;
            let pressed = attack && self.players[i].last_input.buttons & buttons::ATTACK == 0;
            let mut fire = false;
            {
                let p = &mut self.players[i];
                let ducked = p.movement.ducked;
                if let Some(w) = p.weapons.get_mut(p.active) {
                    if attack && w.reload_end.is_none() && time >= w.next_attack && (w.def.full_auto || pressed || w.is_melee()) {
                        if w.uses_ammo() && w.clip <= 0 {
                            if w.can_reload() {
                                w.reload_end = Some(time + weapon::reload_time(&w.def));
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
                self.fire(i, time);
            }
            self.players[i].last_input = input;
        }
        self.tick += 1;
    }

    fn fire(&mut self, shooter: usize, time: f32) {
        let tick = self.tick;
        let p = &self.players[shooter];
        let id = p.id;
        let team = p.team;
        let eye = p.eye();
        let Some(w) = p.weapons.get(p.active) else { return };
        let def = w.def.clone();
        let melee = w.is_melee();
        let punch = w.aim_punch();
        let ctx = AccuracyContext {
            speed: p.movement.speed_2d(),
            on_ground: p.movement.on_ground,
            ducked: p.movement.ducked,
        };
        let inacc = if melee { 0.0 } else { weapon::inaccuracy(w, ctx) * 0.001 };
        let spread = if melee { 0.0 } else { def.accuracy.spread * 0.001 };
        let angles = p.view_angles
            + Vec3::new(punch[0] * weapon::RECOIL_SCALE, punch[1] * weapon::RECOIL_SCALE, 0.0);
        let (fwd, right, up) = movement::angle_vectors(angles);
        let shot = p.shots;
        let range = if melee { MELEE_RANGE } else { def.range.max(1.0) };

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
            match best {
                Some((j, t)) => {
                    let hit = eye + dir * t;
                    self.events.push(GameEvent::Tracer { start: eye, end: hit });
                    let victim = &self.players[j];
                    let (mn, mx) = victim.bounds();
                    let frac = ((hit.z - mn.z) / (mx.z - mn.z)).clamp(0.0, 1.0);
                    let (_, vright, _) = movement::angle_vectors(Vec3::new(0.0, victim.view_angles.y, 0.0));
                    let center = (mn + mx) * 0.5;
                    let side = ((hit - center).dot(vright) / 16.0).clamp(-1.0, 1.0);
                    let group = if melee { HitGroup::Chest } else { HitGroup::from_height(frac, side) };
                    self.apply_damage(shooter, j, &def, t, group, hit);
                }
                None => {
                    self.events.push(GameEvent::Tracer { start: eye, end: tr.end });
                    if tr.fraction < 1.0 && !self.collision.is_sky(tr.surface) {
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
            w.next_attack = time + if melee { 0.5 } else { def.cycle_time.max(0.01) };
            if !melee {
                w.accuracy_penalty += def.accuracy.inaccuracy_fire;
                w.recoil_index += 1.0;
            }
        }
    }

    fn apply_damage(&mut self, attacker: usize, victim: usize, def: &weapons::WeaponDefinition, distance: f32, group: HitGroup, pos: Vec3) {
        let v = &self.players[victim];
        let (health_dmg, armor_dmg) = if def.class == weapons::WeaponClass::Knife {
            (40, 0)
        } else {
            weapon::damage(def, distance, group, v.armor, v.helmet)
        };
        let attacker_id = self.players[attacker].id;
        let now = self.time();
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
        if v.health <= 0 {
            v.health = 0;
            v.alive = false;
            v.deaths += 1;
            v.respawn_at = now + RESPAWN_DELAY;
            self.players[attacker].kills += 1;
            self.events.push(GameEvent::Kill {
                attacker: attacker_id,
                victim: victim_id,
                weapon: def.class_name.clone(),
                headshot: group == HitGroup::Head,
            });
        }
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
mod tests {
    use super::*;

    #[test]
    fn ray_box_hits_and_misses() {
        let t = ray_box(Vec3::ZERO, Vec3::X, Vec3::new(10.0, -1.0, -1.0), Vec3::new(12.0, 1.0, 1.0));
        assert_eq!(t, Some(10.0));
        assert!(ray_box(Vec3::ZERO, Vec3::Y, Vec3::new(10.0, -1.0, -1.0), Vec3::new(12.0, 1.0, 1.0)).is_none());
    }
}
