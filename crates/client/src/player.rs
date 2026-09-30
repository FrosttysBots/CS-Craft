//! Local player: input → [`sim::World`] at 64 Hz → interpolated camera.

use crate::coords::{to_bevy, view_forward};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use movement::buttons;
use sim::{PlayerId, TickInput};

/// CS:GO's `m_yaw` / `m_pitch`: degrees per mouse count.
const M_YAW: f32 = 0.022;

#[derive(Resource)]
pub struct SimWorld(pub sim::World);

#[derive(Resource)]
pub struct LocalPlayer {
    pub id: PlayerId,
    pub pitch: f32,
    pub yaw: f32,
    /// CS:GO `sensitivity`.
    pub sensitivity: f32,
    pub noclip: bool,
    pub noclip_pos: glam::Vec3,
    /// Eye position before and after the latest tick (Source axes).
    pub prev_eye: glam::Vec3,
    pub curr_eye: glam::Vec3,
    /// Buttons captured since the last tick, so short taps are not lost.
    pub latched: u32,
}

#[derive(Component)]
pub struct PlayerCamera;

/// False in automated runs (smoke tests) so stray input cannot move the camera.
#[derive(Resource)]
pub struct InputEnabled(pub bool);

pub fn grab_cursor(
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if mouse.just_pressed(MouseButton::Left) && cursor.grab_mode == CursorGrabMode::None {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

pub fn look(
    mut local: ResMut<LocalPlayer>,
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    keys: Res<ButtonInput<KeyCode>>,
    enabled: Res<InputEnabled>,
) {
    if !enabled.0 {
        return;
    }
    if cursor.grab_mode != CursorGrabMode::None {
        let d = motion.delta;
        local.yaw -= d.x * local.sensitivity * M_YAW;
        local.pitch = (local.pitch + d.y * local.sensitivity * M_YAW).clamp(-89.0, 89.0);
        local.yaw = local.yaw.rem_euclid(360.0);
    }
    if keys.just_pressed(KeyCode::KeyV) {
        local.noclip = !local.noclip;
        local.noclip_pos = local.curr_eye;
    }
    let mut b = 0;
    if keys.pressed(KeyCode::Space) {
        b |= buttons::JUMP;
    }
    if keys.pressed(KeyCode::ControlLeft) {
        b |= buttons::DUCK;
    }
    if keys.pressed(KeyCode::ShiftLeft) {
        b |= buttons::WALK;
    }
    local.latched |= b;
}

fn move_axes(keys: &ButtonInput<KeyCode>) -> (f32, f32) {
    let mut forward = 0.0;
    let mut side = 0.0;
    if keys.pressed(KeyCode::KeyW) {
        forward += 450.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        forward -= 450.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        side += 450.0;
    }
    if keys.pressed(KeyCode::KeyA) {
        side -= 450.0;
    }
    (forward, side)
}

/// Runs in `FixedUpdate` at the sim tick rate.
pub fn tick(
    mut world: ResMut<SimWorld>,
    mut local: ResMut<LocalPlayer>,
    real_keys: Res<ButtonInput<KeyCode>>,
    enabled: Res<InputEnabled>,
) {
    let no_keys = ButtonInput::<KeyCode>::default();
    let keys: &ButtonInput<KeyCode> = if enabled.0 { &real_keys } else { &no_keys };
    let (forward, side) = move_axes(keys);
    let mut held = 0;
    if keys.pressed(KeyCode::Space) {
        held |= buttons::JUMP;
    }
    if keys.pressed(KeyCode::ControlLeft) {
        held |= buttons::DUCK;
    }
    if keys.pressed(KeyCode::ShiftLeft) {
        held |= buttons::WALK;
    }
    let buttons = held | local.latched;
    local.latched = 0;

    if local.noclip {
        let dt = sim::TICK_DT;
        let (f, r, _) = movement::angle_vectors(glam::Vec3::new(local.pitch, local.yaw, 0.0));
        let speed = if keys.pressed(KeyCode::ShiftLeft) { 1600.0 } else { 600.0 };
        let wish = f * (forward / 450.0) + r * (side / 450.0);
        local.prev_eye = local.noclip_pos;
        local.noclip_pos += wish * speed * dt;
        local.curr_eye = local.noclip_pos;
        return;
    }

    let input = TickInput {
        view_angles: glam::Vec3::new(local.pitch, local.yaw, 0.0),
        forward_move: forward,
        side_move: side,
        buttons,
        weapon_max_speed: 250.0,
    };
    let id = local.id;
    world.0.step(&[(id, input)]);
    if let Some(p) = world.0.player(id) {
        local.prev_eye = local.curr_eye;
        local.curr_eye = p.movement.eye_position();
    }
}

pub fn update_camera(
    local: Res<LocalPlayer>,
    time: Res<Time<Fixed>>,
    mut cams: Query<&mut Transform, With<PlayerCamera>>,
) {
    let alpha = time.overstep_fraction();
    let eye = local.prev_eye.lerp(local.curr_eye, alpha);
    let fwd = view_forward(local.pitch, local.yaw);
    for mut cam in &mut cams {
        cam.translation = to_bevy(eye);
        cam.look_to(fwd, Vec3::Y);
    }
}
