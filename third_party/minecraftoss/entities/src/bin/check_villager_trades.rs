//! Exact pinned 26.3 gate for villager offers (`scenarios/mobs/villager-trades.json`):
//! villagers of every profession, level and several types are made to build
//! their offers in a set order (`villager_offers_probe`); this replays the
//! probes in trace order through the trade book (the trade sets' named
//! random sequences carrying on across villagers) and compares every
//! villager's offers, as `MerchantOffer.CODEC` writes them, in every
//! snapshot.
use minecraftoss_entities::trading::{MerchantOffer, TradeBook, TradeSequences};
use serde_json::Value;
use std::{collections::BTreeMap, env, fs, path::Path};

fn main() {
    let mut args = env::args().skip(1);
    let trace = args.next().expect("usage: check_villager_trades TRACE.jsonl DATA.jar [ITEM_CATALOG.json]");
    let jar = args.next().expect("the pinned data JAR");
    let catalog = args.next().unwrap_or_else(|| "artifacts/item-catalog/26.3.json".to_owned());
    let book = TradeBook::from_jar(Path::new(&jar), Path::new(&catalog)).expect("trade data");
    let mut sequences = TradeSequences::default();
    // Each tagged villager's type, profession and level, and its offers
    // once made.
    let mut villagers: BTreeMap<String, (String, String, i32)> = BTreeMap::new();
    let mut offers: BTreeMap<String, Vec<MerchantOffer>> = BTreeMap::new();
    let (mut frames, mut compared, mut probes) = (0, 0, 0);
    let mut complete = false;
    for line in fs::read_to_string(&trace).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => {
                // The world seed the trade sets' random sequences start from.
                let seed = row["data"]["suite"]["seed"].as_str().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0) as u64;
                sequences = TradeSequences::new(seed);
            }
            "complete" => complete = true,
            "villager_offers_probe" => {
                let tag = row["data"]["tag"].as_str().unwrap().to_owned();
                let (kind, profession, level) = villagers.get(&tag).cloned().expect("a probed villager was seen");
                // `getOffers`: made once, from the current level's set.
                if !offers.contains_key(&tag) {
                    let made = book.villager_offers(&kind, &profession, level, &mut sequences);
                    offers.insert(tag.clone(), made);
                }
                assert_eq!(row["data"]["offers"].as_u64().unwrap() as usize, offers[&tag].len(), "tick {} {tag} offer count", row["tick"]);
                probes += 1;
            }
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let Some(entities) = row["data"]["entities"].as_object() else { continue };
                for (tag, states) in entities {
                    let Some(e) = states.as_array().and_then(|s| s.first()) else { continue };
                    if let (Some(kind), Some(profession), Some(level)) = (e["villager_type"].as_str(), e["villager_profession"].as_str(), e["villager_level"].as_i64()) {
                        villagers.insert(tag.clone(), (kind.to_owned(), profession.to_owned(), level as i32));
                    }
                    let theirs = e.get("offers").cloned();
                    let ours = offers.get(tag).map(|list| Value::Array(list.iter().map(MerchantOffer::to_json).collect()));
                    assert_eq!(normalize(ours.as_ref()), normalize(theirs.as_ref()), "{} tick {tick} {tag} offers", row["scenario"]);
                    compared += usize::from(theirs.is_some());
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    println!("{frames} exact frames matched ({probes} probes, {compared} villager offer lists)");
}

/// Offers as JSON with floats compared by their `float` value.
fn normalize(value: Option<&Value>) -> Option<Value> {
    let mut value = value?.clone();
    if let Value::Array(list) = &mut value {
        for offer in list {
            if let Some(m) = offer.get("priceMultiplier").and_then(Value::as_f64) {
                offer["priceMultiplier"] = Value::from(format!("{:08x}", (m as f32).to_bits()));
            }
        }
    }
    Some(value)
}
