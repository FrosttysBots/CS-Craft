//! Sky rendering.
//!
//! Source maps draw two skies behind the world:
//!
//! * the **3D skybox**: a miniature area of the map around `sky_camera`,
//!   drawn `scale`× larger (16 by default) as if the player stood at the
//!   sky camera. Distant buildings, terrain and cloud domes live here.
//! * the **2D skybox** (`skyname`), behind that.
//!
//! The 3D skybox geometry is transformed once into world space and drawn by
//! a second camera on [`SKY_LAYER`] that renders before the main camera, so
//! it is always behind the playable world.

use crate::coords::to_bevy;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

pub const SKY_LAYER: usize = 1;

pub fn sky_layer() -> RenderLayers {
    RenderLayers::layer(SKY_LAYER)
}

/// Maps 3D-skybox space into world space: `p' = (p - origin) * scale`.
pub fn sky_transform(sky: &bsp::SkyCamera) -> Transform {
    Transform {
        translation: -to_bevy(sky.origin) * sky.scale,
        rotation: Quat::IDENTITY,
        scale: Vec3::splat(sky.scale),
    }
}

/// Average colour of the 2D sky's top face, used as the clear colour.
pub fn sky_color(fs: &vfs::Vfs, bsp: &bsp::Bsp) -> Option<Color> {
    let name = bsp.worldspawn()?.str("skyname")?;
    let read = |p: &str| fs.read_string(p);
    let mat = vmt::Material::load(&format!("skybox/{name}up"), &read)?;
    let tex = mat.base_texture()?;
    let v = vtf::Vtf::parse(&fs.read(&vmt::vtf_path(tex))?).ok()?;
    // Smallest mip is a cheap average.
    let level = v.mip_count.saturating_sub(1);
    let px = v.rgba8(level, 0, 0)?;
    let n = (px.len() / 4).max(1) as f32;
    let mut sum = [0.0f32; 3];
    for c in px.chunks_exact(4) {
        for k in 0..3 {
            sum[k] += c[k] as f32;
        }
    }
    Some(Color::srgb(sum[0] / n / 255.0, sum[1] / n / 255.0, sum[2] / n / 255.0))
}
