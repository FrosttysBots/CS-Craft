use glam::Vec3;
fn main() {
    let level = level::Level::load("de_dust2", None).unwrap();
    let c = &level.collision;
    println!("convexes {} tris {} rejected {} bounds {:?} {:?}", c.convexes.len(), c.triangles.len(), c.rejected, c.mins, c.maxs);
    let mut world = sim::World::new(c.clone());
    world.load_spawns(&level.bsp);
    for s in world.spawns.iter().take(3) {
        let o = s.origin;
        let tr = c.trace_ray(o + Vec3::Z * 10.0, o - Vec3::Z * 500.0, collision::contents::MASK_PLAYERSOLID);
        println!("spawn {:?}: ray hit frac {} end {:?} n {:?} surf {} solid {}", o, tr.fraction, tr.end, tr.normal, c.surface_name(tr.surface), tr.start_solid);
        let tr = c.trace_box(o + Vec3::Z * 10.0, o - Vec3::Z * 500.0, movement::STAND_MINS, movement::STAND_MAXS, collision::contents::MASK_PLAYERSOLID);
        println!("   box hit frac {} end {:?} n {:?} surf {}", tr.fraction, tr.end, tr.normal, c.surface_name(tr.surface));
        // brute force over all convexes/triangles near
        let near_tris = c.triangles.iter().filter(|t| t.mins.x <= o.x && t.maxs.x >= o.x && t.mins.y <= o.y && t.maxs.y >= o.y).count();
        let near_cvx: Vec<_> = c.convexes.iter().filter(|t| t.mins.x <= o.x && t.maxs.x >= o.x && t.mins.y <= o.y && t.maxs.y >= o.y).collect();
        println!("   tris under {} convexes under {}", near_tris, near_cvx.len());
        for cv in near_cvx.iter().take(4) {
            println!("     cvx {:?}..{:?} contents {:#x} planes {} surf {}", cv.mins, cv.maxs, cv.contents, cv.planes.len(), c.surface_name(cv.surface));
        }
        for t in c.triangles.iter().filter(|t| t.mins.x <= o.x && t.maxs.x >= o.x && t.mins.y <= o.y && t.maxs.y >= o.y).take(3) {
            println!("     tri {:?} n {:?}", t.v, t.normal);
        }
    }
}
