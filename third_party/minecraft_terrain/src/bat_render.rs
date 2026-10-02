//! Bats: `BatModel.createBodyLayer` (26.3) posed by its keyframe
//! animations (`BatAnimation.BAT_FLYING` and `BAT_RESTING`, each from the
//! client tick its `AnimationState` started), turned with the body; a
//! resting bat also turns its head (`applyHeadRotation`).
use crate::{
    client_mobs::ClientMobs,
    cow_render::cube_tinted_pose,
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
};
use glam::{EulerRot, Quat, Vec3};
use minecraftoss_entities::world::BatEntity;

/// A part's placement: its rotation and origin (model pixels) in the
/// model's frame.
#[derive(Clone, Copy)]
struct Pose {
    rotation: Quat,
    origin: Vec3,
}

impl Pose {
    const ROOT: Pose = Pose { rotation: Quat::IDENTITY, origin: Vec3::ZERO };

    /// `ModelPart.translateAndRotate` of a child: its offset, then its
    /// rotation (Z, Y, X).
    fn child(self, offset: Vec3, rotation: Vec3) -> Pose {
        Pose {
            rotation: self.rotation * Quat::from_euler(EulerRot::ZYX, rotation.z, rotation.y, rotation.x),
            origin: self.origin + self.rotation * offset,
        }
    }
}

/// A keyframe channel: timestamps (seconds) and values, all linear.
type Channel = &'static [(f32, [f32; 3])];

/// The parts `BatAnimation` moves.
#[derive(Clone, Copy, PartialEq)]
enum Bone {
    Head,
    Body,
    Feet,
    RightWing,
    RightWingTip,
    LeftWing,
    LeftWingTip,
}

/// `(bone, position channel, rotation channel)`: positions in
/// `KeyframeAnimations.posVec` (y as authored; negated when applied),
/// rotations in degrees (`degreeVec`).
type Animation = &'static [(Bone, Channel, Channel)];

const RESTING: Animation = &[
    (Bone::Head, &[(0.0, [0.0, 0.5, 0.0])], &[(0.0, [180.0, 0.0, 0.0])]),
    (Bone::Body, &[(0.0, [0.0, 0.5, 0.0])], &[(0.0, [180.0, 0.0, 0.0])]),
    (Bone::Feet, &[], &[(0.0, [0.0, 0.0, 0.0])]),
    (Bone::RightWing, &[(0.0, [0.0, 0.0, 1.0])], &[(0.0, [0.0, -10.0, 0.0])]),
    (Bone::RightWingTip, &[], &[(0.0, [0.0, -120.0, 0.0])]),
    (Bone::LeftWing, &[(0.0, [0.0, 0.0, 1.0])], &[(0.0, [0.0, 10.0, 0.0])]),
    (Bone::LeftWingTip, &[], &[(0.0, [0.0, 120.0, 0.0])]),
];

const BOB: Channel = &[
    (0.0, [0.0, 0.0, 0.0]),
    (0.125, [0.0, 2.0, 0.0]),
    (0.25, [0.0, 1.0, 0.0]),
    (0.375, [0.0, 0.0, 0.0]),
    (0.4583, [0.0, -1.0, 0.0]),
    (0.5, [0.0, 0.0, 0.0]),
];

const FLYING: Animation = &[
    (Bone::Head, BOB, &[(0.0, [0.0, 0.0, 0.0]), (0.125, [20.0, 0.0, 0.0]), (0.5, [0.0, 0.0, 0.0])]),
    (Bone::Body, BOB, &[(0.0, [40.0, 0.0, 0.0]), (0.25, [52.5, 0.0, 0.0]), (0.5, [40.0, 0.0, 0.0])]),
    (Bone::Feet, &[], &[(0.0, [10.0, 0.0, 0.0]), (0.125, [-21.25, 0.0, 0.0]), (0.25, [-12.5, 0.0, 0.0]), (0.5, [10.0, 0.0, 0.0])]),
    (
        Bone::RightWing,
        &[],
        &[(0.0, [0.0, 85.0, 0.0]), (0.125, [0.0, -55.0, 0.0]), (0.25, [0.0, 50.0, 0.0]), (0.375, [0.0, 70.0, 0.0]), (0.5, [0.0, 85.0, 0.0])],
    ),
    (Bone::RightWingTip, &[], &[(0.0, [0.0, 10.5, 0.0]), (0.0417, [0.0, 65.5, 0.0]), (0.2083, [0.0, -135.0, 0.0]), (0.5, [0.0, 10.5, 0.0])]),
    (
        Bone::LeftWing,
        &[],
        &[(0.0, [0.0, -85.0, 0.0]), (0.125, [0.0, 55.0, 0.0]), (0.25, [0.0, -50.0, 0.0]), (0.375, [0.0, -70.0, 0.0]), (0.5, [0.0, -85.0, 0.0])],
    ),
    (Bone::LeftWingTip, &[], &[(0.0, [0.0, -10.5, 0.0]), (0.0417, [0.0, -65.5, 0.0]), (0.2083, [0.0, 135.0, 0.0]), (0.5, [0.0, -10.5, 0.0])]),
];

/// Both animations last half a second and loop.
const LENGTH: f32 = 0.5;

/// `KeyframeAnimation.Entry.apply` with `LINEAR` keyframes: the keyframe
/// pair around `t` (`Mth.binarySearch`) and JOML's unfused lerp between
/// their values, each first scaled per axis as its `Keyframe` was built
/// (`posVec` negates y; `degreeVec` turns degrees to radians).
fn sample(keys: Channel, t: f32, scale: Vec3) -> Vec3 {
    let (mut from, mut len) = (0usize, keys.len());
    while len > 0 {
        let half = len / 2;
        if t <= keys[from + half].0 {
            len = half;
        } else {
            from += half + 1;
            len -= half + 1;
        }
    }
    let prev = from.saturating_sub(1);
    let next = (prev + 1).min(keys.len() - 1);
    let alpha = if next != prev { ((t - keys[prev].0) / (keys[next].0 - keys[prev].0)).clamp(0.0, 1.0) } else { 0.0 };
    let (a, b) = (Vec3::from_array(keys[prev].1) * scale, Vec3::from_array(keys[next].1) * scale);
    (b - a) * alpha + a
}

/// `KeyframeAnimation.apply` for an animation started at `start`: each
/// bone's (position, rotation) offsets, positions in model pixels (y
/// negated as `posVec` does), rotations in radians.
fn offsets(animation: Animation, start: i32, age_in_ticks: f32) -> impl Iterator<Item = (Bone, Vec3, Vec3)> {
    // `AnimationState.getTimeInMillis`, then seconds, looped.
    let millis = ((age_in_ticks - start as f32) * 50.0) as i64;
    let t = (millis as f32 / 1000.0) % LENGTH;
    // `(float) (Math.PI / 180.0)`.
    let radians = Vec3::splat(std::f32::consts::PI / 180.0);
    animation.iter().map(move |&(bone, position, rotation)| {
        let p = if position.is_empty() { Vec3::ZERO } else { sample(position, t, Vec3::new(1.0, -1.0, 1.0)) };
        let r = if rotation.is_empty() { Vec3::ZERO } else { sample(rotation, t, radians) };
        (bone, p, r)
    })
}

/// A part's box and texture offset.
type Cube = ([f32; 3], [f32; 3], [f32; 2]);

pub fn append_bats<'a>(
    mesh: &mut ChunkMesh,
    bats: impl IntoIterator<Item = &'a BatEntity>,
    poses: &ClientMobs,
    atlas: &Atlas,
    light: &SkyLight,
    partial: f32,
) {
    let id = ResourceId::parse("minecraft:entity/bat/bat").unwrap();
    let region = atlas.entity_region(&id);
    // Each mob's first vertex and overlay (`getOverlayCoords`).
    let mut marks = Vec::new();
    for entity in bats {
        let Some(mob) = poses.pose(entity.id, partial) else { continue };
        marks.push((mesh.vertices.len(), mob.overlay(0.0)));
        let sample_at = mob.light_block();
        let sky = light.get(sample_at) as f32;
        let block = light.get_block(sample_at) as f32;
        let rotation = mob.body_rotation(90.0);
        // `setupAnim`: the reset pose, a resting bat's head yaw, then the
        // running animation's offsets.
        let mut moved = [(Vec3::ZERO, Vec3::ZERO); 7];
        let index = |bone: Bone| bone as usize;
        if mob.resting {
            moved[index(Bone::Head)].1.y = mob.head_yaw * (std::f32::consts::PI / 180.0);
        }
        let running = if mob.resting { mob.rest_start.map(|s| (RESTING, s)) } else { mob.fly_start.map(|s| (FLYING, s)) };
        if let Some((animation, start)) = running {
            for (bone, p, r) in offsets(animation, start, mob.age_in_ticks) {
                moved[index(bone)].0 += p;
                moved[index(bone)].1 += r;
            }
        }
        let part = |parent: Pose, bone: Option<Bone>, pivot: [f32; 3]| {
            let (p, r) = bone.map_or((Vec3::ZERO, Vec3::ZERO), |b| moved[index(b)]);
            parent.child(Vec3::from_array(pivot) + p, r)
        };
        let body = part(Pose::ROOT, Some(Bone::Body), [0.0, 17.0, 0.0]);
        let head = part(Pose::ROOT, Some(Bone::Head), [0.0, 17.0, 0.0]);
        let right_wing = part(body, Some(Bone::RightWing), [-1.5, 0.0, 0.0]);
        let left_wing = part(body, Some(Bone::LeftWing), [1.5, 0.0, 0.0]);
        let parts: [(Pose, Cube); 9] = [
            (body, ([-1.5, 0.0, -1.0], [1.5, 5.0, 1.0], [0.0, 0.0])),
            (head, ([-2.0, -3.0, -1.0], [2.0, 0.0, 1.0], [0.0, 7.0])),
            (part(head, None, [-1.5, -2.0, 0.0]), ([-2.5, -4.0, 0.0], [0.5, 1.0, 0.0], [1.0, 15.0])),
            (part(head, None, [1.1, -3.0, 0.0]), ([-0.1, -3.0, 0.0], [2.9, 2.0, 0.0], [8.0, 15.0])),
            (right_wing, ([-2.0, -2.0, 0.0], [0.0, 5.0, 0.0], [12.0, 0.0])),
            (part(right_wing, Some(Bone::RightWingTip), [-2.0, 0.0, 0.0]), ([-6.0, -2.0, 0.0], [0.0, 6.0, 0.0], [16.0, 0.0])),
            (left_wing, ([0.0, -2.0, 0.0], [2.0, 5.0, 0.0], [12.0, 7.0])),
            (part(left_wing, Some(Bone::LeftWingTip), [2.0, 0.0, 0.0]), ([0.0, -2.0, 0.0], [6.0, 6.0, 0.0], [16.0, 8.0])),
            (part(body, Some(Bone::Feet), [0.0, 5.0, 0.0]), ([-1.5, 0.0, 0.0], [1.5, 2.0, 0.0], [16.0, 16.0])),
        ];
        for (pose, (from, to, uv)) in parts {
            cube_tinted_pose(mesh, mob.feet, rotation, 1.0, region, sky, block, from, to, uv, pose.origin.to_array(), pose.rotation, [1.0; 3], [32.0, 32.0], None);
        }
    }
    crate::cow_render::apply_overlays(mesh, &marks);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyframes_interpolate_linearly_between_their_pair() {
        let keys: Channel = &[(0.0, [0.0, 0.0, 0.0]), (0.125, [20.0, 0.0, 0.0]), (0.5, [0.0, 0.0, 0.0])];
        let one = Vec3::ONE;
        assert_eq!(sample(keys, 0.0, one).x, 0.0);
        assert_eq!(sample(keys, 0.0625, one).x, 10.0);
        assert_eq!(sample(keys, 0.125, one).x, 20.0);
        assert_eq!(sample(keys, 0.3125, one).x, 10.0);
        // A lone keyframe holds.
        assert_eq!(sample(&[(0.0, [180.0, 0.0, 0.0])], 0.3, one).x, 180.0);
    }
}
