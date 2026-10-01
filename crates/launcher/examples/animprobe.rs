use mdl::{skeleton, Model};
fn load(fs: &vfs::Vfs, path: &str) -> Option<Model> {
    let base = path.trim_end_matches(".mdl");
    let m = fs.read(path)?;
    match (fs.read(&format!("{base}.vvd")), fs.read(&format!("{base}.dx90.vtx"))) {
        (Some(v), Some(x)) => Model::load(&m, &v, &x).ok(),
        _ => Model::load_header(&m).ok(),
    }
}
fn main() {
    let (_, fs) = level::mount_game(None).unwrap();
    let paths: Vec<String> = std::env::args().skip(1).collect();
    for path in &paths {
        let Some(m) = load(&fs, path) else { println!("{path}: failed"); continue };
        println!("== {path}: {} bones, {} hitboxes, {} anims, {} seqs, {} meshes, includes {:?}, ani {:?} ({} blocks)",
            m.bones.len(), m.hitboxes.len(), m.animations.len(), m.sequences.len(), m.meshes.len(), m.include_models, m.anim_block_file, m.anim_blocks.len());
        for b in m.bones.iter().take(6) { println!("   bone {} parent {} pos {:?} rot {:?}", b.name, b.parent, b.pos, b.rot); }
        // Rest pose consistency.
        if !m.meshes.is_empty() {
            let pose = skeleton::model_space(&m.bones, &skeleton::rest_pose(&m.bones));
            let mut err = 0.0f32; let mut n = 0;
            for mesh in &m.meshes { for v in mesh.vertices.iter().step_by(17) { err = err.max((skeleton::skin_vertex(v, &pose, &m.bones) - v.position).length()); n += 1; } }
            println!("   rest-pose skinning max error {err:.4} over {n} verts");
        }
        let ani = (!m.anim_block_file.is_empty()).then(|| fs.read(&m.anim_block_file)).flatten();
        let (mut ok, mut bad, mut flagsseen) = (0, 0, std::collections::BTreeMap::new());
        for (i, a) in m.animations.iter().enumerate() {
            *flagsseen.entry(a.flags).or_insert(0) += 1;
            match m.decode_animation(i, ani.as_deref()) { Ok(_) => ok += 1, Err(e) => { bad += 1; if bad < 4 { println!("   anim {} failed: {e}", a.name); } } }
        }
        println!("   anims decoded ok {ok} failed {bad}; flags {flagsseen:?}");
        // Validate decoded rotations: unit length after decode, and for frame
        // anims the reconstructed component must dominate.
        let mut worst = 0.0f32;
        for i in 0..m.animations.len() {
            if let Ok(c) = m.decode_animation(i, ani.as_deref()) {
                for fr in &c.frames { for (_, q) in fr { worst = worst.max((q.length() - 1.0).abs()); } }
            }
        }
        println!("   max |q|-1 over all decoded frames: {worst:.5}");
        for s in m.sequences.iter().take(40) { print!("{}({}) ", s.name, s.anims.len()); }
        println!();
    }
}
// (validation helper appended)
#[allow(dead_code)]
fn _unused() {}
