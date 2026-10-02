//! Compare direct struck-block cleaning against repeatable vanilla traces.
use anyhow::{bail, Context, Result};
use minecraftoss_player::{lightning::clean_struck_copper, Block, Pos, World};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs};

#[derive(Default)]
struct CopperWorld(BTreeMap<Pos, Block>);

impl World for CopperWorld {
    fn block(&self, pos: Pos) -> Option<Block> {
        self.0.get(&pos).cloned()
    }
    fn set_block(&mut self, pos: Pos, block: Option<Block>) {
        if let Some(block) = block {
            self.0.insert(pos, block);
        } else {
            self.0.remove(&pos);
        }
    }
}

fn main() -> Result<()> {
    let paths: Vec<_> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        bail!("usage: lightning_copper_trace FIRST_TRACE.jsonl [SECOND_TRACE.jsonl ...]");
    }
    for path in paths {
        let contents = fs::read_to_string(&path).with_context(|| path.clone())?;
        let mut scenarios: BTreeMap<String, BTreeMap<u64, Value>> = BTreeMap::new();
        for line in contents.lines() {
            let row: Value = serde_json::from_str(line)?;
            let Some(scenario) = row["scenario"].as_str() else {
                continue;
            };
            if row["type"] == "snapshot"
                && matches!(
                    scenario,
                    "lightning_copper_direct_clean"
                        | "lightning_oxidized_copper_bulb"
                        | "lightning_weathered_cut_copper_stairs"
                        | "lightning_waxed_oxidized_copper_bulb"
                )
            {
                scenarios.entry(scenario.to_owned()).or_default().insert(
                    row["tick"].as_u64().context("missing tick")?,
                    row["data"]["blocks"]["0,81,0"].clone(),
                );
            }
        }
        if scenarios.is_empty() {
            bail!("{path}: no supported copper snapshots");
        }
        for (scenario, expected) in scenarios {
            let final_tick = if scenario == "lightning_copper_direct_clean" {
                5
            } else {
                4
            };
            if !expected.keys().copied().eq(0..=final_tick) {
                bail!("{path}/{scenario}: missing copper snapshots");
            }
            let initial = expected.get(&0).context("missing initial block")?;
            let block = Block {
                id: initial["id"]
                    .as_str()
                    .context("missing block id")?
                    .to_owned(),
                properties: serde_json::from_value(initial["properties"].clone())?,
            };
            let pos = (0, 81, 0);
            let mut world = CopperWorld::default();
            world.set_block(pos, Some(block));
            for tick in 0..=final_tick {
                if tick == 2 {
                    clean_struck_copper(&mut world, pos);
                }
                let block = world.block(pos).context("missing copper block")?;
                let actual = json!({"id": block.id, "properties": block.properties});
                if expected.get(&tick) != Some(&actual) {
                    bail!(
                        "{path}/{scenario}: mismatch at tick {tick}: vanilla={:?}, Rust={actual}",
                        expected.get(&tick)
                    );
                }
            }
            println!(
                "{path}/{scenario}: {} exact snapshots match",
                expected.len()
            );
        }
    }
    Ok(())
}
