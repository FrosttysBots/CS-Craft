//! Baked geometry and sampled animation observed from the user's installed client.
use crate::{
    glam_mc::{Mat4, Quat, Vec3},
    terrain::{
        mesh::{ChunkMesh, Vertex},
        pack::ResourceId,
    },
    *,
};

#[derive(Deserialize)]
pub struct DragonModel {
    parts: Vec<Part>,
    clips: Vec<Vec<Vec<[f32; 9]>>>,
}
#[derive(Deserialize)]
struct Part {
    parent: i32,
    name: String,
    quads: Vec<[[f32; 5]; 4]>,
}
impl DragonModel {
    /// No geometry (the dragon is invisible and cannot be hit).
    pub fn empty() -> Self {
        Self { parts: Vec::new(), clips: Vec::new() }
    }

    pub fn load(root: &std::path::Path) -> Result<Self, String> {
        // A release ships the exported model with its assets, so players
        // need neither Java nor a Minecraft launcher install.
        let shipped = root.join("dragon-native-v1.json");
        if shipped.is_file() {
            return Self::parse(&shipped);
        }
        let jar = root.join("client.jar");
        let meta = jar.metadata().map_err(|e| e.to_string())?;
        let stamp = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |t| t.as_secs());
        let cache = save_root().join("_cache");
        std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
        let path = cache.join(format!("dragon-native-v1-{}-{stamp}.json", meta.len()));
        if !path.exists() {
            let home = PathBuf::from(std::env::var_os("USERPROFILE").ok_or("USERPROFILE missing")?);
            let libraries = home.join("AppData/Roaming/.minecraft/libraries");
            let manifest: serde_json::Value = serde_json::from_slice(
                &std::fs::read(root.join("version.json")).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let mut classpath = vec![jar];
            for lib in manifest["libraries"]
                .as_array()
                .ok_or("Missing library manifest")?
            {
                let Some(relative) = lib["downloads"]["artifact"]["path"].as_str() else {
                    continue;
                };
                let mut p = libraries.join(relative);
                if !p.exists() {
                    let fields: Vec<_> = lib["name"]
                        .as_str()
                        .unwrap_or_default()
                        .split(':')
                        .collect();
                    if fields.len() < 2 {
                        continue;
                    }
                    let dir = libraries.join(fields[0].replace('.', "/")).join(fields[1]);
                    let mut candidates = vec![];
                    for version in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                        for file in std::fs::read_dir(version.path())
                            .into_iter()
                            .flatten()
                            .flatten()
                        {
                            let f = file.path();
                            if f.extension().is_some_and(|x| x == "jar")
                                && !f.to_string_lossy().contains("natives")
                            {
                                candidates.push(f);
                            }
                        }
                    }
                    candidates.sort_by_key(|p| p.metadata().ok().and_then(|m| m.modified().ok()));
                    if let Some(found) = candidates.pop() {
                        p = found;
                    }
                }
                if p.exists() && !classpath.contains(&p) {
                    classpath.push(p);
                }
            }
            let mut java =
                std::env::var_os("JAVA_HOME").map(|p| PathBuf::from(p).join("bin/java.exe"));
            if java.as_ref().is_none_or(|p| !p.exists()) {
                let base = PathBuf::from(
                    std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:/Program Files".into()),
                )
                .join("Eclipse Adoptium");
                let mut jdks: Vec<_> = std::fs::read_dir(base)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path().join("bin/java.exe"))
                    .filter(|p| p.exists())
                    .collect();
                jdks.sort();
                java = jdks.pop();
            }
            let source = cache.join("ExportDragon.java");
            std::fs::write(&source, include_str!("ExportDragon.java"))
                .map_err(|e| e.to_string())?;
            let mut command = std::process::Command::new(java.unwrap_or_else(|| "java".into()));
            command
                .arg("-cp")
                .arg(std::env::join_paths(classpath).map_err(|e| e.to_string())?)
                .arg(source)
                .arg(&path);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            let result = command
                .output()
                .map_err(|e| format!("Native dragon cache requires Java 25: {e}"))?;
            if !result.status.success() {
                return Err(String::from_utf8_lossy(&result.stderr).into_owned());
            }
            eprintln!("{}", String::from_utf8_lossy(&result.stdout).trim());
        }
        Self::parse(&path)
    }

    fn parse(path: &std::path::Path) -> Result<Self, String> {
        let model: Self = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if model.parts.is_empty()
            || model.clips.len() != 2
            || model
                .clips
                .iter()
                .any(|c| c.len() != 64 || c.iter().any(|f| f.len() != model.parts.len()))
        {
            return Err("Invalid native dragon cache".into());
        }
        Ok(model)
    }
    fn matrices(&self, dragon: &adventure::Dragon) -> Vec<Mat4> {
        if self.parts.is_empty() || self.clips.len() < 2 {
            return Vec::new();
        }
        let perch = dragon.phase_time % 40. > 30.;
        let frames = &self.clips[usize::from(perch)];
        let phase =
            (dragon.phase_time * if perch { 0.6 } else { 1. }).rem_euclid(1.) * frames.len() as f32;
        let (a, b) = (
            &frames[phase as usize],
            &frames[(phase as usize + 1) % frames.len()],
        );
        let blend = phase.fract();
        // Native model units are sixteenths of a block, with downward-positive Y.
        let base = Mat4::from_translation(
            Vec3::from_array(dragon.position.map(|x| x as f32))
                + Vec3::Y * (1.5 + dragon.death_time),
        ) * Mat4::from_rotation_y(std::f32::consts::PI - dragon.phase_time * 0.16)
            * Mat4::from_scale(Vec3::new(1., -1., 1.));
        let mut matrices = Vec::with_capacity(self.parts.len());
        for (i, part) in self.parts.iter().enumerate() {
            let position =
                Vec3::from_slice(&a[i][0..3]).lerp(Vec3::from_slice(&b[i][0..3]), blend) / 16.;
            let rotation =
                |p: &[f32; 9]| Quat::from_euler(glam_mc::EulerRot::ZYX, p[5], p[4], p[3]);
            let scale = Vec3::from_slice(&a[i][6..9]).lerp(Vec3::from_slice(&b[i][6..9]), blend);
            let parent = if part.parent < 0 {
                base
            } else {
                matrices[part.parent as usize]
            };
            matrices.push(
                parent
                    * Mat4::from_scale_rotation_translation(
                        scale,
                        rotation(&a[i]).slerp(rotation(&b[i]), blend),
                        position,
                    ),
            );
        }
        matrices
    }
    pub fn bounds(&self, dragon: &adventure::Dragon) -> Vec<([f64; 3], [f64; 3])> {
        let matrices = self.matrices(dragon);
        self.parts
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(p.name.as_str(), "head" | "body"))
            .map(|(i, p)| {
                let mut min = Vec3::splat(f32::INFINITY);
                let mut max = Vec3::splat(f32::NEG_INFINITY);
                for q in &p.quads {
                    for v in q {
                        let v = matrices[i].transform_point3(Vec3::new(v[0], v[1], v[2]) / 16.);
                        min = min.min(v);
                        max = max.max(v);
                    }
                }
                (
                    ((min + max) * 0.5).to_array().map(f64::from),
                    ((max - min) * 0.5).to_array().map(f64::from),
                )
            })
            .collect()
    }
    pub fn append(
        &self,
        mesh: &mut ChunkMesh,
        dragon: &adventure::Dragon,
        atlas: &terrain::mesh::Atlas,
    ) {
        let region =
            atlas.entity_region(&ResourceId::parse("minecraft:entity/enderdragon/dragon").unwrap());
        let matrices = self.matrices(dragon);
        for glow in [false, true] {
            let region = if glow {
                atlas.entity_region(
                    &ResourceId::parse("minecraft:entity/enderdragon/dragon_eyes").unwrap(),
                )
            } else {
                region
            };
            for (i, part) in self.parts.iter().enumerate() {
                if glow && part.name != "head" {
                    continue;
                }
                for quad in &part.quads {
                    let start = mesh.vertices.len() as u32;
                    // Reflecting Y reverses winding. Native mirrored cubes already contain their own corrected winding.
                    for &n in &[3, 2, 1, 0] {
                        let v = quad[n];
                        let mut position =
                            matrices[i].transform_point3(Vec3::new(v[0], v[1], v[2]) / 16.);
                        if glow {
                            let point = |n: usize| {
                                matrices[i].transform_point3(
                                    Vec3::new(quad[n][0], quad[n][1], quad[n][2]) / 16.,
                                )
                            };
                            position += (point(2) - point(3))
                                .cross(point(1) - point(3))
                                .normalize_or_zero()
                                * 0.002;
                        }
                        mesh.vertices.push(Vertex {
                            position: position.to_array(),
                            uv: [
                                region[0] + v[3] * (region[2] - region[0]),
                                region[1] + v[4] * (region[3] - region[1]),
                            ],
                            color: if dragon.health <= 0. {
                                [1., 0.35, 0.65, 1.]
                            } else {
                                [1.; 4]
                            },
                            sky_light: 15.,
                            block_light: if glow { 15. } else { 2. },
                        });
                    }
                    mesh.indices
                        .extend([start, start + 1, start + 2, start, start + 2, start + 3]);
                    mesh.faces += 1;
                }
            }
        }
    }
}
