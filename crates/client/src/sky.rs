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

#[derive(Component)]
pub struct SkyFace;

/// Original six sky textures surround the camera behind the 3D skybox.
#[allow(clippy::too_many_arguments)]
pub fn spawn_2d(commands: &mut Commands, level: &level::Level, meshes: &mut Assets<Mesh>, materials: &mut Assets<crate::material::WorldMaterial>, images: &mut Assets<Image>, textures: &mut crate::textures::TextureCache, shared: &crate::world_mesh::SharedHandles) {
    use bevy::{asset::RenderAssetUsages, mesh::{Indices, PrimitiveTopology}};
    let Some(name) = level.bsp.worldspawn().and_then(|w| w.str("skyname")) else { return };
    // (outward normal, texture right, texture down), all in Source axes.
    let x = glam::Vec3::X; let y = glam::Vec3::Y; let z = glam::Vec3::Z;
    for (suffix, normal, right, down) in [("rt",x,-y,-z),("bk",y,x,-z),("lf",-x,y,-z),("ft",-y,-x,-z),("up",z,x,y),("dn",-z,x,-y)] {
        let Some(vmt) = vmt::Material::load(&format!("skybox/{name}{suffix}"), &|p| level.vfs.read_string(p)) else { continue };
        let mut material = crate::world_mesh::material_from_vmt(Some(&vmt), &level.vfs, textures, images, shared, 1.0, false);
        material.params.flags = crate::material::flags::UNLIT;
        material.cull = false;
        let corners = [normal-right-down, normal+right-down, normal+right+down, normal-right+down];
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, corners.map(|p| to_bevy(p * 180_000.0).to_array()).to_vec());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![to_bevy(-normal).to_array();4]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,1.0]]);
        mesh.insert_indices(Indices::U32(vec![0,1,2,0,2,3]));
        commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(materials.add(material)), Transform::default(), sky_layer(), SkyFace));
    }
}

pub fn follow_camera(local: Res<crate::player::LocalPlayer>, mut faces: Query<&mut Transform, With<SkyFace>>) {
    for mut face in &mut faces { face.translation = to_bevy(local.render_eye); }
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
