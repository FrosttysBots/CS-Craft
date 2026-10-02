//! Block sound families, copied from MinecraftOSS (`engine/viewer/src/audio.rs`).

/// Default-state sound types used by the current build's placeable blocks.
/// Minecraft 26.3 Block registrations select these SoundType families.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockSoundProfile {
    pub family: &'static str,
    pub volume: f32,
    pub pitch: f32,
}

/// The values below are from the repeatable 26.3 default-state SoundType
/// catalog in scenarios/sound-type-catalog.json. Other blocks still use the
/// documented legacy family fallback until their state-dependent type is gated.
pub fn measured_block_sound_profile(id: &str) -> Option<BlockSoundProfile> {
    let (family, pitch) = match id {
        "minecraft:stone"
        | "minecraft:cobblestone"
        | "minecraft:furnace"
        | "minecraft:redstone_wire"
        | "minecraft:stone_pressure_plate" => ("stone", 1.0),
        "minecraft:grass_block" | "minecraft:oak_leaves" => ("grass", 1.0),
        "minecraft:dirt" | "minecraft:gravel" => ("gravel", 1.0),
        "minecraft:oak_planks"
        | "minecraft:oak_log"
        | "minecraft:oak_slab"
        | "minecraft:oak_stairs"
        | "minecraft:chest"
        | "minecraft:oak_pressure_plate" => ("wood", 1.0),
        "minecraft:glass" | "minecraft:redstone_lamp" => ("glass", 1.0),
        "minecraft:sand" => ("sand", 1.0),
        "minecraft:light_weighted_pressure_plate" => ("metal", 1.5),
        "minecraft:heavy_weighted_pressure_plate" => ("iron", 1.0),
        _ => return None,
    };
    Some(BlockSoundProfile {
        family,
        volume: 1.0,
        pitch,
    })
}

pub fn block_sound_family(id: &str) -> &'static str {
    if let Some(profile) = measured_block_sound_profile(id) {
        return profile.family;
    }
    let path = id.rsplit(':').next().unwrap_or(id);
    if path.contains("glass") {
        "glass"
    } else if path.contains("wool") || path.contains("carpet") {
        "wool"
    } else if path.contains("snow") {
        "snow"
    } else if path.contains("grass")
        || path.contains("leaves")
        || path.contains("moss")
        || path.contains("sapling")
    {
        "grass"
    } else if path.contains("gravel") {
        "gravel"
    } else if path.contains("sand") {
        "sand"
    } else if path.contains("deepslate") {
        "deepslate"
    } else if path.contains("iron") || path.contains("gold") || path.contains("copper") {
        "metal"
    } else if path.contains("wood")
        || path.contains("planks")
        || path.contains("log")
        || path.contains("chest")
        || path.contains("crafting_table")
        || [
            "oak_",
            "spruce_",
            "birch_",
            "jungle_",
            "acacia_",
            "dark_oak_",
            "mangrove_",
            "cherry_",
            "bamboo_",
            "pale_oak_",
        ]
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        "wood"
    } else if path == "dirt" || path == "coarse_dirt" || path == "podzol" || path == "rooted_dirt" {
        "gravel"
    } else {
        "stone"
    }
}
