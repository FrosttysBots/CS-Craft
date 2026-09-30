//! Static props: studio models placed by the map's `sprp` game lump.

use crate::coords::{rotation_to_bevy, to_bevy};
use crate::material::{PropLight, WorldMaterial};
use crate::textures::TextureCache;
use crate::world_mesh::{material_from_vmt, SharedHandles};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, MeshTag, PrimitiveTopology};
use bevy::render::storage::ShaderBuffer;
use bevy::prelude::*;
use collision::{contents, CollisionWorld};
use std::collections::HashMap;

/// The map's sun, from `light_environment`.
#[derive(Clone, Copy, Debug)]
pub struct Sun {
    /// Direction towards the sun, Source axes.
    pub to_sun: glam::Vec3,
    pub color: glam::Vec3,
    pub ambient: glam::Vec3,
}

impl Sun {
    pub fn from_bsp(bsp: &bsp::Bsp) -> Option<Sun> {
        let ent = bsp.entities_of_class("light_environment").next()?;
        let angles = kv::parse_vec(ent.str("angles").unwrap_or("0 0 0"));
        let yaw = angles.get(1).copied().unwrap_or(0.0);
        let pitch = ent
            .str("pitch")
            .and_then(kv::parse_f32)
            .or_else(|| angles.first().copied())
            .unwrap_or(-45.0);
        // Pitch is the light's downward angle (negative = pointing down).
        let elevation = (-pitch).to_radians();
        let (sy, cy) = yaw.to_radians().sin_cos();
        let travel = glam::Vec3::new(elevation.cos() * cy, elevation.cos() * sy, -elevation.sin());
        let color = parse_light(ent.str("_lighthdr").filter(|s| !s.starts_with("-1")).or(ent.str("_light")));
        let ambient = parse_light(ent.str("_ambienthdr").filter(|s| !s.starts_with("-1")).or(ent.str("_ambient")));
        Some(Sun {
            to_sun: -travel.normalize_or_zero(),
            color,
            ambient,
        })
    }
}

/// `"r g b brightness"` → linear colour.
fn parse_light(s: Option<&str>) -> glam::Vec3 {
    let v = kv::parse_vec(s.unwrap_or("255 255 255 200"));
    let srgb = |c: f32| (c / 255.0).powf(2.2);
    let rgb = glam::Vec3::new(
        srgb(v.first().copied().unwrap_or(255.0)),
        srgb(v.get(1).copied().unwrap_or(255.0)),
        srgb(v.get(2).copied().unwrap_or(255.0)),
    );
    let brightness = v.get(3).copied().unwrap_or(200.0) / 255.0;
    rgb * brightness
}

struct LoadedModel {
    /// (mesh, shared material) per mesh.
    parts: Vec<(Handle<Mesh>, Handle<WorldMaterial>)>,
}

pub struct PropStats {
    pub instances: usize,
    pub draws: usize,
    pub materials: usize,
    pub failed_models: usize,
    pub sky_draws: usize,
}

fn build_mesh(mesh: &mdl::Mesh) -> Mesh {
    let positions: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| to_bevy(v.position).to_array()).collect();
    let normals: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| to_bevy(v.normal).to_array()).collect();
    let uvs: Vec<[f32; 2]> = mesh.vertices.iter().map(|v| v.uv.to_array()).collect();
    // Studio models wind clockwise; vote against the vertex normals to be sure.
    let mut agree = 0i64;
    for tri in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        let va = &mesh.vertices[a];
        let n = (mesh.vertices[b].position - va.position).cross(mesh.vertices[c].position - va.position);
        let vn = va.normal + mesh.vertices[b].normal + mesh.vertices[c].normal;
        agree += if n.dot(vn) >= 0.0 { 1 } else { -1 };
    }
    let mut indices = mesh.indices.clone();
    if agree < 0 {
        for tri in indices.chunks_exact_mut(3) {
            tri.swap(1, 2);
        }
    }
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    m.insert_indices(Indices::U32(indices));
    m
}

/// Lighting for a prop centred at `center`: the map's ambient cube there,
/// plus the sun if a ray towards it reaches the sky.
fn prop_light(
    bsp: &bsp::Bsp,
    collision: &CollisionWorld,
    sun: Option<Sun>,
    center: glam::Vec3,
    origin: glam::Vec3,
    radius: f32,
) -> PropLight {
    let cube = bsp
        .ambient_cube(center + glam::Vec3::Z * 4.0)
        .or_else(|| bsp.ambient_cube(origin + glam::Vec3::Z * 16.0))
        .unwrap_or([[0.3; 3]; 6]);
    // Source cube order (+X -X +Y -Y +Z -Z) → Bevy axes.
    let ambient_cube = [cube[0], cube[1], cube[4], cube[5], cube[3], cube[2]].map(|c| Vec4::new(c[0], c[1], c[2], 1.0));
    let (sun_dir, sun_color) = match sun {
        Some(s) => {
            let start = center + s.to_sun * (radius + 4.0);
            let tr = collision.trace_ray(start, start + s.to_sun * 16384.0, contents::MASK_SOLID);
            let lit = !tr.hit() || collision.is_sky(tr.surface);
            (
                to_bevy(s.to_sun).extend(if lit { 1.0 } else { 0.0 }),
                Vec4::new(s.color.x, s.color.y, s.color.z, 1.0),
            )
        }
        None => (Vec4::new(0.0, 1.0, 0.0, 0.0), Vec4::ZERO),
    };
    PropLight {
        ambient_cube,
        sun_dir,
        sun_color,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn(
    commands: &mut Commands,
    level: &level::Level,
    collision: &CollisionWorld,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WorldMaterial>,
    images: &mut Assets<Image>,
    buffers: &mut Assets<ShaderBuffer>,
    textures: &mut TextureCache,
    lightmap: &Handle<Image>,
    exposure: f32,
    sun: Option<Sun>,
    sky: Option<bsp::SkyCamera>,
) -> PropStats {
    let fs = &level.vfs;
    let bsp = &level.bsp;
    let read = |p: &str| fs.read_string(p);
    let mut stats = PropStats {
        instances: 0,
        draws: 0,
        materials: 0,
        failed_models: 0,
        sky_draws: 0,
    };

    // Pass 1: load every referenced model once.
    let mut raw_models: HashMap<u16, Option<mdl::Model>> = HashMap::new();
    for prop in &bsp.static_props.props {
        raw_models.entry(prop.model).or_insert_with(|| {
            let name = bsp.static_props.models.get(prop.model as usize)?;
            let base = name.trim_end_matches(".mdl");
            let m = fs.read(name)?;
            let v = fs.read(&format!("{base}.vvd"))?;
            let x = fs.read(&format!("{base}.dx90.vtx"))?;
            mdl::Model::load(&m, &v, &x).ok()
        });
    }

    // Pass 2: per-instance lighting into one storage buffer, indexed by MeshTag.
    let mut lights = Vec::with_capacity(bsp.static_props.props.len().max(1));
    for prop in &bsp.static_props.props {
        let light = match raw_models.get(&prop.model).and_then(|m| m.as_ref()) {
            Some(model) => {
                let rot = mdl::angles_to_mat3(prop.angles);
                let scale = if prop.scale > 0.0 { prop.scale } else { 1.0 };
                let center = prop.origin + rot * ((model.hull_min + model.hull_max) * 0.5 * scale);
                let radius = (model.hull_max - model.hull_min).length() * 0.5 * scale;
                prop_light(bsp, collision, sun, center, prop.origin, radius)
            }
            None => PropLight::default(),
        };
        lights.push(light);
    }
    if lights.is_empty() {
        lights.push(PropLight::default());
    }
    let shared = SharedHandles {
        lightmap: lightmap.clone(),
        prop_lights: buffers.add(ShaderBuffer::from(lights)),
    };

    // Pass 3: meshes and one shared material per (model, skin, mesh); spawn
    // instances tagged with their lighting index.
    let mut cache: HashMap<(u16, i32), Option<LoadedModel>> = HashMap::new();
    for (index, prop) in bsp.static_props.props.iter().enumerate() {
        let key = (prop.model, prop.skin);
        if !cache.contains_key(&key) {
            let loaded = raw_models.get(&prop.model).and_then(|m| m.as_ref()).map(|model| {
                let parts = model
                    .meshes
                    .iter()
                    .map(|mesh| {
                        let tex = model.skin_texture(prop.skin.max(0) as usize, mesh.material);
                        let vmt = model
                            .material_candidates(tex)
                            .into_iter()
                            .find_map(|c| vmt::Material::load(&c, &read));
                        let mat = material_from_vmt(vmt.as_ref(), fs, textures, images, &shared, exposure, false);
                        stats.materials += 1;
                        (meshes.add(build_mesh(mesh)), materials.add(mat))
                    })
                    .collect();
                LoadedModel { parts }
            });
            if loaded.is_none() {
                stats.failed_models += 1;
            }
            cache.insert(key, loaded);
        }
        let Some(loaded) = cache.get(&key).and_then(|m| m.as_ref()) else { continue };

        let rot = mdl::angles_to_mat3(prop.angles);
        let scale = if prop.scale > 0.0 { prop.scale } else { 1.0 };
        let mut transform = Transform {
            translation: to_bevy(prop.origin),
            rotation: rotation_to_bevy(rot),
            scale: Vec3::splat(scale),
        };
        let in_sky = sky.filter(|s| bsp.point_area(prop.origin + glam::Vec3::Z * 4.0) == s.area);
        if let Some(s) = in_sky {
            transform = crate::sky::sky_transform(&s) * transform;
        }
        for (mesh, material) in &loaded.parts {
            let mut e = commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                MeshTag(index as u32),
                transform,
            ));
            if in_sky.is_some() {
                e.insert(crate::sky::sky_layer());
                stats.sky_draws += 1;
            }
            stats.draws += 1;
        }
        stats.instances += 1;
    }
    stats
}
