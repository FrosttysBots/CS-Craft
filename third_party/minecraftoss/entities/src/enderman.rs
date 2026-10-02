//! Pinned Java 26.3 enderman state and the world reads its goals share.
//! Sources: `EntityTypes.ENDERMAN` (0.6 wide, 2.9 tall, eyes at 2.55),
//! `Enderman.createAttributes` (40 health, 0.3 speed and 0.15 more with a
//! target, 7 attack damage, 64 follow range, a one-block step), its
//! `teleport`, `teleportTowards`, `teleport(x, y, z)` and
//! `canRandomlyTeleportTo`, `LivingEntity.randomTeleport` and
//! `isLookingAtMe`, and `Entity.calculateViewVector`.
use crate::{health::DamageState, movement::Body, sight::line_of_sight};
use glam::DVec3;
use minecraftoss_player::{
    collision::no_block_collision, holds_fluid, minecraft_sin_cos, rng::LegacyRandom, World,
};

pub const MAX_HEALTH: f32 = 40.0;
pub const WIDTH: f32 = 0.6;
pub const HEIGHT: f32 = 2.9;
pub const EYE_HEIGHT: f32 = 2.55;
pub const ATTACK_DAMAGE: f32 = 7.0;
pub const FOLLOW_RANGE: f64 = 64.0;

/// `MOVEMENT_SPEED`: 0.3, and with a target `SPEED_MODIFIER_ATTACKING`'s
/// 0.15 more (`AttributeInstance.calculateValue` adds in doubles).
pub fn movement_speed(targeting: bool) -> f64 {
    let base = f64::from(0.3_f32);
    if targeting {
        base + f64::from(0.15_f32)
    } else {
        base
    }
}

#[derive(Clone, Debug)]
pub struct Enderman {
    pub body: Body,
    pub health: f32,
    pub damage: DamageState,
    pub persistence_required: bool,
    pub yaw: f32,
}

impl Enderman {
    pub fn new(position: DVec3) -> Self {
        let mut body = Body::new(position, WIDTH, HEIGHT);
        // `STEP_HEIGHT` 1.
        body.step_height = 1.0;
        Self { body, health: MAX_HEALTH, damage: DamageState::default(), persistence_required: false, yaw: 0.0 }
    }

    pub fn eye_height(&self) -> f32 {
        EYE_HEIGHT
    }
}

/// Where a player looks, for `isLookingAtMe`: its head's yaw
/// (`getViewYRot` is `yHeadRot`), its pitch, and whether something in
/// `#gaze_disguise_equipment` on its head hides the stare
/// (`PLAYER_NOT_WEARING_DISGUISE_ITEM`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerView {
    pub head_yaw: f32,
    pub pitch: f32,
    pub disguised: bool,
}

/// `Entity.calculateViewVector`: through `Mth.sin` and `Mth.cos`, in floats.
pub fn view_vector(pitch: f32, yaw: f32) -> DVec3 {
    let (y_sin, y_cos) = minecraft_sin_cos(f64::from(-yaw));
    let (x_sin, x_cos) = minecraft_sin_cos(f64::from(pitch));
    let (y_sin, y_cos, x_sin, x_cos) = (y_sin as f32, y_cos as f32, x_sin as f32, x_cos as f32);
    DVec3::new(f64::from(y_sin * x_cos), f64::from(-x_sin), f64::from(y_cos * x_cos))
}

/// `Vec3.normalize` (the zero vector below 1e-5 as a float).
pub fn normalize(v: DVec3) -> DVec3 {
    let length = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    if length < f64::from(1.0e-5_f32) {
        DVec3::ZERO
    } else {
        DVec3::new(v.x / length, v.y / length, v.z / length)
    }
}

/// `LivingEntity.isLookingAtMe(player, 0.025, true, false, gazeY)` for a
/// body at `me`: the player's view points within a cone that narrows with
/// distance at the gaze height above `me`, which it sees with nothing
/// solid in the way.
pub fn is_looking_at_me(world: &dyn World, player: DVec3, player_eye: f32, view: PlayerView, me: DVec3, gaze_y: f64) -> bool {
    let eye = player + DVec3::new(0.0, f64::from(player_eye), 0.0);
    looking_at(view, eye, me, gaze_y) && line_of_sight(world, eye, DVec3::new(me.x, gaze_y, me.z))
}

/// The cone test of `isLookingAtMe` alone.
pub fn looking_at(view: PlayerView, eye: DVec3, me: DVec3, gaze_y: f64) -> bool {
    let look = normalize(view_vector(view.pitch, view.head_yaw));
    let dir = DVec3::new(me.x - eye.x, gaze_y - eye.y, me.z - eye.z);
    let distance = (dir.x * dir.x + dir.y * dir.y + dir.z * dir.z).sqrt();
    let dir = normalize(dir);
    let dot = look.x * dir.x + look.y * dir.y + look.z * dir.z;
    dot > 1.0 - 0.025 / distance
}

/// `Enderman.teleport()`: a random spot up to 32 blocks away and 32 up or
/// down (two doubles and an int from its random).
pub fn teleport(body: &mut Body, random: &mut LegacyRandom, world: &dyn World) -> bool {
    let x = body.position.x + (random.next_double() - 0.5) * 64.0;
    let y = body.position.y + f64::from(random.next_int(64) as i32 - 32);
    let z = body.position.z + (random.next_double() - 0.5) * 64.0;
    teleport_to(body, world, x, y, z)
}

/// `Enderman.teleportTowards`: sixteen blocks nearer the target along the
/// line from its eyes to the enderman's middle, give or take four.
pub fn teleport_towards(body: &mut Body, random: &mut LegacyRandom, world: &dyn World, target: DVec3, target_eye_y: f64) -> bool {
    let middle = body.position.y + f64::from(body.height) * 0.5;
    let dir = normalize(DVec3::new(body.position.x - target.x, middle - target_eye_y, body.position.z - target.z));
    let x = body.position.x + (random.next_double() - 0.5) * 8.0 - dir.x * 16.0;
    let y = body.position.y + f64::from(random.next_int(16) as i32 - 8) - dir.y * 16.0;
    let z = body.position.z + (random.next_double() - 0.5) * 8.0 - dir.z * 16.0;
    teleport_to(body, world, x, y, z)
}

/// `Enderman.teleport(x, y, z)` through `LivingEntity.randomTeleport`:
/// down from the spot to the first block entities can teleport to
/// (`#entities_can_teleport_to`), the feet a block lower for each block
/// passed; there, unless that block is one endermen shun
/// (`#enderman_does_not_teleport_to`), the body must fit without a
/// collision, a liquid or a shunned block, over no water
/// (`canRandomlyTeleportTo`). Unloaded blocks read as air, so no ground is
/// found there (`hasChunkAt`). Returns whether it moved.
pub fn teleport_to(body: &mut Body, world: &dyn World, x: f64, mut y: f64, z: f64) -> bool {
    let (bx, bz) = (x.floor() as i32, z.floor() as i32);
    let mut by = y.floor() as i32;
    while by > world.min_y() {
        by -= 1;
        if world.block_in_tag((bx, by, bz), "minecraft:entities_can_teleport_to") {
            return fits(body, world, (bx, by, bz), x, y, z);
        }
        y -= 1.0;
    }
    false
}

/// `LivingEntity.checkPositionAndTeleport` for an enderman.
fn fits(body: &mut Body, world: &dyn World, ground: (i32, i32, i32), x: f64, y: f64, z: f64) -> bool {
    const SHUNNED: &str = "minecraft:enderman_does_not_teleport_to";
    if world.block_in_tag(ground, SHUNNED) {
        return false;
    }
    // `EntityDimensions.makeBoundingBox`: the half width as a float.
    let half = f64::from(body.width / 2.0);
    let min = DVec3::new(x - half, y, z - half);
    let max = DVec3::new(x + half, y + f64::from(body.height), z + half);
    if !no_block_collision(world, min, max) {
        return false;
    }
    // `containsAnyLiquid` and `findBlocksIn(...).filterState(shunned)` over
    // the blocks the box spans.
    let (lo, hi) = (min.floor(), max.floor());
    for bx in lo.x as i32..=hi.x as i32 {
        for by in lo.y as i32..=hi.y as i32 {
            for bz in lo.z as i32..=hi.z as i32 {
                if world.block((bx, by, bz)).is_some_and(|b| holds_fluid(&b)) || world.block_in_tag((bx, by, bz), SHUNNED) {
                    return false;
                }
            }
        }
    }
    // `canRandomlyTeleportTo`: no water under the new feet.
    let below = (x.floor() as i32, y.floor() as i32 - 1, z.floor() as i32);
    if world.block(below).is_some_and(|b| holds_water(&b)) {
        return false;
    }
    body.position = DVec3::new(x, y, z);
    true
}

/// `FluidState.is(FluidTags.WATER)`.
fn holds_water(block: &minecraftoss_player::Block) -> bool {
    holds_fluid(block) && block.id != "minecraft:lava"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_looking_level_at_the_eyes_stares() {
        // Due south (+Z) and level: the view is (0, 0, 1), within a hair.
        let view = PlayerView { head_yaw: 0.0, pitch: 0.0, disguised: false };
        let v = view_vector(view.pitch, view.head_yaw);
        assert!(v.z > 0.9999 && v.x.abs() < 1e-3 && v.y.abs() < 1e-3, "{v:?}");
        let eye = DVec3::new(0.5, 1.62, 0.5);
        assert!(looking_at(view, eye, DVec3::new(0.5, 0.0, 8.5), 1.62));
        // A block to the side at eight blocks is outside the cone.
        assert!(!looking_at(view, eye, DVec3::new(1.5, 0.0, 8.5), 1.62));
    }
}
