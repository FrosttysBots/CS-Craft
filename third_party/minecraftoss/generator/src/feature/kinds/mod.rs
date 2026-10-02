//! Configured feature types (vanilla `FeatureTypes`).

pub mod cave;
pub mod misc;
pub mod multiface;
pub mod nether;
pub mod ore;
pub mod plant;
pub mod sculk;
pub mod simple;
pub mod terrain;
pub mod the_end;

use super::{Ctx, Library};
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::BlockPos;
use serde_json::Value;

/// One configured feature.
#[derive(Debug)]
pub enum Feature {
    NoOp,
    SimpleBlock(simple::SimpleBlock),
    BlockColumn(simple::BlockColumn),
    BlockPile(simple::BlockPile),
    RandomSelector(simple::RandomSelector),
    SimpleRandomSelector(simple::SimpleRandomSelector),
    WeightedRandomSelector(simple::WeightedRandomSelector),
    RandomBooleanSelector(simple::RandomBooleanSelector),
    Sequence(simple::Sequence),
    Overlay(simple::Overlay),
    SingleBlockPillar(simple::SingleBlockPillar),
    ProjectedRandomPatchySquare(simple::ProjectedRandomPatchySquare),
    RandomNeighborSpread(simple::RandomNeighborSpread),
    FillLayer(simple::FillLayer),
    BlueIce,
    FreezeTopLayer,
    Vines,
    MultifaceGrowth(multiface::MultifaceGrowth),
    Ore(ore::Ore),
    ScatteredOre(ore::ScatteredOre),
    ReplaceBlobs(ore::ReplaceBlobs),
    UnderwaterMagma(ore::UnderwaterMagma),
    Tree(Box<super::tree::Tree>),
    FallenTree(Box<super::tree::FallenTree>),
    Other(Box<dyn Placeable>),
    /// A feature type this engine does not place yet.
    Unsupported(String),
}

/// A feature type implemented in its own module.
pub trait Placeable: std::fmt::Debug + Send + Sync {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, pos: BlockPos) -> bool;
}

impl Feature {
    /// `Feature.getSubFeatures`: the placed features selectors, sequences
    /// and overlays hold.
    pub fn sub_placed(&self) -> Vec<crate::feature::PlacedId> {
        match self {
            Self::RandomSelector(f) => f.nested(),
            Self::SimpleRandomSelector(f) => f.nested(),
            Self::WeightedRandomSelector(f) => f.nested(),
            Self::RandomBooleanSelector(f) => f.nested(),
            Self::Sequence(f) => f.nested(),
            Self::Overlay(f) => f.nested(),
            _ => Vec::new(),
        }
    }

    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let kind = json["type"].as_str().ok_or_else(|| format!("feature lacks a type: {json}"))?;
        let kind = kind.trim_start_matches("minecraft:");
        let other = |p: Box<dyn Placeable>| Ok(Self::Other(p));
        match kind {
            "no_op" => Ok(Self::NoOp),
            "simple_block" => Ok(Self::SimpleBlock(simple::SimpleBlock::parse(lib, json)?)),
            "block_column" => Ok(Self::BlockColumn(simple::BlockColumn::parse(lib, json)?)),
            "block_pile" => Ok(Self::BlockPile(simple::BlockPile::parse(lib, json)?)),
            "random_selector" => Ok(Self::RandomSelector(simple::RandomSelector::parse(lib, json)?)),
            "simple_random_selector" => Ok(Self::SimpleRandomSelector(simple::SimpleRandomSelector::parse(lib, json)?)),
            "weighted_random_selector" => Ok(Self::WeightedRandomSelector(simple::WeightedRandomSelector::parse(lib, json)?)),
            "random_boolean_selector" => Ok(Self::RandomBooleanSelector(simple::RandomBooleanSelector::parse(lib, json)?)),
            "sequence" => Ok(Self::Sequence(simple::Sequence::parse(lib, json)?)),
            "overlay" => Ok(Self::Overlay(simple::Overlay::parse(lib, json)?)),
            "single_block_pillar" => Ok(Self::SingleBlockPillar(simple::SingleBlockPillar::parse(lib, json)?)),
            "projected_random_patchy_square" => Ok(Self::ProjectedRandomPatchySquare(simple::ProjectedRandomPatchySquare::parse(lib, json)?)),
            "random_neighbor_spread" => Ok(Self::RandomNeighborSpread(simple::RandomNeighborSpread::parse(lib, json)?)),
            "fill_layer" => Ok(Self::FillLayer(simple::FillLayer::parse(lib, json)?)),
            "blue_ice" => Ok(Self::BlueIce),
            "freeze_top_layer" => Ok(Self::FreezeTopLayer),
            "vines" => Ok(Self::Vines),
            "multiface_growth" => Ok(Self::MultifaceGrowth(multiface::MultifaceGrowth::parse(lib, json)?)),
            "ore" => Ok(Self::Ore(ore::Ore::parse(lib, json)?)),
            "scattered_ore" => Ok(Self::ScatteredOre(ore::ScatteredOre::parse(lib, json)?)),
            "netherrack_replace_blobs" => Ok(Self::ReplaceBlobs(ore::ReplaceBlobs::parse(lib, json)?)),
            "underwater_magma" => Ok(Self::UnderwaterMagma(ore::UnderwaterMagma::parse(lib, json)?)),
            "tree" => Ok(Self::Tree(Box::new(super::tree::Tree::parse(lib, json)?))),
            "fallen_tree" => Ok(Self::FallenTree(Box::new(super::tree::FallenTree::parse(lib, json)?))),
            _ => match terrain::parse(lib, kind, json)
                .or_else(|| cave::parse(lib, kind, json))
                .or_else(|| plant::parse(lib, kind, json))
                .or_else(|| nether::parse(lib, kind, json))
                .or_else(|| the_end::parse(lib, kind, json))
                .or_else(|| sculk::parse(lib, kind, json))
                .or_else(|| misc::parse(lib, kind, json))
            {
                Some(parsed) => other(parsed?),
                None => Ok(Self::Unsupported(kind.to_owned())),
            },
        }
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, pos: BlockPos) -> bool {
        match self {
            Self::NoOp => true,
            Self::SimpleBlock(f) => f.place(ctx, random, pos),
            Self::BlockColumn(f) => f.place(ctx, random, pos),
            Self::BlockPile(f) => f.place(ctx, random, pos),
            Self::RandomSelector(f) => f.place(ctx, random, pos),
            Self::SimpleRandomSelector(f) => f.place(ctx, random, pos),
            Self::WeightedRandomSelector(f) => f.place(ctx, random, pos),
            Self::RandomBooleanSelector(f) => f.place(ctx, random, pos),
            Self::Sequence(f) => f.place(ctx, random, pos),
            Self::Overlay(f) => f.place(ctx, random, pos),
            Self::SingleBlockPillar(f) => f.place(ctx, random, pos),
            Self::ProjectedRandomPatchySquare(f) => f.place(ctx, random, pos),
            Self::RandomNeighborSpread(f) => f.place(ctx, random, pos),
            Self::FillLayer(f) => f.place(ctx, pos),
            Self::BlueIce => simple::blue_ice(ctx, random, pos),
            Self::FreezeTopLayer => simple::freeze_top_layer(ctx, pos),
            Self::Vines => simple::vines(ctx, pos),
            Self::MultifaceGrowth(f) => f.place(ctx, random, pos),
            Self::Ore(f) => f.place(ctx, random, pos),
            Self::ScatteredOre(f) => f.place(ctx, random, pos),
            Self::ReplaceBlobs(f) => f.place(ctx, random, pos),
            Self::UnderwaterMagma(f) => f.place(ctx, random, pos),
            Self::Tree(f) => f.place(ctx, random, pos),
            Self::FallenTree(f) => f.place(ctx, random, pos),
            Self::Other(f) => f.place(ctx, random, pos),
            Self::Unsupported(kind) => {
                ctx.region.note_unsupported(kind);
                false
            }
        }
    }
}

/// Reads an optional integer field.
pub fn int_or(json: &Value, key: &str, default: i32) -> i32 {
    json.get(key).and_then(Value::as_i64).map_or(default, |v| v as i32)
}

pub fn float_or(json: &Value, key: &str, default: f32) -> f32 {
    json.get(key).and_then(Value::as_f64).map_or(default, |v| v as f32)
}

pub fn bool_or(json: &Value, key: &str, default: bool) -> bool {
    json.get(key).and_then(Value::as_bool).unwrap_or(default)
}

pub fn int(json: &Value, key: &str) -> Result<i32, String> {
    json.get(key).and_then(Value::as_i64).map(|v| v as i32).ok_or_else(|| format!("missing {key}"))
}

pub fn float(json: &Value, key: &str) -> Result<f32, String> {
    json.get(key).and_then(Value::as_f64).map(|v| v as f32).ok_or_else(|| format!("missing {key}"))
}

/// `Util.shuffle`: Fisher-Yates from the end.
pub fn shuffle<T>(list: &mut [T], random: &mut WorldgenRandom) {
    for i in (2..=list.len()).rev() {
        let j = random.next_i32_bound(i as i32) as usize;
        list.swap(i - 1, j);
    }
}
