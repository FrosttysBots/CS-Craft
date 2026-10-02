//! The loaded registries every engine component shares.

use crate::biome::BiomeRegistry;
use crate::block::{BlockRegistry, BlockStateId, flags};
use crate::chunk::HeightmapKind;
use crate::datapack::DataPack;
use crate::tags::Tags;
use std::path::{Path, PathBuf};

pub struct Registries {
    pub blocks: BlockRegistry,
    pub block_tags: Tags,
    pub biomes: BiomeRegistry,
    pub biome_tags: Tags,
    pub datapack: DataPack,
    /// `minecraft:air` itself, which vanilla heightmap priming skips by identity.
    pub plain_air: BlockStateId,
    /// Per state: which `HeightmapKind`s treat it as opaque (`HeightmapKind::bit`).
    heightmap_masks: Vec<u8>,
    /// Block entity types and defaults, when the catalog has been exported;
    /// without it generation keeps block entities as `DUMMY` tags.
    pub block_entities: Option<crate::block_entity::BlockEntities>,
    /// Entity default tags, when the catalog has been exported; without it
    /// generation creates no entities.
    pub entities: Option<crate::entity_data::EntityCatalog>,
    /// Default item stack sizes and fuels; without the catalog every item
    /// stacks to 64.
    pub items: crate::item::ItemCatalog,
    /// Loot tables and predicates, read from the data pack on first use.
    pub loot: crate::loot::LootTables,
}

/// Default local data locations under the repository root.
pub struct DataPaths {
    pub block_catalog: PathBuf,
    pub datapack: PathBuf,
    /// Optional: block entity defaults (`harness/export_block_entity_catalog.py`).
    pub block_entity_catalog: PathBuf,
    /// Optional: entity defaults (`harness/export_entity_catalog.py`).
    pub entity_catalog: PathBuf,
    /// Optional: default item components (`harness/export_item_catalog.py`).
    pub item_catalog: PathBuf,
}

impl DataPaths {
    pub fn under(root: &Path) -> Self {
        Self {
            block_catalog: root.join("artifacts/block-state-catalog/26.3.json"),
            datapack: root.join("datapacks/local/minecraft-26.3"),
            block_entity_catalog: root.join("artifacts/block-entity-catalog/26.3.json"),
            entity_catalog: root.join("artifacts/entity-catalog/26.3.json"),
            item_catalog: root.join("artifacts/item-catalog/26.3.json"),
        }
    }

    /// Finds the repository root from the current directory or `MINECRAFTOSS_ROOT`.
    pub fn discover() -> Result<Self, String> {
        if let Ok(root) = std::env::var("MINECRAFTOSS_ROOT") {
            return Ok(Self::under(Path::new(&root)));
        }
        let mut dir = std::env::current_dir().map_err(|e| e.to_string())?;
        loop {
            if dir.join("AGENTS.md").is_file() && dir.join("engine").is_dir() {
                return Ok(Self::under(&dir));
            }
            if !dir.pop() {
                return Err("repository root not found; set MINECRAFTOSS_ROOT".into());
            }
        }
    }
}

impl Registries {
    pub fn load(paths: &DataPaths) -> Result<Self, String> {
        let blocks = BlockRegistry::load(&paths.block_catalog).map_err(|e| {
            format!(
                "{e}\nExport it with harness/export_block_state_catalog.py (see harness/README.md)."
            )
        })?;
        let datapack = DataPack::open(&paths.datapack)
            .map_err(|e| format!("{e}\nImport it with python tools/import_datapack.py."))?;
        let block_tags = Tags::load(&datapack, "block", blocks.block_count(), |id| {
            blocks.block_by_name(id.as_str()).map(|b| usize::from(b.0))
        })?;
        let biomes = BiomeRegistry::load(&datapack)?;
        let biome_tags = Tags::load(&datapack, "worldgen/biome", biomes.len(), |id| {
            biomes.id(id.as_str()).map(|b| usize::from(b.0))
        })?;
        let plain_air = blocks.parse_state("minecraft:air")?;
        let motion = block_tags.require("minecraft:blocks_motion_in_heightmap")?;
        let motion_no_leaves =
            block_tags.require("minecraft:blocks_motion_in_heightmap_no_leaves")?;
        let heightmap_masks = (0..blocks.state_count())
            .map(|i| {
                let state = BlockStateId(i as u16);
                let info = blocks.state(state);
                let block = usize::from(info.block.0);
                let fluid = info.fluid.is_some();
                let mut mask = 0;
                if !info.has(flags::AIR) {
                    mask |= HeightmapKind::WorldSurfaceWg.bit() | HeightmapKind::WorldSurface.bit();
                }
                if block_tags.contains(motion, block) {
                    mask |= HeightmapKind::OceanFloorWg.bit() | HeightmapKind::OceanFloor.bit();
                }
                if block_tags.contains(motion, block) || fluid {
                    mask |= HeightmapKind::MotionBlocking.bit();
                }
                if block_tags.contains(motion_no_leaves, block) || fluid {
                    mask |= HeightmapKind::MotionBlockingNoLeaves.bit();
                }
                mask
            })
            .collect();
        Ok(Self {
            blocks,
            block_tags,
            biomes,
            biome_tags,
            datapack,
            plain_air,
            heightmap_masks,
            block_entities: paths
                .block_entity_catalog
                .is_file()
                .then(|| crate::block_entity::BlockEntities::load(&paths.block_entity_catalog))
                .transpose()?,
            entities: paths.entity_catalog.is_file().then(|| crate::entity_data::EntityCatalog::load(&paths.entity_catalog)).transpose()?,
            loot: crate::loot::LootTables::default(),
            items: if paths.item_catalog.is_file() { crate::item::ItemCatalog::load(&paths.item_catalog)? } else { crate::item::ItemCatalog::default() },
        })
    }

    pub fn heightmap_mask(&self, state: BlockStateId) -> u8 {
        self.heightmap_masks[usize::from(state.0)]
    }

    /// Whether a biome is in a biome tag.
    pub fn biome_in_tag(&self, biome: crate::BiomeId, tag: crate::tags::TagId) -> bool {
        self.biome_tags.contains(tag, usize::from(biome.0))
    }

    /// Whether a block state's block is in a tag, e.g. `minecraft:logs`.
    pub fn block_in_tag(&self, state: BlockStateId, tag: crate::tags::TagId) -> bool {
        self.block_tags
            .contains(tag, usize::from(self.blocks.block_of(state).0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{Chunk, HeightmapKind};
    use crate::pos::ChunkPos;

    /// Loads the real local data; skipped with a note when it has not been restored.
    pub(crate) fn local() -> Option<Registries> {
        let paths = DataPaths::discover().ok()?;
        if !paths.block_catalog.is_file() || !paths.datapack.is_dir() {
            eprintln!("skipping: local vanilla data not restored");
            return None;
        }
        Some(Registries::load(&paths).expect("local vanilla data loads"))
    }

    #[test]
    fn real_catalog_and_datapack_load() {
        let Some(r) = local() else { return };
        assert_eq!(r.blocks.state_count(), 35_723);
        assert_eq!(r.blocks.block_count(), 1_286);
        let leaves = r
            .blocks
            .parse_state("minecraft:oak_leaves[distance=3,persistent=true,waterlogged=true]")
            .unwrap();
        assert_eq!(
            r.blocks.state_to_string(leaves),
            "minecraft:oak_leaves[distance=3,persistent=true,waterlogged=true]"
        );
        let stone = r.blocks.parse_state("minecraft:stone").unwrap();
        let water = r.blocks.parse_state("minecraft:water").unwrap();
        let mask = |s| r.heightmap_mask(s);
        assert_eq!(mask(stone), 0b11_1111);
        assert_eq!(mask(water) & HeightmapKind::OceanFloor.bit(), 0);
        assert_ne!(mask(water) & HeightmapKind::MotionBlocking.bit(), 0);
        let leaves_tag = r.block_tags.require("minecraft:leaves").unwrap();
        assert!(r.block_in_tag(leaves, leaves_tag));
        // Waterlogged leaves hold water, so vanilla's fluid clause still counts them.
        assert_ne!(
            mask(leaves) & HeightmapKind::MotionBlockingNoLeaves.bit(),
            0
        );
        let dry_leaves = r
            .blocks
            .with_property(leaves, "waterlogged", "false")
            .unwrap();
        assert_eq!(
            mask(dry_leaves) & HeightmapKind::MotionBlockingNoLeaves.bit(),
            0
        );
        assert_ne!(mask(dry_leaves) & HeightmapKind::MotionBlocking.bit(), 0);
        assert_eq!(r.biomes.len(), 67);
        assert!(r.biome_tags.id("minecraft:is_ocean").is_some());
    }

    #[test]
    fn heightmaps_follow_vanilla_update_rules() {
        let Some(r) = local() else { return };
        let stone = r.blocks.parse_state("minecraft:stone").unwrap();
        let mut chunk = Chunk::new(ChunkPos::new(0, 0), -64, 384, crate::biome::BiomeId(0));
        for y in -64..10 {
            chunk.set_block_raw(3, y, 4, stone, &r);
        }
        chunk.prime_heightmaps(&HeightmapKind::FINAL, &r);
        assert_eq!(chunk.heightmaps.get(HeightmapKind::WorldSurface, 3, 4), 10);
        assert_eq!(chunk.heightmaps.get(HeightmapKind::WorldSurface, 0, 0), -64);
        // Removing the top block rescans; removing a buried block is ignored.
        chunk.set_block_raw(3, 9, 4, BlockStateId::AIR, &r);
        assert!(chunk.update_heightmap(
            HeightmapKind::WorldSurface,
            3,
            9,
            4,
            BlockStateId::AIR,
            &r
        ));
        assert_eq!(chunk.heightmaps.get(HeightmapKind::WorldSurface, 3, 4), 9);
        assert!(!chunk.update_heightmap(
            HeightmapKind::WorldSurface,
            3,
            0,
            4,
            BlockStateId::AIR,
            &r
        ));
    }
}
