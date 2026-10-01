fn main() {
    let (_, fs) = level::mount_game(None).unwrap();
    let path = std::env::args().nth(1).unwrap();
    let filter = std::env::args().nth(2).unwrap_or_default();
    let m = mdl::Model::load_header(&fs.read(&path).unwrap()).unwrap();
    println!("pose params: {:?}", m.pose_params);
    for s in &m.sequences {
        if !filter.is_empty() && !s.name.contains(&filter) { continue; }
        let anims: Vec<String> = s.anims.iter().map(|&a| m.animations.get(a as usize).map(|d| format!("{}({}f@{})", d.name, d.num_frames, d.fps)).unwrap_or("?".into())).collect();
        let pp = |i: i32| m.pose_params.get(i as usize).map(|p| p.0.clone()).unwrap_or("-".into());
        let zero = s.bone_weights.iter().filter(|w| **w == 0.0).count();
        print!("[weights: {} zero of {}] ", zero, s.bone_weights.len());
        println!("{} act={} flags={:#x} grid {:?} params ({}, {}) {:?}..{:?}: {}", s.name, s.activity, s.flags, s.group_size, pp(s.param_index[0]), pp(s.param_index[1]), s.param_start, s.param_end, anims.join(", "));
    }
}
#[allow(dead_code)]
fn _w() {}
