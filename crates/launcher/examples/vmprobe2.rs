// vmprobe2 <model> <activity>: muzzle/hand positions through an animation.
fn main() {
    let (_, fs) = level::mount_game(None).unwrap();
    let args: Vec<String> = std::env::args().collect();
    let path = &args[1];
    let act = &args[2];
    let base = path.trim_end_matches(".mdl");
    let m = mdl::Model::load(&fs.read(path).unwrap(), &fs.read(&format!("{base}.vvd")).unwrap(), &fs.read(&format!("{base}.dx90.vtx")).unwrap()).unwrap();
    let ani = fs.read(&m.anim_block_file);
    let mut worst = (0.0f32, String::new());
    for b in &m.bones {
        let e = mdl::skeleton::euler_to_quat(b.rot);
        let d = b.quat.dot(e).abs();
        let ang = 2.0 * d.min(1.0).acos().to_degrees();
        if ang > worst.0 { worst = (ang, b.name.clone()); }
    }
    println!("max quat-vs-euler rest mismatch: {:.2} deg at {}", worst.0, worst.1);
    for seq in m.sequences.iter().filter(|s| &s.activity == act) {
        let desc = &m.animations[seq.anims[0] as usize];
        let clip = m.decode_animation(seq.anims[0] as usize, ani.as_deref()).unwrap();
        let parents: Vec<String> = m.bones.iter().enumerate().filter(|(_, b)| b.name.to_lowercase().contains("parent")).map(|(i, b)| format!("{}:{}", b.name, clip.animated[i])).collect();
        println!("  parent bones animated: {parents:?}");
        println!("{} anim {} frames {} sectionframes {} sections {:?} animated {}", seq.name, desc.name, desc.num_frames, desc.section_frames, desc.sections, clip.animated.iter().filter(|a| **a).count());
        let flash = std::env::var("BONE").ok().and_then(|n| m.bone_index(&n)).unwrap_or_else(|| m.bones.iter().position(|b| b.name.to_lowercase().contains("flash") || b.name.to_lowercase().contains("muzzle")).unwrap_or(0));
        println!("  tracking bone {} animated in this clip: {}", m.bones[flash].name, clip.animated[flash]);
        let hand = m.bone_index("v_weapon.Bip01_R_Hand").unwrap_or(0);
        for k in 0..=5 {
            let t = clip.duration() * k as f32 / 5.0;
            let mut local = mdl::skeleton::rest_pose(&m.bones);
            clip.sample(t, &mut local);
            let pose = mdl::skeleton::model_space(&m.bones, &local);
            println!("  t {:.2}: muzzle {:?} hand {:?}", t, pose[flash].transform_point3(glam::Vec3::ZERO), pose[hand].transform_point3(glam::Vec3::ZERO));
        }
    }
}
#[allow(dead_code)]
fn unused() {}
