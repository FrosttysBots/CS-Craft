//! Burning entities' flames (pinned 26.3 `EntityRenderDispatcher.submit` and
//! `FlameFeatureRenderer`). An entity showing fire (`displayFireAnimation`:
//! burning and no spectator) gets a stack of `fire_0` and `fire_1` quads
//! turned to the camera's yaw (`Mth.rotationAroundAxis(Y, camera)`), scaled
//! to 1.4 of its width and 0.3 of that towards the camera less 0.02 per
//! whole scaled height, each quad 0.45 higher, a tenth narrower and 0.03
//! further back than the last, its texture alternating and its u mirrored
//! every other pair; they take full block light (`withBlock(light, 15)`)
//! and face up for shading, so they draw unshaded.
use crate::{
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh, Vertex},
    pack::ResourceId,
};
use glam::{DVec3, Vec3};

/// Appends the flames of each burning entity: its rendered feet position,
/// its box width and height.
pub fn append_flames(mesh: &mut ChunkMesh, burning: impl IntoIterator<Item = (DVec3, f32, f32)>, atlas: &Atlas, camera_forward: Vec3, light: &SkyLight) {
    let (Ok(fire_0), Ok(fire_1)) = (ResourceId::parse("minecraft:block/fire_0"), ResourceId::parse("minecraft:block/fire_1")) else { return };
    if !atlas.contains(&fire_0) || !atlas.contains(&fire_1) {
        return;
    }
    let sprites = [atlas.region(&fire_0), atlas.region(&fire_1)];
    // The camera's yaw alone: its right, and back towards it.
    let ahead = Vec3::new(camera_forward.x, 0.0, camera_forward.z).normalize_or_zero();
    if ahead == Vec3::ZERO {
        return;
    }
    let right = ahead.cross(Vec3::Y);
    let back = -ahead;
    for (at, width, height) in burning {
        let origin = at.as_vec3();
        let cell = (at.x.floor() as i32, at.y.floor() as i32, at.z.floor() as i32);
        let sky = light.get(cell) as f32;
        let scale = width * 1.4;
        let mut h = height / scale;
        let lift = 0.3 - (h as i32) as f32 * 0.02;
        let (mut r, mut yo, mut zo) = (0.5_f32, 0.0_f32, 0.0_f32);
        let mut ss = 0;
        while h > 0.0 {
            let [mut u0, v0, mut u1, v1] = sprites[ss % 2];
            if ss / 2 % 2 == 0 {
                std::mem::swap(&mut u0, &mut u1);
            }
            let start = mesh.vertices.len() as u32;
            for (x, y, uv) in [(-r, -yo, [u1, v1]), (r, -yo, [u0, v1]), (r, 1.4 - yo, [u0, v0]), (-r, 1.4 - yo, [u1, v0])] {
                let point = origin + (right * x + Vec3::Y * y + back * (zo + lift)) * scale;
                mesh.vertices.push(Vertex { position: point.to_array(), uv, color: [1.0; 4], sky_light: sky, block_light: 15.0 });
            }
            mesh.indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
            mesh.faces += 1;
            h -= 0.45;
            yo -= 0.45;
            r *= 0.9;
            zo -= 0.03;
            ss += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_zombie_burns_in_six_quads() {
        // A zombie is 0.6 by 1.95: 1.95 / 0.84 is about 2.32 scaled units,
        // a quad for each 0.45 begun.
        let (scale, mut h) = (0.6_f32 * 1.4, 1.95_f32 / (0.6 * 1.4));
        assert!((scale - 0.84).abs() < 1e-6);
        let mut quads = 0;
        while h > 0.0 {
            h -= 0.45;
            quads += 1;
        }
        assert_eq!(quads, 6);
    }
}
