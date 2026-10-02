//! Bots: produce [`TickInput`]s for AI players, so bots obey exactly the
//! same movement, weapon, buying and objective rules as humans.
//!
//! * Navigation: with the map's nav mesh, bots follow A* paths through area
//!   portals, jump/crouch where the mesh says so, and re-path when stuck.
//!   Without a mesh they wander and turn away from walls.
//! * Combat: perception runs ~8 times a second per bot (staggered); on
//!   seeing an enemy a bot reacts after a delay, turns at a limited rate
//!   with aim error, stops and fires controlled bursts.
//! * Deathmatch: bots roam between hot places (bombsites, main routes).
//! * Competitive: bots buy in the buy zone (rifle + armour when they can
//!   afford it, save otherwise, armour or a pistol on pistol rounds, a kit
//!   for CTs). Terrorists pick a site each round; the carrier walks there
//!   and plants, the others escort, and the nearest one fetches a dropped
//!   bomb. Counter-Terrorists split between the sites and hold them, and
//!   everyone converges on a planted bomb: CTs to defuse, Ts to guard.

use crate::objectives::Zone;
use crate::rng::Rng;
use crate::rules::{Phase, Purchase};
use crate::{GameMode, Player, PlayerId, Team, TickInput, World};
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
    aim_error: glam::Vec2,
    aim_goal: glam::Vec2,
    next_aim_change: f32,
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
    /// Round the bot finished buying in, and failed attempts this round.
    bought_round: u32,
    buy_attempts: u32,
    /// The area the current path leads to, when following an objective.
    goal_area: Option<u32>,
    /// (site index, chosen area) for site objectives.
    site_area: Option<(usize, u32)>,
    /// When to pick a new spot while holding a site.
    repick_at: f32,
    hold: Option<(Vec3, u32, Vec3, Vec3)>, // objective, area, position, angle
    visible: bool,
    turn_velocity: glam::Vec2,
    escape_until: f32,
    escape_side: f32,
    last_shots: u64,
    reload_cover: Option<(u32, Vec3)>,
    throw: Option<(u32, Vec3, f32)>,
    next_utility: f32,
    last_health: i32,
    hurt_at: f32,

}

impl Brain {
    fn fresh(bot: &Player, bought_round: u32) -> Self {
        Brain {
            yaw: bot.view_angles.y,
            wander_yaw: bot.view_angles.y,
            last_pos: bot.movement.origin,
            bought_round,
            last_shots: bot.shots,
            last_health: bot.health,
            hurt_at: -10.0,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug)]
pub struct Bots {
    brains: HashMap<PlayerId, Brain>,
    /// 0 (harmless) .. 1 (sharp).
    pub skill: f32,
    /// If false, bots stand still and never shoot (target practice).
    pub aggressive: bool,
    pub nav: Option<Arc<NavMesh>>,
    /// Nav areas inside each bomb site (computed on first use).
    site_areas: Vec<Vec<u32>>,
    team: crate::tactics::Coordinator,
}

impl Default for Bots {
    fn default() -> Self {
        Self {
            brains: HashMap::new(),
            skill: 0.5,
            aggressive: true,
            nav: None,
            site_areas: Vec::new(),
            team: Default::default(),
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

/// Nav areas whose centre lies inside a bomb site's trigger volume.
fn areas_in(nav: &NavMesh, zone: &Zone) -> Vec<u32> {
    let mut v: Vec<u32> = nav
        .areas
        .iter()
        .filter(|a| {
            let c = a.center();
            zone.contains(Vec3::new(c.x, c.y, (c.z + 8.0).clamp(zone.mins.z, zone.maxs.z))) && c.z > zone.mins.z - 32.0 && c.z < zone.maxs.z
        })
        .map(|a| a.id)
        .collect();
    if v.is_empty() {
        v.extend(nav.nearest_area(zone.center()).map(|a| a.id));
    }
    v
}

/// What a bot is trying to do right now (besides fighting).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Task {
    /// Roam (deathmatch, or nothing better to do).
    Roam,
    /// Walk to a bomb site and hold a spot there.
    HoldSite(usize),
    /// Carry the bomb to a site and plant it.
    Plant(usize),
    /// Walk to a point (a dropped bomb, a planted bomb to guard).
    GoTo(Vec3),
    /// Walk to the planted bomb and defuse it.
    Defuse(Vec3),
    Guard(Vec3),
}

/// The best gun slot a bot has: primary, else pistol.
fn gun_slot(bot: &Player) -> u8 {
    if bot.weapon_in_slot(1).is_some() { 1 } else { 2 }
}

/// The next thing a bot buys, given what it has (one item per tick).
fn next_purchase(world: &World, bot: &Player, team: &crate::tactics::Coordinator) -> Option<Purchase> {
    use crate::tactics::Economy;
    let money=bot.money-team.reserve.get(&bot.id).copied().unwrap_or(0);
    let economy=team.plans.get(&bot.team).map_or(Economy::Full,|p|p.economy);
    let ct=bot.team==Team::CounterTerrorist;
    let rank=world.players.iter().filter(|p|p.team==bot.team&&p.id<bot.id).count();
    let weapon=|name:&str|world.catalog.get(name).filter(|d|!bot.has(name)&&d.price<=money).map(|d|Purchase::Weapon(d.def_index));
    if economy==Economy::Pistol {
        if rank<3 && bot.armor==0 && money>=650 {return Some(Purchase::Kevlar)}
        if ct && rank==3 && !bot.defuse_kit && money>=400 {return Some(Purchase::DefuseKit)}
    } else if bot.weapon_in_slot(1).is_none() {
        if economy==Economy::Eco {return weapon("weapon_p250")}
        let sniper=rank==1 && !world.players.iter().any(|p|p.team==bot.team&&p.has("weapon_awp"));
        let choices=if ct {[if sniper {"weapon_awp"}else{"weapon_m4a1_silencer"},"weapon_m4a1_silencer","weapon_famas","weapon_mp9"]}
            else {[if sniper {"weapon_awp"}else{"weapon_ak47"},"weapon_ak47","weapon_galilar","weapon_mac10"]};
        for name in choices {if let Some(d)=world.catalog.get(name) {if d.price+if bot.armor<50{650}else{0}<=money {return Some(Purchase::Weapon(d.def_index))}}}
        if economy==Economy::Force {return weapon("weapon_deagle").or_else(||weapon("weapon_p250"))}
        return None;
    }
    if economy==Economy::Eco {return None}
    if economy!=Economy::Pistol {
        if bot.armor<50 && money>=650 {return Some(Purchase::Kevlar)}
        if !bot.helmet && money>=350 {return Some(Purchase::KevlarHelmet)}
        if ct && rank%2==0 && !bot.defuse_kit && money>=400 {return Some(Purchase::DefuseKit)}
    }
    if bot.weapons.iter().filter(|w|w.def.class==weapons::WeaponClass::Grenade).count()<4 {
        for name in ["weapon_smokegrenade","weapon_flashbang",if ct{"weapon_incgrenade"}else{"weapon_molotov"},"weapon_hegrenade"] {
            if let Some(p)=weapon(name) {return Some(p)}
        }
    }
    None
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

    /// A one-line description of a bot's state, for debugging.
    pub fn describe(&self, world: &World, id: PlayerId) -> String {
        let (Some(b), Some(p)) = (self.brains.get(&id), world.player(id)) else { return String::new() };
        format!(
            "task {:?} target {:?} path {}/{} goal {:?} site {:?} role {:?} economy {:?}",
            Self::task(world, p),
            b.target,
            b.path_index,
            b.path.len(),
            b.goal_area,
            b.site_area, self.team.role(p), self.team.plans.get(&p.team).map(|p|p.economy)
        )
    }

    /// The Terrorists' target site this round.
    fn t_site(world: &World) -> usize {
        let n = world.objectives.bomb_sites.len().max(1);
        (Rng::from_parts(&[world.round.round as u64, 0x517E]).next_u64() % n as u64) as usize
    }

    fn task(world: &World, bot: &Player) -> Task {
        if world.mode != GameMode::Competitive || world.objectives.bomb_sites.is_empty() || world.round.phase != Phase::Live {
            return Task::Roam;
        }
        if let Some(b) = world.bomb {
            // Keep an active defuser, otherwise choose one reachable teammate
            // by estimated travel + kit duration. The others cover approaches.
            let defuser = b.defuser.map(|d| d.0).filter(|id| world.player(*id).is_some_and(|p| p.alive))
                .or_else(|| world.players.iter().filter(|p| p.alive && p.team == Team::CounterTerrorist)
                    .min_by(|a, c| defuse_cost(a, b.pos).total_cmp(&defuse_cost(c, b.pos)).then(a.id.cmp(&c.id))).map(|p| p.id));
            return if defuser == Some(bot.id) { Task::Defuse(b.pos) } else { Task::Guard(b.pos) };
        }
        let sites = world.objectives.bomb_sites.len();
        match bot.team {
            Team::Terrorist => {
                if bot.has_bomb() {
                    return Task::Plant(Self::t_site(world));
                }
                if let Some(item) = world.dropped_bomb() {
                    // The nearest living Terrorist fetches it.
                    let nearest = world
                        .players
                        .iter()
                        .filter(|p| p.alive && p.team == Team::Terrorist)
                        .min_by(|a, b| a.movement.origin.distance(item.pos).total_cmp(&b.movement.origin.distance(item.pos)))
                        .map(|p| p.id);
                    if nearest == Some(bot.id) {
                        return Task::GoTo(item.pos);
                    }
                }
                Task::HoldSite(Self::t_site(world))
            }
            Team::CounterTerrorist => {
                let rank = world.players.iter().filter(|p| p.team == Team::CounterTerrorist && p.id < bot.id).count();
                Task::HoldSite((rank + world.round.round as usize) % sites)
            }
            Team::Spectator => Task::Roam,
        }
    }

    fn planned_task(&self, world: &World, bot: &Player) -> Task {
        use crate::tactics::Role;
        let fallback=Self::task(world,bot);
        if world.mode!=GameMode::Competitive || world.round.phase!=Phase::Live || world.objectives.bomb_sites.is_empty() {return fallback}
        let now=world.time();
        let role=self.team.role(bot);
        if let Some(b)=world.bomb {
            if bot.team==Team::CounterTerrorist {
                // Don't throw away saved equipment when no living CT can finish in time.
                let duration=world.rules.defuse_duration(bot.defuse_kit);
                if b.defuser.is_none() && b.explode_at-now<duration+bot.movement.origin.distance(b.pos)/250.0+0.25 {
                    if let Some(spawn)=world.spawns.iter().filter(|s|s.team==bot.team).max_by(|a,c|a.origin.distance_squared(b.pos).total_cmp(&c.origin.distance_squared(b.pos))) {return Task::GoTo(spawn.origin)}
                }
                if self.team.defuser==Some(bot.id) {return Task::Defuse(b.pos)}
                // Reach the site before taking a covering angle; don't camp outside a retake.
                if bot.movement.origin.distance(b.pos)>420.0 {return Task::GoTo(b.pos)}
            }
            return Task::Guard(b.pos)
        }
        let plan=&self.team.plans[&bot.team];
        if bot.team==Team::Terrorist {
            if matches!(fallback,Task::GoTo(_)) {return fallback}
            let site=plan.site;
            if !plan.executing && role!=Role::Lurk {
                if let Some(stage)=plan.staging {return Task::GoTo(stage+Vec3::new((bot.id%3) as f32*32.0-32.0,0.0,0.0))}
            }
            if bot.has_bomb() {return Task::Plant(site)}
            // Support follows the carrier along the actual route and can trade an entry.
            if role==Role::Support {
                if let Some(carrier)=world.players.iter().find(|p|p.alive&&p.has_bomb()) {
                    if carrier.movement.origin.distance(bot.movement.origin)>280.0 {return Task::GoTo(carrier.movement.origin)}
                }
            }
            if role==Role::Lurk && now<plan.execute_at+12.0 && world.objectives.bomb_sites.len()>1 {return Task::HoldSite((site+1)%world.objectives.bomb_sites.len())}
            Task::HoldSite(site)
        } else {
            if let Some(c)=self.team.contact(bot.team,now,bot.movement.origin) {
                if role==Role::Rotator || now-world.round.live_start>28.0 {
                    let site=world.objectives.bomb_sites.iter().enumerate().min_by(|a,b|a.1.zone.center().distance_squared(c.pos).total_cmp(&b.1.zone.center().distance_squared(c.pos))).map(|s|s.0).unwrap_or(0);
                    if c.pos.distance(world.objectives.bomb_sites[site].zone.center())<1100.0 {return Task::HoldSite(site)}
                }
            }
            fallback
        }
    }

    pub fn think(&mut self, world: &World) -> Vec<(PlayerId, TickInput)> {
        if world.mode == GameMode::Special { return Vec::new(); }
        let time = world.time();
        let dt = crate::TICK_DT;
        let nav = self.nav.clone();
        self.team.update(world,nav.as_deref());
        if let (Some(mesh), true) = (nav.as_deref(), self.site_areas.len() != world.objectives.bomb_sites.len()) {
            self.site_areas = world.objectives.bomb_sites.iter().map(|s| areas_in(mesh, &s.zone)).collect();
        }
        let competitive = world.mode == GameMode::Competitive;
        let frozen = competitive && world.round.phase == Phase::Freeze;
        let t_side: Vec3 = average(world.spawns.iter().filter(|s| s.team == Team::Terrorist).map(|s| s.origin));
        let ct_side: Vec3 = average(world.spawns.iter().filter(|s| s.team == Team::CounterTerrorist).map(|s| s.origin));
        let reservations: Vec<_> = self.brains.iter().filter_map(|(id, b)| {
            let p = world.player(*id).filter(|p| p.alive)?;
            Some((*id, p.team, b.hold?.2))
        }).collect();
        let mut out = Vec::new();
        for bot in world.players.iter().filter(|p| p.is_bot) {
            let task=self.planned_task(world,bot);
            let contact=self.team.contact(bot.team,time,bot.movement.origin);
            let purchase=next_purchase(world,bot,&self.team);
            let brain = self.brains.entry(bot.id).or_insert_with(|| Brain::fresh(bot, 0));
            if !bot.alive {
                // Forget everything; start fresh after respawning.
                *brain = Brain::fresh(bot, 0);
                continue;
            }
            if brain.progress_time == 0.0 {
                brain.progress_time = time;
                brain.progress_pos = bot.movement.origin;
                brain.yaw = bot.view_angles.y;
            }
            let mut rng = Rng::from_parts(&[world.tick, bot.id as u64, 0xB07]);
            let eye = bot.eye();
            if bot.health<brain.last_health {brain.hurt_at=time;}
            brain.last_health=bot.health;
            let mut input = TickInput {
                weapon_slot: gun_slot(bot),
                ..Default::default()
            };

            // Buying, one item per tick, while allowed.
            if competitive && bot.in_buy_zone && brain.bought_round != world.round.round && world.round.can_buy(&world.rules, time) {
                match purchase {
                    Some(item) if brain.buy_attempts < 12 => {
                        input.buy = item;
                        brain.buy_attempts += 1;
                    }
                    _ => {
                        brain.bought_round = world.round.round;
                        brain.buy_attempts = 0;
                    }
                }
            }
            if frozen {
                // New round: drop the old path and objectives.
                brain.path.clear();
                brain.goal_area = None;
                brain.site_area = None;
                brain.hold = None;
                brain.turn_velocity = glam::Vec2::ZERO;
                brain.target = None;
                brain.yaw = bot.view_angles.y;
                brain.pitch = 0.0;
                brain.progress_time = time;
                brain.progress_pos = bot.movement.origin;
                input.view_angles = Vec3::new(0.0, brain.yaw, 0.0);
                out.push((bot.id, input));
                continue;
            }

            if bot.active_weapon().is_none_or(|w| w.reload_end.is_none()) { brain.reload_cover = None; }
            if bot.shots > brain.last_shots {
                 brain.burst_left = brain.burst_left.saturating_sub((bot.shots - brain.last_shots) as u32);
                 if brain.burst_left == 0 { brain.next_burst = time + 0.25 + rng.next_f32() * 0.3; }
             }
            brain.last_shots = bot.shots;
            let old_angles = glam::Vec2::new(brain.pitch, brain.yaw);
            // Perception (throttled and staggered).
            if self.aggressive && world.flash_amount(bot.id)<0.25 && (world.tick + bot.id as u64) % PERCEPTION_INTERVAL == 0 {
                let mut best: Option<(PlayerId, Vec3, f32)> = None;
                let (fwd, _, _) = movement::angle_vectors(Vec3::new(0.0, brain.yaw, 0.0));
                for other in world.players.iter().filter(|o| o.alive && bot.team.enemy_of(o.team)) {
                    let chest = other.movement.origin + Vec3::new(0.0, 0.0, other.movement.view_offset() - 12.0);
                    let to = chest - eye;
                    let dist = to.length();
                    let score=dist*if brain.target==Some(other.id){0.7}else{1.0};
                    if dist > 3500.0 || best.is_some_and(|(_, _, d)| d < score) {
                        continue;
                    }
                    // Field of view ~140°, but notice anyone very close.
                    let facing = to.truncate().normalize_or_zero().dot(fwd.truncate());
                    if facing < 0.342 && dist > 160.0 {
                        continue;
                    }
                    if let Some(spot) = visible_target(world, eye, other) {
                        best = Some((other.id, spot, score));
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
                    None => {},
                }
            }
            // A single tracked target gets a current LOS check. Never update
            // its remembered position through walls or fire at stale visibility.
            brain.visible = false;
            if let Some(t) = brain.target.and_then(|id| world.player(id)) {
                if !t.alive { brain.target = None; }
                else if let Some(spot) = (world.flash_amount(bot.id)<0.25).then(|| visible_target(world, eye, t)).flatten() {
                    brain.target_pos = spot;
                    brain.last_seen = time;
                    brain.visible = true;
                } else if time - brain.last_seen > 1.25 { brain.target = None; }
            }

            if brain.visible {
                self.team.report(crate::tactics::Contact { team:bot.team,enemy:brain.target.unwrap(),pos:brain.target_pos,observed:time,available:time+0.3,visual:true });
            }
            // A defuse close to done is worth finishing under fire.
            let finishing = world.bomb.and_then(|b| b.defuser).is_some_and(|(d, done)| d == bot.id && (done - time < 2.5 || world.smoke_occludes(eye,brain.target_pos) || world.bomb.is_some_and(|b|b.explode_at-done<2.0)));
            let turn_rate = (240.0 + 480.0 * self.skill) * dt;
            if brain.visible && !finishing {
                let err = (1.0 - self.skill) * 3.0;
                let (mut tp, mut ty) = look_angles(eye, brain.target_pos);
                // Slowly drifting error preserves imperfect aim without a new
                // camera shake every 1/64 second when this bot is spectated.
                if time >= brain.next_aim_change {
                    brain.aim_goal = glam::Vec2::new(rng.range(-err, err), rng.range(-err, err));
                    brain.next_aim_change = time + 0.25;
                }
                brain.aim_error = brain.aim_error.lerp(brain.aim_goal, 1.0 - (-8.0 * dt).exp());
                let punch=bot.active_weapon().map_or([0.0;2],|w|w.aim_punch());
                tp -= punch[0]*crate::weapon::RECOIL_SCALE*self.skill;
                ty -= punch[1]*crate::weapon::RECOIL_SCALE*self.skill;
                tp += brain.aim_error.x;
                ty += brain.aim_error.y;
                brain.pitch = approach(brain.pitch, tp, turn_rate);
                brain.yaw = approach(brain.yaw, ty, turn_rate);
                let on_target = angle_diff(ty, brain.yaw).abs() < 2.5 && angle_diff(tp, brain.pitch).abs() < 2.5;
                let reaction = 0.45 - 0.3 * self.skill;
                if on_target && brain.visible && time - brain.seen_at > reaction {
                    if brain.burst_left == 0 && time >= brain.next_burst {
                        brain.burst_left = if eye.distance(brain.target_pos)>1000.0 {2} else if eye.distance(brain.target_pos)<250.0 {7} else {3};
                    }
                    if brain.burst_left > 0 {
                        input.buttons |= buttons::ATTACK;
                        // A burst counts actual shots, not simulation ticks.
                        if let Some(w) = bot.active_weapon() {
                            if !w.def.full_auto && bot.last_input.buttons & buttons::ATTACK != 0 {
                                input.buttons &= !buttons::ATTACK;
                            }
                        }
                    }
                } else if brain.visible {
                    if time >= brain.next_strafe_change {
                        brain.strafe = if rng.next_f32() < 0.5 { -450.0 } else { 450.0 };
                        brain.next_strafe_change = time + 0.3 + rng.next_f32() * 0.5;
                    }
                    let (_, right, _) = movement::angle_vectors(Vec3::new(0.0, brain.yaw, 0.0));
                    if can_walk(world, bot, right * brain.strafe.signum() * 48.0) {
                        input.side_move = brain.strafe * 0.45;
                    }
                }
                if let Some(w) = bot.active_weapon() {
                    if w.zoom_levels() > 0 && w.zoom_level == 0 && brain.visible && eye.distance(brain.target_pos) > 450.0
                        && bot.last_input.buttons & buttons::ATTACK2 == 0 {
                        input.buttons |= buttons::ATTACK2;
                        input.buttons &= !buttons::ATTACK;
                    }
                    if w.reload_end.is_some() || (w.clip < (w.def.clip_size / 4).max(1) && time - brain.last_seen > 0.4) {
                        if w.can_reload() { input.buttons |= buttons::RELOAD; }
                        input.buttons &= !buttons::ATTACK;
                        if let Some(mesh) = nav.as_deref() {
                            if brain.reload_cover.is_none() {
                                brain.reload_cover = reload_cover(mesh, world, bot, brain.target_pos);
                            }
                            if let Some((area, point)) = brain.reload_cover {
                                follow_path(mesh, world, bot, brain, &mut input, &mut rng, time, turn_rate, Some(area), Some(point));
                            }
                        }
                    }
                }
            } else if self.aggressive {
                let (mn, mx) = bot.bounds();
                match task {
                    Task::Plant(site) if world.objectives.site_at(mn, mx) == Some(site) || (world.objectives.site_at(mn, mx).is_some() && bot.plant_start.is_some()) => {
                        input.weapon_slot = 5;
                        input.buttons |= buttons::ATTACK;
                        brain.pitch = approach(brain.pitch, 30.0, turn_rate);
                    }
                    Task::Defuse(pos) if (pos - bot.movement.origin).truncate().length() < 40.0 => {
                        let (p, y) = look_angles(eye, pos);
                        brain.pitch = approach(brain.pitch, p, turn_rate);
                        brain.yaw = approach(brain.yaw, y, turn_rate);
                        input.buttons |= buttons::USE | buttons::DUCK;
                    }
                    _ => match nav.as_deref() {
                        Some(mesh) => {
                            let (goal_area, point, hold_dir) = match task {
                                Task::Roam => (None, None, None),
                                Task::Plant(site) => {
                                    let areas = self.site_areas.get(site).cloned().unwrap_or_default();
                                    if brain.site_area.map(|s| s.0) != Some(site) || (matches!(task, Task::HoldSite(_)) && time > brain.repick_at && brain.repick_at > 0.0) {
                                        if !areas.is_empty() {
                                            brain.site_area = Some((site, areas[(rng.next_u64() as usize) % areas.len()]));
                                            brain.repick_at = 0.0;
                                        }
                                    }
                                    let face = if bot.team == Team::Terrorist { ct_side } else { t_side };
                                    (brain.site_area.map(|s| s.1), None, Some(face))
                                }
                                Task::HoldSite(_) | Task::Guard(_) => {
                                    let anchor = match task {
                                        Task::HoldSite(site) => world.objectives.bomb_sites[site].zone.center(),
                                        Task::Guard(pos) => pos,
                                        _ => unreachable!(),
                                    };
                                    let enemy_side = if bot.team == Team::Terrorist { ct_side } else { t_side };
                                    if brain.hold.is_none_or(|h| h.0.distance(anchor) > 48.0) || (brain.repick_at > 0.0 && time > brain.repick_at) {
                                        brain.hold = tactical_spot(mesh, world, bot, anchor, enemy_side, &reservations)
                                            .map(|(area, point, look)| (anchor, area, point, look));
                                        let covered=brain.hold.is_some();
                                        // Cache a reachable fallback too. An occluded bomb must not
                                        // trigger a full cover/LOS search every simulation tick.
                                        if !covered {brain.hold=mesh.nearest_area(anchor).map(|a|(anchor,a.id,a.center(),enemy_side));}
                                        brain.repick_at = time + if covered {12.0 + (bot.id % 5) as f32}else{3.0};
                                    }
                                    brain.hold.map_or((mesh.nearest_area(anchor).map(|a| a.id), Some(anchor), Some(enemy_side)),
                                        |(_, area, pos, look)| (Some(area), Some(pos), Some(look)))
                                }
                                Task::GoTo(pos) | Task::Defuse(pos) => {
                                    let face = if bot.team == Team::Terrorist { ct_side } else { t_side };
                                    (mesh.nearest_area(pos).map(|a| a.id), Some(pos), Some(face))
                                }
                            };
                            let arrived = follow_path(mesh, world, bot, brain, &mut input, &mut rng, time, turn_rate, goal_area, point);
                            if arrived {
                                // Holding a spot: face the enemy's side, now and then move.
                                if let Some(face) = hold_dir {
                                    let (_, y) = look_angles(bot.movement.origin, face);
                                    brain.yaw = approach(brain.yaw, y, turn_rate * 0.5);
                                    brain.pitch = approach(brain.pitch, 0.0, turn_rate);
                                }
                                if brain.repick_at == 0.0 {
                                    brain.repick_at = time + 8.0 + rng.next_f32() * 10.0;
                                }
                            }
                        }
                        None => wander(bot, brain, &mut input, &mut rng, turn_rate),
                    },
                }
            }
            // Reload while out of contact, and watch reported positions without aiming through walls.
            if !brain.visible && !matches!(task,Task::Plant(_)|Task::Defuse(_)) {
                if let Some(w)=bot.active_weapon() {if w.can_reload() && w.clip<w.def.clip_size/2 && time-brain.last_seen>1.0 {input.buttons|=buttons::RELOAD;}}
            }
            if brain.throw.is_none() && self.aggressive && !finishing && world.flash_amount(bot.id)<0.2
                && time>=brain.next_utility && time>=self.team.utility_after.get(&bot.team).copied().unwrap_or(0.0)
                && (world.tick+bot.id as u64)%16==0 && time-brain.hurt_at>1.5 && bot.plant_start.is_none()
                && (!brain.visible || (eye.distance(brain.target_pos)>550.0 && bot.health>60
                    && world.players.iter().any(|p|p.id!=bot.id&&p.alive&&p.team==bot.team&&p.eye().distance(eye)<500.0&&world.visible(p.eye(),brain.target_pos)))) {
                if let Some((def,angles))=choose_utility(world,bot,contact,task) {
                    brain.throw=Some((def,angles,time+3.5));
                    self.team.utility_after.insert(bot.team,time+3.0);
                }
                brain.next_utility=time+1.0;
            }
            if let Some((def,target,until))=brain.throw {
                let angles=crate::utility::lob_angles(eye,target).unwrap_or(bot.view_angles);
                if time>until || !bot.weapons.iter().any(|w|w.def.def_index==def) || time-brain.hurt_at<0.6 || (brain.visible&&eye.distance(brain.target_pos)<300.0) {
                    brain.throw=None;brain.next_utility=time+5.0;
                } else {
                    input.utility=def;input.weapon_slot=0;input.buttons=0;
                    input.forward_move=0.0;input.side_move=0.0;
                    brain.pitch=approach(brain.pitch,angles.x,turn_rate);brain.yaw=approach(brain.yaw,angles.y,turn_rate);
                }
            }
            // Preserve the path's world-space direction while the head turns.
            // Stable yielding also keeps teammates from occupying one point.
            let (forward, right, _) = movement::angle_vectors(Vec3::new(0.0, brain.yaw, 0.0));
            let mut wish = forward * input.forward_move + right * input.side_move;
            if !brain.visible && brain.throw.is_none() && input.buttons & (buttons::USE|buttons::ATTACK)==0 {
                if let Some(report)=contact.filter(|c| c.pos.distance(eye)<1200.0) {
                    let (pitch,yaw)=look_angles(eye,report.pos);
                    brain.yaw=approach(old_angles.y,yaw,turn_rate*0.7);
                    brain.pitch=approach(old_angles.x,pitch.clamp(-25.0,25.0),turn_rate*0.7);
                    if world.bomb.is_none() && self.team.role(bot)==crate::tactics::Role::Lurk && report.pos.distance(eye)<700.0 {
                        input.buttons|=buttons::WALK;
                    }
                }
            }
            if wish.length_squared() > 1.0 {
                for other in world.players.iter().filter(|p| p.alive && p.id != bot.id && p.team == bot.team) {
                    let delta = (bot.movement.origin - other.movement.origin).truncate();
                    let distance = delta.length();
                    if distance < 62.0 && (bot.movement.origin.z - other.movement.origin.z).abs() < 48.0 {
                        let away = if distance > 1.0 { delta / distance } else { glam::Vec2::new(0.0, if bot.id > other.id { 1.0 } else { -1.0 }) };
                        wish += away.extend(0.0) * (62.0 - distance) * 5.0;
                        if bot.id > other.id && wish.dot((other.movement.origin - bot.movement.origin).normalize_or_zero()) > 0.0 { wish *= 0.55; }
                    }
                }
            }
            brain.pitch = smooth_angle(old_angles.x, brain.pitch, &mut brain.turn_velocity.x, dt, 300.0);
            brain.yaw = smooth_angle(old_angles.y, brain.yaw, &mut brain.turn_velocity.y, dt, 420.0);
            let (forward, right, _) = movement::angle_vectors(Vec3::new(0.0, brain.yaw, 0.0));
            wish = wish.clamp_length_max(450.0);
            input.forward_move = wish.dot(forward);
            input.side_move = wish.dot(right);
            if brain.visible && !finishing {
                let (p, y) = look_angles(eye, brain.target_pos);
                let punch=bot.active_weapon().map_or([0.0;2],|w|w.aim_punch());
                let blocked=world.players.iter().filter(|p|p.alive&&p.team==bot.team&&p.id!=bot.id).any(|p|{
                    let delta=brain.target_pos-eye;let t=((p.eye()-eye).dot(delta)/delta.length_squared().max(1.0)).clamp(0.0,1.0);
                    t>0.0 && t<1.0 && (eye+delta*t).distance(p.eye()-Vec3::Z*10.0)<30.0
                });
                let moving=bot.movement.velocity.truncate().length()>bot.active_weapon().map_or(55.0,|w|w.def.max_player_speed*0.28);
                if blocked || moving || !bot.movement.on_ground || !brain.visible || angle_diff(y, brain.yaw+punch[1]*crate::weapon::RECOIL_SCALE).abs() > 4.0 || angle_diff(p, brain.pitch+punch[0]*crate::weapon::RECOIL_SCALE).abs() > 4.0 {
                    input.buttons &= !buttons::ATTACK;
                }
            }
            if let Some((def,target,_))=brain.throw {
                let angles=crate::utility::lob_angles(eye,target).unwrap_or(bot.view_angles);
                if input.utility==def && angle_diff(angles.x,brain.pitch).abs()<1.5 && angle_diff(angles.y,brain.yaw).abs()<1.5
                    && bot.movement.velocity.length()<35.0 && bot.active_weapon().is_some_and(|w|w.def.def_index==def && time>=w.next_attack)
                    && world.clear_lob(eye,target,angles) {input.buttons|=buttons::ATTACK;}
            }
            if world.flash_amount(bot.id)>0.4 { input.buttons &= !buttons::ATTACK; input.buttons|=buttons::WALK; }
            // Jumping needs the key released in between (no auto-bhop), so
            // tap it; tuck the legs in the air to clear ledges (duck-jump).
            if input.buttons & buttons::JUMP != 0 {
                if bot.last_input.buttons & buttons::JUMP != 0 {
                    input.buttons &= !buttons::JUMP;
                }
                if !bot.movement.on_ground {
                    input.buttons |= buttons::DUCK;
                }
            }
            input.view_angles = Vec3::new(brain.pitch.clamp(-89.0, 89.0), brain.yaw, 0.0);
            out.push((bot.id, input));
        }
        out
    }
}

fn average(points: impl Iterator<Item = Vec3>) -> Vec3 {
    let (sum, n) = points.fold((Vec3::ZERO, 0), |(s, n), p| (s + p, n + 1));
    if n == 0 { Vec3::ZERO } else { sum / n as f32 }
}

fn choose_utility(world: &World, bot: &Player, contact: Option<crate::tactics::Contact>, task: Task) -> Option<(u32,Vec3)> {
    use crate::utility::{Kind,lob_angles};
    let eye=bot.eye();
    let recent=contact.filter(|c|world.time()-c.observed<3.0);
    let defending=bot.team==Team::CounterTerrorist && world.bomb.is_none();
    let retaking=bot.team==Team::CounterTerrorist && world.bomb.is_some();
    for w in &bot.weapons {
        let Some(kind)=Kind::from_name(&w.def.class_name) else {continue};
        let target=match kind {
            Kind::Smoke if retaking => world.bomb?.pos+Vec3::Z*6.0,
            Kind::Smoke if recent.is_some() && (bot.health<45 || !defending) => {
                let c=recent?;eye.lerp(c.pos,0.7)-Vec3::Z*40.0
            }
            Kind::Flash if recent.is_some() => recent?.pos+Vec3::Z*20.0,
            Kind::He if recent.is_some() => recent?.pos-Vec3::Z*36.0,
            Kind::Molotov|Kind::Incendiary if recent.is_some() && !matches!(task,Task::Defuse(_)) => recent?.pos-Vec3::Z*48.0,
            _ => continue,
        };
        let distance=eye.distance(target);
        if !(180.0..1000.0).contains(&distance) {continue}
        if world.utility_fields.iter().any(|f| f.pos.distance(target)<200.0 && (f.kind==kind || (f.kind.fire()&&kind.fire()))) {continue}
        if world.projectiles.iter().any(|p| p.kind==kind&&world.player(p.owner).is_some_and(|p|p.team==bot.team)) {continue}
        if kind!=Kind::Smoke && world.players.iter().filter(|p|p.alive&&p.team==bot.team).any(|p|{
            let predicted=p.eye()+p.movement.velocity*0.5;
            predicted.distance(target)<if kind==Kind::Flash {650.0}else{260.0} && world.visible(predicted,target)
        }) {continue}
        let Some(angles)=lob_angles(eye+Vec3::Z*0.0,target) else {continue};
        if world.clear_lob(eye,target,angles) {return Some((w.def.def_index,target))}
    }
    None
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

/// Follows (or plans) a path. With `goal` the path leads to that area and,
/// with `point`, on to that exact spot; without, to a random hot place.
/// Returns true when the goal has been reached (the bot stands still).
#[allow(clippy::too_many_arguments)]
fn follow_path(
    mesh: &NavMesh,
    world: &World,
    bot: &crate::Player,
    brain: &mut Brain,
    input: &mut TickInput,
    rng: &mut Rng,
    time: f32,
    turn_rate: f32,
    goal: Option<u32>,
    point: Option<Vec3>,
) -> bool {
    let pos = bot.movement.origin;
    if goal != brain.goal_area {
        brain.goal_area = goal;
        brain.path.clear();
        brain.path_index = 0;
    }
    if (world.tick+bot.id as u64)%32==0 && brain.path.iter().skip(brain.path_index).take(3).any(|id|mesh.area(*id).is_some_and(|a|world.fire_at(a.center()))) {brain.path.clear();}
    let need_path = brain.path_index + 1 >= brain.path.len();
    if need_path {
        let start = mesh.nearest_area(pos).map(|a| a.id);
        if let (Some(g), Some(s)) = (goal, start) {
            if s == g {
                // In the goal area: walk to the exact point, if any.
                if let Some(p) = point {
                    if (p - pos).truncate().length() > 16.0 {
                        let (_, ty) = look_angles(pos, p);
                        brain.yaw = approach(brain.yaw, ty, turn_rate);
                        brain.pitch = approach(brain.pitch, 0.0, turn_rate);
                        move_towards(input, brain.yaw, p - pos, ((p - pos).truncate().length() * 6.0).min(250.0));
                        return false;
                    }
                }
                brain.path.clear();
                return true;
            }
        }
        brain.path.clear();
        brain.path_index = 0;
        let target = goal.or_else(|| pick_goal(mesh, rng));
        if let (Some(start), Some(target)) = (start, target) {
            if let Some(p) = mesh.find_path_with_cost(start, target, |a| if world.fire_at(a.center()) {5000.0}else{0.0}) {
                brain.path = p;
            }
        }
        brain.progress_time = time;
        brain.progress_pos = pos;
        if brain.path.len() < 2 {
            wander(bot, brain, input, rng, turn_rate);
            return false;
        }
    }
    // Advance when inside one of the next few areas (a bot may stand above
    // the mesh on a prop, or drop past a small area).
    let inside = |id: u32| {
        mesh.area(id).is_some_and(|a| {
            let dz = pos.z - a.z_at(pos.x, pos.y);
            a.contains_xy(pos, -2.0) && (-48.0..120.0).contains(&dz)
        })
    };
    for ahead in (1..=3).rev() {
        let k = brain.path_index + ahead;
        if k < brain.path.len() && inside(brain.path[k]) {
            brain.path_index = k;
            break;
        }
    }
    if brain.path_index + 1 >= brain.path.len() {
        return false;
    }
    let cur = brain.path[brain.path_index];
    let next = brain.path[brain.path_index + 1];
    let Some(target) = mesh.portal(cur, next, 20.0) else {
        brain.path.clear();
        return false;
    };
    if world.fire_at(target) && !world.fire_at(pos) && pos.distance(target)<180.0 {
        // If every route crosses a burning choke, hold outside it until it clears.
        input.forward_move=0.0;input.side_move=0.0;
        brain.progress_time=time;
        return false;
    }
    let (_, ty) = look_angles(pos, target);
    brain.yaw = approach(brain.yaw, ty, turn_rate);
    brain.pitch = approach(brain.pitch, 0.0, turn_rate);
    // Move along the path; slow down for sharp turns.
    move_towards(input, brain.yaw, target - pos, 450.0);
    if let Some(a) = mesh.area(next) {
        let rise = target.z - pos.z;
        let near = (target - pos).truncate().length() < 96.0;
        if near && a.flags & nav::attr::NO_JUMP == 0 && (rise > 20.0 || (a.flags & nav::attr::JUMP != 0 && rise > 8.0)) {
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
        if time >= brain.escape_until {
            let (_, right, _) = movement::angle_vectors(Vec3::new(0.0, brain.yaw, 0.0));
            let preferred = if bot.id % 2 == 0 { 1.0 } else { -1.0 };
            brain.escape_side = if can_walk(world, bot, right * preferred * 48.0) { preferred } else { -preferred };
            brain.escape_until = time + 0.65;
        }
        input.side_move = brain.escape_side * 300.0;
    }
    false
}

 fn reload_cover(mesh: &NavMesh, world: &World, bot: &Player, threat: Vec3) -> Option<(u32, Vec3)> {
     let start = mesh.nearest_area(bot.movement.origin)?.id;
     let mut choices: Vec<_> = mesh.areas.iter().flat_map(|a| a.hiding_spots.iter().map(move |p| (a.id, *p + Vec3::Z * 2.0)))
         .filter(|(_, p)| p.distance(bot.movement.origin) < 360.0 && (p.z - bot.movement.origin.z).abs() < 64.0)
         .collect();
     choices.sort_by(|a,b| a.1.distance_squared(bot.movement.origin).total_cmp(&b.1.distance_squared(bot.movement.origin)));
     choices.into_iter().take(16).find(|(area, p)| {
         !world.visible(threat, *p + Vec3::Z * 52.0)
             && !world.collision.trace_box(*p,*p,movement::STAND_MINS,movement::STAND_MAXS,collision::contents::MASK_PLAYERSOLID).start_solid
             && (*area == start || mesh.find_path(start,*area).is_some())
     })
 }

fn visible_target(world: &World, eye: Vec3, target: &Player) -> Option<Vec3> {
    let head = target.eye() - Vec3::Z * 2.0;
    [head - Vec3::Z * 12.0, head].into_iter().find(|&spot| world.visible(eye, spot) && !world.smoke_occludes(eye,spot))
}

fn defuse_cost(p: &Player, bomb: Vec3) -> f32 {
    p.movement.origin.distance(bomb) / 220.0 + if p.defuse_kit { 5.0 } else { 10.0 }
}

fn move_towards(input: &mut TickInput, yaw: f32, delta: Vec3, speed: f32) {
    let desired = delta.truncate().normalize_or_zero().extend(0.0) * speed;
    let (f, r, _) = movement::angle_vectors(Vec3::new(0.0, yaw, 0.0));
    input.forward_move = desired.dot(f);
    input.side_move = desired.dot(r);
}

fn can_walk(world: &World, bot: &Player, delta: Vec3) -> bool {
    let pos = bot.movement.origin;
    let (mn, mx) = bot.bounds();
    !world.collision.trace_box(pos + Vec3::Z * 2.0, pos + delta + Vec3::Z * 2.0,
        mn - pos, mx - pos, collision::contents::MASK_PLAYERSOLID).hit()
}

/// Bounded acceleration and braking remove instant angular velocity changes
/// without smoothing player mouse input or changing the simulation tick rate.
fn smooth_angle(current: f32, target: f32, velocity: &mut f32, dt: f32, max_speed: f32) -> f32 {
    let error = angle_diff(target, current);
    let acceleration = 1800.0;
    let desired = error.signum() * max_speed.min((2.0 * acceleration * error.abs()).sqrt()).min(error.abs() * 14.0);
    *velocity += (desired - *velocity).clamp(-acceleration * dt, acceleration * dt);
    let step = *velocity * dt;
    if step.signum() == error.signum() && step.abs() >= error.abs() {
        *velocity = 0.0;
        current + error
    } else { current + step }
}

/// Choose reachable cover around an objective, with room for teammates and
/// a visible nearby approach to watch. Only static map knowledge is used.
fn tactical_spot(mesh: &NavMesh, world: &World, bot: &Player, anchor: Vec3, enemy_side: Vec3,
    reservations: &[(PlayerId, Team, Vec3)]) -> Option<(u32, Vec3, Vec3)> {
    let start = mesh.nearest_area(bot.movement.origin)?.id;
    let mut candidates = Vec::new();
    for a in &mesh.areas {
        let center = a.center();
        if center.truncate().distance(anchor.truncate()) > 620.0 || (center.z - anchor.z).abs() > 140.0 { continue; }
        for (i, point) in a.hiding_spots.iter().copied().chain(std::iter::once(center)).enumerate() {
            let p = Vec3::new(point.x, point.y, a.z_at(point.x, point.y) + 2.0);
            let distance = p.truncate().distance(anchor.truncate());
            if distance < 90.0 || !a.contains_xy(p, -12.0) { continue; }
            let tr = world.collision.trace_box(p, p, Vec3::new(-16.0,-16.0,0.0), Vec3::new(16.0,16.0,72.0), collision::contents::MASK_PLAYERSOLID);
            if tr.start_solid { continue; }
            let occupied = reservations.iter().filter(|(id, team, _)| *id != bot.id && *team == bot.team)
                .map(|(_,_,q)| (170.0 - p.distance(*q)).max(0.0) * 5.0).sum::<f32>();
            let allies = world.players.iter().filter(|q| q.alive && q.team == bot.team && q.id != bot.id)
                .map(|q| (110.0 - p.distance(q.movement.origin)).max(0.0) * 3.0).sum::<f32>();
            let cover = if i < a.hiding_spots.len() { 100.0 } else { 0.0 };
            let diversity = ((a.id.wrapping_mul(31).wrapping_add(bot.id.wrapping_mul(127))) % 97) as f32;
            let objective_sight=world.visible(p+Vec3::Z*52.0,anchor+Vec3::Z*12.0);
            if world.bomb.is_some() && !objective_sight {continue}
            if world.fire_at(p) {continue}
            let score = cover + if objective_sight {80.0}else{0.0} + diversity - (distance - 260.0).abs() * 0.3 - occupied - allies;
            candidates.push((score, a.id, p));
        }
    }
    candidates.sort_by(|a,b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    for (_, area, pos) in candidates.into_iter().take(24) {
        if area != start && mesh.find_path(start, area).is_none() { continue; }
        let eye = pos + Vec3::Z * 62.0;
        let towards_enemy = (enemy_side - pos).truncate().normalize_or_zero();
        let look = mesh.areas.iter().filter_map(|a| {
            let target = a.center() + Vec3::Z * 52.0;
            let d = target - eye;
            let distance = d.length();
            if !(140.0..900.0).contains(&distance) || !world.visible(eye, target) { return None; }
            Some((d.truncate().normalize_or_zero().dot(towards_enemy) * 300.0 + distance * 0.2, target))
        }).max_by(|a,b| a.0.total_cmp(&b.0)).map(|v| v.1);
        if let Some(look) = look { return Some((area, pos, look)); }
    }
    None
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

#[cfg(test)] mod regression_tests {
    use super::*;
    #[test] fn unseen_targets_remain_at_last_seen_position_and_cannot_be_fired_at() {
        let mut collision = collision::CollisionWorld::new();
        let surf = collision.surface_id("wall");
        let a=Vec3::new(50.0,40.0,0.0); let b=Vec3::new(50.0,300.0,0.0);
        let c=Vec3::new(50.0,300.0,200.0); let d=Vec3::new(50.0,40.0,200.0);
        collision.add_triangle([a,b,c],collision::contents::SOLID,surf);
        collision.add_triangle([a,c,d],collision::contents::SOLID,surf);
        collision.finalize();
        let mut w=World::with_catalog(Arc::new(collision), Arc::new(weapons::Catalog { weapons: Vec::new() }));
        let id=w.add_bot("observer",Team::Terrorist);
        let enemy=w.add_player("target",Team::CounterTerrorist);
        w.player_mut(id).unwrap().movement.origin=Vec3::ZERO;
        w.player_mut(id).unwrap().view_angles=Vec3::ZERO;
        w.player_mut(enemy).unwrap().movement.origin=Vec3::new(100.0,0.0,0.0);
        w.tick=(PERCEPTION_INTERVAL-id as u64%PERCEPTION_INTERVAL)%PERCEPTION_INTERVAL;
        let mut bots=Bots::with_skill(0.7);
        bots.think(&w);
        assert_eq!(bots.brains[&id].target,Some(enemy));
        let seen=bots.brains[&id].target_pos;
        w.player_mut(enemy).unwrap().movement.origin=Vec3::new(100.0,200.0,0.0);
        w.tick+=1;
        let input=bots.think(&w)[0].1.clone();
        assert_eq!(bots.brains[&id].target_pos,seen);
        assert!(!bots.brains[&id].visible);
        assert_eq!(input.buttons & buttons::ATTACK,0);
        w.tick+=100;
        bots.think(&w);
        assert_eq!(bots.brains[&id].target,None);
    }
    #[test] fn angular_motion_is_bounded_and_converges_across_wrap() {
        let mut angle=350.0; let mut velocity=0.0;
        for _ in 0..160 {
            let next=smooth_angle(angle,10.0,&mut velocity,crate::TICK_DT,420.0);
            assert!((next-angle).abs()<=420.0*crate::TICK_DT+0.001);
            assert!(next<=370.001);
            angle=next;
        }
        assert!(angle_diff(angle,10.0).abs()<0.01);
    }
    #[test] fn path_movement_is_independent_of_head_direction() {
        for yaw in [0.0,45.0,90.0,180.0,270.0] {
            let mut input=TickInput::default();
            move_towards(&mut input,yaw,Vec3::X,250.0);
            let (f,r,_)=movement::angle_vectors(Vec3::new(0.0,yaw,0.0));
            assert!((f*input.forward_move+r*input.side_move-Vec3::X*250.0).length()<0.001);
        }
    }
}

 #[cfg(test)] mod objective_tests {
     use super::*;
     #[test] fn one_defuser_others_cover_and_active_defuser_is_preserved() {
         let mut collision=collision::CollisionWorld::new(); collision.finalize();
         let mut world=World::with_catalog(Arc::new(collision),Arc::new(weapons::Catalog {weapons:Vec::new()}));
         let t=world.add_bot("T",Team::Terrorist);
         let a=world.add_bot("CT1",Team::CounterTerrorist);
         let b=world.add_bot("CT2",Team::CounterTerrorist);
         world.mode=GameMode::Competitive; world.round.phase=Phase::Live;
         world.objectives.bomb_sites.push(crate::objectives::BombSite { name:'A',zone:Zone { mins:Vec3::splat(-100.0),maxs:Vec3::splat(100.0) } });
         world.player_mut(a).unwrap().movement.origin=Vec3::X*200.0;
         world.player_mut(b).unwrap().movement.origin=Vec3::X*250.0;
         world.player_mut(b).unwrap().defuse_kit=true;
         world.bomb=Some(crate::PlantedBomb {pos:Vec3::ZERO,site:0,planter:t,planted_at:0.0,explode_at:40.0,defuser:None});
         assert!(matches!(Bots::task(&world,world.player(b).unwrap()),Task::Defuse(_)));
         assert!(matches!(Bots::task(&world,world.player(a).unwrap()),Task::Guard(_)));
         assert!(matches!(Bots::task(&world,world.player(t).unwrap()),Task::Guard(_)));
         world.bomb.as_mut().unwrap().defuser=Some((a,5.0));
         assert!(matches!(Bots::task(&world,world.player(a).unwrap()),Task::Defuse(_)));
         assert!(matches!(Bots::task(&world,world.player(b).unwrap()),Task::Guard(_)));
     }
 }
