use super::*;
use collision::{CollisionWorld, Convex, Plane};

const DT: f32 = 1.0 / 64.0;

fn aabb(mins: Vec3, maxs: Vec3) -> Convex {
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

/// A big floor, a 16-unit step at x=512 and a wall at x=2048.
fn test_world() -> CollisionWorld {
    let mut w = CollisionWorld::new();
    w.add_convex(aabb(Vec3::new(-4096.0, -4096.0, -64.0), Vec3::new(4096.0, 4096.0, 0.0)));
    w.add_convex(aabb(Vec3::new(512.0, -4096.0, 0.0), Vec3::new(4096.0, 4096.0, 16.0)));
    w.add_convex(aabb(Vec3::new(2048.0, -4096.0, 16.0), Vec3::new(2080.0, 4096.0, 512.0)));
    w.finalize();
    w
}

fn run(pm: &mut PlayerMove, input: MoveInput, ticks: usize, w: &CollisionWorld) -> Vec<MoveEvents> {
    let cfg = MoveConfig::default();
    (0..ticks).map(|_| player_move(pm, &input, &cfg, w, DT)).collect()
}

fn forward(speed: f32) -> MoveInput {
    MoveInput {
        view_angles: Vec3::ZERO,
        forward_move: 450.0,
        weapon_max_speed: speed,
        ..Default::default()
    }
}

#[test]
fn falls_and_lands() {
    let w = test_world();
    let mut pm = PlayerMove::spawn_at(Vec3::new(0.0, 0.0, 100.0));
    let events = run(&mut pm, MoveInput::default(), 64, &w);
    assert!(pm.on_ground);
    assert!(pm.origin.z.abs() < 0.1, "{:?}", pm.origin);
    assert!(events.iter().any(|e| e.landed.is_some()));
}

#[test]
fn runs_at_knife_speed() {
    let w = test_world();
    let mut pm = PlayerMove::spawn_at(Vec3::new(-2000.0, 0.0, 1.0));
    run(&mut pm, MoveInput::default(), 4, &w);
    run(&mut pm, forward(250.0), 64, &w);
    let speed = pm.speed_2d();
    assert!((speed - 250.0).abs() < 1.0, "speed {speed}");
}

#[test]
fn walking_and_crouching_are_slower() {
    let w = test_world();
    let mut pm = PlayerMove::spawn_at(Vec3::new(-2000.0, 0.0, 1.0));
    run(&mut pm, MoveInput::default(), 4, &w);
    let mut input = forward(250.0);
    input.buttons = buttons::WALK;
    run(&mut pm, input, 64, &w);
    assert!((pm.speed_2d() - 130.0).abs() < 1.0, "walk speed {}", pm.speed_2d());

    let mut pm = PlayerMove::spawn_at(Vec3::new(-2000.0, 0.0, 1.0));
    run(&mut pm, MoveInput::default(), 4, &w);
    input.buttons = buttons::DUCK;
    run(&mut pm, input, 64, &w);
    assert!(pm.ducked);
    assert!((pm.speed_2d() - 85.0).abs() < 1.0, "duck speed {}", pm.speed_2d());
}

#[test]
fn stops_with_friction() {
    let w = test_world();
    let mut pm = PlayerMove::spawn_at(Vec3::new(-2000.0, 0.0, 1.0));
    run(&mut pm, MoveInput::default(), 4, &w);
    run(&mut pm, forward(250.0), 64, &w);
    run(&mut pm, MoveInput { weapon_max_speed: 250.0, ..Default::default() }, 32, &w);
    assert_eq!(pm.speed_2d(), 0.0);
}

#[test]
fn jump_height_is_about_57_units() {
    let w = test_world();
    let mut pm = PlayerMove::spawn_at(Vec3::new(-2000.0, 0.0, 1.0));
    run(&mut pm, MoveInput::default(), 16, &w);
    let jump = MoveInput { buttons: buttons::JUMP, weapon_max_speed: 250.0, ..Default::default() };
    let cfg = MoveConfig::default();
    let mut peak: f32 = 0.0;
    let ev = player_move(&mut pm, &jump, &cfg, &w, DT);
    assert!(ev.jumped);
    for _ in 0..80 {
        player_move(&mut pm, &MoveInput::default(), &cfg, &w, DT);
        peak = peak.max(pm.origin.z);
    }
    assert!((peak - 55.0).abs() < 4.0, "peak {peak}");
    assert!(pm.on_ground);
}

#[test]
fn holding_jump_does_not_rehop() {
    let w = test_world();
    let mut pm = PlayerMove::spawn_at(Vec3::new(-2000.0, 0.0, 1.0));
    run(&mut pm, MoveInput::default(), 16, &w);
    let jump = MoveInput { buttons: buttons::JUMP, weapon_max_speed: 250.0, ..Default::default() };
    let events = run(&mut pm, jump, 200, &w);
    assert_eq!(events.iter().filter(|e| e.jumped).count(), 1);
}

#[test]
fn steps_up_small_ledges() {
    let w = test_world();
    let mut pm = PlayerMove::spawn_at(Vec3::new(400.0, 0.0, 1.0));
    run(&mut pm, MoveInput::default(), 4, &w);
    run(&mut pm, forward(250.0), 64, &w);
    assert!(pm.origin.x > 600.0, "{:?}", pm.origin);
    assert!((pm.origin.z - 16.0).abs() < 0.1, "{:?}", pm.origin);
    assert!(pm.speed_2d() > 200.0, "lost speed on step: {}", pm.speed_2d());
}

#[test]
fn wall_stops_movement() {
    let w = test_world();
    let mut pm = PlayerMove::spawn_at(Vec3::new(1900.0, 0.0, 17.0));
    run(&mut pm, MoveInput::default(), 4, &w);
    run(&mut pm, forward(250.0), 128, &w);
    assert!((pm.origin.x - (2048.0 - 16.0)).abs() < 0.5, "{:?}", pm.origin);
    assert!(pm.speed_2d() < 1.0);
}

#[test]
fn air_strafing_gains_speed() {
    let w = test_world();
    let cfg = MoveConfig::default();
    let mut pm = PlayerMove::spawn_at(Vec3::new(-3000.0, 0.0, 400.0));
    pm.velocity = Vec3::new(250.0, 0.0, 0.0);
    let start = pm.speed_2d();
    // Turn right while holding right strafe: the classic air strafe.
    let mut yaw = 0.0f32;
    for _ in 0..40 {
        yaw -= 1.5;
        let input = MoveInput {
            view_angles: Vec3::new(0.0, yaw, 0.0),
            side_move: 450.0,
            weapon_max_speed: 250.0,
            ..Default::default()
        };
        player_move(&mut pm, &input, &cfg, &w, DT);
    }
    assert!(pm.speed_2d() > start + 20.0, "{} -> {}", start, pm.speed_2d());
}

#[test]
fn crouch_jump_lifts_feet() {
    let w = test_world();
    let cfg = MoveConfig::default();
    let mut pm = PlayerMove::spawn_at(Vec3::new(-2000.0, 0.0, 1.0));
    run(&mut pm, MoveInput::default(), 8, &w);
    let jump = MoveInput { buttons: buttons::JUMP, weapon_max_speed: 250.0, ..Default::default() };
    player_move(&mut pm, &jump, &cfg, &w, DT);
    let before = pm.origin.z;
    let duck = MoveInput { buttons: buttons::DUCK, weapon_max_speed: 250.0, ..Default::default() };
    player_move(&mut pm, &duck, &cfg, &w, DT);
    assert!(pm.ducked);
    assert!(pm.origin.z > before + 17.0, "{before} -> {}", pm.origin.z);
    let mut peak = 0.0f32;
    for _ in 0..80 {
        player_move(&mut pm, &duck, &cfg, &w, DT);
        peak = peak.max(pm.origin.z);
    }
    assert!(peak > 64.0, "crouch-jump peak {peak}");
}

#[test]
fn angle_vectors_match_source() {
    let (f, r, u) = angle_vectors(Vec3::new(0.0, 90.0, 0.0));
    assert!((f - Vec3::Y).length() < 1e-5);
    assert!((r - Vec3::X).length() < 1e-5);
    assert!((u - Vec3::Z).length() < 1e-5);
}

struct VoxelPlatform(CollisionWorld);
impl TraceWorld for VoxelPlatform {
    fn voxel_scale(&self)->f32 {40.}
    fn trace_hull(&self,a:Vec3,b:Vec3,lo:Vec3,hi:Vec3,mask:i32)->Trace {self.0.trace_box(a,b,lo,hi,mask)}
}
#[test]
fn minecraft_crouch_stops_at_straight_and_diagonal_edges_but_release_allows_falling() {
    let mut collision=CollisionWorld::new();
    collision.add_convex(aabb(Vec3::new(0.,0.,-40.),Vec3::new(40.,40.,0.)));collision.finalize();
    let w=VoxelPlatform(collision);
    for yaw in [0.,45.,90.,180.,225.,270.] {
        let mut p=PlayerMove::default();p.origin=Vec3::new(20.,20.,0.04);p.on_ground=true;
        let mut input=forward(250.);input.view_angles.y=yaw;input.buttons=buttons::DUCK;
        for _ in 0..256 {player_move(&mut p,&input,&MoveConfig::default(),&w,DT);}
        assert!(p.origin.z>=-0.1,"fell at yaw {yaw}: {:?}",p.origin);
        assert!(p.origin.x>=-16. && p.origin.x<=56. && p.origin.y>=-16. && p.origin.y<=56.);
        input.buttons=0;
        for _ in 0..128 {player_move(&mut p,&input,&MoveConfig::default(),&w,DT);}
        assert!(p.origin.z< -40.,"release should allow walking off edge");
    }
}
