//! Source studio models.
//!
//! A model is split over several files that share a base name:
//!
//! * `.mdl` — header, bones, textures, skins, body parts, meshes
//! * `.vvd` — vertex data (position, normal, UV, bone weights)
//! * `.dx90.vtx` — optimised index buffers per LOD
//! * `.phy` — collision convex hulls
//!
//! [`Model::load`] combines the first three into triangle meshes for LOD 0
//! grouped by material. Skeletal animation is not handled yet: vertices are
//! returned in the model's bind pose, which is what static props need.
//! Layouts follow public community documentation of the formats.

pub mod phy;

use glam::{Vec2, Vec3};
use std::io;

pub use phy::{ConvexHull, Phy};

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

#[derive(Clone, Copy)]
struct R<'a>(&'a [u8]);

impl<'a> R<'a> {
    fn i32(&self, o: usize) -> io::Result<i32> {
        self.0
            .get(o..o + 4)
            .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| invalid(format!("read past end at {o}")))
    }
    fn u16(&self, o: usize) -> io::Result<u16> {
        self.0
            .get(o..o + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .ok_or_else(|| invalid(format!("read past end at {o}")))
    }
    fn i16(&self, o: usize) -> io::Result<i16> {
        self.u16(o).map(|v| v as i16)
    }
    fn f32(&self, o: usize) -> io::Result<f32> {
        self.i32(o).map(|v| f32::from_bits(v as u32))
    }
    fn vec3(&self, o: usize) -> io::Result<Vec3> {
        Ok(Vec3::new(self.f32(o)?, self.f32(o + 4)?, self.f32(o + 8)?))
    }
    fn cstr(&self, o: usize) -> io::Result<String> {
        let rest = self.0.get(o..).ok_or_else(|| invalid("string offset past end"))?;
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
}

/// Triangles that share one material slot.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    /// Index into the model's skin table columns (`skin_ref`).
    pub material: usize,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub name: String,
    pub version: i32,
    pub hull_min: Vec3,
    pub hull_max: Vec3,
    pub view_min: Vec3,
    pub view_max: Vec3,
    pub flags: i32,
    pub contents: i32,
    pub surface_prop: String,
    /// Texture (material) names as stored in the model.
    pub textures: Vec<String>,
    /// Directories to search for those materials (`models/props/de_dust/`).
    pub cd_textures: Vec<String>,
    /// `skins[family][skin_ref]` → texture index.
    pub skins: Vec<Vec<usize>>,
    /// LOD 0 meshes of the default body group (sub-model 0 of each body part).
    pub meshes: Vec<Mesh>,
}

impl Model {
    /// Candidate material paths (without `materials/` and `.vmt`) for a
    /// texture index, in search order.
    pub fn material_candidates(&self, texture: usize) -> Vec<String> {
        let Some(name) = self.textures.get(texture) else {
            return Vec::new();
        };
        let name = name.replace('\\', "/").to_ascii_lowercase();
        let mut out: Vec<String> = self
            .cd_textures
            .iter()
            .map(|cd| {
                let cd = cd.replace('\\', "/").to_ascii_lowercase();
                let cd = cd.trim_start_matches('/');
                if cd.is_empty() || cd.ends_with('/') {
                    format!("{cd}{name}")
                } else {
                    format!("{cd}/{name}")
                }
            })
            .collect();
        out.push(name);
        out
    }

    /// Texture index used by a mesh material slot with the given skin family.
    pub fn skin_texture(&self, skin: usize, slot: usize) -> usize {
        self.skins
            .get(skin)
            .or_else(|| self.skins.first())
            .and_then(|f| f.get(slot).copied())
            .unwrap_or(slot)
    }

    /// Loads a model from its `.mdl`, `.vvd` and `.dx90.vtx` bytes.
    pub fn load(mdl: &[u8], vvd: &[u8], vtx: &[u8]) -> io::Result<Model> {
        let m = R(mdl);
        if mdl.get(0..4) != Some(b"IDST") {
            return Err(invalid("not an MDL (bad id)"));
        }
        let version = m.i32(4)?;
        let mut model = Model {
            name: m.cstr(12)?.chars().take(64).collect(),
            version,
            hull_min: m.vec3(104)?,
            hull_max: m.vec3(116)?,
            view_min: m.vec3(128)?,
            view_max: m.vec3(140)?,
            flags: m.i32(152)?,
            contents: m.i32(332)?,
            ..Default::default()
        };
        let surfaceprop_index = m.i32(308)?;
        if surfaceprop_index > 0 {
            model.surface_prop = m.cstr(surfaceprop_index as usize)?;
        }

        // Textures.
        let num_textures = m.i32(204)?.max(0) as usize;
        let texture_index = m.i32(208)?.max(0) as usize;
        for i in 0..num_textures {
            let t = texture_index + i * 64;
            let name_ofs = m.i32(t)?;
            model.textures.push(m.cstr((t as i64 + name_ofs as i64) as usize)?);
        }
        let num_cd = m.i32(212)?.max(0) as usize;
        let cd_index = m.i32(216)?.max(0) as usize;
        for i in 0..num_cd {
            let ofs = m.i32(cd_index + i * 4)?.max(0) as usize;
            model.cd_textures.push(m.cstr(ofs)?);
        }
        let num_skin_ref = m.i32(220)?.max(0) as usize;
        let num_skin_families = m.i32(224)?.max(0) as usize;
        let skin_index = m.i32(228)?.max(0) as usize;
        for f in 0..num_skin_families {
            let mut family = Vec::with_capacity(num_skin_ref);
            for r in 0..num_skin_ref {
                family.push(m.i16(skin_index + (f * num_skin_ref + r) * 2)?.max(0) as usize);
            }
            model.skins.push(family);
        }

        let verts = read_vvd(vvd)?;
        let vt = R(vtx);
        let vtx_version = vt.i32(0)?;
        if vtx_version != 7 {
            return Err(invalid(format!("unsupported VTX version {vtx_version}")));
        }
        let vtx_body_parts = vt.i32(28)?.max(0) as usize;
        let vtx_body_ofs = vt.i32(32)?.max(0) as usize;

        let num_body_parts = m.i32(232)?.max(0) as usize;
        let body_part_index = m.i32(236)?.max(0) as usize;
        let strip_group_stride = if version >= 49 { 33 } else { 25 };

        for bp in 0..num_body_parts.min(vtx_body_parts) {
            let bp_ofs = body_part_index + bp * 16;
            let num_models = m.i32(bp_ofs + 4)?;
            let model_index = m.i32(bp_ofs + 12)?;
            if num_models <= 0 {
                continue;
            }
            // Default body group: sub-model 0.
            let sm_ofs = (bp_ofs as i64 + model_index as i64) as usize;
            let num_meshes = m.i32(sm_ofs + 72)?.max(0) as usize;
            let mesh_index = m.i32(sm_ofs + 76)?;
            let model_vertex_start = (m.i32(sm_ofs + 84)?.max(0) / 48) as usize;

            // Matching VTX body part → model → LOD 0.
            let vbp = vtx_body_ofs + bp * 8;
            let v_model_ofs = vbp + vt.i32(vbp + 4)?.max(0) as usize;
            let v_lod_ofs = v_model_ofs + vt.i32(v_model_ofs + 4)?.max(0) as usize;
            let v_num_meshes = vt.i32(v_lod_ofs)?.max(0) as usize;
            let v_mesh_ofs = v_lod_ofs + vt.i32(v_lod_ofs + 4)?.max(0) as usize;

            for mi in 0..num_meshes.min(v_num_meshes) {
                let mesh_ofs = (sm_ofs as i64 + mesh_index as i64) as usize + mi * 116;
                let material = m.i32(mesh_ofs)?.max(0) as usize;
                let mesh_vertex_offset = m.i32(mesh_ofs + 12)?.max(0) as usize;

                let vm = v_mesh_ofs + mi * 9;
                let num_sg = vt.i32(vm)?.max(0) as usize;
                let sg_base = vm + vt.i32(vm + 4)?.max(0) as usize;
                let mut out = Mesh {
                    material,
                    ..Default::default()
                };
                for sg in 0..num_sg {
                    let sgo = sg_base + sg * strip_group_stride;
                    let num_verts = vt.i32(sgo)?.max(0) as usize;
                    let vert_ofs = sgo + vt.i32(sgo + 4)?.max(0) as usize;
                    let num_indices = vt.i32(sgo + 8)?.max(0) as usize;
                    let index_ofs = sgo + vt.i32(sgo + 12)?.max(0) as usize;
                    let base = out.vertices.len() as u32;
                    for v in 0..num_verts {
                        let orig = vt.u16(vert_ofs + v * 9 + 4)? as usize;
                        let idx = model_vertex_start + mesh_vertex_offset + orig;
                        out.vertices.push(verts.get(idx).copied().ok_or_else(|| {
                            invalid(format!("vertex {idx} out of range ({})", verts.len()))
                        })?);
                    }
                    for i in 0..num_indices / 3 {
                        let a = vt.u16(index_ofs + i * 6)? as u32;
                        let b = vt.u16(index_ofs + i * 6 + 2)? as u32;
                        let c = vt.u16(index_ofs + i * 6 + 4)? as u32;
                        if a as usize >= num_verts || b as usize >= num_verts || c as usize >= num_verts {
                            return Err(invalid("vtx index out of range"));
                        }
                        out.indices.extend_from_slice(&[base + a, base + b, base + c]);
                    }
                }
                if !out.indices.is_empty() {
                    model.meshes.push(out);
                }
            }
        }
        Ok(model)
    }
}

/// Reads LOD 0 vertices from a `.vvd`, applying the fixup table.
fn read_vvd(vvd: &[u8]) -> io::Result<Vec<Vertex>> {
    let r = R(vvd);
    if vvd.get(0..4) != Some(b"IDSV") {
        return Err(invalid("not a VVD (bad id)"));
    }
    let num_lod0 = r.i32(16)?.max(0) as usize;
    let num_fixups = r.i32(48)?.max(0) as usize;
    let fixup_start = r.i32(52)?.max(0) as usize;
    let data_start = r.i32(56)?.max(0) as usize;
    let read_vertex = |i: usize| -> io::Result<Vertex> {
        let o = data_start + i * 48;
        Ok(Vertex {
            position: r.vec3(o + 16)?,
            normal: r.vec3(o + 28)?,
            uv: Vec2::new(r.f32(o + 40)?, r.f32(o + 44)?),
        })
    };
    let mut out = Vec::with_capacity(num_lod0);
    if num_fixups == 0 {
        for i in 0..num_lod0 {
            out.push(read_vertex(i)?);
        }
    } else {
        for f in 0..num_fixups {
            let o = fixup_start + f * 12;
            let lod = r.i32(o)?;
            let src = r.i32(o + 4)?.max(0) as usize;
            let n = r.i32(o + 8)?.max(0) as usize;
            if lod >= 0 {
                for i in src..src + n {
                    out.push(read_vertex(i)?);
                }
            }
        }
    }
    Ok(out)
}

/// Converts Source `QAngle` (pitch, yaw, roll degrees) into a rotation
/// matrix acting on Source-space vectors.
pub fn angles_to_mat3(angles: Vec3) -> glam::Mat3 {
    let (pitch, yaw, roll) = (
        angles.x.to_radians(),
        angles.y.to_radians(),
        angles.z.to_radians(),
    );
    glam::Mat3::from_rotation_z(yaw) * glam::Mat3::from_rotation_y(pitch) * glam::Mat3::from_rotation_x(roll)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qangle_forward_vector() {
        // Yaw 90 turns +X (forward) into +Y (left in Source).
        let m = angles_to_mat3(Vec3::new(0.0, 90.0, 0.0));
        let f = m * Vec3::X;
        assert!((f - Vec3::Y).length() < 1e-5);
        // Positive pitch looks down.
        let m = angles_to_mat3(Vec3::new(45.0, 0.0, 0.0));
        let f = m * Vec3::X;
        assert!(f.z < 0.0);
    }
}
