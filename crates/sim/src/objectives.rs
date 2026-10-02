//! Map objectives: buy zones and bomb sites.
//!
//! Both are brush entities (`func_buyzone` with a `TeamNum`,
//! `func_bomb_target`) whose `model` key names a BSP brush model (`*28`);
//! the model's bounding box is used as the trigger volume. Maps without
//! buy zones get one around each team's spawn points, as the game does.
//!
//! Sites are named A/B from the map's radar overview file
//! (`resource/overviews/<map>.txt`, `bombA_x`/`bombA_y` in radar space)
//! when it exists, otherwise in entity order.

use crate::Team;
use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zone {
    pub mins: Vec3,
    pub maxs: Vec3,
}

impl Zone {
    pub fn center(&self) -> Vec3 {
        (self.mins + self.maxs) * 0.5
    }

    /// Box-vs-box overlap with a player hull (how triggers touch players).
    pub fn touches(&self, mins: Vec3, maxs: Vec3) -> bool {
        (0..3).all(|a| mins[a] <= self.maxs[a] && maxs[a] >= self.mins[a])
    }

    pub fn contains(&self, p: Vec3) -> bool {
        self.touches(p, p)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BuyZone {
    pub team: Team,
    pub zone: Zone,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BombSite {
    /// 'A', 'B', ...
    pub name: char,
    pub zone: Zone,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Objectives {
    pub buy_zones: Vec<BuyZone>,
    pub bomb_sites: Vec<BombSite>,
}

fn brush_zone(map: &bsp::Bsp, ent: &kv::Block) -> Option<Zone> {
    let index: usize = ent.str("model")?.strip_prefix('*')?.parse().ok()?;
    let m = map.models.get(index)?;
    let origin = ent.str("origin").and_then(bsp::parse_origin).unwrap_or(Vec3::ZERO);
    Some(Zone {
        mins: m.mins + origin,
        maxs: m.maxs + origin,
    })
}

/// `bombA`/`bombB` world positions from an overview file.
fn overview_sites(text: &str) -> Vec<(char, Vec3)> {
    let Ok(doc) = kv::parse(text) else { return Vec::new() };
    let Some((_, root)) = doc.iter().find_map(|(k, v)| v.as_block().map(|b| (k, b))) else {
        return Vec::new();
    };
    let f = |k: &str| root.str(k).and_then(kv::parse_f32);
    let (Some(pos_x), Some(pos_y), Some(scale)) = (f("pos_x"), f("pos_y"), f("scale")) else {
        return Vec::new();
    };
    // Radar coordinates are 0..1 across a 1024-pixel image.
    let mut out = Vec::new();
    for name in ['A', 'B'] {
        if let (Some(u), Some(v)) = (f(&format!("bomb{name}_x")), f(&format!("bomb{name}_y"))) {
            out.push((name, Vec3::new(pos_x + u * 1024.0 * scale, pos_y - v * 1024.0 * scale, 0.0)));
        }
    }
    out
}

impl Objectives {
    pub fn from_bsp(map: &bsp::Bsp, spawns: &[crate::SpawnPoint], overview: Option<&str>) -> Self {
        let mut out = Objectives::default();
        for ent in map.entities_of_class("func_buyzone") {
            let team = match ent.str("TeamNum").or_else(|| ent.str("teamnum")) {
                Some("2") => Team::Terrorist,
                Some("3") => Team::CounterTerrorist,
                _ => continue,
            };
            if let Some(zone) = brush_zone(map, ent) {
                out.buy_zones.push(BuyZone { team, zone });
            }
        }
        for team in [Team::Terrorist, Team::CounterTerrorist] {
            if out.buy_zones.iter().any(|z| z.team == team) {
                continue;
            }
            // No buy zone for this team: cover its spawns, generously.
            let points: Vec<Vec3> = spawns.iter().filter(|s| s.team == team).map(|s| s.origin).collect();
            if let Some(first) = points.first() {
                let (mut mins, mut maxs) = (*first, *first);
                for p in &points {
                    mins = mins.min(*p);
                    maxs = maxs.max(*p);
                }
                let pad = Vec3::new(200.0, 200.0, 100.0);
                out.buy_zones.push(BuyZone {
                    team,
                    zone: Zone { mins: mins - pad, maxs: maxs + pad },
                });
            }
        }

        let zones: Vec<Zone> = map
            .entities_of_class("func_bomb_target")
            .filter_map(|e| brush_zone(map, e))
            .collect();
        let named = overview.map(overview_sites).unwrap_or_default();
        let mut taken = Vec::new();
        for (i, zone) in zones.iter().enumerate() {
            let c = zone.center();
            let by_overview = named
                .iter()
                .filter(|(n, _)| !taken.contains(n))
                .min_by(|a, b| a.1.truncate().distance(c.truncate()).total_cmp(&b.1.truncate().distance(c.truncate())))
                .map(|(n, _)| *n);
            let name = by_overview.unwrap_or((b'A' + i as u8) as char);
            taken.push(name);
            out.bomb_sites.push(BombSite { name, zone: *zone });
        }
        out.bomb_sites.sort_by_key(|s| s.name);
        out
    }

    pub fn in_buy_zone(&self, team: Team, mins: Vec3, maxs: Vec3) -> bool {
        self.buy_zones.iter().any(|z| z.team == team && z.zone.touches(mins, maxs))
    }

    /// Index of the bomb site a hull touches.
    pub fn site_at(&self, mins: Vec3, maxs: Vec3) -> Option<usize> {
        self.bomb_sites.iter().position(|s| s.zone.touches(mins, maxs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overview_names_sites() {
        let text = r#""de_test" { "pos_x" "-2476" "pos_y" "3239" "scale" "4.4" "bombA_x" "0.80" "bombA_y" "0.16" "bombB_x" "0.21" "bombB_y" "0.12" }"#;
        let sites = overview_sites(text);
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].0, 'A');
        assert!((sites[0].1.x - 1128.5).abs() < 1.0 && (sites[0].1.y - 2518.1).abs() < 1.0);
    }

    #[test]
    fn zone_touch() {
        let z = Zone { mins: Vec3::ZERO, maxs: Vec3::splat(100.0) };
        assert!(z.touches(Vec3::new(-20.0, 10.0, 0.0), Vec3::new(10.0, 40.0, 72.0)));
        assert!(!z.touches(Vec3::new(101.0, 0.0, 0.0), Vec3::new(130.0, 10.0, 72.0)));
    }
}
