//! Navigation meshes (`maps/<map>.nav`) and path-finding.
//!
//! A nav mesh is a set of axis-aligned rectangular walkable areas with
//! one-way connections in four directions. The layout follows the public
//! description of the NAV format (version 16 as shipped with CS:GO), with
//! the CS:GO-specific per-area trailer (a count byte plus 14-byte records)
//! skipped.

use glam::Vec3;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::io;

const MAGIC: u32 = 0xFEED_FACE;

/// `NavAttributeType` bits used by bots.
pub mod attr {
    pub const CROUCH: u32 = 0x0001;
    pub const JUMP: u32 = 0x0002;
    pub const PRECISE: u32 = 0x0004;
    pub const NO_JUMP: u32 = 0x0008;
    pub const STOP: u32 = 0x0010;
    pub const RUN: u32 = 0x0020;
    pub const WALK: u32 = 0x0040;
    pub const AVOID: u32 = 0x0080;
    pub const STAIRS: u32 = 0x1000;
}

/// Connection directions: north (-Y), east (+X), south (+Y), west (-X).
pub const DIRECTIONS: usize = 4;

#[derive(Clone, Debug)]
pub struct Area {
    pub id: u32,
    pub flags: u32,
    /// North-west corner (min X, min Y).
    pub nw: Vec3,
    /// South-east corner (max X, max Y).
    pub se: Vec3,
    pub ne_z: f32,
    pub sw_z: f32,
    pub connections: [Vec<u32>; DIRECTIONS],
    pub place: u16,
    pub hiding_spots: Vec<Vec3>,
}

impl Area {
    pub fn center(&self) -> Vec3 {
        Vec3::new(
            (self.nw.x + self.se.x) * 0.5,
            (self.nw.y + self.se.y) * 0.5,
            (self.nw.z + self.se.z + self.ne_z + self.sw_z) * 0.25,
        )
    }

    pub fn contains_xy(&self, p: Vec3, margin: f32) -> bool {
        p.x >= self.nw.x - margin && p.x <= self.se.x + margin && p.y >= self.nw.y - margin && p.y <= self.se.y + margin
    }

    /// Ground height at an XY position (bilinear over the four corners).
    pub fn z_at(&self, x: f32, y: f32) -> f32 {
        let w = (self.se.x - self.nw.x).max(1e-3);
        let h = (self.se.y - self.nw.y).max(1e-3);
        let u = ((x - self.nw.x) / w).clamp(0.0, 1.0);
        let v = ((y - self.nw.y) / h).clamp(0.0, 1.0);
        // Corners: nw (0,0), ne (1,0), sw (0,1), se (1,1).
        let north = self.nw.z + (self.ne_z - self.nw.z) * u;
        let south = self.sw_z + (self.se.z - self.sw_z) * u;
        north + (south - north) * v
    }

    pub fn size(&self) -> f32 {
        (self.se.x - self.nw.x) * (self.se.y - self.nw.y)
    }
}

#[derive(Clone, Debug, Default)]
pub struct NavMesh {
    pub version: u32,
    pub places: Vec<String>,
    pub areas: Vec<Area>,
    index: HashMap<u32, usize>,
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        let s = self.d.get(self.p..self.p + n).ok_or_else(|| invalid(format!("nav truncated at {}", self.p)))?;
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> io::Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> io::Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f32(&mut self) -> io::Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn vec3(&mut self) -> io::Result<Vec3> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    fn count(&mut self, max: u32) -> io::Result<usize> {
        let n = self.u32()?;
        if n > max {
            return Err(invalid(format!("implausible count {n} at {}", self.p - 4)));
        }
        Ok(n as usize)
    }
}

impl NavMesh {
    /// Build runtime navigation (e.g. a voxel world whose floor can change).
    pub fn from_areas(areas: Vec<Area>) -> Self {
        let index=areas.iter().enumerate().map(|(i,a)|(a.id,i)).collect();
        Self{version:16,places:vec!["Overworld".into()],areas,index}
    }
    pub fn parse(bytes: &[u8]) -> io::Result<NavMesh> {
        let mut r = Reader { d: bytes, p: 0 };
        if r.u32()? != MAGIC {
            return Err(invalid("not a nav file"));
        }
        let version = r.u32()?;
        if !(6..=16).contains(&version) {
            return Err(invalid(format!("unsupported nav version {version}")));
        }
        let subversion = if version >= 10 { r.u32()? } else { 0 };
        if version >= 4 {
            r.u32()?; // BSP size the mesh was built for
        }
        if version >= 14 {
            r.u8()?; // analysed
        }
        let mut places = Vec::new();
        if version >= 5 {
            let n = r.u16()?;
            for _ in 0..n {
                let len = r.u16()? as usize;
                let raw = r.take(len)?;
                places.push(String::from_utf8_lossy(raw).trim_end_matches('\0').to_string());
            }
            if version > 11 {
                r.u8()?; // has unnamed areas
            }
        }
        let area_count = r.count(1_000_000)?;
        let mut areas = Vec::with_capacity(area_count);
        for _ in 0..area_count {
            let id = r.u32()?;
            let flags = match version {
                ..=8 => r.u8()? as u32,
                9..=12 => r.u16()? as u32,
                _ => r.u32()?,
            };
            let nw = r.vec3()?;
            let se = r.vec3()?;
            let ne_z = r.f32()?;
            let sw_z = r.f32()?;
            let mut connections: [Vec<u32>; DIRECTIONS] = Default::default();
            for dir in connections.iter_mut() {
                let n = r.count(4096)?;
                for _ in 0..n {
                    dir.push(r.u32()?);
                }
            }
            let spots = r.u8()?;
            let mut hiding_spots = Vec::with_capacity(spots as usize);
            for _ in 0..spots {
                r.u32()?;
                hiding_spots.push(r.vec3()?);
                r.u8()?;
            }
            if version < 15 {
                let approach = r.u8()?;
                r.take(approach as usize * 14)?;
            }
            let encounters = r.count(65_536)?;
            for _ in 0..encounters {
                r.take(4 + 1 + 4 + 1)?;
                let n = r.u8()?;
                r.take(n as usize * 5)?;
            }
            let place = r.u16()?;
            for _ in 0..2 {
                let n = r.count(4096)?;
                r.take(n * 4)?;
            }
            r.take(8)?; // earliest occupy time per team
            if version >= 11 {
                r.take(16)?; // light intensity per corner
            }
            if version >= 16 {
                let n = r.count(1_000_000)?;
                r.take(n * 5)?; // potentially visible areas
                r.u32()?; // inherit visibility from
            }
            if subversion > 0 {
                // CS:GO-specific trailer.
                let n = r.u8()?;
                r.take(n as usize * 14)?;
            }
            areas.push(Area {
                id,
                flags,
                nw,
                se,
                ne_z,
                sw_z,
                connections,
                place,
                hiding_spots,
            });
        }
        let index = areas.iter().enumerate().map(|(i, a)| (a.id, i)).collect();
        Ok(NavMesh {
            version,
            places,
            areas,
            index,
        })
    }

    pub fn area(&self, id: u32) -> Option<&Area> {
        self.index.get(&id).map(|&i| &self.areas[i])
    }

    pub fn place_name(&self, place: u16) -> Option<&str> {
        // Place 0 is "no place"; names are 1-based.
        place.checked_sub(1).and_then(|i| self.places.get(i as usize)).map(String::as_str)
    }

    /// The area under (or nearest to) a point.
    pub fn nearest_area(&self, p: Vec3) -> Option<&Area> {
        let mut best: Option<(&Area, f32)> = None;
        for a in &self.areas {
            let score = if a.contains_xy(p, 0.0) {
                let dz = p.z - a.z_at(p.x, p.y);
                // Prefer the floor just below the point.
                if (-8.0..=80.0).contains(&dz) { dz.abs() * 0.01 } else { dz.abs() + 1000.0 }
            } else {
                let cx = p.x.clamp(a.nw.x, a.se.x);
                let cy = p.y.clamp(a.nw.y, a.se.y);
                let cz = a.z_at(cx, cy);
                Vec3::new(cx, cy, cz).distance(p) + 100.0
            };
            if best.is_none_or(|(_, s)| score < s) {
                best = Some((a, score));
            }
        }
        best.map(|(a, _)| a)
    }

    /// A* over area centres. Returns area ids from `from` to `to` inclusive.
    pub fn find_path(&self, from: u32, to: u32) -> Option<Vec<u32>> {
        self.find_path_with_cost(from,to,|_|0.0)
    }

    /// Additional nonnegative tactical traversal cost (fire, exposed approaches).
    pub fn find_path_with_cost(&self, from: u32, to: u32, extra: impl Fn(&Area)->f32) -> Option<Vec<u32>> {
        #[derive(PartialEq)]
        struct Node(f32, u32);
        impl Eq for Node {}
        impl PartialOrd for Node {
            fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for Node {
            fn cmp(&self, o: &Self) -> Ordering {
                o.0.total_cmp(&self.0)
            }
        }
        let goal = self.area(to)?.center();
        self.area(from)?;
        let mut open = BinaryHeap::new();
        let mut came: HashMap<u32, u32> = HashMap::new();
        let mut cost: HashMap<u32, f32> = HashMap::new();
        cost.insert(from, 0.0);
        open.push(Node(0.0, from));
        while let Some(Node(_, id)) = open.pop() {
            if id == to {
                let mut path = vec![to];
                let mut cur = to;
                while let Some(&prev) = came.get(&cur) {
                    path.push(prev);
                    cur = prev;
                }
                path.reverse();
                return Some(path);
            }
            let a = self.area(id)?;
            let g = cost[&id];
            for dir in &a.connections {
                for &next in dir {
                    let Some(b) = self.area(next) else { continue };
                    let mut step = a.center().distance(b.center());
                    if b.flags & attr::AVOID != 0 {
                        step *= 4.0;
                    }
                    if b.flags & attr::CROUCH != 0 {
                        step *= 1.5;
                    }
                    let penalty=extra(b);
                    let ng = g + step + if penalty.is_finite() {penalty.max(0.0)}else{0.0};
                    if cost.get(&next).is_none_or(|&c| ng < c) {
                        cost.insert(next, ng);
                        came.insert(next, id);
                        open.push(Node(ng + b.center().distance(goal), next));
                    }
                }
            }
        }
        None
    }

    /// A point to aim for when crossing from area `a` into its neighbour
    /// `b`: the middle of their shared edge, pulled inside by `inset`.
    pub fn portal(&self, a: u32, b: u32, inset: f32) -> Option<Vec3> {
        let (a, b) = (self.area(a)?, self.area(b)?);
        let lo_x = a.nw.x.max(b.nw.x);
        let hi_x = a.se.x.min(b.se.x);
        let lo_y = a.nw.y.max(b.nw.y);
        let hi_y = a.se.y.min(b.se.y);
        let mid = |lo: f32, hi: f32, fallback: f32| {
            if hi >= lo {
                let m = (lo + hi) * 0.5;
                if hi - lo > 2.0 * inset { m.clamp(lo + inset, hi - inset) } else { m }
            } else {
                fallback
            }
        };
        let bc = b.center();
        let x = mid(lo_x, hi_x, bc.x);
        let y = mid(lo_y, hi_y, bc.y);
        Some(Vec3::new(x, y, b.z_at(x, y)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(id: u32, x0: f32, y0: f32, x1: f32, y1: f32, conns: [Vec<u32>; 4]) -> Area {
        Area {
            id,
            flags: 0,
            nw: Vec3::new(x0, y0, 0.0),
            se: Vec3::new(x1, y1, 0.0),
            ne_z: 0.0,
            sw_z: 0.0,
            connections: conns,
            place: 0,
            hiding_spots: Vec::new(),
        }
    }

    fn mesh(areas: Vec<Area>) -> NavMesh {
        let index = areas.iter().enumerate().map(|(i, a)| (a.id, i)).collect();
        NavMesh {
            version: 16,
            places: vec![],
            areas,
            index,
        }
    }

    #[test]
    fn path_goes_around() {
        // 1 → 2 → 3 in an L; 1 and 3 are not directly connected.
        let m = mesh(vec![
            area(1, 0.0, 0.0, 100.0, 100.0, [vec![], vec![2], vec![], vec![]]),
            area(2, 100.0, 0.0, 200.0, 100.0, [vec![], vec![], vec![3], vec![1]]),
            area(3, 100.0, 100.0, 200.0, 200.0, [vec![2], vec![], vec![], vec![]]),
        ]);
        assert_eq!(m.find_path(1, 3), Some(vec![1, 2, 3]));
        assert_eq!(m.find_path(3, 1), Some(vec![3, 2, 1]));
        let p = m.portal(1, 2, 16.0).unwrap();
        assert!((p.x - 100.0).abs() < 1e-3 && (p.y - 50.0).abs() < 1e-3);
        assert_eq!(m.nearest_area(Vec3::new(150.0, 150.0, 10.0)).unwrap().id, 3);
    }
}
