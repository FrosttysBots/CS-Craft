//! Pinned Java 26.3 slime state. Sources: `EntityTypes.SLIME` (0.52 wide
//! and tall, eyes at 0.325, all scaled by its size), `AbstractCubeMob`
//! (`setSize`: size² health and `0.2F + 0.1F * size` speed; the landing
//! squash; `getSoundVolume`, `getSoundPitch`, `getJumpDelay`) and
//! `Slime.setSize` (attack damage and experience equal to the size).
use crate::{health::DamageState, movement::Body};
use glam::DVec3;

#[derive(Clone, Debug)]
pub struct Slime {
    pub body: Body,
    pub health: f32,
    pub damage: DamageState,
    /// `ID_SIZE`: 1, 2 or 4 when spawned naturally (1 to 127).
    pub size: i32,
    pub persistence_required: bool,
    pub yaw: f32,
    /// `wasOnGround`: on the ground when its last tick ended.
    pub was_on_ground: bool,
    /// `targetSquish`, `squish` and `oSquish`: the squash the renderer
    /// draws, pressed flat on landing and stretched on take-off.
    pub target_squish: f32,
    pub squish: f32,
    pub previous_squish: f32,
}

impl Slime {
    /// A slime of `size` (clamped as `setSize` does) at full health.
    pub fn new(position: DVec3, size: i32) -> Self {
        let size = size.clamp(1, 127);
        let (width, height) = dimensions(size);
        Self {
            body: Body::new(position, width, height),
            health: (size * size) as f32,
            damage: DamageState::default(),
            size,
            persistence_required: false,
            yaw: 0.0,
            was_on_ground: false,
            target_squish: 0.0,
            squish: 0.0,
            previous_squish: 0.0,
        }
    }

    /// `setCubeMobHealth`: the size squared.
    pub fn max_health(&self) -> f32 {
        (self.size * self.size) as f32
    }

    /// `MOVEMENT_SPEED`'s base, `0.2F + 0.1F * size`.
    pub fn movement_speed(&self) -> f64 {
        f64::from(0.2_f32 + 0.1_f32 * self.size as f32)
    }

    /// `Slime.setSize`: `ATTACK_DAMAGE` is the size.
    pub fn attack_damage(&self) -> f32 {
        self.size as f32
    }

    pub fn eye_height(&self) -> f32 {
        0.325_f32 * self.size as f32
    }

    /// `isTiny`: a size-1 slime deals no damage and squeaks.
    pub fn tiny(&self) -> bool {
        self.size <= 1
    }

    /// `getSoundVolume`: 0.4 a size step.
    pub fn sound_volume(&self) -> f32 {
        0.4 * self.size as f32
    }
}

/// `EntityDimensions.scale(size)` of the 0.52 cube.
pub fn dimensions(size: i32) -> (f32, f32) {
    let scale = size as f32;
    (0.52_f32 * scale, 0.52_f32 * scale)
}

/// The landing's particle loop in `AbstractCubeMob.tick`: `i < size * 16`
/// with `size` the width doubled, as floats. Each turn draws two floats
/// from the slime's random (the server draws them too).
pub fn landing_particles(size: i32) -> usize {
    let (width, _) = dimensions(size);
    let limit = width * 2.0 * 16.0;
    (0..).take_while(|&i| (i as f32) < limit).count()
}
