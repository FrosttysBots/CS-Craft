//! Transfer authored first-person decals through shared weapon texture UVs.
//! First-person placement is authored; world-model transforms are fitted from
//! shared UV landmarks because the lower-detail meshes use different skeletons.
use glam::Vec3;
pub fn world_decal(
    decal: &mdl::Model,
    view: &mdl::Model,
    world: &mdl::Model,
) -> Option<mdl::Model> {
    // Fit the common weapon coordinate frame using corresponding texture UVs.
    // At most 96 samples per model avoids a per-decal-vertex triangle search.
    let mut samples = vec![];
    for mesh in &view.meshes {
        let name = view.textures.get(view.skin_texture(0, mesh.material))?;
        let targets: Vec<_> = world
            .meshes
            .iter()
            .filter(|m| {
                world
                    .textures
                    .get(world.skin_texture(0, m.material))
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .flat_map(|m| m.vertices.iter())
            .collect();
        if targets.is_empty() {
            continue;
        }
        for v in mesh
            .vertices
            .iter()
            .step_by((mesh.vertices.len() / 96).max(1))
        {
            if let Some(w) = targets.iter().min_by(|a, b| {
                a.uv.distance_squared(v.uv)
                    .total_cmp(&b.uv.distance_squared(v.uv))
            }) {
                if w.uv.distance_squared(v.uv) < 0.00002 {
                    samples.push((v.position, w.position));
                }
            }
        }
    }
    if samples.len() < 4 {
        return None;
    }
    let center = samples.iter().map(|p| p.0).sum::<Vec3>() / samples.len() as f32;
    let mut ata = glam::Mat4::ZERO;
    let mut rhs = [glam::Vec4::ZERO; 3];
    for (a, b) in samples {
        let x = ((a - center) * 0.1).extend(1.);
        ata += glam::Mat4::from_cols(x * x.x, x * x.y, x * x.z, x * x.w);
        for j in 0..3 {
            rhs[j] += x * b[j];
        }
    }
    if ata.determinant().abs() < 1e-8 {
        return None;
    }
    let rows = rhs.map(|b| ata.inverse() * b);
    let transform = glam::Mat4::from_cols(rows[0], rows[1], rows[2], glam::Vec4::W).transpose()
        * glam::Mat4::from_scale(Vec3::splat(0.1))
        * glam::Mat4::from_translation(-center);
    if !transform.is_finite() {
        return None;
    }
    let normal_transform = transform.inverse().transpose();
    let mut out = world.clone();
    out.meshes = decal.meshes.clone();
    out.textures = decal.textures.clone();
    out.skins = decal.skins.clone();
    out.cd_textures = decal.cd_textures.clone();
    for mesh in &mut out.meshes {
        let center = mesh
            .vertices
            .iter()
            .map(|v| transform.transform_point3(v.position))
            .sum::<Vec3>()
            / mesh.vertices.len().max(1) as f32;
        let near = world
            .meshes
            .iter()
            .flat_map(|m| m.vertices.iter())
            .min_by(|a, b| {
                a.position
                    .distance_squared(center)
                    .total_cmp(&b.position.distance_squared(center))
            })?;
        for v in &mut mesh.vertices {
            v.position = transform.transform_point3(v.position);
            v.normal = normal_transform
                .transform_vector3(v.normal)
                .normalize_or_zero();
            v.position += v.normal * 0.02;
            v.tangent = None;
            v.bones = near.bones;
            v.weights = near.weights;
            v.bone_count = near.bone_count;
        }
    }
    Some(out)
}

/// Keep the team's original sleeves when replacing its default gloves.
pub fn sleeves(mut glove: mdl::Model, arms: &mdl::Model) -> mdl::Model {
    glove.meshes.retain(|m| {
        glove
            .textures
            .get(
                glove
                    .skins
                    .first()
                    .and_then(|s| s.get(m.material))
                    .copied()
                    .unwrap_or(m.material),
            )
            .is_some_and(|n| !n.contains("bare_arm"))
    });
    // Flatten the skin table before appending materials.
    for m in &mut glove.meshes {
        m.material = glove
            .skins
            .first()
            .and_then(|s| s.get(m.material))
            .copied()
            .unwrap_or(m.material);
    }
    glove.skins.clear();
    for m in &arms.meshes {
        let texture = arms.skin_texture(0, m.material);
        let name = &arms.textures[texture];
        if name.contains("glove") {
            continue;
        }
        let mut mesh = m.clone();
        mesh.material = glove.textures.len();
        // Preserve resolved material search paths.
        glove.textures.push(
            arms.material_candidates(texture)
                .first()
                .cloned()
                .unwrap_or_else(|| name.clone()),
        );
        for v in &mut mesh.vertices {
            let original = *v;
            let mut position = Vec3::ZERO;
            let mut normal = Vec3::ZERO;
            for i in 0..v.bone_count as usize {
                let b = &arms.bones[original.bones[i] as usize];
                let target = glove
                    .bones
                    .iter()
                    .position(|t| {
                        t.name == b.name || (b.name == "Bip01" && t.name == "v_weapon.Bip01")
                    })
                    .unwrap_or(0);
                let transform = glove.bones[target].pose_to_bone.inverse() * b.pose_to_bone;
                position += transform.transform_point3(original.position) * original.weights[i];
                normal += transform.transform_vector3(original.normal) * original.weights[i];
                v.bones[i] = target as u8;
            }
            v.position = position;
            v.normal = normal.normalize_or_zero();
            v.tangent = None;
        }
        glove.meshes.push(mesh);
    }
    glove
}
