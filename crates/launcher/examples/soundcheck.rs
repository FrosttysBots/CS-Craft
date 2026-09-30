//! Decodes every weapon sound through the same checks + decoder the client uses.
fn main() {
    let (_, fs) = level::mount_game(None).unwrap();
    let paths = fs.archive_paths_with_prefix("sound/weapons/");
    let (mut ok, mut skipped, mut failed) = (0, 0, 0);
    for p in paths.iter().filter(|p| p.ends_with(".wav") || p.ends_with(".mp3")) {
        let Some(bytes) = fs.read(p) else { continue };
        match client::audio::playable(bytes) {
            Some(b) => {
                let cursor = std::io::Cursor::new(b.clone());
                match rodio::Decoder::builder().with_byte_len(b.len() as u64).with_data(cursor).build() {
                    Ok(_) => ok += 1,
                    Err(e) => { failed += 1; println!("decode failed {p}: {e}"); }
                }
            }
            None => skipped += 1,
        }
    }
    println!("weapon sounds: {ok} decodable, {skipped} skipped (unsupported), {failed} would have crashed");
    if failed > 0 { std::process::exit(1); }
}
