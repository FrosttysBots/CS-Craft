//! Source ↔ Bevy coordinate conversion.
//!
//! Source: X forward, Y left, Z up (right-handed, inches).
//! Bevy:   X right,   Y up,   -Z forward (right-handed).
//! The mapping (x, y, z) → (x, z, -y) is a proper rotation (−90° about X),
//! so triangle winding is preserved. Units stay in inches.

use bevy::math::{Quat, Vec3};

/// Source position/direction → Bevy.
#[inline]
pub fn to_bevy(v: glam::Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y)
}

/// Bevy position/direction → Source.
#[inline]
pub fn to_source(v: Vec3) -> glam::Vec3 {
    glam::Vec3::new(v.x, -v.z, v.y)
}

/// Rotation taking Source-space vectors to Bevy-space vectors.
pub fn source_to_bevy_rotation() -> Quat {
    Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
}

/// Converts a rotation expressed in Source space to Bevy space.
pub fn rotation_to_bevy(source_rotation: glam::Mat3) -> Quat {
    let s = source_to_bevy_rotation();
    let r = Quat::from_mat3(&bevy::math::Mat3::from_cols_array(&source_rotation.to_cols_array()));
    s * r * s.inverse()
}

/// Source pitch/yaw (degrees) → Bevy look direction.
pub fn view_forward(pitch: f32, yaw: f32) -> Vec3 {
    let (f, _, _) = movement::angle_vectors(glam::Vec3::new(pitch, yaw, 0.0));
    to_bevy(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_matches_mapping() {
        let s = source_to_bevy_rotation();
        for v in [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z, glam::Vec3::new(1.0, 2.0, 3.0)] {
            let a = s * Vec3::new(v.x, v.y, v.z);
            let b = to_bevy(v);
            assert!((a - b).length() < 1e-5, "{v:?}: {a:?} vs {b:?}");
        }
        assert!((to_source(to_bevy(glam::Vec3::new(1.0, 2.0, 3.0))) - glam::Vec3::new(1.0, 2.0, 3.0)).length() < 1e-6);
    }
}
