//! Compare the semantic furnace slots and lit state against a repeated 26.3
//! server trace from scenarios/furnace-smelt.json.
use anyhow::{bail, Context, Result};
use minecraftoss_player::{crafting::RecipeBook, furnace::Furnace, inventory::ItemStack};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let trace = PathBuf::from(
        args.next()
            .context("usage: compare_furnace TRACE DATA_JAR")?,
    );
    let jar = PathBuf::from(
        args.next()
            .context("usage: compare_furnace TRACE DATA_JAR")?,
    );
    if args.next().is_some() {
        bail!("usage: compare_furnace TRACE DATA_JAR");
    }
    let recipes = RecipeBook::from_jar(&jar)?;
    let mut furnace = Furnace::default();
    furnace.slots[0] = Some(ItemStack::new("minecraft:sand", 2));
    furnace.slots[1] = Some(ItemStack::new("minecraft:coal", 1));
    let mut current_tick = 0;
    let mut snapshots = 0;
    for line in BufReader::new(File::open(&trace)?).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        if row["type"] != "snapshot" || row["scenario"] != "sand_to_glass_with_coal" {
            continue;
        }
        let tick = row["tick"].as_u64().context("snapshot tick")?;
        while current_tick < tick {
            furnace.tick(&recipes);
            current_tick += 1;
        }
        let observed = &row["data"]["inventories"]["0,80,0"];
        let lit = observed["block"]["properties"]["lit"]
            .as_str()
            .context("lit state")?;
        if lit != furnace.is_lit().to_string() {
            bail!("tick {tick}: lit {lit}, Rust {}", furnace.is_lit());
        }
        let slots = observed["slots"].as_array().context("furnace slots")?;
        if slots.len() != 3 {
            bail!("tick {tick}: expected three furnace slots");
        }
        for (index, observed) in slots.iter().enumerate() {
            let expected_id = furnace.slots[index]
                .as_ref()
                .map_or("minecraft:air", |stack| stack.id.as_str());
            let expected_count = furnace.slots[index].as_ref().map_or(0, |stack| stack.count);
            if observed["id"] != expected_id || observed["count"] != expected_count {
                bail!(
                    "tick {tick} slot {index}: vanilla {} x {}, Rust {} x {}",
                    observed["id"],
                    observed["count"],
                    expected_id,
                    expected_count
                );
            }
        }
        snapshots += 1;
    }
    if snapshots != 206 {
        bail!("expected 206 tick snapshots, got {snapshots}");
    }
    println!("matched {snapshots} furnace snapshots, ticks 0..205");
    Ok(())
}
