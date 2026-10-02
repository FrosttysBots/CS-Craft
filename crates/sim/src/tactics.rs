//! Team knowledge is built from reports and audible shots, never hidden enemy positions.
use crate::{GameEvent, Player, PlayerId, Team, World};
use glam::Vec3;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Economy {
    Pistol,
    Full,
    Force,
    Eco,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Entry,
    Support,
    Carrier,
    Lurk,
    Anchor,
    Rotator,
}
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    pub team: Team,
    pub enemy: PlayerId,
    pub pos: Vec3,
    pub observed: f32,
    pub available: f32,
    pub visual: bool,
}
#[derive(Clone, Debug)]
pub struct Plan {
    pub economy: Economy,
    pub site: usize,
    pub execute_at: f32,
    pub rotated: bool,
    pub staging: Option<Vec3>,
    pub executing: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Coordinator {
    round: u32,
    pub plans: HashMap<Team, Plan>,
    pub roles: HashMap<PlayerId, Role>,
    pub reserve: HashMap<PlayerId, i32>,
    pub contacts: Vec<Contact>,
    pub defuser: Option<PlayerId>,
    next_plan: f32,
    last_tick: Option<u64>,
    pub utility_after: HashMap<Team, f32>,
}
impl Coordinator {
    pub fn update(&mut self, world: &World, mesh: Option<&nav::NavMesh>) {
        let now = world.time();
        if self.plans.is_empty() || self.round != world.round.round {
            *self = Self::default();
            self.round = world.round.round;
            for team in [Team::Terrorist, Team::CounterTerrorist] {
                let players: Vec<_> = world.players.iter().filter(|p| p.team == team).collect();
                let ct = team == Team::CounterTerrorist;
                let threshold = if ct { 4700 } else { 4200 };
                let ready = players
                    .iter()
                    .filter(|p| {
                        p.money
                            + if p.weapon_in_slot(1).is_some() {
                                3000
                            } else {
                                0
                            }
                            >= threshold
                    })
                    .count();
                let forced = world.round.score_t.max(world.round.score_ct) + 1
                    >= world.rules.rounds_to_win()
                    || (world.rules.halftime && world.round.round == world.rules.max_rounds / 2);
                let economy = if world.round.is_pistol_round(&world.rules) {
                    Economy::Pistol
                } else if ready * 2 >= players.len().max(1) {
                    Economy::Full
                } else if forced {
                    Economy::Force
                } else {
                    Economy::Eco
                };
                let site = crate::rng::Rng::from_parts(&[world.round.round as u64, 0x517E])
                    .next_u64() as usize
                    % world.objectives.bomb_sites.len().max(1);
                let staging = if !ct {
                    mesh.and_then(|m| {
                        let start = m.nearest_area(players.first()?.movement.origin)?;
                        let target = world.objectives.bomb_sites.get(site)?.zone.center();
                        let goal = m.nearest_area(target)?;
                        m.find_path(start.id, goal.id)?
                            .into_iter()
                            .rev()
                            .filter_map(|id| m.area(id))
                            .find(|a| a.center().distance(target) > 650.0)
                            .map(|a| a.center())
                    })
                } else {
                    None
                };
                self.plans.insert(
                    team,
                    Plan {
                        economy,
                        site,
                        execute_at: world.round.live_start + 25.0,
                        rotated: false,
                        staging,
                        executing: false,
                    },
                );
                for (rank, p) in players.iter().enumerate() {
                    self.roles.insert(
                        p.id,
                        if ct {
                            if rank == players.len().saturating_sub(1) {
                                Role::Rotator
                            } else {
                                Role::Anchor
                            }
                        } else if p.has_bomb() {
                            Role::Carrier
                        } else if rank == 0 {
                            Role::Entry
                        } else if rank == players.len().saturating_sub(1) {
                            Role::Lurk
                        } else {
                            Role::Support
                        },
                    );
                    // Preserve enough for next round's rifle/armour after a minimum loss bonus.
                    self.reserve.insert(
                        p.id,
                        if economy == Economy::Eco {
                            (threshold - world.rules.loser_bonus).min(p.money)
                        } else {
                            0
                        },
                    );
                }
            }
        }
        self.contacts
            .retain(|c| now - c.observed < if c.visual { 7.0 } else { 3.0 });
        if self.last_tick != Some(world.tick) {
            self.last_tick = Some(world.tick);
            for e in &world.events {
                if let GameEvent::Fire { player, origin, .. }
                | GameEvent::DecoyShot {
                    player,
                    pos: origin,
                } = e
                {
                    let Some(shooter) = world.player(*player) else {
                        continue;
                    };
                    for team in [Team::Terrorist, Team::CounterTerrorist]
                        .into_iter()
                        .filter(|t| t.enemy_of(shooter.team))
                    {
                        let heard = world.players.iter().any(|p| {
                            p.alive
                                && p.team == team
                                && p.eye().distance(*origin)
                                    < if world.visible(p.eye(), *origin) {
                                        1800.0
                                    } else {
                                        1000.0
                                    }
                        });
                        if heard {
                            // Audio supplies a coarse location, never a target suitable for shooting.
                            self.report(Contact {
                                team,
                                enemy: *player,
                                pos: (*origin / 96.0).round() * 96.0,
                                observed: now,
                                available: now + 0.35,
                                visual: false,
                            });
                        }
                    }
                }
            }
        }
        if now < self.next_plan {
            return;
        }
        self.next_plan = now + 0.5;
        if let Some(plan) = self.plans.get_mut(&Team::Terrorist) {
            if !plan.executing {
                let alive = world
                    .players
                    .iter()
                    .filter(|p| p.alive && p.team == Team::Terrorist)
                    .count();
                let ready = plan.staging.map_or(alive, |pos| {
                    world
                        .players
                        .iter()
                        .filter(|p| {
                            p.alive
                                && p.team == Team::Terrorist
                                && p.movement.origin.distance(pos) < 380.0
                        })
                        .count()
                });
                if ready >= alive.min(3) || now >= plan.execute_at {
                    plan.executing = true;
                }
            }
        }
        self.defuser = None;
        if let Some(b) = world.bomb {
            self.defuser = b
                .defuser
                .map(|d| d.0)
                .filter(|id| world.player(*id).is_some_and(|p| p.alive))
                .or_else(|| {
                    world
                        .players
                        .iter()
                        .filter(|p| p.alive && p.team == Team::CounterTerrorist)
                        .filter_map(|p| {
                            travel_time(mesh, p.movement.origin, b.pos)
                                .map(|t| (p.id, t + world.rules.defuse_duration(p.defuse_kit)))
                        })
                        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
                        .map(|p| p.0)
                });
        } else if world.objectives.bomb_sites.len() > 1 {
            let plan = self.plans.get_mut(&Team::Terrorist).unwrap();
            let target = world.objectives.bomb_sites[plan.site].zone.center();
            let stack = self
                .contacts
                .iter()
                .filter(|c| {
                    c.team == Team::Terrorist
                        && c.visual
                        && c.available <= now
                        && now - c.observed < 4.0
                        && c.pos.distance(target) < 850.0
                })
                .count();
            let carrier_near = world
                .players
                .iter()
                .any(|p| p.alive && p.has_bomb() && p.movement.origin.distance(target) < 650.0);
            if !plan.rotated && !carrier_near && stack >= 3 && now - world.round.live_start < 55.0 {
                plan.site = (plan.site + 1) % world.objectives.bomb_sites.len();
                plan.rotated = true;
                plan.executing = true;
            }
        }
    }
    pub fn report(&mut self, c: Contact) {
        if let Some(old) = self
            .contacts
            .iter_mut()
            .find(|o| o.team == c.team && o.enemy == c.enemy)
        {
            if !c.visual && old.visual && c.observed - old.observed < 1.0 {
                return;
            }
            // Repeated reports must not continually postpone the original radio delay.
            let available = old.available.min(c.available);
            *old = c;
            old.available = available;
        } else {
            self.contacts.push(c)
        }
    }
    pub fn contact(&self, team: Team, now: f32, near: Vec3) -> Option<Contact> {
        self.contacts
            .iter()
            .filter(|c| c.team == team && c.available <= now)
            .min_by(|a, b| {
                (a.pos.distance(near) + (now - a.observed) * 180.0)
                    .total_cmp(&(b.pos.distance(near) + (now - b.observed) * 180.0))
            })
            .copied()
    }
    pub fn role(&self, p: &Player) -> Role {
        if p.has_bomb() {
            Role::Carrier
        } else {
            self.roles.get(&p.id).copied().unwrap_or(Role::Entry)
        }
    }
}
pub fn travel_time(mesh: Option<&nav::NavMesh>, from: Vec3, to: Vec3) -> Option<f32> {
    let Some(mesh) = mesh else {
        return Some(from.distance(to) / 220.0);
    };
    let a = mesh.nearest_area(from)?;
    let b = mesh.nearest_area(to)?;
    let path = mesh.find_path(a.id, b.id)?;
    let mut previous = from;
    let mut distance = 0.0;
    for id in path.into_iter().skip(1) {
        let p = mesh.area(id)?.center();
        distance += previous.distance(p);
        previous = p;
    }
    Some((distance + previous.distance(to)) / 220.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_are_delayed_and_audio_cannot_replace_fresh_sight() {
        let mut c = Coordinator::default();
        c.report(Contact {
            team: Team::Terrorist,
            enemy: 8,
            pos: Vec3::X,
            observed: 1.0,
            available: 1.3,
            visual: true,
        });
        assert!(c.contact(Team::Terrorist, 1.2, Vec3::ZERO).is_none());
        c.report(Contact {
            team: Team::Terrorist,
            enemy: 8,
            pos: Vec3::Y,
            observed: 1.1,
            available: 1.4,
            visual: false,
        });
        assert_eq!(
            c.contact(Team::Terrorist, 1.4, Vec3::ZERO).unwrap().pos,
            Vec3::X
        );
        assert!(c.contact(Team::CounterTerrorist, 2.0, Vec3::ZERO).is_none());
    }
}
