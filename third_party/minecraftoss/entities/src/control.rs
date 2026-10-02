//! Ground mob MoveControl from pinned Java 26.3, separate from goals and paths.
//! The caller supplies block-shape/path-type facts from the shared world.
use glam::DVec3;
use minecraftoss_player::minecraft_sin_cos;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoveOperation {
    Wait,
    MoveTo,
    Strafe,
    Jumping,
}

#[derive(Clone, Copy, Debug)]
pub struct MoveFrame {
    pub yaw: f32,
    pub speed: f32,
    pub forward: f32,
    pub sideways: f32,
    pub jump: bool,
}

#[derive(Clone, Debug)]
pub struct MoveControl {
    pub wanted: DVec3,
    pub speed_modifier: f64,
    pub strafe_forwards: f32,
    pub strafe_right: f32,
    pub operation: MoveOperation,
}

impl Default for MoveControl {
    fn default() -> Self {
        Self {
            wanted: DVec3::ZERO,
            speed_modifier: 0.0,
            strafe_forwards: 0.0,
            strafe_right: 0.0,
            operation: MoveOperation::Wait,
        }
    }
}

impl MoveControl {
    pub fn has_wanted(&self) -> bool {
        self.operation == MoveOperation::MoveTo
    }

    pub fn set_wanted_position(&mut self, wanted: DVec3, speed_modifier: f64) {
        self.wanted = wanted;
        self.speed_modifier = speed_modifier;
        if self.operation != MoveOperation::Jumping {
            self.operation = MoveOperation::MoveTo;
        }
    }

    pub fn strafe(&mut self, forwards: f32, right: f32) {
        self.operation = MoveOperation::Strafe;
        self.strafe_forwards = forwards;
        self.strafe_right = right;
        self.speed_modifier = 0.25;
    }

    pub fn set_wait(&mut self) {
        self.operation = MoveOperation::Wait;
    }

    /// DrownedMoveControl's water branch. Unlike the ground branch, MOVE_TO
    /// remains active after the controller tick.
    pub fn tick_drowned_swimming(
        &mut self,
        position: DVec3,
        yaw: f32,
        speed: f32,
        sideways: f32,
        movement_speed: f64,
        navigation_done: bool,
        target_above: bool,
        velocity: &mut DVec3,
    ) -> MoveFrame {
        if target_above {
            velocity.y += 0.002;
        }
        if self.operation != MoveOperation::MoveTo || navigation_done {
            return MoveFrame {
                yaw,
                speed: 0.0,
                forward: 0.0,
                sideways,
                jump: false,
            };
        }
        let delta = self.wanted - position;
        let distance = delta.length();
        let normalized_y = delta.y / distance;
        let target_yaw = (minecraft_atan2(delta.z, delta.x) * 57.2957763671875_f64) as f32 - 90.0;
        let yaw = rotlerp(yaw, target_yaw, 90.0);
        let target_speed = (self.speed_modifier * movement_speed) as f32;
        let speed = speed + 0.125_f32 * (target_speed - speed);
        velocity.x += f64::from(speed) * delta.x * 0.005;
        velocity.y += f64::from(speed) * normalized_y * 0.1;
        velocity.z += f64::from(speed) * delta.z * 0.005;
        MoveFrame {
            yaw,
            speed,
            forward: speed,
            sideways,
            jump: false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        position: DVec3,
        on_ground: bool,
        yaw: f32,
        speed: f32,
        forward: f32,
        sideways: f32,
        movement_speed: f64,
        width: f32,
        step_height: f32,
        obstacle_top: Option<f64>,
        strafe_walkable: impl Fn(f32, f32) -> bool,
    ) -> MoveFrame {
        let mut frame = MoveFrame {
            yaw,
            speed,
            forward,
            sideways,
            jump: false,
        };
        match self.operation {
            MoveOperation::Strafe => {
                let speed = movement_speed as f32;
                let modified = self.speed_modifier as f32 * speed;
                let mut xa = self.strafe_forwards;
                let mut za = self.strafe_right;
                let mut dist = (xa * xa + za * za).sqrt();
                if dist < 1.0 {
                    dist = 1.0;
                }
                dist = modified / dist;
                xa *= dist;
                za *= dist;
                let (sin, cos) = minecraft_sin_cos(yaw as f64);
                let dx = xa * cos as f32 - za * sin as f32;
                let dz = za * cos as f32 + xa * sin as f32;
                // The caller evaluates PathType.WALKABLE for the projected step.
                if !strafe_walkable(dx, dz) {
                    self.strafe_forwards = 1.0;
                    self.strafe_right = 0.0;
                }
                frame.speed = modified;
                frame.forward = self.strafe_forwards;
                frame.sideways = self.strafe_right;
                self.operation = MoveOperation::Wait;
            }
            MoveOperation::MoveTo => {
                self.operation = MoveOperation::Wait;
                let xd = self.wanted.x - position.x;
                let zd = self.wanted.z - position.z;
                let yd = self.wanted.y - position.y;
                if xd * xd + yd * yd + zd * zd < 2.5000003e-7_f32 as f64 {
                    frame.forward = 0.0;
                    return frame;
                }
                // The pinned common bytecode folds 180.0F / (float)PI to this
                // double constant before the angle multiplication.
                let target = (minecraft_atan2(zd, xd) * 57.2957763671875_f64) as f32 - 90.0;
                frame.yaw = rotlerp(yaw, target, 90.0);
                frame.speed = (self.speed_modifier * movement_speed) as f32;
                frame.forward = frame.speed; // Mob.setSpeed also sets Zza.
                let should_jump = yd > step_height as f64
                    && xd * xd + zd * zd < f64::from(width.max(1.0))
                    || obstacle_top.is_some_and(|top| position.y < top);
                if should_jump {
                    frame.jump = true;
                    self.operation = MoveOperation::Jumping;
                }
            }
            MoveOperation::Jumping => {
                frame.speed = (self.speed_modifier * movement_speed) as f32;
                frame.forward = frame.speed;
                if on_ground {
                    self.operation = MoveOperation::Wait;
                }
            }
            MoveOperation::Wait => frame.forward = 0.0,
        }
        frame
    }
}

/// `MoveControl`'s obstacle: the top of a collision shape at the mob's
/// feet, unless the block is a door or a fence; a mob below it jumps.
pub fn obstacle_top<W: minecraftoss_player::World + ?Sized>(world: &W, position: DVec3) -> Option<f64> {
    let feet = (position.x.floor() as i32, position.y.floor() as i32, position.z.floor() as i32);
    world
        .block(feet)
        .filter(|b| !b.id.ends_with("_door") && !b.id.ends_with("_fence"))
        .and_then(|_| world.collision_boxes(feet).iter().map(|b| b[4]).reduce(f64::max))
        .map(|top| top + f64::from(feet.1))
}

pub(crate) fn rotlerp(from: f32, to: f32, max: f32) -> f32 {
    let mut diff = (to - from) % 360.0;
    if diff >= 180.0 {
        diff -= 360.0;
    }
    if diff < -180.0 {
        diff += 360.0;
    }
    let result = from + diff.clamp(-max, max);
    if result < 0.0 {
        result + 360.0
    } else if result > 360.0 {
        result - 360.0
    } else {
        result
    }
}

/// `Mth.atan2` (in the player crate, which the player's hurt direction
/// shares).
pub(crate) use minecraftoss_player::mth::atan2 as minecraft_atan2;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotation_and_wait_follow_pinned_controller() {
        let mut control = MoveControl::default();
        control.set_wanted_position(DVec3::new(10.5, 1.0, 4.5), 1.0);
        let frame = control.tick(
            DVec3::new(2.5, 1.0, 4.5),
            false,
            0.0,
            0.0,
            0.0,
            0.0,
            0.2,
            0.9,
            0.6,
            None,
            |_, _| true,
        );
        assert_eq!((frame.yaw, frame.speed, frame.forward), (270.0, 0.2, 0.2));
        assert!(!control.has_wanted());
        let wait = control.tick(
            DVec3::new(2.5, 1.0, 4.5),
            true,
            frame.yaw,
            frame.speed,
            frame.forward,
            0.0,
            0.2,
            0.9,
            0.6,
            None,
            |_, _| true,
        );
        assert_eq!((wait.speed, wait.forward), (0.2, 0.0));
    }
}
