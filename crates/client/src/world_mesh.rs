//! Map geometry: brush faces and displacements, batched into one mesh per
//! material, plus a lightmap atlas holding every face's baked lighting.

use crate::coords::to_bevy;
use crate::material::{flags, WorldMaterial, WorldParams};
use crate::textures::{f32_to_f16, TextureCache};
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bsp::{surf, Bsp};
use std::collections::HashMap;

/// Luxel border around each lightmap in the atlas (stops bilinear bleeding).
const PAD: usize = 1;

pub struct LightmapAtlas {
    pub width: usize,
    pub height: usize,
    /// RGBA half floats.
    pub texels: Vec<u16>,
    /// Face index → top-left of the face's luxels (inside the padding).
    pub rects: HashMap<usize, (usize, usize)>,
    /// Texel centre of an always-white spot, for faces without lightmaps.
    pub white: (f32, f32),
}

impl LightmapAtlas {
    pub fn build(bsp: &Bsp) -> Self {
        let mut items: Vec<(usize, usize, usize)> = bsp
            .faces
            .iter()
            .enumerate()
            .filter_map(|(i, f)| bsp.lightmap_dims(f).map(|(w, h)| (i, w, h)))
            .collect();
        items.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));

        let total: usize = items.iter().map(|(_, w, h)| (w + 2 * PAD) * (h + 2 * PAD)).sum();
        let mut width = 1024;
        while width * width < total * 2 && width < 8192 {
            width *= 2;
        }

        // Shelf packing, starting after a reserved 4x4 white block.
        let mut rects = HashMap::with_capacity(items.len());
        let (mut x, mut y, mut shelf_h) = (4usize, 0usize, 4usize);
        let mut placed = Vec::with_capacity(items.len());
        for &(face, w, h) in &items {
            let (pw, ph) = (w + 2 * PAD, h + 2 * PAD);
            if x + pw > width {
                x = 0;
                y += shelf_h;
                shelf_h = 0;
            }
            placed.push((face, x, y, w, h));
            rects.insert(face, (x + PAD, y + PAD));
            x += pw;
            shelf_h = shelf_h.max(ph);
        }
        let height = (y + shelf_h).next_power_of_two().max(4);

        let mut texels = vec![0u16; width * height * 4 * 4];
        for layer in 0..4 {
        let layer_offset = layer * width * height * 4;
        let one = f32_to_f16(1.0);
        for yy in 0..4 {
            for xx in 0..4 {
                let o = layer_offset + (yy * width + xx) * 4;
                texels[o..o + 4].copy_from_slice(&[one, one, one, one]);
            }
        }
        for &(face, px, py, w, h) in &placed {
            let Some((_, _, lux)) = bsp.face_lightmap_layer(&bsp.faces[face], layer).or_else(|| bsp.face_lightmap(&bsp.faces[face])) else { continue };
            for ty in 0..h + 2 * PAD {
                for tx in 0..w + 2 * PAD {
                    // Clamp into the face's luxels so the border repeats the edge.
                    let sx = tx.saturating_sub(PAD).min(w - 1);
                    let sy = ty.saturating_sub(PAD).min(h - 1);
                    let c = lux[sy * w + sx];
                    let o = layer_offset + ((py + ty) * width + px + tx) * 4;
                    texels[o] = f32_to_f16(c[0]);
                    texels[o + 1] = f32_to_f16(c[1]);
                    texels[o + 2] = f32_to_f16(c[2]);
                    texels[o + 3] = one;
                }
            }
        }
        }
        Self {
            width,
            height,
            texels,
            rects,
            white: (2.0, 2.0),
        }
    }

    /// Atlas UV for a luxel coordinate (relative to the face's lightmap mins).
    pub fn uv(&self, face: usize, s: f32, t: f32) -> [f32; 2] {
        match self.rects.get(&face) {
            Some(&(x, y)) => [
                (x as f32 + s + 0.5) / self.width as f32,
                (y as f32 + t + 0.5) / self.height as f32,
            ],
            None => [self.white.0 / self.width as f32, self.white.1 / self.height as f32],
        }
    }

    pub fn to_image(&self) -> Image {
        let bytes: Vec<u8> = self.texels.iter().flat_map(|h| h.to_le_bytes()).collect();
        let mut img = Image::new(
            Extent3d {
                width: self.width as u32,
                height: self.height as u32,
                depth_or_array_layers: 4,
            },
            TextureDimension::D2,
            bytes,
            TextureFormat::Rgba16Float,
            RenderAssetUsages::RENDER_WORLD,
        );
        img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..Default::default()
        });
        img
    }
}

#[derive(Default)]
struct MeshBuilder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uv0: Vec<[f32; 2]>,
    uv1: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl MeshBuilder {
    fn push(&mut self, pos: glam::Vec3, normal: glam::Vec3, uv0: [f32; 2], uv1: [f32; 2], alpha: f32) -> u32 {
        let i = self.positions.len() as u32;
        self.positions.push(to_bevy(pos).to_array());
        self.normals.push(to_bevy(normal).to_array());
        self.uv0.push(uv0);
        self.uv1.push(uv1);
        self.colors.push([1.0, 1.0, 1.0, alpha]);
        i
    }

    fn into_mesh(self) -> Mesh {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uv0);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, self.uv1);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        mesh.insert_indices(Indices::U32(self.indices));
        if let Err(e) = mesh.generate_tangents() { warn!("world tangents: {e}"); }
        mesh
    }
}

fn texture_uv(ti: &bsp::TexInfo, p: glam::Vec3, w: f32, h: f32) -> [f32; 2] {
    let tv = &ti.texture_vecs;
    [
        (tv[0].truncate().dot(p) + tv[0].w) / w,
        (tv[1].truncate().dot(p) + tv[1].w) / h,
    ]
}

const SKIP_SURF: i32 = surf::NODRAW | surf::SKY | surf::SKY2D | surf::TRIGGER | surf::HINT | surf::SKIP;

/// Brush models to draw: the world, plus brush entities with a `*N` model.
fn visible_models(bsp: &Bsp) -> Vec<(usize, glam::Vec3)> {
    let mut out = vec![(0usize, glam::Vec3::ZERO)];
    for ent in &bsp.entities {
        let class = ent.str("classname").unwrap_or("");
        if class.starts_with("trigger_") || class == "func_buyzone" || class == "func_bomb_target" {
            continue;
        }
        let Some(model) = ent.str("model").and_then(|m| m.strip_prefix('*')).and_then(|n| n.parse::<usize>().ok()) else {
            continue;
        };
        if model == 0 {
            continue;
        }
        // Brushes a game mode enables later (Retakes walls) start hidden.
        if ent.i64("startdisabled") == Some(1) {
            continue;
        }
        let origin = ent.str("origin").and_then(bsp::parse_origin).unwrap_or(glam::Vec3::ZERO);
        out.push((model, origin));
    }
    out
}

pub struct WorldBuild {
    pub meshes: Vec<(String, Mesh)>,
    pub triangles: usize,
}

/// Faces that belong to the 3D skybox area: faces listed by its leaves,
/// displacements whose surface lies in it, and brush-entity faces whose
/// model centre lies in it.
pub fn sky_faces(bsp: &Bsp, sky: &bsp::SkyCamera) -> std::collections::HashSet<usize> {
    let mut set = bsp.faces_in_area(sky.area);
    for (i, face) in bsp.faces.iter().enumerate() {
        if face.dispinfo < 0 {
            continue;
        }
        let Some(d) = bsp.displacement(i) else { continue };
        let center = d.positions.iter().copied().sum::<glam::Vec3>() / d.positions.len().max(1) as f32;
        let n = bsp.face_normal(face).unwrap_or(glam::Vec3::Z);
        if bsp.point_area(center + n * 4.0) == sky.area {
            set.insert(i);
        }
    }
    for (model_index, origin) in visible_models(bsp).into_iter().skip(1) {
        let Some(m) = bsp.models.get(model_index) else { continue };
        let center = (m.mins + m.maxs) * 0.5 + origin;
        if bsp.point_area(center) == sky.area {
            let first = m.first_face.max(0) as usize;
            set.extend(first..first + m.num_faces.max(0) as usize);
        }
    }
    set
}

/// Builds batched world meshes keyed by material name (lowercase), for the
/// faces `include` accepts.
pub fn build(bsp: &Bsp, atlas: &LightmapAtlas, include: &dyn Fn(usize) -> bool) -> WorldBuild {
    let mut groups: HashMap<(String, [i32; 3]), MeshBuilder> = HashMap::new();
    let mut triangles = 0usize;

    for (model_index, origin) in visible_models(bsp) {
        let Some(model) = bsp.models.get(model_index) else { continue };
        let first = model.first_face.max(0) as usize;
        let count = model.num_faces.max(0) as usize;
        for face_index in first..(first + count).min(bsp.faces.len()) {
            if !include(face_index) {
                continue;
            }
            let face = &bsp.faces[face_index];
            let Some(ti) = bsp.texinfo.get(face.texinfo.max(0) as usize) else { continue };
            if face.texinfo < 0 || ti.flags & SKIP_SURF != 0 {
                continue;
            }
            let Some(td) = bsp.texdata.get(ti.texdata.max(0) as usize) else { continue };
            let name = bsp.texinfo_material(face.texinfo as usize).unwrap_or("").to_ascii_lowercase();
            if name.starts_with("tools/") {
                continue;
            }
            let (tw, th) = (td.width.max(1) as f32, td.height.max(1) as f32);
            let verts_for_bounds = bsp.face_vertices(face);
            let center = verts_for_bounds.iter().copied().sum::<glam::Vec3>() / verts_for_bounds.len().max(1) as f32 + origin;
            let mb = groups.entry((name, spatial_cell(center))).or_default();

            if face.dispinfo >= 0 {
                let Some(d) = bsp.displacement(face_index) else { continue };
                // Smooth normals from the displaced surface.
                let mut normals = vec![glam::Vec3::ZERO; d.positions.len()];
                for tri in d.indices.chunks_exact(3) {
                    let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
                    let n = (d.positions[b] - d.positions[a]).cross(d.positions[c] - d.positions[a]);
                    normals[a] += n;
                    normals[b] += n;
                    normals[c] += n;
                }
                let base = mb.positions.len() as u32;
                for i in 0..d.positions.len() {
                    let lc = d.luxel_coords[i];
                    mb.push(
                        d.positions[i] + origin,
                        normals[i].normalize_or_zero(),
                        texture_uv(ti, d.base_positions[i], tw, th),
                        atlas.uv(face_index, lc.x, lc.y),
                        d.alphas[i],
                    );
                }
                mb.indices.extend(d.indices.iter().map(|i| base + i));
                triangles += d.indices.len() / 3;
                continue;
            }

            let verts = bsp.face_vertices(face);
            if verts.len() < 3 {
                continue;
            }
            let Some(normal) = bsp.face_normal(face) else { continue };
            let lv = &ti.lightmap_vecs;
            let base = mb.positions.len() as u32;
            for v in &verts {
                let s = lv[0].truncate().dot(*v) + lv[0].w - face.lightmap_mins[0] as f32;
                let t = lv[1].truncate().dot(*v) + lv[1].w - face.lightmap_mins[1] as f32;
                mb.push(*v + origin, normal, texture_uv(ti, *v, tw, th), atlas.uv(face_index, s, t), 0.0);
            }
            // Fan triangulation, wound counter-clockwise around the normal.
            // Newell's method: the compiler inserts collinear vertices to fix
            // T-junctions, so the first three points are not reliable.
            let ccw = {
                let mut n = glam::Vec3::ZERO;
                for i in 0..verts.len() {
                    let (a, b) = (verts[i], verts[(i + 1) % verts.len()]);
                    n += glam::Vec3::new(
                        (a.y - b.y) * (a.z + b.z),
                        (a.z - b.z) * (a.x + b.x),
                        (a.x - b.x) * (a.y + b.y),
                    );
                }
                n.dot(normal) >= 0.0
            };
            for i in 1..verts.len() as u32 - 1 {
                if ccw {
                    mb.indices.extend_from_slice(&[base, base + i, base + i + 1]);
                } else {
                    mb.indices.extend_from_slice(&[base, base + i + 1, base + i]);
                }
                triangles += 1;
            }
        }
    }

    let meshes = groups
        .into_iter()
        .filter(|(_, mb)| !mb.indices.is_empty())
        .map(|((name, _), mb)| (name, mb.into_mesh()))
        .collect();
    WorldBuild { meshes, triangles }
}

/// Creates the material for a world surface.
pub fn world_material(
    name: &str,
    fs: &vfs::Vfs,
    textures: &mut TextureCache,
    images: &mut Assets<Image>,
    shared: &SharedHandles,
    exposure: f32,
) -> WorldMaterial {
    let read = |p: &str| fs.read_string(p);
    let mat = vmt::Material::load(name, &read);
    material_from_vmt(mat.as_ref(), fs, textures, images, shared, exposure, true)
}

/// GPU resources every [`WorldMaterial`] binds.
#[derive(Clone)]
pub struct SharedHandles {
    pub lightmap: Handle<Image>,
}

/// Shared VMT → [`WorldMaterial`] translation for world and props.
pub fn material_from_vmt(
    mat: Option<&vmt::Material>,
    fs: &vfs::Vfs,
    textures: &mut TextureCache,
    images: &mut Assets<Image>,
    shared: &SharedHandles,
    exposure: f32,
    lightmapped: bool,
) -> WorldMaterial {
    let mut params = WorldParams {
        exposure,
        ..Default::default()
    };
    let mut out = WorldMaterial {
        params,
        base: textures.missing.clone(),
        base2: textures.white.clone(),
        lightmap: shared.lightmap.clone(),
        modulate: textures.black.clone(),
        normal: textures.flat_normal.clone(),
        normal2: textures.flat_normal.clone(),
        prop_light: textures.white.clone(),
        phong_map: textures.white.clone(),
        skin_surface:textures.white.clone(),
        skin_position:textures.white.clone(),
        alpha_mode: AlphaMode::Opaque,
        cull: true,
    };
    let Some(mat) = mat else {
        out.params.flags = if lightmapped { flags::LIGHTMAPPED } else { flags::VERTEX_LIT };
        return out;
    };
    let base_name = mat
        .base_texture()
        .or_else(|| mat.str("$hdrcompressedtexture"))
        .or_else(|| mat.str("$hdrbasetexture"));
    if let Some(info) = base_name.and_then(|n| textures.load(fs, images, n, true)) {
        out.base = info.handle;
    } else if base_name.is_none() {
        out.base = textures.white.clone();
    }
    let mut f = 0u32;
    if lightmapped && !mat.is_unlit() {
        f |= flags::LIGHTMAPPED;
    } else if !lightmapped {
        f |= flags::VERTEX_LIT;
    }
    if mat.is_unlit() {
        f |= flags::UNLIT;
    }
    if let Some(info) = mat.base_texture2().and_then(|n| textures.load(fs, images, n, true)) {
        out.base2 = info.handle;
        f |= flags::BLEND;
        if let Some(m) = mat
            .str("$blendmodulatetexture")
            .and_then(|n| textures.load(fs, images, n, false))
        {
            out.modulate = m.handle;
            f |= flags::BLEND_MODULATE;
        }
    }
    if let Some(info) = mat.str("$bumpmap").and_then(|n| textures.load(fs, images, n, false)) {
        out.normal = info.handle;
        f |= flags::NORMAL;
        if mat.flag("$ssbump") { f |= flags::SSBUMP; }
        if mat.flag("$ssbumpmathfix") { f |= flags::SSBUMP_MATHFIX; }
    }
    if let Some(info) = mat.str("$bumpmap2").and_then(|n| textures.load(fs, images, n, false)) {
        out.normal2 = info.handle;
        f |= flags::NORMAL2;
    }
    if !lightmapped && mat.flag("$phong") {
        params.phong=Vec4::new(mat.f32("$phongexponent").unwrap_or(20.0).max(1.0),mat.f32("$phongboost").unwrap_or(1.0),0.0,if mat.flag("$basemapalphaphongmask"){1.0}else{0.0});
        if let Some(info)=mat.str("$phongexponenttexture").and_then(|n|textures.load(fs,images,n,false)) {
            out.phong_map=info.handle;params.phong.z=1.0;
        }
        let tint=mat.color("$phongtint").unwrap_or([1.0;3]);
        params.phong_tint=Vec4::new(tint[0],tint[1],tint[2],if mat.flag("$phongalbedotint"){1.0}else{0.0});
        let fresnel=mat.color("$phongfresnelranges").unwrap_or([0.0,0.5,1.0]);
        params.fresnel=Vec4::new(fresnel[0],fresnel[1],fresnel[2],0.0);
        params.rim=Vec4::new(mat.f32("$rimlightexponent").unwrap_or(4.0).max(1.0),if mat.flag("$rimlight"){mat.f32("$rimlightboost").unwrap_or(1.0)}else{0.0},if mat.flag("$rimmask"){1.0}else{0.0},if mat.flag("$halflambert")&&!mat.flag("$phongdisablehalflambert"){1.0}else{0.0});
    }
    if mat.alpha_test() {
        f |= flags::ALPHA_TEST;
        params.alpha_ref = mat.alpha_test_ref();
        out.alpha_mode = AlphaMode::Mask(params.alpha_ref);
    } else if mat.translucent() {
        f |= flags::TRANSLUCENT;
        out.alpha_mode = if mat.additive() { AlphaMode::Add } else { AlphaMode::Blend };
    }
    let debug_no_cull = std::env::var("CSR_NO_CULL").ok();
    let debug_no_cull = match debug_no_cull.as_deref() {
        Some("world") => lightmapped,
        Some("props") => !lightmapped,
        Some(_) => true,
        None => false,
    };
    if mat.no_cull() || debug_no_cull {
        out.cull = false;
    }
    let tint = mat.color("$color").or_else(|| mat.color("$color2")).unwrap_or([1.0; 3]);
    params.color = Vec4::new(tint[0], tint[1], tint[2], mat.alpha());
    params.flags = f;
    out.params = params;
    out
}

/// Keep batches local so their actual bounds can be rejected by the frustum.
/// Faces/props remain intact; a large object is never clipped at a cell edge.
pub fn spatial_cell(p: glam::Vec3) -> [i32; 3] {
    (p / 768.0).floor().as_ivec3().to_array()
}
