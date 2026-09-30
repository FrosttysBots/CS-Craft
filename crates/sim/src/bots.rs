//! Bots: produce [`TickInput`]s for AI players, so bots obey exactly the
//! same movement, weapon and hit rules as humans.
//!
//! * Navigation: with the map's nav mesh, bots pick destinations (weighted
//!   towards bombsites and main routes), follow A* paths through area
//!   portals, jump/crouch where the mesh says so, and re-path when stuck.
//!   Without a mesh they wander and turn away from walls.
//! * Combat: perception runs ~8 times a second per bot (staggered); on
//!   seeing an enemy a bot reacts after a delay, turns at a limited rate
//!   with aim error, stops and fires controlled bursts.

use crate::rng::Rng;
use crate::{PlayerId, TickInput, World};
use glam::Vec3;
use movement::buttons;
use nav::NavMesh;
use std::collections::HashMap;
use std::sync::Arc;

/// Ticks between perception updates (64 Hz / 8 ≈ 8 per second).
const PERCEPTION_INTERVAL: u64 = 8;

#[derive(Clone, Debug, Default)]
struct Brain {
    target: Option<PlayerId>,
    target_pos: Vec3,
    seen_at: f32,
    last_seen: f32,
    pitch: f32,
    yaw: f32,
    wander_yaw: f32,
    last_pos: Vec3,
    stuck_ticks: u32,
    burst_left: u32,
    next_burst: f32,
    strafe: f32,
    next_strafe_change: f32,
    path: Vec<u32>,
    path_index: usize,
    progress_pos: Vec3,
    progress_time: f32,
}

#[derive(Clone, Debug)]
pub struct Bots {
    brains: HashMap<PlayerId, Brain>,
    /// 0 (harmless) .. 1 (sharp).
    pub skill: f32,
    /// If false, bots stand still and never shoot (target practice).
    pub aggressive: bool,
    pub nav: Option<Arc<NavMesh>>,
}

impl Default for Bots {
    fn default() -> Self {
        Self {
            brains: HashMap::new(),
            skill: 0.5,
            aggressive: true,
            nav: None,
        }
    }
}

/// Pitch/yaw (degrees) looking from `from` to `to`, Source conventions.
pub fn look_angles(from: Vec3, to: Vec3) -> (f32, f32) {
    let d = to - from;
    let yaw = d.y.atan2(d.x).to_degrees();
    let pitch = -(d.z.atan2(d.truncate().length())).to_degrees();
    (pitch, yaw)
}

fn angle_diff(a: f32, b: f32) -> f32 {
    let mut d = (a - b) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d
}

fn approach(current: f32, target: f32, max_step: f32) -> f32 {
    let d = angle_diff(target, current);
    current + d.clamp(-max_step, max_step)
}

/// Places bots like to go, with weights.
const HOT_PLACES: &[(&str, u32)] = &[
    ("BombsiteA", 6),
    ("BombsiteB", 6),
    ("Middle", 4),
    ("LongA", 4),
    ("Catwalk", 3),
    ("TopofMid", 3),
    ("UpperTunnel", 3),
    ("Short", 2),
    ("CTSpawn", 2),
    ("TSpawn", 2),
];

fn pick_goal(nav: &NavMesh, rng: &mut Rng) -> Option<u32> {
    if nav.areas.is_empty() {
        return None;
    }
    // 70 %: a hot place, else any area.
    if rng.next_f32() < 0.7 {
        let total: u32 = HOT_PLACES.iter().map(|(_, w)| w).sum();
        let mut roll = (rng.next_f32() * total as f32) as u32;
        for (name, w) in HOT_PLACES {
            if roll < *w {
                let candidates: Vec<u32> = nav
                    .areas
                    .iter()
                    .filter(|a| nav.place_name(a.place) == Some(name) && a.size() > 1500.0)
                    .map(|a| a.id)
                    .collect();
                if !candidates.is_empty() {
                    return Some(candidates[(rng.next_u64() as usize) % candidates.len()]);
                }
                break;
            }
            roll -= w;
        }
    }
    let big: Vec<u32> = nav.areas.iter().filter(|a| a.size() > 2500.0).map(|a| a.id).collect();
    let pool = if big.is_empty() { nav.areas.iter().map(|a| a.id).collect() } else { big };
    Some(pool[(rng.next_u64() as usize) % pool.len()])
}

impl Bots {
    pub fn with_skill(skill: f32) -> Self {
        Self {
            skill: skill.clamp(0.0, 1.0),
            ..Default::default()
        }
    }

    pub fn with_nav(mut self, nav: Option<Arc<NavMesh>>) -> Self {
        self.nav = nav;
        self
    }

    pub fn think(&mut self, world: &World) -> Vec<(PlayerId, TickInput)> {
        let time = world.time();
        let dt = crate::TICK_DT;
        let nav = self.nav.clone();
        let mut out = Vec::new();
        for bot in world.players.iter().filter(|p| p.is_bot) {
            let brain = self.brains.entry(bot.id).or_insert_with(|| Brain {
                yaw: bot.view_angles.y,
                wander_yaw: bot.view_angles.y,
                last_pos: bot.movement.origin,
                ..Default::default()
            });
            if !bot.alive {
                // Forget everything; start fresh after respawning.
                *brain = Brain {
                    yaw: bot.view_angles.y,
                    wander_yaw: bot.view_angles.y,
                    last_pos: bot.movement.origin,
                    ..Default::default()
                };
                continue;
            }
            if brain.progress_time == 0.0 {
                brain.progress_time = time;
                brain.progress_pos = bot.movement.origin;
                brain.yaw = bot.view_angles.y;
            }
            let mut rng = Rng::from_parts(&[world.tick, bot.id as u64, 0xB07]);
            let eye = bot.eye();

            // Perception (throttled and staggered).
            if self.aggressive && (world.tick + bot.id as u64) % PERCEPTION_INTERVAL == 0 {
                let mut best: Option<(PlayerId, Vec3, f32)> = None;
                let (fwd, _, _) = movement::angle_vectors(Vec3::new(0.0, brain.yaw, 0.0));
                for other in world.players.iter().filter(|o| o.alive && bot.team.enemy_of(o.team)) {
                    let chest = other.movement.origin + Vec3::new(0.0, 0.0, other.movement.view_offset() - 12.0);
                    let to = chest - eye;
                    let dist = to.length();
                    if dist > 3500.0 || best.is_some_and(|(_, _, d)| d < dist) {
                        continue;
                    }
                    // Field of view ~140°, but notice anyone very close.
                    let facing = to.truncate().normalize_or_zero().dot(fwd.truncate());
                    if facing < -0.35 && dist > 300.0 {
                        continue;
                    }
                    if world.visible(eye, chest) {
                        best = Some((other.id, chest, dist));
                    }
                }
                match best {
                    Some((id, pos, _)) => {
                        if brain.target != Some(id) {
                            brain.seen_at = time;
                        }
                        brain.target = Some(id);
                        brain.target_pos = pos;
                        brain.last_seen = time;
                    }
                    None => brain.target = None,
                }
            }
            // Track a known target between perception updates.
            if let Some(t) = brain.target.and_then(|id| world.player(id)) {
                if t.alive {
                    brain.target_pos = t.movement.origin + Vec3::new(0.0, 0.0, t.movement.view_offset() - 12.0);
                } else {
                    brain.target = None;
                }
            }

            let mut input = TickInput {
                weapon_slot: 1,
                ..Default::default()
            };
            let turn_rate = (240.0 + 480.0 * self.skill) * dt;
            if brain.target.is_some() {
                let err = (1.0 - self.skill) * 3.0;
                let (mut tp, mut ty) = look_angles(eye, brain.target_pos);
                tp += rng.range(-err, err);
                ty += rng.range(-err, err);
                brain.pitch = approach(brain.pitch, tp, turn_rate);
                brain.yaw = approach(brain.yaw, ty, turn_rate);
                let on_target = angle_diff(ty, brain.yaw).abs() < 2.5 && angle_diff(tp, brain.pitch).abs() < 2.5;
                let reaction = 0.45 - 0.3 * self.skill;
                if on_target && time - brain.seen_at > reaction {
                    if brain.burst_left == 0 && time >= brain.next_burst {
                        brain.burst_left = 3 + (rng.next_f32() * 4.0) as u32;
                    }
                    if brain.burst_left > 0 {
                        input.buttons |= buttons::ATTACK;
                        brain.burst_left -= 1;
                        if brain.burst_left == 0 {
                            brain.next_burst = time + 0.25 + rng.next_f32() * 0.3;
                        }
                    }
                } else {
                    if time >= brain.next_strafe_change {
                        brain.strafe = if rng.next_f32() < 0.5 { -450.0 } else { 450.0 };
                        brain.next_strafe_change = time + 0.3 + rng.next_f32() * 0.5;
                    }
                    input.side_move = brain.strafe;
                }
            } else if self.aggressive {
                match nav.as_deref() {
                    Some(mesh) => follow_path(mesh, bot, brain, &mut input, &mut rng, time, turn_rate),
                    None => wander(bot, brain, &mut input, &mut rng, turn_rate),
                }
            }
            input.view_angles = Vec3::new(brain.pitch.clamp(-89.0, 89.0), brain.yaw, 0.0);
            out.push((bot.id, input));
        }
        out
    }
}

fn wander(bot: &crate::Player, brain: &mut Brain, input: &mut TickInput, rng: &mut Rng, turn_rate: f32) {
    let moved = (bot.movement.origin - brain.last_pos).truncate().length();
    brain.last_pos = bot.movement.origin;
    brain.stuck_ticks = if moved < 0.5 { brain.stuck_ticks + 1 } else { 0 };
    if brain.stuck_ticks > 8 {
        brain.wander_yaw += rng.range(90.0, 270.0);
        brain.stuck_ticks = 0;
    } else if rng.next_f32() < 0.004 {
        brain.wander_yaw += rng.range(-60.0, 60.0);
    }
    brain.yaw = approach(brain.yaw, brain.wander_yaw, turn_rate * 0.5);
    brain.pitch = approach(brain.pitch, 0.0, turn_rate);
    input.forward_move = 450.0;
}

fn follow_path(
    mesh: &NavMesh,
    bot: &crate::Player,
    brain: &mut Brain,
    input: &mut TickInput,
    rng: &mut Rng,
    time: f32,
    turn_rate: f32,
) {
    let pos = bot.movement.origin;
    let need_path = brain.path_index + 1 >= brain.path.len();
    if need_path {
        brain.path.clear();
        brain.path_index = 0;
        if let (Some(start), Some(goal)) = (mesh.nearest_area(pos), pick_goal(mesh, rng)) {
            if let Some(p) = mesh.find_path(start.id, goal) {
                brain.path = p;
            }
        }
        brain.progress_time = time;
        brain.progress_pos = pos;
        if brain.path.len() < 2 {
            wander(bot, brain, input, rng, turn_rate);
            return;
        }
    }
    // Advance when inside the next area.
    while brain.path_index + 1 < brain.path.len() {
        let next = brain.path[brain.path_index + 1];
        match mesh.area(next) {
            Some(a) if a.contains_xy(pos, -4.0) && (pos.z - a.z_at(pos.x, pos.y)).abs() < 48.0 => {
                brain.path_index += 1;
            }
            _ => break,
        }
    }
    if brain.path_index + 1 >= brain.path.len() {
        return;
    }
    let cur = brain.path[brain.path_index];
    let next = brain.path[brain.path_index + 1];
    let Some(target) = mesh.portal(cur, next, 20.0) else {
        brain.path.clear();
        return;
    };
    let (_, ty) = look_angles(pos, target);
    brain.yaw = approach(brain.yaw, ty, turn_rate);
    brain.pitch = approach(brain.pitch, 0.0, turn_rate);
    // Move along the path; slow down for sharp turns.
    let off = angle_diff(ty, brain.yaw).abs();
    input.forward_move = if off > 60.0 { 150.0 } else { 450.0 };
    if let Some(a) = mesh.area(next) {
        if a.flags & nav::attr::JUMP != 0 || target.z - pos.z > 20.0 {
            input.buttons |= buttons::JUMP;
        }
        if a.flags & nav::attr::CROUCH != 0 {
            input.buttons |= buttons::DUCK;
        }
    }
    // Stuck handling: jump after 0.75 s without progress, re-path after 2 s.
    if pos.distance(brain.progress_pos) > 48.0 {
        brain.progress_pos = pos;
        brain.progress_time = time;
    } else if time - brain.progress_time > 2.0 {
        brain.path.clear();
        brain.progress_time = time;
    } else if time - brain.progress_time > 0.75 {
        input.buttons |= buttons::JUMP;
        input.side_move = if rng.next_f32() < 0.5 { -450.0 } else { 450.0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_angles_conventions() {
        let (p, y) = look_angles(Vec3::ZERO, Vec3::new(0.0, 10.0, 0.0));
        assert!((y - 90.0).abs() < 1e-4 && p.abs() < 1e-4);
        let (p, _) = look_angles(Vec3::ZERO, Vec3::new(10.0, 0.0, -10.0));
        assert!((p - 45.0).abs() < 1e-3, "looking down is positive pitch: {p}");
    }

    #[test]
    fn approach_wraps() {
        assert_eq!(approach(350.0, 10.0, 5.0), 355.0);
        assert!((approach(10.0, 350.0, 5.0) - 5.0).abs() < 1e-4);
    }
}
