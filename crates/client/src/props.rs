//! Static props: studio models placed by the map's `sprp` game lump.
//!
//! Static props never move, so every instance is transformed into world
//! space at load time and merged into one mesh per material (per sky/world
//! layer). Per-instance lighting (map ambient cube + sun visibility) is baked
//! into vertex colours. A few hundred draws replace thousands of entities.

use crate::coords::{rotation_to_bevy, to_bevy};
use crate::material::WorldMaterial;
use crate::textures::TextureCache;
use crate::world_mesh::{material_from_vmt, SharedHandles};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
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

/// Lighting for one prop instance, in Source axes.
#[derive(Clone, Copy, Debug)]
pub struct PropLight {
    /// +X, -X, +Y, -Y, +Z, -Z.
    pub cube: [glam::Vec3; 6],
    pub to_sun: glam::Vec3,
    /// Sun colour, zero when the prop is in shadow.
    pub sun: glam::Vec3,
}

impl PropLight {
    /// Irradiance for a Source-space normal.
    pub fn eval(&self, n: glam::Vec3) -> glam::Vec3 {
        let n2 = n * n;
        let pick = |pos: bool, a: usize| if pos { self.cube[a] } else { self.cube[a + 1] };
        let ambient = pick(n.x >= 0.0, 0) * n2.x + pick(n.y >= 0.0, 2) * n2.y + pick(n.z >= 0.0, 4) * n2.z;
        ambient + self.sun * n.dot(self.to_sun).max(0.0)
    }
}

/// Map ambient cube at the prop, plus the sun if a ray towards it reaches the sky.
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
        .unwrap_or([[0.3; 3]; 6])
        .map(glam::Vec3::from);
    match sun {
        Some(s) => {
            let start = center + s.to_sun * (radius + 4.0);
            let tr = collision.trace_ray(start, start + s.to_sun * 16384.0, contents::MASK_SOLID);
            let lit = !tr.hit() || collision.is_sky(tr.surface);
            PropLight {
                cube,
                to_sun: s.to_sun,
                sun: if lit { s.color } else { glam::Vec3::ZERO },
            }
        }
        None => PropLight {
            cube,
            to_sun: glam::Vec3::Z,
            sun: glam::Vec3::ZERO,
        },
    }
}

#[derive(Default)]
struct Batch {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl Batch {
    fn into_mesh(self) -> Mesh {
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        m.insert_indices(Indices::U32(self.indices));
        m
    }
}

/// Whether a studio mesh's triangles need flipping to wind counter-clockwise
/// around their vertex normals.
pub fn needs_flip(mesh: &mdl::Mesh) -> bool {
    let mut agree = 0i64;
    for tri in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        let va = &mesh.vertices[a];
        let n = (mesh.vertices[b].position - va.position).cross(mesh.vertices[c].position - va.position);
        let vn = va.normal + mesh.vertices[b].normal + mesh.vertices[c].normal;
        agree += if n.dot(vn) >= 0.0 { 1 } else { -1 };
    }
    agree < 0
}

pub struct PropStats {
    pub instances: usize,
    pub draws: usize,
    pub materials: usize,
    pub failed_models: usize,
    pub sky_draws: usize,
    pub triangles: usize,
}

#[allow(clippy::too_many_arguments)]
pub fn spawn(
    commands: &mut Commands,
    level: &level::Level,
    collision: &CollisionWorld,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WorldMaterial>,
    images: &mut Assets<Image>,
    textures: &mut TextureCache,
    shared: &SharedHandles,
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
        triangles: 0,
    };

    // Load every referenced model once, with per-mesh winding fixes.
    let mut models: HashMap<u16, Option<(mdl::Model, Vec<bool>)>> = HashMap::new();
    for prop in &bsp.static_props.props {
        models.entry(prop.model).or_insert_with(|| {
            let name = bsp.static_props.models.get(prop.model as usize)?;
            let base = name.trim_end_matches(".mdl");
            let m = fs.read(name)?;
            let v = fs.read(&format!("{base}.vvd"))?;
            let x = fs.read(&format!("{base}.dx90.vtx"))?;
            let model = mdl::Model::load(&m, &v, &x).ok()?;
            let flips = model.meshes.iter().map(needs_flip).collect();
            Some((model, flips))
        });
    }
    stats.failed_models = models.values().filter(|m| m.is_none()).count();

    // One material per resolved VMT, shared by every prop that uses it.
    let mut material_cache: HashMap<String, Handle<WorldMaterial>> = HashMap::new();
    let mut batches: HashMap<(AssetId<WorldMaterial>, bool), (Handle<WorldMaterial>, Batch)> = HashMap::new();

    for prop in &bsp.static_props.props {
        let Some(Some((model, flips))) = models.get(&prop.model) else { continue };
        let rot = mdl::angles_to_mat3(prop.angles);
        let scale = if prop.scale > 0.0 { prop.scale } else { 1.0 };
        let center = prop.origin + rot * ((model.hull_min + model.hull_max) * 0.5 * scale);
        let radius = (model.hull_max - model.hull_min).length() * 0.5 * scale;
        let light = prop_light(bsp, collision, sun, center, prop.origin, radius);

        let in_sky = sky.filter(|s| bsp.point_area(prop.origin + glam::Vec3::Z * 4.0) == s.area);
        let mut transform = Transform {
            translation: to_bevy(prop.origin),
            rotation: rotation_to_bevy(rot),
            scale: Vec3::splat(scale),
        };
        if let Some(s) = in_sky {
            transform = crate::sky::sky_transform(&s) * transform;
        }
        let affine = transform.compute_affine();

        for (mesh, &flip) in model.meshes.iter().zip(flips) {
            let tex = model.skin_texture(prop.skin.max(0) as usize, mesh.material);
            let candidates = model.material_candidates(tex);
            let key = candidates.first().cloned().unwrap_or_default();
            let material = material_cache
                .entry(key)
                .or_insert_with(|| {
                    let vmt = candidates.iter().find_map(|c| vmt::Material::load(c, &read));
                    stats.materials += 1;
                    materials.add(material_from_vmt(vmt.as_ref(), fs, textures, images, shared, exposure, false))
                })
                .clone();
            let (_, batch) = batches
                .entry((material.id(), in_sky.is_some()))
                .or_insert_with(|| (material.clone(), Batch::default()));
            let base = batch.positions.len() as u32;
            for v in &mesh.vertices {
                let n_source = (rot * v.normal).normalize_or_zero();
                let lit = light.eval(n_source);
                batch.positions.push(affine.transform_point3(to_bevy(v.position)).to_array());
                batch.normals.push(to_bevy(n_source).to_array());
                batch.uvs.push(v.uv.to_array());
                batch.colors.push([lit.x, lit.y, lit.z, 1.0]);
            }
            for tri in mesh.indices.chunks_exact(3) {
                if flip {
                    batch.indices.extend_from_slice(&[base + tri[0], base + tri[2], base + tri[1]]);
                } else {
                    batch.indices.extend_from_slice(&[base + tri[0], base + tri[1], base + tri[2]]);
                }
            }
            stats.triangles += mesh.indices.len() / 3;
        }
        stats.instances += 1;
    }

    for ((_, in_sky), (material, batch)) in batches {
        if batch.indices.is_empty() {
            continue;
        }
        let mut e = commands.spawn((
            Mesh3d(meshes.add(batch.into_mesh())),
            MeshMaterial3d(material),
            Transform::IDENTITY,
        ));
        if in_sky {
            e.insert(crate::sky::sky_layer());
            stats.sky_draws += 1;
        }
        stats.draws += 1;
    }
    stats
}
