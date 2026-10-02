//! Biome decoration: the FEATURES chunk step (vanilla
//! `ChunkGenerator.applyBiomeDecoration`, `FeatureSorter`, `FeaturePlacer`
//! and the configured feature types).
//!
//! Source-informed from the pinned 26.3 common JAR. Placed and configured
//! features load from the data pack; every feature type is our own Rust
//! implementation of the vanilla algorithm, preserving random consumption,
//! placement order and block-update side effects that change results.

pub mod blocks;
pub mod entities;
pub mod java_set;
pub mod kinds;
pub mod placement;
pub mod post_process;
pub mod predicate;
pub mod region;
pub mod rule_test;
pub mod state;
pub mod survive;
pub mod template;
pub mod tree;
pub mod update;

use minecraftoss_core::nbt::Tag;
use minecraftoss_core::ChunkPos;
use crate::providers::GenContext;
use crate::temperature::BiomeTemperature;
use blocks::FluidType;
use kinds::Feature;
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::ident::Identifier;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::tags::TagId;
use minecraftoss_core::{BiomeId, BlockId, BlockPos, BlockStateId, Registries};
pub use placement::Placed;
pub use region::Region;
use serde_json::Value;
use state::StateProvider;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use survive::Survival;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FeatureId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlacedId(pub u32);

/// Block states features refer to by name.
#[derive(Debug)]
pub struct CommonBlocks {
    pub air: BlockStateId,
    pub cave_air: BlockStateId,
    pub water: BlockStateId,
    pub lava: BlockStateId,
    pub bedrock: BlockStateId,
    pub stone: BlockStateId,
    pub dirt: BlockStateId,
    pub snow: BlockStateId,
    pub ice: BlockStateId,
    pub packed_ice: BlockStateId,
    pub blue_ice: BlockStateId,
    pub snow_block: BlockStateId,
    pub grass_block: BlockStateId,
    pub podzol: BlockStateId,
    pub vine: BlockStateId,
    pub obsidian: BlockStateId,
    pub netherrack: BlockStateId,
    pub glowstone: BlockStateId,
}

impl CommonBlocks {
    fn load(registries: &Registries) -> Result<Self, String> {
        let s = |name: &str| registries.blocks.parse_state(name);
        Ok(Self {
            air: s("minecraft:air")?,
            cave_air: s("minecraft:cave_air")?,
            water: s("minecraft:water")?,
            lava: s("minecraft:lava")?,
            bedrock: s("minecraft:bedrock")?,
            stone: s("minecraft:stone")?,
            dirt: s("minecraft:dirt")?,
            snow: s("minecraft:snow")?,
            ice: s("minecraft:ice")?,
            packed_ice: s("minecraft:packed_ice")?,
            blue_ice: s("minecraft:blue_ice")?,
            snow_block: s("minecraft:snow_block")?,
            grass_block: s("minecraft:grass_block")?,
            podzol: s("minecraft:podzol")?,
            vine: s("minecraft:vine")?,
            obsidian: s("minecraft:obsidian")?,
            netherrack: s("minecraft:netherrack")?,
            glowstone: s("minecraft:glowstone")?,
        })
    }
}

/// Block tags features test often.
#[derive(Debug)]
pub struct CommonTags {
    pub logs: TagId,
    pub leaves: TagId,
    pub replaceable_by_trees: TagId,
    pub fire: TagId,
}

impl CommonTags {
    fn load(registries: &Registries) -> Result<Self, String> {
        let t = |name: &str| registries.block_tags.require(name);
        Ok(Self {
            logs: t("minecraft:logs")?,
            leaves: t("minecraft:leaves")?,
            replaceable_by_trees: t("minecraft:replaceable_by_trees")?,
            fire: t("minecraft:fire")?,
        })
    }
}

/// Every placed and configured feature of a data pack, parsed once.
pub struct Library {
    pub registries: Arc<Registries>,
    pub survival: Survival,
    pub update_rules: update::UpdateRules,
    pub tags: CommonTags,
    pub fire_tag: Option<TagId>,
    pub temperature: BiomeTemperature,
    pub blocks: CommonBlocks,
    /// Generation bounds of the dimension these features decorate.
    pub generation: GenContext,
    features: Vec<Feature>,
    feature_names: HashMap<String, FeatureId>,
    placed: Vec<Placed>,
    placed_names: HashMap<String, PlacedId>,
    names: HashMap<PlacedId, String>,
    providers: HashMap<String, Arc<StateProvider>>,
    tag_elements: HashMap<String, Vec<BlockId>>,
    /// Per biome: placed features per decoration step.
    biome_steps: Vec<Vec<Vec<PlacedId>>>,
    /// Per biome: every placed feature in any step (`BiomeGenerationSettings.hasFeature`).
    /// Per biome, its placed features by placed feature id.
    biome_sets: Vec<Vec<bool>>,
    /// Every block state's rotated and mirrored states.
    pub transforms: template::StateTransforms,
    pub processor_blocks: template::processor::ProcessorBlocks,
    pub templates: template::TemplateManager,
    processor_lists: HashMap<String, Arc<Vec<template::Processor>>>,
    /// The world's difficulty is peaceful: structures then create no hostile
    /// mobs, and markers that clear their block only for a created mob keep it.
    pub peaceful: bool,
    /// Cat variants and sound variants, for cats structures create.
    pub cats: entities::CatVariants,
}

/// Entity types built with `EntityType.Builder.notInPeaceful` in 26.3.
const NOT_IN_PEACEFUL: [&str; 38] = [
    "blaze", "bogged", "breeze", "cave_spider", "creaking", "creeper", "drowned", "elder_guardian", "enderman", "endermite", "evoker", "ghast",
    "giant", "guardian", "hoglin", "husk", "illusioner", "magma_cube", "parched", "phantom", "piglin_brute", "pillager", "ravager", "silverfish",
    "skeleton", "slime", "spider", "stray", "vex", "vindicator", "warden", "witch", "wither", "wither_skeleton", "zoglin", "zombie",
    "zombie_villager", "zombified_piglin",
];

impl Library {
    /// `EntityType.canSpawn`: whether `create` returns an entity at this difficulty.
    pub fn can_spawn(&self, entity: &str) -> bool {
        !self.peaceful || !NOT_IN_PEACEFUL.contains(&entity.trim_start_matches("minecraft:"))
    }

    pub fn load(registries: Arc<Registries>, generation: GenContext) -> Result<Self, String> {
        let tags = CommonTags::load(&registries)?;
        let mut lib = Self {
            peaceful: false,
            cats: entities::CatVariants::load(&registries.datapack)?,
            survival: Survival::load(&registries)?,
            update_rules: update::UpdateRules::load(&registries)?,
            fire_tag: Some(tags.fire),
            tags,
            temperature: BiomeTemperature::new(),
            blocks: CommonBlocks::load(&registries)?,
            generation,
            features: Vec::new(),
            feature_names: HashMap::new(),
            placed: Vec::new(),
            placed_names: HashMap::new(),
            names: HashMap::new(),
            providers: HashMap::new(),
            tag_elements: HashMap::new(),
            biome_steps: Vec::new(),
            biome_sets: Vec::new(),
            transforms: template::StateTransforms::build(&registries),
            processor_blocks: template::processor::ProcessorBlocks::load(&registries)?,
            templates: template::TemplateManager::default(),
            processor_lists: HashMap::new(),
            registries: registries.clone(),
        };
        for (_, info) in registries.biomes.iter() {
            let json = registries.datapack.read_json("worldgen/biome", &info.name)?;
            let mut steps = Vec::new();
            for step in json["features"].as_array().ok_or_else(|| format!("biome {} lacks features", info.name))? {
                let mut list = Vec::new();
                let entries: Vec<&Value> = match step {
                    Value::Array(a) => a.iter().collect(),
                    other => vec![other],
                };
                for entry in entries {
                    if entry.as_str().is_some_and(|s| s.starts_with('#')) {
                        return Err(format!("placed feature tags are not supported ({entry})"));
                    }
                    list.push(lib.placed_ref(entry).map_err(|e| format!("biome {}: {e}", info.name))?);
                }
                steps.push(list);
            }
            let mut members = Vec::new();
            for placed in steps.iter().flatten() {
                let i = placed.0 as usize;
                if members.len() <= i {
                    members.resize(i + 1, false);
                }
                members[i] = true;
            }
            lib.biome_sets.push(members);
            lib.biome_steps.push(steps);
        }
        // Features the game places outside decoration (`TreeGrower`), where
        // they parse; placing a missing one is reported by its caller.
        for name in GAMEPLAY_FEATURES {
            let _ = lib.feature_ref(&Value::String((*name).to_owned()));
        }
        // `GrassBlock` bone meal's grass.
        let _ = lib.placed_ref(&Value::String("minecraft:grass_bonemeal".to_owned()));
        Ok(lib)
    }

    /// Applies a `dimension_type` entry: survival rules that read light
    /// depend on whether the dimension has skylight.
    pub fn set_dimension_type(&mut self, name: &str) -> Result<(), String> {
        let json = self.registries.datapack.read_json("dimension_type", &Identifier::parse(name)?)?;
        let sky = json["has_skylight"].as_bool().ok_or_else(|| format!("dimension type {name} lacks has_skylight"))?;
        self.survival.unlit_raw_brightness = if sky { 15 } else { 0 };
        Ok(())
    }

    pub fn feature(&self, id: FeatureId) -> &Feature {
        &self.features[id.0 as usize]
    }

    pub fn placed(&self, id: PlacedId) -> &Placed {
        &self.placed[id.0 as usize]
    }

    pub fn placed_name(&self, id: PlacedId) -> &str {
        self.names.get(&id).map_or("<inline>", String::as_str)
    }

    pub fn placed_by_name(&self, name: &str) -> Option<PlacedId> {
        self.placed_names.get(name).copied()
    }

    pub fn feature_by_name(&self, name: &str) -> Option<FeatureId> {
        self.feature_names.get(name).copied()
    }

    /// A registered configured feature's name (inline features have none).
    pub fn feature_name(&self, id: FeatureId) -> Option<&str> {
        self.feature_names.iter().find(|(_, f)| **f == id).map(|(name, _)| name.as_str())
    }

    /// `PlacedFeature.getFeatures`: its feature, then that feature's
    /// sub-features' trees.
    fn placed_feature_tree(&self, placed: PlacedId, out: &mut Vec<FeatureId>) {
        let feature = self.placed(placed).feature;
        out.push(feature);
        for nested in self.feature(feature).sub_placed() {
            self.placed_feature_tree(nested, out);
        }
    }

    /// `BiomeGenerationSettings.getBoneMealFeatures`: every configured
    /// feature the biome's placed features reach, in step order, that is in
    /// the `can_spawn_from_bone_meal` tag.
    pub fn bone_meal_features(&self, biome: BiomeId) -> Vec<FeatureId> {
        let Ok(tag) = self.registries.datapack.read_json("tags/worldgen/feature", &Identifier::parse("minecraft:can_spawn_from_bone_meal").expect("valid id")) else {
            return Vec::new();
        };
        let names: Vec<&str> = tag["values"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let mut tree = Vec::new();
        for step in &self.biome_steps[usize::from(biome.0)] {
            for &placed in step {
                self.placed_feature_tree(placed, &mut tree);
            }
        }
        tree.into_iter().filter(|&f| self.feature_name(f).is_some_and(|n| names.contains(&n))).collect()
    }

    pub fn biome_has_feature(&self, biome: BiomeId, placed: PlacedId) -> bool {
        self.biome_sets[usize::from(biome.0)].get(placed.0 as usize).copied().unwrap_or(false)
    }

    /// A `Holder<ConfiguredFeature>`: a registry name or an inline feature.
    pub fn feature_ref(&mut self, json: &Value) -> Result<FeatureId, String> {
        if let Some(name) = json.as_str() {
            let key = Identifier::parse(name)?.to_string();
            if let Some(&id) = self.feature_names.get(&key) {
                return Ok(id);
            }
            let body = self.registries.datapack.read_json("worldgen/feature", &Identifier::parse(&key)?)?;
            let feature = Feature::parse(self, &body).map_err(|e| format!("feature {key}: {e}"))?;
            let id = FeatureId(self.features.len() as u32);
            self.features.push(feature);
            self.feature_names.insert(key, id);
            return Ok(id);
        }
        let feature = Feature::parse(self, json)?;
        let id = FeatureId(self.features.len() as u32);
        self.features.push(feature);
        Ok(id)
    }

    /// A `Holder<PlacedFeature>`: a registry name or an inline placed feature.
    pub fn placed_ref(&mut self, json: &Value) -> Result<PlacedId, String> {
        if let Some(name) = json.as_str() {
            let key = Identifier::parse(name)?.to_string();
            if let Some(&id) = self.placed_names.get(&key) {
                return Ok(id);
            }
            let body = self.registries.datapack.read_json("worldgen/placed_feature", &Identifier::parse(&key)?)?;
            let placed = self.parse_placed(&body).map_err(|e| format!("placed feature {key}: {e}"))?;
            let id = PlacedId(self.placed.len() as u32);
            self.placed.push(placed);
            self.placed_names.insert(key.clone(), id);
            self.names.insert(id, key);
            return Ok(id);
        }
        let placed = self.parse_placed(json)?;
        let id = PlacedId(self.placed.len() as u32);
        self.placed.push(placed);
        Ok(id)
    }

    fn parse_placed(&mut self, json: &Value) -> Result<Placed, String> {
        let feature = self.feature_ref(&json["feature"])?;
        let mut placement = Vec::new();
        for modifier in json["placement"].as_array().ok_or("placed feature lacks placement")? {
            placement.push(placement::Modifier::parse(self, modifier)?);
        }
        Ok(Placed { feature, placement })
    }

    /// A registered `worldgen/block_state_provider` entry.
    pub fn state_provider(&mut self, name: &str) -> Result<Arc<StateProvider>, String> {
        let key = Identifier::parse(name)?.to_string();
        if let Some(p) = self.providers.get(&key) {
            return Ok(p.clone());
        }
        let json = self.registries.datapack.read_json("worldgen/block_state_provider", &Identifier::parse(&key)?)?;
        let provider = StateProvider::parse(self, &json).map_err(|e| format!("block state provider {key}: {e}"))?;
        self.providers.insert(key, provider.clone());
        Ok(provider)
    }

    /// A `Holder<StructureProcessorList>`: a registry name or an inline list.
    pub fn processor_list(&mut self, json: &Value) -> Result<Arc<Vec<template::Processor>>, String> {
        if let Some(name) = json.as_str() {
            let key = Identifier::parse(name)?.to_string();
            if let Some(list) = self.processor_lists.get(&key) {
                return Ok(list.clone());
            }
            let body = self.registries.datapack.read_json("worldgen/processor_list", &Identifier::parse(&key)?)?;
            let list = Arc::new(template::Processor::parse_list(&self.registries, &body).map_err(|e| format!("processor list {key}: {e}"))?);
            self.processor_lists.insert(key, list.clone());
            return Ok(list);
        }
        Ok(Arc::new(template::Processor::parse_list(&self.registries, json)?))
    }

    /// A block tag's elements in data-pack order (`HolderSet` iteration order).
    pub fn block_tag_elements(&mut self, name: &str) -> Result<Vec<BlockId>, String> {
        let key = Identifier::parse(name.trim_start_matches('#'))?.to_string();
        if let Some(list) = self.tag_elements.get(&key) {
            return Ok(list.clone());
        }
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        self.expand_tag(&key, &mut out, &mut seen)?;
        self.tag_elements.insert(key, out.clone());
        Ok(out)
    }

    fn expand_tag(&self, name: &str, out: &mut Vec<BlockId>, seen: &mut HashSet<BlockId>) -> Result<(), String> {
        let json = self.registries.datapack.read_json("tags/block", &Identifier::parse(name)?)?;
        for value in json["values"].as_array().ok_or_else(|| format!("tag {name} lacks values"))? {
            let (entry, required) = match value {
                Value::String(s) => (s.as_str(), true),
                other => (other["id"].as_str().ok_or("bad tag entry")?, other["required"].as_bool().unwrap_or(true)),
            };
            if let Some(nested) = entry.strip_prefix('#') {
                self.expand_tag(nested, out, seen)?;
            } else if let Some(block) = self.registries.blocks.block_by_name(entry) {
                if seen.insert(block) {
                    out.push(block);
                }
            } else if required {
                return Err(format!("tag {name} names unknown block {entry}"));
            }
        }
        Ok(())
    }
}

/// `FeatureSorter.StepFeatureData`: one decoration step's global feature order.
#[derive(Debug, Default)]
pub struct StepFeatures {
    pub features: Vec<PlacedId>,
    /// Position in `features` by placed feature id (`usize::MAX`: absent).
    index: Vec<usize>,
}

/// The decoration plan of one biome source (`ChunkGenerator.featuresPerStep`).
pub struct Decoration {
    pub steps: Vec<StepFeatures>,
    /// The biome source's possible biomes, by biome id.
    possible: Vec<bool>,
}

impl Decoration {
    /// `FeatureSorter.buildFeaturesPerStep` over the biome source's possible
    /// biomes, in the source's order.
    pub fn new(lib: &Library, possible_biomes: &[BiomeId]) -> Result<Self, String> {
        // FeatureData ordered by (step, first-seen feature index).
        type Node = (usize, u32);
        let mut feature_index: HashMap<PlacedId, u32> = HashMap::new();
        let mut node_feature: HashMap<Node, PlacedId> = HashMap::new();
        let mut edges: BTreeMap<Node, BTreeSet<Node>> = BTreeMap::new();
        let mut max_step = 0;
        for &biome in possible_biomes {
            let steps = &lib.biome_steps[usize::from(biome.0)];
            max_step = max_step.max(steps.len());
            let mut list = Vec::new();
            for (step, features) in steps.iter().enumerate() {
                for &placed in features {
                    let next = feature_index.len() as u32;
                    let index = *feature_index.entry(placed).or_insert(next);
                    node_feature.insert((step, index), placed);
                    list.push((step, index));
                }
            }
            for (i, &node) in list.iter().enumerate() {
                let set = edges.entry(node).or_default();
                if let Some(&next) = list.get(i + 1) {
                    set.insert(next);
                }
            }
        }
        let mut discovered = BTreeSet::new();
        let mut visiting = BTreeSet::new();
        let mut sorted = Vec::new();
        fn dfs(edges: &BTreeMap<Node, BTreeSet<Node>>, discovered: &mut BTreeSet<Node>, visiting: &mut BTreeSet<Node>, sorted: &mut Vec<Node>, current: Node) -> bool {
            if discovered.contains(&current) {
                return false;
            }
            if !visiting.insert(current) {
                return true;
            }
            if let Some(next) = edges.get(&current) {
                for &n in next {
                    if dfs(edges, discovered, visiting, sorted, n) {
                        return true;
                    }
                }
            }
            visiting.remove(&current);
            discovered.insert(current);
            sorted.push(current);
            false
        }
        for &node in edges.keys() {
            if !discovered.contains(&node) && dfs(&edges, &mut discovered, &mut visiting, &mut sorted, node) {
                return Err("feature order cycle found".into());
            }
        }
        sorted.reverse();
        let mut steps: Vec<StepFeatures> = (0..max_step).map(|_| StepFeatures::default()).collect();
        for node in sorted {
            let step = &mut steps[node.0];
            let placed = node_feature[&node];
            let slot = placed.0 as usize;
            if step.index.len() <= slot {
                step.index.resize(slot + 1, usize::MAX);
            }
            step.index[slot] = step.features.len();
            step.features.push(placed);
        }
        let mut possible = Vec::new();
        for &biome in possible_biomes {
            let i = usize::from(biome.0);
            if possible.len() <= i {
                possible.resize(i + 1, false);
            }
            possible[i] = true;
        }
        Ok(Self { steps, possible })
    }

    /// `ChunkGenerator.applyBiomeDecoration` without structures.
    pub fn decorate(&self, lib: &Library, region: &mut Region, world_seed: i64) {
        self.decorate_with(lib, region, world_seed, None);
    }

    /// `ChunkGenerator.applyBiomeDecoration`: per step, the step's structures
    /// (each with its own feature seed) and then its features.
    pub fn decorate_with(&self, lib: &Library, region: &mut Region, world_seed: i64, structures: Option<(&crate::structure::Structures, &crate::structure::References)>) {
        let center = region.center();
        let origin = BlockPos::new(center.min_block_x(), region.min_y(), center.min_block_z());
        let mut random = WorldgenRandom::new(0);
        let decoration_seed = random.decoration_seed(world_seed, origin.x, origin.z);
        let mut possible = BTreeSet::new();
        for chunk in region.chunks() {
            for section in chunk.sections() {
                let is_possible = |biome: BiomeId| self.possible.get(usize::from(biome.0)).copied().unwrap_or(false);
                if let Some(biome) = section.biomes.single() {
                    if is_possible(biome) {
                        possible.insert(biome);
                    }
                    continue;
                }
                for i in 0..64 {
                    let biome = section.biomes.get(i);
                    if is_possible(biome) {
                        possible.insert(biome);
                    }
                }
            }
        }
        let mut ctx: Ctx = Ctx { lib, region };
        let step_count = crate::structure::STEPS.len().max(self.steps.len());
        for step_index in 0..step_count {
            if let Some((structures, references)) = structures {
                structures.place_step(&mut ctx, references, &mut random, decoration_seed, step_index);
            }
            let Some(step) = self.steps.get(step_index) else { continue };
            let mut indices = BTreeSet::new();
            for &biome in &possible {
                if let Some(features) = lib.biome_steps[usize::from(biome.0)].get(step_index) {
                    for placed in features {
                        let index = step.index[placed.0 as usize];
                        assert!(index != usize::MAX, "a possible biome's feature is in its step");
                        indices.insert(index);
                    }
                }
            }
            for index in indices {
                random.feature_seed(decoration_seed, index as i32, step_index as i32);
                placement::place(&mut ctx, &mut random, step.features[index], origin, true);
            }
        }
    }
}

/// The world block rules act on: a worldgen `Region`, or a server level.
pub trait World {
    fn block_at(&self, x: i32, y: i32, z: i32) -> BlockStateId;
    /// `LevelWriter.setBlock(pos, state, flags)`: false outside the
    /// writable area.
    fn set_block_with_flags(&mut self, lib: &Library, pos: BlockPos, state: BlockStateId, flags: u32) -> bool;
    /// `LevelAccessor.scheduleTick` for the fluid (`fluid`) or the block at a
    /// position, `delay` ticks from now.
    fn schedule(&mut self, pos: BlockPos, fluid: bool, delay: i32);
    /// `FlowingFluid.getTickDelay` for lava: 30, or 10 with fast lava.
    fn lava_tick_delay(&self) -> i32 {
        30
    }
    /// The level random (`LevelAccessor.getRandom`).
    fn random(&mut self) -> &mut minecraftoss_core::random::AnyRandom;
    /// A fluid tick for the fluid of a given state, which need not be placed
    /// yet.
    fn schedule_fluid(&mut self, pos: BlockPos, _state: BlockStateId, delay: i32) {
        self.schedule(pos, true, delay);
    }
    /// `scheduleTick(pos, block, delay)` for a given block, which need not
    /// be placed yet.
    fn schedule_block(&mut self, pos: BlockPos, _block: minecraftoss_core::BlockId, delay: i32) {
        self.schedule(pos, false, delay);
    }
    /// `getBlockTicks().hasScheduledTick(pos, block)`.
    fn has_block_tick(&self, _pos: BlockPos, _block: minecraftoss_core::BlockId) -> bool {
        false
    }
    /// A comparator's `ComparatorBlockEntity.getOutputSignal`.
    fn comparator_output(&self, _pos: BlockPos) -> i32 {
        0
    }

    // ---- level context features read; worldgen-only writes default to a
    // note on levels that do not support them -------------------------------

    /// `LevelReader.getRawBrightness(pos, darkening)` and whether the sky
    /// is visible there, where the level knows its light; world
    /// generation's regions are unlit and answer `None`.
    fn light(&self, _pos: BlockPos, _darkening: i32) -> Option<(i32, bool)> {
        None
    }
    /// The world seed (`WorldGenLevel.getSeed`).
    fn world_seed(&self) -> i64;
    /// `LevelHeightAccessor.getMinY`.
    fn min_y(&self) -> i32;
    /// `LevelHeightAccessor.getMaxY`: the highest buildable Y.
    fn max_y(&self) -> i32;
    fn is_outside_build_height(&self, y: i32) -> bool {
        y < self.min_y() || y > self.max_y()
    }
    /// `getHeight(type, x, z)`: one above the highest matching block.
    fn height_at(&self, kind: HeightmapKind, x: i32, z: i32) -> i32;
    /// Whether any `height_at` in the block rectangle is at least `y`.
    fn any_height_at_least(&self, kind: HeightmapKind, min_x: i32, min_z: i32, max_x: i32, max_z: i32, y: i32) -> bool {
        (min_x..=max_x).any(|x| (min_z..=max_z).any(|z| self.height_at(kind, x, z) >= y))
    }
    /// The biome at a block (`getBiome`), where known.
    fn biome(&self, _x: i32, _y: i32, _z: i32) -> Option<BiomeId> {
        None
    }
    /// Records something the level cannot simulate yet.
    fn note_unsupported(&mut self, _kind: &str) {}
    /// The random features use for level-owned draws (`getRandom`).
    fn level_random(&mut self) -> &mut minecraftoss_core::random::AnyRandom {
        self.random()
    }
    /// Whether a column is inside the writable area.
    /// The concrete decoration region, for hot loops that call the world
    /// often enough to want its methods inlined.
    fn as_region_mut(&mut self) -> Option<&mut Region> {
        None
    }
    fn contains(&self, _x: i32, _z: i32) -> bool {
        true
    }
    /// The region's centre chunk (structure placement only).
    fn center(&self) -> ChunkPos {
        ChunkPos::new(0, 0)
    }
    /// `BulkSectionAccess` writes (ores): raw section changes.
    fn set_block_section(&mut self, _x: i32, _y: i32, _z: i32, _state: BlockStateId) -> bool {
        self.note_unsupported("bulk section writes");
        false
    }
    fn mark_post_processing(&mut self, _x: i32, _y: i32, _z: i32) {}
    fn add_entity(&mut self, _entity: Tag) {
        self.note_unsupported("entities from features");
    }
    fn next_uuid(&mut self) -> [i32; 4] {
        [0; 4]
    }
    fn set_loot_table(&mut self, _x: i32, _y: i32, _z: i32, _table: &str, _seed: i64) {
        self.note_unsupported("loot tables from features");
    }
    fn set_brushable_loot(&mut self, _x: i32, _y: i32, _z: i32, _table: &str, _seed: i64) {
        self.note_unsupported("brushable loot from features");
    }
    fn set_spawner_entity(&mut self, _x: i32, _y: i32, _z: i32, _entity: &str) {
        self.note_unsupported("spawners from features");
    }
    fn add_bees(&mut self, _x: i32, _y: i32, _z: i32, _ticks_in_hive: &[i32]) {
        self.note_unsupported("bees from features");
    }
    fn set_gateway_exit(&mut self, _x: i32, _y: i32, _z: i32, _exit: (i32, i32, i32), _exact: bool) {
        self.note_unsupported("gateway exits from features");
    }
    fn load_block_entity(&mut self, _x: i32, _y: i32, _z: i32, _nbt: &Tag) {
        self.note_unsupported("block entities from features");
    }
}

impl World for Region {
    fn block_at(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.block(x, y, z)
    }

    /// `WorldGenRegion.setBlock`: no neighbour updates; a few blocks mark a
    /// position for post-processing.
    fn set_block_with_flags(&mut self, lib: &Library, pos: BlockPos, state: BlockStateId, flags: u32) -> bool {
        if !self.set_block(pos.x, pos.y, pos.z, state) {
            return false;
        }
        if flags & 0x10 == 0 {
            if let Some(p) = post_process_pos(lib, state, pos) {
                self.mark_post_processing(p.x, p.y, p.z);
            }
        }
        true
    }

    /// Proto chunks keep ticks with delay 0.
    fn schedule(&mut self, pos: BlockPos, fluid: bool, _delay: i32) {
        self.schedule_tick(pos.x, pos.y, pos.z, fluid);
    }

    fn has_block_tick(&self, pos: BlockPos, block: minecraftoss_core::BlockId) -> bool {
        let name = self.registries().blocks.block(block).name.clone();
        Region::has_block_tick(self, pos.x, pos.y, pos.z, name.as_str())
    }

    fn random(&mut self) -> &mut minecraftoss_core::random::AnyRandom {
        Region::level_random(self)
    }

    fn world_seed(&self) -> i64 {
        Region::world_seed(self)
    }
    fn min_y(&self) -> i32 {
        Region::min_y(self)
    }
    fn max_y(&self) -> i32 {
        Region::max_y(self)
    }
    fn is_outside_build_height(&self, y: i32) -> bool {
        Region::is_outside_build_height(self, y)
    }
    fn height_at(&self, kind: HeightmapKind, x: i32, z: i32) -> i32 {
        Region::height_at(self, kind, x, z)
    }
    fn any_height_at_least(&self, kind: HeightmapKind, min_x: i32, min_z: i32, max_x: i32, max_z: i32, y: i32) -> bool {
        Region::any_height_at_least(self, kind, min_x, min_z, max_x, max_z, y)
    }
    fn biome(&self, x: i32, y: i32, z: i32) -> Option<BiomeId> {
        Region::biome(self, x, y, z)
    }
    fn note_unsupported(&mut self, kind: &str) {
        Region::note_unsupported(self, kind)
    }
    fn level_random(&mut self) -> &mut minecraftoss_core::random::AnyRandom {
        Region::level_random(self)
    }
    fn as_region_mut(&mut self) -> Option<&mut Region> {
        Some(self)
    }
    fn contains(&self, x: i32, z: i32) -> bool {
        Region::contains(self, x, z)
    }
    fn center(&self) -> ChunkPos {
        Region::center(self)
    }
    fn set_block_section(&mut self, x: i32, y: i32, z: i32, state: BlockStateId) -> bool {
        Region::set_block_section(self, x, y, z, state)
    }
    fn mark_post_processing(&mut self, x: i32, y: i32, z: i32) {
        Region::mark_post_processing(self, x, y, z)
    }
    fn add_entity(&mut self, entity: Tag) {
        Region::add_entity(self, entity)
    }
    fn next_uuid(&mut self) -> [i32; 4] {
        Region::next_uuid(self)
    }
    fn set_loot_table(&mut self, x: i32, y: i32, z: i32, table: &str, seed: i64) {
        Region::set_loot_table(self, x, y, z, table, seed)
    }
    fn set_brushable_loot(&mut self, x: i32, y: i32, z: i32, table: &str, seed: i64) {
        Region::set_brushable_loot(self, x, y, z, table, seed)
    }
    fn set_spawner_entity(&mut self, x: i32, y: i32, z: i32, entity: &str) {
        Region::set_spawner_entity(self, x, y, z, entity)
    }
    fn add_bees(&mut self, x: i32, y: i32, z: i32, ticks_in_hive: &[i32]) {
        Region::add_bees(self, x, y, z, ticks_in_hive)
    }
    fn set_gateway_exit(&mut self, x: i32, y: i32, z: i32, exit: (i32, i32, i32), exact: bool) {
        Region::set_gateway_exit(self, x, y, z, exit, exact)
    }
    fn load_block_entity(&mut self, x: i32, y: i32, z: i32, nbt: &Tag) {
        Region::load_block_entity(self, x, y, z, nbt)
    }
}

/// Configured features placed by gameplay rather than biome decoration:
/// every `TreeGrower` tree (26.3 `TreeGrower`).
pub const GAMEPLAY_FEATURES: &[&str] = &[
    "minecraft:oak", "minecraft:fancy_oak", "minecraft:oak_bees_005", "minecraft:fancy_oak_bees_005",
    "minecraft:spruce", "minecraft:mega_spruce", "minecraft:mega_pine",
    "minecraft:mangrove", "minecraft:tall_mangrove", "minecraft:azalea_tree",
    "minecraft:birch", "minecraft:birch_bees_005",
    "minecraft:jungle_tree_no_vine", "minecraft:mega_jungle_tree",
    "minecraft:acacia", "minecraft:cherry", "minecraft:cherry_bees_005",
    "minecraft:dark_oak", "minecraft:pale_oak_bonemeal",
    "minecraft:red_poplar", "minecraft:orange_poplar", "minecraft:yellow_poplar",
    "minecraft:huge_brown_mushroom", "minecraft:huge_red_mushroom",
];

/// `BlockState.getPostProcessPos`.
fn post_process_pos(lib: &Library, state: BlockStateId, pos: BlockPos) -> Option<BlockPos> {
    let blocks = &lib.registries.blocks;
    match blocks.block(blocks.block_of(state)).name.as_str() {
        "minecraft:brown_mushroom" | "minecraft:red_mushroom" => Some(pos),
        "minecraft:soul_sand" | "minecraft:magma_block" => Some(pos.above()),
        _ => None,
    }
}

/// The level block rules run in: a feature's `WorldGenRegion` by default.
pub struct Ctx<'a, W: World + ?Sized + 'a = dyn World + 'a> {
    pub lib: &'a Library,
    pub region: &'a mut W,
}

impl<W: World + ?Sized> Ctx<'_, W> {
    pub fn registries(&self) -> &Registries {
        &self.lib.registries
    }

    pub fn block(&self, pos: BlockPos) -> BlockStateId {
        self.region.block_at(pos.x, pos.y, pos.z)
    }

    /// `LevelWriter.setBlock(pos, state, flags)`. Returns false outside the write radius.
    pub fn set_block_flags(&mut self, pos: BlockPos, state: BlockStateId, flags: u32) -> bool {
        self.region.set_block_with_flags(self.lib, pos, state, flags)
    }

    pub fn name(&self, state: BlockStateId) -> &str {
        let blocks = &self.lib.registries.blocks;
        blocks.block(blocks.block_of(state)).name.as_str()
    }

    pub fn is(&self, state: BlockStateId, name: &str) -> bool {
        self.name(state) == name
    }

    pub fn is_air(&self, state: BlockStateId) -> bool {
        self.lib.registries.blocks.is_air(state)
    }

    pub fn fluid(&self, state: BlockStateId) -> FluidType {
        blocks::Behaviour { registries: &self.lib.registries }.fluid(state)
    }

    pub fn fluid_at(&self, pos: BlockPos) -> FluidType {
        self.fluid(self.block(pos))
    }

    pub fn in_tag(&self, state: BlockStateId, tag: minecraftoss_core::tags::TagId) -> bool {
        self.lib.registries.block_in_tag(state, tag)
    }

    pub fn property(&self, state: BlockStateId, name: &str) -> Option<&str> {
        self.lib.registries.blocks.property(state, name)
    }

    pub fn with(&self, state: BlockStateId, name: &str, value: &str) -> BlockStateId {
        state::try_with(&self.lib.registries, state, name, value)
    }

    pub fn can_survive(&self, state: BlockStateId, pos: BlockPos) -> bool {
        self.lib.survival.can_survive(&self.lib.registries, self.region, state, (pos.x, pos.y, pos.z))
    }

    /// `LevelAccessor.scheduleTick` for the fluid at a position, after the
    /// fluid's tick delay (water 5, lava 30 or 10).
    /// `scheduleTick(pos, state.getFluidState().getType(), delay)` for a
    /// state that need not be placed yet (shape updates run before
    /// placement).
    pub fn schedule_fluid_tick_for(&mut self, pos: BlockPos, state: BlockStateId) {
        let delay = match self.fluid(state) {
            FluidType::Lava | FluidType::FlowingLava => self.region.lava_tick_delay(),
            _ => 5,
        };
        self.region.schedule_fluid(pos, state, delay);
    }

    pub fn schedule_fluid_tick(&mut self, pos: BlockPos) {
        let delay = match self.fluid_at(pos) {
            FluidType::Lava | FluidType::FlowingLava => self.region.lava_tick_delay(),
            _ => 5,
        };
        self.region.schedule(pos, true, delay);
    }

    /// `LevelAccessor.scheduleTick` for the block at a position after `delay` ticks.
    pub fn schedule_block_tick_in(&mut self, pos: BlockPos, delay: i32) {
        self.region.schedule(pos, false, delay);
    }
}

impl Ctx<'_> {



    /// `setBlock(pos, state, 2)`.
    pub fn set_block(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        self.set_block_flags(pos, state, 2)
    }

    /// `Feature.setBlock`: `setBlockAndUpdate`, flags 3.
    pub fn set_block_update(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        self.set_block_flags(pos, state, 3)
    }





    pub fn is_empty_block(&self, pos: BlockPos) -> bool {
        self.is_air(self.block(pos))
    }

    pub fn is_solid(&self, state: BlockStateId) -> bool {
        self.lib.registries.blocks.is(state, minecraftoss_core::block::flags::LEGACY_SOLID)
    }

    pub fn is_replaceable(&self, state: BlockStateId) -> bool {
        self.lib.registries.blocks.is(state, minecraftoss_core::block::flags::REPLACEABLE)
    }







    /// `WorldGenRegion.getHeight`: the first free Y of a column.
    pub fn height(&self, kind: HeightmapKind, x: i32, z: i32) -> i32 {
        self.region.height_at(kind, x, z)
    }

    /// Whether any `height` in a block rectangle is at least `y`.
    pub fn any_height_at_least(&self, kind: HeightmapKind, min_x: i32, min_z: i32, max_x: i32, max_z: i32, y: i32) -> bool {
        self.region.any_height_at_least(kind, min_x, min_z, max_x, max_z, y)
    }

    /// `getHeightmapPos`.
    pub fn heightmap_pos(&self, kind: HeightmapKind, pos: BlockPos) -> BlockPos {
        pos.at_y(self.height(kind, pos.x, pos.z))
    }

    pub fn biome(&self, pos: BlockPos) -> Option<BiomeId> {
        self.region.biome(pos.x, pos.y, pos.z)
    }

    /// `Biome.getTemperature` at a block.
    pub fn temperature(&self, biome: BiomeId, pos: BlockPos) -> f32 {
        let info = self.lib.registries.biomes.get(biome);
        self.lib.temperature.at(info.temperature, info.frozen_temperature_modifier, pos.x, pos.y, pos.z, self.lib.generation.sea_level)
    }

    pub fn is_outside_build_height(&self, y: i32) -> bool {
        self.region.is_outside_build_height(y)
    }

    pub fn min_y(&self) -> i32 {
        self.region.min_y()
    }

    pub fn max_y(&self) -> i32 {
        self.region.max_y()
    }


    /// `LevelAccessor.scheduleTick` for a block (worldgen: proto chunks keep delay 0).
    pub fn schedule_block_tick(&mut self, pos: BlockPos) {
        self.region.schedule(pos, false, 0);
    }

    /// `Feature.markAboveForPostProcessing`.
    pub fn mark_above_for_post_processing(&mut self, pos: BlockPos) {
        let mut p = pos;
        for _ in 0..2 {
            p = p.above();
            if self.is_air(self.block(p)) {
                return;
            }
            self.region.mark_post_processing(p.x, p.y, p.z);
        }
    }

    /// `Feature.safeSetBlock`.
    pub fn safe_set_block(&mut self, pos: BlockPos, state: BlockStateId, can_replace: impl Fn(&Self, BlockStateId) -> bool) {
        if can_replace(self, self.block(pos)) {
            self.set_block(pos, state);
        }
    }
}

/// `Feature.place` for a configured feature.
pub fn place_feature(ctx: &mut Ctx, random: &mut WorldgenRandom, id: FeatureId, pos: BlockPos) -> bool {
    let lib = ctx.lib;
    lib.feature(id).place(ctx, random, pos)
}

/// `PlacedFeature.place`: a nested placement without a biome check.
pub fn place_placed(ctx: &mut Ctx, random: &mut WorldgenRandom, id: PlacedId, pos: BlockPos) -> bool {
    placement::place(ctx, random, id, pos, false)
}

/// A map keyed by block, stored densely by block id (lookups on every
/// block placed need no hashing).
#[derive(Clone, Debug)]
pub struct BlockMap<T> {
    values: Vec<Option<T>>,
}

impl<T> Default for BlockMap<T> {
    fn default() -> Self {
        Self { values: Vec::new() }
    }
}

impl<T> BlockMap<T> {
    pub fn get(&self, block: &minecraftoss_core::block::BlockId) -> Option<&T> {
        self.values.get(usize::from(block.0)).and_then(Option::as_ref)
    }

    pub fn contains_key(&self, block: &minecraftoss_core::block::BlockId) -> bool {
        self.get(block).is_some()
    }

    pub fn insert(&mut self, block: minecraftoss_core::block::BlockId, value: T) {
        let i = usize::from(block.0);
        if self.values.len() <= i {
            self.values.resize_with(i + 1, || None);
        }
        self.values[i] = Some(value);
    }
}
