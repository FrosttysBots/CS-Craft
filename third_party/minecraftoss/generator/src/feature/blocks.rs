//! Block state parsing and the block behaviour queries features ask about.

use minecraftoss_core::block::{flags, FluidKind};
use minecraftoss_core::pos::Direction;
use minecraftoss_core::tags::TagId;
use minecraftoss_core::{BlockId, BlockStateId, Registries, SupportType};
use serde_json::Value;

/// A block state in data-pack form: `"minecraft:x"` (the default state), or
/// `{"Name"|"id": ..., "Properties"|"properties": {...}}`.
pub fn parse_state(registries: &Registries, json: &Value) -> Result<BlockStateId, String> {
    let blocks = &registries.blocks;
    if let Some(name) = json.as_str() {
        return blocks.parse_state(name);
    }
    let name = json.get("Name").or_else(|| json.get("id")).and_then(Value::as_str).ok_or_else(|| format!("block state lacks a name: {json}"))?;
    let mut state = blocks.parse_state(name)?;
    if let Some(props) = json.get("Properties").or_else(|| json.get("properties")).and_then(Value::as_object) {
        for (key, value) in props {
            let value = value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string());
            state = blocks.with_property(state, key, &value).ok_or_else(|| format!("{name} has no {key}={value}"))?;
        }
    }
    Ok(state)
}

/// A `HolderSet<Block>`: a block, a list of blocks, or `#tag`.
#[derive(Clone, Debug)]
pub enum BlockSet {
    Blocks(Vec<BlockId>),
    Tag(TagId),
}

impl BlockSet {
    pub fn parse(registries: &Registries, json: &Value) -> Result<Self, String> {
        let one = |name: &str| -> Result<Self, String> {
            if let Some(tag) = name.strip_prefix('#') {
                Ok(Self::Tag(registries.block_tags.require(tag)?))
            } else {
                Ok(Self::Blocks(vec![registries.blocks.block_by_name(name).ok_or_else(|| format!("unknown block {name}"))?]))
            }
        };
        match json {
            Value::String(name) => one(name),
            Value::Array(list) => {
                let mut blocks = Vec::new();
                for entry in list {
                    let name = entry.as_str().ok_or("block list entry is not a string")?;
                    blocks.push(registries.blocks.block_by_name(name).ok_or_else(|| format!("unknown block {name}"))?);
                }
                Ok(Self::Blocks(blocks))
            }
            other => Err(format!("invalid block set {other}")),
        }
    }

    pub fn contains(&self, registries: &Registries, state: BlockStateId) -> bool {
        match self {
            Self::Blocks(blocks) => blocks.contains(&registries.blocks.block_of(state)),
            Self::Tag(tag) => registries.block_in_tag(state, *tag),
        }
    }
}

/// A fluid type a `FluidState` can be (`Fluids`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluidType {
    Empty,
    Water,
    FlowingWater,
    Lava,
    FlowingLava,
}

impl FluidType {
    fn parse_one(name: &str) -> Result<Vec<Self>, String> {
        Ok(match name.trim_start_matches("minecraft:") {
            "empty" => vec![Self::Empty],
            "water" => vec![Self::Water],
            "flowing_water" => vec![Self::FlowingWater],
            "lava" => vec![Self::Lava],
            "flowing_lava" => vec![Self::FlowingLava],
            // FluidTags.WATER / LAVA.
            "#water" | "#minecraft:water" => vec![Self::Water, Self::FlowingWater],
            "#lava" | "#minecraft:lava" => vec![Self::Lava, Self::FlowingLava],
            other => return Err(format!("unknown fluid {other}")),
        })
    }

    /// A `HolderSet<Fluid>`.
    pub fn parse_set(json: &Value) -> Result<Vec<Self>, String> {
        match json {
            Value::String(name) if name.starts_with('#') => Self::parse_one(&format!("#{}", name.trim_start_matches('#').trim_start_matches("minecraft:"))),
            Value::String(name) => Self::parse_one(name),
            Value::Array(list) => {
                let mut out = Vec::new();
                for entry in list {
                    out.extend(Self::parse_one(entry.as_str().ok_or("fluid list entry is not a string")?)?);
                }
                Ok(out)
            }
            other => Err(format!("invalid fluid set {other}")),
        }
    }
}

/// Block behaviour for feature placement, from the block-state catalog and tags.
pub struct Behaviour<'r> {
    pub registries: &'r Registries,
}

impl Behaviour<'_> {
    pub fn is_air(&self, state: BlockStateId) -> bool {
        self.registries.blocks.is_air(state)
    }

    /// `BlockState.isSolid` (the legacy solid flag).
    pub fn is_solid(&self, state: BlockStateId) -> bool {
        self.registries.blocks.is(state, flags::LEGACY_SOLID)
    }

    /// `BlockState.canBeReplaced`.
    pub fn is_replaceable(&self, state: BlockStateId) -> bool {
        self.registries.blocks.is(state, flags::REPLACEABLE)
    }

    /// `BlockState.isFaceSturdy` with `SupportType.FULL`.
    pub fn is_face_sturdy(&self, state: BlockStateId, direction: Direction) -> bool {
        self.registries.blocks.is_face_sturdy(state, direction, SupportType::Full)
    }

    pub fn is_face_sturdy_as(&self, state: BlockStateId, direction: Direction, support: SupportType) -> bool {
        self.registries.blocks.is_face_sturdy(state, direction, support)
    }

    /// The fluid a block state holds.
    pub fn fluid(&self, state: BlockStateId) -> FluidType {
        match self.registries.blocks.state(state).fluid {
            None => FluidType::Empty,
            Some(f) => match (f.kind, f.source) {
                (FluidKind::Water, true) => FluidType::Water,
                (FluidKind::Water, false) => FluidType::FlowingWater,
                (FluidKind::Lava, true) => FluidType::Lava,
                (FluidKind::Lava, false) => FluidType::FlowingLava,
            },
        }
    }

    pub fn is_water(&self, state: BlockStateId) -> bool {
        matches!(self.fluid(state), FluidType::Water | FluidType::FlowingWater)
    }

    pub fn is_lava(&self, state: BlockStateId) -> bool {
        matches!(self.fluid(state), FluidType::Lava | FluidType::FlowingLava)
    }

    /// `FluidState.isSource` for water or lava.
    pub fn is_source(&self, state: BlockStateId) -> bool {
        self.registries.blocks.state(state).fluid.is_some_and(|f| f.source)
    }

    pub fn block_name(&self, state: BlockStateId) -> &str {
        self.registries.blocks.block(self.registries.blocks.block_of(state)).name.as_str()
    }

    pub fn is_block(&self, state: BlockStateId, name: &str) -> bool {
        self.block_name(state) == name
    }

    pub fn in_tag(&self, state: BlockStateId, tag: TagId) -> bool {
        self.registries.block_in_tag(state, tag)
    }

    pub fn property<'a>(&'a self, state: BlockStateId, name: &str) -> Option<&'a str> {
        self.registries.blocks.property(state, name)
    }

    /// `BlockState.trySetValue`: unchanged when the block lacks the property.
    pub fn try_with(&self, state: BlockStateId, name: &str, value: &str) -> BlockStateId {
        self.registries.blocks.with_property(state, name, value).unwrap_or(state)
    }
}
