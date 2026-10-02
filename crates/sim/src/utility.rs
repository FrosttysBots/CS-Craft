//! Deterministic grenades. Inventory, collision and effects are shared by humans and bots.
use crate::{GameEvent, PlayerId, TICK_DT, World};
use collision::contents;
use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Smoke,
    Flash,
    He,
    Molotov,
    Incendiary,
    Decoy,
}
impl Kind {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "weapon_smokegrenade" => Self::Smoke,
            "weapon_flashbang" => Self::Flash,
            "weapon_hegrenade" => Self::He,
            "weapon_molotov" => Self::Molotov,
            "weapon_incgrenade" => Self::Incendiary,
            "weapon_decoy" => Self::Decoy,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Smoke => "weapon_smokegrenade",
            Self::Flash => "weapon_flashbang",
            Self::He => "weapon_hegrenade",
            Self::Molotov => "weapon_molotov",
            Self::Incendiary => "weapon_incgrenade",
            Self::Decoy => "weapon_decoy",
        }
    }
    pub fn fire(self) -> bool {
        matches!(self, Self::Molotov | Self::Incendiary)
    }
}
#[derive(Clone, Debug)]
pub struct Projectile {
    pub id: u32,
    pub owner: PlayerId,
    pub kind: Kind,
    pub pos: Vec3,
    pub velocity: Vec3,
    pub born: f32,
}
#[derive(Clone, Debug)]
pub struct Field {
    pub id: u32,
    pub owner: PlayerId,
    pub kind: Kind,
    pub pos: Vec3,
    pub born: f32,
    pub until: f32,
}

/// Low ballistic arc, rejecting out-of-range targets. Source angles, gravity 320.
pub fn lob_angles(from: Vec3, to: Vec3) -> Option<Vec3> {
    let delta = to - from;
    let x = delta.truncate().length();
    if x < 32.0 {
        return None;
    }
    let v2: f32 = 750.0 * 750.0;
    let disc = v2 * v2 - 320.0 * (320.0 * x * x + 2.0 * delta.z * v2);
    if disc < 0.0 {
        return None;
    }
    let elevation = ((v2 - disc.sqrt()) / (320.0 * x)).atan();
    Some(Vec3::new(
        -elevation.to_degrees(),
        delta.y.atan2(delta.x).to_degrees(),
        0.0,
    ))
}

pub fn crosses_smoke(a: Vec3, b: Vec3, center: Vec3, radius: f32) -> bool {
    let d = b - a;
    let t = ((center - a).dot(d) / d.length_squared().max(0.001)).clamp(0.0, 1.0);
    (a + d * t).distance_squared(center) < radius * radius
}

impl World {
    pub fn smoke_occludes(&self, from: Vec3, to: Vec3) -> bool {
        self.utility_fields.iter().any(|f| {
            f.kind == Kind::Smoke
                && f.until > self.time()
                && crosses_smoke(
                    from,
                    to,
                    f.pos + Vec3::Z * 55.0,
                    140.0 * ((self.time() - f.born) / 0.6).clamp(0.0, 1.0),
                )
        })
    }
    pub fn fire_at(&self, point: Vec3) -> bool {
        self.utility_fields.iter().any(|f| {
            f.kind.fire()
                && f.until > self.time()
                && (point.z - f.pos.z).abs() < 65.0
                && point.truncate().distance(f.pos.truncate()) < 130.0
        })
    }
    pub fn flash_amount(&self, id: PlayerId) -> f32 {
        self.player(id).map_or(0.0, |p| {
            ((p.flash_until - self.time()) / 1.5).clamp(0.0, 1.0)
        })
    }
    /// Validate the actual throw arc; bots don't throw through ceilings/walls.
    pub fn clear_lob(&self, from: Vec3, target: Vec3, angles: Vec3) -> bool {
        let (f, _, _) = movement::angle_vectors(angles);
        let duration =
            from.truncate().distance(target.truncate()) / (f.truncate().length() * 750.0).max(1.0);
        let mut previous = from;
        for step in 1..=24 {
            let t = duration * step as f32 / 24.0;
            let next = from + f * 750.0 * t - Vec3::Z * 160.0 * t * t;
            let tr = self.collision.trace_box(
                previous,
                next,
                Vec3::splat(-2.0),
                Vec3::splat(2.0),
                contents::MASK_SOLID,
            );
            if tr.hit() {
                return next.distance(target) < 45.0 && !tr.start_solid;
            }
            previous = next;
        }
        true
    }
    pub(crate) fn throw_grenade(&mut self, i: usize) {
        let time = self.time();
        let p = &self.players[i];
        let Some(kind) = p
            .active_weapon()
            .and_then(|w| Kind::from_name(&w.def.class_name))
        else {
            return;
        };
        let (forward, _, _) = movement::angle_vectors(p.view_angles);
        let tr = self.collision.trace_box(
            p.eye(),
            p.eye() + forward * 18.0,
            Vec3::splat(-2.0),
            Vec3::splat(2.0),
            contents::MASK_SOLID,
        );
        if tr.start_solid {
            return;
        }
        let pos = p.eye() + forward * 18.0 * tr.fraction;
        let owner = p.id;
        let velocity = forward * 750.0 + p.movement.velocity * 0.5;
        let active = p.active;
        self.players[i].take(active, time);
        self.next_item += 1;
        self.projectiles.push(Projectile {
            id: self.next_item,
            owner,
            kind,
            pos,
            velocity,
            born: time,
        });
        self.events.push(GameEvent::UtilityThrown {
            player: owner,
            kind,
            pos,
        });
    }
    pub(crate) fn step_utility(&mut self) {
        let time = self.time();
        let mut exploded = Vec::new();
        for p in &mut self.projectiles {
            p.velocity.z -= 320.0 * TICK_DT;
            let end = p.pos + p.velocity * TICK_DT;
            let tr = self.collision.trace_box(
                p.pos,
                end,
                Vec3::splat(-2.0),
                Vec3::splat(2.0),
                contents::MASK_SOLID,
            );
            p.pos = p.pos.lerp(end, tr.fraction);
            let floor = tr.hit() && tr.normal.z > 0.65;
            if tr.hit() {
                p.pos += tr.normal * 0.1;
                p.velocity = (p.velocity - 2.0 * p.velocity.dot(tr.normal) * tr.normal) * 0.45;
                if floor && p.velocity.length() < 35.0 {
                    p.velocity = Vec3::ZERO;
                }
            }
            let age = time - p.born;
            let detonate = if p.kind.fire() {
                (floor && age > 0.15) || age > 3.5
            } else if matches!(p.kind, Kind::Smoke | Kind::Decoy) {
                age > 1.5 && (p.velocity.length() < 20.0 || age > 3.0)
            } else {
                age > 1.5
            };
            if detonate {
                exploded.push(p.clone());
            }
        }
        self.projectiles
            .retain(|p| !exploded.iter().any(|e| e.id == p.id));
        for p in exploded {
            self.events.push(GameEvent::UtilityDetonated {
                player: p.owner,
                kind: p.kind,
                pos: p.pos,
            });
            match p.kind {
                Kind::Flash => {
                    for i in 0..self.players.len() {
                        let player = &self.players[i];
                        let delta = p.pos - player.eye();
                        if !player.alive
                            || delta.length() > 1800.0
                            || !self.visible(player.eye(), p.pos)
                        {
                            continue;
                        }
                        let (f, _, _) = movement::angle_vectors(player.view_angles);
                        let facing = f.dot(delta.normalize_or_zero());
                        let duration = (1.0 - delta.length() / 1800.0)
                            * (if facing > 0.5 {
                                4.8
                            } else if facing > -0.3 {
                                2.0
                            } else {
                                0.5
                            });
                        self.players[i].flash_until =
                            self.players[i].flash_until.max(time + duration);
                    }
                }
                Kind::He => self.blast_utility(p.owner, p.kind, p.pos, 350.0, 98.0),
                kind => self.utility_fields.push(Field {
                    id: p.id,
                    owner: p.owner,
                    kind,
                    pos: p.pos,
                    born: time,
                    until: time
                        + if kind == Kind::Smoke {
                            18.0
                        } else if kind == Kind::Decoy {
                            15.0
                        } else {
                            7.0
                        },
                }),
            }
        }
        // Smoke extinguishes fire, including fire created after the smoke.
        let smokes: Vec<_> = self
            .utility_fields
            .iter()
            .filter(|f| f.kind == Kind::Smoke && f.until > time)
            .map(|f| f.pos)
            .collect();
        self.utility_fields.retain(|f| {
            f.until > time && !(f.kind.fire() && smokes.iter().any(|s| s.distance(f.pos) < 180.0))
        });
        if self.tick % 32 == 0 {
            for f in self.utility_fields.iter().filter(|f| f.kind == Kind::Decoy) {
                self.events.push(GameEvent::DecoyShot {
                    player: f.owner,
                    pos: f.pos,
                });
            }
        }
        if self.tick % 13 == 0 {
            let fields = self.utility_fields.clone();
            for f in fields.iter().filter(|f| f.kind.fire()) {
                self.blast_utility(f.owner, f.kind, f.pos + Vec3::Z * 12.0, 140.0, 8.0);
            }
        }
    }
    fn blast_utility(&mut self, owner: PlayerId, kind: Kind, pos: Vec3, radius: f32, base: f32) {
        let attacker = self.players.iter().position(|p| p.id == owner);
        for i in 0..self.players.len() {
            let p = &self.players[i];
            let chest = p.movement.origin + Vec3::Z * 32.0;
            let dist = chest.distance(pos);
            if !p.alive || dist >= radius || !self.visible(pos, chest) {
                continue;
            }
            let friendly = attacker.is_some_and(|a| a != i && self.players[a].team == p.team);
            let raw =
                base * (if kind.fire() {
                    1.0
                } else {
                    (1.0 - dist / radius).powf(1.2)
                }) * if friendly { 0.35 } else { 1.0 };
            let absorbed = if kind == Kind::He {
                (raw * 0.5).min(p.armor as f32)
            } else {
                0.0
            };
            let damage = (raw - absorbed).ceil() as i32;
            let victim = p.id;
            self.players[i].armor -= absorbed as i32;
            self.players[i].health -= damage;
            self.events.push(GameEvent::Damage {
                attacker: owner,
                victim,
                health: damage,
                group: crate::weapon::HitGroup::Chest,
                pos: chest,
            });
            if self.players[i].health <= 0 {
                self.kill(i, attacker, kind.name(), false);
                if let Some(a) = attacker {
                    if friendly || a == i {
                        self.players[a].kills = self.players[a].kills.saturating_sub(1);
                    } else if self.competitive() {
                        self.players[a].money =
                            (self.players[a].money + 300).min(self.rules.max_money);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn arena() -> World {
        let mut c = collision::CollisionWorld::new();
        c.finalize();
        World::new(std::sync::Arc::new(c))
    }
    #[test]
    fn throwing_consumes_inventory_and_never_emits_bullets() {
        let mut w = arena();
        let cat=weapons::Catalog::from_items_game(r#""items_game" { "items" { "43" { "name" "weapon_flashbang" "baseitem" "0" "attributes" { "primary default clip size" "1" "in game price" "200" } } } }"#).unwrap();
        assert_eq!(cat.weapons.len(), 1);
        let id = w.add_player("thrower", crate::Team::Terrorist);
        let def = cat.get("weapon_flashbang").unwrap();
        assert_eq!(def.clip_size, 1);
        w.player_mut(id)
            .unwrap()
            .give(crate::weapon::WeaponState::new(std::sync::Arc::new(
                def.clone(),
            )));
        w.step(&[(
            id,
            crate::TickInput {
                buttons: movement::buttons::ATTACK,
                ..Default::default()
            },
        )]);
        assert!(!w.player(id).unwrap().has("weapon_flashbang"));
        assert_eq!(w.projectiles.len(), 1);
        assert!(
            w.events
                .iter()
                .any(|e| matches!(e, GameEvent::UtilityThrown { .. }))
        );
        assert!(
            !w.events
                .iter()
                .any(|e| matches!(e, GameEvent::Fire { .. } | GameEvent::Tracer { .. }))
        );
        // Selecting a grenade that isn't carried must never create one.
        w.step(&[(
            id,
            crate::TickInput {
                utility: 43,
                buttons: movement::buttons::ATTACK,
                ..Default::default()
            },
        )]);
        assert_eq!(w.projectiles.len(), 1);
    }
    #[test]
    fn flash_respects_facing_and_line_of_sight() {
        let mut w = arena();
        let a = w.add_player("looking", crate::Team::Terrorist);
        let b = w.add_player("turned", crate::Team::CounterTerrorist);
        for p in &mut w.players {
            p.movement.origin = Vec3::ZERO;
            p.view_angles = Vec3::ZERO;
        }
        w.player_mut(b).unwrap().view_angles.y = 180.0;
        w.tick = 128;
        w.projectiles.push(Projectile {
            id: 1,
            owner: a,
            kind: Kind::Flash,
            pos: Vec3::new(200.0, 0.0, 64.0),
            velocity: Vec3::ZERO,
            born: 0.0,
        });
        w.step_utility();
        assert!(w.player(a).unwrap().flash_until > w.player(b).unwrap().flash_until + 2.0);
        assert_eq!(w.projectiles.len(), 0);
    }
    #[test]
    fn smoke_extinguishes_fire_expires_and_does_not_block_bullet_geometry() {
        let mut w = arena();
        w.tick = 64;
        w.utility_fields.push(Field {
            id: 1,
            owner: 1,
            kind: Kind::Molotov,
            pos: Vec3::ZERO,
            born: 0.0,
            until: 7.0,
        });
        w.utility_fields.push(Field {
            id: 2,
            owner: 2,
            kind: Kind::Smoke,
            pos: Vec3::ZERO,
            born: 0.0,
            until: 18.0,
        });
        assert!(w.fire_at(Vec3::ZERO));
        w.step_utility();
        assert!(!w.fire_at(Vec3::ZERO));
        let a = Vec3::new(-300.0, 0.0, 55.0);
        let b = Vec3::new(300.0, 0.0, 55.0);
        assert!(w.smoke_occludes(a, b));
        assert!(w.visible(a, b));
        w.tick = 19 * 64;
        w.step_utility();
        assert!(!w.smoke_occludes(a, b));
        assert!(w.utility_fields.is_empty());
    }
    #[test]
    fn explosive_damage_falls_off_and_armor_absorbs() {
        let mut w = arena();
        let a = w.add_player("near", crate::Team::Terrorist);
        let b = w.add_player("far", crate::Team::CounterTerrorist);
        w.player_mut(a).unwrap().movement.origin = Vec3::X * 70.0;
        w.player_mut(b).unwrap().movement.origin = Vec3::X * 300.0;
        w.player_mut(a).unwrap().armor = 0;
        w.player_mut(b).unwrap().armor = 0;
        w.blast_utility(crate::WORLD, Kind::He, Vec3::Z * 32.0, 350.0, 98.0);
        let near = w.player(a).unwrap().health;
        let far = w.player(b).unwrap().health;
        assert!(near < far && far < 100);
        w.player_mut(a).unwrap().health = 100;
        w.player_mut(a).unwrap().armor = 100;
        w.blast_utility(crate::WORLD, Kind::He, Vec3::Z * 32.0, 350.0, 98.0);
        assert!(w.player(a).unwrap().health > near);
        assert!(w.player(a).unwrap().armor < 100);
    }
    #[test]
    fn lob_hits_target_at_requested_height() {
        let from = Vec3::new(0.0, 0.0, 64.0);
        let to = Vec3::new(700.0, 120.0, 8.0);
        let angles = lob_angles(from, to).unwrap();
        let (f, _, _) = movement::angle_vectors(angles);
        let t = (to - from).truncate().length() / (750.0 * f.truncate().length());
        assert!((from + f * 750.0 * t - Vec3::Z * 160.0 * t * t - to).length() < 0.01);
        assert!(lob_angles(from, Vec3::X * 4000.0).is_none());
    }
    #[test]
    fn smoke_blocks_only_intersecting_segments() {
        assert!(crosses_smoke(
            -Vec3::X * 300.0,
            Vec3::X * 300.0,
            Vec3::ZERO,
            140.0
        ));
        assert!(!crosses_smoke(
            Vec3::X * 200.0,
            Vec3::X * 400.0,
            Vec3::ZERO,
            140.0
        ));
    }
}
