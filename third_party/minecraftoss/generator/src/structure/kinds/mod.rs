//! Structure types (vanilla `StructureType` registry).

pub mod buried_treasure;
pub mod desert_pyramid;
pub mod end_city;
pub mod fortress;
pub mod igloo;
pub mod jungle_temple;
pub mod mineshaft;
pub mod nether_fossil;
pub mod ocean_monument;
pub mod ocean_ruin;
pub mod ruined_portal;
pub mod scattered;
pub mod shipwreck;
pub mod stronghold;
pub mod swamp_hut;
pub mod template_piece;
pub mod woodland_mansion;

use super::jigsaw::JigsawStructure;
use super::{StructureKind, Unsupported};
use crate::feature::Library;
use serde_json::Value;

/// Parses one structure's type-specific configuration.
pub fn parse(lib: &mut Library, json: &Value) -> Result<Box<dyn StructureKind>, String> {
    let _ = lib;
    let kind = json["type"].as_str().ok_or("structure lacks a type")?;
    Ok(match kind.trim_start_matches("minecraft:") {
        "jigsaw" => Box::new(JigsawStructure::parse(json)?),
        "desert_pyramid" => Box::new(desert_pyramid::DesertPyramid),
        "jungle_temple" => Box::new(jungle_temple::JungleTemple),
        "swamp_hut" => Box::new(swamp_hut::SwampHut),
        "igloo" => Box::new(igloo::Igloo),
        "mineshaft" => Box::new(mineshaft::Mineshaft::parse(json)?),
        "fortress" => Box::new(fortress::Fortress),
        "end_city" => Box::new(end_city::EndCity),
        "buried_treasure" => Box::new(buried_treasure::BuriedTreasure),
        "nether_fossil" => Box::new(nether_fossil::NetherFossil::parse(json)?),
        "shipwreck" => Box::new(shipwreck::Shipwreck::parse(json)?),
        "ocean_ruin" => Box::new(ocean_ruin::OceanRuin::parse(json)?),
        "ruined_portal" => Box::new(ruined_portal::RuinedPortal::parse(json)?),
        "stronghold" => Box::new(stronghold::Stronghold),
        "ocean_monument" => Box::new(ocean_monument::OceanMonument),
        "woodland_mansion" => Box::new(woodland_mansion::WoodlandMansion),
        other => Box::new(Unsupported(other.to_owned())),
    })
}
