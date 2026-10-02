//! Inspect installed feedback assets and world-model resting orientations.
fn main() {
    let (_, fs) = level::mount_game(None).unwrap();
    let v = vtf::Vtf::parse(&fs.read("materials/decals/blood1.vtf").unwrap()).unwrap();
    let rgba = v.rgba8(0, 0, 0).unwrap();
    println!(
        "blood: {}x{} {:?}; corners {:?} {:?} {:?}",
        v.width,
        v.height,
        v.format,
        &rgba[..4],
        &rgba[(v.width - 1) * 4..v.width * 4],
        &rgba[rgba.len() - 4..]
    );
    let mut frequencies = std::collections::HashMap::new();
    for pixel in rgba.chunks_exact(4) {
        *frequencies.entry(pixel.to_vec()).or_insert(0usize) += 1;
    }
    let mut frequencies: Vec<_> = frequencies.into_iter().collect();
    frequencies.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    println!(
        "most common blood pixels: {:?}",
        &frequencies[..frequencies.len().min(8)]
    );
    let catalog =
        weapons::Catalog::from_items_game(&fs.read_string("scripts/items/items_game.txt").unwrap())
            .unwrap();
    for name in [
        "weapon_ak47",
        "weapon_m4a1_silencer",
        "weapon_glock",
        "weapon_knife",
    ] {
        let def = catalog.get(name).unwrap();
        let base = def.world_model.trim_end_matches(".mdl");
        let model = mdl::Model::load(
            &fs.read(&def.world_model).unwrap(),
            &fs.read(&format!("{base}.vvd")).unwrap(),
            &fs.read(&format!("{base}.dx90.vtx")).unwrap(),
        )
        .unwrap();
        println!(
            "bones: {:?}",
            model.bones.iter().map(|b| &b.name).collect::<Vec<_>>()
        );
        for bone in model.bones.iter().filter(|b| b.name.contains("weapon")) {
            let (mut lo, mut hi) = (
                glam::Vec3::splat(f32::INFINITY),
                glam::Vec3::splat(f32::NEG_INFINITY),
            );
            for v in model.meshes.iter().flat_map(|m| &m.vertices) {
                let p = bone.pose_to_bone.transform_point3(v.position);
                lo = lo.min(p);
                hi = hi.max(p);
            }
            println!(
                "{} local bounds {lo:?}..{hi:?}, size {:?}",
                bone.name,
                hi - lo
            );
        }
        for roll in [0.0, 90.0] {
            let rot = mdl::angles_to_mat3(glam::Vec3::new(0.0, 0.0, roll));
            let (mut lo, mut hi) = (
                glam::Vec3::splat(f32::INFINITY),
                glam::Vec3::splat(f32::NEG_INFINITY),
            );
            for v in model.meshes.iter().flat_map(|m| &m.vertices) {
                lo = lo.min(rot * v.position);
                hi = hi.max(rot * v.position);
            }
            println!(
                "{name} roll {roll}: bounds {lo:?}..{hi:?}, size {:?}",
                hi - lo
            );
        }
    }
}
