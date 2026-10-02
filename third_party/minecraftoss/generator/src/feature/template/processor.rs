//! Structure processors (`StructureProcessor` types), processor rules,
//! position rule tests and block entity modifiers.
//!
//! Source-informed from the pinned 26.3 JAR (`structure.templatesystem`).
//! Random consumption follows each processor: rule processors draw from a
//! fresh `LegacyRandom` seeded by `Mth.getSeed(pos)`, block rot and block age
//! draw from the placement settings' random, and capped processors from a
//! positional fork of the world seed.

use super::{BlockInfo, PlaceSettings};
use crate::feature::blocks::{parse_state, BlockSet};
use crate::feature::placement::parse_heightmap;
use crate::feature::rule_test::RuleTest;
use crate::feature::state::{copy_properties, try_with};
use crate::feature::Ctx;
use crate::mth;
use crate::providers::IntProvider;
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::nbt::Tag;
use minecraftoss_core::pos::{Axis, Direction};
use minecraftoss_core::random::{positional_seed, LegacyRandom, RandomSource};
use minecraftoss_core::tags::TagId;
use minecraftoss_core::{BlockId, BlockPos, BlockStateId, Registries};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// `PosRuleTest`.
#[derive(Clone, Debug)]
pub enum PosRuleTest {
    AlwaysTrue,
    Linear { min_chance: f32, max_chance: f32, min_dist: i32, max_dist: i32 },
    AxisAlignedLinear { min_chance: f32, max_chance: f32, min_dist: i32, max_dist: i32, axis: Axis },
}

impl PosRuleTest {
    fn parse(json: &Value) -> Result<Self, String> {
        let kind = json["predicate_type"].as_str().ok_or("position predicate lacks predicate_type")?;
        let f = |key: &str| json.get(key).and_then(Value::as_f64).map_or(0.0, |v| v as f32);
        let i = |key: &str| json.get(key).and_then(Value::as_i64).map_or(0, |v| v as i32);
        Ok(match kind.trim_start_matches("minecraft:") {
            "always_true" => Self::AlwaysTrue,
            "linear_pos" => Self::Linear { min_chance: f("min_chance"), max_chance: f("max_chance"), min_dist: i("min_dist"), max_dist: i("max_dist") },
            "axis_aligned_linear_pos" => Self::AxisAlignedLinear {
                min_chance: f("min_chance"),
                max_chance: f("max_chance"),
                min_dist: i("min_dist"),
                max_dist: i("max_dist"),
                axis: json.get("axis").and_then(Value::as_str).and_then(Axis::from_name).unwrap_or(Axis::Y),
            },
            other => return Err(format!("unknown position predicate {other}")),
        })
    }

    fn test(&self, world_pos: BlockPos, reference: BlockPos, random: &mut impl RandomSource) -> bool {
        let chance = |dist: i32, min_dist: i32, max_dist: i32, min_chance: f32, max_chance: f32| {
            mth::clamped_lerp_f32(mth::inverse_lerp_f32(dist as f32, min_dist as f32, max_dist as f32), min_chance, max_chance)
        };
        match *self {
            Self::AlwaysTrue => true,
            Self::Linear { min_chance, max_chance, min_dist, max_dist } => {
                let dist = world_pos.dist_manhattan(reference);
                random.next_f32() <= chance(dist, min_dist, max_dist, min_chance, max_chance)
            }
            Self::AxisAlignedLinear { min_chance, max_chance, min_dist, max_dist, axis } => {
                let d = Direction::from_axis(axis, true).offset();
                let xd = ((world_pos.x - reference.x) * d.0).abs() as f32;
                let yd = ((world_pos.y - reference.y) * d.1).abs() as f32;
                let zd = ((world_pos.z - reference.z) * d.2).abs() as f32;
                let dist = (xd + yd + zd) as i32;
                random.next_f32() <= chance(dist, min_dist, max_dist, min_chance, max_chance)
            }
        }
    }
}

/// `RuleBlockEntityModifier`.
#[derive(Clone, Debug)]
pub enum BlockEntityModifier {
    Clear,
    Passthrough,
    AppendStatic(Tag),
    AppendLoot(String),
}

impl BlockEntityModifier {
    fn parse(json: &Value) -> Result<Self, String> {
        let kind = json["type"].as_str().ok_or("block entity modifier lacks type")?;
        Ok(match kind.trim_start_matches("minecraft:") {
            "clear" => Self::Clear,
            "passthrough" => Self::Passthrough,
            "append_static" => Self::AppendStatic(json_to_tag(&json["data"])),
            "append_loot" => Self::AppendLoot(json["loot_table"].as_str().ok_or("append_loot lacks loot_table")?.to_owned()),
            other => return Err(format!("unknown block entity modifier {other}")),
        })
    }

    fn apply(&self, random: &mut impl RandomSource, existing: Option<Tag>) -> Option<Tag> {
        match self {
            Self::Clear => Some(Tag::Compound(BTreeMap::new())),
            Self::Passthrough => existing,
            Self::AppendStatic(tag) => Some(match existing {
                None => tag.clone(),
                Some(mut e) => {
                    merge(&mut e, tag);
                    e
                }
            }),
            Self::AppendLoot(table) => {
                let mut map = match existing {
                    Some(Tag::Compound(map)) => map,
                    _ => BTreeMap::new(),
                };
                map.insert("LootTable".into(), Tag::String(table.clone()));
                map.insert("LootTableSeed".into(), Tag::Long(random.next_i64()));
                Some(Tag::Compound(map))
            }
        }
    }
}

/// `CompoundTag.merge`: nested compounds merge, other values replace.
fn merge(into: &mut Tag, from: &Tag) {
    let (Tag::Compound(dst), Tag::Compound(src)) = (into, from) else { return };
    for (key, value) in src {
        match (dst.get_mut(key), value) {
            (Some(existing @ Tag::Compound(_)), Tag::Compound(_)) => merge(existing, value),
            _ => {
                dst.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Static block entity data written as JSON in a data pack.
fn json_to_tag(json: &Value) -> Tag {
    match json {
        Value::Object(map) => Tag::Compound(map.iter().map(|(k, v)| (k.clone(), json_to_tag(v))).collect()),
        Value::Array(list) => Tag::List(list.iter().map(json_to_tag).collect()),
        Value::String(s) => Tag::String(s.clone()),
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(v) => Tag::Int(v as i32),
            None => Tag::Double(n.as_f64().unwrap_or(0.0)),
        },
        Value::Null => Tag::Compound(BTreeMap::new()),
    }
}

/// `ProcessorRule`.
#[derive(Clone, Debug)]
pub struct ProcessorRule {
    input: RuleTest,
    location: RuleTest,
    position: PosRuleTest,
    output: BlockStateId,
    modifier: BlockEntityModifier,
}

impl ProcessorRule {
    /// A rule written in code (`new ProcessorRule(...)`), always true at
    /// the location and position.
    pub fn new(input: RuleTest, output: BlockStateId, modifier: BlockEntityModifier) -> Self {
        Self { input, location: RuleTest::AlwaysTrue, position: PosRuleTest::AlwaysTrue, output, modifier }
    }

    fn parse(registries: &Registries, json: &Value) -> Result<Self, String> {
        Ok(Self {
            input: RuleTest::parse(registries, &json["input_predicate"])?,
            location: RuleTest::parse(registries, &json["location_predicate"])?,
            position: match json.get("position_predicate") {
                Some(p) => PosRuleTest::parse(p)?,
                None => PosRuleTest::AlwaysTrue,
            },
            output: parse_state(registries, &json["output_state"])?,
            modifier: match json.get("block_entity_modifier") {
                Some(m) => BlockEntityModifier::parse(m)?,
                None => BlockEntityModifier::Passthrough,
            },
        })
    }

    fn test(&self, ctx: &Ctx, input: BlockStateId, template_pos: BlockPos, world_pos: BlockPos, reference: BlockPos, random: &mut LegacyRandom) -> bool {
        let registries = ctx.registries();
        self.input.test(registries, input, world_pos, random)
            && (matches!(self.location, RuleTest::AlwaysTrue) || self.location.test(registries, ctx.block(world_pos), world_pos, random))
            && {
                let _ = template_pos;
                self.position.test(world_pos, reference, random)
            }
    }
}

/// Block tags and states the built-in processors refer to.
#[derive(Debug)]
pub struct ProcessorBlocks {
    stairs: TagId,
    slabs: TagId,
    walls: TagId,
    lava: BlockStateId,
    /// `StructureTemplate.placeInWorld` sets a barrier before a block entity.
    pub barrier: BlockStateId,
    structure_block: BlockId,
    air: BlockId,
    by_name: HashMap<&'static str, BlockStateId>,
}

const NAMED: &[&str] = &[
    "minecraft:stone_bricks",
    "minecraft:stone",
    "minecraft:chiseled_stone_bricks",
    "minecraft:obsidian",
    "minecraft:stone_slab",
    "minecraft:stone_brick_slab",
    "minecraft:cracked_stone_bricks",
    "minecraft:stone_brick_stairs",
    "minecraft:mossy_stone_bricks",
    "minecraft:mossy_stone_brick_stairs",
    "minecraft:mossy_stone_brick_slab",
    "minecraft:mossy_stone_brick_wall",
    "minecraft:crying_obsidian",
    "minecraft:jigsaw",
    "minecraft:structure_void",
];

/// `BlackstoneReplaceProcessor.replacements`.
const BLACKSTONE: &[(&str, &str)] = &[
    ("minecraft:cobblestone", "minecraft:blackstone"),
    ("minecraft:mossy_cobblestone", "minecraft:blackstone"),
    ("minecraft:stone", "minecraft:polished_blackstone"),
    ("minecraft:stone_bricks", "minecraft:polished_blackstone_bricks"),
    ("minecraft:mossy_stone_bricks", "minecraft:polished_blackstone_bricks"),
    ("minecraft:cobblestone_stairs", "minecraft:blackstone_stairs"),
    ("minecraft:mossy_cobblestone_stairs", "minecraft:blackstone_stairs"),
    ("minecraft:stone_stairs", "minecraft:polished_blackstone_stairs"),
    ("minecraft:stone_brick_stairs", "minecraft:polished_blackstone_brick_stairs"),
    ("minecraft:mossy_stone_brick_stairs", "minecraft:polished_blackstone_brick_stairs"),
    ("minecraft:cobblestone_slab", "minecraft:blackstone_slab"),
    ("minecraft:mossy_cobblestone_slab", "minecraft:blackstone_slab"),
    ("minecraft:smooth_stone_slab", "minecraft:polished_blackstone_slab"),
    ("minecraft:stone_slab", "minecraft:polished_blackstone_slab"),
    ("minecraft:stone_brick_slab", "minecraft:polished_blackstone_brick_slab"),
    ("minecraft:mossy_stone_brick_slab", "minecraft:polished_blackstone_brick_slab"),
    ("minecraft:stone_brick_wall", "minecraft:polished_blackstone_brick_wall"),
    ("minecraft:mossy_stone_brick_wall", "minecraft:polished_blackstone_brick_wall"),
    ("minecraft:cobblestone_wall", "minecraft:blackstone_wall"),
    ("minecraft:mossy_cobblestone_wall", "minecraft:blackstone_wall"),
    ("minecraft:chiseled_stone_bricks", "minecraft:chiseled_polished_blackstone"),
    ("minecraft:cracked_stone_bricks", "minecraft:cracked_polished_blackstone_bricks"),
    ("minecraft:iron_bars", "minecraft:iron_chain"),
];

/// Blocks whose outline shape is a full cube although their collision shape
/// is not (`LavaSubmergedBlockProcessor` tests the outline shape).
const FULL_OUTLINE: &[&str] = &["minecraft:soul_sand", "minecraft:mud", "minecraft:honey_block", "minecraft:powder_snow"];

impl ProcessorBlocks {
    pub fn load(registries: &Registries) -> Result<Self, String> {
        let t = |name: &str| registries.block_tags.require(name);
        let mut by_name = HashMap::new();
        for name in NAMED.iter().chain(BLACKSTONE.iter().flat_map(|(a, b)| [a, b])) {
            by_name.insert(*name, registries.blocks.parse_state(name)?);
        }
        Ok(Self { stairs: t("minecraft:stairs")?, slabs: t("minecraft:slabs")?, walls: t("minecraft:walls")?, lava: registries.blocks.parse_state("minecraft:lava")?,
            barrier: registries.blocks.parse_state("minecraft:barrier")?,
            structure_block: registries.blocks.block_by_name("minecraft:structure_block").ok_or("no structure block")?,
            air: registries.blocks.block_by_name("minecraft:air").ok_or("no air")?,
            by_name,
        })
    }

    /// `BlockIgnoreProcessor.STRUCTURE_BLOCK`.
    pub fn structure_block(&self) -> Vec<BlockId> {
        vec![self.structure_block]
    }

    /// `BlockIgnoreProcessor.STRUCTURE_AND_AIR`.
    pub fn structure_and_air(&self) -> Vec<BlockId> {
        vec![self.structure_block, self.air]
    }

    fn state(&self, name: &str) -> BlockStateId {
        self.by_name[name]
    }

    fn block(&self, registries: &Registries, name: &str) -> BlockId {
        registries.blocks.block_of(self.state(name))
    }
}

/// One structure processor.
#[derive(Clone, Debug)]
pub enum Processor {
    Rule(Vec<ProcessorRule>),
    BlockIgnore(Vec<BlockId>),
    BlockRot { rottable: Option<BlockSet>, integrity: f32 },
    Gravity { heightmap: HeightmapKind, offset: i32 },
    ProtectedBlocks(BlockSet),
    Capped { delegate: Box<Processor>, limit: IntProvider },
    JigsawReplacement,
    BlockAge { mossiness: f32 },
    BlackstoneReplace,
    LavaSubmerged,
    Nop,
}

impl Processor {
    /// `OceanRuinPieces.archyRuleProcessor`: up to five candidate blocks
    /// become suspicious blocks with an archaeology loot table.
    pub fn archaeology(registries: &Registries, candidate: &str, replacement: &str, loot: &str) -> Result<Self, String> {
        let block = registries.blocks.block_by_name(candidate).ok_or_else(|| format!("unknown block {candidate}"))?;
        let output = registries.blocks.parse_state(replacement)?;
        let rule = ProcessorRule::new(RuleTest::Block(block), output, BlockEntityModifier::AppendLoot(loot.to_owned()));
        Ok(Self::Capped { delegate: Box::new(Self::Rule(vec![rule])), limit: IntProvider::Constant(5) })
    }

    pub fn parse(registries: &Registries, json: &Value) -> Result<Self, String> {
        let kind = json["processor_type"].as_str().ok_or_else(|| format!("processor lacks processor_type: {json}"))?;
        Ok(match kind.trim_start_matches("minecraft:") {
            "rule" => Self::Rule(
                json["rules"].as_array().ok_or("rule processor lacks rules")?.iter().map(|r| ProcessorRule::parse(registries, r)).collect::<Result<_, _>>()?,
            ),
            "block_ignore" => Self::BlockIgnore(
                json["blocks"]
                    .as_array()
                    .ok_or("block_ignore lacks blocks")?
                    .iter()
                    .map(|b| parse_state(registries, b).map(|s| registries.blocks.block_of(s)))
                    .collect::<Result<_, _>>()?,
            ),
            "block_rot" => Self::BlockRot {
                rottable: json.get("rottable_blocks").map(|b| BlockSet::parse(registries, b)).transpose()?,
                integrity: json["integrity"].as_f64().ok_or("block_rot lacks integrity")? as f32,
            },
            "gravity" => Self::Gravity {
                heightmap: match json.get("heightmap") {
                    Some(h) => parse_heightmap(h)?,
                    None => HeightmapKind::WorldSurfaceWg,
                },
                offset: json.get("offset").and_then(Value::as_i64).map_or(0, |v| v as i32),
            },
            "protected_blocks" => Self::ProtectedBlocks(BlockSet::parse(registries, &json["value"])?),
            "capped" => Self::Capped { delegate: Box::new(Self::parse(registries, &json["delegate"])?), limit: IntProvider::parse(&json["limit"])? },
            "jigsaw_replacement" => Self::JigsawReplacement,
            "block_age" => Self::BlockAge { mossiness: json["mossiness"].as_f64().ok_or("block_age lacks mossiness")? as f32 },
            "blackstone_replace" => Self::BlackstoneReplace,
            "lava_submerged_block" => Self::LavaSubmerged,
            "nop" => Self::Nop,
            other => return Err(format!("unknown structure processor {other}")),
        })
    }

    /// A processor list: an object with `processors`, or a bare list.
    pub fn parse_list(registries: &Registries, json: &Value) -> Result<Vec<Self>, String> {
        let list = match json {
            Value::Array(list) => list,
            _ => json["processors"].as_array().ok_or_else(|| format!("invalid processor list {json}"))?,
        };
        list.iter().map(|p| Self::parse(registries, p)).collect()
    }

    /// `StructureProcessor.evaluatesEntirePieceState`.
    pub fn evaluates_entire_piece(&self) -> bool {
        matches!(self, Self::Capped { .. })
    }

    /// `StructureProcessor.processBlock`.
    #[allow(clippy::too_many_arguments)]
    pub fn process<R: RandomSource>(
        &self,
        ctx: &Ctx,
        position: BlockPos,
        reference: BlockPos,
        template_pos: BlockPos,
        info: BlockInfo,
        settings: &PlaceSettings,
        random: &mut R,
    ) -> Option<BlockInfo> {
        let registries = ctx.registries();
        let blocks = &ctx.lib.processor_blocks;
        let _ = position;
        match self {
            Self::Rule(rules) => {
                let mut rng = LegacyRandom::new(positional_seed(info.pos.x, info.pos.y, info.pos.z));
                for rule in rules {
                    if rule.test(ctx, info.state, template_pos, info.pos, reference, &mut rng) {
                        let nbt = rule.modifier.apply(&mut rng, info.nbt);
                        return Some(BlockInfo { pos: info.pos, state: rule.output, nbt });
                    }
                }
                Some(info)
            }
            Self::BlockIgnore(ignore) => (!ignore.contains(&registries.blocks.block_of(info.state))).then_some(info),
            Self::BlockRot { rottable, integrity } => {
                let rots = rottable.as_ref().is_none_or(|set| set.contains(registries, info.state));
                if rots && !(settings.random_at(random, info.pos).next_f32() <= *integrity) {
                    None
                } else {
                    Some(info)
                }
            }
            Self::Gravity { heightmap, offset } => {
                let height = ctx.height(*heightmap, info.pos.x, info.pos.z) + offset;
                Some(BlockInfo { pos: BlockPos::new(info.pos.x, height + template_pos.y, info.pos.z), ..info })
            }
            Self::ProtectedBlocks(set) => (!set.contains(registries, ctx.block(info.pos))).then_some(info),
            Self::Capped { .. } | Self::Nop => Some(info),
            Self::JigsawReplacement => {
                if registries.blocks.block_of(info.state) != blocks.block(registries, "minecraft:jigsaw") {
                    return Some(info);
                }
                if info.nbt.is_none() {
                    return Some(info);
                }
                let nbt = info.nbt.as_ref().expect("checked above");
                let text = nbt.get("final_state").and_then(Tag::as_str).unwrap_or("minecraft:air");
                let text = text.split('{').next().unwrap_or(text);
                let state = registries.blocks.parse_state(text).ok()?;
                (registries.blocks.block_of(state) != blocks.block(registries, "minecraft:structure_void")).then_some(BlockInfo { pos: info.pos, state, nbt: None })
            }
            Self::BlockAge { mossiness } => {
                let mut rng = settings.random_at(random, info.pos);
                let new = block_age(ctx, blocks, info.state, *mossiness, &mut rng);
                Some(match new {
                    Some(state) => BlockInfo { state, ..info },
                    None => info,
                })
            }
            Self::BlackstoneReplace => {
                let name = ctx.name(info.state);
                let Some((_, to)) = BLACKSTONE.iter().find(|(from, _)| *from == name) else { return Some(info) };
                let mut state = blocks.state(to);
                for property in ["facing", "half", "type"] {
                    if let Some(value) = registries.blocks.property(info.state, property) {
                        state = try_with(registries, state, property, value);
                    }
                }
                Some(BlockInfo { state, ..info })
            }
            Self::LavaSubmerged => {
                let was_lava = ctx.block(info.pos) == blocks.lava;
                let full = FULL_OUTLINE.contains(&ctx.name(info.state)) || registries.blocks.state(info.state).collision_full_block;
                Some(if was_lava && !full { BlockInfo { state: blocks.lava, ..info } } else { info })
            }
        }
    }

    /// `StructureProcessor.finalizeProcessing`.
    pub fn finalize<R: RandomSource>(
        &self,
        ctx: &Ctx,
        position: BlockPos,
        reference: BlockPos,
        original: &[BlockInfo],
        mut processed: Vec<BlockInfo>,
        settings: &PlaceSettings,
        random: &mut R,
    ) -> Vec<BlockInfo> {
        let Self::Capped { delegate, limit } = self else { return processed };
        if limit.max_inclusive() == 0 || processed.is_empty() || original.len() != processed.len() {
            return processed;
        }
        let mut seed = LegacyRandom::new(ctx.region.world_seed());
        let mut rng = seed.fork_positional().at(position.x, position.y, position.z);
        let max = limit.sample(&mut rng).min(processed.len() as i32);
        if max < 1 {
            return processed;
        }
        let mut indices: Vec<usize> = (0..processed.len()).collect();
        for i in (2..=indices.len()).rev() {
            let j = rng.next_i32_bound(i as i32) as usize;
            indices.swap(i - 1, j);
        }
        let mut replaced = 0;
        for index in indices {
            if replaced >= max {
                break;
            }
            let current = processed[index].clone();
            if let Some(altered) = delegate.process(ctx, position, reference, original[index].pos, current.clone(), settings, random) {
                if altered != current {
                    replaced += 1;
                    processed[index] = altered;
                }
            }
        }
        processed
    }
}

/// `BlockAgeProcessor.getRandomFacingStairs`: a random horizontal facing,
/// then a random half.
fn random_stairs(registries: &Registries, stairs: BlockStateId, random: &mut impl RandomSource) -> BlockStateId {
    let facing = Direction::HORIZONTAL[random.next_i32_bound(4) as usize];
    let half = if random.next_i32_bound(2) == 0 { "top" } else { "bottom" };
    try_with(registries, try_with(registries, stairs, "facing", facing.name()), "half", half)
}

/// `BlockAgeProcessor.getRandomBlock(random, nonMossy, mossy)`.
fn pick_aged(random: &mut impl RandomSource, mossiness: f32, non_mossy: [BlockStateId; 2], mossy: [BlockStateId; 2]) -> BlockStateId {
    let list = if random.next_f32() < mossiness { mossy } else { non_mossy };
    list[random.next_i32_bound(2) as usize]
}

/// `BlockAgeProcessor.processBlock`'s replacement choice.
fn block_age(ctx: &Ctx, blocks: &ProcessorBlocks, state: BlockStateId, mossiness: f32, random: &mut impl RandomSource) -> Option<BlockStateId> {
    let registries = ctx.registries();
    let is = |name: &str| registries.blocks.block_of(state) == blocks.block(registries, name);
    if is("minecraft:stone_bricks") || is("minecraft:stone") || is("minecraft:chiseled_stone_bricks") {
        if random.next_f32() >= 0.5 {
            return None;
        }
        // Both arrays are built, each drawing a random stair, before the choice.
        let non_mossy = [blocks.state("minecraft:cracked_stone_bricks"), random_stairs(registries, blocks.state("minecraft:stone_brick_stairs"), random)];
        let mossy = [blocks.state("minecraft:mossy_stone_bricks"), random_stairs(registries, blocks.state("minecraft:mossy_stone_brick_stairs"), random)];
        Some(pick_aged(random, mossiness, non_mossy, mossy))
    } else if ctx.in_tag(state, blocks.stairs) {
        if random.next_f32() >= 0.5 {
            return None;
        }
        let non_mossy = [blocks.state("minecraft:stone_slab"), blocks.state("minecraft:stone_brick_slab")];
        let mossy = [copy_properties(registries, blocks.state("minecraft:mossy_stone_brick_stairs"), state), blocks.state("minecraft:mossy_stone_brick_slab")];
        Some(pick_aged(random, mossiness, non_mossy, mossy))
    } else if ctx.in_tag(state, blocks.slabs) {
        (random.next_f32() < mossiness).then(|| copy_properties(registries, blocks.state("minecraft:mossy_stone_brick_slab"), state))
    } else if ctx.in_tag(state, blocks.walls) {
        (random.next_f32() < mossiness).then(|| copy_properties(registries, blocks.state("minecraft:mossy_stone_brick_wall"), state))
    } else if is("minecraft:obsidian") {
        (random.next_f32() < 0.15).then(|| blocks.state("minecraft:crying_obsidian"))
    } else {
        None
    }
}
