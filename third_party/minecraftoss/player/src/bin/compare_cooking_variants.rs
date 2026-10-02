//! Compare blast-furnace and smoker slots and lit state to the repeated
//! scenarios/cooking-variants.json 26.3 server trace.
use anyhow::{bail, Context, Result};
use minecraftoss_player::{
    crafting::{CookingKind, RecipeBook},
    furnace::Furnace,
    inventory::ItemStack,
};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
};

fn compare_one(tick: u64, key: &str, furnace: &Furnace, observed: &Value) -> Result<()> {
    let lit = observed["block"]["properties"]["lit"]
        .as_str()
        .with_context(|| format!("{key} lit state"))?;
    if lit != furnace.is_lit().to_string() {
        bail!("tick {tick} {key}: lit {lit}, Rust {}", furnace.is_lit());
    }
    let slots = observed["slots"].as_array().context("cooking slots")?;
    if slots.len() != 3 {
        bail!("tick {tick} {key}: expected three slots");
    }
    for (index, observed) in slots.iter().enumerate() {
        let expected_id = furnace.slots[index]
            .as_ref()
            .map_or("minecraft:air", |stack| stack.id.as_str());
        let expected_count = furnace.slots[index].as_ref().map_or(0, |stack| stack.count);
        if observed["id"] != expected_id || observed["count"] != expected_count {
            bail!(
                "tick {tick} {key} slot {index}: vanilla {} x {}, Rust {expected_id} x {expected_count}",
                observed["id"], observed["count"]
            );
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let trace = PathBuf::from(
        args.next()
            .context("usage: compare_cooking_variants TRACE DATA_JAR")?,
    );
    let jar = PathBuf::from(
        args.next()
            .context("usage: compare_cooking_variants TRACE DATA_JAR")?,
    );
    if args.next().is_some() {
        bail!("usage: compare_cooking_variants TRACE DATA_JAR");
    }
    let recipes = RecipeBook::from_jar(&jar)?;
    let mut blast = Furnace::new(CookingKind::BlastFurnace);
    blast.slots[0] = Some(ItemStack::new("minecraft:raw_iron", 2));
    blast.slots[1] = Some(ItemStack::new("minecraft:coal", 1));
    let mut smoker = Furnace::new(CookingKind::Smoker);
    smoker.slots[0] = Some(ItemStack::new("minecraft:beef", 2));
    smoker.slots[1] = Some(ItemStack::new("minecraft:coal", 1));
    let mut current_tick = 0;
    let mut snapshots = 0;
    for line in BufReader::new(File::open(&trace)?).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        if row["type"] != "snapshot" || row["scenario"] != "blasting_and_smoking" {
            continue;
        }
        let tick = row["tick"].as_u64().context("snapshot tick")?;
        while current_tick < tick {
            blast.tick(&recipes);
            smoker.tick(&recipes);
            current_tick += 1;
        }
        let inventories = &row["data"]["inventories"];
        compare_one(tick, "blast_furnace", &blast, &inventories["0,80,0"])?;
        compare_one(tick, "smoker", &smoker, &inventories["2,80,0"])?;
        snapshots += 1;
    }
    if snapshots != 206 {
        bail!("expected 206 tick snapshots, got {snapshots}");
    }
    println!("matched {snapshots} blast-furnace and smoker snapshots, ticks 0..205");
    Ok(())
}
