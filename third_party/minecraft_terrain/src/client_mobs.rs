//! The client's copies of the server's mobs, as vanilla 26.3 keeps them.
//!
//! The server does not show the client its mobs directly: each tick its
//! `ServerEntity` trackers decide what to send (every `updateInterval`
//! ticks a move packet with rotations packed to 1/256 of a turn and the
//! position as a 1/4096-block delta path, a full sync when ground contact
//! changes, a head packet when the packed head yaw changes), and the client
//! entity replays that: `SteppedInterpolationHandler` walks the position
//! path, the head eases to its packet over three ticks, the body turns by
//! the client's own `BodyRotationControl.clientTick`, and the walk swing
//! follows the client's own movement. Rendering then interpolates each
//! value between the client's previous and current tick.
//!
//! Sources: `ServerEntity.sendChanges`/`createMovePacket`,
//! `SteppedInterpolationTracker`, `VecDeltaCodec`, `VecDelta`,
//! `ClientPacketListener.handleAddEntity`/`handleMoveEntity`/
//! `handleEntityPositionSync`/`handleRotateMob`,
//! `AbstractInterpolationHandler`, `SteppedInterpolationHandler`,
//! `Entity.commonTick`/`moveOrInterpolateTo`/`setRot`,
//! `LivingEntity.baseTick`/`tick`/`aiStep`/`recreateFromPacket`/
//! `calculateEntityAnimation`, `Mob.tickHeadTurn`,
//! `BodyRotationControl.clientTick`, `LivingEntityRenderer.extractRenderState`
//! and `EntityRenderer.getPackedLightCoords`.

use crate::walk_animation::WalkAnimation;
use glam::DVec3;
use std::collections::{HashMap, VecDeque};

/// One mob as the server has it at the end of a tick.
#[derive(Clone, Copy, Debug)]
pub struct ServerMob {
    pub id: u64,
    pub position: DVec3,
    /// `getYRot`, `getXRot` and `getYHeadRot`.
    pub y_rot: f32,
    pub x_rot: f32,
    pub y_head_rot: f32,
    pub on_ground: bool,
    pub baby: bool,
    /// `getEyeHeight` in its current pose.
    pub eye_height: f32,
    /// `Mob.getMaxHeadYRot`.
    pub max_head_y_rot: f32,
    /// `EntityType.updateInterval`.
    pub update_interval: i32,
    /// What makes the tracker send off its interval this tick.
    pub sync: SyncFlags,
    /// A bat's resting flag (`BatFlags`), synced as entity data.
    pub resting: bool,
    /// Full hits taken so far (each a damage event) and whether it is dead
    /// (its synced health at zero).
    pub hurts: u32,
    pub dead: bool,
    /// Attacks swung so far (each a `ClientboundSwingAnimationPacket`).
    pub swings: u32,
    /// Its bounding box's width and height.
    pub size: (f32, f32),
}

/// The entity flags `ServerEntity.sendChanges` reads besides its interval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncFlags {
    /// `Entity.needsSync`: set by jumps and pushes, sends at once.
    pub needs_sync: bool,
    /// `SynchedEntityData.isDirty`: changed data also sends at once.
    pub data_dirty: bool,
    /// `Entity.syncPosition` (bounces): the tracker records a step.
    pub sync_position: bool,
}

/// `Mth.wrapDegrees(float)`.
pub fn wrap_degrees(angle: f32) -> f32 {
    let mut a = angle % 360.0;
    if a >= 180.0 {
        a -= 360.0;
    }
    if a < -180.0 {
        a += 360.0;
    }
    a
}

/// `Mth.rotLerp(float, float, float)`.
pub fn rot_lerp(a: f32, from: f32, to: f32) -> f32 {
    from + a * wrap_degrees(to - from)
}

fn lerp(a: f32, from: f32, to: f32) -> f32 {
    from + a * (to - from)
}

/// `Mth.rotateIfNecessary`.
fn rotate_if_necessary(base: f32, target: f32, max: f32) -> f32 {
    target - wrap_degrees(target - base).clamp(-max, max)
}

/// `Mth.packDegrees`.
fn pack_degrees(angle: f32) -> i8 {
    (angle * 256.0 / 360.0).floor() as i32 as i8
}

/// `Mth.unpackDegrees`.
fn unpack_degrees(rot: i8) -> f32 {
    (i32::from(rot) * 360) as f32 / 256.0
}

/// `VecDeltaCodec.encode`.
/// `VecDeltaCodec.encode`: `Math.round`, whose ties go up.
fn encode(v: f64) -> i64 {
    let scaled = v * 4096.0;
    let floor = scaled.floor();
    (if scaled - floor >= 0.5 { floor + 1.0 } else { floor }) as i64
}

fn decode(v: i64) -> f64 {
    v as f64 / 4096.0
}

/// `Vec3.lerp` (`Mth.lerp` per component: `a + t * (b - a)`).
fn vec_lerp(a: DVec3, b: DVec3, t: f64) -> DVec3 {
    DVec3::new(a.x + t * (b.x - a.x), a.y + t * (b.y - a.y), a.z + t * (b.z - a.z))
}

/// Vanilla's `Vec3.equals` (`Double.compare` per component).
fn same(a: DVec3, b: DVec3) -> bool {
    a.x.total_cmp(&b.x).is_eq() && a.y.total_cmp(&b.y).is_eq() && a.z.total_cmp(&b.z).is_eq()
}

/// `VecDeltaCodec`: positions relative to the last one sent.
#[derive(Clone, Copy, Debug, Default)]
struct DeltaCodec {
    base: DVec3,
}

impl DeltaCodec {
    fn delta(&self, pos: DVec3) -> [i64; 3] {
        [encode(pos.x) - encode(self.base.x), encode(pos.y) - encode(self.base.y), encode(pos.z) - encode(self.base.z)]
    }

    fn decode(&self, d: [i64; 3]) -> DVec3 {
        if d == [0, 0, 0] {
            return self.base;
        }
        let axis = |delta: i64, base: f64| if delta == 0 { base } else { decode(encode(base) + delta) };
        DVec3::new(axis(d[0], self.base.x), axis(d[1], self.base.y), axis(d[2], self.base.z))
    }
}

fn too_big(d: [i64; 3]) -> bool {
    d.iter().any(|&v| !(-32768..=32767).contains(&v))
}

/// A position path: its steps and the ticks each takes (`PositionPath`).
#[derive(Clone, Debug)]
enum Path {
    Linear(DVec3),
    Stepped(Vec<(DVec3, i32)>),
}

impl Path {
    fn end(&self) -> DVec3 {
        match self {
            Path::Linear(p) => *p,
            Path::Stepped(steps) => steps.last().map(|s| s.0).expect("a stepped path has steps"),
        }
    }
}

/// What the client receives about a mob in one tick.
#[derive(Clone, Debug)]
enum Packet {
    /// `ClientboundEntityPositionSyncPacket`: full precision.
    Sync { path: Path, y_rot: f32, x_rot: f32, on_ground: bool },
    /// `ClientboundMoveEntityPacket.Pos`/`PosRot`/`Rot`, as decoded.
    Move { path: Option<Path>, rotation: Option<(f32, f32)>, on_ground: bool },
    /// `ClientboundRotateHeadPacket`.
    Head(f32),
}

/// A mob's `ServerEntity` (with its `SteppedInterpolationTracker`).
#[derive(Clone, Debug)]
struct Tracker {
    tick_count: i32,
    last_sent_y_rot: i8,
    last_sent_x_rot: i8,
    last_sent_y_head_rot: i8,
    codec: DeltaCodec,
    /// The client's codec: it decodes against what it decoded last.
    client_codec: DeltaCodec,
    was_on_ground: bool,
    teleport_delay: i32,
    /// `SteppedInterpolationTracker.trackedSteps`/`ticksSinceLastStep`.
    tracked_steps: Vec<(DVec3, i32)>,
    ticks_since_last_step: i32,
}

impl Tracker {
    fn new(mob: &ServerMob) -> Self {
        Self {
            tick_count: 0,
            last_sent_y_rot: pack_degrees(mob.y_rot),
            last_sent_x_rot: pack_degrees(mob.x_rot),
            last_sent_y_head_rot: pack_degrees(mob.y_head_rot),
            codec: DeltaCodec { base: mob.position },
            client_codec: DeltaCodec { base: mob.position },
            was_on_ground: mob.on_ground,
            teleport_delay: 0,
            tracked_steps: Vec::new(),
            ticks_since_last_step: -1,
        }
    }

    fn add_step(&mut self, pos: DVec3) {
        if self.ticks_since_last_step > 0 {
            self.tracked_steps.push((pos, self.ticks_since_last_step));
            self.ticks_since_last_step = 0;
        }
    }

    /// `ServerEntity.sendChanges` for a mob that rides nothing, plus what
    /// the client decodes from each packet.
    fn send_changes(&mut self, mob: &ServerMob, out: &mut Vec<Packet>) {
        let current = mob.position;
        // SteppedInterpolationTracker.updateTracking; it clears
        // syncPosition before ServerEntity would look at it.
        self.ticks_since_last_step += 1;
        if mob.sync.sync_position {
            self.add_step(current);
        }
        if mob.sync.needs_sync || self.tick_count % mob.update_interval == 0 || mob.sync.data_dirty {
            let y_rotn = pack_degrees(mob.y_rot);
            let x_rotn = pack_degrees(mob.x_rot);
            let send_rotation = (i32::from(y_rotn) - i32::from(self.last_sent_y_rot)).abs() >= 1 || (i32::from(x_rotn) - i32::from(self.last_sent_x_rot)).abs() >= 1;
            self.teleport_delay += 1;
            // getPositionPath, then clear.
            self.add_step(current);
            let path = if self.tracked_steps.is_empty() { Path::Linear(current) } else { Path::Stepped(std::mem::take(&mut self.tracked_steps)) };
            self.tracked_steps.clear();
            self.ticks_since_last_step = 0;
            let moved = (current - self.codec.base).length_squared() >= f64::from(7.629_394_5e-6_f32);
            let send_position = moved || self.tick_count % 60 == 0;
            let packet = if self.teleport_delay > 400 || self.was_on_ground != mob.on_ground {
                self.was_on_ground = mob.on_ground;
                self.teleport_delay = 0;
                Some((Packet::Sync { path: path.clone(), y_rot: mob.y_rot, x_rot: mob.x_rot, on_ground: mob.on_ground }, true, true))
            } else if send_position {
                match self.encode_path(&path) {
                    None => Some((Packet::Sync { path: path.clone(), y_rot: mob.y_rot, x_rot: mob.x_rot, on_ground: mob.on_ground }, true, true)),
                    Some(decoded) => {
                        let rotation = send_rotation.then(|| (unpack_degrees(y_rotn), unpack_degrees(x_rotn)));
                        Some((Packet::Move { path: Some(decoded), rotation, on_ground: mob.on_ground }, true, send_rotation))
                    }
                }
            } else if send_rotation {
                Some((Packet::Move { path: None, rotation: Some((unpack_degrees(y_rotn), unpack_degrees(x_rotn))), on_ground: mob.on_ground }, false, true))
            } else {
                None
            };
            if let Some((packet, has_position, has_rotation)) = packet {
                if has_position {
                    self.codec.base = current;
                    self.client_codec.base = match &packet {
                        Packet::Sync { path, .. } | Packet::Move { path: Some(path), .. } => path.end(),
                        Packet::Move { path: None, .. } | Packet::Head(_) => unreachable!("a position packet"),
                    };
                }
                out.push(packet);
                if has_rotation {
                    self.last_sent_y_rot = y_rotn;
                    self.last_sent_x_rot = x_rotn;
                }
            }
            let head = pack_degrees(mob.y_head_rot);
            if (i32::from(head) - i32::from(self.last_sent_y_head_rot)).abs() >= 1 {
                out.push(Packet::Head(unpack_degrees(head)));
                self.last_sent_y_head_rot = head;
            }
        }
        self.tick_count += 1;
    }

    /// `VecDeltaCodec.tryEncode` then the client's `decode`: the path as the
    /// client sees it, or `None` when a delta is too big to send.
    fn encode_path(&self, path: &Path) -> Option<Path> {
        match path {
            Path::Linear(pos) => {
                let d = self.codec.delta(*pos);
                (!too_big(d)).then(|| Path::Linear(self.client_codec.decode(d)))
            }
            Path::Stepped(steps) => {
                let mut codec = self.codec;
                let mut deltas = Vec::with_capacity(steps.len());
                for &(pos, ticks) in steps {
                    let d = codec.delta(pos);
                    if too_big(d) {
                        return None;
                    }
                    deltas.push((d, ticks));
                    codec.base = pos;
                }
                // The client decodes each step against the one it decoded
                // before.
                let mut client = self.client_codec;
                let mut out = Vec::with_capacity(deltas.len());
                for (d, ticks) in deltas {
                    let pos = client.decode(d);
                    out.push((pos, ticks));
                    client.base = pos;
                }
                Some(Path::Stepped(out))
            }
        }
    }
}

/// `SteppedInterpolationHandler`'s interpolation data.
#[derive(Clone, Debug)]
struct Stepped {
    interpolation_steps: i32,
    /// The target (`interpolationData` itself).
    position: DVec3,
    y_rot: f32,
    x_rot: f32,
    remaining: VecDeque<(DVec3, f32, f32, i32)>,
    last_step: (DVec3, f32, f32),
    current_step_ticks: f32,
    remaining_ticks: f32,
    speed: f32,
}

impl Stepped {
    fn new(steps: i32) -> Self {
        Self {
            interpolation_steps: steps,
            position: DVec3::ZERO,
            y_rot: 0.0,
            x_rot: 0.0,
            remaining: VecDeque::new(),
            last_step: (DVec3::ZERO, 0.0, 0.0),
            current_step_ticks: 0.0,
            remaining_ticks: 0.0,
            speed: 1.0,
        }
    }

    fn active(&self) -> bool {
        !self.remaining.is_empty()
    }

    fn reset(&mut self) {
        self.remaining.clear();
        self.remaining_ticks = 0.0;
        self.speed = 1.0;
    }

    fn add_step(&mut self, pos: DVec3, y_rot: f32, x_rot: f32, ticks: i32) {
        self.remaining.push_back((pos, y_rot, x_rot, ticks));
        self.remaining_ticks += ticks as f32;
    }
}

/// A mob as the client has it.
#[derive(Clone, Debug)]
pub struct ClientMob {
    tracker: Tracker,
    codec: DeltaCodec,
    interpolation: Stepped,
    /// `lastPositionAndRotation` of the interpolation handler.
    last: (DVec3, f32, f32),
    pub position: DVec3,
    pub old_position: DVec3,
    pub y_rot: f32,
    pub y_rot_o: f32,
    pub x_rot: f32,
    pub x_rot_o: f32,
    pub y_head_rot: f32,
    pub y_head_rot_o: f32,
    pub y_body_rot: f32,
    pub y_body_rot_o: f32,
    lerp_y_head_rot: f64,
    lerp_head_steps: i32,
    /// `BodyRotationControl` state.
    head_stable_time: i32,
    last_stable_y_head_rot: f32,
    pub walk: WalkAnimation,
    /// `Chicken`'s wing flap (`flap`, `flapSpeed`, their old values and
    /// `flapping`), kept for every mob; chickens show it.
    flap: f32,
    flap_o: f32,
    flap_speed: f32,
    flap_speed_o: f32,
    flapping: f32,
    /// Its box's width and height, for the poof it leaves.
    size: (f32, f32),
    /// A bat's resting flag and its `restAnimationState` and
    /// `flyAnimationState` start ticks.
    resting: bool,
    rest_start: Option<i32>,
    fly_start: Option<i32>,
    /// The damage events seen, `hurtTime` and `deathTime`, and whether its
    /// health is at zero.
    hurts_seen: u32,
    /// `LivingEntity.SwingState` and the swings seen.
    swings_seen: u32,
    swing: SwingState,
    hurt_time: i32,
    death_time: i32,
    dead: bool,
    pub tick_count: i32,
    pub on_ground: bool,
    baby: bool,
    eye_height: f32,
    max_head_y_rot: f32,
    seen: bool,
}

impl ClientMob {
    /// `ClientboundAddEntityPacket` from a new tracker, then
    /// `LivingEntity.recreateFromPacket`.
    fn spawn(mob: &ServerMob) -> Self {
        let tracker = Tracker::new(mob);
        // The packet carries the tracker's packed rotations as floats and
        // packs them again.
        let y_rot = unpack_degrees(tracker.last_sent_y_rot);
        let x_rot = unpack_degrees(tracker.last_sent_x_rot).clamp(-90.0, 90.0);
        let head = unpack_degrees(tracker.last_sent_y_head_rot);
        let position = tracker.codec.base;
        Self {
            codec: DeltaCodec { base: position },
            tracker,
            interpolation: Stepped::new(mob.update_interval),
            last: (position, y_rot, x_rot),
            position,
            old_position: position,
            y_rot,
            y_rot_o: y_rot,
            x_rot,
            x_rot_o: x_rot,
            y_head_rot: head,
            y_head_rot_o: head,
            y_body_rot: head,
            y_body_rot_o: head,
            lerp_y_head_rot: 0.0,
            lerp_head_steps: 0,
            head_stable_time: 0,
            last_stable_y_head_rot: 0.0,
            walk: WalkAnimation::default(),
            flap: 0.0,
            flap_o: 0.0,
            flap_speed: 0.0,
            flap_speed_o: 0.0,
            flapping: 1.0,
            size: mob.size,
            resting: mob.resting,
            rest_start: None,
            fly_start: None,
            hurts_seen: mob.hurts,
            swings_seen: mob.swings,
            swing: SwingState::default(),
            hurt_time: 0,
            death_time: 0,
            dead: mob.dead,
            tick_count: 0,
            on_ground: mob.on_ground,
            baby: mob.baby,
            eye_height: mob.eye_height,
            max_head_y_rot: mob.max_head_y_rot,
            seen: true,
        }
    }

    /// `LivingEntity.handleDamageEvent`: the limbs jerk and it flashes red.
    fn damage_event(&mut self) {
        self.walk.set_speed(1.5);
        self.hurt_time = 10;
    }

    /// `Entity.setRot`.
    fn set_rot(&mut self, y_rot: f32, x_rot: f32) {
        if y_rot.is_finite() {
            self.y_rot = y_rot % 360.0;
        }
        if x_rot.is_finite() {
            self.x_rot = (x_rot % 360.0 % 360.0).clamp(-90.0, 90.0);
        }
    }

    /// `Entity.snapTo`: position and rotation, and the old values with them.
    fn snap_to(&mut self, pos: DVec3, y_rot: f32, x_rot: f32) {
        self.position = pos;
        self.old_position = pos;
        self.y_rot = y_rot;
        self.x_rot = x_rot.clamp(-90.0, 90.0) % 360.0;
        self.y_rot_o = self.y_rot;
        self.x_rot_o = self.x_rot;
    }

    /// `Entity.moveOrInterpolateTo` through `AbstractInterpolationHandler`.
    fn move_or_interpolate_to(&mut self, path: Option<Path>, y_rot: f32, x_rot: f32, has_rotation: bool) {
        let (cur_pos, cur_y, cur_x) = if self.interpolation.active() {
            (self.interpolation.position, self.interpolation.y_rot, self.interpolation.x_rot)
        } else {
            (self.position, self.y_rot, self.x_rot)
        };
        let path = path.unwrap_or(Path::Linear(cur_pos));
        let (y_rot, x_rot) = if has_rotation { (y_rot, x_rot) } else { (cur_y, cur_x) };
        if self.interpolation.interpolation_steps == 0 {
            self.snap_to(path.end(), y_rot, x_rot);
            self.interpolation.reset();
            return;
        }
        let end = path.end();
        let unchanged = self.interpolation.active() && self.interpolation.y_rot == y_rot && self.interpolation.x_rot == x_rot && same(self.interpolation.position, end);
        if !unchanged {
            self.start_interpolating(&path, y_rot, x_rot);
            self.last = (self.position, self.y_rot, self.x_rot);
        }
    }

    /// `SteppedInterpolationHandler.startInterpolating`.
    fn start_interpolating(&mut self, path: &Path, y_rot: f32, x_rot: f32) {
        let steps = self.interpolation.interpolation_steps;
        let data = &mut self.interpolation;
        if data.remaining.is_empty() {
            data.last_step = (self.position, self.y_rot, self.x_rot);
            data.current_step_ticks = 1.0;
        }
        let end = path.end();
        if same(end, data.position) {
            data.add_step(end, y_rot, x_rot, steps);
        } else {
            match path {
                Path::Linear(pos) => data.add_step(*pos, y_rot, x_rot, steps),
                Path::Stepped(list) => {
                    if y_rot == data.y_rot && x_rot == data.x_rot {
                        for &(pos, ticks) in list {
                            data.add_step(pos, y_rot, x_rot, ticks);
                        }
                    } else {
                        let total: i32 = list.iter().map(|s| s.1).sum();
                        let (from_y, from_x) = (data.y_rot, data.x_rot);
                        let mut offset = 0;
                        for &(pos, ticks) in list {
                            offset += ticks;
                            let a = offset as f32 / total as f32;
                            data.add_step(pos, rot_lerp(a, from_y, y_rot), lerp(a, from_x, x_rot), ticks);
                        }
                    }
                }
            }
        }
        data.position = end;
        data.y_rot = y_rot;
        data.x_rot = x_rot;
    }

    /// `AbstractInterpolationHandler.interpolate`.
    fn interpolate(&mut self) {
        if !self.interpolation.active() {
            self.interpolation.reset();
            return;
        }
        // adjustInterpolationTargetFromDeltas: nothing moves a server mob
        // on the client between interpolations.
        let dy = self.y_rot - self.last.1;
        let dx = self.x_rot - self.last.2;
        let data = &mut self.interpolation;
        data.y_rot += dy;
        data.x_rot += dx;
        for step in data.remaining.iter_mut() {
            step.1 += dy;
            step.2 += dx;
        }
        data.last_step.1 += dy;
        data.last_step.2 += dx;
        // doInterpolate: getNewPositionAndRotation, then advance.
        let target = loop {
            let Some(&(pos, y, x, ticks)) = data.remaining.front() else {
                break (data.position, data.y_rot, data.x_rot);
            };
            if data.current_step_ticks < ticks as f32 {
                let a = data.current_step_ticks / ticks as f32;
                let (lp, ly, lx) = data.last_step;
                break (vec_lerp(lp, pos, f64::from(a)), rot_lerp(a, ly, y), lerp(a, lx, x));
            }
            data.current_step_ticks -= ticks as f32;
            data.last_step = (pos, y, x);
            data.remaining.pop_front();
        };
        let steps = data.interpolation_steps as f32;
        let target_speed = (data.remaining_ticks / steps).max(1.0);
        data.speed = lerp(1.0 / steps, data.speed, target_speed);
        let mut ticks = 1.0;
        if ticks * data.speed < data.remaining_ticks {
            ticks *= data.speed;
        } else {
            ticks = data.remaining_ticks;
            data.speed = 1.0;
        }
        data.current_step_ticks += ticks;
        data.remaining_ticks -= ticks;
        self.position = target.0;
        self.set_rot(target.1, target.2);
        self.last = (self.position, self.y_rot, self.x_rot);
    }

    /// The client packet handlers.
    fn receive(&mut self, packet: Packet) {
        match packet {
            Packet::Sync { path, y_rot, x_rot, on_ground } => {
                let end = path.end();
                self.codec.base = end;
                if self.position.distance_squared(end) > 4096.0 {
                    self.snap_to(end, y_rot, x_rot);
                    self.interpolation.reset();
                } else {
                    self.move_or_interpolate_to(Some(path), y_rot, x_rot, true);
                }
                self.on_ground = on_ground;
            }
            Packet::Move { path, rotation, on_ground } => {
                if let Some(path) = path {
                    self.codec.base = path.end();
                    match rotation {
                        Some((y, x)) => self.move_or_interpolate_to(Some(path), y, x, true),
                        None => self.move_or_interpolate_to(Some(path), 0.0, 0.0, false),
                    }
                } else if let Some((y, x)) = rotation {
                    self.move_or_interpolate_to(None, y, x, true);
                }
                self.on_ground = on_ground;
            }
            Packet::Head(y) => {
                self.lerp_y_head_rot = f64::from(y);
                self.lerp_head_steps = 3;
            }
        }
    }

    /// `ClientLevel.tickNonPassenger` for a mob: `commonTick`, then
    /// `LivingEntity.tick` as it runs on the client.
    fn tick(&mut self) {
        // commonTick: setOldPosAndRot, interpolate, tickCount++.
        self.old_position = self.position;
        self.y_rot_o = self.y_rot;
        self.x_rot_o = self.x_rot;
        self.interpolate();
        self.tick_count += 1;
        // LivingEntity.baseTick: the swing runs on, the hurt flash wears
        // off, and a dead mob ticks its death (`tickDeath`).
        self.swing.tick();
        if self.hurt_time > 0 {
            self.hurt_time -= 1;
        }
        if self.dead {
            self.death_time += 1;
        }
        // LivingEntity.baseTick ends by keeping the head and body yaw.
        self.y_head_rot_o = self.y_head_rot;
        self.y_body_rot_o = self.y_body_rot;
        // aiStep: the head eases to its last packet, then the walk swing.
        if self.lerp_head_steps > 0 {
            let a = 1.0 / f64::from(self.lerp_head_steps);
            let from = f64::from(self.y_head_rot);
            let to = self.lerp_y_head_rot;
            let mut d = (to - from) % 360.0;
            if d >= 180.0 {
                d -= 360.0;
            }
            if d < -180.0 {
                d += 360.0;
            }
            self.y_head_rot = (from + a * d) as f32;
            self.lerp_head_steps -= 1;
        }
        let (dx, dz) = (self.position.x - self.old_position.x, self.position.z - self.old_position.z);
        // calculateEntityAnimation: a dead mob's swing stops.
        let distance = (dx * dx + dz * dz).sqrt() as f32;
        if self.dead {
            self.walk.stop();
        } else {
            self.walk.update(distance, self.baby);
        }
        // Chicken.aiStep after LivingEntity's.
        self.flap_o = self.flap;
        self.flap_speed_o = self.flap_speed;
        self.flap_speed = (self.flap_speed + if self.on_ground { -1.0 } else { 4.0 } * 0.3).clamp(0.0, 1.0);
        if !self.on_ground && self.flapping < 1.0 {
            self.flapping = 1.0;
        }
        self.flapping *= 0.9;
        self.flap += self.flapping * 2.0;
        // Mob.tickHeadTurn: BodyRotationControl.clientTick.
        if dx * dx + dz * dz > 2.500_000_3e-7_f32 as f64 {
            self.y_body_rot = self.y_rot;
            self.y_head_rot = rotate_if_necessary(self.y_head_rot, self.y_body_rot, self.max_head_y_rot);
            self.last_stable_y_head_rot = self.y_head_rot;
            self.head_stable_time = 0;
        } else if (self.y_head_rot - self.last_stable_y_head_rot).abs() > 15.0 {
            self.head_stable_time = 0;
            self.last_stable_y_head_rot = self.y_head_rot;
            self.y_body_rot = rotate_if_necessary(self.y_body_rot, self.y_head_rot, self.max_head_y_rot);
        } else {
            self.head_stable_time += 1;
            if self.head_stable_time > 10 {
                let fraction = ((self.head_stable_time - 10) as f32 / 10.0).clamp(0.0, 1.0);
                self.y_body_rot = rotate_if_necessary(self.y_body_rot, self.y_head_rot, self.max_head_y_rot * (1.0 - fraction));
            }
        }
        // The range checks: each old value within half a turn of the new.
        let keep_near = |old: &mut f32, now: f32| {
            while now - *old < -180.0 {
                *old -= 360.0;
            }
            while now - *old >= 180.0 {
                *old += 360.0;
            }
        };
        keep_near(&mut self.y_rot_o, self.y_rot);
        keep_near(&mut self.y_body_rot_o, self.y_body_rot);
        keep_near(&mut self.x_rot_o, self.x_rot);
        keep_near(&mut self.y_head_rot_o, self.y_head_rot);
        // Bat.tick ends with setupAnimationStates.
        if self.resting {
            self.fly_start = None;
            self.rest_start.get_or_insert(self.tick_count);
        } else {
            self.rest_start = None;
            self.fly_start.get_or_insert(self.tick_count);
        }
    }

    /// `LivingEntityRenderer.extractRenderState` for a partial tick.
    pub fn pose(&self, partial: f32) -> MobPose {
        let head = rot_lerp(partial, self.y_head_rot_o, self.y_head_rot);
        let body = rot_lerp(partial, self.y_body_rot_o, self.y_body_rot);
        let feet = vec_lerp(self.old_position, self.position, f64::from(partial));
        MobPose {
            feet,
            body_rot: body,
            head_yaw: wrap_degrees(head - body),
            head_pitch: if partial == 1.0 { self.x_rot } else { lerp(partial, self.x_rot_o, self.x_rot) },
            walk_position: self.walk.position(partial),
            walk_speed: self.walk.speed(partial),
            age_in_ticks: self.tick_count as f32 + partial,
            light_probe: feet + DVec3::Y * f64::from(self.eye_height),
            flap: lerp(partial, self.flap_o, self.flap),
            flap_speed: lerp(partial, self.flap_speed_o, self.flap_speed),
            resting: self.resting,
            rest_start: self.rest_start,
            fly_start: self.fly_start,
            swing: self.swing.swinging().then(|| self.swing.animation(partial)),
            red_overlay: self.hurt_time > 0 || self.death_time > 0,
            death_time: if self.death_time > 0 { self.death_time as f32 + partial } else { 0.0 },
        }
    }
}

/// What a renderer needs of a mob at a partial tick.
#[derive(Clone, Copy, Debug)]
pub struct MobPose {
    pub feet: DVec3,
    /// `bodyRot`, degrees.
    pub body_rot: f32,
    /// `yRot`: the head's yaw from the body, degrees.
    pub head_yaw: f32,
    /// `xRot`, degrees.
    pub head_pitch: f32,
    pub walk_position: f32,
    pub walk_speed: f32,
    pub age_in_ticks: f32,
    /// `getLightProbePosition`: the eye position.
    pub light_probe: DVec3,
    /// `ChickenRenderState.flap` and `flapSpeed`.
    pub flap: f32,
    pub flap_speed: f32,
    /// `BatRenderState.isResting` and its animation states' start ticks.
    pub resting: bool,
    pub rest_start: Option<i32>,
    pub fly_start: Option<i32>,
    /// `swingAnimation` while a swing runs (`currentSwing`).
    pub swing: Option<f32>,
    /// `hasRedOverlay` (hurt or dying) and `deathTime` with the partial
    /// tick.
    pub red_overlay: bool,
    pub death_time: f32,
}

/// `LivingEntity.SwingState`: an arm swing's ticks and animation.
#[derive(Clone, Copy, Debug, Default)]
struct SwingState {
    /// The running swing's duration, if one runs.
    duration: Option<i32>,
    ticks: i32,
    old_animation: f32,
    animation: f32,
}

impl SwingState {
    /// `startIfAble`: not in the first half of a running swing. A new
    /// swing like the last keeps its animation value.
    fn start_if_able(&mut self, duration: i32) {
        if let Some(running) = self.duration {
            if self.ticks <= running / 2 && self.ticks > 0 {
                return;
            }
        }
        if self.duration != Some(duration) {
            self.old_animation = 0.0;
            self.animation = 0.0;
        }
        self.duration = Some(duration);
        self.ticks = 0;
    }

    fn tick(&mut self) {
        self.old_animation = self.animation;
        match self.duration {
            Some(duration) => {
                if duration > 0 {
                    self.animation = (self.ticks as f32 / duration as f32).min(1.0);
                }
                let ticks = self.ticks;
                self.ticks += 1;
                if ticks > duration {
                    self.duration = None;
                    self.animation = 0.0;
                }
            }
            None => self.animation = 0.0,
        }
    }

    fn swinging(&self) -> bool {
        self.duration.is_some()
    }

    /// `getAnimation(partialTicks)`: wrapping forward past a restart.
    fn animation(&self, partial: f32) -> f32 {
        if self.animation >= self.old_animation {
            lerp(partial, self.old_animation, self.animation)
        } else {
            lerp(partial, self.old_animation, self.animation + 1.0)
        }
    }
}

/// `OverlayTexture`'s red rows (`0xB2FF0000`): the texture keeps 178/255.
pub const RED_OVERLAY_ALPHA: f32 = 178.0 / 255.0;

impl MobPose {
    /// `LivingEntityRenderer.setupRotations`: turned to the body's yaw,
    /// then, dying, tipped over about Z by up to `flip_degrees`
    /// (`getFlipDegrees`).
    pub fn body_rotation(&self, flip_degrees: f32) -> glam::Quat {
        let turn = glam::Quat::from_rotation_y((180.0 - self.body_rot).to_radians());
        if self.death_time > 0.0 {
            let fall = ((self.death_time - 1.0) / 20.0 * 1.6).sqrt().min(1.0);
            turn * glam::Quat::from_rotation_z((fall * flip_degrees).to_radians())
        } else {
            turn
        }
    }

    /// The entity shader's overlay for this mob, as the model mesh carries
    /// it in vertex alpha: negative for the red rows, otherwise the white
    /// rows' texture alpha for `white` progress (`OverlayTexture.u`).
    pub fn overlay(&self, white: f32) -> f32 {
        if self.red_overlay {
            -RED_OVERLAY_ALPHA
        } else {
            let u = (white * 15.0) as i32;
            ((1.0 - u as f32 / 15.0 * 0.75) * 255.0) as i32 as f32 / 255.0
        }
    }
}

impl MobPose {
    /// `BlockPos.containing` of the light probe.
    pub fn light_block(&self) -> (i32, i32, i32) {
        (self.light_probe.x.floor() as i32, self.light_probe.y.floor() as i32, self.light_probe.z.floor() as i32)
    }
}

/// Every tracked mob by entity ID.
#[derive(Default)]
pub struct ClientMobs {
    mobs: HashMap<u64, ClientMob>,
}

impl ClientMobs {
    /// A server tick's mobs: the packets its trackers send, handled as the
    /// client handles them (once per frame, before that frame's ticks).
    /// Returns where the dead mobs the server removed were, with their
    /// boxes (entity event 60, `makePoofParticles`).
    pub fn receive(&mut self, batch: Vec<ServerMob>) -> Vec<(DVec3, f32, f32)> {
        let mut packets = Vec::new();
        for mob in self.mobs.values_mut() {
            mob.seen = false;
        }
        for server in &batch {
            let mob = self.mobs.entry(server.id).or_insert_with(|| ClientMob::spawn(server));
            mob.seen = true;
            mob.baby = server.baby;
            mob.eye_height = server.eye_height;
            mob.max_head_y_rot = server.max_head_y_rot;
            mob.size = server.size;
            mob.resting = server.resting;
            mob.dead = server.dead;
            if server.hurts != mob.hurts_seen {
                mob.hurts_seen = server.hurts;
                mob.damage_event();
            }
            // `handleSwingAnimation`: `swing(hand, DEFAULT)`, six ticks.
            if server.swings != mob.swings_seen {
                mob.swings_seen = server.swings;
                mob.swing.start_if_able(6);
            }
            packets.clear();
            mob.tracker.send_changes(server, &mut packets);
            for packet in packets.drain(..) {
                mob.receive(packet);
            }
        }
        let poofs = self.mobs.values().filter(|mob| !mob.seen && mob.dead).map(|mob| (mob.position, mob.size.0, mob.size.1)).collect();
        self.mobs.retain(|_, mob| mob.seen);
        poofs
    }

    /// Mobs the client has not seen yet, added as their trackers start
    /// (`ClientboundAddEntityPacket`) without a server tick.
    pub fn spawn_missing(&mut self, batch: &[ServerMob]) {
        for server in batch {
            self.mobs.entry(server.id).or_insert_with(|| ClientMob::spawn(server));
        }
    }

    /// Forget every mob (a new world).
    pub fn clear(&mut self) {
        self.mobs.clear();
    }

    /// One client tick of every mob.
    pub fn tick(&mut self) {
        for mob in self.mobs.values_mut() {
            mob.tick();
        }
    }

    pub fn get(&self, id: u64) -> Option<&ClientMob> {
        self.mobs.get(&id)
    }

    /// A mob's render pose, if the client has it.
    /// Every mob, by ID, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (u64, &ClientMob)> {
        self.mobs.iter().map(|(&id, mob)| (id, mob))
    }

    pub fn pose(&self, id: u64, partial: f32) -> Option<MobPose> {
        self.mobs.get(&id).map(|mob| mob.pose(partial.clamp(0.0, 1.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mob(id: u64, x: f64, yaw: f32) -> ServerMob {
        ServerMob { id, position: DVec3::new(x, 64.0, 0.0), y_rot: yaw, x_rot: 0.0, y_head_rot: yaw, on_ground: true, baby: false, eye_height: 1.74, max_head_y_rot: 75.0, update_interval: 3, sync: SyncFlags::default(), resting: false, hurts: 0, dead: false, swings: 0, size: (0.6, 1.8) }
    }

    #[test]
    fn degrees_pack_to_a_256th_of_a_turn() {
        assert_eq!(pack_degrees(90.0), 64);
        assert_eq!(unpack_degrees(64), 90.0);
        assert_eq!(pack_degrees(-1.0), -1);
        assert_eq!(unpack_degrees(pack_degrees(359.0)), -1.40625);
    }

    #[test]
    fn a_walking_mob_follows_its_server_path_smoothly() {
        let mut mobs = ClientMobs::default();
        let mut x = 0.0;
        let mut seen = Vec::new();
        for _ in 0..60 {
            x += 0.1;
            mobs.receive(vec![mob(1, x, -90.0)]);
            mobs.tick();
            seen.push(mobs.get(1).unwrap().position.x);
        }
        // After the first packets it moves every tick, never backwards,
        // a few ticks behind the server.
        for pair in seen[10..].windows(2) {
            assert!(pair[1] > pair[0], "{seen:?}");
            assert!(pair[1] - pair[0] < 0.2, "{seen:?}");
        }
        assert!(x - seen[59] < 0.6 && x - seen[59] > 0.0, "{} {}", x, seen[59]);
        // Walking east (yaw -90) turns the body that way.
        let client = mobs.get(1).unwrap();
        assert!((wrap_degrees(client.y_body_rot + 90.0)).abs() < 2.0, "{}", client.y_body_rot);
        assert!(client.walk.speed(1.0) > 0.3);
    }

    #[test]
    fn the_head_eases_to_a_turned_packet() {
        let mut mobs = ClientMobs::default();
        mobs.receive(vec![mob(1, 0.0, 0.0)]);
        mobs.tick();
        let mut turned = mob(1, 0.0, 0.0);
        turned.y_head_rot = 60.0;
        let mut heads = Vec::new();
        for _ in 0..8 {
            mobs.receive(vec![turned]);
            mobs.tick();
            heads.push(mobs.get(1).unwrap().y_head_rot);
        }
        assert!(heads.windows(2).all(|w| w[1] >= w[0]), "{heads:?}");
        assert!((heads[7] - 59.0625).abs() < 1.0, "{heads:?}");
    }
}

/// Every living mob of a server entity world, as trackers see it.
pub fn server_mobs(world: &minecraftoss_entities::world::EntityWorld) -> Vec<ServerMob> {
    // Vanilla `EntityType` eye heights for the mobs without a helper;
    // babies are half size.
    let scaled = |eye: f32, baby: bool| if baby { eye * 0.5 } else { eye };
    let mut out = Vec::new();
    let mut push = |id: u64, y_rot: f32, look: &minecraftoss_entities::look::LookControl, body: &minecraftoss_entities::movement::Body, baby: bool, eye_height: f32| {
        let sync = SyncFlags { needs_sync: body.needs_sync, ..SyncFlags::default() };
        out.push(ServerMob { id, position: body.position, y_rot, x_rot: look.pitch, y_head_rot: look.head_yaw, on_ground: body.on_ground, baby, eye_height, max_head_y_rot: 75.0, update_interval: 3, sync, resting: false, hurts: 0, dead: false, swings: 0, size: (body.width, body.height) });
    };
    for e in world.cows() {
        let baby = e.cow.age.baby();
        let eye = match &e.horse {
            Some(horse) => horse.kind.eye_height(baby),
            None => scaled(1.3, baby),
        };
        push(e.id, e.cow.yaw, &e.look_control, &e.cow.body, baby, eye);
    }
    for e in world.sheep() {
        let baby = e.sheep.age.baby();
        push(e.id, e.yaw, &e.look_control, &e.body, baby, scaled(1.235, baby));
    }
    for e in world.pigs() {
        let baby = e.pig.age.baby();
        push(e.id, e.pig.yaw, &e.look_control, &e.pig.body, baby, scaled(0.765, baby));
    }
    for e in world.chickens() {
        let baby = e.chicken.age.baby();
        push(e.id, e.chicken.yaw, &e.look_control, &e.chicken.body, baby, scaled(0.644, baby));
    }
    for e in world.bats() {
        push(e.id, e.yaw, &e.look_control, &e.bat.body, false, 0.45);
    }
    let resting_bats: std::collections::HashSet<u64> = world.bats().iter().filter(|e| e.bat.resting).map(|e| e.id).collect();
    // The melee swingers' swings (humanoid models show them).
    let swings: HashMap<u64, u32> = world
        .zombies()
        .iter()
        .filter_map(|e| Some((e.id, e.ai.as_deref()?.state.melee.swings)))
        .chain(world.skeletons().iter().filter_map(|e| Some((e.id, e.ai.as_deref()?.state.melee.swings))))
        .chain(world.endermen().iter().map(|e| (e.id, e.ai.state.melee.swings)))
        .collect();
    for e in world.zombies() {
        push(e.id, e.yaw, &e.look_control, &e.zombie.body, e.zombie.baby, e.zombie.eye_height());
    }
    for e in world.skeletons() {
        push(e.id, e.yaw, &e.look_control, &e.skeleton.body, false, e.skeleton.eye_height());
    }
    for e in world.spiders() {
        push(e.id, e.spider.yaw, &e.ai.state.look_control, &e.spider.body, false, e.spider.eye_height());
    }
    for e in world.creepers() {
        push(e.id, e.creeper.yaw, &e.ai.state.look_control, &e.creeper.body, false, e.creeper.body.height * 0.85);
    }
    for e in world.villagers() {
        let baby = e.villager.age.baby();
        match e.ai.as_deref() {
            Some(ai) => push(e.id, ai.yaw, &ai.look_control, &e.villager.body, baby, e.eye_height()),
            // Without AI it faces its saved yaw, head and all.
            None => push(e.id, e.yaw, &minecraftoss_entities::look::LookControl::new(e.yaw), &e.villager.body, baby, e.eye_height()),
        }
    }
    for e in world.endermen() {
        push(e.id, e.enderman.yaw, &e.ai.state.look_control, &e.enderman.body, false, e.enderman.eye_height());
    }
    for e in world.iron_golems() {
        push(e.id, e.golem.yaw, &e.ai.state.look_control, &e.golem.body, false, minecraftoss_entities::iron_golem::EYE_HEIGHT);
    }
    for e in world.slimes() {
        push(e.id, e.slime.yaw, &e.ai.state.look_control, &e.slime.body, false, e.slime.eye_height());
    }
    for e in world.witches() {
        push(e.id, e.witch.yaw, &e.ai.state.look_control, &e.witch.body, false, minecraftoss_entities::witch::EYE_HEIGHT);
    }
    for e in world.wolves() {
        push(e.id, e.wolf.yaw, &e.ai.state.look_control, &e.wolf.body, e.wolf.baby(), e.wolf.eye_height());
    }
    for mob in &mut out {
        mob.resting = resting_bats.contains(&mob.id);
        if let Some(damage) = world.damage_state(mob.id) {
            mob.hurts = damage.hurts;
            mob.dead = damage.dead;
        }
        mob.swings = swings.get(&mob.id).copied().unwrap_or(0);
    }
    out
}

/// Every living mob's shadow (`EntityRenderer.extractShadow`) from its
/// client copy: the renderer's radius (`MobRenderer` scales it by
/// `getAgeScale`, a half for babies; a baby villager's halves again, and a
/// slime's is a quarter of its size), at full strength.
pub fn shadow_casters(world: &minecraftoss_entities::world::EntityWorld, mobs: &ClientMobs, camera: DVec3, partial: f32) -> Vec<crate::mesh::ShadowCaster> {
    let age = |baby: bool| if baby { 0.5 } else { 1.0 };
    let mut kinds: Vec<(u64, f32)> = Vec::new();
    // `AbstractHorseRenderer`'s shadow is 0.75.
    kinds.extend(world.cows().iter().map(|e| (e.id, if e.horse.is_some() { 0.75 } else { 0.7 } * age(e.cow.age.baby()))));
    kinds.extend(world.sheep().iter().map(|e| (e.id, 0.7 * age(e.sheep.age.baby()))));
    kinds.extend(world.pigs().iter().map(|e| (e.id, 0.7 * age(e.pig.age.baby()))));
    kinds.extend(world.chickens().iter().map(|e| (e.id, 0.3 * age(e.chicken.age.baby()))));
    kinds.extend(world.bats().iter().map(|e| (e.id, 0.25)));
    kinds.extend(world.zombies().iter().map(|e| (e.id, 0.5 * age(e.zombie.baby))));
    kinds.extend(world.skeletons().iter().map(|e| (e.id, 0.5)));
    kinds.extend(world.spiders().iter().map(|e| (e.id, 0.8)));
    kinds.extend(world.creepers().iter().map(|e| (e.id, 0.5)));
    kinds.extend(world.villagers().iter().map(|e| (e.id, if e.villager.age.baby() { 0.5 * 0.5 * 0.5 } else { 0.5 })));
    kinds.extend(world.endermen().iter().map(|e| (e.id, 0.5)));
    kinds.extend(world.iron_golems().iter().map(|e| (e.id, 0.7)));
    kinds.extend(world.slimes().iter().map(|e| (e.id, e.slime.size as f32 * 0.25)));
    kinds.extend(world.witches().iter().map(|e| (e.id, 0.5)));
    kinds.extend(world.wolves().iter().map(|e| (e.id, 0.5 * age(e.wolf.baby()))));
    kinds
        .into_iter()
        .filter_map(|(id, radius)| {
            let mob = mobs.get(id)?;
            Some(crate::mesh::ShadowCaster {
                position: mob.pose(partial.clamp(0.0, 1.0)).feet,
                distance_squared: mob.position.distance_squared(camera),
                radius,
                strength: 1.0,
            })
        })
        .collect()
}

/// `Mth.cos` (the 65536-entry sine table).
pub fn mth_cos(x: f32) -> f32 {
    minecraftoss_player::mth::cos(f64::from(x))
}

/// `QuadrupedModel.setupAnim`'s leg swing: the right hind, left hind, right
/// front and left front legs' X rotation, radians.
pub fn quadruped_legs(walk_position: f32, walk_speed: f32) -> [f32; 4] {
    let a = mth_cos(walk_position * 0.6662) * 1.4 * walk_speed;
    let b = mth_cos(walk_position * 0.6662 + std::f32::consts::PI) * 1.4 * walk_speed;
    [a, b, b, a]
}

/// A packet as a trace records it, for replaying a vanilla client's view.
#[derive(Clone, Debug)]
pub enum TracePacket {
    /// Path steps as decoded (a tick offset of -1 marks a linear path).
    Sync { steps: Vec<(DVec3, i32)>, y_rot: f32, x_rot: f32, on_ground: bool },
    Move { steps: Option<Vec<(DVec3, i32)>>, rotation: Option<(f32, f32)>, on_ground: bool },
    Head(f32),
    /// `ClientboundDamageEventPacket`.
    Damage,
    /// Entity data with this health.
    Health(f32),
    /// `ClientboundSwingAnimationPacket` with the swing's duration.
    Swing(i32),
    /// Entity event 3: it died (`setHealth(0)`).
    Death,
}

fn trace_path(steps: Vec<(DVec3, i32)>) -> Path {
    if steps.len() == 1 && steps[0].1 < 0 {
        Path::Linear(steps[0].0)
    } else {
        Path::Stepped(steps)
    }
}

impl ClientMob {
    /// A client mob made from a traced `ClientboundAddEntityPacket`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_add_packet(position: DVec3, y_rot: f32, x_rot: f32, y_head_rot: f32, baby: bool, eye_height: f32, max_head_y_rot: f32, update_interval: i32) -> Self {
        let server = ServerMob { id: 0, position, y_rot, x_rot, y_head_rot, on_ground: false, baby, eye_height, max_head_y_rot, update_interval, sync: SyncFlags::default(), resting: false, hurts: 0, dead: false, swings: 0, size: (0.6, 1.8) };
        let mut mob = Self::spawn(&server);
        mob.y_rot = y_rot;
        mob.y_rot_o = y_rot;
        mob.x_rot = x_rot.clamp(-90.0, 90.0);
        mob.x_rot_o = mob.x_rot;
        mob.y_head_rot = y_head_rot;
        mob.y_head_rot_o = y_head_rot;
        mob.y_body_rot = y_head_rot;
        mob.y_body_rot_o = y_head_rot;
        mob.last = (position, mob.y_rot, mob.x_rot);
        mob
    }

    /// Handles a traced packet.
    pub fn handle(&mut self, packet: TracePacket) {
        self.receive(match packet {
            TracePacket::Sync { steps, y_rot, x_rot, on_ground } => Packet::Sync { path: trace_path(steps), y_rot, x_rot, on_ground },
            TracePacket::Move { steps, rotation, on_ground } => Packet::Move { path: steps.map(trace_path), rotation, on_ground },
            TracePacket::Head(y) => Packet::Head(y),
            TracePacket::Damage => return self.damage_event(),
            TracePacket::Health(health) => {
                self.dead = health <= 0.0;
                return;
            }
            TracePacket::Swing(duration) => return self.swing.start_if_able(duration),
            TracePacket::Death => {
                self.dead = true;
                return;
            }
        });
    }

    /// `hurtTime` and `deathTime`.
    pub fn hurt_and_death_time(&self) -> (i32, i32) {
        (self.hurt_time, self.death_time)
    }

    /// `getSwingAnimation(1)` and `isSwinging`.
    pub fn swing_state(&self) -> (f32, bool) {
        (self.swing.animation(1.0), self.swing.swinging())
    }

    /// One client tick.
    pub fn client_tick(&mut self) {
        self.tick();
    }

    /// The interpolation state, for debugging a replay.
    pub fn interpolation_debug(&self) -> String {
        let d = &self.interpolation;
        format!(
            "target {:?} remaining {:?} last {:?} current_step_ticks {} remaining_ticks {} speed {}",
            d.position, d.remaining, d.last_step, d.current_step_ticks, d.remaining_ticks, d.speed
        )
    }
}

fn to_trace_path(path: Path) -> Vec<(DVec3, i32)> {
    match path {
        Path::Linear(p) => vec![(p, -1)],
        Path::Stepped(steps) => steps,
    }
}

/// A mob's server tracker driven by traced server states, for checking
/// the packets it sends against a vanilla trace.
pub struct TraceTracker(Tracker);

impl TraceTracker {
    /// The tracker as `ServerEntity` starts it for a mob in this state.
    pub fn new(position: DVec3, y_rot: f32, x_rot: f32, y_head_rot: f32, on_ground: bool) -> Self {
        Self(Tracker::new(&Self::mob(position, y_rot, x_rot, y_head_rot, on_ground, SyncFlags::default())))
    }

    fn mob(position: DVec3, y_rot: f32, x_rot: f32, y_head_rot: f32, on_ground: bool, sync: SyncFlags) -> ServerMob {
        ServerMob { id: 0, position, y_rot, x_rot, y_head_rot, on_ground, baby: false, eye_height: 1.0, max_head_y_rot: 75.0, update_interval: 3, sync, resting: false, hurts: 0, dead: false, swings: 0, size: (0.6, 1.8) }
    }

    /// One `sendChanges` with the mob in this state: the packets, as the
    /// client decodes them.
    pub fn send_changes(&mut self, position: DVec3, y_rot: f32, x_rot: f32, y_head_rot: f32, on_ground: bool, sync: SyncFlags) -> Vec<TracePacket> {
        let mut out = Vec::new();
        self.0.send_changes(&Self::mob(position, y_rot, x_rot, y_head_rot, on_ground, sync), &mut out);
        out.into_iter()
            .map(|p| match p {
                Packet::Sync { path, y_rot, x_rot, on_ground } => TracePacket::Sync { steps: to_trace_path(path), y_rot, x_rot, on_ground },
                Packet::Move { path, rotation, on_ground } => TracePacket::Move { steps: path.map(to_trace_path), rotation, on_ground },
                Packet::Head(y) => TracePacket::Head(y),
            })
            .collect()
    }
}
