//! Compare pinned 26.3 component-fuel furnace traces with Rust tick state.
use anyhow::{bail, Context, Result};
use minecraftoss_player::{crafting::RecipeBook, furnace::Furnace, item_catalog::ItemCatalog};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
    sync::Arc,
};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let trace = PathBuf::from(
        args.next()
            .context("usage: compare_furnace_fuels TRACE DATA_JAR ITEM_CATALOG")?,
    );
    let jar = PathBuf::from(args.next().context("missing DATA_JAR")?);
    let catalog = PathBuf::from(args.next().context("missing ITEM_CATALOG")?);
    if args.next().is_some() {
        bail!("unexpected extra argument");
    }
    let recipes =
        RecipeBook::from_jar(&jar)?.with_item_catalog(Arc::new(ItemCatalog::from_path(&catalog)?));
    let mut furnace = Furnace::default();
    let mut current_tick = 0;
    let mut snapshots = 0;
    for line in BufReader::new(File::open(trace)?).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let scenario = row["scenario"].as_str().unwrap_or("");
        if row["type"] == "scenario_start" {
            furnace = Furnace::default();
            current_tick = 0;
            match scenario {
                "bamboo_fuel_burns_fifty_ticks" => {
                    furnace.slots[0] = Some(recipes.stack("minecraft:sand", 1));
                    furnace.slots[1] = Some(recipes.stack("minecraft:bamboo", 1));
                }
                "lava_bucket_wet_sponge_remainder" => {
                    furnace.slots[0] = Some(recipes.stack("minecraft:wet_sponge", 1));
                    furnace.slots[1] = Some(recipes.stack("minecraft:lava_bucket", 1));
                }
                _ => bail!("unexpected scenario {scenario}"),
            }
        }
        if row["type"] != "snapshot" {
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
            bail!(
                "{scenario} tick {tick}: vanilla lit={lit}, Rust lit={}",
                furnace.is_lit()
            );
        }
        let slots = observed["slots"].as_array().context("furnace slots")?;
        for (index, expected) in slots.iter().enumerate() {
            let (id, count) = furnace.slots[index]
                .as_ref()
                .map_or(("minecraft:air", 0), |stack| {
                    (stack.id.as_str(), stack.count)
                });
            if expected["id"] != id || expected["count"] != count {
                bail!(
                    "{scenario} tick {tick} slot {index}: vanilla {} x {}, Rust {id} x {count}",
                    expected["id"],
                    expected["count"]
                );
            }
        }
        snapshots += 1;
    }
    if snapshots != 262 {
        bail!("expected 262 snapshots; found {snapshots}");
    }
    println!("matched {snapshots} exact component-fuel furnace snapshots");
    Ok(())
}
