//! Displacement surface tessellation.
//!
//! A displacement replaces a four-sided face with a `(2^p+1)²` vertex grid.
//! The grid is interpolated from the face's corners, starting at the corner
//! nearest `DispInfo::start_position`, and each vertex is pushed along its
//! stored offset vector.

use crate::{Bsp, DispInfo};
use glam::{Vec2, Vec3};

#[derive(Clone, Debug)]
pub struct DispMesh {
    /// Vertices per side.
    pub side: usize,
    /// Displaced positions, row-major `side * side`.
    pub positions: Vec<Vec3>,
    /// The flat, undisplaced positions (used for texture mapping).
    pub base_positions: Vec<Vec3>,
    /// Blend alpha 0..1 (second texture weight in WorldVertexTransition).
    pub alphas: Vec<f32>,
    /// Lightmap coordinates in luxels relative to the face's lightmap mins.
    pub luxel_coords: Vec<Vec2>,
    /// Triangle list, wound so the geometric normal faces the face's front.
    pub indices: Vec<u32>,
    /// `DISPTRI_TAG_*` flags per triangle.
    pub tri_tags: Vec<u16>,
    pub info: DispInfo,
}

/// Triangle tag bits.
pub mod tags {
    pub const SURFACE: u16 = 0x1;
    pub const WALKABLE: u16 = 0x2;
    pub const BUILDABLE: u16 = 0x4;
}

pub fn build(bsp: &Bsp, face_index: usize) -> Option<DispMesh> {
    let face = bsp.faces.get(face_index)?;
    if face.dispinfo < 0 {
        return None;
    }
    let info = *bsp.dispinfos.get(face.dispinfo as usize)?;
    let corners = bsp.face_vertices(face);
    if corners.len() != 4 {
        return None;
    }


    // Rotate so corner 0 is the displacement's start corner.
    let start = (0..4)
        .min_by(|&a, &b| {
            let da = corners[a].distance_squared(info.start_position);
            let db = corners[b].distance_squared(info.start_position);
            da.total_cmp(&db)
        })
        .unwrap_or(0);
    let c: [Vec3; 4] = std::array::from_fn(|i| corners[(start + i) % 4]);

    // Displacements have a rectangular lightmap parameterised over their
    // grid, starting at start_position. Brush texture/lightmap vectors may
    // be rotated or skewed and do not describe this grid. Projecting them
    // here samples neighbouring faces (or empty atlas pixels).
    let lightmap_extent = Vec2::new(face.lightmap_size[0].max(0) as f32, face.lightmap_size[1].max(0) as f32);

    let n = info.side_len();
    let count = n * n;
    let first = info.disp_vert_start.max(0) as usize;
    let verts = bsp.disp_verts.get(first..first + count)?;

    let mut positions = Vec::with_capacity(count);
    let mut base_positions = Vec::with_capacity(count);
    let mut alphas = Vec::with_capacity(count);
    let mut luxel_coords = Vec::with_capacity(count);
    let denom = (n - 1) as f32;
    for y in 0..n {
        let ty = y as f32 / denom;
        let a = c[0].lerp(c[1], ty);
        let b = c[3].lerp(c[2], ty);
        for x in 0..n {
            let tx = x as f32 / denom;
            let base = a.lerp(b, tx);
            let dv = &verts[y * n + x];
            positions.push(base + dv.vec * dv.dist);
            base_positions.push(base);
            alphas.push((dv.alpha / 255.0).clamp(0.0, 1.0));
            luxel_coords.push(Vec2::new(tx, ty) * lightmap_extent);
        }
    }

    // Orientation reference: the face's plane normal. It already points the
    // way the face looks; `side` only relates the face to its BSP node.
    let front = bsp.face_normal(face)?;

    let mut indices = Vec::with_capacity((n - 1) * (n - 1) * 6);
    let mut push_tri = |i0: usize, i1: usize, i2: usize, base: &[Vec3]| {
        let nrm = (base[i1] - base[i0]).cross(base[i2] - base[i0]);
        if nrm.dot(front) >= 0.0 {
            indices.extend_from_slice(&[i0 as u32, i1 as u32, i2 as u32]);
        } else {
            indices.extend_from_slice(&[i0 as u32, i2 as u32, i1 as u32]);
        }
    };
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let i = y * n + x;
            let (i00, i10, i01, i11) = (i, i + 1, i + n, i + n + 1);
            // Alternate the split diagonal in a checkerboard, as the
            // engine's tessellation does, to avoid directional artefacts.
            if (x + y) % 2 == 0 {
                push_tri(i00, i01, i11, &base_positions);
                push_tri(i00, i11, i10, &base_positions);
            } else {
                push_tri(i00, i01, i10, &base_positions);
                push_tri(i10, i01, i11, &base_positions);
            }
        }
    }

    let tri_count = (n - 1) * (n - 1) * 2;
    let tri_start = info.disp_tri_start.max(0) as usize;
    let tri_tags = bsp
        .disp_tris
        .get(tri_start..tri_start + tri_count)
        .map(<[u16]>::to_vec)
        .unwrap_or_else(|| vec![tags::SURFACE; tri_count]);

    Some(DispMesh {
        side: n,
        positions,
        base_positions,
        alphas,
        luxel_coords,
        indices,
        tri_tags,
        info,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Face, Plane, DispVert};
    #[test]
    fn skewed_displacement_uses_grid_lightmap_not_brush_projection() {
        let mut bsp = Bsp::default();
        bsp.vertices = vec![Vec3::new(0.0,0.0,0.0), Vec3::new(0.0,320.0,0.0), Vec3::new(448.0,320.0,0.0), Vec3::new(448.0,64.0,0.0)];
        bsp.edges = vec![[0,1],[1,2],[2,3],[3,0]];
        bsp.surfedges = vec![0,1,2,3];
        bsp.planes = vec![Plane { normal: Vec3::Z, dist: 0.0, kind: 2 }];
        bsp.faces = vec![Face { plane: 0, side: 0, first_edge: 0, num_edges: 4, texinfo: 0, dispinfo: 0, styles: [0,255,255,255], light_offset: 0, lightmap_mins: [-128,-126], lightmap_size: [57,41] }];
        bsp.dispinfos = vec![DispInfo { start_position: Vec3::ZERO, disp_vert_start: 0, disp_tri_start: 0, power: 2, flags: 0, contents: 1, map_face: 0 }];
        bsp.disp_verts = vec![DispVert { vec: Vec3::Z, dist: 0.0, alpha: 0.0 };25];
        let mesh = build(&bsp,0).unwrap();
        assert_eq!(mesh.luxel_coords[0], Vec2::ZERO);
        assert_eq!(mesh.luxel_coords[4], Vec2::new(57.0,0.0));
        assert_eq!(mesh.luxel_coords[20], Vec2::new(0.0,41.0));
        assert_eq!(mesh.luxel_coords[24], Vec2::new(57.0,41.0));
        assert_eq!(mesh.luxel_coords[12], Vec2::new(28.5,20.5));
    }
}
