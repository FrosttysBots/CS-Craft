//! GPU-skinned studio models.
//!
//! A [`SkinnedAsset`] is a model's meshes with joint indices/weights, its
//! skeleton and inverse bind poses. Instances spawn one joint entity per
//! bone; every frame [`apply_pose`] writes each joint's world transform:
//!
//! `joint = world_from_model · S · bone_to_model · S⁻¹`
//!
//! where `S` is the Source→Bevy axis change. Mesh vertices are stored as
//! `S · v` and inverse bind poses as `S · pose_to_bone · S⁻¹`, so skinning
//! reproduces `world_from_model · S · bone_to_model · pose_to_bone · v`.
//!
//! Bone-merging (viewmodel arms onto the gun, a rifle into a player's hand)
//! copies matching bones' model-space matrices by name from a parent pose.

use crate::coords::to_bevy;
use crate::material::WorldMaterial;
use crate::textures::TextureCache;
use crate::world_mesh::{material_from_vmt, SharedHandles};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::math::Mat4;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use mdl::skeleton::Bone;
use std::sync::Arc;

/// Source → Bevy axis change as a matrix: (x, y, z) → (x, z, -y).
pub fn axis_change() -> Mat4 {
    Mat4::from_cols(
        Vec4::new(1.0, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 0.0, -1.0, 0.0),
        Vec4::new(0.0, 1.0, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.0, 1.0),
    )
}

fn to_bevy_mat(m: glam::Mat4) -> Mat4 {
    Mat4::from_cols_array(&m.to_cols_array())
}

#[derive(Clone)]
pub struct SkinnedAsset {
    pub model: Arc<mdl::Model>,
    pub parts: Vec<(Handle<Mesh>, Handle<WorldMaterial>)>,
    pub inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
}

/// One spawned copy of a [`SkinnedAsset`].
#[derive(Clone, Debug)]
pub struct SkinnedInstance {
    pub joints: Vec<Entity>,
    pub meshes: Vec<Entity>,
}

#[allow(clippy::too_many_arguments)]
pub fn build_asset(
    model: Arc<mdl::Model>,
    fs: &vfs::Vfs,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WorldMaterial>,
    images: &mut Assets<Image>,
    textures: &mut TextureCache,
    shared: &SharedHandles,
    bindposes: &mut Assets<SkinnedMeshInverseBindposes>,
    exposure: f32,
    dynamic_lit: bool,
) -> SkinnedAsset {
    let read = |p: &str| fs.read_string(p);
    let joint_limit = model.bones.len().saturating_sub(1).min(255) as u16;
    let parts = model
        .meshes
        .iter()
        .map(|mesh| {
            let positions: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| to_bevy(v.position).to_array()).collect();
            let normals: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| to_bevy(v.normal).to_array()).collect();
            let uvs: Vec<[f32; 2]> = mesh.vertices.iter().map(|v| v.uv.to_array()).collect();
            let mut joints = Vec::with_capacity(mesh.vertices.len());
            let mut weights = Vec::with_capacity(mesh.vertices.len());
            for v in &mesh.vertices {
                let n = v.bone_count.clamp(1, 3) as usize;
                let mut j = [0u16; 4];
                let mut w = [0.0f32; 4];
                let mut sum = 0.0;
                for k in 0..n {
                    j[k] = (v.bones[k] as u16).min(joint_limit);
                    w[k] = v.weights[k].max(0.0);
                    sum += w[k];
                }
                if sum <= 1e-6 {
                    w = [1.0, 0.0, 0.0, 0.0];
                } else {
                    for x in w.iter_mut() {
                        *x /= sum;
                    }
                }
                joints.push(j);
                weights.push(w);
            }
            // Wind counter-clockwise around the vertex normals.
            let mut indices = mesh.indices.clone();
            if crate::props::needs_flip(mesh) {
                for t in indices.chunks_exact_mut(3) {
                    t.swap(1, 2);
                }
            }
            let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            m.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(joints));
            m.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
            m.insert_indices(Indices::U32(indices));
            let tex = model.skin_texture(0, mesh.material);
            let vmt = model
                .material_candidates(tex)
                .into_iter()
                .find_map(|c| vmt::Material::load(&c, &read));
            let mut mat = material_from_vmt(vmt.as_ref(), fs, textures, images, shared, exposure, false);
            if dynamic_lit {
                mat.params.flags = (mat.params.flags & !crate::material::flags::VERTEX_LIT) | crate::material::flags::DYNAMIC_LIT;
            }
            (meshes.add(m), materials.add(mat))
        })
        .collect();
    let s = axis_change();
    let s_inv = s.inverse();
    let inverse: Vec<Mat4> = model.bones.iter().map(|b| s * to_bevy_mat(b.pose_to_bone) * s_inv).collect();
    SkinnedAsset {
        model,
        parts,
        inverse_bindposes: bindposes.add(SkinnedMeshInverseBindposes::from(inverse)),
    }
}

/// Spawns joints and skinned meshes. `own_materials` clones each material
/// so the instance can be lit independently.
pub fn spawn(
    commands: &mut Commands,
    asset: &SkinnedAsset,
    layer: Option<RenderLayers>,
    materials: Option<&mut Assets<WorldMaterial>>,
) -> SkinnedInstance {
    let joints: Vec<Entity> = asset
        .model
        .bones
        .iter()
        .map(|_| commands.spawn((Transform::default(), Visibility::default())).id())
        .collect();
    let mut materials = materials;
    let meshes = asset
        .parts
        .iter()
        .map(|(mesh, mat)| {
            let mat = match materials.as_deref_mut() {
                Some(store) => {
                    let copy = store.get(mat).cloned();
                    copy.map_or_else(|| mat.clone(), |m| store.add(m))
                }
                None => mat.clone(),
            };
            let mut e = commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat),
                SkinnedMesh {
                    inverse_bindposes: asset.inverse_bindposes.clone(),
                    joints: joints.clone(),
                },
                Transform::default(),
                NoFrustumCulling,
            ));
            if let Some(l) = &layer {
                e.insert(l.clone());
            }
            e.id()
        })
        .collect();
    SkinnedInstance { joints, meshes }
}

/// Model-space bone matrices (Source space) for a child skeleton merged
/// onto a parent pose: bones with a parent-skeleton namesake copy its
/// matrix, the rest follow their own parent with their rest transform.
pub fn bone_merge(child: &[Bone], parent_bones: &[Bone], parent_pose: &[glam::Mat4]) -> Vec<glam::Mat4> {
    let mut out: Vec<glam::Mat4> = Vec::with_capacity(child.len());
    for b in child {
        let from_parent = parent_bones
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(&b.name))
            .and_then(|i| parent_pose.get(i).copied());
        let m = match from_parent {
            Some(m) => m,
            None => {
                let local = glam::Mat4::from_rotation_translation(b.quat, b.pos);
                if b.parent >= 0 { out[b.parent as usize] * local } else { local }
            }
        };
        out.push(m);
    }
    out
}

/// Writes joint transforms for a pose. `world_from_model` is in Bevy
/// space; `pose` holds Source-space bone-to-model matrices.
pub fn apply_pose(instance: &SkinnedInstance, world_from_model: Mat4, pose: &[glam::Mat4], transforms: &mut Query<&mut Transform>) {
    let s = axis_change();
    let s_inv = s.inverse();
    for (joint, m) in instance.joints.iter().zip(pose) {
        if let Ok(mut t) = transforms.get_mut(*joint) {
            let world = world_from_model * s * to_bevy_mat(*m) * s_inv;
            *t = Transform::from_matrix(world);
        }
    }
}
