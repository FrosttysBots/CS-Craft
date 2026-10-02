//! `RuleTest` (vanilla `structure.templatesystem`): block-state tests used by
//! ore targets and structure processors.

use super::blocks::parse_state;
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::tags::TagId;
use minecraftoss_core::{BlockId, BlockPos, BlockStateId, Registries};
use serde_json::Value;

#[derive(Clone, Debug)]
pub enum RuleTest {
    AlwaysTrue,
    Block(BlockId),
    BlockState(BlockStateId),
    Tag(TagId),
    Height(i32, i32),
    RandomBlock(BlockId, f32),
    RandomBlockState(BlockStateId, f32),
    Not(Box<RuleTest>),
    AnyOf(Vec<RuleTest>),
    AllOf(Vec<RuleTest>),
}

impl RuleTest {
    pub fn parse(registries: &Registries, json: &Value) -> Result<Self, String> {
        let kind = json["predicate_type"].as_str().ok_or_else(|| format!("rule test lacks predicate_type: {json}"))?;
        let block = |key: &str| {
            let name = json[key].as_str().ok_or_else(|| format!("{kind} lacks {key}"))?;
            registries.blocks.block_by_name(name).ok_or_else(|| format!("unknown block {name}"))
        };
        let float = |key: &str| json[key].as_f64().map(|v| v as f32).ok_or_else(|| format!("{kind} lacks {key}"));
        let list = |key: &str| -> Result<Vec<Self>, String> {
            json[key].as_array().ok_or_else(|| format!("{kind} lacks {key}"))?.iter().map(|r| Self::parse(registries, r)).collect()
        };
        Ok(match kind.trim_start_matches("minecraft:") {
            "always_true" => Self::AlwaysTrue,
            "block_match" => Self::Block(block("block")?),
            "blockstate_match" => Self::BlockState(parse_state(registries, &json["block_state"])?),
            "tag_match" => Self::Tag(registries.block_tags.require(json["tag"].as_str().ok_or("tag_match lacks tag")?)?),
            "height_match" => Self::Height(
                json["min_inclusive"].as_i64().map_or(-2032, |v| v as i32),
                json["max_inclusive"].as_i64().map_or(2031, |v| v as i32),
            ),
            "random_block_match" => Self::RandomBlock(block("block")?, float("probability")?),
            "random_blockstate_match" => Self::RandomBlockState(parse_state(registries, &json["block_state"])?, float("probability")?),
            "not" => Self::Not(Box::new(Self::parse(registries, &json["rule"])?)),
            "any_of" => Self::AnyOf(list("rules")?),
            "all_of" => Self::AllOf(list("rules")?),
            other => return Err(format!("unknown rule test {other}")),
        })
    }

    /// `RuleTest.test(state, pos, random)`.
    pub fn test(&self, registries: &Registries, state: BlockStateId, pos: BlockPos, random: &mut impl RandomSource) -> bool {
        match self {
            Self::AlwaysTrue => true,
            Self::Block(block) => registries.blocks.block_of(state) == *block,
            Self::BlockState(s) => state == *s,
            Self::Tag(tag) => registries.block_in_tag(state, *tag),
            Self::Height(min, max) => *min <= pos.y && pos.y <= *max,
            Self::RandomBlock(block, p) => registries.blocks.block_of(state) == *block && random.next_f32() < *p,
            Self::RandomBlockState(s, p) => state == *s && random.next_f32() < *p,
            Self::Not(rule) => !rule.test(registries, state, pos, random),
            Self::AnyOf(rules) => rules.iter().any(|r| r.test(registries, state, pos, random)),
            Self::AllOf(rules) => rules.iter().all(|r| r.test(registries, state, pos, random)),
        }
    }
}

/// `BlockReplacement`: an ore target and the state it becomes.
#[derive(Clone, Debug)]
pub struct Replacement {
    pub target: RuleTest,
    pub state: BlockStateId,
}

impl Replacement {
    pub fn parse_list(registries: &Registries, json: &Value) -> Result<Vec<Self>, String> {
        json.as_array()
            .ok_or("targets is not a list")?
            .iter()
            .map(|t| Ok(Self { target: RuleTest::parse(registries, &t["target"])?, state: parse_state(registries, &t["state"])? }))
            .collect()
    }
}
