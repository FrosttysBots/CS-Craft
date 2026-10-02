//! Decodes every weapon sound through the same checks + decoder the client uses.
fn main() {
    let (_, fs) = level::mount_game(None).unwrap();
    let mut paths = fs.archive_paths_with_prefix("sound/weapons/");
    let library = client::audio::SoundLibrary::load(&fs);
    for name in [
        "Flesh.BulletImpact",
        "Player.DamageHeadShot",
        "Player.DamageHelmet",
        "Player.DamageKevlar",
        "Concrete.BulletImpact",
        "SolidMetal.BulletImpact",
        "Wood.BulletImpact",
        "Glass.BulletImpact",
        "HEGrenade.Throw",
        "Molotov.Throw",
        "IncGrenade.Throw",
        "Flashbang.Explode",
        "BaseGrenade.Explode",
        "Inferno.Start_IncGrenade",
        "SmokeGrenade.Bounce",
    ] {
        let waves = library
            .waves(name)
            .unwrap_or_else(|| panic!("Missing feedback soundscript: {name}"));
        assert!(!waves.is_empty(), "Empty feedback sound: {name}");
        for wave in waves {
            assert!(fs.exists(wave), "Missing feedback audio: {wave}");
            paths.push(wave.clone());
        }
    }
    paths.sort();
    paths.extend(["sound/ui/panorama/case_unlock_01.wav","sound/ui/csgo_ui_crate_item_scroll.wav","sound/ui/item_reveal3_rare.wav","sound/ui/item_reveal4_mythical.wav","sound/ui/item_reveal5_legendary.wav","sound/ui/item_reveal6_ancient.wav"].map(String::from));
    paths.sort();
    paths.dedup();
    let (mut ok, mut skipped, mut failed) = (0, 0, 0);
    for p in paths
        .iter()
        .filter(|p| p.ends_with(".wav") || p.ends_with(".mp3"))
    {
        let Some(bytes) = fs.read(p) else { continue };
        match client::audio::playable(bytes) {
            Some(b) => {
                let cursor = std::io::Cursor::new(b.clone());
                match rodio::Decoder::builder()
                    .with_byte_len(b.len() as u64)
                    .with_data(cursor)
                    .build()
                {
                    Ok(_) => ok += 1,
                    Err(e) => {
                        failed += 1;
                        println!("decode failed {p}: {e}");
                    }
                }
            }
            None => skipped += 1,
        }
    }
    println!(
        "weapon and feedback sounds: {ok} decodable, {skipped} skipped (unsupported), {failed} would have crashed"
    );
    if failed > 0 {
        std::process::exit(1);
    }
}
