//! `csr` — CSGO Rewrite launcher.
//!
//! ```text
//! csr [play] [map]        open the game window (default map de_dust2)
//! csr sim [map]           headless movement run on a real map, prints a report
//! csr probe [map]         load every asset a map references and report
//! ```
//! Set `CSR_GAME_DIR` if the CS:GO install is not found automatically.

mod probe;
mod simcheck;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = match args.first().map(String::as_str) {
        Some("sim") | Some("probe") | Some("play") => (args[0].as_str(), &args[1..]),
        _ => ("play", &args[..]),
    };
    let map = rest.first().cloned().unwrap_or_else(|| "de_dust2".into());
    let code = match cmd {
        "probe" => probe::run(&map),
        "sim" => simcheck::run(&map),
        _ => {
            eprintln!("the game window is not built yet; try `csr sim {map}`");
            2
        }
    };
    std::process::exit(code);
}
