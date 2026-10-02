//! Provisional renderer input. Coordinates and semantic block states are independent of stage 1 storage.
use crate::pack::ResourceId;
use crate::terrain::BlockStates;
use minecraftoss_core::Chunk;
use minecraftoss_player::{Block as PlayerBlock, World as PlayerWorld};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Block {
    pub id: ResourceId,
    pub properties: BTreeMap<String, String>,
}
impl Block {
    pub fn new(id: &str) -> Self {
        Self {
            id: ResourceId::parse(id).expect("handcrafted block id"),
            properties: BTreeMap::new(),
        }
    }
    pub fn with(mut self, key: &str, value: &str) -> Self {
        self.properties.insert(key.into(), value.into());
        self
    }
    pub fn is_opaque(&self) -> bool {
        let path = self.id.path.as_str();
        if matches!(path, "piston" | "sticky_piston")
            && self
                .properties
                .get("extended")
                .is_some_and(|value| value == "true")
        {
            return false;
        }
        !matches!(
            path,
            "air"
                | "water"
                | "lava"
                | "glass"
                | "tinted_glass"
                | "short_grass"
                | "torch"
                | "redstone_torch"
                | "redstone_wall_torch"
                | "chest"
                | "lever"
                | "redstone_wire"
                | "daylight_detector"
                | "moving_piston"
                | "piston_head"
        ) && !path.ends_with("_leaves")
            && !path.ends_with("_stained_glass")
            && !path.ends_with("_button")
    }
}
pub type BlockPos = (i32, i32, i32);
pub type ChunkPos = (i32, i32);
/// Biome data needed by the renderer. World generation can return its own
/// sample at each block position when its biome source is connected.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiomeSample {
    pub temperature: f32,
    pub downfall: f32,
    pub has_precipitation: bool,
    pub grass_color: Option<[u8; 3]>,
    pub foliage_color: Option<[u8; 3]>,
    pub water_color: [u8; 3],
    pub sky_color: Option<[u8; 3]>,
    pub fog_color: Option<[u8; 3]>,
}
impl BiomeSample {
    pub const THE_VOID: Self = Self {
        temperature: 0.5,
        downfall: 0.5,
        has_precipitation: false,
        grass_color: None,
        foliage_color: None,
        water_color: [63, 118, 228],
        sky_color: Some([123, 164, 255]),
        fog_color: None,
    };
}
pub trait Scene {
    fn block(&self, pos: BlockPos) -> Option<&Block>;
    fn chunks(&self) -> Vec<ChunkPos>;
    fn vertical_range(&self) -> std::ops::Range<i32>;
    fn revision(&self) -> u64;
    fn biome_at(&self, _pos: BlockPos) -> BiomeSample {
        BiomeSample::THE_VOID
    }
    /// The fluid at a position (`FluidCell::from_block` of its block).
    fn fluid_at(&self, pos: BlockPos) -> Option<crate::fluid::FluidCell> {
        crate::fluid::FluidCell::from_block(self.block(pos)?)
    }
    /// Whether the block at a position darkens the corners of the faces
    /// beside it (`getShadeBrightness` below 1). Scenes without collision
    /// shapes guess from block names.
    fn shade_darkens_at(&self, pos: BlockPos) -> bool {
        self.block(pos).is_some_and(|block| block.is_opaque() || block.id.path.ends_with("_leaves"))
    }
    /// Whether the block at a position stops fluid faces
    /// (`fluid::full_collision`).
    fn full_collision_at(&self, pos: BlockPos) -> bool {
        crate::fluid::full_collision(self.block(pos))
    }
}

#[derive(Clone, Default)]
pub struct HandcraftedScene {
    blocks: BTreeMap<ChunkPos, Arc<BTreeMap<BlockPos, Block>>>,
    /// A streamed world's generated chunks, as the server sent them. Edits in
    /// `blocks` and `cleared` sit on top and outlive the chunk being dropped.
    generated: HashMap<ChunkPos, Arc<Chunk>>,
    /// Positions over generated terrain that were set to air.
    cleared: BTreeMap<ChunkPos, Arc<BTreeSet<BlockPos>>>,
    states: Option<Arc<BlockStates>>,
    revision: u64,
}
impl HandcraftedScene {
    pub fn new() -> Self {
        let mut s = Self::default();
        // Four full chunks; every placement is fixed and authored here, never seeded.
        for x in -16..32 {
            for z in -16..32 {
                let h = if (12..19).contains(&x) && (7..15).contains(&z) {
                    2
                } else if (7..23).contains(&x) && (3..20).contains(&z) {
                    1
                } else {
                    0
                };
                s.set((x, -2, z), Some(Block::new("minecraft:stone")));
                s.set((x, -1, z), Some(Block::new("minecraft:dirt")));
                for y in 0..h {
                    s.set((x, y, z), Some(Block::new("minecraft:dirt")));
                }
                s.set(
                    (x, h, z),
                    Some(Block::new("minecraft:grass_block").with("snowy", "false")),
                );
            }
        }
        // Stone footpath crossing a chunk seam, with a small timber hut.
        for x in -16..20 {
            for z in -1..=1 {
                s.set((x, 0, z), Some(Block::new("minecraft:cobblestone")));
            }
        }
        for x in -8..=-3 {
            for z in 5..=10 {
                s.set((x, 1, z), Some(Block::new("minecraft:oak_planks")));
                if x == -8 || x == -3 || z == 5 || z == 10 {
                    for y in 2..=4 {
                        s.set(
                            (x, y, z),
                            Some(if (x == -8 || x == -3) && (z == 5 || z == 10) {
                                Block::new("minecraft:oak_log").with("axis", "y")
                            } else {
                                Block::new("minecraft:oak_planks")
                            }),
                        );
                    }
                }
                s.set((x, 5, z), Some(Block::new("minecraft:oak_planks")));
            }
        }
        s.set((-5, 2, 5), None);
        s.set((-5, 3, 5), None);
        for (x, z) in [(-8, 7), (-3, 7), (-6, 10)] {
            s.set((x, 3, z), Some(Block::new("minecraft:glass")));
        }
        // One tree with distinct opaque trunk and cutout canopy.
        for y in 1..=5 {
            s.set(
                (5, y, -7),
                Some(Block::new("minecraft:oak_log").with("axis", "y")),
            );
        }
        for x in 3i32..=7 {
            for z in -9i32..=-5 {
                for y in 5i32..=7 {
                    if (x - 5).abs() + (z + 7).abs() + (y - 6).abs() <= 4 {
                        s.set(
                            (x, y, z),
                            Some(
                                Block::new("minecraft:oak_leaves")
                                    // The trunk ends at Y=4 after the canopy overwrites Y=5.
                                    // Use the stable vanilla leaf distance from that log.
                                    .with(
                                        "distance",
                                        &((x - 5).abs() + (z + 7).abs() + y - 4).to_string(),
                                    )
                                    .with("persistent", "true")
                                    .with("waterlogged", "false"),
                            ),
                        );
                    }
                }
            }
        }
        for x in -12..=-8 {
            for z in -9..=-5 {
                s.set((x, 0, z), Some(Block::new("minecraft:sand")));
                if (-11..=-9).contains(&x) && (-8..=-6).contains(&z) {
                    s.set(
                        (x, 0, z),
                        Some(Block::new("minecraft:water").with("level", "0")),
                    );
                }
            }
        }
        for (x, id) in [
            (25, "minecraft:stone"),
            (26, "minecraft:bricks"),
            (27, "minecraft:glass"),
            (28, "minecraft:gold_block"),
            (29, "minecraft:oak_planks"),
        ] {
            for y in 1..=3 {
                s.set((x, y, 4), Some(Block::new(id)));
            }
        }
        s
    }
    /// An empty world whose terrain arrives chunk by chunk.
    pub fn streamed(states: Arc<BlockStates>) -> Self {
        Self {
            states: Some(states),
            ..Self::default()
        }
    }
    pub fn states(&self) -> Option<&Arc<BlockStates>> {
        self.states.as_ref()
    }
    pub fn is_streamed(&self) -> bool {
        self.states.is_some()
    }
    pub fn insert_chunk(&mut self, chunk: Arc<Chunk>) {
        self.generated.insert((chunk.pos.x, chunk.pos.z), chunk);
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn remove_chunk(&mut self, pos: ChunkPos) {
        if self.generated.remove(&pos).is_some() {
            self.revision = self.revision.wrapping_add(1);
        }
    }
    /// Whether a 16x16x16 section holds no block (`Scene::block` is `None`
    /// everywhere in it), read from the chunk storage and edit overlays.
    pub fn section_is_empty(&self, (sx, sy, sz): (i32, i32, i32)) -> bool {
        let chunk = (sx, sz);
        let (y0, y1) = (sy * 16, sy * 16 + 15);
        if self.blocks.get(&chunk).is_some_and(|placed| placed.keys().any(|&(_, y, _)| (y0..=y1).contains(&y))) {
            return false;
        }
        let (Some(generated), Some(states)) = (self.generated.get(&chunk), self.states.as_ref()) else {
            return true;
        };
        let Some(section) = generated.sections().get((sy - generated.min_section_y()) as usize) else {
            return true;
        };
        if section.is_empty() {
            return true;
        }
        let cleared = self.cleared.get(&chunk);
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    let state = section.block(x, y, z);
                    if states.block(state).is_none() {
                        continue;
                    }
                    let pos = (sx * 16 + x as i32, y0 + y as i32, sz * 16 + z as i32);
                    if !cleared.is_some_and(|c| c.contains(&pos)) {
                        return false;
                    }
                }
            }
        }
        true
    }
    pub fn generated_chunk(&self, pos: ChunkPos) -> Option<&Arc<Chunk>> {
        self.generated.get(&pos)
    }
    pub fn generated_chunks(&self) -> impl Iterator<Item = ChunkPos> + '_ {
        self.generated.keys().copied()
    }
    /// Edits over one chunk: placed blocks, then positions cleared to air.
    pub fn chunk_edits(
        &self,
        pos: ChunkPos,
    ) -> (
        Option<&Arc<BTreeMap<BlockPos, Block>>>,
        Option<&Arc<BTreeSet<BlockPos>>>,
    ) {
        (self.blocks.get(&pos), self.cleared.get(&pos))
    }
    pub fn set(&mut self, pos: BlockPos, block: Option<Block>) {
        let chunk = (pos.0.div_euclid(16), pos.2.div_euclid(16));
        if let Some(block) = block {
            Arc::make_mut(self.blocks.entry(chunk).or_default()).insert(pos, block);
            if let Some(cleared) = self.cleared.get_mut(&chunk) {
                Arc::make_mut(cleared).remove(&pos);
                if cleared.is_empty() {
                    self.cleared.remove(&chunk);
                }
            }
        } else {
            if let Some(blocks) = self.blocks.get_mut(&chunk) {
                Arc::make_mut(blocks).remove(&pos);
                if blocks.is_empty() {
                    self.blocks.remove(&chunk);
                }
            }
            if self.generated_block(pos).is_some() {
                Arc::make_mut(self.cleared.entry(chunk).or_default()).insert(pos);
            }
        }
        self.revision = self.revision.wrapping_add(1);
    }
    /// A block's sound type from the catalog (none for an authored scene or
    /// an older catalog).
    pub fn sound_type(&self, block: &Block) -> Option<minecraftoss_core::block::SoundType> {
        self.states.as_ref()?.sound_type(block).cloned()
    }

    /// The sound type of the block named `id` in its default state.
    pub fn sound_type_of(&self, id: &str) -> Option<minecraftoss_core::block::SoundType> {
        self.states.as_ref()?.sound_type_of(id).cloned()
    }

    /// The block state at a position of a streamed world: an edit's state,
    /// air where an edit cleared, else the generated chunk's. None where no
    /// catalog applies (an authored scene) or the chunk is not loaded.
    pub fn state_at(&self, pos: BlockPos) -> Option<minecraftoss_core::BlockStateId> {
        let states = self.states.as_ref()?;
        let chunk = (pos.0.div_euclid(16), pos.2.div_euclid(16));
        if let Some(block) = self.blocks.get(&chunk).and_then(|blocks| blocks.get(&pos)) {
            return states.state_of(block);
        }
        if self.cleared.get(&chunk).is_some_and(|cleared| cleared.contains(&pos)) {
            return Some(minecraftoss_core::BlockStateId::AIR);
        }
        let generated = self.generated.get(&chunk)?;
        Some(generated.block((pos.0 & 15) as usize, pos.1, (pos.2 & 15) as usize))
    }
    fn generated_block(&self, pos: BlockPos) -> Option<&Block> {
        let chunk = self.generated.get(&(pos.0 >> 4, pos.2 >> 4))?;
        let state = chunk.block((pos.0 & 15) as usize, pos.1, (pos.2 & 15) as usize);
        self.states.as_ref()?.block(state)
    }
    pub fn block_count(&self) -> usize {
        self.blocks.values().map(|chunk| chunk.len()).sum()
    }
    pub fn positions_with_block(&self, id: &str) -> Vec<BlockPos> {
        self.blocks
            .values()
            .flat_map(|chunk| chunk.iter())
            .filter_map(|(&pos, block)| (block.id.key() == id).then_some(pos))
            .collect()
    }
}
impl Scene for HandcraftedScene {
    fn block(&self, pos: BlockPos) -> Option<&Block> {
        let chunk = (pos.0.div_euclid(16), pos.2.div_euclid(16));
        if let Some(block) = self.blocks.get(&chunk).and_then(|blocks| blocks.get(&pos)) {
            return Some(block);
        }
        if self.generated.is_empty()
            || self
                .cleared
                .get(&chunk)
                .is_some_and(|cleared| cleared.contains(&pos))
        {
            return None;
        }
        self.generated_block(pos)
    }
    fn shade_darkens_at(&self, pos: BlockPos) -> bool {
        let Some(block) = Scene::block(self, pos) else { return false };
        match &self.states {
            Some(states) => states.shade_darkens(block),
            None => block.is_opaque() || block.id.path.ends_with("_leaves"),
        }
    }
    fn chunks(&self) -> Vec<ChunkPos> {
        let mut chunks: Vec<ChunkPos> = self.blocks.keys().copied().collect();
        if !self.generated.is_empty() {
            chunks.extend(self.generated.keys().copied());
            chunks.sort_unstable();
            chunks.dedup();
        }
        chunks
    }
    fn vertical_range(&self) -> std::ops::Range<i32> {
        if let Some(states) = &self.states {
            return states.vertical_range();
        }
        -2..self
            .blocks
            .values()
            .flat_map(|chunk| chunk.keys().map(|(_, y, _)| *y))
            .max()
            .unwrap_or(7)
            .max(7)
            + 1
    }
    fn revision(&self) -> u64 {
        self.revision
    }
    fn biome_at(&self, pos: BlockPos) -> BiomeSample {
        if let Some(states) = &self.states {
            return states.biome_at(pos, |chunk| self.generated.get(&chunk).map(Arc::as_ref));
        }
        // The authored comparison map is temperate and receives precipitation.
        // Its existing color inputs stay fixed until the generator biome source is wired in.
        BiomeSample {
            has_precipitation: true,
            ..BiomeSample::THE_VOID
        }
    }
}
impl HandcraftedScene {
    /// The noise biome at a quart position of a generated world.
    pub fn noise_biome(&self, quart: (i32, i32, i32)) -> Option<minecraftoss_core::BiomeId> {
        let states = self.states.as_ref()?;
        Some(states.noise_biome(quart, |chunk| self.generated.get(&chunk).map(Arc::as_ref)))
    }
}
impl PlayerWorld for HandcraftedScene {
    fn block(&self, pos: BlockPos) -> Option<PlayerBlock> {
        Scene::block(self, pos).map(|b| PlayerBlock {
            id: b.id.key(),
            properties: b.properties.clone(),
        })
    }
    fn set_block(&mut self, pos: BlockPos, block: Option<PlayerBlock>) {
        self.set(
            pos,
            block.map(|b| Block {
                id: ResourceId::parse(&b.id).expect("player block id"),
                properties: b.properties,
            }),
        );
    }
    /// A streamed world's blocks collide with their exact catalog shapes.
    fn collision_boxes(&self, pos: BlockPos) -> Vec<[f64; 6]> {
        if let (Some(states), Some(state)) = (&self.states, self.state_at(pos)) {
            return states.registries().blocks.collision_boxes(state);
        }
        PlayerWorld::block(self, pos).map_or_else(Vec::new, |block| minecraftoss_player::authored_collision_boxes(&block))
    }
}
#[cfg(test)]
mod collision_tests {
    use super::*;

    /// Streamed worlds collide with the block catalog's exact shapes: a
    /// flower has none, a fence reaches 1.5 blocks, a bottom slab half one.
    #[test]
    fn streamed_blocks_collide_with_catalog_shapes() {
        use minecraftoss_core::registries::DataPaths;
        use minecraftoss_core::Registries;
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let states = Arc::new(crate::terrain::BlockStates::new(Arc::new(registries), 0, -64, 384).unwrap());
        let mut scene = HandcraftedScene::streamed(states);
        scene.set((0, 100, 0), Some(Block::new("minecraft:poppy")));
        scene.set((1, 100, 0), Some(Block::new("minecraft:oak_fence")));
        scene.set((2, 100, 0), Some(Block::new("minecraft:stone_slab").with("type", "bottom")));
        scene.set((3, 100, 0), Some(Block::new("minecraft:stone")));
        assert!(PlayerWorld::collision_boxes(&scene, (0, 100, 0)).is_empty(), "flowers do not collide");
        let fence = PlayerWorld::collision_boxes(&scene, (1, 100, 0));
        assert!(fence.iter().any(|b| b[4] == 1.5), "{fence:?}");
        assert_eq!(PlayerWorld::collision_boxes(&scene, (2, 100, 0)), vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]);
        assert_eq!(PlayerWorld::collision_boxes(&scene, (3, 100, 0)), vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]);
    }
}

#[cfg(test)]
mod tests {
    /// `section_is_empty` agrees with reading every block of the section,
    /// with placed blocks and cleared positions over generated terrain.
    #[test]
    fn section_emptiness_matches_block_reads() {
        use minecraftoss_core::registries::DataPaths;
        use minecraftoss_core::Registries;
        use minecraftoss_generator::terrain::TerrainGenerator;
        use minecraftoss_world::chunk_map::{ChunkMap, WorldGen};
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let registries = std::sync::Arc::new(registries);
        let worldgen = std::sync::Arc::new(WorldGen::new(std::sync::Arc::new(TerrainGenerator::overworld(registries.clone(), 0).unwrap())).unwrap());
        let states = std::sync::Arc::new(crate::terrain::BlockStates::new(registries.clone(), 0, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen, 2, 2);
        let mut scene = HandcraftedScene::streamed(states);
        scene.insert_chunk(map.load_now(minecraftoss_core::ChunkPos::new(0, 0)));
        let slow = |scene: &HandcraftedScene, (sx, sy, sz): (i32, i32, i32)| {
            (0..16).all(|x| (0..16).all(|y| (0..16).all(|z| Scene::block(scene, (sx * 16 + x, sy * 16 + y, sz * 16 + z)).is_none())))
        };
        // Clear a whole section of terrain, then place one block high up.
        for x in 0..16 {
            for y in -64..-48 {
                for z in 0..16 {
                    scene.set((x, y, z), None);
                }
            }
        }
        scene.set((3, 250, 3), Some(Block::new("minecraft:stone")));
        for sy in -4..20 {
            assert_eq!(scene.section_is_empty((0, sy, 0)), slow(&scene, (0, sy, 0)), "section {sy}");
        }
        assert!(scene.section_is_empty((0, -4, 0)), "the cleared section is empty");
        assert!(!scene.section_is_empty((0, 15, 0)), "the placed block keeps its section");
    }

    use super::*;
    #[test]
    fn handcrafted_spans_chunks() {
        let s = HandcraftedScene::new();
        assert_eq!(s.chunks().len(), 9);
        assert!(s.block_count() > 4000);
    }

    #[test]
    fn cloned_scene_edits_only_the_affected_chunk() {
        let source = HandcraftedScene::new();
        let before = source.block_count();
        let mut edited = source.clone();
        edited.set((0, 10, 0), Some(Block::new("minecraft:glass")));
        assert!(Scene::block(&source, (0, 10, 0)).is_none());
        assert_eq!(edited.block_count(), before + 1);
        assert_eq!(source.block_count(), before);
        assert!(Arc::ptr_eq(
            source.blocks.get(&(1, 1)).unwrap(),
            edited.blocks.get(&(1, 1)).unwrap()
        ));
        assert!(!Arc::ptr_eq(
            source.blocks.get(&(0, 0)).unwrap(),
            edited.blocks.get(&(0, 0)).unwrap()
        ));
    }

    #[test]
    fn glass_and_leaf_families_do_not_occlude_full_faces() {
        for id in [
            "minecraft:oak_leaves",
            "minecraft:birch_leaves",
            "minecraft:glass",
            "minecraft:blue_stained_glass",
            "minecraft:tinted_glass",
        ] {
            assert!(!Block::new(id).is_opaque(), "{id}");
        }
        assert!(Block::new("minecraft:stone").is_opaque());
    }
}
