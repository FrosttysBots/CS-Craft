//! Pinned 26.3 honeycomb block conversion for dispenser and hand use.
use crate::Block;
use std::{collections::HashSet, sync::OnceLock};

pub fn waxed_block_id(id: &str) -> Option<String> {
    static WAXABLE: OnceLock<HashSet<&'static str>> = OnceLock::new();
    let known = WAXABLE.get_or_init(|| {
        include_str!("../data/waxable_blocks_26_3.txt")
            .lines()
            .filter(|line| line.starts_with("minecraft:"))
            .collect()
    });
    let path = id.strip_prefix("minecraft:")?;
    known
        .contains(id)
        .then(|| format!("minecraft:waxed_{path}"))
}

pub fn waxed_block(block: &Block) -> Option<Block> {
    let mut result = block.clone();
    result.id = waxed_block_id(&block.id)?;
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_waxable_boundary() {
        assert_eq!(
            waxed_block_id("minecraft:copper_block").as_deref(),
            Some("minecraft:waxed_copper_block")
        );
        assert_eq!(
            waxed_block_id("minecraft:oxidized_cut_copper_stairs").as_deref(),
            Some("minecraft:waxed_oxidized_cut_copper_stairs")
        );
        assert_eq!(waxed_block_id("minecraft:waxed_copper_block"), None);
        assert_eq!(waxed_block_id("minecraft:stone"), None);
        let stairs = Block::new("minecraft:oxidized_cut_copper_stairs")
            .with("facing", "west")
            .with("half", "top")
            .with("shape", "straight")
            .with("waterlogged", "false");
        let waxed = waxed_block(&stairs).expect("measured waxable stair");
        assert_eq!(waxed.id, "minecraft:waxed_oxidized_cut_copper_stairs");
        assert_eq!(waxed.properties, stairs.properties);
    }
}
