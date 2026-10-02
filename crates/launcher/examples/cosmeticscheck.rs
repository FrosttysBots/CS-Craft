//! Audit installed cosmetic assets and transactional opening without touching the player profile.
use client::cosmetics::*;
fn main() {
    let (_, fs) = level::mount_game(None).unwrap();
    let catalog = Catalog::load(&fs).unwrap();
    assert!(catalog.cases.len() >= 39);
    println!(
        "Catalog: {} containers, {} finishes, {} stickers",
        catalog.cases.len(),
        catalog.finishes.len(),
        catalog.stickers.len()
    );
    assert_eq!(catalog.cases.iter().filter(|c| !c.souvenir).count(), 39);
    assert!(catalog.stickers.len() > 1000);
    for c in &catalog.cases {
        if !c.souvenir {
            let rare = c
                .contents
                .iter()
                .filter(|&&i| catalog.finishes[i].rarity == 7)
                .count();
            println!("{} {}: {rare} rare finishes", c.id, c.name);
            assert!(rare > 0, "No rare pool: {}", c.name);
        } else {
            assert!(c.event > 0, "No souvenir event: {}", c.name);
            assert!(c.contents.iter().all(|&i| catalog.finishes[i].rarity < 7));
        }
    }
    let revolution = catalog.find_case("Revolution").unwrap();
    let counts: Vec<_> = (3..=6)
        .map(|r| {
            revolution
                .contents
                .iter()
                .filter(|&&i| catalog.finishes[i].rarity == r)
                .count()
        })
        .collect();
    assert_eq!(counts, [7, 5, 3, 2]);
    assert_eq!(
        catalog
            .finishes
            .iter()
            .find(|f| f.paint == "gs_r8_banana")
            .unwrap()
            .rarity,
        4
    );
    assert_eq!(
        catalog
            .finishes
            .iter()
            .find(|f| f.paint == "cu_ak_head_shot_holo")
            .unwrap()
            .rarity,
        6
    );
    assert!(catalog.find_case("Prisma").is_err()); // Ambiguous substring, except exact case names.
    let mut missing = vec![];
    for c in &catalog.cases {
        if !fs.exists(&format!("resource/flash/{}.png", c.icon)) {
            missing.push(c.icon.clone());
        }
    }
    for f in &catalog.finishes {
        if !fs.exists(&format!("resource/flash/{}.png", f.icon)) {
            missing.push(f.icon.clone());
        }
        if !f.pattern.is_empty() && !fs.exists(&format!("materials/{}.vtf", f.pattern)) {
            missing.push(f.pattern.clone());
        }
        assert!(f.wear[0] <= f.wear[1]);
        assert!(
            catalog.materials.contains_key(&f.weapon),
            "Missing paint mapping: {}",
            f.weapon
        );
    }
    assert!(missing.is_empty(), "Missing cosmetic assets: {missing:?}");
    let weapons =
        weapons::Catalog::from_items_game(&fs.read_string("scripts/items/items_game.txt").unwrap())
            .unwrap();
    let model = |path: &str| {
        let base = path.trim_end_matches(".mdl");
        mdl::Model::load(
            &fs.read(path).unwrap(),
            &fs.read(&format!("{base}.vvd")).unwrap(),
            &fs.read(&format!("{base}.dx90.vtx")).unwrap(),
        )
        .unwrap()
    };
    let mut decals = 0;
    let mut unmapped = vec![];
    for (name, info) in &catalog.models {
        if info.stickers.is_empty() || !weapons.weapons.iter().any(|w| w.class_name == *name) {
            continue;
        }
        let view = model(&info.view);
        let world = model(&info.world);
        for path in &info.stickers {
            let decal = model(path);
            if client::cosmetic_geometry::world_decal(&decal, &view, &world).is_some() {
                decals += 1;
            } else {
                unmapped.push(path.clone());
            }
        }
    }
    println!(
        "World sticker mappings {decals}, unavailable {}: {unmapped:?}",
        unmapped.len()
    );
    assert!(
        unmapped.is_empty(),
        "World sticker mappings must not silently disappear"
    );
    for (_, _, path) in client::cosmetic_catalog::model_items(&fs) {
        let glove = model(&path);
        for body in [client::characters::CT_MODEL, client::characters::T_MODEL] {
            let body = model(body);
            assert!(
                glove
                    .bones
                    .iter().enumerate()
                    .all(|(index,g)| (g.parent>=0 && (g.parent as usize)<index) || body.bones.iter().any(|b| b.name == g.name)),
                "Glove skeleton cannot merge: {path}"
            );
        }
    }
    for f in catalog.finishes.iter().filter(|f| is_rare_knife(&f.weapon)) {
        let def = weapons.get(&f.weapon).unwrap();
        assert_eq!(def.class, weapons::WeaponClass::Knife);
        assert!(fs.exists(&def.view_model) && fs.exists(&def.world_model));
    }
    for case in &catalog.cases {
        let mut i = Inventory::default();
        i.grant(case.id, 1).unwrap();
        let item = i.open(case, &catalog, &mut Random(case.id as u64)).unwrap();
        assert_eq!(item.souvenir.is_some(), case.souvenir);
        if case.souvenir {
            assert!(item.stattrak.is_none());
            assert!(
                item.stickers.iter().any(Option::is_some),
                "Missing souvenir stickers for {}",
                case.name
            );
            if case.event >= 18 {
                assert!(
                    item.stickers.iter().all(Option::is_some),
                    "Modern package needs four stickers: {}",
                    case.name
                );
                let kits: Vec<_> = item
                    .stickers
                    .iter()
                    .flatten()
                    .map(|s| &catalog.stickers[&s.kit])
                    .collect();
                assert!(
                    kits.iter().all(|s| s.player == 0),
                    "Modern package must not award MVP signatures"
                );
                assert_eq!(kits[0].team, 0);
                assert!(kits[1].team > 0 && kits[2].team > 0 && kits[1].team != kits[2].team);
                assert!(kits[3].material.contains("tournament_assets/de_"));
            }
        }
    }
    for (weapon, mappings) in &catalog.materials {
        if is_glove(weapon) {
            continue;
        }
        if !catalog.finishes.iter().any(|f| f.weapon == *weapon) {
            continue;
        }
        let def = weapons
            .weapons
            .iter()
            .find(|w| w.class_name == *weapon)
            .unwrap();
        for path in [&def.view_model, &def.world_model] {
            let base = path.trim_end_matches(".mdl");
            let model = mdl::Model::load(
                &fs.read(path).unwrap(),
                &fs.read(&format!("{base}.vvd")).unwrap(),
                &fs.read(&format!("{base}.dx90.vtx")).unwrap(),
            )
            .unwrap();
            assert!(
                model.textures.iter().any(|m| mappings
                    .iter()
                    .any(|(orig, name, _)| m.eq_ignore_ascii_case(orig)
                        || m.eq_ignore_ascii_case(name))),
                "Unmapped skin model {path}: {:?}",
                model.textures
            );
        }
        for (_, name, folder) in mappings {
            assert!(
                fs.exists(&format!(
                    "materials/models/weapons/customization/{folder}/{name}_masks.vtf"
                )),
                "Missing masks for {weapon}"
            );
            assert!(
                fs.exists(&format!(
                    "materials/models/weapons/customization/{folder}/{name}_ao.vtf"
                )),
                "Missing AO for {weapon}"
            );
        }
    }
    let temp = std::env::temp_dir().join(format!(
        "csr-econ-audit-{}-{}",
        std::process::id(),
        Random::fresh().0
    ));
    let path = temp.join("cosmetics.cfg");
    let mut inventory = Inventory::default();
    command("give_case Revolution 1000", &catalog, &mut inventory, &path).unwrap();
    let mut rng = Random(42);
    let mut distribution = [0; 8];
    for _ in 0..1000 {
        let item = inventory
            .transaction(&path, |i| i.open(revolution, &catalog, &mut rng))
            .unwrap();
        let finish = catalog.finish(&item).unwrap();
        distribution[finish.rarity as usize] += 1;
        assert!(item.seed <= 1000 && item.wear >= finish.wear[0] && item.wear <= finish.wear[1]);
    }
    assert_eq!(inventory.skins.len(), 1000);
    assert_eq!(inventory.cases[&revolution.id], 0);
    let before = std::fs::read(&path).unwrap();
    assert!(
        inventory
            .transaction(&path, |i| i.open(revolution, &catalog, &mut rng))
            .is_err()
    );
    assert_eq!(before, std::fs::read(&path).unwrap());
    inventory.transaction(&path, |i| i.equip(1000)).unwrap();
    assert_eq!(Inventory::load(&path).unwrap(), inventory);
    for path in [
        "sound/ui/panorama/case_unlock_01.wav",
        "sound/ui/csgo_ui_crate_item_scroll.wav",
        "sound/ui/item_reveal3_rare.wav",
        "sound/ui/item_reveal4_mythical.wav",
        "sound/ui/item_reveal5_legendary.wav",
        "sound/ui/item_reveal6_ancient.wav",
    ] {
        let bytes = client::audio::playable(fs.read(path).unwrap()).unwrap();
        rodio::Decoder::builder()
            .with_byte_len(bytes.len() as u64)
            .with_data(std::io::Cursor::new(bytes))
            .build()
            .unwrap();
    }
    let source = mdl::Model::load_header(
        &fs.read("models/player/custom_player/uiplayer/animset_uiplayer_loadout.mdl")
            .unwrap(),
    )
    .unwrap();
    let index = source
        .animation_index("@ct_loadout_rifle_idle_handrepo_m4")
        .unwrap();
    let clip = source.decode_animation(index, None).unwrap();
    assert!(clip.frames.len() > 100);
    let mut a = mdl::skeleton::rest_pose(&source.bones);
    let mut b = a.clone();
    clip.sample(0., &mut a);
    clip.sample(2., &mut b);
    assert!(
        a.iter()
            .zip(&b)
            .any(|(a, b)| a.0.distance(b.0) > 0.001 || a.1.dot(b.1).abs() < 0.9999)
    );
    println!(
        "{} cases; {} finishes; icons/textures present; 1000 durable openings; rarity counts {:?}; six opening sounds decoded; menu idle {} frames / {:.2}s; failures: 0",
        catalog.cases.len(),
        catalog.finishes.len(),
        &distribution[3..],
        clip.frames.len(),
        clip.duration()
    );
    std::fs::remove_dir_all(temp).unwrap();
}
