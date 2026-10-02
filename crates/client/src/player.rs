//! Local player: input → [`sim::World`] at 64 Hz → interpolated camera.
//! While dead, the camera follows a living teammate (CS:GO's first-person
//! spectating); left click switches to the next one.

use crate::coords::{to_bevy, view_forward};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use movement::buttons;
use sim::{PlayerId, Purchase, TickInput};

/// CS:GO's `m_yaw` / `m_pitch`: degrees per mouse count.
const M_YAW: f32 = 0.022;

#[derive(Resource)]
pub struct SimWorld(pub sim::World);

#[derive(Resource, Default)]
pub struct BotBrains(pub sim::bots::Bots);

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
    /// The pose shared by all cameras and the weapon this render frame.
    pub render_eye: glam::Vec3,
    pub render_angles: glam::Vec3,
    /// Buttons pressed since the last tick, so short taps are not lost.
    pub latched: u32,
    /// Inventory slot requested since the last tick (0 = none).
    pub pending_slot: u8,
    /// Purchases waiting to be sent, one per tick.
    pub pending_buys: Vec<Purchase>,
    pub pending_drop: bool,
    /// The teammate being watched while dead.
    pub spectating: Option<PlayerId>,
    was_alive: bool,
}

impl LocalPlayer {
    pub fn new(id: PlayerId, eye: glam::Vec3, yaw: f32) -> Self {
        Self {
            id,
            pitch: 0.0,
            yaw,
            sensitivity: 2.0,
            noclip: false,
            noclip_pos: eye,
            prev_eye: eye,
            curr_eye: eye,
            render_eye: eye,
            render_angles: glam::Vec3::new(0.0, yaw, 0.0),
            latched: 0,
            pending_slot: 0,
            pending_buys: Vec::new(),
            pending_drop: false,
            spectating: None,
            was_alive: true,
        }
    }

    /// Whose eyes the camera is in (the local player unless spectating).
    pub fn view_target(&self) -> PlayerId {
        self.spectating.unwrap_or(self.id)
    }
}

#[derive(Component)]
pub struct PlayerCamera;

#[derive(Component)]
pub struct ViewModelCamera;

/// False in automated runs (smoke tests) so stray input cannot move the camera.
#[derive(Resource)]
pub struct InputEnabled(pub bool);
#[derive(Resource,Default)]
pub struct InputBlockFrames(pub u8);

fn captured(cursor: &CursorOptions) -> bool {
    cursor.grab_mode != CursorGrabMode::None
}

pub fn grab_cursor(
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    enabled: Res<InputEnabled>,
    menu: Res<crate::buymenu::BuyMenu>,
    front: Res<crate::menu::Menu>,
    building: Res<crate::overworld_client::Building>,

) {
    if !enabled.0 {
        return;
    }
    if menu.open || front.open() || building.inventory_open {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
        return;
    }
    if (mouse.just_pressed(MouseButton::Left) || menu.just_closed) && cursor.grab_mode == CursorGrabMode::None {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
}

/// Living players the local player may watch: teammates, or anyone once
/// the whole team is dead.
fn spectate_candidates(world: &sim::World, me: &sim::Player) -> Vec<PlayerId> {
    let team: Vec<PlayerId> = world.players.iter().filter(|p| p.alive && p.team == me.team && p.id != me.id).map(|p| p.id).collect();
    if !team.is_empty() {
        return team;
    }
    world.players.iter().filter(|p| p.alive && p.id != me.id).map(|p| p.id).collect()
}

#[allow(clippy::too_many_arguments)]
pub fn look(
    mut local: ResMut<LocalPlayer>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    enabled: Res<InputEnabled>,
    world: Res<SimWorld>,
    menu: Res<crate::buymenu::BuyMenu>,
    front: Res<crate::menu::Menu>,
    settings: Res<crate::settings::Settings>,
    mut block: ResMut<InputBlockFrames>,
    mut inspect: ResMut<crate::tracers::InspectRequest>,
    building:Res<crate::overworld_client::Building>,
) {
    if front.open() || building.inventory_open {return;}
    if block.0 > 0 { if !mouse.pressed(MouseButton::Left) && !mouse.pressed(MouseButton::Right) {block.0=0;} return; }
    if !enabled.0 {
        return;
    }
    let grabbed = captured(&cursor) && !menu.open;
    if grabbed {
        let d = motion.delta;
        let zoom = if local.noclip { 1.0 } else {
            world.0.player(local.id).filter(|p| p.alive).and_then(|p| p.active_weapon()).map_or(1.0, |w| w.rendered_fov(world.0.time()) / 90.0)
        };
        local.yaw -= d.x * local.sensitivity * M_YAW * zoom;
        local.pitch = (local.pitch + d.y * (if settings.invert_y {-1.0} else {1.0}) * local.sensitivity * M_YAW * zoom).clamp(-89.0, 89.0);
        local.yaw = local.yaw.rem_euclid(360.0);
    }
    if !menu.open && !building.initialized && keys.just_pressed(KeyCode::KeyV) {
        local.noclip = !local.noclip;
        local.noclip_pos = local.curr_eye;
    }
    let me = world.0.player(local.id);
    let alive = me.is_some_and(|p| p.alive);
    if alive && grabbed && keys.just_pressed(settings.binds[12]) {inspect.0=inspect.0.wrapping_add(1);}

    // Spectating: left click cycles through living teammates.
    if !alive && grabbed && mouse.just_pressed(MouseButton::Left) {
        if let Some(me) = me {
            let list = spectate_candidates(&world.0, me);
            if !list.is_empty() {
                let at = local.spectating.and_then(|s| list.iter().position(|&id| id == s)).map_or(0, |i| (i + 1) % list.len());
                local.spectating = Some(list[at]);
            }
        }
    }

    if menu.open {
        return;
    }

    let mut b = 0;
    if keys.pressed(settings.binds[4]) {
        b |= buttons::JUMP;
    }
    if keys.pressed(settings.binds[5]) {
        b |= buttons::DUCK;
    }
    if keys.pressed(settings.binds[6]) {
        b |= buttons::WALK;
    }
    if keys.just_pressed(settings.binds[8]) {
        b |= buttons::RELOAD;
    }
    // `look` runs before `grab_cursor`, so the click that captures the
    // mouse is never seen here as a shot.
    if grabbed && alive && mouse.just_pressed(MouseButton::Left) {
        b |= buttons::ATTACK;
    }
    if grabbed && alive && mouse.just_pressed(MouseButton::Right) {
        b |= buttons::ATTACK2;
    }
    if building.active { b &= !(buttons::ATTACK|buttons::ATTACK2); }
    if building.using_block {b &= !buttons::ATTACK2;}
    local.latched |= b;
    if building.active {return;}

    if !menu.open {
        for (key, slot) in [
            (KeyCode::Digit1, 1u8),
            (KeyCode::Digit2, 2),
            (KeyCode::Digit3, 3),
            (KeyCode::Digit4, 4),
            (KeyCode::Digit5, 5),
            (KeyCode::KeyQ, sim::SLOT_LAST),
        ] {
            if keys.just_pressed(key) {
                local.pending_slot = slot;
            }
        }
    }
    if keys.just_pressed(settings.binds[10]) {
        local.pending_drop = true;
    }
    if grabbed && scroll.delta.y != 0.0 {
        if let Some(p) = me.filter(|p| !p.weapons.is_empty()) {
            // Step through the inventory, by the slot of the next weapon.
            let count = p.weapons.len() as i32;
            let step = if scroll.delta.y > 0.0 { -1 } else { 1 };
            let next = (p.active as i32 + step).rem_euclid(count) as usize;
            local.pending_slot = sim::weapon::slot(&p.weapons[next].def);
        }
    }
}

fn move_axes(keys: &ButtonInput<KeyCode>, settings: &crate::settings::Settings) -> (f32, f32) {
    let mut forward = 0.0;
    let mut side = 0.0;
    if keys.pressed(settings.binds[0]) {
        forward += 450.0;
    }
    if keys.pressed(settings.binds[1]) {
        forward -= 450.0;
    }
    if keys.pressed(settings.binds[3]) {
        side += 450.0;
    }
    if keys.pressed(settings.binds[2]) {
        side -= 450.0;
    }
    (forward, side)
}

/// Runs in `FixedUpdate` at the sim tick rate.
#[allow(clippy::too_many_arguments)]
pub fn tick(
    mut world: ResMut<SimWorld>,
    mut bots: ResMut<BotBrains>,
    mut pending: ResMut<crate::effects::PendingEvents>,
    mut local: ResMut<LocalPlayer>,
    real_keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    enabled: Res<InputEnabled>,
    menu: Res<crate::buymenu::BuyMenu>,
    front: Res<crate::menu::Menu>,
    settings: Res<crate::settings::Settings>,
    block: Res<InputBlockFrames>,
    mut poses: ResMut<crate::interpolation::RenderPoses>,
    inventory: Res<crate::inventory::Inventory>,
    building:Res<crate::overworld_client::Building>,
) {
    if front.open() {return;}
    let no_keys = ButtonInput::<KeyCode>::default();
    let keys: &ButtonInput<KeyCode> = if enabled.0 && !menu.open && !building.inventory_open && block.0==0 { &real_keys } else { &no_keys };
    let (forward, side) = move_axes(keys, &settings);
    let mut held = 0;
    if keys.pressed(settings.binds[4]) {
        held |= buttons::JUMP;
    }
    if keys.pressed(settings.binds[5]) {
        held |= buttons::DUCK;
    }
    if keys.pressed(settings.binds[6]) {
        held |= buttons::WALK;
    }
    if keys.pressed(settings.binds[7]) {
        held |= buttons::USE;
    }
    if enabled.0 && block.0==0 && captured(&cursor) && !menu.open && mouse.pressed(MouseButton::Left) {
        held |= buttons::ATTACK;
    }
    if enabled.0 && block.0==0 && captured(&cursor) && !menu.open && mouse.pressed(MouseButton::Right) {
        held |= buttons::ATTACK2;
    }
    // Smoke tests can hold the trigger to exercise the fire path.
    if !enabled.0 && std::env::var_os("CSR_AUTOFIRE").is_some() {
        held |= buttons::ATTACK;
    }
    if !enabled.0 && std::env::var_os("CSR_ATTACK2").is_some() { held |= buttons::ATTACK2; }
    let mut pressed = held | local.latched;
    if building.active || building.inventory_open {pressed &= !(buttons::ATTACK|buttons::ATTACK2);}
    if building.using_block {pressed &= !buttons::ATTACK2;}
    local.latched = 0;
    let mut slot = std::mem::take(&mut local.pending_slot);
    if !enabled.0 && std::env::var_os("CSR_TEST_SWITCH").is_some() {
        match world.0.tick {
            64 => pressed |= buttons::ATTACK,
            80 => slot = 3,
            144 => slot = 1,
            _ => {}
        }
    }
    let buy = if local.pending_buys.is_empty() { Purchase::None } else { local.pending_buys.remove(0) };
    let drop = std::mem::take(&mut local.pending_drop);

    let mut inputs = bots.0.think(&world.0);

    if local.noclip {
        let dt = sim::TICK_DT;
        let (f, r, _) = movement::angle_vectors(glam::Vec3::new(local.pitch, local.yaw, 0.0));
        let speed = if keys.pressed(settings.binds[6]) { 1600.0 } else { 600.0 };
        let wish = f * (forward / 450.0) + r * (side / 450.0);
        local.prev_eye = local.noclip_pos;
        local.noclip_pos += wish * speed * dt;
        local.curr_eye = local.noclip_pos;
    } else {
        inputs.push((
            local.id,
            TickInput {
                view_angles: glam::Vec3::new(local.pitch, local.yaw, 0.0),
                forward_move: forward,
                side_move: side,
                buttons: pressed,
                weapon_slot: slot,
                buy,
                drop,
                utility: 0,
            },
        ));
    }
    let id = local.id;
    let alive_before = world.0.player(id).is_some_and(|p| p.alive);
    world.0.step(&inputs);
    if !alive_before && world.0.player(id).is_some_and(|p| p.alive) {
        crate::inventory::apply(&mut world.0,id,&inventory);
    }
    poses.capture(&world.0);
    pending.0.extend(world.0.events.iter().cloned());
    if local.noclip {
        return;
    }
    // First-person spectator smoke test, isolated from interactive controls.
    if !enabled.0 {
        if let Ok(index) = std::env::var("CSR_SPECTATE_BOT").and_then(|s| s.parse::<usize>().map_err(|_| std::env::VarError::NotPresent)) {
            if let Some(target) = world.0.players.iter().filter(|p| p.is_bot && p.alive).nth(index) {
                local.spectating = Some(target.id);
                local.prev_eye = local.curr_eye;
                local.curr_eye = target.eye();
                return;
            }
        }
    }
    let Some(me) = world.0.player(id) else { return };
    if me.alive {
        // Fresh spawn: face the way the spawn point faces.
        if !local.was_alive || world.0.events.iter().any(|e| matches!(e, sim::GameEvent::RoundStart { .. })) {
            local.yaw = me.view_angles.y;
            local.pitch = 0.0;
            local.prev_eye = me.eye();
        } else {
            local.prev_eye = local.curr_eye;
        }
        local.curr_eye = me.eye();
        local.spectating = None;
        local.was_alive = true;
        return;
    }
    local.was_alive = false;
    // Dead: keep watching someone alive.
    let valid = local.spectating.and_then(|s| world.0.player(s)).is_some_and(|p| p.alive);
    if !valid {
        let list = spectate_candidates(&world.0, me);
        local.spectating = list.first().copied();
        if let Some(t) = local.spectating.and_then(|s| world.0.player(s)) {
            local.curr_eye = t.eye();
        }
    }
    if let Some(t) = local.spectating.and_then(|s| world.0.player(s)) {
        local.prev_eye = local.curr_eye;
        local.curr_eye = t.eye();
    }
}

pub fn update_camera(
    mut local: ResMut<LocalPlayer>,
    world: Res<SimWorld>,
    time: Res<Time<Fixed>>,
    poses: Res<crate::interpolation::RenderPoses>,
    mut cams: Query<(&mut Transform, &mut Projection, Option<&ViewModelCamera>), With<PlayerCamera>>,
) {
    let alpha = time.overstep_fraction();
    let target = world.0.player(local.view_target()).map(|p| poses.sample(p, alpha));
    let eye = if local.noclip { local.prev_eye.lerp(local.curr_eye, alpha) } else { target.map_or(local.curr_eye, |p| p.eye) };
    let mut angles = if local.spectating.is_some() && !local.noclip {
        target.map_or(glam::Vec3::ZERO, |p| p.angles)
    } else { glam::Vec3::new(local.pitch, local.yaw, 0.0) };
    if !local.noclip {
        if let Some(p) = target { angles.x += p.punch.x; angles.y += p.punch.y; }
    }
    local.render_eye = eye;
    local.render_angles = angles;
    let fwd = view_forward(angles.x, angles.y);
    let fov = if local.noclip { 90.0 } else {
        world.0.player(local.view_target()).filter(|p| p.alive).and_then(|p| p.active_weapon())
            .map_or(90.0, |w| w.rendered_fov(world.0.time() + alpha * sim::TICK_DT))
    };
    for (mut cam, mut projection, viewmodel) in &mut cams {
        if viewmodel.is_none() {
            if let Projection::Perspective(p) = &mut *projection { p.fov = crate::viewmodel::vertical_fov(fov); }
        }
        cam.translation = to_bevy(eye);
        cam.look_to(fwd, Vec3::Y);
    }
}
