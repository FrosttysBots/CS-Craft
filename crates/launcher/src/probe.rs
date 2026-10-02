//! `csr probe <map>`: loads everything a map references and reports.

use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

pub fn run(map: &str) -> i32 {
    let t = Instant::now();
    let level = match level::Level::load(map, None) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let bsp = &level.bsp;
    let fs = &level.vfs;
    println!("game dir: {}", level.game_dir.display());
    println!("loaded {map} in {:.2}s {:?}", t.elapsed().as_secs_f32(), level.timings);
    println!(
        "bsp v{} rev {}: {} faces, {} brushes, {} displacements, {} static props ({} models), {} entities, lighting {} ({})",
        bsp.version,
        bsp.map_revision,
        bsp.faces.len(),
        bsp.brushes.len(),
        bsp.dispinfos.len(),
        bsp.static_props.props.len(),
        bsp.static_props.models.len(),
        bsp.entities.len(),
        bsp.lighting.len(),
        if bsp.lighting_is_hdr { "HDR" } else { "LDR" }
    );
    println!(
        "collision: {} convexes, {} triangles, bounds {:?}..{:?}",
        level.collision.convexes.len(),
        level.collision.triangles.len(),
        level.collision.mins,
        level.collision.maxs
    );

    let mut failures = 0;
    let mut displacement_lightmaps = 0;
    for (i, face) in bsp.faces.iter().enumerate().filter(|(_, f)| f.dispinfo >= 0) {
        if let (Some(mesh), Some((width, height, _))) = (bsp.displacement(i), bsp.face_lightmap(face)) {
            displacement_lightmaps += 1;
            if mesh.luxel_coords.iter().any(|uv| !uv.is_finite() || uv.x < 0.0 || uv.y < 0.0 || uv.x > (width-1) as f32 || uv.y > (height-1) as f32) {
                failures += 1;
                println!("  displacement {i}: lightmap UV outside its own atlas rectangle");
            }
        }
    }
    println!("displacement lightmaps: {displacement_lightmaps} checked");
    let read = |p: &str| fs.read_string(p);
    let mut shaders: BTreeMap<String, usize> = BTreeMap::new();
    let mut formats: BTreeMap<String, usize> = BTreeMap::new();
    let mut seen = HashSet::new();
    let mut check_tex = |name: &str, formats: &mut BTreeMap<String, usize>, failures: &mut i32| {
        let path = vmt::vtf_path(name);
        if !seen.insert(path.clone()) {
            return;
        }
        match fs.read(&path).map(|b| vtf::Vtf::parse(&b)) {
            Some(Ok(v)) => *formats.entry(format!("{:?}", v.format)).or_default() += 1,
            Some(Err(e)) => {
                *failures += 1;
                println!("  bad texture {path}: {e}");
            }
            None => {
                *failures += 1;
                println!("  missing texture {path}");
            }
        }
    };
    for name in &bsp.texdata_strings {
        match vmt::Material::load(name, &read) {
            Some(m) => {
                *shaders.entry(m.shader.clone()).or_default() += 1;
                if !m.is_nodraw() {
                    for k in ["$basetexture", "$basetexture2"] {
                        if let Some(t) = m.str(k) {
                            check_tex(t, &mut formats, &mut failures);
                        }
                    }
                }
            }
            None => {
                failures += 1;
                println!("  missing material {name}");
            }
        }
    }
    println!("world shaders: {shaders:?}");

    let t = Instant::now();
    let (mut ok, mut tris) = (0, 0usize);
    for name in &bsp.static_props.models {
        let base = name.trim_end_matches(".mdl");
        let files = (
            fs.read(name),
            fs.read(&format!("{base}.vvd")),
            fs.read(&format!("{base}.dx90.vtx")),
        );
        let (Some(m), Some(v), Some(x)) = files else {
            failures += 1;
            println!("  missing model files for {name}");
            continue;
        };
        match mdl::Model::load(&m, &v, &x) {
            Ok(model) => {
                ok += 1;
                tris += model.meshes.iter().map(|m| m.indices.len() / 3).sum::<usize>();
                for mesh in &model.meshes {
                    let tex = model.skin_texture(0, mesh.material);
                    let found = model
                        .material_candidates(tex)
                        .into_iter()
                        .find_map(|c| vmt::Material::load(&c, &read));
                    match found {
                        Some(mat) => {
                            if let Some(t) = mat.base_texture() {
                                check_tex(t, &mut formats, &mut failures);
                            }
                        }
                        None => {
                            failures += 1;
                            println!("  {name}: no material for texture {tex}");
                        }
                    }
                }
            }
            Err(e) => {
                failures += 1;
                println!("  bad model {name}: {e}");
            }
        }
    }
    println!(
        "prop models: {ok}/{} loaded, {tris} triangles ({:.2}s)",
        bsp.static_props.models.len(),
        t.elapsed().as_secs_f32()
    );
    println!("texture formats: {formats:?}");
    println!("failures: {failures}");
    if failures == 0 { 0 } else { 1 }
}
