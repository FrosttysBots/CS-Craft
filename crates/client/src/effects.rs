//! Turns simulation events into feedback: tracers, impact marks, sounds,
//! the kill feed and a hit marker.

use crate::audio::SoundLibrary;
use crate::coords::to_bevy;
use crate::player::{LocalPlayer, SimWorld};
use crate::LoadedLevel;
use bevy::audio::AudioSource;
use bevy::prelude::*;
use sim::GameEvent;

/// Events produced by fixed ticks since the last frame.
#[derive(Resource, Default)]
pub struct PendingEvents(pub Vec<GameEvent>);

#[derive(Clone, Debug)]
pub struct KillFeedEntry {
    pub text: String,
    pub local_involved: bool,
    pub time: f32,
}

#[derive(Resource, Default)]
pub struct Effects {
    tracers: Vec<(Vec3, Vec3, f32)>,
    impacts: Vec<(Vec3, Vec3, f32)>,
    pub kill_feed: Vec<KillFeedEntry>,
    /// Time of the local player's last hit on someone (hit marker).
    pub last_hit: f32,
    /// Last time each player fired (drives fire animations).
    pub last_fire: std::collections::HashMap<sim::PlayerId, f32>,
    pub last_hit_headshot: bool,
}

const TRACER_LIFE: f32 = 0.06;
const IMPACT_LIFE: f32 = 4.0;
const MAX_IMPACTS: usize = 256;

#[allow(clippy::too_many_arguments)]
pub fn process(
    mut commands: Commands,
    mut pending: ResMut<PendingEvents>,
    mut effects: ResMut<Effects>,
    mut sounds: ResMut<SoundLibrary>,
    mut sources: ResMut<Assets<AudioSource>>,
    world: Res<SimWorld>,
    local: Res<LocalPlayer>,
    level: Res<LoadedLevel>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs();
    for event in pending.0.drain(..) {
        match event {
            GameEvent::Fire { player, weapon, origin } => {
                effects.last_fire.insert(player, now);
                let sound = world.0.catalog.get(&weapon).map(|d| d.fire_sound.clone()).unwrap_or_default();
                if !sound.is_empty() {
                    let at = if player == local.id && !local.noclip { None } else { Some(origin) };
                    sounds.play(&mut commands, &level.0.vfs, &mut sources, &sound, at);
                }
            }
            GameEvent::Tracer { start, end } => {
                // Start tracers a little ahead of the eye so they don't fill the screen.
                let dir = (end - start).normalize_or_zero();
                let s = start + dir * 40.0 - glam::Vec3::Z * 6.0;
                effects.tracers.push((to_bevy(s), to_bevy(end), now));
            }
            GameEvent::Impact { pos, normal, .. } => {
                effects.impacts.push((to_bevy(pos), to_bevy(normal), now));
                if effects.impacts.len() > MAX_IMPACTS {
                    effects.impacts.remove(0);
                }
            }
            GameEvent::Damage { attacker, group, .. } => {
                if attacker == local.id {
                    effects.last_hit = now;
                    effects.last_hit_headshot = group == sim::weapon::HitGroup::Head;
                }
            }
            GameEvent::Kill { attacker, victim, weapon, headshot } => {
                let name = |id| world.0.player(id).map_or("?".to_string(), |p| p.name.clone());
                let gun = weapon.trim_start_matches("weapon_");
                effects.kill_feed.push(KillFeedEntry {
                    text: format!("{}  [{}{}]  {}", name(attacker), gun, if headshot { " HS" } else { "" }, name(victim)),
                    local_involved: attacker == local.id || victim == local.id,
                    time: now,
                });
                if effects.kill_feed.len() > 6 {
                    effects.kill_feed.remove(0);
                }
            }
            _ => {}
        }
    }
    effects.tracers.retain(|t| now - t.2 < TRACER_LIFE);
    effects.impacts.retain(|t| now - t.2 < IMPACT_LIFE);
    effects.kill_feed.retain(|k| now - k.time < 6.0);
}

pub fn draw(effects: Res<Effects>, time: Res<Time>, mut gizmos: Gizmos) {
    let now = time.elapsed_secs();
    for (s, e, _) in &effects.tracers {
        gizmos.line(*s, *e, Color::srgba(1.0, 0.9, 0.6, 0.8));
    }
    for (p, n, t) in &effects.impacts {
        let fade = 1.0 - (now - t) / IMPACT_LIFE;
        let c = Color::srgba(0.1, 0.08, 0.06, 0.9 * fade);
        // Small cross lying on the surface.
        let n = n.normalize_or_zero();
        let a = if n.y.abs() < 0.9 { n.cross(Vec3::Y) } else { n.cross(Vec3::X) }.normalize_or_zero() * 2.0;
        let b = n.cross(a).normalize_or_zero() * 2.0;
        let p = *p + n * 0.3;
        gizmos.line(p - a, p + a, c);
        gizmos.line(p - b, p + b, c);
    }
}
