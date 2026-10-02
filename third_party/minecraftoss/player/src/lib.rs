//! Deterministic, fixed-tick creative player prototype. World state is semantic and has no
//! renderer or resource-pack dependency. Measured 26.3 flat-ground profiles are validated.
use glam::DVec3;
use inventory::Inventory;
use std::collections::{BTreeMap, BTreeSet};

pub mod path_type;
pub mod advancement;
pub mod chest;
pub mod collision;
pub mod crafting;
pub mod daylight;
pub mod dropper;
pub mod food;
pub mod furnace;
pub mod hopper;
pub mod inventory;
pub mod item_catalog;
pub mod items;
pub mod lightning;
pub mod loot;
pub mod mining;
pub mod jmath;
pub mod mth;
pub mod rail;
pub mod redstone;
pub mod rng;
pub mod statistics;
pub mod survival;
pub mod ticks;
pub mod wax;
pub use survival::Difficulty;
use survival::{DamageScaling, SurvivalStatus};

pub type Pos = (i32, i32, i32);
/// Local-space selection boxes shared by targeting and its renderer.
pub fn selection_boxes(world: &impl World, pos: Pos) -> Vec<([f64;3], [f64;3])> {
    world.block(pos).map(|b| local_shapes(&b).into_iter().map(|b|(b.min.to_array(),b.max.to_array())).collect()).unwrap_or_default()
}


#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameMode {
    Creative,
    Survival,
    Adventure,
    Spectator,
}
impl GameMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Survival => "survival",
            Self::Creative => "creative",
            Self::Adventure => "adventure",
            Self::Spectator => "spectator",
        }
    }
    pub fn is_survival(self) -> bool {
        matches!(self, Self::Survival | Self::Adventure)
    }
    pub fn may_fly(self) -> bool {
        matches!(self, Self::Creative | Self::Spectator)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Block {
    pub id: String,
    pub properties: BTreeMap<String, String>,
}
impl Block {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            properties: BTreeMap::new(),
        }
    }
    pub fn with(mut self, name: &str, value: &str) -> Self {
        self.properties.insert(name.into(), value.into());
        self
    }
    pub fn property(&self, name: &str) -> Option<&str> {
        self.properties.get(name).map(String::as_str)
    }
}
/// An item entity as mobs see it (`ItemEntity`): where it is, its stack and
/// whether it can be picked up yet.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldItem {
    pub id: i32,
    pub position: DVec3,
    pub item: String,
    pub count: i32,
    pub components: Option<serde_json::Value>,
    pub pickup_delay: i32,
}

pub trait World {
    fn block(&self, pos: Pos) -> Option<Block>;
    fn set_block(&mut self, pos: Pos, block: Option<Block>);
    /// The item entities whose boxes meet the box from `min` to `max`
    /// (`getEntitiesOfClass(ItemEntity.class, box)`), in the level's order.
    /// Worlds without item entities have none.
    fn items_in(&self, _min: DVec3, _max: DVec3) -> Vec<WorldItem> {
        Vec::new()
    }
    /// An item entity by its ID, while it is there.
    fn item(&self, _id: i32) -> Option<WorldItem> {
        None
    }
    /// Takes `count` items from an item entity (`ItemEntity.getItem().shrink`),
    /// discarding it when none are left.
    fn take_item(&mut self, _id: i32, _count: i32) {}
    /// `addFreshEntity` for an item entity made at `position` with `velocity`
    /// and a pickup delay (its age 0); worlds without item entities drop it.
    fn spawn_item(&mut self, _position: DVec3, _stack: &inventory::ItemStack, _velocity: DVec3, _pickup_delay: i32) {}
    /// `Block.popResource`'s item entity (`new ItemEntity(level, x, y, z,
    /// stack)`, its own random turning and throwing it) with the default
    /// pickup delay.
    fn spawn_popped_item(&mut self, _position: DVec3, _stack: &inventory::ItemStack) {}
    /// `scheduleTick(pos, block, delay)` for the block at `pos`; worlds
    /// without block ticks drop it.
    fn schedule_block_tick(&mut self, _pos: Pos, _delay: i32) {}
    /// `FarmlandBlock.turnToBaseBlock`: the farmland at `pos` becomes dirt
    /// (`setBlockAndUpdate`), so what grew on it cannot stay (`updateShape`
    /// breaks a crop, its loot popped out by the level random). Worlds
    /// without neighbour updates just set the dirt.
    fn trample_farmland(&mut self, pos: Pos, _random: &mut rng::LegacyRandom) {
        self.set_block(pos, Some(Block::new("minecraft:dirt")));
    }
    /// `Block.getDrops` for `block` at `pos` broken by a mob with no tool:
    /// its loot table's stacks (tables with a random sequence draw from the
    /// world's). Worlds without loot give none.
    fn block_drops(&self, _pos: Pos, _block: &Block) -> Vec<inventory::ItemStack> {
        Vec::new()
    }
    /// `Level.getBiome` at a block (the zoomed biome), by ID; worlds without
    /// biomes have none.
    fn biome(&self, _pos: Pos) -> Option<String> {
        None
    }
    /// The block's collision boxes at `pos` in block-local units,
    /// `[min_x, min_y, min_z, max_x, max_y, max_z]` (`BlockState.getCollisionShape`
    /// in an empty context). The default knows a few authored shapes by name
    /// and treats other blocks as full cubes; worlds backed by the block
    /// catalog answer exactly.
    fn collision_boxes(&self, pos: Pos) -> Vec<[f64; 6]> {
        self.block(pos).map_or_else(Vec::new, |block| authored_collision_boxes(&block))
    }
    /// `WalkNodeEvaluator.getPathTypeFromState` at `pos`.
    fn path_type_from_state(&self, pos: Pos) -> path_type::PathType {
        path_type::path_type_of_block(self.block(pos).as_ref())
    }
    /// `BlockState.isPathfindable(PathComputationType.LAND)`. By default,
    /// blocks without a full collision cube, but lava (`LiquidBlock`),
    /// cactus and campfires, which refuse it.
    fn pathfindable(&self, pos: Pos) -> bool {
        self.block(pos).is_none_or(|b| {
            !matches!(b.id.as_str(), "minecraft:lava" | "minecraft:cactus" | "minecraft:campfire" | "minecraft:soul_campfire") && authored_collision_boxes(&b) != [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]
        })
    }
    /// Whether the fluid at `pos` is in `#minecraft:entity_floatable`
    /// (water).
    fn floatable_fluid(&self, pos: Pos) -> bool {
        self.block(pos).is_some_and(|b| b.id == "minecraft:water" || b.property("waterlogged") == Some("true"))
    }
    /// `BlockState.isSolid`. By default, blocks with a full collision cube.
    fn solid(&self, pos: Pos) -> bool {
        self.block(pos).is_some_and(|b| authored_collision_boxes(&b) == [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]])
    }
    /// `BlockState.isSolidRender`. By default, as `solid`.
    fn solid_render(&self, pos: Pos) -> bool {
        self.solid(pos)
    }
    /// `LevelReader.getPathfindingCostFromLightLevels`: the light-dependent
    /// magic value less a half. Authored worlds give every position 0.
    fn light_path_cost(&self, _pos: Pos) -> f32 {
        0.0
    }
    /// `LevelReader.canSeeSky`: full sky light at the block.
    fn can_see_sky(&self, _pos: Pos) -> bool {
        false
    }
    /// `Level.isRainingAt`.
    fn rain_at(&self, _pos: Pos) -> bool {
        false
    }
    /// The lowest block Y (`LevelHeightAccessor.getMinY`).
    fn min_y(&self) -> i32 {
        -64
    }
    /// The highest block Y (`LevelHeightAccessor.getMaxY`).
    fn max_y(&self) -> i32 {
        319
    }
    /// `BlockState.is(tag)` for the block at `pos` (a `minecraft:` block
    /// tag). Authored worlds know the tags their mobs ask for
    /// ([`authored_block_in_tag`]); worlds backed by the registries answer
    /// from the data.
    fn block_in_tag(&self, pos: Pos, tag: &str) -> bool {
        self.block(pos).is_some_and(|block| authored_block_in_tag(&block, tag, &self.collision_boxes(pos)))
    }
    /// The block's `SoundType` step sound at `pos`: its event, volume and
    /// pitch (`playStepSound` plays it at 0.15 of the volume). Authored
    /// worlds know none.
    fn step_sound(&self, _pos: Pos) -> Option<(String, f32, f32)> {
        None
    }
    /// `BlockState.isSuffocating` for the block at `pos`. Authored worlds:
    /// full collision cubes but glass, leaves, copper grates, chorus
    /// flowers and mangrove roots; and dirt paths, farmland, mud and soul
    /// sand, whose tops are lower.
    fn suffocating(&self, pos: Pos) -> bool {
        let Some(block) = self.block(pos) else { return false };
        let id = block.id.as_str();
        if matches!(id, "minecraft:dirt_path" | "minecraft:farmland" | "minecraft:mud" | "minecraft:soul_sand") {
            return true;
        }
        let exempt = id.ends_with("glass") || id.ends_with("leaves") || id.ends_with("copper_grate") || id == "minecraft:chorus_flower" || id == "minecraft:mangrove_roots";
        !exempt && self.collision_boxes(pos) == [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]
    }
    /// `BlockState.isFaceSturdy(DOWN, FULL)` for the block at `pos`: its
    /// underside is a full face. Authored worlds take a full collision cube.
    fn sturdy_underside(&self, pos: Pos) -> bool {
        self.collision_boxes(pos) == [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]
    }
    /// The block's `SoundType` fall sound at `pos`: its event, volume and
    /// pitch. Authored worlds take the step's family (`block.<family>.fall`).
    fn fall_sound(&self, pos: Pos) -> Option<(String, f32, f32)> {
        let (step, volume, pitch) = self.step_sound(pos)?;
        let family = step.strip_suffix(".step")?;
        Some((format!("{family}.fall"), volume, pitch))
    }
    /// `ServerLevel.getCurrentDifficultyAt`'s inputs at a block besides the
    /// difficulty: the overworld clock, and the chunk's inhabited time and
    /// the moon's brightness (both nothing where the chunk is not loaded).
    /// Authored worlds are fresh: no time has passed.
    fn difficulty_inputs(&self, _pos: Pos) -> (i64, i64, f32) {
        (0, 0, 0.0)
    }
}

/// The collision boxes the authored shape list gives a block by name (other
/// blocks are full cubes), in block-local units.
pub fn authored_collision_boxes(block: &Block) -> Vec<[f64; 6]> {
    if matches!(block.id.as_str(),"minecraft:vine"|"minecraft:weeping_vines"|"minecraft:weeping_vines_plant"|"minecraft:twisting_vines"|"minecraft:twisting_vines_plant"|"minecraft:cave_vines"|"minecraft:cave_vines_plant") {return Vec::new();}
    if block.id == "minecraft:redstone_wire" || block.id == "minecraft:lever" || block.id.ends_with("_button") {
        return Vec::new();
    }
    local_shapes(block).into_iter().map(|b| [b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z]).collect()
}

/// The connected half of a chest, according to ChestBlock's facing/type rule.
pub fn chest_connected_pos(pos: Pos, block: &Block) -> Option<Pos> {
    if block.id != "minecraft:chest" {
        return None;
    }
    let facing = block.property("facing").unwrap_or("north");
    let clockwise = match facing {
        "north" => (1, 0),
        "east" => (0, 1),
        "south" => (-1, 0),
        "west" => (0, -1),
        _ => return None,
    };
    let (dx, dz) = match block.property("type")? {
        "left" => clockwise,
        "right" => (-clockwise.0, -clockwise.1),
        _ => return None,
    };
    Some((pos.0 + dx, pos.1, pos.2 + dz))
}

pub fn chest_partner(world: &impl World, pos: Pos) -> Option<Pos> {
    let block = world.block(pos)?;
    let partner_pos = chest_connected_pos(pos, &block)?;
    let partner = world.block(partner_pos)?;
    let opposite = match block.property("type")? {
        "left" => "right",
        "right" => "left",
        _ => return None,
    };
    (partner.id == block.id
        && partner.property("facing") == block.property("facing")
        && partner.property("type") == Some(opposite)
        && chest_connected_pos(partner_pos, &partner) == Some(pos))
    .then_some(partner_pos)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub forward: f64,
    pub strafe: f64,
    pub jump: bool,
    pub crouch: bool,
    pub sprint: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Face {
    Down,
    Up,
    North,
    South,
    West,
    East,
}
impl Face {
    pub fn offset(self) -> Pos {
        match self {
            Self::Down => (0, -1, 0),
            Self::Up => (0, 1, 0),
            Self::North => (0, 0, -1),
            Self::South => (0, 0, 1),
            Self::West => (-1, 0, 0),
            Self::East => (1, 0, 0),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub pos: Pos,
    pub face: Face,
    pub distance: f64,
    pub point: DVec3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FluidRay {
    None,
    SourceOnly,
    Any,
}

#[derive(Clone, Copy, Debug)]
struct Box3 {
    min: DVec3,
    max: DVec3,
}
impl Box3 {
    fn new(min: DVec3, max: DVec3) -> Self {
        Self { min, max }
    }
    fn offset(self, v: DVec3) -> Self {
        Self::new(self.min + v, self.max + v)
    }
    fn intersects(self, b: Self) -> bool {
        self.min.x < b.max.x
            && self.max.x > b.min.x
            && self.min.y < b.max.y
            && self.max.y > b.min.y
            && self.min.z < b.max.z
            && self.max.z > b.min.z
    }
}

/// Whether a horizontal point lies in the extra half-height step of a stair.
/// The masks follow StairBlock's straight, inner and outer VoxelShapes.
pub fn stair_step_contains(facing: &str, shape: &str, x: f64, z: f64) -> bool {
    if !(0.0..1.0).contains(&x) || !(0.0..1.0).contains(&z) {
        return false;
    }
    let forward = match facing {
        "north" => z < 0.5,
        "east" => x >= 0.5,
        "south" => z >= 0.5,
        "west" => x < 0.5,
        _ => z < 0.5,
    };
    let left = match facing {
        "north" => x < 0.5,
        "east" => z < 0.5,
        "south" => x >= 0.5,
        "west" => z >= 0.5,
        _ => x < 0.5,
    };
    match shape {
        "outer_left" => forward && left,
        "outer_right" => forward && !left,
        "inner_left" => forward || left,
        "inner_right" => forward || !left,
        _ => forward,
    }
}

fn stair_direction(facing: &str) -> Pos {
    match facing {
        "north" => (0, 0, -1),
        "east" => (1, 0, 0),
        "south" => (0, 0, 1),
        "west" => (-1, 0, 0),
        _ => (0, 0, -1),
    }
}

fn stair_left(facing: &str) -> &'static str {
    match facing {
        "north" => "west",
        "east" => "north",
        "south" => "east",
        "west" => "south",
        _ => "west",
    }
}

fn stair_at(world: &impl World, pos: Pos) -> Option<Block> {
    world
        .block(pos)
        .filter(|block| block.id.ends_with("_stairs"))
}

fn stair_neighbor_pos(pos: Pos, facing: &str) -> Pos {
    let (dx, _, dz) = stair_direction(facing);
    (pos.0 + dx, pos.1, pos.2 + dz)
}

fn stair_can_take_shape(
    world: &impl World,
    pos: Pos,
    facing: &str,
    half: &str,
    side: &str,
) -> bool {
    stair_at(world, stair_neighbor_pos(pos, side)).is_none_or(|neighbor| {
        neighbor.property("facing") != Some(facing)
            || neighbor.property("half").unwrap_or("bottom") != half
    })
}

fn stair_shape_at(world: &impl World, pos: Pos, block: &Block) -> &'static str {
    let facing = block.property("facing").unwrap_or("north");
    let half = block.property("half").unwrap_or("bottom");
    let left = stair_left(facing);
    let perpendicular =
        |other: &str| matches!(facing, "north" | "south") != matches!(other, "north" | "south");
    if let Some(neighbor) = stair_at(world, stair_neighbor_pos(pos, facing)) {
        let neighbor_facing = neighbor.property("facing").unwrap_or("north");
        let opposite = match neighbor_facing {
            "north" => "south",
            "east" => "west",
            "south" => "north",
            "west" => "east",
            _ => "south",
        };
        if neighbor.property("half").unwrap_or("bottom") == half
            && perpendicular(neighbor_facing)
            && stair_can_take_shape(world, pos, facing, half, opposite)
        {
            return if neighbor_facing == left {
                "outer_left"
            } else {
                "outer_right"
            };
        }
    }
    let (dx, _, dz) = stair_direction(facing);
    let behind = (pos.0 - dx, pos.1, pos.2 - dz);
    if let Some(neighbor) = stair_at(world, behind) {
        let neighbor_facing = neighbor.property("facing").unwrap_or("north");
        if neighbor.property("half").unwrap_or("bottom") == half
            && perpendicular(neighbor_facing)
            && stair_can_take_shape(world, pos, facing, half, neighbor_facing)
        {
            return if neighbor_facing == left {
                "inner_left"
            } else {
                "inner_right"
            };
        }
    }
    "straight"
}

/// StairBlock.getStateForPlacement for the authored-world placement context.
pub fn stair_placement_state(
    world: &impl World,
    pos: Pos,
    block: Block,
    yaw: f64,
    clicked_face: Face,
    click_y: f64,
) -> Block {
    let yaw = yaw.rem_euclid(360.0);
    let facing = if yaw < 45.0 || yaw >= 315.0 {
        "south"
    } else if yaw < 135.0 {
        "west"
    } else if yaw < 225.0 {
        "north"
    } else {
        "east"
    };
    let half = if clicked_face == Face::Down
        || (clicked_face != Face::Up && click_y - pos.1 as f64 > 0.5)
    {
        "top"
    } else {
        "bottom"
    };
    let waterlogged = source_water_at(world, pos);
    let block = block
        .with("facing", facing)
        .with("half", half)
        .with("waterlogged", if waterlogged { "true" } else { "false" });
    let shape = stair_shape_at(world, pos, &block);
    block.with("shape", shape)
}

fn source_water_at(world: &impl World, pos: Pos) -> bool {
    world.block(pos).is_some_and(|existing| {
        existing.id == "minecraft:water" && existing.property("level").unwrap_or("0") == "0"
    })
}

/// BaseRailBlock.getStateForPlacement for the flat default rail state.
pub fn rail_placement_state(world: &impl World, pos: Pos, block: Block, yaw: f64) -> Block {
    let yaw = yaw.rem_euclid(360.0);
    let east_west = (45.0..135.0).contains(&yaw) || (225.0..315.0).contains(&yaw);
    let block = block.with(
        "shape",
        if east_west {
            "east_west"
        } else {
            "north_south"
        },
    );
    let block = if block.id == "minecraft:rail" {
        block
    } else {
        block.with("powered", "false")
    };
    block.with(
        "waterlogged",
        if source_water_at(world, pos) {
            "true"
        } else {
            "false"
        },
    )
}

pub fn powered_rail_placement_state(world: &impl World, pos: Pos, block: Block, yaw: f64) -> Block {
    rail_placement_state(world, pos, block, yaw)
}

/// SlabBlock.canBeReplaced for a matching slab item on the clicked block.
pub fn slab_placement_target(
    world: &impl World,
    clicked_pos: Pos,
    clicked_face: Face,
    click_y: f64,
    slab_id: &str,
) -> Pos {
    let above = click_y - clicked_pos.1 as f64 > 0.5;
    let horizontal = matches!(
        clicked_face,
        Face::North | Face::East | Face::South | Face::West
    );
    let replaces_clicked = world.block(clicked_pos).is_some_and(|existing| {
        if existing.id != slab_id {
            return false;
        }
        match existing.property("type").unwrap_or("bottom") {
            "bottom" => clicked_face == Face::Up || (horizontal && above),
            "top" => clicked_face == Face::Down || (horizontal && !above),
            _ => false,
        }
    });
    if replaces_clicked {
        clicked_pos
    } else {
        let (dx, dy, dz) = clicked_face.offset();
        (clicked_pos.0 + dx, clicked_pos.1 + dy, clicked_pos.2 + dz)
    }
}

/// SlabBlock.getStateForPlacement after BlockPlaceContext resolves the target.
pub fn slab_placement_state(
    world: &impl World,
    pos: Pos,
    block: Block,
    clicked_face: Face,
    click_y: f64,
) -> Block {
    if let Some(existing) = world.block(pos).filter(|existing| existing.id == block.id) {
        return existing.with("type", "double").with("waterlogged", "false");
    }
    let half = if clicked_face == Face::Down
        || (clicked_face != Face::Up && click_y - pos.1 as f64 > 0.5)
    {
        "top"
    } else {
        "bottom"
    };
    block.with("type", half).with(
        "waterlogged",
        if source_water_at(world, pos) {
            "true"
        } else {
            "false"
        },
    )
}

/// Apply StairBlock.getStairsShape to a placed stair and its horizontal
/// neighbors after a world edit. Returns every extra position whose shape changed.
pub fn update_stair_shapes(world: &mut impl World, changed: &[Pos]) -> Vec<Pos> {
    let mut candidates = BTreeSet::new();
    for &pos in changed {
        candidates.insert(pos);
        for facing in ["north", "east", "south", "west"] {
            candidates.insert(stair_neighbor_pos(pos, facing));
        }
    }
    let mut updates = Vec::new();
    for pos in candidates {
        let Some(mut block) = stair_at(world, pos) else {
            continue;
        };
        let shape = stair_shape_at(world, pos, &block);
        if block.property("shape") != Some(shape) {
            block.properties.insert("shape".into(), shape.into());
            updates.push((pos, block));
        }
    }
    let positions = updates.iter().map(|(pos, _)| *pos).collect();
    for (pos, block) in updates {
        world.set_block(pos, Some(block));
    }
    positions
}
fn local_shapes(block: &Block) -> Vec<Box3> {
    let id = block.id.rsplit(':').next().unwrap_or(&block.id);
    if matches!(
        id,
        "air" | "cave_air" | "void_air" | "water" | "lava" | "short_grass" | "tall_grass" | "torch" | "fire" | "soul_fire" | "sweet_berry_bush" | "cobweb"
    ) {
        return vec![];
    }
    let unit = Box3::new(DVec3::ZERO, DVec3::ONE);
    if id=="ladder" {
        let (a,b)=match block.property("facing").unwrap_or("north") {
            "south"=>([0.,0.,0.],[1.,1.,3./16.]),
            "east"=>([0.,0.,0.],[3./16.,1.,1.]),
            "west"=>([13./16.,0.,0.],[1.,1.,1.]),
            _=>([0.,0.,13./16.],[1.,1.,1.]),
        };return vec![Box3::new(DVec3::from_array(a),DVec3::from_array(b))];
    }
    if id=="vine" {
        let mut shapes=vec![];
        for (face,a,b) in [
            ("north",[0.,0.,0.],[1.,1.,1./16.]),("south",[0.,0.,15./16.],[1.,1.,1.]),
            ("west",[0.,0.,0.],[1./16.,1.,1.]),("east",[15./16.,0.,0.],[1.,1.,1.]),
            ("up",[0.,15./16.,0.],[1.,1.,1.])
        ] {if block.property(face)==Some("true") {shapes.push(Box3::new(DVec3::from_array(a),DVec3::from_array(b)));}}
        return shapes;
    }
    if matches!(id,"weeping_vines"|"weeping_vines_plant"|"twisting_vines"|"twisting_vines_plant"|"cave_vines"|"cave_vines_plant") {return vec![Box3::new(DVec3::new(0.25,0.,0.25),DVec3::new(0.75,1.,0.75))];}
    if id=="scaffolding" {return vec![Box3::new(DVec3::new(0.,14./16.,0.),DVec3::ONE)];}
    // `CactusBlock`: a column 14 wide and 15 tall; campfires 7 tall.
    if id == "cactus" {
        return vec![Box3::new(DVec3::new(1.0 / 16.0, 0.0, 1.0 / 16.0), DVec3::new(15.0 / 16.0, 15.0 / 16.0, 15.0 / 16.0))];
    }
    if matches!(id, "campfire" | "soul_campfire") {
        return vec![Box3::new(DVec3::ZERO, DVec3::new(1.0, 7.0 / 16.0, 1.0))];
    }
    if id == "redstone_wire" {
        return vec![Box3::new(DVec3::ZERO, DVec3::new(1.0, 1.0 / 16.0, 1.0))];
    }
    if id == "daylight_detector" {
        return vec![Box3::new(DVec3::ZERO, DVec3::new(1.0, 6.0 / 16.0, 1.0))];
    }
    if matches!(id, "farmland" | "dirt_path") {
        // FarmlandBlock and PathBlock both expose a 15/16-high column.
        return vec![Box3::new(DVec3::ZERO, DVec3::new(1.0, 15.0 / 16.0, 1.0))];
    }
    if id == "lever" || id.ends_with("_button") {
        return vec![Box3::new(
            DVec3::new(0.25, 0.0, 0.25),
            DVec3::new(0.75, 0.375, 0.75),
        )];
    }
    if id.ends_with("_slab") {
        return match block.property("type").unwrap_or("bottom") {
            "double" => vec![unit],
            "top" => vec![Box3::new(DVec3::new(0.0, 0.5, 0.0), DVec3::ONE)],
            _ => vec![Box3::new(DVec3::ZERO, DVec3::new(1.0, 0.5, 1.0))],
        };
    }
    if id.ends_with("_trapdoor") {
        // TrapDoorBlock.getShape selects one of Shapes.rotateAll(boxZ(16, 13, 16)).
        // The six boxes below were independently observed in the 26.3 harness.
        let three_sixteenths = 3.0 / 16.0;
        let thirteen_sixteenths = 13.0 / 16.0;
        let (min, max) = if block.property("open") == Some("true") {
            match block.property("facing").unwrap_or("north") {
                "south" => (DVec3::ZERO, DVec3::new(1.0, 1.0, three_sixteenths)),
                "east" => (DVec3::ZERO, DVec3::new(three_sixteenths, 1.0, 1.0)),
                "west" => (DVec3::new(thirteen_sixteenths, 0.0, 0.0), DVec3::ONE),
                _ => (DVec3::new(0.0, 0.0, thirteen_sixteenths), DVec3::ONE),
            }
        } else if block.property("half") == Some("top") {
            (DVec3::new(0.0, thirteen_sixteenths, 0.0), DVec3::ONE)
        } else {
            (DVec3::ZERO, DVec3::new(1.0, three_sixteenths, 1.0))
        };
        return vec![Box3::new(min, max)];
    }
    if id.ends_with("_fence_gate") {
        // FenceGateBlock.getCollisionShape: an open gate has no collision;
        // each closed axis is a 4/16-wide barrier, 1.5 blocks tall.
        if block.property("open") == Some("true") {
            return vec![];
        }
        return vec![
            if matches!(block.property("facing"), Some("east" | "west")) {
                Box3::new(DVec3::new(0.375, 0.0, 0.0), DVec3::new(0.625, 1.5, 1.0))
            } else {
                Box3::new(DVec3::new(0.0, 0.0, 0.375), DVec3::new(1.0, 1.5, 0.625))
            },
        ];
    }
    if id.ends_with("_door") {
        // DoorBlock.getShape uses a 3/16-thick sheet. Its open side depends
        // on facing and hinge, independently of upper/lower half.
        let open = block.property("open") == Some("true");
        let right = block.property("hinge") == Some("right");
        let side = match block.property("facing").unwrap_or("north") {
            "east" if open => {
                if right {
                    "south"
                } else {
                    "north"
                }
            }
            "south" if open => {
                if right {
                    "west"
                } else {
                    "east"
                }
            }
            "west" if open => {
                if right {
                    "north"
                } else {
                    "south"
                }
            }
            "north" if open => {
                if right {
                    "east"
                } else {
                    "west"
                }
            }
            "east" => "west",
            "south" => "north",
            "west" => "east",
            _ => "south",
        };
        let thickness = 3.0 / 16.0;
        return vec![match side {
            "north" => Box3::new(DVec3::ZERO, DVec3::new(1.0, 1.0, thickness)),
            "south" => Box3::new(DVec3::new(0.0, 0.0, 1.0 - thickness), DVec3::ONE),
            "west" => Box3::new(DVec3::ZERO, DVec3::new(thickness, 1.0, 1.0)),
            _ => Box3::new(DVec3::new(1.0 - thickness, 0.0, 0.0), DVec3::ONE),
        }];
    }
    if id.ends_with("_stairs") {
        let top = block.property("half") == Some("top");
        let base = if top {
            Box3::new(DVec3::new(0.0, 0.5, 0.0), DVec3::ONE)
        } else {
            Box3::new(DVec3::ZERO, DVec3::new(1.0, 0.5, 1.0))
        };
        let (lo, hi) = if top { (0.0, 0.5) } else { (0.5, 1.0) };
        let facing = block.property("facing").unwrap_or("north");
        let shape = block.property("shape").unwrap_or("straight");
        let mut boxes = vec![base];
        for x in 0..2 {
            for z in 0..2 {
                if stair_step_contains(facing, shape, x as f64 * 0.5 + 0.25, z as f64 * 0.5 + 0.25)
                {
                    boxes.push(Box3::new(
                        DVec3::new(x as f64 * 0.5, lo, z as f64 * 0.5),
                        DVec3::new(x as f64 * 0.5 + 0.5, hi, z as f64 * 0.5 + 0.5),
                    ));
                }
            }
        }
        return boxes;
    }
    if id == "chest" {
        let mut min = DVec3::new(1.0 / 16.0, 0.0, 1.0 / 16.0);
        let mut max = DVec3::new(15.0 / 16.0, 14.0 / 16.0, 15.0 / 16.0);
        if let Some((dx, _, dz)) = chest_connected_pos((0, 0, 0), block) {
            if dx < 0 {
                min.x = 0.0;
            }
            if dx > 0 {
                max.x = 1.0;
            }
            if dz < 0 {
                min.z = 0.0;
            }
            if dz > 0 {
                max.z = 1.0;
            }
        }
        return vec![Box3::new(min, max)];
    }
    vec![unit]
}

/// Collision boxes in block-local coordinates for client particles and other
/// consumers of the same measured player collision shapes.
pub fn collision_shape_boxes(block: &Block) -> Vec<([f64; 3], [f64; 3])> {
    local_shapes(block)
        .into_iter()
        .map(|shape| (shape.min.to_array(), shape.max.to_array()))
        .collect()
}

/// `Block.canSupportRigidBlock` tests whether the upward support face fills
/// the two-pixel-wide outer ring around a centered 12/16 column. For blocks
/// whose collision boxes represent their support shape, sample each texel of
/// that ring against the authored local shapes.
pub(crate) fn supports_rigid_top(block: &Block) -> bool {
    let id = block.id.rsplit(':').next().unwrap_or(&block.id);
    if id == "cauldron" {
        // AbstractCauldronBlock's hollow top still contains the complete
        // outer ring required by SupportType.RIGID.
        return true;
    }
    if id.ends_with("_fence") || id == "iron_bars" || id.ends_with("_pane") {
        // CrossCollisionBlock's post and arms leave the corner of the rigid
        // top-face ring uncovered, including with all four arms connected.
        return false;
    }
    let shapes = local_shapes(block);
    if shapes.iter().any(|shape| {
        shape.max.y >= 1.0
            && shape.min.x <= 0.0
            && shape.max.x >= 1.0
            && shape.min.z <= 0.0
            && shape.max.z >= 1.0
    }) {
        return true;
    }
    if !shapes.iter().any(|shape| shape.max.y >= 1.0) {
        return false;
    }
    for x in 0..16 {
        for z in 0..16 {
            if (2..14).contains(&x) && (2..14).contains(&z) {
                continue;
            }
            let px = (x as f64 + 0.5) / 16.0;
            let pz = (z as f64 + 0.5) / 16.0;
            if !shapes.iter().any(|shape| {
                shape.max.y >= 1.0
                    && shape.min.x <= px
                    && shape.max.x > px
                    && shape.min.z <= pz
                    && shape.max.z > pz
            }) {
                return false;
            }
        }
    }
    true
}
fn shapes_near(world: &impl World, bbox: Box3, pad: f64) -> Vec<Box3> {
    let min = (bbox.min - DVec3::splat(pad)).floor();
    let max = (bbox.max + DVec3::splat(pad)).ceil();
    let mut out = Vec::new();
    for x in min.x as i32..max.x as i32 {
        for y in min.y as i32..max.y as i32 {
            for z in min.z as i32..max.z as i32 {
                let offset = DVec3::new(x as f64, y as f64, z as f64);
                out.extend(world.collision_boxes((x, y, z)).into_iter().map(|b| {
                    Box3::new(DVec3::new(b[0], b[1], b[2]) + offset, DVec3::new(b[3], b[4], b[5]) + offset)
                }));
            }
        }
    }
    out
}

/// Pinned 26.3 block tags for authored worlds: `#blocks_motion` (and
/// `#entities_can_teleport_to`, which is it) as any block with a collision
/// shape; `#enderman_holdable` and `#enderman_does_not_teleport_to` with
/// their contents in full.
pub fn authored_block_in_tag(block: &Block, tag: &str, collision: &[[f64; 6]]) -> bool {
    let id = block.id.as_str();
    match tag {
        "minecraft:blocks_motion" | "minecraft:entities_can_teleport_to" => !collision.is_empty(),
        "minecraft:enderman_holdable" => matches!(
            id,
            // `#small_flowers`
            "minecraft:dandelion" | "minecraft:open_eyeblossom" | "minecraft:poppy" | "minecraft:blue_orchid" | "minecraft:allium"
                | "minecraft:azure_bluet" | "minecraft:red_tulip" | "minecraft:orange_tulip" | "minecraft:white_tulip"
                | "minecraft:pink_tulip" | "minecraft:oxeye_daisy" | "minecraft:cornflower" | "minecraft:lily_of_the_valley"
                | "minecraft:wither_rose" | "minecraft:torchflower" | "minecraft:closed_eyeblossom" | "minecraft:golden_dandelion"
                // `#dirt`, `#mud`, `#moss_blocks`, `#grass_blocks`
                | "minecraft:dirt" | "minecraft:coarse_dirt" | "minecraft:rooted_dirt" | "minecraft:mud" | "minecraft:muddy_mangrove_roots"
                | "minecraft:moss_block" | "minecraft:pale_moss_block" | "minecraft:grass_block" | "minecraft:podzol" | "minecraft:mycelium"
                | "minecraft:sand" | "minecraft:red_sand" | "minecraft:gravel" | "minecraft:brown_mushroom" | "minecraft:red_mushroom"
                | "minecraft:tnt" | "minecraft:cactus" | "minecraft:clay" | "minecraft:pumpkin" | "minecraft:carved_pumpkin"
                | "minecraft:melon" | "minecraft:crimson_fungus" | "minecraft:crimson_nylium" | "minecraft:crimson_roots"
                | "minecraft:warped_fungus" | "minecraft:warped_nylium" | "minecraft:warped_roots" | "minecraft:cactus_flower"
        ),
        "minecraft:enderman_does_not_teleport_to" => matches!(
            id,
            // `#dangerous_for_teleportation`
            "minecraft:fire" | "minecraft:soul_fire" | "minecraft:lava_cauldron" | "minecraft:campfire" | "minecraft:soul_campfire"
                | "minecraft:cactus" | "minecraft:magma_block" | "minecraft:sweet_berry_bush" | "minecraft:wither_rose"
                | "minecraft:pointed_dripstone" | "minecraft:powder_snow" | "minecraft:bedrock"
        ),
        _ => false,
    }
}

/// `FluidState.isEmpty` for an authored block: water, lava and their
/// kin, and anything waterlogged.
pub fn holds_fluid(block: &Block) -> bool {
    matches!(
        block.id.as_str(),
        "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column" | "minecraft:kelp" | "minecraft:kelp_plant" | "minecraft:seagrass" | "minecraft:tall_seagrass"
    ) || block.property("waterlogged") == Some("true")
}

/// Authored block collision boxes in world coordinates for projectile clips.
/// Body and projectile movement use the same shape definitions.
pub fn collision_boxes_at(world: &impl World, pos: Pos) -> Vec<(DVec3, DVec3)> {
    let offset = DVec3::new(pos.0 as f64, pos.1 as f64, pos.2 as f64);
    world
        .collision_boxes(pos)
        .into_iter()
        .map(|b| (DVec3::new(b[0], b[1], b[2]) + offset, DVec3::new(b[3], b[4], b[5]) + offset))
        .collect()
}
fn clip_axis(b: Box3, blocks: &[Box3], axis: usize, mut delta: f64) -> f64 {
    if blocks.is_empty() {
        return delta;
    }
    if delta.abs() < 1.0e-7 {
        return 0.0;
    }
    for s in blocks {
        let (a0, a1, c0, c1) = match axis {
            0 if b.min.y < s.max.y
                && b.max.y > s.min.y
                && b.min.z < s.max.z
                && b.max.z > s.min.z =>
            {
                (b.min.x, b.max.x, s.min.x, s.max.x)
            }
            1 if b.min.x < s.max.x
                && b.max.x > s.min.x
                && b.min.z < s.max.z
                && b.max.z > s.min.z =>
            {
                (b.min.y, b.max.y, s.min.y, s.max.y)
            }
            2 if b.min.x < s.max.x
                && b.max.x > s.min.x
                && b.min.y < s.max.y
                && b.max.y > s.min.y =>
            {
                (b.min.z, b.max.z, s.min.z, s.max.z)
            }
            _ => continue,
        };
        if delta > 0.0 && a1 <= c0 {
            delta = delta.min(c0 - a1);
        }
        if delta < 0.0 && a0 >= c1 {
            delta = delta.max(c1 - a0);
        }
    }
    delta
}
fn clip_motion(b: Box3, blocks: &[Box3], requested: DVec3) -> DVec3 {
    let dy = clip_axis(b, blocks, 1, requested.y);
    let b = b.offset(DVec3::new(0.0, dy, 0.0));
    let dx = clip_axis(b, blocks, 0, requested.x);
    let b = b.offset(DVec3::new(dx, 0.0, 0.0));
    let dz = clip_axis(b, blocks, 2, requested.z);
    DVec3::new(dx, dy, dz)
}
fn can_fall_at_least(world: &impl World, bbox: Box3, dx: f64, dz: f64, height: f64) -> bool {
    const EPS: f64 = 1.0e-7;
    let below = Box3::new(
        DVec3::new(
            bbox.min.x + EPS + dx,
            bbox.min.y - height - EPS,
            bbox.min.z + EPS + dz,
        ),
        DVec3::new(bbox.max.x - EPS + dx, bbox.min.y, bbox.max.z - EPS + dz),
    );
    !shapes_near(world, below, 0.0)
        .into_iter()
        .any(|shape| shape.intersects(below))
}

/// Find a standing spawn above the authored spawn column when its original
/// position is obstructed or flooded. The authored scene has no world-spawn
/// search radius yet, so this keeps its fixed x/z and checks successive feet
/// heights against the same player collision shapes used during movement.
pub fn safe_respawn_position(world: &impl World, spawn: DVec3) -> DVec3 {
    for y in spawn.y.floor() as i32..=319 {
        let candidate = DVec3::new(spawn.x, y as f64, spawn.z);
        let bbox = Box3::new(
            candidate + DVec3::new(-0.3, 0.0, -0.3),
            candidate + DVec3::new(0.3, 1.8, 0.3),
        );
        if shapes_near(world, bbox, 0.001)
            .into_iter()
            .any(|shape| shape.intersects(bbox))
            || can_fall_at_least(world, bbox, 0.0, 0.0, 0.1)
        {
            continue;
        }
        let x = spawn.x.floor() as i32;
        let z = spawn.z.floor() as i32;
        if (y..=y + 1).any(|yy| {
            world.block((x, yy, z)).is_some_and(|block| {
                matches!(block.id.as_str(), "minecraft:water" | "minecraft:lava")
            })
        }) {
            continue;
        }
        return candidate;
    }
    spawn
}
fn back_off_from_edge(world: &impl World, bbox: Box3, delta: DVec3) -> DVec3 {
    const STEP: f64 = 0.05;
    const MAX_DOWN_STEP: f64 = 0.6;
    let mut x = delta.x;
    let mut z = delta.z;
    let step_x = x.signum() * STEP;
    let step_z = z.signum() * STEP;
    while x != 0.0 && can_fall_at_least(world, bbox, x, 0.0, MAX_DOWN_STEP) {
        if x.abs() <= STEP {
            x = 0.0;
        } else {
            x -= step_x;
        }
    }
    while z != 0.0 && can_fall_at_least(world, bbox, 0.0, z, MAX_DOWN_STEP) {
        if z.abs() <= STEP {
            z = 0.0;
        } else {
            z -= step_z;
        }
    }
    while x != 0.0 && z != 0.0 && can_fall_at_least(world, bbox, x, z, MAX_DOWN_STEP) {
        if x.abs() <= STEP {
            x = 0.0;
        } else {
            x -= step_x;
        }
        if z.abs() <= STEP {
            z = 0.0;
        } else {
            z -= step_z;
        }
    }
    DVec3::new(x, delta.y, z)
}
fn modified_ground_input(input: Input, crouching: bool) -> (f64, f64) {
    let mut x = input.strafe.clamp(-1.0, 1.0) as f32;
    let mut z = input.forward.clamp(-1.0, 1.0) as f32;
    let raw_length = (x * x + z * z).sqrt();
    if raw_length > 0.0 {
        x /= raw_length;
        z /= raw_length;
        x *= 0.98_f32;
        z *= 0.98_f32;
    }
    if crouching {
        x *= 0.3_f32;
        z *= 0.3_f32;
    }
    let length = (x * x + z * z).sqrt();
    if length > 0.0 {
        let inverse_length = 1.0_f32 / length;
        let dx = x * inverse_length;
        let dz = z * inverse_length;
        let (small, large) = if dx.abs() < dz.abs() {
            (dx.abs(), dz.abs())
        } else {
            (dz.abs(), dx.abs())
        };
        let tangent = small / large;
        let distance_to_square = (1.0_f32 + tangent * tangent).sqrt();
        let new_length = (length * distance_to_square).min(1.0);
        x = dx * new_length;
        z = dz * new_length;
    }
    (x as f64, z as f64)
}

pub fn minecraft_sin_cos(yaw: f64) -> (f64, f64) {
    // Mth uses a 65536-entry float sine table. The cardinal headings retain
    // its tiny signed residuals, which matter for exact velocity bits.
    let radians = (yaw as f32) * ((std::f64::consts::PI / 180.0) as f32);
    const SCALE: f64 = 10430.378350470453;
    let scaled = radians as f64 * SCALE;
    let sine_index = ((scaled as i64) & 65535) as usize;
    let cosine_index = (((scaled + 16384.0) as i64) & 65535) as usize;
    let sample = |index: usize| ((index as f64 / SCALE).sin() as f32) as f64;
    (sample(sine_index), sample(cosine_index))
}

/// Where a hit comes from, for its knockback and hurt direction.
#[derive(Clone, Copy, Debug)]
pub enum HitFrom {
    /// A melee attacker's position (`DamageSource.getSourcePosition`).
    Position(DVec3),
    /// A projectile's velocity (`calculateHorizontalHurtKnockbackDirection`).
    Projectile(DVec3),
    /// An explosion (`#no_knockback`): no default knockback and no new hurt
    /// direction; the blast pushes the player itself.
    Explosion,
}

/// A hit on the player from a mob or its projectile.
#[derive(Clone, Copy, Debug)]
pub struct IncomingHit {
    pub damage: f32,
    pub from: HitFrom,
    /// `DamageSource.scalesWithDifficulty` (mob attacks and arrows from a
    /// living non-player).
    pub scales_with_difficulty: bool,
    /// The damage type's food exhaustion (0.1 for mob attacks and arrows).
    pub exhaustion: f32,
}

#[derive(Clone, Debug)]
pub struct Player {
    pub pos: DVec3, // feet
    pub velocity: DVec3,
    pub yaw: f64,   // Minecraft degrees: 0 south, 180 north
    pub pitch: f64, // degrees: positive down
    pub on_ground: bool,
    pub crouching: bool,
    previous_crouch_input: bool,
    pub sprinting: bool,
    pub flying: bool,
    /// `Abilities.flyingSpeed` (0.05; spectators change it with the mouse
    /// wheel, and it is kept across game modes).
    pub flying_speed: f32,
    pub mayfly: bool,
    pub spectator: bool,
    jump_trigger_time: u8,
    previous_jump_input: bool,
    pub swimming: bool,
    pub in_water: bool,
    /// Entity.checkFallDamage accumulates collision-clipped downward travel.
    pub fall_distance: f32,
    /// `LivingEntity.hurtTime` (10 after a full hit, down to 0) and the
    /// player's `hurtDir`: the hit's direction against its yaw.
    pub hurt_time: i32,
    pub hurt_dir: f32,
    /// `LivingEntity.attackStrengthTicker`: ticks since the last attack or
    /// change of main-hand item.
    pub attack_strength_ticker: i32,
    /// The main hand as the ticker last saw it (`lastItemInMainHand`).
    pub last_main_hand: Option<crate::inventory::ItemStack>,
    pub tick: u64,
    pub selected: usize,
    pub hotbar: [Block; 9],
    pub survival: SurvivalStatus,
}
impl Player {
    pub fn new(pos: DVec3) -> Self {
        let ids = [
            "stone",
            "dirt",
            "grass_block",
            "oak_planks",
            "cobblestone",
            "glass",
            "oak_slab",
            "oak_stairs",
            "water",
        ];
        Self {
            pos,
            velocity: DVec3::ZERO,
            yaw: 180.0,
            pitch: 0.0,
            on_ground: false,
            crouching: false,
            previous_crouch_input: false,
            sprinting: false,
            flying: false,
            flying_speed: 0.05,
            mayfly: true,
            spectator: false,
            jump_trigger_time: 0,
            previous_jump_input: false,
            swimming: false,
            in_water: false,
            fall_distance: 0.0,
            hurt_time: 0,
            hurt_dir: 0.0,
            attack_strength_ticker: 0,
            last_main_hand: None,
            tick: 0,
            selected: 0,
            hotbar: ids.map(|id| Block::new(format!("minecraft:{id}"))),
            survival: SurvivalStatus::default(),
        }
    }
    pub fn set_game_mode(&mut self, mode: GameMode) {
        self.mayfly = mode.may_fly();
        self.spectator = mode == GameMode::Spectator;
        if self.spectator {
            self.flying = true;
            self.on_ground = false;
        } else if !self.mayfly {
            self.flying = false;
        }
        self.fall_distance = 0.0;
    }
    pub fn eye_height(&self) -> f64 {
        if self.swimming {
            0.4
        } else if self.crouching {
            1.27
        } else {
            1.62
        }
    }
    pub fn eye(&self) -> DVec3 {
        self.pos + DVec3::new(0.0, self.eye_height(), 0.0)
    }
    pub fn look(&self) -> DVec3 {
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        DVec3::new(
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        )
    }
    fn bbox(&self) -> Box3 {
        let h = if self.swimming {
            0.6
        } else if self.crouching {
            1.5
        } else {
            1.8
        };
        Box3::new(
            self.pos + DVec3::new(-0.3, 0.0, -0.3),
            self.pos + DVec3::new(0.3, h, 0.3),
        )
    }

    /// Candidate validation for Consumables' random teleport. The caller
    /// supplies the three entity-RNG draws for each of up to 16 attempts.
    pub fn try_random_teleport_target(&mut self, world: &impl World, target: DVec3) -> bool {
        let mut y = target.y.clamp(-64.0, 319.0);
        while y.floor() > -64.0 {
            let below = (
                target.x.floor() as i32,
                y.floor() as i32 - 1,
                target.z.floor() as i32,
            );
            if world
                .block(below)
                .is_some_and(|block| !local_shapes(&block).is_empty())
            {
                let candidate = DVec3::new(target.x, y, target.z);
                let bbox = self.bbox().offset(candidate - self.pos);
                if !shapes_near(world, bbox, 0.001)
                    .iter()
                    .any(|shape| shape.intersects(bbox))
                {
                    let min = bbox.min.floor();
                    let max = bbox.max.ceil();
                    let liquid = (min.x as i32..max.x as i32).any(|x| {
                        (min.y as i32..max.y as i32).any(|yy| {
                            (min.z as i32..max.z as i32).any(|z| {
                                world.block((x, yy, z)).is_some_and(|block| {
                                    block.id == "minecraft:water" || block.id == "minecraft:lava"
                                })
                            })
                        })
                    });
                    if !liquid {
                        self.pos = candidate;
                        self.fall_distance = 0.0;
                        return true;
                    }
                }
                return false;
            }
            y -= 1.0;
        }
        false
    }
    pub fn tick(&mut self, world: &impl World, input: Input) {
        self.tick += 1;
        // `LivingEntity.baseTick`.
        if self.hurt_time > 0 {
            self.hurt_time -= 1;
        }
        // LivingEntity.aiStep discards residual player motion before applying
        // this tick's input. This gives a finite, exact stopping distance.
        if self.velocity.x * self.velocity.x + self.velocity.z * self.velocity.z < 9.0e-6 {
            self.velocity.x = 0.0;
            self.velocity.z = 0.0;
        }
        if self.velocity.y.abs() < 0.003 {
            self.velocity.y = 0.0;
        }
        // 26.3 travelInAir samples ground friction before movement, including a jump tick.
        let was_on_ground = self.on_ground;
        // LocalPlayer.aiStep checks the newly polled jump against the prior
        // key state. Player.aiStep decrements the seven-tick window later.
        let mut just_toggled_flight = false;
        if self.mayfly && input.jump && !self.previous_jump_input {
            if self.jump_trigger_time == 0 {
                self.jump_trigger_time = 7;
            } else {
                self.flying = !self.flying;
                just_toggled_flight = true;
                self.jump_trigger_time = 0;
            }
        }
        self.previous_jump_input = input.jump;
        self.jump_trigger_time = self.jump_trigger_time.saturating_sub(1);
        // Entity.tick updates water contact and the swimming pose before
        // LocalPlayer.aiStep polls this tick's sprint input. Entering the pose
        // requires submerged eyes and water at the feet; staying in it only
        // requires continued sprinting and water contact.
        let feet = (
            self.pos.x.floor() as i32,
            self.pos.y.floor() as i32,
            self.pos.z.floor() as i32,
        );
        let torso = (feet.0, (self.pos.y + 0.8).floor() as i32, feet.2);
        self.in_water = !self.spectator
            && [feet, torso]
                .into_iter()
                .any(|p| world.block(p).is_some_and(|b| b.id == "minecraft:water"));
        let eye = self.eye();
        let eye_block = (
            eye.x.floor() as i32,
            eye.y.floor() as i32,
            eye.z.floor() as i32,
        );
        let under_water = self.in_water
            && world
                .block(eye_block)
                .is_some_and(|block| block.id == "minecraft:water");
        let feet_in_water = world
            .block(feet)
            .is_some_and(|block| block.id == "minecraft:water");
        self.swimming = !self.flying
            && self.sprinting
            && if self.swimming {
                self.in_water
            } else {
                under_water && feet_in_water
            };
        // LocalPlayer updates crouching from the previous KeyboardInput state
        // before polling this tick's keys.
        self.crouching = self.previous_crouch_input && !self.flying && !self.swimming;
        self.previous_crouch_input = input.crouch;
        // isSprintingPossible rejects shallow water for a standing player.
        // An existing swim sprint may continue while the eyes leave water.
        let shallow_water = self.in_water && !under_water;
        if input.sprint
            && input.forward > 0.0
            && !self.crouching
            && (self.flying || self.swimming || !shallow_water)
        {
            self.sprinting = true;
        } else if input.forward <= 0.0
            || self.crouching
            || (shallow_water && !self.swimming && !self.flying)
        {
            self.sprinting = false;
        }
        if just_toggled_flight && self.flying && self.on_ground {
            self.velocity.y = self.velocity.y.max(0.42_f32 as f64);
            if self.sprinting {
                let (sin, cos) = minecraft_sin_cos(self.yaw);
                self.velocity.x -= sin * 0.2;
                self.velocity.z += cos * 0.2;
            }
        }
        let (x, z) = modified_ground_input(input, self.crouching);
        let friction = ground_friction(world, self.pos);
        let speed = self.movement_speed();
        let acceleration = if self.in_water {
            0.02
        } else if was_on_ground {
            if friction > 0.6_f32 as f64 {
                let f = friction as f32;
                (speed * (0.21600002_f32 / (f * f * f))) as f64
            } else {
                speed as f64
            }
        } else {
            if self.flying {
                (self.flying_speed * if self.sprinting { 2.0_f32 } else { 1.0_f32 }) as f64
            } else if self.sprinting {
                0.025999999_f32 as f64
            } else {
                0.02_f32 as f64
            }
        };
        let (sin, cos) = minecraft_sin_cos(self.yaw);
        if self.flying {
            let direction = i8::from(input.jump) - i8::from(input.crouch);
            self.velocity.y += (direction as f32 * (self.flying_speed * 3.0_f32)) as f64;
        } else if self.in_water {
            if input.jump {
                self.velocity.y += 0.04;
            }
            if input.crouch {
                self.velocity.y -= 0.04;
            }
            self.velocity.y -= 0.005;
        } else if input.jump && self.on_ground {
            self.velocity.y = 0.42_f32 as f64;
            if self.sprinting {
                self.velocity.x -= sin * 0.2;
                self.velocity.z += cos * 0.2;
            }
        }
        self.velocity.x += (x * cos - z * sin) * acceleration;
        self.velocity.z += (z * cos + x * sin) * acceleration;
        // Player.travel restores this pre-travel Y speed after LivingEntity
        // movement, including on a tick where a block clips the descent.
        let flight_travel_y = self.velocity.y;
        let mut requested = self.velocity;
        let bbox = self.bbox();
        if self.crouching
            && requested.y <= 0.0
            && (was_on_ground || !can_fall_at_least(world, bbox, 0.0, 0.0, 0.6))
        {
            requested = back_off_from_edge(world, bbox, requested);
        }
        let swept = Box3::new(
            bbox.min.min(bbox.min + requested),
            bbox.max.max(bbox.max + requested),
        );
        let blocks: Vec<_> = if self.spectator {
            Vec::new()
        } else {
            shapes_near(world, bbox, requested.length() + 0.7)
                .into_iter()
                .filter(|shape| shape.intersects(swept))
                .collect()
        };
        let mut actual = if self.spectator {
            requested
        } else {
            clip_motion(bbox, &blocks, requested)
        };
        if !self.spectator && self.on_ground && (actual.x != requested.x || actual.z != requested.z)
        {
            let raised = clip_motion(bbox, &blocks, DVec3::new(requested.x, 0.6, requested.z));
            let down = clip_axis(bbox.offset(raised), &blocks, 1, -0.6);
            let stepped = raised + DVec3::new(0.0, down, 0.0);
            if stepped.x * stepped.x + stepped.z * stepped.z
                > actual.x * actual.x + actual.z * actual.z
            {
                actual = stepped;
            }
        }
        self.pos += actual;
        self.on_ground = !self.spectator && requested.y < 0.0 && actual.y > requested.y;
        if (actual.x - requested.x).abs() >= 1.0e-7 {
            self.velocity.x = 0.0;
        }
        if (actual.z - requested.z).abs() >= 1.0e-7 {
            self.velocity.z = 0.0;
        }
        if actual.y != requested.y {
            self.velocity.y = 0.0;
        }
        // 26.3 travelInAir applies gravity after Entity.move has clipped the
        // requested displacement, including a grounded tick.
        if !self.flying && !self.in_water {
            self.velocity.y -= 0.08;
        }
        let drag = if self.in_water {
            0.8
        } else if was_on_ground {
            ((friction as f32) * 0.91_f32) as f64
        } else {
            0.91_f32 as f64
        };
        self.velocity.x *= drag;
        self.velocity.z *= drag;
        self.velocity.y = if self.flying {
            flight_travel_y * 0.6
        } else if self.in_water {
            self.velocity.y * 0.8
        } else {
            self.velocity.y * 0.98_f32 as f64
        };
        if self.flying {
            self.fall_distance = 0.0;
            if self.on_ground && !self.spectator {
                self.flying = false;
            }
        }
    }
    /// `getSpeed`: the movement speed attribute (`AttributeInstance`,
    /// base 0.1) with its `ADD_MULTIPLIED_TOTAL` modifiers, sprinting's 30%
    /// and slowness's -15% a level (applied in that order here; vanilla's
    /// order is its modifier map's).
    pub fn movement_speed(&self) -> f32 {
        let mut value = f64::from(0.1_f32);
        if self.sprinting {
            value *= 1.0 + f64::from(0.3_f32);
        }
        if let Some(slowness) = self.survival.effect(survival::EffectKind::Slowness) {
            value *= 1.0 + f64::from(-0.15_f32) * (f64::from(slowness.amplifier) + 1.0);
        }
        value.clamp(0.0, 1024.0) as f32
    }

    /// Run one server-style survival tick around the shared movement core.
    /// `Player.hurtServer` for a mob's hit: the damage type's difficulty
    /// scaling, then `LivingEntity.hurtServer` — the cooldown, armor,
    /// resistance and absorption (`SurvivalStatus::hurt`), and for a full
    /// hit the default knockback (`dealDefaultKnockback`: from the
    /// attacker, or along a projectile's flight) and the hurt animation
    /// (`indicateDamage`). Returns whether the player was hurt.
    pub fn hurt_by(&mut self, hit: &IncomingHit, difficulty: Difficulty, armor: survival::Armor, knockback_resistance: f64) -> bool {
        if self.survival.health <= 0.0 {
            return false;
        }
        let mut damage = hit.damage;
        if hit.scales_with_difficulty {
            damage = match difficulty {
                Difficulty::Peaceful => 0.0,
                Difficulty::Easy => (damage / 2.0 + 1.0).min(damage),
                Difficulty::Normal => damage,
                Difficulty::Hard => damage * 3.0 / 2.0,
            };
        }
        if damage == 0.0 {
            return false;
        }
        let Some(full) = self.survival.hurt(damage, armor, hit.exhaustion) else { return false };
        let direction = match hit.from {
            HitFrom::Position(p) => Some((p.x - self.pos.x, p.z - self.pos.z)),
            HitFrom::Projectile(velocity) => Some((-velocity.x, -velocity.z)),
            HitFrom::Explosion => None,
        };
        if !full {
            return true;
        }
        self.hurt_time = 10;
        // `dealDefaultKnockback` and `indicateDamage`, except for sources
        // without knockback.
        if let Some((mut xd, mut zd)) = direction {
            // `LivingEntity.knockback(0.4, xd, zd)`; a hit from straight
            // above gets no knockback here instead of a random direction.
            let power = f64::from(0.4f32) * (1.0 - knockback_resistance);
            if power > 0.0 && xd * xd + zd * zd >= f64::from(1.0e-5f32) {
                let length = (xd * xd + zd * zd).sqrt();
                (xd, zd) = (xd / length * power, zd / length * power);
                let v = self.velocity;
                let y = if self.on_ground { (v.y / 2.0 + power).min(0.4) } else { v.y };
                self.velocity = DVec3::new(v.x / 2.0 - xd, y, v.z / 2.0 - zd);
            }
            let (xd, zd) = direction.unwrap();
            // `ServerPlayer.indicateDamage`: `Mth.atan2` in degrees.
            self.hurt_dir = (mth::atan2(zd, xd) * mth::RAD_TO_DEG - self.yaw) as f32;
        }
        true
    }

    /// `Player.tick`'s attack strength: it counts up each tick and starts
    /// over when a different item comes into the main hand (a changed count
    /// or components of the same item keeps it).
    pub fn tick_attack_strength(&mut self, held: Option<&crate::inventory::ItemStack>) {
        self.attack_strength_ticker += 1;
        if self.last_main_hand.as_ref() != held {
            if self.last_main_hand.as_ref().map(|s| &s.id) != held.map(|s| &s.id) {
                self.attack_strength_ticker = 0;
            }
            self.last_main_hand = held.cloned();
        }
    }

    /// `getAttackStrengthScale(adjust)` with the main hand's attack speed:
    /// how far the attack has recharged.
    pub fn attack_strength_scale(&self, adjust: f32, attack_speed: f64) -> f32 {
        let delay = (1.0 / attack_speed * 20.0) as f32;
        ((self.attack_strength_ticker as f32 + adjust) / delay).clamp(0.0, 1.0)
    }

    pub fn tick_survival(&mut self, world: &impl World, input: Input) {
        self.tick_survival_with_rules(world, input, true, Difficulty::Normal);
    }
    pub fn tick_survival_with_regen(
        &mut self,
        world: &impl World,
        input: Input,
        natural_regeneration: bool,
    ) {
        self.tick_survival_with_rules(world, input, natural_regeneration, Difficulty::Normal);
    }
    pub fn tick_survival_with_rules(
        &mut self,
        world: &impl World,
        input: Input,
        natural_regeneration: bool,
        difficulty: Difficulty,
    ) {
        self.mayfly = false;
        self.flying = false;
        self.survival.tick_effects();
        let feet = (
            self.pos.x.floor() as i32,
            self.pos.y.floor() as i32,
            self.pos.z.floor() as i32,
        );
        let in_lava = world
            .block(feet)
            .is_some_and(|block| block.id == "minecraft:lava");
        let in_water = world
            .block(feet)
            .is_some_and(|block| block.id == "minecraft:water");
        self.survival.tick_fire(in_lava, in_water);
        let eye = self.eye().floor();
        let eye_block = (eye.x as i32, eye.y as i32, eye.z as i32);
        let submerged = world
            .block(eye_block)
            .is_some_and(|block| block.id == "minecraft:water");
        self.survival.tick_air(submerged, false);
        let before = self.pos;
        let jumped = input.jump && self.on_ground && !self.flying && !self.in_water;
        self.tick(world, input);
        let after_feet = (
            self.pos.x.floor() as i32,
            self.pos.y.floor() as i32,
            self.pos.z.floor() as i32,
        );
        if world
            .block(after_feet)
            .is_some_and(|block| block.id == "minecraft:lava")
        {
            self.survival.touch_lava();
        }
        let motion = self.pos - before;
        if self.flying || self.in_water {
            self.fall_distance = 0.0;
        } else {
            if motion.y < 0.0 {
                self.fall_distance -= motion.y as f32;
            }
            if self.on_ground {
                // The scene's solid terrain uses the ordinary fall reduction
                // and the default player SAFE_FALL_DISTANCE attribute (3.0).
                let damage = (self.fall_distance as f64 + 1.0e-6 - 3.0).floor() as f32;
                if damage > 0.0 {
                    self.survival.damage_with_difficulty(
                        damage,
                        difficulty,
                        DamageScaling::WhenCausedByLivingNonPlayer,
                        false,
                    );
                }
                self.fall_distance = 0.0;
            }
        }
        if jumped {
            self.survival
                .food
                .add_exhaustion(if self.sprinting { 0.2 } else { 0.05 });
        }
        if self.swimming || self.in_water {
            let distance = if self.swimming {
                motion.length()
            } else {
                motion.x.hypot(motion.z)
            };
            let centimeters = ((distance * 100.0) as f32).round() as i32;
            if centimeters > 0 {
                self.survival
                    .food
                    .add_exhaustion(0.01_f32 * centimeters as f32 * 0.01_f32);
            }
        } else if self.on_ground && self.sprinting {
            let centimeters = ((motion.x.hypot(motion.z) * 100.0) as f32).round() as i32;
            if centimeters > 0 {
                self.survival
                    .food
                    .add_exhaustion(0.1_f32 * centimeters as f32 * 0.01_f32);
            }
        }
        self.survival
            .tick_with_rules(natural_regeneration, difficulty, self.tick);
    }
    pub fn target(&self, world: &impl World, reach: f64) -> Option<Hit> {
        self.target_with_fluid_sources(world, reach, FluidRay::None)
    }
    /// ClipContext.Fluid.SOURCE_ONLY used by an empty bucket. Filled buckets
    /// use the ordinary block ray, so they can aim through water.
    pub fn target_source_fluid(&self, world: &impl World, reach: f64) -> Option<Hit> {
        self.target_with_fluid_sources(world, reach, FluidRay::SourceOnly)
    }
    /// Entity.pick(..., true) uses ClipContext.Fluid.ANY for the F3 targeted
    /// fluid entry, so flowing liquid also participates in the ray.
    pub fn target_any_fluid(&self, world: &impl World, reach: f64) -> Option<Hit> {
        self.target_with_fluid_sources(world, reach, FluidRay::Any)
    }
    fn target_with_fluid_sources(
        &self,
        world: &impl World,
        reach: f64,
        fluid_ray: FluidRay,
    ) -> Option<Hit> {
        let origin = self.eye();
        let dir = self.look();
        let mut best: Option<Hit> = None;
        let min = (origin - DVec3::splat(reach)).floor();
        let max = (origin + DVec3::splat(reach)).ceil();
        for x in min.x as i32..max.x as i32 {
            for y in min.y as i32..max.y as i32 {
                for z in min.z as i32..max.z as i32 {
                    let pos = (x, y, z);
                    // Block shapes stay within half a block of their cell:
                    // skip cells the ray cannot reach before the best hit.
                    let cell = Box3::new(DVec3::splat(-0.5), DVec3::splat(1.5)).offset(DVec3::new(x as f64, y as f64, z as f64));
                    let enter = ray_box(origin, dir, cell).map(|(d, _)| d);
                    if enter.is_none_or(|d| d > reach || best.as_ref().is_some_and(|b| d > b.distance)) {
                        continue;
                    }
                    if let Some(block) = world.block(pos) {
                        let mut shapes = local_shapes(&block);
                        if fluid_ray != FluidRay::None {
                            let waterlogged = block.property("waterlogged") == Some("true");
                            let liquid =
                                matches!(block.id.as_str(), "minecraft:water" | "minecraft:lava");
                            if waterlogged || liquid {
                                let level = block
                                    .property("level")
                                    .and_then(|value| value.parse::<u8>().ok())
                                    .unwrap_or(0)
                                    .min(15);
                                if fluid_ray == FluidRay::Any || waterlogged || level == 0 {
                                    let kind = if waterlogged {
                                        "minecraft:water"
                                    } else {
                                        block.id.as_str()
                                    };
                                    let same_above =
                                        world.block((x, y + 1, z)).is_some_and(|above| {
                                            above.id == kind
                                                || (kind == "minecraft:water"
                                                    && above.property("waterlogged")
                                                        == Some("true"))
                                        });
                                    let amount = if waterlogged || level == 0 || level >= 8 {
                                        8
                                    } else {
                                        8 - level
                                    };
                                    let height = if same_above { 1.0 } else { amount as f64 / 9.0 };
                                    shapes
                                        .push(Box3::new(DVec3::ZERO, DVec3::new(1.0, height, 1.0)));
                                }
                            }
                        }
                        for shape in shapes {
                            if let Some((distance, face)) = ray_box(
                                origin,
                                dir,
                                shape.offset(DVec3::new(x as f64, y as f64, z as f64)),
                            ) {
                                if distance <= reach
                                    && best.as_ref().is_none_or(|b| distance < b.distance)
                                {
                                    best = Some(Hit {
                                        pos,
                                        face,
                                        distance,
                                        point: origin + dir * distance,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        best
    }
    pub fn break_target(&self, world: &mut impl World) -> Option<Pos> {
        self.break_target_state(world).map(|(pos, _)| pos)
    }
    pub fn break_target_state(&self, world: &mut impl World) -> Option<(Pos, Block)> {
        let hit = self.target(world, 5.0)?;
        let old = world.block(hit.pos)?;
        world.set_block(hit.pos, None);
        Some((hit.pos, old))
    }
    pub fn place_target(&self, world: &mut impl World) -> Option<Pos> {
        self.place_target_with(world, self.hotbar[self.selected].clone())
    }
    pub fn place_selected(
        &self,
        world: &mut impl World,
        inventory: &mut Inventory,
        mode: GameMode,
    ) -> Option<Pos> {
        let slot = inventory.slots.get(self.selected)?.as_ref()?;
        if slot.count == 0 {
            return None;
        }
        if mode == GameMode::Spectator {
            return None;
        }
        if mode == GameMode::Adventure {
            let hit = self.target(world, 5.0)?;
            let clicked = world.block(hit.pos)?;
            if !slot.allows_adventure_block("minecraft:can_place_on", &clicked) {
                return None;
            }
        }
        let block = Block::new(if slot.id == "minecraft:redstone" {
            "minecraft:redstone_wire".to_owned()
        } else {
            slot.id.clone()
        });
        let placed = self.place_target_with(world, block)?;
        if mode.is_survival() {
            let selected = &mut inventory.slots[self.selected];
            let stack = selected.as_mut().unwrap();
            stack.count -= 1;
            if stack.count == 0 {
                *selected = None;
            }
        }
        Some(placed)
    }
    pub fn place_target_with(&self, world: &mut impl World, mut block: Block) -> Option<Pos> {
        let hit = self.target(world, 5.0)?;
        let pos = if block.id.ends_with("_slab") {
            slab_placement_target(world, hit.pos, hit.face, hit.point.y, &block.id)
        } else {
            let (dx, dy, dz) = hit.face.offset();
            (hit.pos.0 + dx, hit.pos.1 + dy, hit.pos.2 + dz)
        };
        let replaced = world.block(pos);
        let merge_slab = block.id.ends_with("_slab")
            && replaced.as_ref().is_some_and(|existing| {
                existing.id == block.id && existing.property("type") != Some("double")
            });
        if replaced
            .as_ref()
            .is_some_and(|existing| !local_shapes(existing).is_empty())
            && !merge_slab
        {
            return None;
        }
        // 26.3 block defaults/placement states: GrassBlock inherits snowy=false
        // from SpreadingSnowyBlock; RotatedPillarBlock uses the clicked axis;
        // LeavesBlock places persistent leaves at distance 7; LiquidBlock's
        // default level is 0. Missing these properties makes a valid placed
        // block impossible to resolve through its blockstate resource.
        if block.id == "minecraft:grass_block" {
            block = block.with("snowy", "false");
        }
        if matches!(
            block.id.as_str(),
            "minecraft:rail"
                | "minecraft:powered_rail"
                | "minecraft:activator_rail"
                | "minecraft:detector_rail"
        ) {
            block = rail_placement_state(world, pos, block, self.yaw);
        }
        if block.id == "minecraft:hopper" {
            let facing = match hit.face {
                Face::North => "south",
                Face::South => "north",
                Face::West => "east",
                Face::East => "west",
                Face::Down | Face::Up => "down",
            };
            block = block.with("facing", facing).with("enabled", "true");
        }
        if block.id.ends_with("_log") || block.id.ends_with("_wood") {
            let axis = match hit.face {
                Face::Up | Face::Down => "y",
                Face::East | Face::West => "x",
                Face::North | Face::South => "z",
            };
            block = block.with("axis", axis);
        }
        if block.id.ends_with("_leaves") {
            block = block
                .with("distance", "7")
                .with("persistent", "true")
                .with("waterlogged", "false");
        }
        if block.id == "minecraft:water" {
            block = block.with("level", "0");
        }
        if block.id == "minecraft:redstone_wire" {
            block = block
                .with("power", "0")
                .with("north", "none")
                .with("east", "none")
                .with("south", "none")
                .with("west", "none");
        }
        if block.id == "minecraft:redstone_lamp" {
            block = block.with("lit", "false");
        }
        if block.id == "minecraft:daylight_detector" {
            block = block.with("inverted", "false").with("power", "0");
        }
        if block.id == "minecraft:lever" || block.id.ends_with("_button") {
            let yaw = self.yaw.rem_euclid(360.0);
            let facing = if yaw < 45.0 || yaw >= 315.0 {
                "south"
            } else if yaw < 135.0 {
                "west"
            } else if yaw < 225.0 {
                "north"
            } else {
                "east"
            };
            let (face, facing) = match hit.face {
                Face::Up => ("floor", facing),
                Face::Down => ("ceiling", facing),
                Face::North => ("wall", "north"),
                Face::South => ("wall", "south"),
                Face::West => ("wall", "west"),
                Face::East => ("wall", "east"),
            };
            block = block
                .with("face", face)
                .with("facing", facing)
                .with("powered", "false");
        }
        if block.id.ends_with("_slab") {
            block = slab_placement_state(world, pos, block, hit.face, hit.point.y);
        }
        if block.id.ends_with("_stairs") {
            block = stair_placement_state(world, pos, block, self.yaw, hit.face, hit.point.y);
        }
        if matches!(
            block.id.as_str(),
            "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker"
        ) {
            let yaw = self.yaw.rem_euclid(360.0);
            let facing = if yaw < 45.0 || yaw >= 315.0 {
                "north"
            } else if yaw < 135.0 {
                "east"
            } else if yaw < 225.0 {
                "south"
            } else {
                "west"
            };
            block = block.with("facing", facing).with("lit", "false");
        }
        let mut chest_partner_to_update = None;
        if block.id == "minecraft:chest" {
            let yaw = self.yaw.rem_euclid(360.0);
            let facing = if yaw < 45.0 || yaw >= 315.0 {
                "north"
            } else if yaw < 135.0 {
                "east"
            } else if yaw < 225.0 {
                "south"
            } else {
                "west"
            };
            block = block
                .with("facing", facing)
                .with("type", "single")
                .with("waterlogged", "false");
            if !self.crouching {
                let clockwise = match facing {
                    "north" => (1, 0),
                    "east" => (0, 1),
                    "south" => (-1, 0),
                    _ => (0, -1),
                };
                for (offset, new_type, other_type) in [
                    (clockwise, "left", "right"),
                    ((-clockwise.0, -clockwise.1), "right", "left"),
                ] {
                    let other_pos = (pos.0 + offset.0, pos.1, pos.2 + offset.1);
                    if let Some(other) = world.block(other_pos).filter(|other| {
                        other.id == "minecraft:chest"
                            && other.property("type") == Some("single")
                            && other.property("facing") == Some(facing)
                    }) {
                        block = block.with("type", new_type);
                        chest_partner_to_update = Some((other_pos, other.with("type", other_type)));
                        break;
                    }
                }
            }
        }
        let player = self.bbox();
        if local_shapes(&block).iter().any(|s| {
            s.offset(DVec3::new(pos.0 as f64, pos.1 as f64, pos.2 as f64))
                .intersects(player)
        }) {
            return None;
        }
        world.set_block(pos, Some(block));
        if let Some((partner_pos, partner)) = chest_partner_to_update {
            world.set_block(partner_pos, Some(partner));
        }
        Some(pos)
    }
}
fn ground_friction(world: &impl World, pos: DVec3) -> f64 {
    let under = (
        pos.x.floor() as i32,
        (pos.y - 0.500001).floor() as i32,
        pos.z.floor() as i32,
    );
    match world.block(under).as_ref().map(|b| b.id.as_str()) {
        Some("minecraft:ice" | "minecraft:packed_ice" | "minecraft:frosted_ice") => 0.98,
        Some("minecraft:slime_block") => 0.8,
        _ => 0.6,
    }
}
fn ray_box(origin: DVec3, dir: DVec3, b: Box3) -> Option<(f64, Face)> {
    let mut near = f64::NEG_INFINITY;
    let mut far = f64::INFINITY;
    let mut face = Face::North;
    for (o, d, lo, hi, neg, pos) in [
        (origin.x, dir.x, b.min.x, b.max.x, Face::West, Face::East),
        (origin.y, dir.y, b.min.y, b.max.y, Face::Down, Face::Up),
        (origin.z, dir.z, b.min.z, b.max.z, Face::North, Face::South),
    ] {
        if d == 0.0 {
            if o < lo || o > hi {
                return None;
            }
            continue;
        }
        let t0 = (lo - o) / d;
        let t1 = (hi - o) / d;
        let (enter, exit, enter_face) = if t0 < t1 {
            (t0, t1, neg)
        } else {
            (t1, t0, pos)
        };
        if enter > near {
            near = enter;
            face = enter_face;
        }
        far = far.min(exit);
        if near > far {
            return None;
        }
    }
    if far < 0.0 {
        None
    } else {
        Some((near.max(0.0), face))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_stair_collision_union_matches_26_3_shape_gate() {
        // scenarios/stair-collision-shapes.json, two exact vanilla runs.
        for half in ["bottom", "top"] {
            for facing in ["north", "east", "south", "west"] {
                let block = Block::new("minecraft:oak_stairs")
                    .with("facing", facing)
                    .with("half", half)
                    .with("shape", "straight");
                let shapes = local_shapes(&block);
                for x in 0..2 {
                    for z in 0..2 {
                        for y in 0..2 {
                            let point = DVec3::new(
                                x as f64 * 0.5 + 0.25,
                                y as f64 * 0.5 + 0.25,
                                z as f64 * 0.5 + 0.25,
                            );
                            let inside = shapes.iter().any(|shape| {
                                point.cmpge(shape.min).all() && point.cmplt(shape.max).all()
                            });
                            let upper = y == 1;
                            let high_half = match facing {
                                "north" => z == 0,
                                "east" => x == 1,
                                "south" => z == 1,
                                "west" => x == 0,
                                _ => unreachable!(),
                            };
                            assert_eq!(
                                inside,
                                (upper == (half == "top")) || high_half,
                                "{half} {facing} cell {x},{y},{z}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn corner_stair_collision_union_matches_26_3_shape_gate() {
        // scenarios/stair-corner-collision-shapes.json; masks are the sampled
        // occupied upper/lower step quarters from both exact server runs.
        for (facing, masks) in [
            ("north", ["1110", "1101", "1000", "0100"]),
            ("east", ["1101", "0111", "0100", "0001"]),
            ("south", ["0111", "1011", "0001", "0010"]),
            ("west", ["1011", "1110", "0010", "1000"]),
        ] {
            for (shape, mask) in ["inner_left", "inner_right", "outer_left", "outer_right"]
                .into_iter()
                .zip(masks)
            {
                for half in ["bottom", "top"] {
                    let block = Block::new("minecraft:oak_stairs")
                        .with("facing", facing)
                        .with("half", half)
                        .with("shape", shape);
                    let boxes = local_shapes(&block);
                    for x in 0..2 {
                        for z in 0..2 {
                            for y in 0..2 {
                                let point = DVec3::new(
                                    x as f64 * 0.5 + 0.25,
                                    y as f64 * 0.5 + 0.25,
                                    z as f64 * 0.5 + 0.25,
                                );
                                let inside = boxes.iter().any(|bbox| {
                                    point.cmpge(bbox.min).all() && point.cmplt(bbox.max).all()
                                });
                                let step = mask.as_bytes()[z * 2 + x] == b'1';
                                let base = (y == 1) == (half == "top");
                                assert_eq!(
                                    inside,
                                    base || step,
                                    "{half} {facing} {shape} at {x},{y},{z}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    #[derive(Default)]
    struct TestWorld(BTreeMap<Pos, Block>);
    impl World for TestWorld {
        fn block(&self, p: Pos) -> Option<Block> {
            self.0.get(&p).cloned()
        }
        fn set_block(&mut self, p: Pos, b: Option<Block>) {
            if let Some(b) = b {
                self.0.insert(p, b);
            } else {
                self.0.remove(&p);
            }
        }
    }
    fn floor() -> TestWorld {
        let mut w = TestWorld::default();
        for x in -5..=5 {
            for z in -5..=5 {
                w.set_block((x, 0, z), Some(Block::new("minecraft:stone")));
            }
        }
        w
    }
    #[test]
    fn powered_rail_placement_uses_yaw_axis_and_source_water() {
        let mut world = floor();
        let pos = (0, 1, 0);
        let north =
            powered_rail_placement_state(&world, pos, Block::new("minecraft:powered_rail"), 0.0);
        assert_eq!(north.property("shape"), Some("north_south"));
        assert_eq!(north.property("powered"), Some("false"));
        assert_eq!(north.property("waterlogged"), Some("false"));
        world.set_block(pos, Some(Block::new("minecraft:water").with("level", "0")));
        let east =
            powered_rail_placement_state(&world, pos, Block::new("minecraft:powered_rail"), 90.0);
        assert_eq!(east.property("shape"), Some("east_west"));
        assert_eq!(east.property("waterlogged"), Some("true"));
    }
    #[test]
    fn trapdoor_collision_boxes_match_26_3_reference() {
        // scenarios/trapdoor-collision-shapes.json, two identical vanilla runs.
        let cases = [
            (
                "false",
                "bottom",
                "north",
                [0.0, 0.0, 0.0, 1.0, 0.1875, 1.0],
            ),
            ("false", "top", "north", [0.0, 0.8125, 0.0, 1.0, 1.0, 1.0]),
            ("true", "bottom", "north", [0.0, 0.0, 0.8125, 1.0, 1.0, 1.0]),
            ("true", "bottom", "south", [0.0, 0.0, 0.0, 1.0, 1.0, 0.1875]),
            ("true", "bottom", "east", [0.0, 0.0, 0.0, 0.1875, 1.0, 1.0]),
            ("true", "bottom", "west", [0.8125, 0.0, 0.0, 1.0, 1.0, 1.0]),
        ];
        for (open, half, facing, expected) in cases {
            let block = Block::new("minecraft:oak_trapdoor")
                .with("open", open)
                .with("half", half)
                .with("facing", facing);
            let boxes = local_shapes(&block);
            assert_eq!(boxes.len(), 1);
            let bounds = boxes[0];
            assert_eq!(
                [
                    bounds.min.x,
                    bounds.min.y,
                    bounds.min.z,
                    bounds.max.x,
                    bounds.max.y,
                    bounds.max.z
                ],
                expected,
                "state open={open} half={half} facing={facing}"
            );
        }
    }

    #[test]
    fn fence_gate_collision_boxes_match_26_3_reference() {
        // scenarios/fence-gate-collision-shapes.json, two identical runs.
        let cases = [
            ("north", false, Some([0.0, 0.0, 0.375, 1.0, 1.5, 0.625])),
            ("east", false, Some([0.375, 0.0, 0.0, 0.625, 1.5, 1.0])),
            ("north", true, None),
            ("east", true, None),
        ];
        for (facing, open, expected) in cases {
            let block = Block::new("minecraft:oak_fence_gate")
                .with("facing", facing)
                .with("open", if open { "true" } else { "false" });
            let shapes = local_shapes(&block);
            match expected {
                Some(values) => {
                    assert_eq!(shapes.len(), 1);
                    assert_eq!(shapes[0].min.to_array(), values[0..3]);
                    assert_eq!(shapes[0].max.to_array(), values[3..6]);
                }
                None => assert!(shapes.is_empty()),
            }
        }
    }
    #[test]
    fn oak_door_collision_boxes_match_26_3_reference() {
        // scenarios/door-collision-shapes.json, two identical vanilla runs.
        let cases = [
            ("north", "left", false, [0.0, 0.0, 0.8125, 1.0, 1.0, 1.0]),
            ("east", "left", false, [0.0, 0.0, 0.0, 0.1875, 1.0, 1.0]),
            ("north", "left", true, [0.0, 0.0, 0.0, 0.1875, 1.0, 1.0]),
            ("north", "right", true, [0.8125, 0.0, 0.0, 1.0, 1.0, 1.0]),
            ("east", "left", true, [0.0, 0.0, 0.0, 1.0, 1.0, 0.1875]),
            ("east", "right", true, [0.0, 0.0, 0.8125, 1.0, 1.0, 1.0]),
            ("south", "left", true, [0.8125, 0.0, 0.0, 1.0, 1.0, 1.0]),
            ("south", "right", true, [0.0, 0.0, 0.0, 0.1875, 1.0, 1.0]),
            ("west", "left", true, [0.0, 0.0, 0.8125, 1.0, 1.0, 1.0]),
            ("west", "right", true, [0.0, 0.0, 0.0, 1.0, 1.0, 0.1875]),
        ];
        for (facing, hinge, open, expected) in cases {
            let block = Block::new("minecraft:oak_door")
                .with("facing", facing)
                .with("hinge", hinge)
                .with("open", if open { "true" } else { "false" });
            let shapes = local_shapes(&block);
            assert_eq!(shapes.len(), 1);
            assert_eq!(shapes[0].min.to_array(), expected[0..3]);
            assert_eq!(shapes[0].max.to_array(), expected[3..6]);
        }
    }
    #[test]
    fn respawn_uses_first_dry_supported_height_in_authored_column() {
        let mut world = floor();
        let spawn = DVec3::new(0.5, 1.0, 4.5);
        assert_eq!(safe_respawn_position(&world, spawn), spawn);
        for y in 1..=3 {
            world.set_block((0, y, 4), Some(Block::new("minecraft:water")));
        }
        world.set_block((0, 4, 4), Some(Block::new("minecraft:stone")));
        assert_eq!(
            safe_respawn_position(&world, spawn),
            DVec3::new(0.5, 5.0, 4.5)
        );
    }
    fn water_pool(layers: &[i32]) -> TestWorld {
        let mut world = floor();
        for x in -3..=3 {
            for z in -3..=3 {
                for &y in layers {
                    world.set_block((x, y, z), Some(Block::new("minecraft:water")));
                }
            }
        }
        world
    }
    #[test]
    fn chorus_target_descends_to_support_and_rejects_liquid() {
        let world = floor();
        let mut player = Player::new(DVec3::new(0.5, 1.0, 0.5));
        player.fall_distance = 4.0;
        assert!(player.try_random_teleport_target(&world, DVec3::new(1.5, 8.25, 1.5)));
        assert_eq!(player.pos, DVec3::new(1.5, 1.25, 1.5));
        assert_eq!(player.fall_distance, 0.0);
        assert!(!player.try_random_teleport_target(&world, DVec3::new(10.5, 8.0, 0.5)));
        let mut wet = floor();
        wet.set_block((2, 1, 2), Some(Block::new("minecraft:water")));
        assert!(!player.try_random_teleport_target(&wet, DVec3::new(2.5, 8.0, 2.5)));
    }
    #[test]
    fn swim_sprint_starts_underwater_but_not_in_one_block_deep_water() {
        let sprint = Input {
            forward: 1.0,
            sprint: true,
            ..Default::default()
        };
        let mut shallow = Player::new(DVec3::new(0.5, 1.0, 0.5));
        shallow.on_ground = true;
        let shallow_pool = water_pool(&[1]);
        for _ in 0..5 {
            shallow.tick(&shallow_pool, sprint);
            assert!(shallow.in_water);
            assert!(!shallow.swimming);
            assert!(!shallow.sprinting);
            assert_eq!(shallow.eye_height(), 1.62);
        }

        let mut deep = Player::new(DVec3::new(0.5, 1.0, 0.5));
        deep.on_ground = true;
        let deep_pool = water_pool(&[1, 2]);
        deep.tick(&deep_pool, sprint);
        assert!(deep.sprinting);
        assert!(!deep.swimming);
        deep.tick(&deep_pool, sprint);
        assert!(deep.swimming);
        assert_eq!(deep.eye_height(), 0.4);
    }
    #[test]
    fn an_existing_swim_continues_into_shallow_water() {
        let mut player = Player::new(DVec3::new(0.5, 1.8, 0.5));
        player.swimming = true;
        player.sprinting = true;
        player.tick(
            &water_pool(&[1]),
            Input {
                forward: 1.0,
                sprint: true,
                ..Default::default()
            },
        );
        assert!(player.in_water);
        assert!(player.swimming);
        assert!(player.sprinting);
    }
    #[test]
    fn chest_partner_requires_matching_halves_and_facing() {
        let mut world = TestWorld::default();
        let right = (0, 1, 0);
        let left = (1, 1, 0);
        world.set_block(
            right,
            Some(
                Block::new("minecraft:chest")
                    .with("facing", "south")
                    .with("type", "right"),
            ),
        );
        world.set_block(
            left,
            Some(
                Block::new("minecraft:chest")
                    .with("facing", "south")
                    .with("type", "left"),
            ),
        );
        assert_eq!(chest_partner(&world, right), Some(left));
        assert_eq!(chest_partner(&world, left), Some(right));
        world.set_block(
            left,
            Some(
                Block::new("minecraft:chest")
                    .with("facing", "north")
                    .with("type", "left"),
            ),
        );
        assert_eq!(chest_partner(&world, right), None);
    }
    #[test]
    fn placing_adjacent_chests_links_halves_without_replacing_their_blocks() {
        let mut world = floor();
        for x in 0..=1 {
            world.set_block((x, 2, 4), Some(Block::new("minecraft:stone")));
            let mut player = Player::new(DVec3::new(x as f64 + 0.5, 1.0, 1.5));
            player.yaw = 0.0;
            assert_eq!(
                player.place_target_with(&mut world, Block::new("minecraft:chest")),
                Some((x, 2, 3))
            );
        }
        assert_eq!(
            world.block((0, 2, 3)).unwrap().property("type"),
            Some("left")
        );
        assert_eq!(
            world.block((1, 2, 3)).unwrap().property("type"),
            Some("right")
        );
        assert_eq!(chest_partner(&world, (0, 2, 3)), Some((1, 2, 3)));
        assert_eq!(chest_partner(&world, (1, 2, 3)), Some((0, 2, 3)));
        world.set_block((2, 2, 4), Some(Block::new("minecraft:stone")));
        let mut third = Player::new(DVec3::new(2.5, 1.0, 1.5));
        third.yaw = 0.0;
        assert_eq!(
            third.place_target_with(&mut world, Block::new("minecraft:chest")),
            Some((2, 2, 3))
        );
        assert_eq!(
            world.block((2, 2, 3)).unwrap().property("type"),
            Some("single")
        );
        assert_eq!(
            world.block((1, 2, 3)).unwrap().property("type"),
            Some("right")
        );
    }
    #[test]
    fn sprint_jump_charges_survival_food_exhaustion() {
        let w = floor();
        let mut p = Player::new(DVec3::new(0.5, 1.0, 0.5));
        p.on_ground = true;
        p.tick_survival(
            &w,
            Input {
                forward: 1.0,
                sprint: true,
                jump: true,
                ..Default::default()
            },
        );
        assert!(p.pos.y > 1.0);
        assert_eq!(p.survival.food.exhaustion, 0.2);
        assert_eq!(p.survival.food.level, 20);
    }
    #[test]
    fn creative_flight_requires_two_new_jump_presses_within_seven_ticks() {
        let world = floor();
        let mut player = Player::new(DVec3::new(0.5, 1.0, 0.5));
        player.on_ground = true;
        player.tick(
            &world,
            Input {
                jump: true,
                ..Default::default()
            },
        );
        assert!(!player.flying);
        player.tick(&world, Input::default());
        player.tick(
            &world,
            Input {
                jump: true,
                ..Default::default()
            },
        );
        assert!(player.flying);
        player.tick(&world, Input::default());
        player.tick(
            &world,
            Input {
                jump: true,
                ..Default::default()
            },
        );
        assert!(player.flying);
        player.tick(&world, Input::default());
        player.tick(
            &world,
            Input {
                jump: true,
                ..Default::default()
            },
        );
        assert!(!player.flying);

        let mut late = Player::new(DVec3::new(0.5, 1.0, 0.5));
        late.on_ground = true;
        late.tick(
            &world,
            Input {
                jump: true,
                ..Default::default()
            },
        );
        for _ in 0..8 {
            late.tick(&world, Input::default());
        }
        late.tick(
            &world,
            Input {
                jump: true,
                ..Default::default()
            },
        );
        assert!(!late.flying);
    }
    #[test]
    fn creative_flight_uses_additive_vertical_impulse_and_momentum() {
        let world = floor();
        let mut player = Player::new(DVec3::new(0.5, 5.0, 0.5));
        player.flying = true;
        player.tick(
            &world,
            Input {
                jump: true,
                ..Default::default()
            },
        );
        let initial = (0.05_f32 * 3.0_f32) as f64;
        assert_eq!(player.pos.y, 5.0 + initial);
        assert_eq!(player.velocity.y, initial * 0.6);
        let before = player.pos.y;
        player.tick(&world, Input::default());
        assert_eq!(player.pos.y, before + initial * 0.6);
        let before = player.pos.y;
        player.tick(
            &world,
            Input {
                crouch: true,
                ..Default::default()
            },
        );
        assert!(player.pos.y < before);
    }
    #[test]
    fn landing_ends_creative_flight_and_survival_cannot_start_it() {
        let world = floor();
        let mut landing = Player::new(DVec3::new(0.5, 1.1, 0.5));
        landing.velocity.y = -0.3;
        landing.flying = true;
        landing.tick(&world, Input::default());
        assert!(landing.on_ground);
        assert!(!landing.flying);
        assert_eq!(landing.velocity.y, -0.3 * 0.6);

        let mut survival = Player::new(DVec3::new(0.5, 1.0, 0.5));
        survival.on_ground = true;
        survival.tick_survival(
            &world,
            Input {
                jump: true,
                ..Default::default()
            },
        );
        survival.tick_survival(&world, Input::default());
        survival.tick_survival(
            &world,
            Input {
                jump: true,
                ..Default::default()
            },
        );
        assert!(!survival.mayfly && !survival.flying);
    }
    #[test]
    fn survival_landing_charges_fall_damage_once() {
        let w = floor();
        let mut p = Player::new(DVec3::new(0.5, 6.0, 0.5));
        for _ in 0..60 {
            p.tick_survival(&w, Input::default());
            if p.on_ground {
                break;
            }
        }
        assert!(p.on_ground);
        assert_eq!(p.pos.y, 1.0);
        assert_eq!(p.survival.health, 18.0);
        assert_eq!(p.fall_distance, 0.0);
        for _ in 0..5 {
            p.tick_survival(&w, Input::default());
        }
        assert_eq!(p.survival.health, 18.0);
    }
    #[test]
    fn eleven_block_fall_matches_pinned_vanilla_health() {
        let w = floor();
        let mut p = Player::new(DVec3::new(0.5, 12.0, 0.5));
        for _ in 0..40 {
            p.tick_survival(&w, Input::default());
            if p.on_ground {
                break;
            }
        }
        assert!(p.on_ground);
        assert_eq!(p.pos.y, 1.0);
        assert_eq!(p.survival.health, 12.0);
        assert_eq!(p.fall_distance, 0.0);
    }
    #[test]
    fn settles_and_jumps() {
        let w = floor();
        let mut p = Player::new(DVec3::new(0.5, 3.0, 0.5));
        for _ in 0..30 {
            p.tick(&w, Input::default());
        }
        assert!(p.on_ground);
        assert!((p.pos.y - 1.0).abs() < 1e-9);
        p.tick(
            &w,
            Input {
                jump: true,
                ..Default::default()
            },
        );
        assert!(p.pos.y > 1.3);
    }
    #[test]
    fn full_block_stops_motion() {
        let mut w = floor();
        w.set_block((0, 1, -2), Some(Block::new("minecraft:stone")));
        let mut p = Player::new(DVec3::new(0.5, 1.0, 0.5));
        p.on_ground = true;
        for _ in 0..40 {
            p.tick(
                &w,
                Input {
                    forward: 1.0,
                    ..Default::default()
                },
            );
        }
        assert!(p.pos.z >= -0.7 - 1e-9);
    }
    #[test]
    fn left_impulse_moves_left_when_facing_north() {
        let w = floor();
        let mut left = Player::new(DVec3::new(0.5, 1.0, 0.5));
        let mut right = left.clone();
        left.on_ground = true;
        right.on_ground = true;
        left.tick(
            &w,
            Input {
                strafe: 1.0,
                ..Default::default()
            },
        );
        right.tick(
            &w,
            Input {
                strafe: -1.0,
                ..Default::default()
            },
        );
        assert!(left.pos.x < 0.5);
        assert!(right.pos.x > 0.5);
    }
    #[test]
    fn diagonal_input_reaches_unit_length_after_square_mapping() {
        let (_, forward) = modified_ground_input(
            Input {
                forward: 1.0,
                ..Default::default()
            },
            false,
        );
        let (left, diagonal_forward) = modified_ground_input(
            Input {
                forward: 1.0,
                strafe: 1.0,
                ..Default::default()
            },
            false,
        );
        assert_eq!(forward, 0.98_f32 as f64);
        assert!(((left * left + diagonal_forward * diagonal_forward).sqrt() - 1.0).abs() < 1e-6);
    }
    #[test]
    fn flat_ground_speed_ratios_match_source_modifiers() {
        let w = floor();
        let initial = Player::new(DVec3::new(0.5, 1.0, 0.5));
        let mut walk = initial.clone();
        let mut diagonal = initial.clone();
        let mut sprint = initial;
        walk.on_ground = true;
        diagonal.on_ground = true;
        sprint.on_ground = true;
        let mut walk_fifth = 0.0;
        let mut walk_last = 0.0;
        let mut sprint_last = 0.0;
        for tick in 0..12 {
            let wz = walk.pos.z;
            let sz = sprint.pos.z;
            walk.tick(
                &w,
                Input {
                    forward: 1.0,
                    ..Default::default()
                },
            );
            diagonal.tick(
                &w,
                Input {
                    forward: 1.0,
                    strafe: 1.0,
                    ..Default::default()
                },
            );
            sprint.tick(
                &w,
                Input {
                    forward: 1.0,
                    sprint: true,
                    ..Default::default()
                },
            );
            if tick == 4 {
                walk_fifth = (walk.pos.z - wz).abs();
            }
            walk_last = (walk.pos.z - wz).abs();
            sprint_last = (sprint.pos.z - sz).abs();
        }
        let walk_distance = (walk.pos - DVec3::new(0.5, 1.0, 0.5)).length();
        let diagonal_distance = (diagonal.pos - DVec3::new(0.5, 1.0, 0.5)).length();
        assert!(
            walk_fifth / walk_last > 0.92,
            "fifth={walk_fifth} final={walk_last}"
        );
        assert!(
            (diagonal_distance / walk_distance - 1.0 / 0.98).abs() < 1e-6,
            "walk={walk_distance} diagonal={diagonal_distance}"
        );
        assert!(
            (sprint_last / walk_last - 1.3).abs() < 1e-6,
            "walk={walk_last} sprint={sprint_last}"
        );
        let mut no_jump = sprint.clone();
        let before = sprint.pos.z;
        no_jump.tick(
            &w,
            Input {
                forward: 1.0,
                sprint: true,
                ..Default::default()
            },
        );
        sprint.tick(
            &w,
            Input {
                forward: 1.0,
                sprint: true,
                jump: true,
                ..Default::default()
            },
        );
        let jump_horizontal = (sprint.pos.z - before).abs();
        let no_jump_horizontal = (no_jump.pos.z - before).abs();
        assert!(
            (jump_horizontal - no_jump_horizontal - 0.2).abs() < 1e-9,
            "sprint={no_jump_horizontal} jump={jump_horizontal}"
        );
        assert!(
            (sprint.velocity.z.abs() / jump_horizontal - (0.6_f32 * 0.91_f32) as f64).abs() < 1e-9,
            "jump tick must retain ground friction before becoming airborne"
        );
    }
    #[test]
    fn slab_is_half_height() {
        let mut w = floor();
        w.set_block(
            (0, 1, -2),
            Some(Block::new("minecraft:oak_slab").with("type", "bottom")),
        );
        let mut p = Player::new(DVec3::new(0.5, 1.0, 0.5));
        p.on_ground = true;
        let mut max_y = p.pos.y;
        for _ in 0..25 {
            p.tick(
                &w,
                Input {
                    forward: 1.0,
                    ..Default::default()
                },
            );
            max_y = max_y.max(p.pos.y);
        }
        assert!(p.pos.z < -1.0);
        assert!(max_y >= 1.5 - 1e-9, "max_y={max_y} pos={:?}", p.pos);
    }
    #[test]
    fn clicking_bottom_slab_top_merges_in_live_player_placement() {
        let mut world = TestWorld::default();
        world.set_block(
            (0, 1, 0),
            Some(
                Block::new("minecraft:oak_slab")
                    .with("type", "bottom")
                    .with("waterlogged", "false"),
            ),
        );
        let mut player = Player::new(DVec3::new(0.5, 1.0, 2.5));
        player.pitch = (1.12_f64 / 2.0).atan().to_degrees();
        let hit = player.target(&world, 5.0).unwrap();
        assert_eq!(hit.pos, (0, 1, 0));
        assert_eq!(hit.face, Face::Up);
        assert_eq!(
            player.place_target_with(&mut world, Block::new("minecraft:oak_slab")),
            Some((0, 1, 0))
        );
        let placed = world.block((0, 1, 0)).unwrap();
        assert_eq!(placed.property("type"), Some("double"));
        assert_eq!(placed.property("waterlogged"), Some("false"));
    }
    #[test]
    fn crouching_holds_cardinal_and_diagonal_ledge_support() {
        let mut w = TestWorld::default();
        w.set_block((0, 0, 0), Some(Block::new("minecraft:stone")));
        let mut east = Player::new(DVec3::new(0.5, 1.0, 0.5));
        east.on_ground = true;
        let mut corner = east.clone();
        for _ in 0..80 {
            east.tick(
                &w,
                Input {
                    strafe: -1.0,
                    crouch: true,
                    ..Default::default()
                },
            );
            corner.tick(
                &w,
                Input {
                    forward: 1.0,
                    strafe: -1.0,
                    crouch: true,
                    ..Default::default()
                },
            );
        }
        assert!(
            east.on_ground && (east.pos.y - 1.0).abs() < 1e-9,
            "east={:?}",
            east.pos
        );
        assert!(east.pos.x > 1.0 && east.pos.x < 1.3, "east={:?}", east.pos);
        assert!(
            corner.on_ground && (corner.pos.y - 1.0).abs() < 1e-9,
            "corner={:?}",
            corner.pos
        );
        assert!(
            corner.pos.x < 1.3 && corner.pos.z > -0.3,
            "corner={:?}",
            corner.pos
        );
        let mut walking = Player::new(DVec3::new(0.5, 1.0, 0.5));
        walking.on_ground = true;
        for _ in 0..40 {
            walking.tick(
                &w,
                Input {
                    strafe: -1.0,
                    ..Default::default()
                },
            );
        }
        assert!(
            walking.pos.y < 1.0,
            "walking should fall from the same edge"
        );
        let mut slab_world = TestWorld::default();
        slab_world.set_block(
            (0, 0, 0),
            Some(Block::new("minecraft:oak_slab").with("type", "bottom")),
        );
        let mut slab = Player::new(DVec3::new(0.5, 0.5, 0.5));
        slab.on_ground = true;
        for _ in 0..80 {
            slab.tick(
                &slab_world,
                Input {
                    strafe: -1.0,
                    crouch: true,
                    ..Default::default()
                },
            );
        }
        assert!(
            slab.on_ground && (slab.pos.y - 0.5).abs() < 1e-9 && slab.pos.x < 1.3,
            "slab={:?}",
            slab.pos
        );
    }
    #[test]
    fn crouch_jump_can_leave_a_ledge() {
        let mut w = TestWorld::default();
        w.set_block((0, 0, 0), Some(Block::new("minecraft:stone")));
        let mut p = Player::new(DVec3::new(1.2, 1.0, 0.5));
        p.on_ground = true;
        p.tick(
            &w,
            Input {
                strafe: -1.0,
                crouch: true,
                jump: true,
                ..Default::default()
            },
        );
        for _ in 0..25 {
            p.tick(
                &w,
                Input {
                    strafe: -1.0,
                    crouch: true,
                    ..Default::default()
                },
            );
        }
        assert!(
            p.pos.x > 1.3,
            "jump should bypass edge backoff: {:?}",
            p.pos
        );
    }
    #[test]
    fn ray_break_place() {
        let mut w = floor();
        w.set_block((0, 2, -2), Some(Block::new("minecraft:stone")));
        let p = Player::new(DVec3::new(0.5, 1.0, 0.5));
        assert_eq!(p.target(&w, 5.0).unwrap().pos, (0, 2, -2));
        assert_eq!(p.break_target(&mut w), Some((0, 2, -2)));
        assert!(w.block((0, 2, -2)).is_none());
    }
    #[test]
    fn empty_bucket_ray_hits_only_source_liquid() {
        let mut world = floor();
        world.set_block(
            (0, 2, -2),
            Some(Block::new("minecraft:water").with("level", "0")),
        );
        world.set_block((0, 2, -3), Some(Block::new("minecraft:stone")));
        let player = Player::new(DVec3::new(0.5, 1.0, 0.5));
        assert_eq!(player.target(&world, 5.0).unwrap().pos, (0, 2, -3));
        assert_eq!(
            player.target_source_fluid(&world, 5.0).unwrap().pos,
            (0, 2, -2)
        );
        world.set_block(
            (0, 2, -2),
            Some(Block::new("minecraft:water").with("level", "1")),
        );
        assert_eq!(
            player.target_source_fluid(&world, 5.0).unwrap().pos,
            (0, 2, -3)
        );
        assert_eq!(
            player.target_any_fluid(&world, 5.0).unwrap().pos,
            (0, 2, -2),
            "F3 fluid picking includes a flowing level-one surface"
        );
        world.set_block(
            (0, 2, -2),
            Some(Block::new("minecraft:water").with("level", "4")),
        );
        assert_eq!(
            player.target_any_fluid(&world, 5.0).unwrap().pos,
            (0, 2, -3),
            "a lower fluid surface stays below the horizontal eye ray"
        );
    }
    #[test]
    fn placement_uses_supplied_inventory_block_not_demo_hotbar() {
        let mut w = floor();
        w.set_block((0, 2, -2), Some(Block::new("minecraft:stone")));
        let p = Player::new(DVec3::new(0.5, 1.0, 0.5));
        let pos = p
            .place_target_with(&mut w, Block::new("minecraft:oak_planks"))
            .unwrap();
        assert_eq!(w.block(pos).unwrap().id, "minecraft:oak_planks");
        assert_eq!(p.hotbar[0].id, "minecraft:stone");
    }
    #[test]
    fn adventure_placement_requires_matching_item_predicate() {
        let mut world = floor();
        world.set_block((0, 2, -2), Some(Block::new("minecraft:stone")));
        let player = Player::new(DVec3::new(0.5, 1.0, 0.5));
        let mut inventory = Inventory::default();
        inventory.slots[0] = Some(inventory::ItemStack::new("minecraft:oak_planks", 2));
        assert_eq!(
            player.place_selected(&mut world, &mut inventory, GameMode::Adventure),
            None
        );
        assert_eq!(inventory.slots[0].as_ref().unwrap().count, 2);
        inventory.slots[0].as_mut().unwrap().components =
            Some(serde_json::json!({"minecraft:can_place_on":{"blocks":"minecraft:stone"}}));
        assert!(player
            .place_selected(&mut world, &mut inventory, GameMode::Adventure)
            .is_some());
        assert_eq!(inventory.slots[0].as_ref().unwrap().count, 1);
    }
    #[test]
    fn spectator_flight_crosses_a_solid_wall_without_collision() {
        let mut world = floor();
        world.set_block((1, 1, 0), Some(Block::new("minecraft:stone")));
        world.set_block((1, 2, 0), Some(Block::new("minecraft:stone")));
        let mut spectator = Player::new(DVec3::new(0.5, 1.0, 0.5));
        spectator.set_game_mode(GameMode::Spectator);
        spectator.velocity.x = 0.4;
        spectator.tick(&world, Input::default());
        assert!(spectator.pos.x > 0.8);
        assert!(spectator.flying);
        assert!(!spectator.on_ground);

        let mut creative = Player::new(DVec3::new(0.5, 1.0, 0.5));
        creative.velocity.x = 0.4;
        creative.tick(&world, Input::default());
        assert!(creative.pos.x <= 0.700001);
    }
    #[test]
    fn flying_speed_scales_flight_like_abilities() {
        // Player.getFlyingSpeed and LocalPlayer.aiStep: horizontal
        // acceleration and vertical input both scale with flyingSpeed.
        let world = floor();
        let fly = |speed: f32| {
            let mut player = Player::new(DVec3::new(0.5, 40.0, 0.5));
            player.set_game_mode(GameMode::Spectator);
            player.flying_speed = speed;
            let input = Input { forward: 1.0, jump: true, ..Input::default() };
            player.tick(&world, input);
            player.velocity
        };
        let (slow, fast) = (fly(0.05), fly(0.1));
        assert!((fast.z / slow.z - 2.0).abs() < 1e-6, "{slow:?} {fast:?}");
        assert!((fast.y / slow.y - 2.0).abs() < 1e-6, "{slow:?} {fast:?}");
    }
    #[test]
    fn survival_placement_consumes_only_on_success() {
        let mut world = floor();
        world.set_block((0, 2, -2), Some(Block::new("minecraft:stone")));
        let player = Player::new(DVec3::new(0.5, 1.0, 0.5));
        let mut inventory = Inventory::default();
        inventory.slots[0] = Some(inventory::ItemStack::new("minecraft:oak_planks", 2));
        let placed = player
            .place_selected(&mut world, &mut inventory, GameMode::Survival)
            .unwrap();
        assert_eq!(world.block(placed).unwrap().id, "minecraft:oak_planks");
        assert_eq!(inventory.slots[0].as_ref().unwrap().count, 1);
        assert!(player
            .place_selected(&mut world, &mut inventory, GameMode::Survival)
            .is_none());
        assert_eq!(inventory.slots[0].as_ref().unwrap().count, 1);
    }
    #[test]
    fn fast_furnaces_place_with_facing_and_unlit_state() {
        for id in ["minecraft:blast_furnace", "minecraft:smoker"] {
            let mut world = floor();
            world.set_block((0, 2, -2), Some(Block::new("minecraft:stone")));
            let player = Player::new(DVec3::new(0.5, 1.0, 0.5));
            let mut inventory = Inventory::default();
            inventory.slots[0] = Some(inventory::ItemStack::new(id, 1));
            let pos = player
                .place_selected(&mut world, &mut inventory, GameMode::Survival)
                .unwrap();
            let placed = world.block(pos).unwrap();
            assert_eq!(placed.id, id);
            assert_eq!(placed.property("facing"), Some("south"));
            assert_eq!(placed.property("lit"), Some("false"));
            assert!(inventory.slots[0].is_none());
        }
    }
    #[test]
    fn replay_is_bitwise_repeatable() {
        let w = floor();
        let inputs = (0..150)
            .map(|i| Input {
                forward: 1.0,
                strafe: if i % 40 < 20 { 1.0 } else { 0.0 },
                jump: i % 30 == 0,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let mut a = Player::new(DVec3::new(0.5, 1.0, 0.5));
        let mut b = a.clone();
        for i in &inputs {
            a.tick(&w, *i);
        }
        for chunk in inputs.chunks(7) {
            for i in chunk {
                b.tick(&w, *i);
            }
        }
        assert_eq!(
            a.pos.to_array().map(f64::to_bits),
            b.pos.to_array().map(f64::to_bits)
        );
        assert_eq!(
            a.velocity.to_array().map(f64::to_bits),
            b.velocity.to_array().map(f64::to_bits)
        );
    }
}
