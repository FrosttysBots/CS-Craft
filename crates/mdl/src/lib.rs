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

pub mod anim;
pub mod phy;
pub mod skeleton;

use glam::{Vec2, Vec3};
use std::io;

pub use phy::{ConvexHull, Phy};

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

#[derive(Clone, Copy)]
pub(crate) struct R<'a>(pub(crate) &'a [u8]);

impl<'a> R<'a> {
    fn u8(&self, o: usize) -> io::Result<u8> {
        self.0.get(o).copied().ok_or_else(|| invalid(format!("read past end at {o}")))
    }
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
    /// Authored tangent and handedness from the VVD tangent stream.
    pub tangent: Option<[f32; 4]>,
    /// Up to three influencing bones (model bone indices) and their weights.
    pub bones: [u8; 3],
    pub weights: [f32; 3],
    pub bone_count: u8,
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
    pub bones: Vec<skeleton::Bone>,
    /// Named points relative to animated bones (muzzles, ejectors, cameras).
    pub attachments: Vec<Attachment>,
    pub hitboxes: Vec<skeleton::Hitbox>,
    pub animations: Vec<anim::AnimDesc>,
    pub sequences: Vec<anim::Sequence>,
    /// Models whose animations this one includes (`$includemodel`).
    pub include_models: Vec<String>,
    /// Body parts: (name, sub-model names, chosen sub-model).
    pub body_parts: Vec<(String, Vec<String>, usize)>,
    /// Pose parameters (name, start, end) that sequences blend over.
    pub pose_params: Vec<(String, f32, f32)>,
    /// `.ani` file holding out-of-line animation blocks, if any.
    pub anim_block_file: String,
    pub anim_blocks: Vec<(u32, u32)>,
    /// The .mdl bytes (animation data is decoded from them on demand).
    pub raw: std::sync::Arc<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct Attachment {
    pub name: String,
    pub bone: usize,
    pub position: Vec3,
}

fn read_attachments(m: R<'_>, bone_count: usize) -> io::Result<Vec<Attachment>> {
    // MDL v48 attachment records observed in the installed assets: a relative
    // name, flags, bone index, row-major 3x4 local transform and reserved data.
    let count=m.i32(240)?.max(0) as usize;
    let base=m.i32(244)?.max(0) as usize;
    if count>m.0.len().saturating_sub(base)/92 {return Err(invalid("attachment table past end"));}
    let mut out=Vec::with_capacity(count);
    for i in 0..count {
        let at=base+i*92;let bone=m.i32(at+8)?;
        if bone<0 || bone as usize>=bone_count {return Err(invalid("attachment bone out of range"));}
        let name=(at as i64+m.i32(at)? as i64).try_into().map_err(|_|invalid("negative attachment name"))?;
        let position=Vec3::new(m.f32(at+24)?,m.f32(at+40)?,m.f32(at+56)?);
        if !position.is_finite(){return Err(invalid("non-finite attachment position"));}
        out.push(Attachment{name:m.cstr(name)?,bone:bone as usize,position});
    }
    Ok(out)
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
    /// Reads only the .mdl: skeleton, animations, sequences, textures (no
    /// geometry). Used for animation-only models such as `$includemodel`s.
    pub fn load_header(mdl: &[u8]) -> io::Result<Model> {
        let m = R(mdl);
        if mdl.get(0..4) != Some(b"IDST") {
            return Err(invalid("not an MDL (bad id)"));
        }
        let mut model = Model {
            name: m.cstr(12)?.chars().take(64).collect(),
            version: m.i32(4)?,
            hull_min: m.vec3(104)?,
            hull_max: m.vec3(116)?,
            flags: m.i32(152)?,
            ..Default::default()
        };
        model.read_animation_data(mdl)?;
        Ok(model)
    }

    fn read_animation_data(&mut self, mdl: &[u8]) -> io::Result<()> {
        let m = R(mdl);
        self.bones = skeleton::read_bones(&m, m.i32(156)?.max(0) as usize, m.i32(160)?.max(0) as usize)?;
        self.attachments = read_attachments(m, self.bones.len())?;
        self.hitboxes = skeleton::read_hitboxes(&m, m.i32(172)?.max(0) as usize, m.i32(176)?.max(0) as usize)
            .unwrap_or_default();
        self.animations = anim::read_anim_descs(&m, m.i32(180)?.max(0) as usize, m.i32(184)?.max(0) as usize)?;
        self.sequences = anim::read_sequences(&m, m.i32(188)?.max(0) as usize, m.i32(192)?.max(0) as usize, self.bones.len())?;
        let pp_count = m.i32(300)?.max(0) as usize;
        let pp_index = m.i32(304)?.max(0) as usize;
        for i in 0..pp_count {
            let o = pp_index + i * 20;
            let name = m.cstr((o as i64 + m.i32(o)? as i64) as usize)?;
            self.pose_params.push((name, m.f32(o + 8)?, m.f32(o + 12)?));
        }
        let includes = m.i32(336)?.max(0) as usize;
        let include_index = m.i32(340)?.max(0) as usize;
        for i in 0..includes {
            let o = include_index + i * 8;
            let ofs = m.i32(o + 4)?;
            self.include_models.push(m.cstr((o as i64 + ofs as i64) as usize)?);
        }
        let block_name = m.i32(348)?;
        let blocks = m.i32(352)?.max(0) as usize;
        let block_index = m.i32(356)?.max(0) as usize;
        if blocks > 0 && block_name > 0 {
            self.anim_block_file = m.cstr(block_name as usize)?;
            for b in 0..blocks {
                let o = block_index + b * 8;
                self.anim_blocks.push((m.i32(o)?.max(0) as u32, m.i32(o + 4)?.max(0) as u32));
            }
        }
        self.raw = std::sync::Arc::new(mdl.to_vec());
        Ok(())
    }

    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name.eq_ignore_ascii_case(name))
    }

    pub fn animation_index(&self, name: &str) -> Option<usize> {
        self.animations.iter().position(|a| a.name.eq_ignore_ascii_case(name))
    }

    pub fn sequence(&self, name: &str) -> Option<&anim::Sequence> {
        self.sequences.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Decodes animation `index` against this model's own skeleton.
    pub fn decode_animation(&self, index: usize, ani: Option<&[u8]>) -> io::Result<anim::Clip> {
        let desc = self
            .animations
            .get(index)
            .ok_or_else(|| invalid(format!("no animation {index}")))?;
        anim::decode_clip(&self.raw, ani, &self.anim_blocks, desc, &self.bones)
    }

    /// Loads a model with the default body groups (sub-model 0 of each part).
    pub fn load(mdl: &[u8], vvd: &[u8], vtx: &[u8]) -> io::Result<Model> {
        Self::load_with_body(mdl, vvd, vtx, &|_, _| 0)
    }

    /// Loads a model choosing a sub-model per body part:
    /// `choose_body(part_name, sub_model_names) -> index`.
    pub fn load_with_body(mdl: &[u8], vvd: &[u8], vtx: &[u8], choose_body: &dyn Fn(&str, &[String]) -> usize) -> io::Result<Model> {
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
            let part_name = m.cstr((bp_ofs as i64 + m.i32(bp_ofs)? as i64) as usize)?;
            let first_sm = (bp_ofs as i64 + model_index as i64) as usize;
            let sub_names: Vec<String> = (0..num_models as usize)
                .map(|k| m.cstr(first_sm + k * 148).unwrap_or_default())
                .collect();
            let choice = choose_body(&part_name, &sub_names).min(num_models as usize - 1);
            model.body_parts.push((part_name, sub_names, choice));
            let sm_ofs = first_sm + choice * 148;
            let num_meshes = m.i32(sm_ofs + 72)?.max(0) as usize;
            let mesh_index = m.i32(sm_ofs + 76)?;
            let model_vertex_start = (m.i32(sm_ofs + 84)?.max(0) / 48) as usize;

            // Matching VTX body part → model → LOD 0.
            let vbp = vtx_body_ofs + bp * 8;
            let v_model_ofs = vbp + vt.i32(vbp + 4)?.max(0) as usize + choice * 8;
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
        model.read_animation_data(mdl)?;
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
    let tangent_start = r.i32(60)?.max(0) as usize;
    let read_vertex = |i: usize| -> io::Result<Vertex> {
        let o = data_start + i * 48;
        Ok(Vertex {
            tangent: if tangent_start > 0 {
                let t = tangent_start + i * 16;
                let values = [r.f32(t)?, r.f32(t + 4)?, r.f32(t + 8)?, r.f32(t + 12)?];
                let xyz = Vec3::new(values[0], values[1], values[2]);
                (values.iter().all(|v| v.is_finite()) && xyz.length_squared() > 0.5 && values[3].abs() > 0.5).then_some(values)
            } else { None },
            position: r.vec3(o + 16)?,
            normal: r.vec3(o + 28)?,
            uv: Vec2::new(r.f32(o + 40)?, r.f32(o + 44)?),
            weights: [r.f32(o)?, r.f32(o + 4)?, r.f32(o + 8)?],
            bones: [vvd[o + 12], vvd[o + 13], vvd[o + 14]],
            bone_count: vvd[o + 15],
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
    fn attachment_local_translation_and_validation() {
        let mut data=vec![0u8;512];
        for (at,n) in [(240,1i32),(244,300),(300,92),(308,0)] {data[at..at+4].copy_from_slice(&n.to_le_bytes());}
        data[392..394].copy_from_slice(b"1\0");
        for (at,v) in [(324,2.0f32),(340,3.0),(356,9.0)] {data[at..at+4].copy_from_slice(&v.to_le_bytes());}
        let a=read_attachments(R(&data),1).unwrap();
        assert_eq!(a[0].name,"1");assert_eq!(a[0].position,Vec3::new(2.0,3.0,9.0));
        assert!(read_attachments(R(&data),0).is_err());
        data[240..244].copy_from_slice(&100i32.to_le_bytes());assert!(read_attachments(R(&data),1).is_err());
    }

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

#[cfg(test)] mod tangent_tests {
    use super::*;
    #[test] fn vvd_fixups_reorder_tangents_with_vertices() {
        let mut data = vec![0u8; 216];
        data[0..4].copy_from_slice(b"IDSV");
        for (offset, value) in [(16,2i32),(48,2),(52,192),(56,64),(60,160)] { data[offset..offset+4].copy_from_slice(&value.to_le_bytes()); }
        for i in 0..2 {
            data[64+i*48+16..64+i*48+20].copy_from_slice(&(i as f32).to_le_bytes());
            let t = [1.0f32,0.0,0.0,if i==0 {1.0} else {-1.0}];
            for (k,v) in t.iter().enumerate() { data[160+i*16+k*4..164+i*16+k*4].copy_from_slice(&v.to_le_bytes()); }
            data[192+i*12+4..192+i*12+8].copy_from_slice(&((1-i) as i32).to_le_bytes());
            data[192+i*12+8..192+i*12+12].copy_from_slice(&1i32.to_le_bytes());
        }
        let vertices = read_vvd(&data).unwrap();
        assert_eq!(vertices[0].position.x, 1.0);
        assert_eq!(vertices[0].tangent.unwrap()[3], -1.0);
        assert_eq!(vertices[1].tangent.unwrap()[3], 1.0);
        data[60..64].fill(0);
        assert!(read_vvd(&data).unwrap().iter().all(|v| v.tangent.is_none()));
    }
}
