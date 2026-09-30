//! `csr` — CSGO Rewrite launcher.
//!
//! ```text
//! csr [play] [map] [options]   open the game window (default map de_dust2)
//!     --screenshot <file.png>  render, save a screenshot, exit (smoke test)
//!     --frames <n>             frames to render before the screenshot (120)
//!     --camera x y z pitch yaw start in noclip at a Source position
//!     --exposure <f>           brightness multiplier (2.0)
//!     --size <w> <h>           window size (1600 900)
//! csr sim [map]                headless movement run on a real map
//! csr probe [map]              load every asset a map references and report
//! csr weapons                  print the weapon catalog from items_game.txt
//! ```
//! Set `CSR_GAME_DIR` if the CS:GO install is not found automatically.

mod probe;
mod simcheck;

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = match args.first().map(String::as_str) {
        Some("sim") | Some("probe") | Some("play") | Some("weapons") => (args[0].as_str(), &args[1..]),
        _ => ("play", &args[..]),
    };
    let map = rest
        .first()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "de_dust2".into());
    let code = match cmd {
        "probe" => probe::run(&map),
        "sim" => simcheck::run(&map),
        "weapons" => weapons_cmd(),
        _ => match parse_play(&map, rest) {
            Ok(opts) => client::run(opts),
            Err(e) => {
                eprintln!("error: {e}");
                2
            }
        },
    };
    std::process::exit(code);
}

fn weapons_cmd() -> i32 {
    let (_, fs) = match level::mount_game(None) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let Some(text) = fs.read_string("scripts/items/items_game.txt") else {
        eprintln!("error: scripts/items/items_game.txt not found");
        return 1;
    };
    let catalog = match weapons::Catalog::from_items_game(&text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: items_game.txt: {e}");
            return 1;
        }
    };
    println!(
        "{:<24} {:<5} {:<10} {:>5} {:>6} {:>5} {:>6} {:>5} {:>4} {:>5} {:>6}",
        "id", "def", "class", "price", "damage", "armor", "rpm", "clip", "auto", "speed", "recoil"
    );
    for w in &catalog.weapons {
        println!(
            "{:<24} {:<5} {:<10} {:>5} {:>6.0} {:>5.2} {:>6.0} {:>5} {:>4} {:>5.0} {:>6.0}",
            w.id.to_string(),
            w.def_index,
            format!("{:?}", w.class),
            w.price,
            w.damage,
            w.armor_ratio,
            w.rpm(),
            w.clip_size,
            if w.full_auto { "yes" } else { "" },
            w.max_player_speed,
            w.recoil.magnitude
        );
    }
    println!("{} weapons", catalog.weapons.len());
    0
}

fn parse_play(map: &str, args: &[String]) -> Result<client::ClientOptions, String> {
    let mut opts = client::ClientOptions {
        map: map.to_string(),
        ..Default::default()
    };
    let num = |s: Option<&String>, what: &str| -> Result<f32, String> {
        s.ok_or(format!("{what} needs a value"))?
            .parse::<f32>()
            .map_err(|_| format!("{what}: not a number"))
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--screenshot" => {
                opts.screenshot = Some(PathBuf::from(args.get(i + 1).ok_or("--screenshot needs a path")?));
                i += 1;
            }
            "--frames" => {
                opts.frames = num(args.get(i + 1), "--frames")? as u32;
                i += 1;
            }
            "--exposure" => {
                opts.exposure = num(args.get(i + 1), "--exposure")?;
                i += 1;
            }
            "--size" => {
                opts.width = num(args.get(i + 1), "--size")? as u32;
                opts.height = num(args.get(i + 2), "--size")? as u32;
                i += 2;
            }
            "--camera" => {
                let v: Result<Vec<f32>, String> = (1..=5).map(|k| num(args.get(i + k), "--camera")).collect();
                let v = v?;
                opts.camera = Some((glam::Vec3::new(v[0], v[1], v[2]), v[3], v[4]));
                i += 5;
            }
            _ => {}
        }
        i += 1;
    }
    Ok(opts)
}
