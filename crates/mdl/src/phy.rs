//! `.phy` collision models.
//!
//! A `.phy` holds one or more solids, each a "compact surface": a set of
//! convex "ledges" made of triangles over a shared point array. We only need
//! the convex pieces, so each ledge becomes a [`ConvexHull`] (its points and
//! triangle faces), converted from physics space (metres, Y down) to Source
//! units (inches, Z up).

use crate::{R, invalid};
use glam::Vec3;
use std::io;

/// Inches per metre as used by the physics conversion.
const METERS_TO_INCHES: f32 = 1.0 / 0.0254;

#[derive(Clone, Debug, Default)]
pub struct ConvexHull {
    pub points: Vec<Vec3>,
    /// Triangles as indices into `points`.
    pub triangles: Vec<[u32; 3]>,
}

impl ConvexHull {
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let mut mn = Vec3::splat(f32::MAX);
        let mut mx = Vec3::splat(f32::MIN);
        for p in &self.points {
            mn = mn.min(*p);
            mx = mx.max(*p);
        }
        (mn, mx)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Phy {
    /// Convex hulls of all solids.
    pub hulls: Vec<ConvexHull>,
    /// Trailing key/value text (surface props, masses...).
    pub text: String,
}

fn physics_to_source(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, z, -y) * METERS_TO_INCHES
}

impl Phy {
    pub fn parse(bytes: &[u8]) -> io::Result<Phy> {
        let r = R(bytes);
        let header_size = r.i32(0)?.max(0) as usize;
        let solid_count = r.i32(8)?.max(0) as usize;
        let mut phy = Phy::default();
        let mut p = header_size;
        for _ in 0..solid_count {
            let size = r.i32(p)?.max(0) as usize;
            let start = p + 4;
            let solid = bytes
                .get(start..start + size)
                .ok_or_else(|| invalid("phy solid runs past end"))?;
            parse_solid(solid, &mut phy.hulls)?;
            p = start + size;
        }
        if let Some(rest) = bytes.get(p..) {
            phy.text = String::from_utf8_lossy(rest).trim_end_matches('\0').to_string();
        }
        Ok(phy)
    }
}

fn parse_solid(solid: &[u8], hulls: &mut Vec<ConvexHull>) -> io::Result<()> {
    // Newer files wrap the surface in a "VPHY" header.
    let surface_start = if solid.get(0..4) == Some(b"VPHY") {
        let model_type = R(solid).i16(6)?;
        if model_type != 0 {
            // Only polygon soups (type 0) carry compact ledges.
            return Ok(());
        }
        28
    } else {
        0
    };
    let s = solid
        .get(surface_start..)
        .ok_or_else(|| invalid("phy surface truncated"))?;
    let r = R(s);
    if s.len() < 48 {
        return Err(invalid("phy compact surface too small"));
    }
    let ledgetree_root = r.i32(32)?.max(0) as usize;

    // Ledges are packed one after another from offset 48. The shared point
    // array follows the last ledge, so stop at the lowest point offset seen.
    let mut offset = 48usize;
    let mut points_start = ledgetree_root.min(s.len());
    let mut guard = 0;
    while offset + 16 <= points_start && guard < 100_000 {
        guard += 1;
        let point_offset = r.i32(offset)?;
        let packed = r.i32(offset + 8)? as u32;
        let size = ((packed >> 8) as usize) * 16;
        let n_triangles = r.i16(offset + 12)?.max(0) as usize;
        if size == 0 {
            break;
        }
        let points_at = (offset as i64 + point_offset as i64) as usize;
        if points_at > offset {
            points_start = points_start.min(points_at);
        }

        let mut hull = ConvexHull::default();
        let mut remap = std::collections::HashMap::new();
        for t in 0..n_triangles {
            let to = offset + 16 + t * 16;
            let mut tri = [0u32; 3];
            for (e, slot) in tri.iter_mut().enumerate() {
                let edge = r.i32(to + 4 + e * 4)? as u32;
                let point_index = (edge & 0xFFFF) as usize;
                let next = remap.len() as u32;
                let idx = *remap.entry(point_index).or_insert_with(|| {
                    let po = points_at + point_index * 16;
                    let pt = (|| -> io::Result<Vec3> {
                        Ok(physics_to_source(r.f32(po)?, r.f32(po + 4)?, r.f32(po + 8)?))
                    })()
                    .unwrap_or(Vec3::ZERO);
                    hull.points.push(pt);
                    next
                });
                *slot = idx;
            }
            hull.triangles.push(tri);
        }
        // Reject ledges whose points decoded to garbage (misparsed data).
        let sane = hull
            .points
            .iter()
            .all(|p| p.is_finite() && p.abs().max_element() < 65_536.0);
        if hull.triangles.len() >= 4 && sane {
            hulls.push(hull);
        }
        offset += size;
    }
    Ok(())
}
