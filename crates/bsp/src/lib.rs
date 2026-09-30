//! Reader for Valve BSP maps (versions 19-21; CS:GO ships version 21).
//!
//! The file is a header of 64 lump descriptors followed by lump data.
//! Structure layouts follow the public Valve Developer Community
//! documentation of the format ("Source BSP File Format").
//!
//! Everything is decoded eagerly into plain vectors. Geometry stays in
//! Source units (inches) and Source axes (Z up); converting to a renderer's
//! frame is the caller's job.

pub mod disp;
pub mod lump;
pub mod staticprops;

use glam::{Vec3, Vec4};
use std::io;

pub use disp::DispMesh;
pub use staticprops::{StaticProp, StaticProps};

/// Lump indices used by this reader.
pub mod lumps {
    pub const ENTITIES: usize = 0;
    pub const PLANES: usize = 1;
    pub const TEXDATA: usize = 2;
    pub const VERTEXES: usize = 3;
    pub const NODES: usize = 5;
    pub const TEXINFO: usize = 6;
    pub const FACES: usize = 7;
    pub const LIGHTING: usize = 8;
    pub const LEAFS: usize = 10;
    pub const EDGES: usize = 12;
    pub const SURFEDGES: usize = 13;
    pub const MODELS: usize = 14;
    pub const LEAFFACES: usize = 16;
    pub const LEAFBRUSHES: usize = 17;
    pub const BRUSHES: usize = 18;
    pub const BRUSHSIDES: usize = 19;
    pub const DISPINFO: usize = 26;
    pub const DISP_VERTS: usize = 33;
    pub const GAME_LUMP: usize = 35;
    pub const PAKFILE: usize = 40;
    pub const CUBEMAPS: usize = 42;
    pub const TEXDATA_STRING_DATA: usize = 43;
    pub const TEXDATA_STRING_TABLE: usize = 44;
    pub const DISP_TRIS: usize = 48;
    pub const LIGHTING_HDR: usize = 53;
    pub const FACES_HDR: usize = 58;
}

/// `texinfo.flags` bits.
pub mod surf {
    pub const LIGHT: i32 = 0x0001;
    pub const SKY2D: i32 = 0x0002;
    pub const SKY: i32 = 0x0004;
    pub const WARP: i32 = 0x0008;
    pub const TRANS: i32 = 0x0010;
    pub const NOPORTAL: i32 = 0x0020;
    pub const TRIGGER: i32 = 0x0040;
    pub const NODRAW: i32 = 0x0080;
    pub const HINT: i32 = 0x0100;
    pub const SKIP: i32 = 0x0200;
    pub const NOLIGHT: i32 = 0x0400;
    pub const BUMPLIGHT: i32 = 0x0800;
    pub const NOSHADOWS: i32 = 0x1000;
    pub const NODECALS: i32 = 0x2000;
    pub const NOCHOP: i32 = 0x4000;
    pub const HITBOX: i32 = 0x8000;
}

/// Brush `contents` bits.
pub mod contents {
    pub const EMPTY: i32 = 0;
    pub const SOLID: i32 = 0x1;
    pub const WINDOW: i32 = 0x2;
    pub const AUX: i32 = 0x4;
    pub const GRATE: i32 = 0x8;
    pub const SLIME: i32 = 0x10;
    pub const WATER: i32 = 0x20;
    pub const BLOCKLOS: i32 = 0x40;
    pub const OPAQUE: i32 = 0x80;
    pub const MOVEABLE: i32 = 0x4000;
    pub const AREAPORTAL: i32 = 0x8000;
    pub const PLAYERCLIP: i32 = 0x10000;
    pub const MONSTERCLIP: i32 = 0x20000;
    pub const ORIGIN: i32 = 0x0100_0000;
    pub const MONSTER: i32 = 0x0200_0000;
    pub const DEBRIS: i32 = 0x0400_0000;
    pub const DETAIL: i32 = 0x0800_0000;
    pub const TRANSLUCENT: i32 = 0x1000_0000;
    pub const LADDER: i32 = 0x2000_0000;
    pub const HITBOX: i32 = 0x4000_0000;

    /// What blocks player movement.
    pub const MASK_PLAYERSOLID: i32 = SOLID | MOVEABLE | PLAYERCLIP | WINDOW | MONSTER | GRATE;
    /// What blocks bullets.
    pub const MASK_SHOT: i32 = SOLID | MOVEABLE | MONSTER | WINDOW | DEBRIS | HITBOX;
    /// Everything solid to anything.
    pub const MASK_SOLID: i32 = SOLID | MOVEABLE | WINDOW | MONSTER | GRATE;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub normal: Vec3,
    pub dist: f32,
    pub kind: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct TexData {
    pub reflectivity: Vec3,
    pub name_id: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub plane: i32,
    pub children: [i32; 2],
    pub mins: [i16; 3],
    pub maxs: [i16; 3],
    pub first_face: u16,
    pub num_faces: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct TexInfo {
    pub texture_vecs: [Vec4; 2],
    pub lightmap_vecs: [Vec4; 2],
    pub flags: i32,
    pub texdata: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct Face {
    pub plane: u16,
    pub side: u8,
    pub first_edge: i32,
    pub num_edges: i16,
    pub texinfo: i16,
    pub dispinfo: i16,
    pub styles: [u8; 4],
    pub light_offset: i32,
    pub lightmap_mins: [i32; 2],
    pub lightmap_size: [i32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct Leaf {
    pub contents: i32,
    pub cluster: i16,
    pub mins: [i16; 3],
    pub maxs: [i16; 3],
    pub first_leaf_face: u16,
    pub num_leaf_faces: u16,
    pub first_leaf_brush: u16,
    pub num_leaf_brushes: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct Model {
    pub mins: Vec3,
    pub maxs: Vec3,
    pub origin: Vec3,
    pub head_node: i32,
    pub first_face: i32,
    pub num_faces: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct Brush {
    pub first_side: i32,
    pub num_sides: i32,
    pub contents: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct BrushSide {
    pub plane: u16,
    pub texinfo: i16,
    pub dispinfo: i16,
    pub bevel: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct DispInfo {
    pub start_position: Vec3,
    pub disp_vert_start: i32,
    pub disp_tri_start: i32,
    pub power: i32,
    pub flags: u32,
    pub contents: i32,
    pub map_face: u16,
}

impl DispInfo {
    /// Vertices per side: `2^power + 1`.
    pub fn side_len(&self) -> usize {
        (1usize << self.power.clamp(0, 4)) + 1
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DispVert {
    pub vec: Vec3,
    pub dist: f32,
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Cubemap {
    pub origin: [i32; 3],
    pub size: i32,
}

/// A decoded map.
#[derive(Debug, Default)]
pub struct Bsp {
    pub version: i32,
    pub map_revision: i32,
    pub entities: Vec<kv::Block>,
    pub planes: Vec<Plane>,
    pub texdata: Vec<TexData>,
    /// Material names, indexed by `TexData::name_id`.
    pub texdata_strings: Vec<String>,
    pub vertices: Vec<Vec3>,
    pub nodes: Vec<Node>,
    pub texinfo: Vec<TexInfo>,
    /// Faces matching `lighting` (HDR set when the map has one).
    pub faces: Vec<Face>,
    /// Raw `ColorRGBExp32` luxels.
    pub lighting: Vec<u8>,
    pub lighting_is_hdr: bool,
    pub leafs: Vec<Leaf>,
    pub edges: Vec<[u16; 2]>,
    pub surfedges: Vec<i32>,
    pub models: Vec<Model>,
    pub leaf_faces: Vec<u16>,
    pub leaf_brushes: Vec<u16>,
    pub brushes: Vec<Brush>,
    pub brush_sides: Vec<BrushSide>,
    pub dispinfos: Vec<DispInfo>,
    pub disp_verts: Vec<DispVert>,
    pub disp_tris: Vec<u16>,
    pub cubemaps: Vec<Cubemap>,
    /// The embedded ZIP of map-specific files.
    pub pakfile: Vec<u8>,
    pub static_props: StaticProps,
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

impl Bsp {
    pub fn read_file(path: impl AsRef<std::path::Path>) -> io::Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::parse(&bytes)
    }

    pub fn parse(file: &[u8]) -> io::Result<Self> {
        let header = lump::Header::parse(file)?;
        if !(19..=21).contains(&header.version) {
            return Err(invalid(format!("unsupported BSP version {}", header.version)));
        }
        let get = |i: usize| header.lump(file, i);

        let mut bsp = Bsp {
            version: header.version,
            map_revision: header.map_revision,
            ..Default::default()
        };

        let ent_bytes = get(lumps::ENTITIES)?;
        let ent_text = String::from_utf8_lossy(&ent_bytes);
        bsp.entities = kv::parse_entities(ent_text.trim_end_matches('\0'))
            .map_err(|e| invalid(format!("entity lump: {e}")))?;

        bsp.planes = lump::records(&get(lumps::PLANES)?, 20, |r| Plane {
            normal: r.vec3(0),
            dist: r.f32(12),
            kind: r.i32(16),
        });
        bsp.texdata = lump::records(&get(lumps::TEXDATA)?, 32, |r| TexData {
            reflectivity: r.vec3(0),
            name_id: r.i32(12),
            width: r.i32(16),
            height: r.i32(20),
        });
        let string_data = get(lumps::TEXDATA_STRING_DATA)?;
        let string_table = lump::records(&get(lumps::TEXDATA_STRING_TABLE)?, 4, |r| r.i32(0));
        bsp.texdata_strings = string_table
            .iter()
            .map(|&ofs| {
                let start = (ofs.max(0) as usize).min(string_data.len());
                let end = string_data[start..]
                    .iter()
                    .position(|&b| b == 0)
                    .map_or(string_data.len(), |p| start + p);
                String::from_utf8_lossy(&string_data[start..end]).into_owned()
            })
            .collect();
        bsp.vertices = lump::records(&get(lumps::VERTEXES)?, 12, |r| r.vec3(0));
        bsp.nodes = lump::records(&get(lumps::NODES)?, 32, |r| Node {
            plane: r.i32(0),
            children: [r.i32(4), r.i32(8)],
            mins: [r.i16(12), r.i16(14), r.i16(16)],
            maxs: [r.i16(18), r.i16(20), r.i16(22)],
            first_face: r.u16(24),
            num_faces: r.u16(26),
        });
        bsp.texinfo = lump::records(&get(lumps::TEXINFO)?, 72, |r| TexInfo {
            texture_vecs: [r.vec4(0), r.vec4(16)],
            lightmap_vecs: [r.vec4(32), r.vec4(48)],
            flags: r.i32(64),
            texdata: r.i32(68),
        });

        // Prefer the HDR lighting set; fall back to LDR.
        let hdr_light = get(lumps::LIGHTING_HDR)?;
        let ldr_light = get(lumps::LIGHTING)?;
        let hdr_faces = get(lumps::FACES_HDR)?;
        let ldr_faces = get(lumps::FACES)?;
        let parse_faces = |bytes: &[u8]| {
            lump::records(bytes, 56, |r| Face {
                plane: r.u16(0),
                side: r.u8(2),
                first_edge: r.i32(4),
                num_edges: r.i16(8),
                texinfo: r.i16(10),
                dispinfo: r.i16(12),
                styles: [r.u8(16), r.u8(17), r.u8(18), r.u8(19)],
                light_offset: r.i32(20),
                lightmap_mins: [r.i32(28), r.i32(32)],
                lightmap_size: [r.i32(36), r.i32(40)],
            })
        };
        if !hdr_light.is_empty() {
            bsp.lighting = hdr_light;
            bsp.lighting_is_hdr = true;
            bsp.faces = if hdr_faces.is_empty() {
                parse_faces(&ldr_faces)
            } else {
                parse_faces(&hdr_faces)
            };
        } else {
            bsp.lighting = ldr_light;
            bsp.faces = parse_faces(&ldr_faces);
        }

        let leaf_version = header.lumps[lumps::LEAFS].version;
        let leaf_stride = if leaf_version == 0 && bsp.version <= 19 { 56 } else { 32 };
        bsp.leafs = lump::records(&get(lumps::LEAFS)?, leaf_stride, |r| Leaf {
            contents: r.i32(0),
            cluster: r.i16(4),
            mins: [r.i16(8), r.i16(10), r.i16(12)],
            maxs: [r.i16(14), r.i16(16), r.i16(18)],
            first_leaf_face: r.u16(20),
            num_leaf_faces: r.u16(22),
            first_leaf_brush: r.u16(24),
            num_leaf_brushes: r.u16(26),
        });
        bsp.edges = lump::records(&get(lumps::EDGES)?, 4, |r| [r.u16(0), r.u16(2)]);
        bsp.surfedges = lump::records(&get(lumps::SURFEDGES)?, 4, |r| r.i32(0));
        bsp.models = lump::records(&get(lumps::MODELS)?, 48, |r| Model {
            mins: r.vec3(0),
            maxs: r.vec3(12),
            origin: r.vec3(24),
            head_node: r.i32(36),
            first_face: r.i32(40),
            num_faces: r.i32(44),
        });
        bsp.leaf_faces = lump::records(&get(lumps::LEAFFACES)?, 2, |r| r.u16(0));
        bsp.leaf_brushes = lump::records(&get(lumps::LEAFBRUSHES)?, 2, |r| r.u16(0));
        bsp.brushes = lump::records(&get(lumps::BRUSHES)?, 12, |r| Brush {
            first_side: r.i32(0),
            num_sides: r.i32(4),
            contents: r.i32(8),
        });
        bsp.brush_sides = lump::records(&get(lumps::BRUSHSIDES)?, 8, |r| BrushSide {
            plane: r.u16(0),
            texinfo: r.i16(2),
            dispinfo: r.i16(4),
            bevel: r.u8(6) != 0,
        });
        bsp.dispinfos = lump::records(&get(lumps::DISPINFO)?, 176, |r| {
            let min_tess = r.i32(24);
            DispInfo {
                start_position: r.vec3(0),
                disp_vert_start: r.i32(12),
                disp_tri_start: r.i32(16),
                power: r.i32(20),
                // A negative minTess carries flag bits instead of a tessellation value.
                flags: if min_tess < 0 { min_tess as u32 } else { 0 },
                contents: r.i32(32),
                map_face: r.u16(36),
            }
        });
        bsp.disp_verts = lump::records(&get(lumps::DISP_VERTS)?, 20, |r| DispVert {
            vec: r.vec3(0),
            dist: r.f32(12),
            alpha: r.f32(16),
        });
        bsp.disp_tris = lump::records(&get(lumps::DISP_TRIS)?, 2, |r| r.u16(0));
        bsp.cubemaps = lump::records(&get(lumps::CUBEMAPS)?, 16, |r| Cubemap {
            origin: [r.i32(0), r.i32(4), r.i32(8)],
            size: r.i32(12),
        });
        bsp.pakfile = get(lumps::PAKFILE)?;
        bsp.static_props = staticprops::parse(file, &get(lumps::GAME_LUMP)?)?;
        Ok(bsp)
    }

    /// The world entity (`classname worldspawn`).
    pub fn worldspawn(&self) -> Option<&kv::Block> {
        self.entities
            .iter()
            .find(|e| e.str("classname") == Some("worldspawn"))
    }

    pub fn entities_of_class<'a>(&'a self, class: &'a str) -> impl Iterator<Item = &'a kv::Block> + 'a {
        self.entities
            .iter()
            .filter(move |e| e.str("classname").is_some_and(|c| c.eq_ignore_ascii_case(class)))
    }

    /// Material name of a texinfo, e.g. `DE_DUST/SANDWALL01`.
    pub fn texinfo_material(&self, texinfo: usize) -> Option<&str> {
        let ti = self.texinfo.get(texinfo)?;
        let td = self.texdata.get(ti.texdata as usize)?;
        self.texdata_strings.get(td.name_id as usize).map(String::as_str)
    }

    /// Polygon vertices of a face, in file winding order.
    pub fn face_vertices(&self, face: &Face) -> Vec<Vec3> {
        let mut out = Vec::with_capacity(face.num_edges.max(0) as usize);
        for i in 0..face.num_edges.max(0) as usize {
            let Some(&se) = self.surfedges.get(face.first_edge as usize + i) else {
                break;
            };
            let edge = self.edges[se.unsigned_abs() as usize];
            let v = if se >= 0 { edge[0] } else { edge[1] };
            out.push(self.vertices[v as usize]);
        }
        out
    }

    /// Luxel grid size (width, height) of a face's lightmap, or `None` if unlit.
    pub fn lightmap_dims(&self, face: &Face) -> Option<(usize, usize)> {
        if face.light_offset < 0 || face.styles[0] == 255 {
            return None;
        }
        let ti = self.texinfo.get(face.texinfo as usize)?;
        if ti.flags & surf::NOLIGHT != 0 {
            return None;
        }
        Some((
            face.lightmap_size[0] as usize + 1,
            face.lightmap_size[1] as usize + 1,
        ))
    }

    /// Linear RGB luxels for the face's first light style (the flat,
    /// non-bumped lightmap), row-major, `w * h` entries.
    pub fn face_lightmap(&self, face: &Face) -> Option<(usize, usize, Vec<[f32; 3]>)> {
        let (w, h) = self.lightmap_dims(face)?;
        let start = face.light_offset as usize;
        let count = w * h;
        let bytes = self.lighting.get(start..start + count * 4)?;
        let texels = bytes
            .chunks_exact(4)
            .map(|c| decode_rgbexp(c[0], c[1], c[2], c[3] as i8))
            .collect();
        Some((w, h, texels))
    }

    /// Builds the triangle mesh of a displacement face.
    pub fn displacement(&self, face_index: usize) -> Option<DispMesh> {
        disp::build(self, face_index)
    }

    /// Which leaf contains a point (Source axes).
    pub fn point_leaf(&self, point: Vec3) -> usize {
        let mut node = self.models.first().map_or(0, |m| m.head_node);
        while node >= 0 {
            let n = &self.nodes[node as usize];
            let p = &self.planes[n.plane as usize];
            let d = p.normal.dot(point) - p.dist;
            node = if d >= 0.0 { n.children[0] } else { n.children[1] };
        }
        (-1 - node) as usize
    }
}

/// `ColorRGBExp32` → linear float colour (0..1 is the LDR range).
pub fn decode_rgbexp(r: u8, g: u8, b: u8, exp: i8) -> [f32; 3] {
    let scale = 2f32.powi(exp as i32) / 255.0;
    [r as f32 * scale, g as f32 * scale, b as f32 * scale]
}

/// Parses a Source `"x y z"` string.
pub fn parse_origin(s: &str) -> Option<Vec3> {
    let v = kv::parse_vec(s);
    (v.len() >= 3).then(|| Vec3::new(v[0], v[1], v[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgbexp_decode() {
        assert_eq!(decode_rgbexp(255, 0, 0, 0), [1.0, 0.0, 0.0]);
        let c = decode_rgbexp(128, 128, 128, 1);
        assert!((c[0] - 256.0 / 255.0).abs() < 1e-6);
        let c = decode_rgbexp(64, 0, 0, -2);
        assert!((c[0] - 16.0 / 255.0).abs() < 1e-6);
    }
}
