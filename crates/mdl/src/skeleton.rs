//! Studio model skeletons: bones, hitboxes and pose evaluation.
//!
//! Bones are stored parent-before-child. Each carries its rest pose
//! (position + quaternion relative to the parent), the Euler rest rotation
//! and the scales animation values are multiplied by, and `pose_to_bone`:
//! the matrix taking model-space reference vertices into the bone's space
//! (the inverse bind matrix).

use crate::{invalid, R};
use glam::{EulerRot, Mat4, Quat, Vec3, Vec4};
use std::io;

pub const BONE_STRIDE: usize = 216;
pub const HITBOX_STRIDE: usize = 68;

#[derive(Clone, Debug)]
pub struct Bone {
    pub name: String,
    pub parent: i32,
    pub pos: Vec3,
    pub quat: Quat,
    /// Rest rotation as Source `RadianEuler` (x = roll, y = pitch, z = yaw).
    pub rot: Vec3,
    pub pos_scale: Vec3,
    pub rot_scale: Vec3,
    /// Model space → bone space in the reference pose.
    pub pose_to_bone: Mat4,
    pub flags: i32,
}

#[derive(Clone, Debug)]
pub struct Hitbox {
    pub bone: i32,
    /// Hit group (1 head, 2 chest, 3 stomach, 4/5 arms, 6/7 legs).
    pub group: i32,
    pub min: Vec3,
    pub max: Vec3,
    /// Capsule radius (CS:GO hitboxes); 0 for boxes.
    pub radius: f32,
    pub name: String,
}

/// Source `RadianEuler` → quaternion (rotation order Z · Y · X).
pub fn euler_to_quat(e: Vec3) -> Quat {
    Quat::from_euler(EulerRot::ZYX, e.z, e.y, e.x)
}

/// Reads a `matrix3x4_t` (row-major, 3 rows of 4) as an affine Mat4.
fn matrix3x4(r: &R, o: usize) -> io::Result<Mat4> {
    let mut m = [0.0f32; 12];
    for (i, v) in m.iter_mut().enumerate() {
        *v = r.f32(o + i * 4)?;
    }
    Ok(Mat4::from_cols(
        Vec4::new(m[0], m[4], m[8], 0.0),
        Vec4::new(m[1], m[5], m[9], 0.0),
        Vec4::new(m[2], m[6], m[10], 0.0),
        Vec4::new(m[3], m[7], m[11], 1.0),
    ))
}

pub(crate) fn read_bones(r: &R, count: usize, index: usize) -> io::Result<Vec<Bone>> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let o = index + i * BONE_STRIDE;
        let name_ofs = r.i32(o)?;
        let name = r.cstr((o as i64 + name_ofs as i64) as usize)?;
        let q = Quat::from_xyzw(r.f32(o + 44)?, r.f32(o + 48)?, r.f32(o + 52)?, r.f32(o + 56)?);
        let parent = r.i32(o + 4)?;
        if parent >= i as i32 {
            return Err(invalid(format!("bone {i} parent {parent} is not earlier in the list")));
        }
        out.push(Bone {
            name,
            parent,
            pos: r.vec3(o + 32)?,
            quat: q.normalize(),
            rot: r.vec3(o + 60)?,
            pos_scale: r.vec3(o + 72)?,
            rot_scale: r.vec3(o + 84)?,
            pose_to_bone: matrix3x4(r, o + 96)?,
            flags: r.i32(o + 160)?,
        });
    }
    Ok(out)
}

pub(crate) fn read_hitboxes(r: &R, sets: usize, set_index: usize) -> io::Result<Vec<Hitbox>> {
    if sets == 0 {
        return Ok(Vec::new());
    }
    // First hitbox set.
    let set = set_index;
    let count = r.i32(set + 4)?.max(0) as usize;
    let first = set + r.i32(set + 8)?.max(0) as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let o = first + i * HITBOX_STRIDE;
        let name_ofs = r.i32(o + 32)?;
        let name = if name_ofs > 0 { r.cstr(o + name_ofs as usize)? } else { String::new() };
        out.push(Hitbox {
            bone: r.i32(o)?,
            group: r.i32(o + 4)?,
            min: r.vec3(o + 8)?,
            max: r.vec3(o + 20)?,
            radius: r.f32(o + 48)?.max(0.0),
            name,
        });
    }
    Ok(out)
}

/// Bone-local transforms → bone-to-model matrices.
pub fn model_space(bones: &[Bone], local: &[(Vec3, Quat)]) -> Vec<Mat4> {
    let mut out: Vec<Mat4> = Vec::with_capacity(bones.len());
    for (i, b) in bones.iter().enumerate() {
        let (p, q) = local.get(i).copied().unwrap_or((b.pos, b.quat));
        let m = Mat4::from_rotation_translation(q, p);
        let world = if b.parent >= 0 { out[b.parent as usize] * m } else { m };
        out.push(world);
    }
    out
}

/// The rest pose as local transforms.
pub fn rest_pose(bones: &[Bone]) -> Vec<(Vec3, Quat)> {
    bones.iter().map(|b| (b.pos, b.quat)).collect()
}

/// Skins one vertex on the CPU (for tests and probes).
pub fn skin_vertex(v: &crate::Vertex, pose: &[Mat4], bones: &[Bone]) -> Vec3 {
    let mut out = Vec3::ZERO;
    let n = v.bone_count.clamp(1, 3) as usize;
    for k in 0..n {
        let b = v.bones[k] as usize;
        let (Some(m), Some(bone)) = (pose.get(b), bones.get(b)) else { continue };
        out += (*m * bone.pose_to_bone).transform_point3(v.position) * v.weights[k];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_matches_axis_rotations() {
        // Pure yaw (z) of 90° turns +X into +Y.
        let q = euler_to_quat(Vec3::new(0.0, 0.0, std::f32::consts::FRAC_PI_2));
        assert!((q * Vec3::X - Vec3::Y).length() < 1e-5);
        // Pure roll (x) of 90° turns +Y into +Z.
        let q = euler_to_quat(Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0));
        assert!((q * Vec3::Y - Vec3::Z).length() < 1e-5);
    }

    #[test]
    fn hierarchy_composes() {
        let bone = |parent, x| Bone {
            name: String::new(),
            parent,
            pos: Vec3::new(x, 0.0, 0.0),
            quat: Quat::IDENTITY,
            rot: Vec3::ZERO,
            pos_scale: Vec3::ONE,
            rot_scale: Vec3::ONE,
            pose_to_bone: Mat4::IDENTITY,
            flags: 0,
        };
        let bones = [bone(-1, 1.0), bone(0, 2.0)];
        let m = model_space(&bones, &rest_pose(&bones));
        assert!((m[1].transform_point3(Vec3::ZERO) - Vec3::new(3.0, 0.0, 0.0)).length() < 1e-6);
    }
}
