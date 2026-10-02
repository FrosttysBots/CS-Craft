//! Conservative camera-space culling before building animated entity vertices.
use crate::terrain;
use glam_mc::{DVec3, Vec3};
pub struct EntityView {
    eye: DVec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
    tan_y: f32,
    tan_x: f32,
    distance: f32,
    cull: bool,
}
impl EntityView {
    pub fn new(camera: &terrain::sections::CullCamera, distance: f32, cull: bool) -> Self {
        let forward = camera.forward.normalize_or_zero();
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        // Expand the frustum slightly for rapid camera turns and interpolated poses.
        let tan_y = ((camera.fov_degrees + 8.).to_radians() * 0.5).tan();
        Self {
            eye: camera.position,
            forward,
            right,
            up,
            tan_y,
            tan_x: tan_y * camera.aspect,
            distance,
            cull,
        }
    }
    pub fn visible(&self, feet: [f64; 3], radius: f32) -> bool {
        let d = (DVec3::from_array(feet) + DVec3::Y - self.eye).as_vec3();
        if d.length_squared() > (self.distance + radius).powi(2) {
            return false;
        }
        if !self.cull {
            return true;
        }
        let depth = d.dot(self.forward);
        depth >= -radius
            && d.dot(self.right).abs()
                <= depth * self.tan_x + radius * (1. + self.tan_x * self.tan_x).sqrt()
            && d.dot(self.up).abs()
                <= depth * self.tan_y + radius * (1. + self.tan_y * self.tan_y).sqrt()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn view(cull: bool) -> EntityView {
        EntityView::new(
            &terrain::sections::CullCamera {
                position: DVec3::ZERO,
                forward: Vec3::Z,
                fov_degrees: 70.,
                aspect: 16. / 9.,
                yaw_degrees: 0.,
                pitch_degrees: 0.,
            },
            64.,
            cull,
        )
    }
    #[test]
    fn culls_behind_and_distant_but_keeps_intersecting_bounds() {
        let v = view(true);
        assert!(v.visible([0., 0., 20.], 4.));
        assert!(!v.visible([0., 0., -20.], 4.));
        assert!(!v.visible([0., 0., 90.], 4.));
        assert!(v.visible([0., 0., -1.], 4.));
        assert!(v.visible([30., 0., 20.], 4.));
        assert!(!v.visible([55., 0., 20.], 4.));
    }
    #[test]
    fn disabling_frustum_keeps_range_limit() {
        assert!(view(false).visible([0., 0., -20.], 4.));
        assert!(!view(false).visible([0., 0., -90.], 4.));
    }
}
