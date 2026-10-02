//! Material rules: bedrock, ore veins, surfaces and underground stone
//! (vanilla 26.3 `levelgen.material`).
//!
//! Rules and conditions load from the data pack. For each chunk they compile
//! into evaluators, in vanilla's order, because compiling an ore vein
//! pre-samples density volumes through the chunk's cached context.

use crate::density::{Compiler, Context, Df, Id};
use crate::noise::{NoiseStack, NormalNoise, Volume};
use crate::providers::{Anchor, GenContext};
use crate::temperature::BiomeTemperature;
use crate::{mth, zoom};
use minecraftoss_core::random::{AnyPositional, RandomSource};
use minecraftoss_core::{BiomeId, BlockStateId, Identifier, Registries};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum Rule {
    Block(BlockStateId),
    Sequence(Vec<Rule>),
    Condition(Box<Condition>, Box<Rule>),
    Bandlands,
    OreVein { ore: BlockStateId, raw_ore: BlockStateId, filler: BlockStateId, raw_ore_chance: f32, density: Id, richness: Id, filler_gap: Id },
}

#[derive(Clone, Debug)]
pub enum Condition {
    Biome(Vec<BiomeId>),
    NoiseThreshold { noise: usize, min: f64, max: f64, is_3d: bool },
    VerticalGradient { random_name: Identifier, true_at_and_below: Anchor, false_at_and_above: Anchor },
    YAbove { anchor: Anchor, surface_depth_multiplier: i32, add_stone_depth: bool },
    Water { offset: i32, surface_depth_multiplier: i32, add_stone_depth: bool },
    Temperature,
    Steep,
    Not(Box<Condition>),
    Hole,
    AbovePreliminarySurface,
    StoneDepth { offset: i32, add_surface_depth: bool, secondary_depth_range: i32, ceiling: bool },
}

/// Parses rule and condition trees, compiling referenced density functions and noises.
pub struct Loader<'a> {
    pub registries: &'a Registries,
    pub compiler: &'a mut Compiler,
    pub noise_index: &'a mut HashMap<Identifier, usize>,
    pub noises: &'a mut Vec<NoiseStack>,
}

fn state(registries: &Registries, json: &Value) -> Result<BlockStateId, String> {
    if let Some(s) = json.as_str() {
        return registries.blocks.parse_state(s);
    }
    let name = json["Name"].as_str().ok_or("block state lacks Name")?;
    let mut s = registries.blocks.parse_state(name)?;
    if let Some(props) = json.get("Properties").and_then(Value::as_object) {
        for (k, v) in props {
            s = registries.blocks.with_property(s, k, v.as_str().ok_or("property value must be a string")?).ok_or_else(|| format!("{name} has no {k}={v}"))?;
        }
    }
    Ok(s)
}

impl Loader<'_> {
    fn density(&mut self, json: &Value) -> Result<Id, String> {
        let f: Arc<Df> = self.compiler.registry.parse(json)?;
        self.compiler.sampler(&f)
    }

    /// `RandomState.getOrCreateNoise` for material noises (shared with the density compiler's seeding).
    fn noise(&mut self, name: &Identifier) -> Result<usize, String> {
        if let Some(&i) = self.noise_index.get(name) {
            return Ok(i);
        }
        let parameters = self.compiler.registry.noise(name)?;
        let stack = NormalNoise::new(parameters).create(&mut self.compiler.random().from_hash_of(name.as_str()));
        self.noises.push(stack);
        self.noise_index.insert(name.clone(), self.noises.len() - 1);
        Ok(self.noises.len() - 1)
    }

    pub fn rule(&mut self, json: &Value) -> Result<Rule, String> {
        if let Some(name) = json.as_str() {
            let id = Identifier::parse(name)?;
            let body = self.registries.datapack.read_json("worldgen/material_rule", &id)?;
            return self.rule(&body).map_err(|e| format!("material rule {id}: {e}"));
        }
        let kind = json["type"].as_str().ok_or("material rule lacks type")?;
        Ok(match kind.strip_prefix("minecraft:").unwrap_or(kind) {
            "block" => Rule::Block(state(self.registries, &json["result_state"])?),
            "sequence" => {
                let items = json["sequence"].as_array().ok_or("sequence must be an array")?;
                if items.is_empty() {
                    return Err("need at least 1 rule for a sequence".into());
                }
                Rule::Sequence(items.iter().map(|r| self.rule(r)).collect::<Result<_, _>>()?)
            }
            "condition" => Rule::Condition(Box::new(self.condition(&json["if_true"])?), Box::new(self.rule(&json["then_run"])?)),
            "bandlands" => Rule::Bandlands,
            "ore_vein" => Rule::OreVein {
                ore: state(self.registries, &json["ore_block"])?,
                raw_ore: state(self.registries, &json["raw_ore_block"])?,
                filler: state(self.registries, &json["filler_block"])?,
                raw_ore_chance: json["raw_ore_chance"].as_f64().ok_or("ore vein lacks raw_ore_chance")? as f32,
                density: self.density(&json["density"])?,
                richness: self.density(&json["richness"])?,
                filler_gap: self.density(&json["filler_gap"])?,
            },
            other => return Err(format!("unknown material rule {other}")),
        })
    }

    fn biome_set(&self, json: &Value) -> Result<Vec<BiomeId>, String> {
        let biomes = &self.registries.biomes;
        let one = |s: &str| -> Result<Vec<BiomeId>, String> {
            if let Some(tag) = s.strip_prefix('#') {
                let t = self.registries.biome_tags.require(tag)?;
                Ok(biomes.iter().filter(|(id, _)| self.registries.biome_tags.contains(t, usize::from(id.0))).map(|(id, _)| id).collect())
            } else {
                Ok(vec![biomes.id(s).ok_or_else(|| format!("unknown biome {s}"))?])
            }
        };
        match json {
            Value::String(s) => one(s),
            Value::Array(items) => {
                let mut out = Vec::new();
                for i in items {
                    out.extend(one(i.as_str().ok_or("biome entry must be a string")?)?);
                }
                Ok(out)
            }
            other => Err(format!("invalid biome set {other}")),
        }
    }

    pub fn condition(&mut self, json: &Value) -> Result<Condition, String> {
        if let Some(name) = json.as_str() {
            let id = Identifier::parse(name)?;
            let body = self.registries.datapack.read_json("worldgen/material_condition", &id)?;
            return self.condition(&body).map_err(|e| format!("material condition {id}: {e}"));
        }
        let kind = json["type"].as_str().ok_or("material condition lacks type")?;
        let int = |k: &str| json.get(k).and_then(Value::as_i64).map(|v| v as i32).ok_or_else(|| format!("{kind} lacks {k}"));
        let boolean = |k: &str| json.get(k).and_then(Value::as_bool).ok_or_else(|| format!("{kind} lacks {k}"));
        Ok(match kind.strip_prefix("minecraft:").unwrap_or(kind) {
            "biome" => Condition::Biome(self.biome_set(&json["biome_is"])?),
            "noise_threshold" => Condition::NoiseThreshold {
                noise: self.noise(&Identifier::parse(json["noise"].as_str().ok_or("noise_threshold lacks noise")?)?)?,
                min: json["min_threshold"].as_f64().ok_or("noise_threshold lacks min_threshold")?,
                max: json["max_threshold"].as_f64().ok_or("noise_threshold lacks max_threshold")?,
                is_3d: json.get("is_3d").and_then(Value::as_bool).unwrap_or(false),
            },
            "vertical_gradient" => Condition::VerticalGradient {
                random_name: Identifier::parse(json["random_name"].as_str().ok_or("vertical_gradient lacks random_name")?)?,
                true_at_and_below: Anchor::parse(&json["true_at_and_below"])?,
                false_at_and_above: Anchor::parse(&json["false_at_and_above"])?,
            },
            "y_above" => Condition::YAbove { anchor: Anchor::parse(&json["anchor"])?, surface_depth_multiplier: int("surface_depth_multiplier")?, add_stone_depth: boolean("add_stone_depth")? },
            "water" => Condition::Water { offset: int("offset")?, surface_depth_multiplier: int("surface_depth_multiplier")?, add_stone_depth: boolean("add_stone_depth")? },
            "temperature" => Condition::Temperature,
            "steep" => Condition::Steep,
            "not" => Condition::Not(Box::new(self.condition(&json["invert"])?)),
            "hole" => Condition::Hole,
            "above_preliminary_surface" => Condition::AbovePreliminarySurface,
            "stone_depth" => Condition::StoneDepth {
                offset: int("offset")?,
                add_surface_depth: boolean("add_surface_depth")?,
                secondary_depth_range: int("secondary_depth_range")?,
                ceiling: match json["surface_type"].as_str() {
                    Some("ceiling") => true,
                    Some("floor") => false,
                    other => return Err(format!("invalid surface_type {other:?}")),
                },
            },
            other => return Err(format!("unknown material condition {other}")),
        })
    }
}

/// World-wide material state (`MaterialSystem`).
pub struct MaterialSystem {
    pub default_block: BlockStateId,
    pub sea_level: i32,
    pub preliminary_surface: Id,
    clay_bands: Vec<BlockStateId>,
    clay_bands_offset: usize,
    surface: usize,
    surface_secondary: usize,
    badlands_pillar: usize,
    badlands_pillar_roof: usize,
    badlands_surface: usize,
    iceberg_pillar: usize,
    iceberg_pillar_roof: usize,
    iceberg_surface: usize,
    noise_random: AnyPositional,
    pub noises: Vec<NoiseStack>,
    pub temperature: BiomeTemperature,
    pub snow_block: BlockStateId,
    pub packed_ice: BlockStateId,
}

impl MaterialSystem {
    pub fn new(loader: &mut Loader, default_block: BlockStateId, sea_level: i32, preliminary_surface: Id) -> Result<Self, String> {
        let mut noise = |name: &str| loader.noise(&Identifier::parse(name).expect("static noise id"));
        let clay_bands_offset = noise("minecraft:clay_bands_offset")?;
        let surface = noise("minecraft:surface")?;
        let surface_secondary = noise("minecraft:surface_secondary")?;
        let badlands_pillar = noise("minecraft:badlands_pillar")?;
        let badlands_pillar_roof = noise("minecraft:badlands_pillar_roof")?;
        let badlands_surface = noise("minecraft:badlands_surface")?;
        let iceberg_pillar = noise("minecraft:iceberg_pillar")?;
        let iceberg_pillar_roof = noise("minecraft:iceberg_pillar_roof")?;
        let iceberg_surface = noise("minecraft:iceberg_surface")?;
        let noise_random = loader.compiler.random().clone();
        let blocks = &loader.registries.blocks;
        let clay_bands = generate_bands(&mut noise_random.from_hash_of("minecraft:clay_bands"), &|s| blocks.parse_state(s).expect("vanilla terracotta"));
        Ok(Self {
            default_block,
            sea_level,
            preliminary_surface,
            clay_bands,
            clay_bands_offset,
            surface,
            surface_secondary,
            badlands_pillar,
            badlands_pillar_roof,
            badlands_surface,
            iceberg_pillar,
            iceberg_pillar_roof,
            iceberg_surface,
            noise_random,
            noises: Vec::new(),
            temperature: BiomeTemperature::new(),
            snow_block: blocks.parse_state("minecraft:snow_block")?,
            packed_ice: blocks.parse_state("minecraft:packed_ice")?,
        })
    }

    fn n(&self, index: usize) -> &NoiseStack {
        &self.noises[index]
    }

    pub fn surface_depth(&self, x: i32, z: i32) -> i32 {
        let v = f64::from(self.n(self.surface).get(f64::from(x), 0.0, f64::from(z)));
        (v * 2.75 + 3.0 + self.noise_random.at(x, 0, z).next_f64() * 0.25) as i32
    }

    pub fn surface_secondary(&self, x: i32, z: i32) -> f64 {
        f64::from(self.n(self.surface_secondary).get(f64::from(x), 0.0, f64::from(z)))
    }

    pub fn band(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        let offset = java_round(self.n(self.clay_bands_offset).get(f64::from(x), 0.0, f64::from(z)) * 4.0);
        let len = self.clay_bands.len() as i32;
        self.clay_bands[(y + offset + len).rem_euclid(len) as usize]
    }

    /// `erodedBadlandsExtension`. Returns the blocks to set.
    pub fn eroded_badlands(&self, column: &mut impl Column, x: i32, z: i32, height: i32, min_y: i32) {
        let pillar_buffer = (f64::from(self.n(self.badlands_surface).get(f64::from(x), 0.0, f64::from(z))) * 8.25).abs().min(f64::from(self.n(self.badlands_pillar).get(f64::from(x) * 0.2, 0.0, f64::from(z) * 0.2) * 15.0));
        if pillar_buffer <= 0.0 {
            return;
        }
        let pillar_floor = (f64::from(self.n(self.badlands_pillar_roof).get(f64::from(x) * 0.75, 0.0, f64::from(z) * 0.75)) * 1.5).abs();
        let top = 64.0 + (pillar_buffer * pillar_buffer * 2.5).min((pillar_floor * 50.0).ceil() + 24.0);
        let start_y = mth::floor(top);
        if height > start_y {
            return;
        }
        let mut y = start_y;
        while y >= min_y {
            let old = column.get(y);
            if column.block_of(old) == column.block_of(self.default_block) {
                break;
            }
            if column.is_water(old) {
                return;
            }
            y -= 1;
        }
        let mut y = start_y;
        while y >= min_y && column.is_air(column.get(y)) {
            column.set(y, self.default_block);
            y -= 1;
        }
    }

    /// `frozenOceanExtension`.
    #[allow(clippy::too_many_arguments)]
    pub fn frozen_ocean(&self, min_surface_level: i32, melt_slightly: bool, column: &mut impl Column, x: i32, z: i32, height: i32) {
        let iceberg = (f64::from(self.n(self.iceberg_surface).get(f64::from(x), 0.0, f64::from(z))) * 8.25).abs().min(f64::from(self.n(self.iceberg_pillar).get(f64::from(x) * 1.28, 0.0, f64::from(z) * 1.28) * 15.0));
        if iceberg <= 1.8 {
            return;
        }
        let roof = (f64::from(self.n(self.iceberg_pillar_roof).get(f64::from(x) * 1.17, 0.0, f64::from(z) * 1.17)) * 1.5).abs();
        let mut top = (iceberg * iceberg * 1.2).min((roof * 40.0).ceil() + 14.0);
        if melt_slightly {
            top -= 2.0;
        }
        if top <= 2.0 {
            return;
        }
        let bottom = f64::from(self.sea_level) - top - 7.0;
        top += f64::from(self.sea_level);
        let extension_top = top;
        let mut random = self.noise_random.at(x, 0, z);
        let max_snow_depth = 2 + random.next_i32_bound(4);
        let min_snow_height = self.sea_level + 18 + random.next_i32_bound(10);
        let mut snow_depth = 0;
        let mut y = height.max(extension_top as i32 + 1);
        while y >= min_surface_level {
            let here = column.get(y);
            let place = (column.is_air(here) && y < extension_top as i32 && random.next_f64() > 0.01)
                || (column.is_water(here) && y > bottom as i32 && y < self.sea_level && random.next_f64() > 0.15);
            if place {
                if snow_depth <= max_snow_depth && y > min_snow_height {
                    column.set(y, self.snow_block);
                    snow_depth += 1;
                } else {
                    column.set(y, self.packed_ice);
                }
            }
            y -= 1;
        }
    }
}

/// `Math.round(float)`.
fn java_round(v: f32) -> i32 {
    (f64::from(v) + 0.5).floor() as i32
}

/// `MaterialSystem.generateBands`.
fn generate_bands(random: &mut impl RandomSource, block: &dyn Fn(&str) -> BlockStateId) -> Vec<BlockStateId> {
    let terracotta = block("minecraft:terracotta");
    let mut bands = vec![terracotta; 192];
    let mut i = 0usize;
    while i < bands.len() {
        i += random.next_i32_bound(5) as usize + 1;
        if i < bands.len() {
            bands[i] = block("minecraft:orange_terracotta");
        }
        i += 1;
    }
    make_bands(random, &mut bands, 1, block("minecraft:yellow_terracotta"));
    make_bands(random, &mut bands, 2, block("minecraft:brown_terracotta"));
    make_bands(random, &mut bands, 1, block("minecraft:red_terracotta"));
    let white = block("minecraft:white_terracotta");
    let light_gray = block("minecraft:light_gray_terracotta");
    let white_count = random.next_i32_bound(15 - 9 + 1) + 9;
    let mut start = 0usize;
    let mut n = 0;
    while n < white_count && start < bands.len() {
        bands[start] = white;
        if start > 1 && random.next_bool() {
            bands[start - 1] = light_gray;
        }
        if start + 1 < bands.len() && random.next_bool() {
            bands[start + 1] = light_gray;
        }
        n += 1;
        start += random.next_i32_bound(16) as usize + 4;
    }
    bands
}

fn make_bands(random: &mut impl RandomSource, bands: &mut [BlockStateId], base_width: i32, state: BlockStateId) {
    let count = random.next_i32_bound(15 - 6 + 1) + 6;
    for _ in 0..count {
        let width = base_width + random.next_i32_bound(3);
        let start = random.next_i32_bound(bands.len() as i32) as usize;
        let mut p = 0;
        while start + p < bands.len() && (p as i32) < width {
            bands[start + p] = state;
            p += 1;
        }
    }
}

/// Column access for surface extensions (`BlockColumn`).
pub trait Column {
    fn get(&self, y: i32) -> BlockStateId;
    fn set(&mut self, y: i32, state: BlockStateId);
    fn is_air(&self, s: BlockStateId) -> bool;
    fn is_water(&self, s: BlockStateId) -> bool;
    fn block_of(&self, s: BlockStateId) -> u16;
}

/// Biome lookup for one chunk and its neighbors (the region's `BiomeManager`).
pub trait BiomeLookup {
    fn noise_biome(&self, qx: i32, qy: i32, qz: i32) -> BiomeId;
}

// ---- Per-chunk compiled evaluation (MaterialRuleContext and evaluators) ----

enum REval {
    Block(BlockStateId),
    Sequence(Vec<usize>),
    Condition(usize, usize),
    Bandlands,
    OreVein { ore: BlockStateId, raw_ore: BlockStateId, filler: BlockStateId, raw_ore_chance: f32, density: DensityGetter, richness: DensityGetter, gap: DensityGetter, random: AnyPositional },
}

struct DensityGetter {
    sampler: Id,
    prefill: Option<Vec<f32>>,
}

#[derive(Clone, Copy)]
enum Lazy {
    Y,
    Xz,
}

#[derive(Clone, Copy)]
enum CKind {
    Const(bool),
    Not(u32),
    Noise { slot: u32, min: f64, max: f64 },
    AbovePreliminary,
    Temperature,
    /// Evaluated at most once per update of `lazy`'s coordinates.
    Lazy { lazy: Lazy, last: i64, result: bool, test: LazyTest },
}

/// A lazy condition's test; biome sets and randoms live in `RuleState`.
#[derive(Clone, Copy)]
enum LazyTest {
    /// Index into `biome_sets`.
    Biome(u32),
    /// `random` indexes `gradient_randoms`.
    VerticalGradient { true_y: i32, false_y: i32, random: u32 },
    YAbove { anchor_y: i32, mult: i32, add_stone_depth: bool },
    Water { offset: i32, mult: i32, add_stone_depth: bool },
    StoneDepth { offset: i32, add_surface_depth: bool, secondary: i32, ceiling: bool },
    Steep,
    Hole,
}

struct NoiseSlot {
    noise: usize,
    is_3d: bool,
    last: i64,
    value: f64,
}

/// Compiled rules for one chunk (`MaterialRuleContext` plus its evaluators).
pub struct ChunkRules<'s> {
    rules: Vec<REval>,
    root: usize,
    /// The rule tree as a flat program (see `Op`).
    code: Vec<Op>,
    state: RuleState<'s>,
}

/// The rule tree in evaluation order. The first rule that yields a block
/// ends the whole evaluation (sequences and conditions pass a child's block
/// straight up), and a rule yielding nothing hands over to what follows it.
/// So a condition becomes a test that skips its rule when false, and the
/// program runs to the first block.
#[derive(Clone, Copy)]
enum Op {
    Test { condition: u32, skip_to: u32 },
    Block(BlockStateId),
    Bandlands,
    OreVein(u32),
}

fn emit(rules: &[REval], conditions: &[CKind], index: usize, code: &mut Vec<Op>) {
    // A condition fixed for the whole chunk (a biome test the chunk's
    // biomes decide, or its negation) needs no test at each block.
    fn constant(conditions: &[CKind], c: usize) -> Option<bool> {
        match conditions[c] {
            CKind::Const(v) => Some(v),
            CKind::Not(inner) => constant(conditions, inner as usize).map(|v| !v),
            _ => None,
        }
    }
    match &rules[index] {
        REval::Block(s) => code.push(Op::Block(*s)),
        REval::Sequence(items) => {
            for &i in items {
                emit(rules, conditions, i, code);
            }
        }
        REval::Condition(c, r) => match constant(conditions, *c) {
            Some(false) => {}
            Some(true) => emit(rules, conditions, *r, code),
            None => {
                let at = code.len();
                code.push(Op::Test { condition: *c as u32, skip_to: 0 });
                emit(rules, conditions, *r, code);
                let end = code.len() as u32;
                if let Op::Test { skip_to, .. } = &mut code[at] {
                    *skip_to = end;
                }
            }
        },
        REval::Bandlands => code.push(Op::Bandlands),
        REval::OreVein { .. } => code.push(Op::OreVein(index as u32)),
    }
}

/// Mutable evaluation state: the rule context and condition caches.
struct RuleState<'s> {
    system: &'s MaterialSystem,
    conditions: Vec<CKind>,
    /// Biome sets of biome conditions, as membership by biome id.
    biome_sets: Vec<Vec<bool>>,
    gradient_randoms: Vec<AnyPositional>,
    noise_slots: Vec<NoiseSlot>,
    expected: Volume,
    preliminary_volume: Volume,
    preliminary_buffer: Option<Vec<f32>>,
    gen_ctx: GenContext,
    zoom: zoom::ZoomCache,
    /// The last noise biome looked up, by quart.
    last_biome: Option<([i32; 3], BiomeId)>,
    // Context state.
    last_xz: i64,
    last_y: i64,
    x: i32,
    y: i32,
    z: i32,
    gradient_x: i32,
    gradient_z: i32,
    surface_depth: i32,
    secondary_last: i64,
    secondary: f64,
    min_surface_last: i64,
    min_surface: i32,
    biome: Option<BiomeId>,
    water_height: i32,
    stone_depth_below: i32,
    stone_depth_above: i32,
    /// Index of the current column's bottom in `expected`, when inside it.
    column_base: Option<usize>,
}

impl<'s> ChunkRules<'s> {
    #[allow(clippy::too_many_arguments)]
    pub fn compile(system: &'s MaterialSystem, rule: &Rule, ctx: &mut Context, expected: Volume, gen_ctx: GenContext, zoom_seed: i64, random_factories: &mut dyn FnMut(&Identifier) -> AnyPositional, possible_biomes: Option<&[BiomeId]>) -> Self {
        let start = i64::MIN + 1;
        let state = RuleState {
            system,
            conditions: Vec::new(),
            biome_sets: Vec::new(),
            gradient_randoms: Vec::new(),
            noise_slots: Vec::new(),
            preliminary_volume: Volume::blocks([expected.size[0], 1, expected.size[2]], [expected.min[0], 0, expected.min[2]]),
            expected,
            preliminary_buffer: None,
            gen_ctx,
            zoom: zoom::ZoomCache::new(zoom_seed),
            last_biome: None,
            last_xz: start,
            last_y: start,
            x: 0,
            y: 0,
            z: 0,
            gradient_x: 0,
            gradient_z: 0,
            surface_depth: 0,
            secondary_last: start - 1,
            secondary: 0.0,
            min_surface_last: start - 1,
            min_surface: 0,
            biome: None,
            water_height: 0,
            stone_depth_below: 0,
            stone_depth_above: 0,
            column_base: None,
        };
        let mut this = Self { rules: Vec::new(), root: 0, code: Vec::new(), state };
        this.root = this.compile_rule(rule, ctx, random_factories, possible_biomes);
        emit(&this.rules, &this.state.conditions, this.root, &mut this.code);
        this
    }

    pub fn update_xz(&mut self, x: i32, z: i32, gradient_x: i32, gradient_z: i32) {
        self.state.update_xz(x, z, gradient_x, gradient_z);
    }

    pub fn update_y(&mut self, stone_depth_above: i32, stone_depth_below: i32, water_height: i32, y: i32) {
        self.state.update_y(stone_depth_above, stone_depth_below, water_height, y);
    }

    pub fn min_surface_level(&mut self, ctx: &mut Context) -> i32 {
        self.state.min_surface_level(ctx)
    }

    pub fn surface_depth(&self) -> i32 {
        self.state.surface_depth
    }

    /// `RuleEvaluator.tryApply` for the root rule at the current position.
    pub fn apply(&mut self, ctx: &mut Context, biomes: &dyn BiomeLookup, registries: &Registries) -> Option<BlockStateId> {
        let mut pc = 0;
        while let Some(&op) = self.code.get(pc) {
            match op {
                Op::Test { condition, skip_to } => {
                    if !self.state.test(condition as usize, ctx, biomes, registries) {
                        pc = skip_to as usize;
                        continue;
                    }
                }
                Op::Block(s) => return Some(s),
                Op::Bandlands => return Some(self.state.system.band(self.state.x, self.state.y, self.state.z)),
                Op::OreVein(index) => {
                    if let Some(s) = self.state.ore_vein(&self.rules[index as usize], ctx) {
                        return Some(s);
                    }
                }
            }
            pc += 1;
        }
        None
    }

    fn compile_rule(&mut self, rule: &Rule, ctx: &mut Context, rf: &mut dyn FnMut(&Identifier) -> AnyPositional, possible: Option<&[BiomeId]>) -> usize {
        let eval = match rule {
            Rule::Block(s) => REval::Block(*s),
            Rule::Sequence(items) if items.len() == 1 => return self.compile_rule(&items[0], ctx, rf, possible),
            Rule::Sequence(items) => REval::Sequence(items.iter().map(|r| self.compile_rule(r, ctx, rf, possible)).collect()),
            Rule::Condition(c, r) => {
                let c = self.state.compile_condition(c, ctx, rf, possible);
                let r = self.compile_rule(r, ctx, rf, possible);
                REval::Condition(c, r)
            }
            Rule::Bandlands => REval::Bandlands,
            Rule::OreVein { ore, raw_ore, filler, raw_ore_chance, density, richness, filler_gap } => {
                let density = self.state.density_getter(ctx, *density, true);
                let richness = self.state.density_getter(ctx, *richness, true);
                let gap = self.state.density_getter(ctx, *filler_gap, false);
                let random = rf(&Identifier::parse("minecraft:ore").expect("static id"));
                REval::OreVein { ore: *ore, raw_ore: *raw_ore, filler: *filler, raw_ore_chance: *raw_ore_chance, density, richness, gap, random }
            }
        };
        self.rules.push(eval);
        self.rules.len() - 1
    }
}

impl RuleState<'_> {
    fn density_getter(&self, ctx: &mut Context, sampler: Id, prefill: bool) -> DensityGetter {
        let prefill = prefill.then(|| ctx.sample(sampler, &self.expected));
        DensityGetter { sampler, prefill }
    }

    fn compile_condition(&mut self, c: &Condition, ctx: &mut Context, rf: &mut dyn FnMut(&Identifier) -> AnyPositional, possible: Option<&[BiomeId]>) -> usize {
        // Lazy caches start one update behind the context.
        let lazy = |this: &Self, lazy, test| {
            let last = match lazy {
                Lazy::Y => this.last_y - 1,
                Lazy::Xz => this.last_xz - 1,
            };
            CKind::Lazy { lazy, last, result: false, test }
        };
        let kind = match c {
            Condition::Biome(set) => match possible {
                Some(p) if !set.iter().any(|b| p.contains(b)) => CKind::Const(false),
                Some(p) if p.iter().all(|b| set.contains(b)) => CKind::Const(true),
                _ => {
                    let size = set.iter().map(|b| usize::from(b.0) + 1).max().unwrap_or(0);
                    let mut members = vec![false; size];
                    for b in set {
                        members[usize::from(b.0)] = true;
                    }
                    self.biome_sets.push(members);
                    lazy(self, Lazy::Y, LazyTest::Biome(self.biome_sets.len() as u32 - 1))
                }
            },
            Condition::NoiseThreshold { noise, min, max, is_3d } => {
                let slot = match self.noise_slots.iter().position(|s| s.noise == *noise && s.is_3d == *is_3d) {
                    Some(i) => i,
                    None => {
                        let last = if *is_3d { self.last_y - 1 } else { self.last_xz - 1 };
                        self.noise_slots.push(NoiseSlot { noise: *noise, is_3d: *is_3d, last, value: 0.0 });
                        self.noise_slots.len() - 1
                    }
                };
                CKind::Noise { slot: slot as u32, min: *min, max: *max }
            }
            Condition::VerticalGradient { random_name, true_at_and_below, false_at_and_above } => {
                self.gradient_randoms.push(rf(random_name));
                let test = LazyTest::VerticalGradient {
                    true_y: true_at_and_below.resolve(&self.gen_ctx),
                    false_y: false_at_and_above.resolve(&self.gen_ctx),
                    random: self.gradient_randoms.len() as u32 - 1,
                };
                lazy(self, Lazy::Y, test)
            }
            Condition::YAbove { anchor, surface_depth_multiplier, add_stone_depth } => {
                lazy(self, Lazy::Y, LazyTest::YAbove { anchor_y: anchor.resolve(&self.gen_ctx), mult: *surface_depth_multiplier, add_stone_depth: *add_stone_depth })
            }
            Condition::Water { offset, surface_depth_multiplier, add_stone_depth } => {
                lazy(self, Lazy::Y, LazyTest::Water { offset: *offset, mult: *surface_depth_multiplier, add_stone_depth: *add_stone_depth })
            }
            Condition::Temperature => CKind::Temperature,
            Condition::Steep => lazy(self, Lazy::Xz, LazyTest::Steep),
            Condition::Not(inner) => CKind::Not(self.compile_condition(inner, ctx, rf, possible) as u32),
            Condition::Hole => lazy(self, Lazy::Xz, LazyTest::Hole),
            Condition::AbovePreliminarySurface => CKind::AbovePreliminary,
            Condition::StoneDepth { offset, add_surface_depth, secondary_depth_range, ceiling } => {
                lazy(self, Lazy::Y, LazyTest::StoneDepth { offset: *offset, add_surface_depth: *add_surface_depth, secondary: *secondary_depth_range, ceiling: *ceiling })
            }
        };
        self.conditions.push(kind);
        self.conditions.len() - 1
    }

    pub fn update_xz(&mut self, x: i32, z: i32, gradient_x: i32, gradient_z: i32) {
        self.last_xz += 1;
        self.last_y += 1;
        self.x = x;
        self.z = z;
        self.gradient_x = gradient_x;
        self.gradient_z = gradient_z;
        self.surface_depth = self.system.surface_depth(x, z);
        let e = &self.expected;
        let (rx, rz) = (x - e.min[0], z - e.min[2]);
        self.column_base = (e.step == [1, 1, 1] && rx >= 0 && rx < e.size[0] && rz >= 0 && rz < e.size[2]).then(|| e.index(rx, 0, rz));
    }

    pub fn update_y(&mut self, stone_depth_above: i32, stone_depth_below: i32, water_height: i32, y: i32) {
        self.last_y += 1;
        self.biome = None;
        self.y = y;
        self.water_height = water_height;
        self.stone_depth_below = stone_depth_below;
        self.stone_depth_above = stone_depth_above;
    }

    fn surface_secondary(&mut self) -> f64 {
        if self.secondary_last != self.last_xz {
            self.secondary_last = self.last_xz;
            self.secondary = self.system.surface_secondary(self.x, self.z);
        }
        self.secondary
    }

    pub fn biome(&mut self, biomes: &dyn BiomeLookup) -> BiomeId {
        if self.biome.is_none() {
            let quart = self.zoom.quart(self.x, self.y, self.z);
            let biome = match self.last_biome {
                Some((last, biome)) if last == quart => biome,
                _ => biomes.noise_biome(quart[0], quart[1], quart[2]),
            };
            self.last_biome = Some((quart, biome));
            self.biome = Some(biome);
        }
        self.biome.expect("set above")
    }

    /// `MaterialRuleContext.getMinSurfaceLevel`.
    pub fn min_surface_level(&mut self, ctx: &mut Context) -> i32 {
        if self.min_surface_last != self.last_xz {
            self.min_surface_last = self.last_xz;
            let preliminary = match self.preliminary_volume.index_of_block(self.x, 0, self.z) {
                Some(index) => {
                    if self.preliminary_buffer.is_none() {
                        self.preliminary_buffer = Some(ctx.sample(self.system.preliminary_surface, &self.preliminary_volume));
                    }
                    self.preliminary_buffer.as_ref().expect("filled above")[index]
                }
                None => ctx.value(self.system.preliminary_surface, self.x, 0, self.z),
            };
            self.min_surface = mth::floor(f64::from(preliminary)) + self.surface_depth - 8;
        }
        self.min_surface
    }

    fn density(&mut self, ctx: &mut Context, getter: &DensityGetter) -> f32 {
        if let Some(buffer) = &getter.prefill {
            let index = match self.column_base {
                // `update_xz` found the column inside `expected`.
                Some(base) => {
                    let ry = self.y - self.expected.min[1];
                    (ry >= 0 && ry < self.expected.size[1]).then(|| base + ry as usize)
                }
                None => self.expected.index_of_block(self.x, self.y, self.z),
            };
            if let Some(index) = index {
                return buffer[index];
            }
        }
        ctx.value(getter.sampler, self.x, self.y, self.z)
    }

    /// `OreVeinifier`'s rule for the current block.
    #[inline]
    fn ore_vein(&mut self, rule: &REval, ctx: &mut Context) -> Option<BlockStateId> {
        let REval::OreVein { ore, raw_ore, filler, raw_ore_chance, density, richness, gap, random } = rule else { unreachable!("an ore vein rule") };
        let d = self.density(ctx, density);
        if d <= 0.0 {
            return None;
        }
        let mut r = random.at(self.x, self.y, self.z);
        if r.next_f32() > d {
            return None;
        }
        let rich = self.density(ctx, richness);
        if r.next_f32() < rich && self.density(ctx, gap) < 0.0 {
            return Some(if r.next_f32() < *raw_ore_chance { *raw_ore } else { *ore });
        }
        Some(*filler)
    }

    fn test(&mut self, index: usize, ctx: &mut Context, biomes: &dyn BiomeLookup, registries: &Registries) -> bool {
        // By reference: the records are large, and most tests read a field or two.
        match &self.conditions[index] {
            &CKind::Const(v) => v,
            &CKind::Not(inner) => !self.test(inner as usize, ctx, biomes, registries),
            &CKind::Noise { slot, min, max } => {
                let slot = slot as usize;
                let (is_3d, last, noise) = {
                    let s = &self.noise_slots[slot];
                    (s.is_3d, s.last, s.noise)
                };
                let current = if is_3d { self.last_y } else { self.last_xz };
                if last != current {
                    let n = &self.system.noises[noise];
                    let value = if is_3d { n.get(f64::from(self.x), f64::from(self.y), f64::from(self.z)) } else { n.get(f64::from(self.x), 0.0, f64::from(self.z)) };
                    let s = &mut self.noise_slots[slot];
                    s.value = f64::from(value);
                    s.last = current;
                }
                let v = self.noise_slots[slot].value;
                v >= min && v <= max
            }
            CKind::AbovePreliminary => self.y >= self.min_surface_level(ctx),
            CKind::Temperature => {
                let biome = self.biome(biomes);
                let info = registries.biomes.get(biome);
                // Biome.coldEnoughToSnow: !warmEnoughToRain, i.e. temperature < 0.15.
                self.system.temperature.at(info.temperature, info.frozen_temperature_modifier, self.x, self.y, self.z, self.system.sea_level) < 0.15
            }
            &CKind::Lazy { lazy, last, result, test } => {
                let current = match lazy {
                    Lazy::Y => self.last_y,
                    Lazy::Xz => self.last_xz,
                };
                if last == current {
                    return result;
                }
                let value = self.compute_lazy(test, biomes);
                if let CKind::Lazy { last, result, .. } = &mut self.conditions[index] {
                    *last = current;
                    *result = value;
                }
                value
            }
        }
    }

    fn compute_lazy(&mut self, test: LazyTest, biomes: &dyn BiomeLookup) -> bool {
        match test {
            LazyTest::Biome(set) => {
                let b = self.biome(biomes);
                self.biome_sets[set as usize].get(usize::from(b.0)).copied().unwrap_or(false)
            }
            LazyTest::VerticalGradient { true_y, false_y, random } => {
                let y = self.y;
                if y <= true_y {
                    return true;
                }
                if y >= false_y {
                    return false;
                }
                let probability = mth::lerp_f64((f64::from(y) - f64::from(true_y)) / (f64::from(false_y) - f64::from(true_y)), 1.0, 0.0);
                f64::from(self.gradient_randoms[random as usize].at(self.x, y, self.z).next_f32()) < probability
            }
            LazyTest::YAbove { anchor_y, mult, add_stone_depth } => self.y + if add_stone_depth { self.stone_depth_above } else { 0 } >= anchor_y + self.surface_depth * mult,
            LazyTest::Water { offset, mult, add_stone_depth } => {
                self.water_height == i32::MIN || self.y + if add_stone_depth { self.stone_depth_above } else { 0 } >= self.water_height + offset + self.surface_depth * mult
            }
            LazyTest::StoneDepth { offset, add_surface_depth, secondary, ceiling } => {
                let depth = if ceiling { self.stone_depth_below } else { self.stone_depth_above };
                let surface = if add_surface_depth { self.surface_depth } else { 0 };
                let secondary_depth = if secondary == 0 { 0 } else { mth::lerp_f64((self.surface_secondary() + 1.0) / 2.0, 0.0, f64::from(secondary)) as i32 };
                depth <= 1 + offset + surface + secondary_depth
            }
            LazyTest::Steep => self.gradient_x <= -4 || self.gradient_z >= 4,
            LazyTest::Hole => self.surface_depth <= 0,
        }
    }
}
