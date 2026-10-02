//! Block effects from the pinned 26.3 lightning strike path. Random nearby
//! copper cleaning and difficulty-gated fire remain separate work.

use crate::{Pos, World};

/// WeatheringCopper.getFirst applies to the struck weathering block before
/// the later random walk. It keeps shared block-state properties.
pub fn clean_struck_copper(world: &mut impl World, pos: Pos) -> Vec<Pos> {
    let Some(mut block) = world.block(pos) else {
        return Vec::new();
    };
    let Some(first) = first_weathering_copper(&block.id) else {
        return Vec::new();
    };
    if first == block.id {
        return Vec::new();
    }
    block.id = first;
    world.set_block(pos, Some(block));
    vec![pos]
}

fn first_weathering_copper(id: &str) -> Option<String> {
    let name = id.strip_prefix("minecraft:")?;
    if matches!(
        name,
        "exposed_copper" | "weathered_copper" | "oxidized_copper"
    ) {
        return Some("minecraft:copper_block".into());
    }
    let base = ["exposed_", "weathered_", "oxidized_"]
        .into_iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .unwrap_or(name);
    if matches!(
        base,
        "copper_block"
            | "cut_copper"
            | "chiseled_copper"
            | "cut_copper_slab"
            | "cut_copper_stairs"
            | "copper_door"
            | "copper_trapdoor"
            | "copper_bars"
            | "copper_grate"
            | "copper_bulb"
            | "copper_lantern"
            | "copper_chest"
            | "copper_golem_statue"
            | "lightning_rod"
            | "copper_chain"
    ) {
        Some(format!("minecraft:{base}"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weathering_collection_families_and_waxed_exclusion() {
        assert_eq!(
            first_weathering_copper("minecraft:oxidized_copper"),
            Some("minecraft:copper_block".into())
        );
        assert_eq!(
            first_weathering_copper("minecraft:weathered_copper_bulb"),
            Some("minecraft:copper_bulb".into())
        );
        assert_eq!(
            first_weathering_copper("minecraft:exposed_lightning_rod"),
            Some("minecraft:lightning_rod".into())
        );
        assert_eq!(
            first_weathering_copper("minecraft:waxed_oxidized_copper"),
            None
        );
    }
}
