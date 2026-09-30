//! Valve Material Type reader.
//!
//! A `.vmt` is KeyValues text with one root block named after the shader:
//!
//! ```text
//! "LightmappedGeneric" { "$basetexture" "de_dust/sandwall01" "$surfaceprop" "plaster" }
//! ```
//!
//! `"patch"` materials (used heavily by map pakfiles to bake cubemaps in)
//! include another material and insert or replace parameters. This crate
//! resolves patches through a caller-supplied reader and exposes the result
//! as a flat parameter table plus the handful of properties a renderer needs.

use kv::{Block, Value};

/// Resolved material.
#[derive(Clone, Debug, Default)]
pub struct Material {
    /// Shader name, lowercase (`lightmappedgeneric`, `worldvertextransition`, ...).
    pub shader: String,
    /// All parameters with lowercase keys (`$basetexture`, `%compilenodraw`, ...).
    pub params: Block,
}

/// Maximum patch include depth (guards against include cycles).
const MAX_DEPTH: usize = 8;

/// Normalises a material path: lowercase, forward slashes, no `materials/`
/// prefix, no `.vmt` suffix.
pub fn normalize_name(name: &str) -> String {
    let mut s = name.replace('\\', "/").to_ascii_lowercase();
    while s.starts_with('/') {
        s.remove(0);
    }
    if let Some(rest) = s.strip_prefix("materials/") {
        s = rest.to_string();
    }
    if let Some(rest) = s.strip_suffix(".vmt") {
        s = rest.to_string();
    }
    s
}

/// VFS path of a material name.
pub fn vmt_path(name: &str) -> String {
    format!("materials/{}.vmt", normalize_name(name))
}

/// VFS path of a texture name (`$basetexture` value).
pub fn vtf_path(name: &str) -> String {
    let mut s = name.replace('\\', "/").to_ascii_lowercase();
    if let Some(rest) = s.strip_prefix("materials/") {
        s = rest.to_string();
    }
    if let Some(rest) = s.strip_suffix(".vtf") {
        s = rest.to_string();
    }
    format!("materials/{s}.vtf")
}

impl Material {
    /// Loads `materials/<name>.vmt` via `read`, resolving patches.
    pub fn load(name: &str, read: &dyn Fn(&str) -> Option<String>) -> Option<Material> {
        Self::load_path(&vmt_path(name), read, 0)
    }

    fn load_path(path: &str, read: &dyn Fn(&str) -> Option<String>, depth: usize) -> Option<Material> {
        if depth > MAX_DEPTH {
            return None;
        }
        let text = read(path)?;
        Self::parse_with(&text, read, depth)
    }

    /// Parses VMT text; `read` resolves patch includes.
    pub fn parse_with(text: &str, read: &dyn Fn(&str) -> Option<String>, depth: usize) -> Option<Material> {
        let doc = kv::parse(text).ok()?;
        let (shader, body) = doc
            .entries
            .iter()
            .find_map(|(k, v)| v.as_block().map(|b| (k.to_ascii_lowercase(), b)))?;

        if shader == "patch" {
            let include = body.str("include")?;
            let mut base = Self::load_path(&include.replace('\\', "/").to_ascii_lowercase(), read, depth + 1)?;
            for section in ["insert", "replace"] {
                for patch in body.get_all(section).filter_map(Value::as_block) {
                    let mut flat = Block::new();
                    flatten_into(&mut flat, patch);
                    for (k, v) in flat.entries {
                        base.params.set(&k, v);
                    }
                }
            }
            return Some(base);
        }

        let mut params = Block::new();
        flatten_into(&mut params, body);
        Some(Material { shader, params })
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.params.str(key)
    }

    pub fn f32(&self, key: &str) -> Option<f32> {
        self.params.f32(key)
    }

    pub fn flag(&self, key: &str) -> bool {
        self.params.str(key).and_then(kv::parse_f32).is_some_and(|v| v != 0.0)
    }

    /// Colour parameter such as `$color` (`"[1 0.5 0]"` or `"{255 128 0}"`).
    pub fn color(&self, key: &str) -> Option<[f32; 3]> {
        let s = self.params.str(key)?;
        let v = kv::parse_vec(s);
        if v.len() < 3 {
            return None;
        }
        // Brace syntax is 0-255.
        let scale = if s.trim_start().starts_with('{') { 1.0 / 255.0 } else { 1.0 };
        Some([v[0] * scale, v[1] * scale, v[2] * scale])
    }

    pub fn base_texture(&self) -> Option<&str> {
        self.str("$basetexture")
    }

    pub fn base_texture2(&self) -> Option<&str> {
        self.str("$basetexture2")
    }

    pub fn translucent(&self) -> bool {
        self.flag("$translucent") || self.flag("$additive")
    }

    pub fn additive(&self) -> bool {
        self.flag("$additive")
    }

    pub fn alpha_test(&self) -> bool {
        self.flag("$alphatest")
    }

    pub fn alpha_test_ref(&self) -> f32 {
        self.f32("$alphatestreference").unwrap_or(0.5)
    }

    pub fn no_cull(&self) -> bool {
        self.flag("$nocull")
    }

    pub fn alpha(&self) -> f32 {
        self.f32("$alpha").unwrap_or(1.0)
    }

    pub fn surface_prop(&self) -> Option<&str> {
        self.str("$surfaceprop")
    }

    /// Materials the compiler strips from rendering (tool textures).
    pub fn is_nodraw(&self) -> bool {
        self.flag("%compilenodraw")
            || self.flag("%compileclip")
            || self.flag("%compileplayerclip")
            || self.flag("%compiletrigger")
            || self.flag("%compileskip")
            || self.flag("%compilehint")
            || self.flag("%compileorigin")
            || self.flag("%compileareaportal")
            || self.flag("%compileinvisible")
            || self.flag("%compilenpcclip")
            || self.flag("%compileblocklos")
            || self.shader == "occlusion"
    }

    pub fn is_sky(&self) -> bool {
        self.flag("%compilesky") || self.flag("%compile2dsky")
    }

    pub fn is_water(&self) -> bool {
        self.shader == "water" || self.flag("%compilewater")
    }

    /// True for shaders that do not take lightmaps (drawn fullbright).
    pub fn is_unlit(&self) -> bool {
        matches!(self.shader.as_str(), "unlitgeneric" | "unlittwotexture" | "sprite")
    }
}

/// Copies parameters into `out` with lowercase keys, merging render-path
/// sub-blocks that apply to a modern PC (`">=dx90"`, `"dx9"`...) and
/// `GPU>=n?`-style conditional keys.
fn flatten_into(out: &mut Block, body: &Block) {
    for (k, v) in &body.entries {
        let key = k.to_ascii_lowercase();
        match v {
            Value::Block(child) => {
                if applies(&key) {
                    flatten_into(out, child);
                } else if key.starts_with('$') || key.starts_with('%') {
                    // Parameter blocks such as `Proxies` inside a param: keep as-is.
                    out.set(&key, v.clone());
                } else if key == "proxies" {
                    out.set(&key, v.clone());
                }
            }
            Value::Str(_) => {
                if let Some((cond, param)) = key.split_once('?') {
                    if condition_holds(cond) {
                        out.set(param, v.clone());
                    }
                } else {
                    out.set(&key, v.clone());
                }
            }
        }
    }
}

/// Render-path sub-block names that are valid for a DX9+ PC.
fn applies(key: &str) -> bool {
    matches!(
        key,
        ">=dx90" | ">=dx90_20b" | ">dx80" | ">=dx80" | ">dx90" | "dx9" | "dx90" | ">=dx81" | ">dx81" | "srgb"
    ) || key.ends_with("_dx9")
        || key.ends_with("_hdr_dx9")
}

fn condition_holds(cond: &str) -> bool {
    let c = cond.replace(' ', "");
    match c.as_str() {
        "srgb" | ">=dx90" | ">dx80" | "gpu>=1" | "gpu>=2" | "gpu>1" | "gpu>=0" | "!lowqualitycsm" => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn reader(files: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |p: &str| files.get(p).map(|s| s.to_string())
    }

    #[test]
    fn simple_material() {
        let m = Material::parse_with(
            r#""LightmappedGeneric" { "$baseTexture" "DE_DUST/SandWall01" "$alphatest" "1" ">=dx90" { "$envmap" "env_cubemap" } "$color" "{255 0 0}" }"#,
            &|_| None,
            0,
        )
        .unwrap();
        assert_eq!(m.shader, "lightmappedgeneric");
        assert_eq!(m.base_texture(), Some("DE_DUST/SandWall01"));
        assert!(m.alpha_test());
        assert_eq!(m.str("$envmap"), Some("env_cubemap"));
        assert_eq!(m.color("$color"), Some([1.0, 0.0, 0.0]));
    }

    #[test]
    fn patch_includes_and_inserts() {
        let files = HashMap::from([(
            "materials/de_dust/wall.vmt",
            r#""LightmappedGeneric" { "$basetexture" "de_dust/wall" "$envmap" "env_cubemap" }"#,
        )]);
        let read = reader(files);
        let m = Material::parse_with(
            r#""patch" { "include" "materials/de_dust/wall.vmt" "insert" { "$envmap" "maps/de_x/c0_0_0" } }"#,
            &read,
            0,
        )
        .unwrap();
        assert_eq!(m.shader, "lightmappedgeneric");
        assert_eq!(m.base_texture(), Some("de_dust/wall"));
        assert_eq!(m.str("$envmap"), Some("maps/de_x/c0_0_0"));
    }

    #[test]
    fn tool_flags() {
        let m = Material::parse_with(r#""LightmappedGeneric" { "%compileclip" 1 }"#, &|_| None, 0).unwrap();
        assert!(m.is_nodraw());
    }

    #[test]
    fn paths() {
        assert_eq!(vmt_path("DE_DUST\\Wall"), "materials/de_dust/wall.vmt");
        assert_eq!(vtf_path("de_dust/wall.vtf"), "materials/de_dust/wall.vtf");
    }
}
