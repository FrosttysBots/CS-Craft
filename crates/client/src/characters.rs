//! Other players, drawn with the game's own character models.
//!
//! Models are shown in their reference pose until skeletal animation is
//! implemented; position, facing and crouch height follow the simulation.
//!
//! CS:GO character meshes are authored Y-up facing +Z (the skeleton's root
//! bone stands them up at runtime), so their vertices are rotated into
//! Source's Z-up, +X-forward frame: (x, y, z) → (z, x, y).

use crate::coords::{rotation_to_bevy, to_bevy};
use crate::material::WorldMaterial;
use crate::player::{LocalPlayer, SimWorld};
use crate::textures::TextureCache;
use crate::world_mesh::{material_from_vmt, SharedHandles};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use std::collections::HashMap;

pub const T_MODEL: &str = "models/player/custom_player/legacy/tm_phoenix.mdl";
pub const CT_MODEL: &str = "models/player/custom_player/legacy/ctm_sas.mdl";

/// Mesh parts (mesh, material) for each team's model.
#[derive(Resource, Default)]
pub struct CharacterModels {
    pub terrorist: Vec<(Handle<Mesh>, Handle<WorldMaterial>)>,
    pub counter_terrorist: Vec<(Handle<Mesh>, Handle<WorldMaterial>)>,
}

/// Marks the root entity drawing a simulated player.
#[derive(Component)]
pub struct CharacterOf(pub sim::PlayerId);

fn build_parts(
    path: &str,
    fs: &vfs::Vfs,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WorldMaterial>,
    images: &mut Assets<Image>,
    textures: &mut TextureCache,
    shared: &SharedHandles,
    exposure: f32,
) -> Vec<(Handle<Mesh>, Handle<WorldMaterial>)> {
    let base = path.trim_end_matches(".mdl");
    let (Some(m), Some(v), Some(x)) = (fs.read(path), fs.read(&format!("{base}.vvd")), fs.read(&format!("{base}.dx90.vtx"))) else {
        warn!("character model {path} not found");
        return Vec::new();
    };
    let model = match mdl::Model::load(&m, &v, &x) {
        Ok(m) => m,
        Err(e) => {
            warn!("character model {path}: {e}");
            return Vec::new();
        }
    };
    let read = |p: &str| fs.read_string(p);
    let upright = |v: glam::Vec3| glam::Vec3::new(v.z, v.x, v.y);
    model
        .meshes
        .iter()
        .map(|mesh| {
            let positions: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| to_bevy(upright(v.position)).to_array()).collect();
            let normals: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| to_bevy(upright(v.normal)).to_array()).collect();
            let uvs: Vec<[f32; 2]> = mesh.vertices.iter().map(|v| v.uv.to_array()).collect();
            // Simple fixed lighting: key from above, fill from the sides.
            let colors: Vec<[f32; 4]> = mesh
                .vertices
                .iter()
                .map(|v| {
                    // Model space is Y-up here (before `upright`).
                    let l = 0.55 + 0.45 * v.normal.y.max(0.0) + 0.15 * v.normal.z.abs();
                    [l, l, l * 0.97, 1.0]
                })
                .collect();
            let mut indices = mesh.indices.clone();
            let flip = {
                let mut agree = 0i64;
                for tri in mesh.indices.chunks_exact(3) {
                    let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
                    let va = &mesh.vertices[a];
                    let n = (mesh.vertices[b].position - va.position).cross(mesh.vertices[c].position - va.position);
                    agree += if n.dot(va.normal) >= 0.0 { 1 } else { -1 };
                }
                agree < 0
            };
            if flip {
                for t in indices.chunks_exact_mut(3) {
                    t.swap(1, 2);
                }
            }
            let mut me = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
            me.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            me.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
            me.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            me.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
            me.insert_indices(Indices::U32(indices));
            let tex = model.skin_texture(0, mesh.material);
            let vmt = model
                .material_candidates(tex)
                .into_iter()
                .find_map(|c| vmt::Material::load(&c, &read));
            let mat = material_from_vmt(vmt.as_ref(), fs, textures, images, shared, exposure, false);
            (meshes.add(me), materials.add(mat))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn load(
    fs: &vfs::Vfs,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WorldMaterial>,
    images: &mut Assets<Image>,
    textures: &mut TextureCache,
    shared: &SharedHandles,
    exposure: f32,
) -> CharacterModels {
    CharacterModels {
        terrorist: build_parts(T_MODEL, fs, meshes, materials, images, textures, shared, exposure),
        counter_terrorist: build_parts(CT_MODEL, fs, meshes, materials, images, textures, shared, exposure),
    }
}

/// Spawns, moves and hides character entities to match the simulation.
pub fn sync(
    mut commands: Commands,
    world: Res<SimWorld>,
    local: Res<LocalPlayer>,
    models: Res<CharacterModels>,
    mut roots: Query<(Entity, &CharacterOf, &mut Transform, &mut Visibility)>,
) {
    let mut seen: HashMap<sim::PlayerId, ()> = HashMap::new();
    for (entity, of, mut transform, mut vis) in &mut roots {
        match world.0.player(of.0) {
            Some(p) => {
                seen.insert(p.id, ());
                *vis = if p.alive && (p.id != local.id || local.noclip) { Visibility::Inherited } else { Visibility::Hidden };
                let rot = mdl::angles_to_mat3(glam::Vec3::new(0.0, p.view_angles.y, 0.0));
                transform.translation = to_bevy(p.movement.origin);
                transform.rotation = rotation_to_bevy(rot);
                // Squash while crouched until animations exist.
                let h = if p.movement.ducked { 54.0 / 72.0 } else { 1.0 };
                transform.scale = Vec3::new(1.0, h, 1.0);
            }
            None => commands.entity(entity).despawn(),
        }
    }
    for p in &world.0.players {
        if seen.contains_key(&p.id) {
            continue;
        }
        let parts = match p.team {
            sim::Team::Terrorist => &models.terrorist,
            _ => &models.counter_terrorist,
        };
        commands
            .spawn((CharacterOf(p.id), Transform::default(), Visibility::Hidden))
            .with_children(|c| {
                for (mesh, mat) in parts {
                    c.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone())));
                }
            });
    }
}
