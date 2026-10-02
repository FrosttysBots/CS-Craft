//! Audit installed first-person inspect clips and muzzle bones without a renderer.
fn main() {
    let (_, fs) = level::mount_game(None).unwrap();
    let catalog =
        weapons::Catalog::from_items_game(&fs.read_string("scripts/items/items_game.txt").unwrap())
            .unwrap();
    let mut checked = 0;
    let mut failed = 0;
    for def in &catalog.weapons {
        if !client::inventory::visible(def, 0) {
            continue;
        }
        let m = mdl::Model::load_header(&fs.read(&def.view_model).unwrap()).unwrap();
        let includes: Vec<_> = m
            .include_models
            .iter()
            .filter_map(|p| fs.read(p).and_then(|b| mdl::Model::load_header(&b).ok()))
            .collect();
        let seq = std::iter::once(&m).chain(includes.iter()).find_map(|src| {
            src.sequences
                .iter()
                .find(|s| s.name == "lookat01" || s.name == "inspect")
                .map(|s| (src, s))
        });
        let inspect = seq.is_some_and(|(src, s)| {
            s.anims.first().is_some_and(|&a| {
                src.decode_animation(
                    a as usize,
                    src.anim_block_file
                        .is_empty()
                        .then_some(None)
                        .unwrap_or_else(|| fs.read(&src.anim_block_file))
                        .as_deref(),
                )
                .is_ok()
            })
        });
        let muzzle = def.class == weapons::WeaponClass::Knife
            || client::tracers::muzzle_attachment(&m, false).is_some();
        println!("{}: inspect={} muzzle={}", def.class_name, inspect, muzzle);
        checked += 1;
        if !inspect || !muzzle {
            failed += 1;
        }
    }
    println!("{checked} weapons checked; {failed} failures");
    assert_eq!(failed, 0);
}
