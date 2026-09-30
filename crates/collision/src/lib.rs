//! Collision world: swept axis-aligned boxes and rays against convex solids.
//!
//! Every solid is either
//! * a [`Convex`] — a set of bounding planes (map brushes with their bevel
//!   planes, static prop physics hulls), or
//! * a triangle (displacement surfaces), clipped on the fly against the
//!   separating-axis planes of the box/triangle Minkowski sum.
//!
//! Sweeps use the classic plane-clipping algorithm from Quake-lineage
//! engines: each plane is pushed out by the box's support distance, and the
//! latest entry / earliest exit fractions decide the hit. All shapes live in
//! a uniform XY grid for broad-phase culling.
//!
//! Units and axes are Source's (inches, Z up).

use glam::{Mat3, Vec3};

pub use bsp::contents;

/// Distance kept between a sweep's end position and the surface it hit.
pub const DIST_EPSILON: f32 = 0.03125;
const GRID_CELL: f32 = 128.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub normal: Vec3,
    pub dist: f32,
}

/// Result of a sweep.
#[derive(Clone, Copy, Debug)]
pub struct Trace {
    /// 0..1 portion of the move completed.
    pub fraction: f32,
    pub end: Vec3,
    /// Surface normal of what was hit (zero if nothing).
    pub normal: Vec3,
    pub plane_dist: f32,
    /// The start position was inside a solid.
    pub start_solid: bool,
    /// The whole move was inside a solid.
    pub all_solid: bool,
    /// Contents of what was hit.
    pub contents: i32,
    /// Surface id of the hit shape (see [`CollisionWorld::surface_name`]).
    pub surface: u32,
}

impl Trace {
    fn empty(end: Vec3) -> Self {
        Self {
            fraction: 1.0,
            end,
            normal: Vec3::ZERO,
            plane_dist: 0.0,
            start_solid: false,
            all_solid: false,
            contents: 0,
            surface: 0,
        }
    }

    pub fn hit(&self) -> bool {
        self.fraction < 1.0 || self.start_solid
    }
}

#[derive(Clone, Debug)]
pub struct Convex {
    pub planes: Vec<Plane>,
    pub mins: Vec3,
    pub maxs: Vec3,
    pub contents: i32,
    pub surface: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Triangle {
    pub v: [Vec3; 3],
    pub normal: Vec3,
    pub mins: Vec3,
    pub maxs: Vec3,
    pub contents: i32,
    pub surface: u32,
}

#[derive(Clone, Copy, Debug)]
enum Shape {
    Convex(u32),
    Triangle(u32),
}

#[derive(Default)]
struct Grid {
    origin: [f32; 2],
    dims: [usize; 2],
    cells: Vec<Vec<Shape>>,
}

impl Grid {
    fn cell_range(&self, mins: Vec3, maxs: Vec3) -> Option<(usize, usize, usize, usize)> {
        if self.dims[0] == 0 {
            return None;
        }
        let to_cell = |v: f32, axis: usize| ((v - self.origin[axis]) / GRID_CELL).floor();
        let x0 = to_cell(mins.x, 0).max(0.0) as usize;
        let y0 = to_cell(mins.y, 1).max(0.0) as usize;
        let x1 = to_cell(maxs.x, 0);
        let y1 = to_cell(maxs.y, 1);
        if x1 < 0.0 || y1 < 0.0 {
            return None;
        }
        let x1 = (x1 as usize).min(self.dims[0] - 1);
        let y1 = (y1 as usize).min(self.dims[1] - 1);
        if x0 > x1 || y0 > y1 {
            return None;
        }
        Some((x0, y0, x1, y1))
    }
}

/// Names for surface ids (materials / surface props), index = surface id.
#[derive(Default)]
pub struct CollisionWorld {
    pub convexes: Vec<Convex>,
    pub triangles: Vec<Triangle>,
    pub surfaces: Vec<String>,
    grid: Grid,
    pub mins: Vec3,
    pub maxs: Vec3,
    /// Shapes dropped for invalid geometry.
    pub rejected: usize,
}

impl CollisionWorld {
    pub fn new() -> Self {
        Self {
            surfaces: vec![String::new()],
            ..Default::default()
        }
    }

    pub fn surface_id(&mut self, name: &str) -> u32 {
        if let Some(i) = self.surfaces.iter().position(|s| s == name) {
            return i as u32;
        }
        self.surfaces.push(name.to_string());
        (self.surfaces.len() - 1) as u32
    }

    pub fn surface_name(&self, id: u32) -> &str {
        self.surfaces.get(id as usize).map_or("", String::as_str)
    }

    /// True for skybox brushes (sunlight and sky traces pass "through" them).
    pub fn is_sky(&self, id: u32) -> bool {
        self.surface_name(id).contains("skybox")
    }

    /// Adds a convex solid. Shapes with non-finite or absurd bounds are
    /// dropped (returns false) so one bad input cannot poison the grid.
    pub fn add_convex(&mut self, c: Convex) -> bool {
        let sane = |v: Vec3| v.is_finite() && v.abs().max_element() < 131_072.0;
        if !sane(c.mins) || !sane(c.maxs) || c.planes.iter().any(|p| !p.normal.is_finite() || !p.dist.is_finite()) {
            self.rejected += 1;
            return false;
        }
        self.convexes.push(c);
        true
    }

    pub fn add_triangle(&mut self, v: [Vec3; 3], contents: i32, surface: u32) {
        let n = (v[1] - v[0]).cross(v[2] - v[0]);
        let len = n.length();
        if len < 1e-6 || !len.is_finite() || v.iter().any(|p| p.abs().max_element() >= 131_072.0) {
            self.rejected += 1;
            return;
        }
        let mins = v[0].min(v[1]).min(v[2]);
        let maxs = v[0].max(v[1]).max(v[2]);
        self.triangles.push(Triangle {
            v,
            normal: n / len,
            mins,
            maxs,
            contents,
            surface,
        });
    }

    /// Builds the broad-phase grid. Call after adding all shapes.
    pub fn finalize(&mut self) {
        let mut mn = Vec3::splat(f32::MAX);
        let mut mx = Vec3::splat(f32::MIN);
        for c in &self.convexes {
            mn = mn.min(c.mins);
            mx = mx.max(c.maxs);
        }
        for t in &self.triangles {
            mn = mn.min(t.mins);
            mx = mx.max(t.maxs);
        }
        if mn.x > mx.x {
            self.grid = Grid::default();
            return;
        }
        self.mins = mn;
        self.maxs = mx;
        let dims = [
            (((mx.x - mn.x) / GRID_CELL).floor() as usize + 1).min(4096),
            (((mx.y - mn.y) / GRID_CELL).floor() as usize + 1).min(4096),
        ];
        let mut grid = Grid {
            origin: [mn.x, mn.y],
            dims,
            cells: vec![Vec::new(); dims[0] * dims[1]],
        };
        let insert = |shape: Shape, mins: Vec3, maxs: Vec3, grid: &mut Grid| {
            if let Some((x0, y0, x1, y1)) = grid.cell_range(mins, maxs) {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        grid.cells[y * grid.dims[0] + x].push(shape);
                    }
                }
            }
        };
        for (i, c) in self.convexes.iter().enumerate() {
            insert(Shape::Convex(i as u32), c.mins, c.maxs, &mut grid);
        }
        for (i, t) in self.triangles.iter().enumerate() {
            insert(Shape::Triangle(i as u32), t.mins, t.maxs, &mut grid);
        }
        self.grid = grid;
    }

    fn candidates(&self, mins: Vec3, maxs: Vec3, out: &mut Vec<Shape>) {
        out.clear();
        let Some((x0, y0, x1, y1)) = self.grid.cell_range(mins, maxs) else {
            return;
        };
        let single = x0 == x1 && y0 == y1;
        for y in y0..=y1 {
            for x in x0..=x1 {
                out.extend_from_slice(&self.grid.cells[y * self.grid.dims[0] + x]);
            }
        }
        if !single {
            out.sort_unstable_by_key(|s| match *s {
                Shape::Convex(i) => (0u8, i),
                Shape::Triangle(i) => (1u8, i),
            });
            out.dedup_by_key(|s| match *s {
                Shape::Convex(i) => (0u8, i),
                Shape::Triangle(i) => (1u8, i),
            });
        }
    }

    /// Sweeps a box `mins..maxs` (relative to the position) from `start` to `end`.
    pub fn trace_box(&self, start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3, mask: i32) -> Trace {
        let offset = (mins + maxs) * 0.5;
        let ext = (maxs - mins) * 0.5;
        let s = start + offset;
        let e = end + offset;
        let mut tr = Trace::empty(end);

        // Long sweeps are split into short segments walked in order, so a
        // bullet or shadow ray only visits the grid cells it passes and can
        // stop at the first segment that contains a hit.
        let length = (e - s).length();
        let segments = ((length / (GRID_CELL * 2.0)).ceil() as usize).clamp(1, 1024);
        let mut cands = Vec::with_capacity(64);
        let mut scratch = Vec::with_capacity(32);
        let mut seen: std::collections::HashSet<(u8, u32)> = std::collections::HashSet::new();
        for seg in 0..segments {
            let f0 = seg as f32 / segments as f32;
            let f1 = (seg + 1) as f32 / segments as f32;
            let a = s.lerp(e, f0);
            let b = s.lerp(e, f1);
            let sweep_min = a.min(b) - ext - Vec3::ONE;
            let sweep_max = a.max(b) + ext + Vec3::ONE;
            self.candidates(sweep_min, sweep_max, &mut cands);
            for &shape in &cands {
                if segments > 1 {
                    let key = match shape {
                        Shape::Convex(i) => (0u8, i),
                        Shape::Triangle(i) => (1u8, i),
                    };
                    if !seen.insert(key) {
                        continue;
                    }
                }
                match shape {
                    Shape::Convex(i) => {
                        let c = &self.convexes[i as usize];
                        if c.contents & mask == 0 || !overlaps(c.mins, c.maxs, sweep_min, sweep_max) {
                            if segments > 1 {
                                seen.remove(&(0, i));
                            }
                            continue;
                        }
                        clip_convex(&c.planes, s, e, ext, c.contents, c.surface, &mut tr);
                    }
                    Shape::Triangle(i) => {
                        let t = &self.triangles[i as usize];
                        if t.contents & mask == 0 || !overlaps(t.mins, t.maxs, sweep_min, sweep_max) {
                            if segments > 1 {
                                seen.remove(&(1, i));
                            }
                            continue;
                        }
                        triangle_planes(t, &mut scratch);
                        clip_convex(&scratch, s, e, ext, t.contents, t.surface, &mut tr);
                    }
                }
                if tr.all_solid {
                    break;
                }
            }
            // Anything later in the sweep cannot beat a hit inside this segment.
            if tr.all_solid || tr.fraction <= f1 {
                break;
            }
        }
        if tr.fraction < 1.0 {
            tr.end = start + (end - start) * tr.fraction;
        } else {
            tr.end = end;
        }
        if tr.all_solid {
            tr.end = start;
        }
        tr
    }

    /// Sweeps a point.
    pub fn trace_ray(&self, start: Vec3, end: Vec3, mask: i32) -> Trace {
        self.trace_box(start, end, Vec3::ZERO, Vec3::ZERO, mask)
    }

    /// Contents at a point (union of all shapes containing it).
    pub fn point_contents(&self, p: Vec3, mask: i32) -> i32 {
        let mut cands = Vec::new();
        self.candidates(p - Vec3::ONE, p + Vec3::ONE, &mut cands);
        let mut out = 0;
        for shape in cands {
            if let Shape::Convex(i) = shape {
                let c = &self.convexes[i as usize];
                if c.contents & mask != 0 && c.planes.iter().all(|pl| pl.normal.dot(p) - pl.dist <= 0.0) {
                    out |= c.contents;
                }
            }
        }
        out
    }
}

fn overlaps(amin: Vec3, amax: Vec3, bmin: Vec3, bmax: Vec3) -> bool {
    amin.x <= bmax.x && amax.x >= bmin.x && amin.y <= bmax.y && amax.y >= bmin.y && amin.z <= bmax.z && amax.z >= bmin.z
}

/// Clips a sweep of a centred box with half-extents `ext` against the convex
/// region `n·x <= d` for every plane, updating `tr` when it hits earlier.
fn clip_convex(planes: &[Plane], s: Vec3, e: Vec3, ext: Vec3, contents: i32, surface: u32, tr: &mut Trace) {
    let mut enter = -1.0f32;
    let mut leave = 1.0f32;
    let mut clip: Option<Plane> = None;
    let mut get_out = false;
    let mut start_out = false;
    for p in planes {
        let n = p.normal;
        let dist = p.dist + n.x.abs() * ext.x + n.y.abs() * ext.y + n.z.abs() * ext.z;
        let d1 = n.dot(s) - dist;
        let d2 = n.dot(e) - dist;
        if d2 > 0.0 {
            get_out = true;
        }
        if d1 > 0.0 {
            start_out = true;
        }
        // Entirely in front of this plane: no contact with the solid.
        if d1 > 0.0 && (d2 >= DIST_EPSILON || d2 >= d1) {
            return;
        }
        if d1 <= 0.0 && d2 <= 0.0 {
            continue;
        }
        if d1 > d2 {
            let f = ((d1 - DIST_EPSILON) / (d1 - d2)).max(0.0);
            if f > enter {
                enter = f;
                clip = Some(Plane { normal: n, dist });
            }
        } else {
            let f = ((d1 + DIST_EPSILON) / (d1 - d2)).min(1.0);
            if f < leave {
                leave = f;
            }
        }
    }
    if !start_out {
        tr.start_solid = true;
        tr.contents |= contents;
        if !get_out {
            tr.all_solid = true;
            tr.fraction = 0.0;
            tr.surface = surface;
        }
        return;
    }
    if enter < leave && enter > -1.0 && enter < tr.fraction {
        if let Some(p) = clip {
            tr.fraction = enter.max(0.0);
            tr.normal = p.normal;
            tr.plane_dist = p.dist;
            tr.contents = contents;
            tr.surface = surface;
        }
    }
}

/// Thickness given to triangles behind their front face, so rays and thin
/// sweeps cannot slip through a zero-volume surface.
const TRIANGLE_THICKNESS: f32 = 1.0;

/// Separating-axis planes of a triangle for box sweeps: its two faces, the
/// three box axes, and the nine edge × axis directions (both signs each).
fn triangle_planes(t: &Triangle, out: &mut Vec<Plane>) {
    out.clear();
    let d = t.normal.dot(t.v[0]);
    out.push(Plane { normal: t.normal, dist: d });
    out.push(Plane { normal: -t.normal, dist: -d + TRIANGLE_THICKNESS });
    let push_axis = |axis: Vec3, out: &mut Vec<Plane>| {
        let len_sq = axis.length_squared();
        if len_sq < 1e-8 {
            return;
        }
        let a = axis / len_sq.sqrt();
        // Already covered by the (thickened) face planes.
        if a.dot(t.normal).abs() > 0.9999 {
            return;
        }
        let d = t.v.iter().map(|v| a.dot(*v));
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for x in d {
            lo = lo.min(x);
            hi = hi.max(x);
        }
        out.push(Plane { normal: a, dist: hi });
        out.push(Plane { normal: -a, dist: -lo });
    };
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        push_axis(axis, out);
    }
    for i in 0..3 {
        let edge = t.v[(i + 1) % 3] - t.v[i];
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            push_axis(edge.cross(axis), out);
        }
    }
}

/// Builds a convex from hull points and triangles: face planes, axial
/// bevels and edge bevels, each placed on the hull's supporting plane.
pub fn convex_from_hull(points: &[Vec3], triangles: &[[u32; 3]], contents: i32, surface: u32) -> Option<Convex> {
    if points.len() < 4 {
        return None;
    }
    let mut mins = Vec3::splat(f32::MAX);
    let mut maxs = Vec3::splat(f32::MIN);
    for p in points {
        mins = mins.min(*p);
        maxs = maxs.max(*p);
    }
    let mut planes: Vec<Plane> = Vec::new();
    let add = |dir: Vec3, planes: &mut Vec<Plane>| {
        let len = dir.length();
        if len < 1e-6 {
            return;
        }
        let n = dir / len;
        let d = points.iter().map(|p| n.dot(*p)).fold(f32::MIN, f32::max);
        if planes
            .iter()
            .any(|p| p.normal.dot(n) > 0.9999 && (p.dist - d).abs() < 0.01)
        {
            return;
        }
        planes.push(Plane { normal: n, dist: d });
    };
    let centroid = points.iter().copied().sum::<Vec3>() / points.len() as f32;
    for tri in triangles {
        let [a, b, c] = tri.map(|i| points[i as usize]);
        let mut n = (b - a).cross(c - a);
        // Orient outward.
        if n.dot(a - centroid) < 0.0 {
            n = -n;
        }
        add(n, &mut planes);
    }
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        add(axis, &mut planes);
        add(-axis, &mut planes);
    }
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for tri in triangles {
        for i in 0..3 {
            let (a, b) = (tri[i], tri[(i + 1) % 3]);
            let key = (a.min(b), a.max(b));
            if !edges.contains(&key) {
                edges.push(key);
            }
        }
    }
    for (a, b) in edges {
        let e = points[b as usize] - points[a as usize];
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let n = e.cross(axis);
            add(n, &mut planes);
            add(-n, &mut planes);
        }
    }
    Some(Convex {
        planes,
        mins,
        maxs,
        contents,
        surface,
    })
}

/// Options for building a collision world from a map.
pub struct MapCollisionOptions {
    /// Brush entities whose brushes are solid (func_brush, func_wall, ...).
    pub solid_brush_classes: Vec<&'static str>,
}

impl Default for MapCollisionOptions {
    fn default() -> Self {
        Self {
            solid_brush_classes: vec![
                "func_brush",
                "func_wall",
                "func_detail_blocker",
                "func_door",
                "func_door_rotating",
                "func_breakable",
                "func_breakable_surf",
                "func_physbox",
                "func_movelinear",
                "func_rotating",
                "func_tracktrain",
                "func_clip_vphysics",
            ],
        }
    }
}

/// Loads a static prop's hulls; returns `None` if the prop is not solid.
pub type PropHullLoader<'a> = dyn Fn(&str) -> Option<Vec<mdl::ConvexHull>> + 'a;

impl CollisionWorld {
    /// Adds the world brushes, solid brush entities, displacements and solid
    /// static props of a map. `prop_hulls` loads a model's `.phy` hulls.
    pub fn from_bsp(map: &bsp::Bsp, opts: &MapCollisionOptions, prop_hulls: &PropHullLoader) -> Self {
        let mut w = CollisionWorld::new();

        // Brushes of model 0 (the world) and of solid brush entities.
        let add_model_brushes = |w: &mut CollisionWorld, model: usize, origin: Vec3| {
            let Some(m) = map.models.get(model) else { return };
            let mut brushes: Vec<u16> = Vec::new();
            collect_leaf_brushes(map, m.head_node, &mut brushes);
            brushes.sort_unstable();
            brushes.dedup();
            for bi in brushes {
                let b = &map.brushes[bi as usize];
                if b.contents & contents::MASK_SOLID == 0 && b.contents & contents::PLAYERCLIP == 0 && b.contents & contents::LADDER == 0 {
                    continue;
                }
                let mut planes = Vec::with_capacity(b.num_sides as usize);
                let mut surface_name = String::new();
                for s in 0..b.num_sides.max(0) as usize {
                    let side = &map.brush_sides[b.first_side as usize + s];
                    let p = &map.planes[side.plane as usize];
                    planes.push(Plane {
                        normal: p.normal,
                        dist: p.dist + p.normal.dot(origin),
                    });
                    if surface_name.is_empty() && !side.bevel && side.texinfo >= 0 {
                        if let Some(name) = map.texinfo_material(side.texinfo as usize) {
                            surface_name = name.to_ascii_lowercase();
                        }
                    }
                }
                let Some((mins, maxs)) = brush_bounds(&planes) else { continue };
                let surface = w.surface_id(&surface_name);
                w.add_convex(Convex {
                    planes,
                    mins,
                    maxs,
                    contents: b.contents,
                    surface,
                });
            }
        };
        add_model_brushes(&mut w, 0, Vec3::ZERO);
        for ent in &map.entities {
            let Some(class) = ent.str("classname") else { continue };
            if !opts.solid_brush_classes.iter().any(|c| c.eq_ignore_ascii_case(class)) {
                continue;
            }
            // func_brush with "Solidity: never solid" (1).
            if class.eq_ignore_ascii_case("func_brush") && ent.i64("solidity") == Some(1) {
                continue;
            }
            let Some(model) = ent.str("model").and_then(|m| m.strip_prefix('*')).and_then(|n| n.parse::<usize>().ok()) else {
                continue;
            };
            let origin = ent.str("origin").and_then(bsp::parse_origin).unwrap_or(Vec3::ZERO);
            add_model_brushes(&mut w, model, origin);
        }

        // Displacements.
        for (fi, face) in map.faces.iter().enumerate() {
            if face.dispinfo < 0 {
                continue;
            }
            let Some(mesh) = map.displacement(fi) else { continue };
            let surface_name = map
                .texinfo_material(face.texinfo as usize)
                .unwrap_or("")
                .to_ascii_lowercase();
            let surface = w.surface_id(&surface_name);
            for tri in mesh.indices.chunks_exact(3) {
                w.add_triangle(
                    [
                        mesh.positions[tri[0] as usize],
                        mesh.positions[tri[1] as usize],
                        mesh.positions[tri[2] as usize],
                    ],
                    contents::SOLID,
                    surface,
                );
            }
        }

        // Static props (solid 6 = vphysics).
        let mut cache: std::collections::HashMap<u16, Option<Vec<mdl::ConvexHull>>> = Default::default();
        for prop in &map.static_props.props {
            if prop.solid != 6 {
                continue;
            }
            let hulls = cache.entry(prop.model).or_insert_with(|| {
                map.static_props
                    .models
                    .get(prop.model as usize)
                    .and_then(|name| prop_hulls(name))
            });
            let Some(hulls) = hulls else { continue };
            let rot: Mat3 = mdl::angles_to_mat3(prop.angles);
            let scale = if prop.scale > 0.0 { prop.scale } else { 1.0 };
            let name = map
                .static_props
                .models
                .get(prop.model as usize)
                .cloned()
                .unwrap_or_default();
            let surface = w.surface_id(&name);
            for hull in hulls.iter() {
                let pts: Vec<Vec3> = hull.points.iter().map(|p| prop.origin + rot * (*p * scale)).collect();
                if let Some(c) = convex_from_hull(&pts, &hull.triangles, contents::SOLID, surface) {
                    w.add_convex(c);
                }
            }
        }
        w.finalize();
        w
    }
}

fn collect_leaf_brushes(map: &bsp::Bsp, node: i32, out: &mut Vec<u16>) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n < 0 {
            let leaf = &map.leafs[(-1 - n) as usize];
            let first = leaf.first_leaf_brush as usize;
            let count = leaf.num_leaf_brushes as usize;
            out.extend_from_slice(&map.leaf_brushes[first..first + count]);
        } else {
            let node = &map.nodes[n as usize];
            stack.push(node.children[0]);
            stack.push(node.children[1]);
        }
    }
}

/// Bounds of a brush from its axial planes (Source brushes always carry all
/// six axial planes as sides or bevels).
fn brush_bounds(planes: &[Plane]) -> Option<(Vec3, Vec3)> {
    let mut mins = Vec3::splat(f32::MIN);
    let mut maxs = Vec3::splat(f32::MAX);
    let mut found = [false; 6];
    for p in planes {
        for axis in 0..3 {
            let n = p.normal[axis];
            let others = (0..3).filter(|&a| a != axis).all(|a| p.normal[a].abs() < 1e-5);
            if !others {
                continue;
            }
            if n > 0.9999 {
                maxs[axis] = p.dist;
                found[axis * 2] = true;
            } else if n < -0.9999 {
                mins[axis] = -p.dist;
                found[axis * 2 + 1] = true;
            }
        }
    }
    found.iter().all(|&f| f).then_some((mins, maxs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_convex(mins: Vec3, maxs: Vec3) -> Convex {
        Convex {
            planes: vec![
                Plane { normal: Vec3::X, dist: maxs.x },
                Plane { normal: -Vec3::X, dist: -mins.x },
                Plane { normal: Vec3::Y, dist: maxs.y },
                Plane { normal: -Vec3::Y, dist: -mins.y },
                Plane { normal: Vec3::Z, dist: maxs.z },
                Plane { normal: -Vec3::Z, dist: -mins.z },
            ],
            mins,
            maxs,
            contents: contents::SOLID,
            surface: 0,
        }
    }

    fn floor_world() -> CollisionWorld {
        let mut w = CollisionWorld::new();
        w.add_convex(box_convex(Vec3::new(-512.0, -512.0, -16.0), Vec3::new(512.0, 512.0, 0.0)));
        w.add_convex(box_convex(Vec3::new(100.0, -512.0, 0.0), Vec3::new(132.0, 512.0, 128.0)));
        w.finalize();
        w
    }

    const HULL_MIN: Vec3 = Vec3::new(-16.0, -16.0, 0.0);
    const HULL_MAX: Vec3 = Vec3::new(16.0, 16.0, 72.0);

    #[test]
    fn box_lands_on_floor() {
        let w = floor_world();
        let tr = w.trace_box(Vec3::new(0.0, 0.0, 64.0), Vec3::new(0.0, 0.0, -64.0), HULL_MIN, HULL_MAX, contents::MASK_PLAYERSOLID);
        assert!(tr.fraction < 1.0);
        assert!((tr.end.z - 0.0).abs() < 0.1, "end {:?}", tr.end);
        assert_eq!(tr.normal, Vec3::Z);
        assert!(!tr.start_solid);
    }

    #[test]
    fn box_stops_at_wall() {
        let w = floor_world();
        let tr = w.trace_box(Vec3::new(0.0, 0.0, 1.0), Vec3::new(200.0, 0.0, 1.0), HULL_MIN, HULL_MAX, contents::MASK_PLAYERSOLID);
        assert!((tr.end.x - 84.0).abs() < 0.1, "end {:?}", tr.end);
        assert_eq!(tr.normal, -Vec3::X);
    }

    #[test]
    fn start_solid_detected() {
        let w = floor_world();
        let tr = w.trace_box(Vec3::new(116.0, 0.0, 10.0), Vec3::new(116.0, 0.0, 20.0), HULL_MIN, HULL_MAX, contents::MASK_PLAYERSOLID);
        assert!(tr.start_solid && tr.all_solid);
    }

    #[test]
    fn triangle_ramp() {
        let mut w = CollisionWorld::new();
        // 45-degree ramp rising along +X.
        w.add_triangle([Vec3::new(0.0, -100.0, 0.0), Vec3::new(100.0, -100.0, 100.0), Vec3::new(0.0, 100.0, 0.0)], contents::SOLID, 0);
        w.add_triangle([Vec3::new(100.0, -100.0, 100.0), Vec3::new(100.0, 100.0, 100.0), Vec3::new(0.0, 100.0, 0.0)], contents::SOLID, 0);
        w.finalize();
        let tr = w.trace_ray(Vec3::new(25.0, 10.0, 200.0), Vec3::new(25.0, 10.0, -200.0), contents::MASK_SOLID);
        assert!((tr.end.z - 25.0).abs() < 0.2, "end {:?}", tr.end);
        assert!(tr.normal.z > 0.7 && tr.normal.x < 0.0);
        // A box resting on the ramp is held up by its lower edge.
        let tr = w.trace_box(Vec3::new(50.0, 0.0, 200.0), Vec3::new(50.0, 0.0, -200.0), HULL_MIN, HULL_MAX, contents::MASK_SOLID);
        assert!((tr.end.z - 66.0).abs() < 0.2, "end {:?}", tr.end);
    }

    #[test]
    fn hull_convex_blocks() {
        // Unit-ish tetrahedron scaled up.
        let pts = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(64.0, 0.0, 0.0), Vec3::new(0.0, 64.0, 0.0), Vec3::new(0.0, 0.0, 64.0)];
        let tris = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
        let c = convex_from_hull(&pts, &tris, contents::SOLID, 0).unwrap();
        let mut w = CollisionWorld::new();
        w.add_convex(c);
        w.finalize();
        let tr = w.trace_ray(Vec3::new(10.0, 10.0, 100.0), Vec3::new(10.0, 10.0, -10.0), contents::MASK_SOLID);
        assert!((tr.end.z - 44.0).abs() < 0.2, "end {:?}", tr.end);
    }
}

#[cfg(test)]
mod long_trace_tests {
    use super::*;

    #[test]
    fn long_ray_hits_far_wall_and_near_wall_first() {
        let mut w = CollisionWorld::new();
        let wall = |x0: f32| Convex {
            planes: vec![
                Plane { normal: Vec3::X, dist: x0 + 16.0 },
                Plane { normal: -Vec3::X, dist: -x0 },
                Plane { normal: Vec3::Y, dist: 512.0 },
                Plane { normal: -Vec3::Y, dist: 512.0 },
                Plane { normal: Vec3::Z, dist: 512.0 },
                Plane { normal: -Vec3::Z, dist: 512.0 },
            ],
            mins: Vec3::new(x0, -512.0, -512.0),
            maxs: Vec3::new(x0 + 16.0, 512.0, 512.0),
            contents: contents::SOLID,
            surface: 0,
        };
        w.add_convex(wall(3000.0));
        w.add_convex(wall(9000.0));
        w.finalize();
        let tr = w.trace_ray(Vec3::ZERO, Vec3::new(12000.0, 0.0, 0.0), contents::MASK_SOLID);
        assert!((tr.end.x - 3000.0).abs() < 0.1, "{:?}", tr.end);
        let tr = w.trace_ray(Vec3::new(3100.0, 0.0, 0.0), Vec3::new(12000.0, 0.0, 0.0), contents::MASK_SOLID);
        assert!((tr.end.x - 9000.0).abs() < 0.1, "{:?}", tr.end);
        let tr = w.trace_ray(Vec3::new(12000.0, 0.0, 0.0), Vec3::new(3100.0, 0.0, 0.0), contents::MASK_SOLID);
        assert!((tr.end.x - 9016.0).abs() < 0.1, "{:?}", tr.end);
    }
}
