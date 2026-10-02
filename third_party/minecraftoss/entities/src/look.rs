//! Pinned 26.3 LookControl, BodyRotationControl and idle look goal state.
use crate::control::minecraft_atan2;
use crate::tempt::PlayerCandidate;
use glam::DVec3;
use minecraftoss_player::rng::LegacyRandom;
use minecraftoss_player::World;

#[derive(Clone, Debug, Default)]
pub struct LookAtPlayerState {
    pub target_id: Option<u64>,
    pub look_time: i32,
}

impl LookAtPlayerState {
    /// `LookAtPlayerGoal.canUse` for an animal's look distance of six
    /// (`getNearestPlayer` with non-combat conditions, which still ask for
    /// sight): among the players within range of the feet whom the eyes
    /// can see, the one nearest the eyes, the first on ties.
    pub fn can_use(&mut self, world: &dyn World, position: DVec3, eye_height: f32, players: &[PlayerCandidate], random: &mut LegacyRandom) -> bool {
        if random.next_float() >= 0.02 {
            return false;
        }
        let range = 6.0_f64;
        let eye = position + DVec3::new(0.0, f64::from(eye_height), 0.0);
        let mut best: Option<(f64, u64)> = None;
        for player in players {
            if !player.alive || player.spectator || position.distance_squared(player.position) > range * range {
                continue;
            }
            let player_eye = player.position + DVec3::new(0.0, f64::from(player.eye_height), 0.0);
            if !crate::sight::line_of_sight(world, eye, player_eye) {
                continue;
            }
            let distance = player.position.distance_squared(eye);
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, player.id));
            }
        }
        self.target_id = best.map(|(_, id)| id);
        self.target_id.is_some()
    }

    pub fn can_continue(&self, cow_position: DVec3, players: &[PlayerCandidate]) -> bool {
        self.can_continue_within(cow_position, players, 6.0)
    }

    pub fn can_continue_within(&self, cow_position: DVec3, players: &[PlayerCandidate], range: f64) -> bool {
        self.look_time > 0
            && players.iter().any(|player| {
                Some(player.id) == self.target_id
                    && player.alive
                    && cow_position.distance_squared(player.position) <= range * range
            })
    }

    pub fn start(&mut self, random: &mut LegacyRandom) {
        let duration = 40 + random.next_int(40) as i32;
        self.look_time = (duration + 1) / 2; // Goal.adjustedTickDelay.
    }

    pub fn stop(&mut self) {
        self.target_id = None;
    }

    pub fn tick(&mut self, players: &[PlayerCandidate]) -> Option<DVec3> {
        let player = players
            .iter()
            .find(|player| Some(player.id) == self.target_id)?;
        if !player.alive {
            return None;
        }
        self.look_time -= 1;
        Some(player.position + DVec3::new(0.0, f64::from(player.eye_height), 0.0))
    }
}

#[derive(Clone, Debug, Default)]
pub struct RandomLookState {
    pub rel_x: f64,
    pub rel_z: f64,
    pub look_time: i32,
}

impl RandomLookState {
    pub fn can_use(random: &mut LegacyRandom) -> bool {
        random.next_float() < 0.02
    }

    pub fn start(&mut self, random: &mut LegacyRandom) {
        // `Math.cos`/`Math.sin` as the JVM rounds them.
        let angle = std::f64::consts::TAU * random.next_double();
        self.rel_x = minecraftoss_player::jmath::cos(angle);
        self.rel_z = minecraftoss_player::jmath::sin(angle);
        self.look_time = 20 + random.next_int(20) as i32;
    }

    pub fn can_continue(&self) -> bool {
        self.look_time >= 0
    }

    pub fn tick(&mut self, position: DVec3, eye_height: f32) -> DVec3 {
        self.look_time -= 1;
        DVec3::new(
            position.x + self.rel_x,
            position.y + f64::from(eye_height),
            position.z + self.rel_z,
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct LookControl {
    pub wanted: DVec3,
    pub cooldown: i32,
    pub head_yaw: f32,
    pub pitch: f32,
    y_max_rot_speed: f32,
    x_max_rot_angle: f32,
}

impl LookControl {
    pub fn new(head_yaw: f32) -> Self {
        Self {
            head_yaw,
            ..Self::default()
        }
    }

    pub fn set_look_at(&mut self, target: DVec3) {
        self.set_look_at_with_limits(target, 10.0, 40.0);
    }

    pub fn set_look_at_with_limits(&mut self, target: DVec3, y_max: f32, x_max: f32) {
        self.wanted = target;
        self.y_max_rot_speed = y_max;
        self.x_max_rot_angle = x_max;
        self.cooldown = 2;
    }

    pub fn tick(&mut self, position: DVec3, eye_height: f32, body_yaw: f32, navigating: bool) {
        self.pitch = 0.0;
        if self.cooldown > 0 {
            self.cooldown -= 1;
            let xd = self.wanted.x - position.x;
            let yd = self.wanted.y - (position.y + f64::from(eye_height));
            let zd = self.wanted.z - position.z;
            if xd.abs() > 1.0e-5_f32 as f64 || zd.abs() > 1.0e-5_f32 as f64 {
                // Java folds 180.0F / (float)PI in f32 before widening.
                let target = (minecraft_atan2(zd, xd) * 57.2957763671875_f64) as f32 - 90.0;
                self.head_yaw = rotate_towards(self.head_yaw, target, self.y_max_rot_speed);
            }
            let horizontal = (xd * xd + zd * zd).sqrt();
            if yd.abs() > 1.0e-5_f32 as f64 || horizontal.abs() > 1.0e-5_f32 as f64 {
                let target = -(minecraft_atan2(yd, horizontal) * 57.2957763671875_f64) as f32;
                self.pitch = rotate_towards(self.pitch, target, self.x_max_rot_angle);
            }
        } else {
            self.head_yaw = rotate_towards(self.head_yaw, body_yaw, 10.0);
        }
        if navigating {
            self.head_yaw = rotate_if_necessary(self.head_yaw, body_yaw, 75.0);
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BodyRotation {
    pub body_yaw: f32,
    stable_time: i32,
    last_stable_head_yaw: f32,
}

impl BodyRotation {
    pub fn new(body_yaw: f32) -> Self {
        Self {
            body_yaw,
            ..Self::default()
        }
    }

    pub fn tick(&mut self, yaw: f32, look: &mut LookControl, old_pos: DVec3, pos: DVec3) {
        let xd = pos.x - old_pos.x;
        let zd = pos.z - old_pos.z;
        if xd * xd + zd * zd > 2.5000003e-7_f32 as f64 {
            self.body_yaw = yaw;
            look.head_yaw = rotate_if_necessary(look.head_yaw, self.body_yaw, 75.0);
            self.last_stable_head_yaw = look.head_yaw;
            self.stable_time = 0;
        } else if (look.head_yaw - self.last_stable_head_yaw).abs() > 15.0 {
            self.stable_time = 0;
            self.last_stable_head_yaw = look.head_yaw;
            self.body_yaw = rotate_if_necessary(self.body_yaw, look.head_yaw, 75.0);
        } else {
            self.stable_time += 1;
            if self.stable_time > 10 {
                let fraction = ((self.stable_time - 10) as f32 / 10.0).clamp(0.0, 1.0);
                let remaining = 75.0 * (1.0 - fraction);
                self.body_yaw = rotate_if_necessary(self.body_yaw, look.head_yaw, remaining);
            }
        }
    }
}

fn wrap_degrees(angle: f32) -> f32 {
    let mut angle = angle % 360.0;
    if angle >= 180.0 {
        angle -= 360.0;
    }
    if angle < -180.0 {
        angle += 360.0;
    }
    angle
}

fn rotate_towards(from: f32, to: f32, max: f32) -> f32 {
    from + wrap_degrees(to - from).clamp(-max, max)
}

fn rotate_if_necessary(base: f32, target: f32, max: f32) -> f32 {
    target - wrap_degrees(target - base).clamp(-max, max)
}
