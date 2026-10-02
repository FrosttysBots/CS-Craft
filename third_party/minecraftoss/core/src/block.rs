//! Block states as dense numeric IDs in vanilla's global registry order.
//!
//! Loaded from the pinned block-state catalog exported by the harness
//! (`harness/export_block_state_catalog.py`, source `Block.BLOCK_STATE_REGISTRY`
//! in 26.3). A block's states are laid out like the digits of a mixed-radix
//! number: its first property varies slowest and each property's values keep
//! their declared order. The loader verifies that layout for every block, so
//! property reads and changes are arithmetic rather than lookups.

use crate::ident::Identifier;
use crate::pos::Direction;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// A global block-state ID, identical to vanilla's `Block.BLOCK_STATE_REGISTRY` ID.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockStateId(pub u16);

impl BlockStateId {
    /// `minecraft:air`, verified at load.
    pub const AIR: Self = Self(0);
}

/// A block registry ID, identical to vanilla's `BuiltInRegistries.BLOCK` ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u16);

/// Context-free per-state flags from the catalog.
pub mod flags {
    pub const AIR: u16 = 1 << 0;
    pub const LIQUID: u16 = 1 << 1;
    pub const CAN_OCCLUDE: u16 = 1 << 2;
    pub const SOLID_RENDER: u16 = 1 << 3;
    pub const LEGACY_SOLID: u16 = 1 << 4;
    pub const REPLACEABLE: u16 = 1 << 5;
    pub const PROPAGATES_SKYLIGHT_DOWN: u16 = 1 << 6;
    pub const USE_SHAPE_FOR_LIGHT_OCCLUSION: u16 = 1 << 7;
    pub const HAS_BLOCK_ENTITY: u16 = 1 << 8;
    pub const RANDOMLY_TICKING: u16 = 1 << 9;
    pub const HAS_OFFSET: u16 = 1 << 10;
    pub const RENDER_MODEL: u16 = 1 << 11;
    /// `isRedstoneConductor` in an empty world (schema 4).
    pub const REDSTONE_CONDUCTOR: u16 = 1 << 12;
    /// `isSignalSource` (schema 4).
    pub const SIGNAL_SOURCE: u16 = 1 << 13;
    /// `isValidSpawn` for a plain land animal (schema 9 catalogs).
    pub const VALID_SPAWN_ANIMAL: u16 = 1 << 14;
    /// `isPathfindable(PathComputationType.LAND)`.
    pub const PATHFINDABLE_LAND: u16 = 1 << 15;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluidKind {
    Water,
    Lava,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FluidInfo {
    pub kind: FluidKind,
    /// Vanilla `FluidState.getAmount`, 1..=8.
    pub amount: u8,
    pub source: bool,
    pub falling: bool,
}

/// A face occlusion shape (`BlockState.getFaceOcclusionShape`).
#[derive(Clone, Debug, PartialEq)]
pub enum FaceShape {
    Empty,
    /// Identical to `Shapes.block()`.
    Full,
    /// Boxes as `[min_x, min_y, min_z, max_x, max_y, max_z]` in block units.
    Boxes(Box<[[f64; 6]]>),
}

#[derive(Clone, Debug)]
pub struct Property {
    pub name: Box<str>,
    pub values: Box<[Box<str>]>,
}

#[derive(Clone, Debug)]
pub struct BlockInfo {
    pub name: Identifier,
    first_state: u16,
    state_count: u16,
    default_state: BlockStateId,
    properties: Box<[Property]>,
    /// State-index stride per property, parallel to `properties`.
    strides: Box<[u16]>,
    /// Vanilla class chain, most derived first (`StairBlock`, `Block`);
    /// empty with a catalog older than schema 4.
    classes: Box<[Box<str>]>,
}

impl BlockInfo {
    pub fn properties(&self) -> &[Property] {
        &self.properties
    }

    pub fn default_state(&self) -> BlockStateId {
        self.default_state
    }

    /// Whether the vanilla block is an instance of the named class.
    pub fn is_a(&self, class: &str) -> bool {
        self.classes.iter().any(|c| &**c == class)
    }

    pub fn classes(&self) -> &[Box<str>] {
        &self.classes
    }

    pub fn states(&self) -> impl Iterator<Item = BlockStateId> + use<> {
        let first = self.first_state;
        (0..self.state_count).map(move |i| BlockStateId(first + i))
    }
}

#[derive(Clone, Debug)]
pub struct StateInfo {
    pub block: BlockId,
    pub flags: u16,
    pub light_emission: u8,
    pub light_dampening: u8,
    pub fluid: Option<FluidInfo>,
    /// Indices into the registry's shape table, in `Direction` order.
    faces: [u16; 6],
    sturdy: u32,
    collision: Option<u16>,
    /// `isCollisionShapeFullBlock` in an empty world.
    pub collision_full_block: bool,
    /// `isSuffocating` in an empty world (catalogs from 2026-09-27 on;
    /// false before).
    pub suffocating: bool,
    /// Index into the registry's collision grids.
    collision_grid: Option<u16>,
    /// `BlockState.instrument()` as an index into [`INSTRUMENTS`] (schema 4; harp before).
    pub instrument: u8,
    /// `BlockState.getPistonPushReaction()`.
    pub push_reaction: PushReaction,
    /// `BlockState.getDestroySpeed` (-1 for unbreakable blocks).
    pub destroy_speed: f32,
    /// `Block.getExplosionResistance`.
    pub explosion_resistance: f32,
    /// `FireBlock`'s ignite and burn odds for the block (`setFlammable`).
    pub ignite_odds: u8,
    pub burn_odds: u8,
    /// Index into the registry's sound types (catalogs from 2026-09-27).
    sound_type: Option<u16>,
}

/// `BlockState.getSoundType()`: a block's sound events (without their
/// namespace) and the volume and pitch they play at.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundType {
    pub volume: f32,
    pub pitch: f32,
    pub break_sound: Box<str>,
    pub step: Box<str>,
    pub place: Box<str>,
    pub hit: Box<str>,
    pub fall: Box<str>,
}

/// Vanilla `PushReaction` for blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PushReaction {
    #[default]
    PushPull,
    Popped,
    Immoveable,
    /// Pushed but never pulled (glazed terracotta).
    Push,
}

/// `NoteBlockInstrument` in declaration order; those from `ZOMBIE` on are
/// mob heads, which `worksAboveNoteBlock`.
pub const INSTRUMENTS: [&str; 27] = [
    "harp", "basedrum", "snare", "hat", "bass", "flute", "bell", "guitar", "chime", "xylophone", "iron_xylophone", "cow_bell",
    "didgeridoo", "bit", "banjo", "pling", "trumpet", "trumpet_exposed", "trumpet_oxidized", "trumpet_weathered", "zombie", "skeleton",
    "creeper", "dragon", "wither_skeleton", "piglin", "custom_head",
];

/// Index of the first mob-head instrument in [`INSTRUMENTS`].
pub const FIRST_HEAD_INSTRUMENT: u8 = 20;

/// Vanilla `SupportType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupportType {
    Full,
    Center,
    Rigid,
}

impl StateInfo {
    pub fn has(&self, flag: u16) -> bool {
        self.flags & flag != 0
    }
}

/// Every block and block state, in vanilla registry order.
pub struct BlockRegistry {
    blocks: Vec<BlockInfo>,
    states: Vec<StateInfo>,
    /// Each state's block and flags, packed small enough to stay in cache
    /// for the per-block lookups generation makes.
    hot: Vec<(BlockId, u16, bool)>,
    shapes: Vec<FaceShape>,
    /// Collision shapes' voxel grid coordinates per axis (X, Y, Z).
    grids: Vec<[Vec<f64>; 3]>,
    sound_types: Vec<SoundType>,
    by_name: HashMap<Identifier, BlockId>,
}

#[derive(Deserialize)]
struct CatalogFile {
    schema_version: u32,
    minecraft_version: String,
    face_order: Vec<String>,
    shapes: Vec<Value>,
    /// Collision voxel grids (exported with schema 4 catalogs from 2026-09-26).
    #[serde(default)]
    collision_grids: Vec<Value>,
    /// Sound types (catalogs from 2026-09-27).
    #[serde(default)]
    sound_types: Vec<CatalogSoundType>,
    states: Vec<CatalogState>,
    /// Schema 4: each block's class chain, most derived first, up to `Block`.
    #[serde(default)]
    block_classes: HashMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct CatalogState {
    id: u32,
    block: String,
    block_id: u32,
    default: bool,
    properties: Vec<(String, String)>,
    air: bool,
    liquid: bool,
    can_occlude: bool,
    solid_render: bool,
    legacy_solid: bool,
    replaceable: bool,
    light_emission: u8,
    light_dampening: u8,
    propagates_skylight_down: bool,
    use_shape_for_light_occlusion: bool,
    has_block_entity: bool,
    randomly_ticking: bool,
    render_shape: String,
    has_offset: bool,
    fluid: Option<CatalogFluid>,
    face_occlusion: Vec<u16>,
    /// Schema 3: `isFaceSturdy` bits, direction-major over FULL, CENTER, RIGID.
    #[serde(default)]
    sturdy_faces: u32,
    /// Schema 3: collision shape index into the shape table.
    #[serde(default)]
    collision: Option<u16>,
    #[serde(default)]
    collision_full_block: bool,
    #[serde(default)]
    suffocating: bool,
    #[serde(default)]
    collision_grid: Option<u16>,
    #[serde(default)]
    redstone_conductor: bool,
    #[serde(default)]
    signal_source: bool,
    #[serde(default)]
    instrument: Option<String>,
    #[serde(default)]
    push_reaction: Option<String>,
    #[serde(default)]
    destroy_speed: f32,
    #[serde(default)]
    explosion_resistance: f32,
    #[serde(default)]
    ignite_odds: u8,
    #[serde(default)]
    burn_odds: u8,
    #[serde(default)]
    valid_spawn_animal: bool,
    #[serde(default)]
    pathfindable_land: bool,
    #[serde(default)]
    sound_type: Option<u16>,
}

#[derive(Deserialize)]
struct CatalogSoundType {
    volume_bits: String,
    pitch_bits: String,
    #[serde(rename = "break")]
    break_sound: String,
    step: String,
    place: String,
    hit: String,
    fall: String,
}

#[derive(Deserialize)]
struct CatalogFluid {
    #[serde(rename = "type")]
    kind: String,
    amount: u8,
    source: bool,
    falling: bool,
}

fn parse_shape(value: &Value) -> Result<FaceShape, String> {
    match value {
        Value::String(s) if s == "full" => Ok(FaceShape::Full),
        Value::String(s) if s == "empty" => Ok(FaceShape::Empty),
        Value::Array(boxes) => {
            let mut out = Vec::with_capacity(boxes.len());
            for b in boxes {
                let values = b
                    .as_array()
                    .filter(|v| v.len() == 6)
                    .ok_or("shape box must have six values")?;
                let mut coords = [0.0; 6];
                for (slot, v) in coords.iter_mut().zip(values) {
                    let bits = v["bits"].as_str().ok_or("shape value lacks bits")?;
                    *slot =
                        f64::from_bits(u64::from_str_radix(bits, 16).map_err(|e| e.to_string())?);
                }
                out.push(coords);
            }
            Ok(FaceShape::Boxes(out.into()))
        }
        other => Err(format!("unexpected shape {other}")),
    }
}

impl BlockRegistry {
    /// Loads the pinned catalog, e.g. `artifacts/block-state-catalog/26.3.json`.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file: CatalogFile =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if !matches!(file.schema_version, 2..=4) || file.minecraft_version != "26.3" {
            return Err(format!(
                "unsupported block-state catalog {} for {}",
                file.schema_version, file.minecraft_version
            ));
        }
        let expected_faces = ["down", "up", "north", "south", "west", "east"];
        if file.face_order != expected_faces {
            return Err("catalog face order differs from Direction order".into());
        }
        Self::from_catalog(file)
    }

    fn from_catalog(file: CatalogFile) -> Result<Self, String> {
        if file.states.len() > usize::from(u16::MAX) {
            return Err("too many block states for u16 IDs".into());
        }
        let shapes = file
            .shapes
            .iter()
            .map(parse_shape)
            .collect::<Result<Vec<_>, _>>()?;
        let mut blocks: Vec<BlockInfo> = Vec::new();
        let mut states = Vec::with_capacity(file.states.len());
        let mut by_name = HashMap::new();
        for (index, s) in file.states.iter().enumerate() {
            if s.id as usize != index {
                return Err(format!("state IDs not contiguous at {index}"));
            }
            let block_id = u16::try_from(s.block_id).map_err(|_| "block ID exceeds u16")?;
            let new_block = blocks.last().is_none_or(|b| b.name.as_str() != s.block);
            if new_block {
                if usize::from(block_id) != blocks.len() {
                    return Err(format!(
                        "block {} has registry ID {} but appears at {}",
                        s.block,
                        block_id,
                        blocks.len()
                    ));
                }
                let name = Identifier::parse(&s.block)?;
                if by_name.insert(name.clone(), BlockId(block_id)).is_some() {
                    return Err(format!(
                        "block {} appears in two separate state runs",
                        s.block
                    ));
                }
                blocks.push(BlockInfo {
                    name,
                    first_state: index as u16,
                    state_count: 0,
                    default_state: BlockStateId(index as u16),
                    properties: s
                        .properties
                        .iter()
                        .map(|(n, _)| Property {
                            name: n.as_str().into(),
                            values: Box::new([]),
                        })
                        .collect(),
                    strides: Box::new([]),
                    classes: file
                        .block_classes
                        .get(&s.block)
                        .map(|chain| chain.iter().map(|c| c.as_str().into()).collect())
                        .unwrap_or_default(),
                });
            }
            let block = blocks.last_mut().expect("pushed above");
            block.state_count += 1;
            if s.default {
                block.default_state = BlockStateId(index as u16);
            }
            // Collect property values in first-seen order; validated below.
            if block.properties.len() != s.properties.len() {
                return Err(format!("{} changes property count between states", s.block));
            }
            for (property, (name, value)) in block.properties.iter_mut().zip(&s.properties) {
                if *property.name != **name {
                    return Err(format!("{} changes property order between states", s.block));
                }
                if !property.values.iter().any(|v| **v == **value) {
                    let mut values = property.values.to_vec();
                    values.push(value.as_str().into());
                    property.values = values.into();
                }
            }
            let mut flags = 0;
            for (set, flag) in [
                (s.air, flags::AIR),
                (s.liquid, flags::LIQUID),
                (s.can_occlude, flags::CAN_OCCLUDE),
                (s.solid_render, flags::SOLID_RENDER),
                (s.legacy_solid, flags::LEGACY_SOLID),
                (s.replaceable, flags::REPLACEABLE),
                (s.propagates_skylight_down, flags::PROPAGATES_SKYLIGHT_DOWN),
                (
                    s.use_shape_for_light_occlusion,
                    flags::USE_SHAPE_FOR_LIGHT_OCCLUSION,
                ),
                (s.has_block_entity, flags::HAS_BLOCK_ENTITY),
                (s.randomly_ticking, flags::RANDOMLY_TICKING),
                (s.has_offset, flags::HAS_OFFSET),
                (s.render_shape == "MODEL", flags::RENDER_MODEL),
                (s.redstone_conductor, flags::REDSTONE_CONDUCTOR),
                (s.signal_source, flags::SIGNAL_SOURCE),
                (s.valid_spawn_animal, flags::VALID_SPAWN_ANIMAL),
                (s.pathfindable_land, flags::PATHFINDABLE_LAND),
            ] {
                if set {
                    flags |= flag;
                }
            }
            if !matches!(s.render_shape.as_str(), "MODEL" | "INVISIBLE") {
                return Err(format!("unknown render shape {}", s.render_shape));
            }
            let fluid = match &s.fluid {
                None => None,
                Some(f) => Some(FluidInfo {
                    kind: match f.kind.as_str() {
                        "minecraft:water" | "minecraft:flowing_water" => FluidKind::Water,
                        "minecraft:lava" | "minecraft:flowing_lava" => FluidKind::Lava,
                        other => return Err(format!("unknown fluid {other}")),
                    },
                    amount: f.amount,
                    source: f.source,
                    falling: f.falling,
                }),
            };
            let faces: [u16; 6] = s
                .face_occlusion
                .as_slice()
                .try_into()
                .map_err(|_| "face_occlusion needs six entries")?;
            if faces.iter().chain(&s.collision).any(|&f| usize::from(f) >= shapes.len()) {
                return Err("face shape index out of range".into());
            }
            states.push(StateInfo {
                block: BlockId(block_id),
                flags,
                light_emission: s.light_emission,
                light_dampening: s.light_dampening,
                fluid,
                faces,
                sturdy: s.sturdy_faces,
                collision: s.collision,
                collision_full_block: s.collision_full_block,
                suffocating: s.suffocating,
                collision_grid: s.collision_grid,
                instrument: match &s.instrument {
                    None => 0,
                    Some(name) => INSTRUMENTS
                        .iter()
                        .position(|i| i == name)
                        .ok_or_else(|| format!("unknown instrument {name}"))? as u8,
                },
                push_reaction: match s.push_reaction.as_deref() {
                    None | Some("PUSH_PULL") => PushReaction::PushPull,
                    Some("POPPED") => PushReaction::Popped,
                    Some("IMMOVEABLE") => PushReaction::Immoveable,
                    Some("PUSH") => PushReaction::Push,
                    Some(other) => return Err(format!("unknown push reaction {other}")),
                },
                destroy_speed: s.destroy_speed,
                explosion_resistance: s.explosion_resistance,
                ignite_odds: s.ignite_odds,
                burn_odds: s.burn_odds,
                sound_type: s.sound_type,
            });
        }
        for block in &mut blocks {
            let mut strides = vec![0u16; block.properties.len()];
            let mut stride = 1usize;
            for (slot, property) in strides.iter_mut().zip(block.properties.iter()).rev() {
                *slot = u16::try_from(stride)
                    .map_err(|_| format!("{} has too many states", block.name))?;
                stride *= property.values.len();
            }
            block.strides = strides.into();
        }
        let bits = |v: &Value| -> Result<f64, String> {
            let text = v["bits"].as_str().ok_or("grid value lacks bits")?;
            Ok(f64::from_bits(u64::from_str_radix(text, 16).map_err(|e| e.to_string())?))
        };
        let mut grids = Vec::with_capacity(file.collision_grids.len());
        for grid in &file.collision_grids {
            let axes = grid.as_array().filter(|a| a.len() == 3).ok_or("collision grid needs three axes")?;
            let axis = |i: usize| -> Result<Vec<f64>, String> { axes[i].as_array().ok_or("grid axis must be a list")?.iter().map(bits).collect() };
            grids.push([axis(0)?, axis(1)?, axis(2)?]);
        }
        let float = |text: &str| -> Result<f32, String> { Ok(f32::from_bits(u32::from_str_radix(text, 16).map_err(|e| e.to_string())?)) };
        // Sound events without the vanilla namespace, as the sound registry keys them.
        let event = |id: &str| -> Box<str> { id.strip_prefix("minecraft:").unwrap_or(id).into() };
        let mut sound_types = Vec::with_capacity(file.sound_types.len());
        for t in &file.sound_types {
            sound_types.push(SoundType {
                volume: float(&t.volume_bits)?,
                pitch: float(&t.pitch_bits)?,
                break_sound: event(&t.break_sound),
                step: event(&t.step),
                place: event(&t.place),
                hit: event(&t.hit),
                fall: event(&t.fall),
            });
        }
        if states.iter().any(|s| s.sound_type.is_some_and(|i| usize::from(i) >= sound_types.len())) {
            return Err("sound type index out of range".into());
        }
        let hot = states.iter().map(|s| (s.block, s.flags, s.fluid.is_some())).collect();
        let registry = Self {
            blocks,
            states,
            hot,
            shapes,
            grids,
            sound_types,
            by_name,
        };
        registry.verify_layout(&file.states)?;
        if !registry.state(BlockStateId::AIR).has(flags::AIR)
            || registry.block(BlockId(0)).name.as_str() != "minecraft:air"
        {
            return Err("state 0 must be minecraft:air".into());
        }
        Ok(registry)
    }

    /// Computes strides and checks every state's properties against the mixed-radix layout.
    fn verify_layout(&self, catalog: &[CatalogState]) -> Result<(), String> {
        for block in &self.blocks {
            let expected: usize = block.properties.iter().map(|p| p.values.len()).product();
            if expected != usize::from(block.state_count) {
                return Err(format!(
                    "{} has {} states but its properties allow {expected}",
                    block.name, block.state_count
                ));
            }
        }
        for (index, s) in catalog.iter().enumerate() {
            let state = BlockStateId(index as u16);
            for (name, value) in &s.properties {
                if self.property(state, name) != Some(value.as_str()) {
                    return Err(format!(
                        "{} state {index} does not follow the mixed-radix layout at {name}",
                        s.block
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn state_count(&self) -> usize {
        self.states.len()
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// `BlockState.getSoundType()`, when the catalog records sound types.
    pub fn sound_type(&self, id: BlockStateId) -> Option<&SoundType> {
        let index = self.state(id).sound_type?;
        self.sound_types.get(usize::from(index))
    }

    pub fn state(&self, id: BlockStateId) -> &StateInfo {
        &self.states[usize::from(id.0)]
    }

    pub fn block(&self, id: BlockId) -> &BlockInfo {
        &self.blocks[usize::from(id.0)]
    }

    pub fn block_of(&self, state: BlockStateId) -> BlockId {
        self.hot[usize::from(state.0)].0
    }

    pub fn block_by_name(&self, name: &str) -> Option<BlockId> {
        self.by_name.get(&Identifier::parse(name).ok()?).copied()
    }

    pub fn blocks(&self) -> impl Iterator<Item = (BlockId, &BlockInfo)> {
        self.blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (BlockId(i as u16), b))
    }

    pub fn is(&self, state: BlockStateId, flag: u16) -> bool {
        self.hot[usize::from(state.0)].1 & flag != 0
    }

    /// Whether the state holds a fluid (`StateInfo::fluid` is set).
    pub fn has_fluid(&self, state: BlockStateId) -> bool {
        self.hot[usize::from(state.0)].2
    }

    pub fn is_air(&self, state: BlockStateId) -> bool {
        self.is(state, flags::AIR)
    }

    /// `BlockState.isFaceSturdy` in an empty world (the cached value vanilla
    /// uses for most blocks). Requires a schema 3 catalog.
    pub fn is_face_sturdy(&self, state: BlockStateId, direction: Direction, support: SupportType) -> bool {
        self.state(state).sturdy & 1 << (direction.index() * 3 + support as usize) != 0
    }

    /// The collision shape in an empty world; `None` from a schema 2 catalog.
    pub fn collision_shape(&self, state: BlockStateId) -> Option<&FaceShape> {
        self.state(state).collision.map(|i| &self.shapes[usize::from(i)])
    }

    /// The collision shape as block-local boxes,
    /// `[min_x, min_y, min_z, max_x, max_y, max_z]`.
    pub fn collision_boxes(&self, state: BlockStateId) -> Vec<[f64; 6]> {
        match self.collision_shape(state) {
            None | Some(FaceShape::Empty) => Vec::new(),
            Some(FaceShape::Full) => vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]],
            Some(FaceShape::Boxes(boxes)) => boxes.to_vec(),
        }
    }

    /// The collision shape's voxel grid coordinates (X, Y, Z), when it is
    /// neither empty nor a full block and the catalog carries grids.
    pub fn collision_grid(&self, state: BlockStateId) -> Option<&[Vec<f64>; 3]> {
        self.state(state).collision_grid.map(|i| &self.grids[usize::from(i)])
    }

    pub fn face_shape(&self, state: BlockStateId, direction: Direction) -> &FaceShape {
        &self.shapes[usize::from(self.state(state).faces[direction.index()])]
    }

    fn stride(&self, block: &BlockInfo, property: usize) -> usize {
        usize::from(block.strides[property])
    }

    fn property_index(block: &BlockInfo, name: &str) -> Option<usize> {
        block.properties.iter().position(|p| &*p.name == name)
    }

    /// The value of one property, or `None` if the block lacks it.
    pub fn property(&self, state: BlockStateId, name: &str) -> Option<&str> {
        let block = self.block(self.block_of(state));
        let index = Self::property_index(block, name)?;
        let offset = usize::from(state.0 - block.first_state);
        let property = &block.properties[index];
        Some(&property.values[offset / self.stride(block, index) % property.values.len()])
    }

    /// The same block with one property changed, or `None` if the property or value does not exist.
    pub fn with_property(
        &self,
        state: BlockStateId,
        name: &str,
        value: &str,
    ) -> Option<BlockStateId> {
        let block = self.block(self.block_of(state));
        let index = Self::property_index(block, name)?;
        let property = &block.properties[index];
        let new = property.values.iter().position(|v| &**v == value)?;
        let stride = self.stride(block, index);
        let offset = usize::from(state.0 - block.first_state);
        let old = offset / stride % property.values.len();
        Some(BlockStateId(
            (usize::from(state.0) - old * stride + new * stride) as u16,
        ))
    }

    /// Parses `minecraft:block[prop=value,...]`; unspecified properties keep their defaults.
    pub fn parse_state(&self, text: &str) -> Result<BlockStateId, String> {
        let (name, props) = match text.split_once('[') {
            Some((name, rest)) => (
                name,
                rest.strip_suffix(']')
                    .ok_or_else(|| format!("unterminated properties in {text}"))?,
            ),
            None => (text, ""),
        };
        let block = self
            .block_by_name(name)
            .ok_or_else(|| format!("unknown block {name}"))?;
        let mut state = self.block(block).default_state;
        for pair in props.split(',').filter(|p| !p.is_empty()) {
            let (key, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("bad property {pair} in {text}"))?;
            state = self
                .with_property(state, key, value)
                .ok_or_else(|| format!("{name} has no {key}={value}"))?;
        }
        Ok(state)
    }

    /// Canonical `minecraft:block[prop=value,...]` text in property order.
    pub fn state_to_string(&self, state: BlockStateId) -> String {
        let block = self.block(self.block_of(state));
        if block.properties.is_empty() {
            return block.name.to_string();
        }
        let props: Vec<String> = block
            .properties
            .iter()
            .map(|p| {
                format!(
                    "{}={}",
                    p.name,
                    self.property(state, &p.name).expect("own property")
                )
            })
            .collect();
        format!("{}[{}]", block.name, props.join(","))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// A four-block catalog in the real layout: air, stone, a two-property block and water.
    pub(crate) fn tiny_catalog() -> Value {
        let base = json!({"air":false,"liquid":false,"can_occlude":true,"solid_render":true,"legacy_solid":true,
            "replaceable":false,"light_emission":0,"light_dampening":15,"propagates_skylight_down":false,
            "use_shape_for_light_occlusion":false,"has_block_entity":false,"randomly_ticking":false,
            "render_shape":"MODEL","has_offset":false,"face_occlusion":[1,1,1,1,1,1]});
        let state =
            |id: u32, block: &str, block_id: u32, default: bool, props: Value, extra: Value| {
                let mut s = base.clone();
                for (k, v) in extra.as_object().unwrap() {
                    s[k] = v.clone();
                }
                s["id"] = json!(id);
                s["block"] = json!(block);
                s["block_id"] = json!(block_id);
                s["default"] = json!(default);
                s["properties"] = props;
                s
            };
        let mut states = vec![
            state(
                0,
                "minecraft:air",
                0,
                true,
                json!([]),
                json!({"air":true,"can_occlude":false,"solid_render":false,"render_shape":"INVISIBLE","face_occlusion":[0,0,0,0,0,0]}),
            ),
            state(1, "minecraft:stone", 1, true, json!([]), json!({})),
        ];
        // axis x/y/z (slowest) then waterlogged true/false, like vanilla StateDefinition.
        let mut id = 2;
        for axis in ["x", "y", "z"] {
            for waterlogged in ["true", "false"] {
                states.push(state(
                    id,
                    "minecraft:test_log",
                    2,
                    axis == "y" && waterlogged == "false",
                    json!([["axis", axis], ["waterlogged", waterlogged]]),
                    json!({}),
                ));
                id += 1;
            }
        }
        states.push(state(id, "minecraft:water", 3, true, json!([["level", "0"]]),
            json!({"liquid":true,"can_occlude":false,"solid_render":false,"render_shape":"INVISIBLE","light_dampening":1,
                   "fluid":{"type":"minecraft:water","amount":8,"source":true,"falling":false},"face_occlusion":[0,0,0,0,0,0]})));
        json!({"schema_version":2,"minecraft_version":"26.3","face_order":["down","up","north","south","west","east"],
               "shapes":["empty","full"],"states":states})
    }

    pub(crate) fn tiny_registry() -> BlockRegistry {
        BlockRegistry::from_catalog(serde_json::from_value(tiny_catalog()).unwrap()).unwrap()
    }

    #[test]
    fn mixed_radix_properties_round_trip() {
        let r = tiny_registry();
        let log = r.parse_state("minecraft:test_log").unwrap();
        assert_eq!(log, BlockStateId(5));
        assert_eq!(
            r.state_to_string(log),
            "minecraft:test_log[axis=y,waterlogged=false]"
        );
        let z_wet = r.parse_state("test_log[axis=z,waterlogged=true]").unwrap();
        assert_eq!(z_wet, BlockStateId(6));
        assert_eq!(r.property(z_wet, "axis"), Some("z"));
        assert_eq!(r.with_property(z_wet, "axis", "x"), Some(BlockStateId(2)));
        assert_eq!(r.with_property(z_wet, "axis", "w"), None);
        assert!(r.parse_state("minecraft:test_log[shape=round]").is_err());
        assert_eq!(r.state_to_string(BlockStateId::AIR), "minecraft:air");
    }

    #[test]
    fn flags_fluids_and_faces_load() {
        let r = tiny_registry();
        assert!(r.is_air(BlockStateId::AIR));
        let water = r.parse_state("minecraft:water").unwrap();
        assert_eq!(r.state(water).fluid.unwrap().kind, FluidKind::Water);
        assert_eq!(
            *r.face_shape(BlockStateId(1), Direction::Up),
            FaceShape::Full
        );
        assert_eq!(*r.face_shape(water, Direction::Up), FaceShape::Empty);
    }

    #[test]
    fn rejects_states_that_break_the_layout() {
        let mut catalog = tiny_catalog();
        // Swap two log states so the property digits are out of order.
        let states = catalog["states"].as_array_mut().unwrap();
        let (a, b) = (
            states[2]["properties"].clone(),
            states[3]["properties"].clone(),
        );
        states[2]["properties"] = b;
        states[3]["properties"] = a;
        let error = BlockRegistry::from_catalog(serde_json::from_value(catalog).unwrap())
            .err()
            .unwrap();
        assert!(error.contains("mixed-radix"), "{error}");
    }
}
