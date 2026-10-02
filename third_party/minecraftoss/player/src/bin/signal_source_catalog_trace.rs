//! Compare authored-circuit signal-source membership with two pinned 26.3 captures.
use anyhow::{bail, Context, Result};
use minecraftoss_player::{redstone::is_signal_source, Block};
use serde_json::Value;
use std::{env, fs, path::Path};

fn catalog(path: &Path) -> Result<Value> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let snapshots: Vec<Value> = raw
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<_, _>>()?;
    let observations: Vec<_> = snapshots
        .iter()
        .filter_map(|row| row.pointer("/data/signal_source_catalog"))
        .collect();
    if observations.len() != 2 || observations[0] != observations[1] {
        bail!(
            "expected matching setup/tick signal-source catalogs in {}",
            path.display()
        );
    }
    Ok(observations[0].clone())
}

fn main() -> Result<()> {
    let paths: Vec<_> = env::args().skip(1).collect();
    if paths.len() != 2 {
        bail!("usage: signal_source_catalog_trace VANILLA_A.jsonl VANILLA_B.jsonl");
    }
    let first = catalog(Path::new(&paths[0]))?;
    let second = catalog(Path::new(&paths[1]))?;
    if first != second {
        bail!("pinned catalogs differ between fresh runs");
    }
    let blocks = first.as_object().context("catalog must be an object")?;
    if blocks.len() != 24 {
        bail!("expected 24 sampled block IDs, got {}", blocks.len());
    }
    let mut checked = 0;
    for (id, states) in blocks {
        for (key, expected) in states.as_object().context("state map")? {
            let mut block = Block::new(id);
            if let Some(value) = key.strip_prefix("powered_") {
                block = block.with("powered", value);
            } else if let Some(value) = key.strip_prefix("lit_") {
                block = block.with("lit", value);
            } else if key != "default" {
                bail!("unexpected catalog state key {key}");
            }
            let actual = is_signal_source(&block);
            if Some(actual) != expected.as_bool() {
                bail!("{id} {key}: Rust={actual}, vanilla={expected}");
            }
            checked += 1;
        }
    }
    println!(
        "matched {checked} state flags across {} block IDs in two vanilla runs",
        blocks.len()
    );
    Ok(())
}
