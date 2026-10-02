//! `BlockState.updateShape` for blocks found in generated terrain, and
//! `StructureTemplate.updateShapeAtEdge`, which trees and structures run
//! over the boundary of what they placed. World generation has no neighbour
//! updates otherwise; scheduled ticks are recorded, not run.

use super::blocks::FluidType;
use super::tree::VoxelShape;
use super::{Ctx, World};
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::tags::TagId;
use minecraftoss_core::block::{flags, FaceShape, FIRST_HEAD_INSTRUMENT, INSTRUMENTS};
use minecraftoss_core::{BlockId, BlockPos, BlockStateId, Registries, SupportType};

#[derive(Clone, Copy, Debug)]
enum Rule {
    /// `VegetationBlock` and blocks with the same rule: air unless it survives.
    SurviveOrAir,
    /// `SeagrassBlock`: survive or air, plus a water tick.
    Seagrass,
    DoublePlant,
    Snowy,
    Cocoa,
    SeaPickle,
    CoralPlant,
    CoralWallFan,
    /// `CoralBlock` (full, living): a die tick when no water is beside it.
    CoralBlock,
    HangingRoots,
    BigDripleaf,
    BigDripleafStem,
    Waterlogged,
    ScheduleIfDead,
    GrowingHead { up: bool, body: &'static str },
    GrowingBody { up: bool, head: &'static str, fluid: bool },
    Bamboo,
    BambooSapling,
    ConcretePowder,
    HangingMoss,
    MangrovePropagule,
    ShelfMushroom,
    SporeBlossom,
    AmethystCluster,
    HugeMushroom,
    Leaves,
    Liquid,
    Fire,
    SoulFire,
    ChorusPlant,
    ChorusFlower,
    Vine,
    Multiface,
    MossyCarpet,
    CreakingHeart,
    Speleothem,
    // Class-keyed rules, mostly for blocks structure templates place.
    Stairs,
    Fence,
    Bars,
    Wall,
    FenceGate,
    Chest { copper: bool },
    Door,
    Bed,
    Tripwire,
    RedstoneWire,
    /// `RepeaterBlock`: support below, and `locked` from side diodes.
    Repeater,
    /// `ComparatorBlock`: support below.
    Comparator,
    /// `ObserverBlock`: a change in front starts a pulse.
    Observer,
    NoteBlock,
    Campfire,
    AttachedStem { fruit: &'static str, stem: &'static str },
    Lantern,
    /// Air when the block below no longer supports it (torches, pressure
    /// plates, banners, standing signs), then a water tick if waterlogged.
    DownSurvive,
    /// Air when the block behind (against `facing`) no longer supports it:
    /// wall torches, ladders, wall banners and signs, tripwire hooks, piston heads.
    BehindSurvive,
    FaceAttached,
    Bell,
    /// `FarmlandBlock`, `PathBlock`: a tick to revert when covered.
    TickIfCovered,
    /// `FallingBlock`, `BrushableBlock`: always a tick.
    Tick,
}

pub struct UpdateRules {
    rules: super::BlockMap<Rule>,
    pub soul_fire_base: TagId,
    snow: TagId,
    supports_chorus_plant: TagId,
    fences: TagId,
    wooden_fences: TagId,
    walls: TagId,
    wall_post_override: TagId,
    shulker_boxes: TagId,
    copper_chests: TagId,
    maintains_farmland: Option<TagId>,
}

impl UpdateRules {
    pub fn load(registries: &Registries) -> Result<Self, String> {
        let mut rules = super::BlockMap::default();
        let mut add = |names: &[&str], rule: Rule| {
            for name in names {
                if let Some(id) = registries.blocks.block_by_name(&format!("minecraft:{name}")) {
                    rules.insert(id, rule);
                }
            }
        };
        add(
            &[
                "oak_sapling", "spruce_sapling", "birch_sapling", "jungle_sapling", "acacia_sapling", "cherry_sapling", "dark_oak_sapling",
                "pale_oak_sapling", "poplar_sapling", "dandelion", "poppy", "blue_orchid", "allium", "azure_bluet", "red_tulip", "orange_tulip",
                "white_tulip", "pink_tulip", "oxeye_daisy", "cornflower", "lily_of_the_valley", "wither_rose", "torchflower", "open_eyeblossom",
                "closed_eyeblossom", "golden_dandelion", "red_shrub", "short_grass", "fern", "bush", "firefly_bush", "sweet_berry_bush", "pink_petals", "wildflowers", "leaf_litter",
                "dead_bush", "short_dry_grass", "tall_dry_grass", "azalea", "flowering_azalea", "brown_mushroom", "red_mushroom", "crimson_fungus",
                "warped_fungus", "crimson_roots", "warped_roots", "nether_sprouts", "lily_pad", "snow", "moss_carpet", "cactus_flower",
            ],
            Rule::SurviveOrAir,
        );
        add(&["seagrass"], Rule::Seagrass);
        add(&["tall_grass", "large_fern", "sunflower", "lilac", "rose_bush", "peony", "pitcher_plant", "tall_seagrass", "small_dripleaf"], Rule::DoublePlant);
        add(&["grass_block", "podzol", "mycelium"], Rule::Snowy);
        add(&["cocoa"], Rule::Cocoa);
        add(&["sea_pickle"], Rule::SeaPickle);
        add(
            &[
                "tube_coral", "brain_coral", "bubble_coral", "fire_coral", "horn_coral", "dead_tube_coral", "dead_brain_coral", "dead_bubble_coral",
                "dead_fire_coral", "dead_horn_coral", "tube_coral_fan", "brain_coral_fan", "bubble_coral_fan", "fire_coral_fan", "horn_coral_fan",
                "dead_tube_coral_fan", "dead_brain_coral_fan", "dead_bubble_coral_fan", "dead_fire_coral_fan", "dead_horn_coral_fan",
            ],
            Rule::CoralPlant,
        );
        add(
            &[
                "tube_coral_wall_fan", "brain_coral_wall_fan", "bubble_coral_wall_fan", "fire_coral_wall_fan", "horn_coral_wall_fan",
                "dead_tube_coral_wall_fan", "dead_brain_coral_wall_fan", "dead_bubble_coral_wall_fan", "dead_fire_coral_wall_fan", "dead_horn_coral_wall_fan",
            ],
            Rule::CoralWallFan,
        );
        add(&["hanging_roots"], Rule::HangingRoots);
        add(&["big_dripleaf"], Rule::BigDripleaf);
        add(&["big_dripleaf_stem"], Rule::BigDripleafStem);
        add(&["mangrove_roots"], Rule::Waterlogged);
        add(&["sugar_cane", "cactus"], Rule::ScheduleIfDead);
        add(&["kelp"], Rule::GrowingHead { up: true, body: "minecraft:kelp_plant" });
        add(&["twisting_vines"], Rule::GrowingHead { up: true, body: "minecraft:twisting_vines_plant" });
        add(&["weeping_vines"], Rule::GrowingHead { up: false, body: "minecraft:weeping_vines_plant" });
        add(&["cave_vines"], Rule::GrowingHead { up: false, body: "minecraft:cave_vines_plant" });
        add(&["kelp_plant"], Rule::GrowingBody { up: true, head: "minecraft:kelp", fluid: true });
        add(&["twisting_vines_plant"], Rule::GrowingBody { up: true, head: "minecraft:twisting_vines", fluid: false });
        add(&["weeping_vines_plant"], Rule::GrowingBody { up: false, head: "minecraft:weeping_vines", fluid: false });
        add(&["cave_vines_plant"], Rule::GrowingBody { up: false, head: "minecraft:cave_vines", fluid: false });
        add(&["bamboo"], Rule::Bamboo);
        add(&["bamboo_sapling"], Rule::BambooSapling);
        add(&["tube_coral_block", "brain_coral_block", "bubble_coral_block", "fire_coral_block", "horn_coral_block"], Rule::CoralBlock);
        for color in [
            "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black",
        ] {
            add(&[&format!("{color}_concrete_powder")], Rule::ConcretePowder);
        }
        add(&["pale_hanging_moss"], Rule::HangingMoss);
        add(&["mangrove_propagule"], Rule::MangrovePropagule);
        add(&["shelf_mushroom"], Rule::ShelfMushroom);
        add(&["spore_blossom"], Rule::SporeBlossom);
        add(&["amethyst_cluster", "large_amethyst_bud", "medium_amethyst_bud", "small_amethyst_bud"], Rule::AmethystCluster);
        add(&["brown_mushroom_block", "red_mushroom_block", "mushroom_stem"], Rule::HugeMushroom);
        add(
            &[
                "oak_leaves", "spruce_leaves", "birch_leaves", "jungle_leaves", "acacia_leaves", "cherry_leaves", "dark_oak_leaves", "pale_oak_leaves",
                "mangrove_leaves", "azalea_leaves", "flowering_azalea_leaves",
            ],
            Rule::Leaves,
        );
        add(&["water", "lava"], Rule::Liquid);
        add(&["fire"], Rule::Fire);
        add(&["soul_fire"], Rule::SoulFire);
        add(&["chorus_plant"], Rule::ChorusPlant);
        add(&["chorus_flower"], Rule::ChorusFlower);
        add(&["vine"], Rule::Vine);
        add(&["glow_lichen", "sculk_vein", "resin_clump"], Rule::Multiface);
        add(&["pale_moss_carpet"], Rule::MossyCarpet);
        add(&["creaking_heart"], Rule::CreakingHeart);
        add(&["pointed_dripstone", "sulfur_spike"], Rule::Speleothem);
        add(&["attached_pumpkin_stem"], Rule::AttachedStem { fruit: "minecraft:pumpkin", stem: "minecraft:pumpkin_stem" });
        add(&["attached_melon_stem"], Rule::AttachedStem { fruit: "minecraft:melon", stem: "minecraft:melon_stem" });
        // The rest by vanilla class (schema 4 catalog), most derived first.
        let by_class: &[(&str, Rule)] = &[
            ("StairBlock", Rule::Stairs),
            ("FenceBlock", Rule::Fence),
            ("IronBarsBlock", Rule::Bars),
            ("WallBlock", Rule::Wall),
            ("FenceGateBlock", Rule::FenceGate),
            ("CopperChestBlock", Rule::Chest { copper: true }),
            ("ChestBlock", Rule::Chest { copper: false }),
            ("DoorBlock", Rule::Door),
            ("AbstractBedBlock", Rule::Bed),
            ("TripWireBlock", Rule::Tripwire),
            ("TripWireHookBlock", Rule::BehindSurvive),
            ("RedstoneWireBlock", Rule::RedstoneWire),
            ("RepeaterBlock", Rule::Repeater),
            ("ComparatorBlock", Rule::Comparator),
            ("ObserverBlock", Rule::Observer),
            ("NoteBlock", Rule::NoteBlock),
            ("CampfireBlock", Rule::Campfire),
            ("LanternBlock", Rule::Lantern),
            ("WallTorchBlock", Rule::BehindSurvive),
            ("RedstoneWallTorchBlock", Rule::BehindSurvive),
            ("LadderBlock", Rule::BehindSurvive),
            ("WallBannerBlock", Rule::BehindSurvive),
            ("WallSignBlock", Rule::BehindSurvive),
            ("PistonHeadBlock", Rule::BehindSurvive),
            ("BaseTorchBlock", Rule::DownSurvive),
            ("BasePressurePlateBlock", Rule::DownSurvive),
            ("BannerBlock", Rule::DownSurvive),
            ("StandingSignBlock", Rule::DownSurvive),
            ("FaceAttachedHorizontalDirectionalBlock", Rule::FaceAttached),
            ("BellBlock", Rule::Bell),
            ("LeavesBlock", Rule::Leaves),
            ("CarpetBlock", Rule::SurviveOrAir),
            ("VegetationBlock", Rule::SurviveOrAir),
            ("FarmlandBlock", Rule::TickIfCovered),
            ("PathBlock", Rule::TickIfCovered),
            ("FallingBlock", Rule::Tick),
            ("BrushableBlock", Rule::Tick),
        ];
        for (id, info) in registries.blocks.blocks() {
            if rules.contains_key(&id) {
                continue;
            }
            let found = info.classes().iter().find_map(|class| by_class.iter().find(|(name, _)| **name == **class));
            if let Some((_, r)) = found {
                rules.insert(id, *r);
            } else if info.properties().iter().any(|p| &*p.name == "waterlogged") {
                // `SimpleWaterloggedBlock` implementations schedule a water tick.
                rules.insert(id, Rule::Waterlogged);
            }
        }
        let tag = |name: &str| registries.block_tags.require(name);
        Ok(Self {
            rules,
            soul_fire_base: tag("minecraft:soul_fire_base_blocks")?,
            snow: tag("minecraft:snow")?,
            supports_chorus_plant: tag("minecraft:supports_chorus_plant")?,
            fences: tag("minecraft:fences")?,
            wooden_fences: tag("minecraft:wooden_fences")?,
            walls: tag("minecraft:walls")?,
            wall_post_override: tag("minecraft:wall_post_override")?,
            shulker_boxes: tag("minecraft:shulker_boxes")?,
            copper_chests: tag("minecraft:copper_chests")?,
            maintains_farmland: registries.block_tags.id("minecraft:maintains_farmland"),
        })
    }
}

fn waterlogged<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId) -> bool {
    ctx.property(state, "waterlogged") == Some("true")
}

/// `BlockState.updateShape(level, ticks, pos, direction, neighbourPos, neighbourState, random)`.
pub fn update_shape<W: World + ?Sized>(ctx: &mut Ctx<W>, state: BlockStateId, pos: BlockPos, direction: Direction, neighbor: BlockStateId) -> BlockStateId {
    let lib = ctx.lib;
    let block = lib.registries.blocks.block_of(state);
    let Some(&rule) = lib.update_rules.rules.get(&block) else {
        return state;
    };
    let air = lib.blocks.air;
    let survives = |ctx: &Ctx<W>| ctx.can_survive(state, pos);
    let same_block = |ctx: &Ctx<W>, s: BlockStateId| ctx.registries().blocks.block_of(s) == block;
    let facing = |ctx: &Ctx<W>| ctx.property(state, "facing").and_then(Direction::from_name).unwrap_or(Direction::North);
    match rule {
        Rule::SurviveOrAir => {
            if survives(ctx) {
                state
            } else {
                air
            }
        }
        Rule::Seagrass => {
            if !survives(ctx) {
                return air;
            }
            ctx.schedule_fluid_tick_for(pos, state);
            state
        }
        Rule::DoublePlant => {
            let lower = ctx.property(state, "half") == Some("lower");
            let neighbor_other_half = same_block(ctx, neighbor) && ctx.property(neighbor, "half") != ctx.property(state, "half");
            if direction.axis() != minecraftoss_core::pos::Axis::Y || lower != (direction == Direction::Up) || neighbor_other_half {
                if lower && direction == Direction::Down && !survives(ctx) {
                    return air;
                }
                if waterlogged(ctx, state) {
                    ctx.schedule_fluid_tick_for(pos, state);
                }
                state
            } else {
                air
            }
        }
        Rule::Snowy => {
            if direction == Direction::Up {
                let snowy = ctx.in_tag(neighbor, lib.update_rules.snow);
                ctx.with(state, "snowy", if snowy { "true" } else { "false" })
            } else {
                state
            }
        }
        Rule::Cocoa => {
            if direction == facing(ctx) && !survives(ctx) {
                air
            } else {
                state
            }
        }
        Rule::SeaPickle => {
            if !survives(ctx) {
                return air;
            }
            if waterlogged(ctx, state) {
                ctx.schedule_fluid_tick_for(pos, state);
            }
            state
        }
        Rule::CoralPlant => {
            if direction == Direction::Down && !survives(ctx) {
                return air;
            }
            // `CoralPlantBlock` / `CoralFanBlock`: the die tick first.
            coral_try_schedule_die_tick(ctx, state, pos, block);
            if waterlogged(ctx, state) {
                ctx.schedule_fluid_tick_for(pos, state);
            }
            state
        }
        Rule::CoralWallFan => {
            if direction.opposite() == facing(ctx) && !survives(ctx) {
                return air;
            }
            if waterlogged(ctx, state) {
                ctx.schedule_fluid_tick_for(pos, state);
            }
            // `CoralWallFanBlock`: the die tick after the water's.
            coral_try_schedule_die_tick(ctx, state, pos, block);
            state
        }
        Rule::CoralBlock => {
            if !coral_scan_for_water(ctx, state, pos) {
                let delay = 60 + ctx.region.random().next_i32_bound(40);
                ctx.region.schedule_block(pos, block, delay);
            }
            state
        }
        Rule::HangingRoots => {
            if direction == Direction::Up && !survives(ctx) {
                return air;
            }
            if waterlogged(ctx, state) {
                ctx.schedule_fluid_tick_for(pos, state);
            }
            state
        }
        Rule::BigDripleaf => {
            if direction == Direction::Down && !survives(ctx) {
                return air;
            }
            if waterlogged(ctx, state) {
                ctx.schedule_fluid_tick_for(pos, state);
            }
            if direction == Direction::Up && same_block(ctx, neighbor) {
                let stem = ctx.registries().blocks.parse_state("minecraft:big_dripleaf_stem").expect("stem exists");
                return super::state::copy_properties(ctx.registries(), stem, state);
            }
            state
        }
        Rule::BigDripleafStem => {
            if matches!(direction, Direction::Down | Direction::Up) && !survives(ctx) {
                ctx.region.schedule_block(pos, block, 1);
            }
            if waterlogged(ctx, state) {
                ctx.schedule_fluid_tick_for(pos, state);
            }
            state
        }
        Rule::Waterlogged => {
            if waterlogged(ctx, state) {
                ctx.schedule_fluid_tick_for(pos, state);
            }
            state
        }
        Rule::ScheduleIfDead => {
            if !survives(ctx) {
                ctx.region.schedule_block(pos, block, 1);
            }
            state
        }
        Rule::GrowingHead { up, body } => {
            let growth = if up { Direction::Up } else { Direction::Down };
            if direction == growth.opposite() {
                if !survives(ctx) {
                    ctx.region.schedule_block(pos, block, 1);
                } else {
                    let next = ctx.block(pos.relative(growth, 1));
                    if same_block(ctx, next) || ctx.is(next, body) {
                        return head_to_body(ctx, state, body);
                    }
                }
            }
            if direction != growth || !same_block(ctx, neighbor) && !ctx.is(neighbor, body) {
                if block_is_kelp(ctx, block) {
                    ctx.schedule_fluid_tick_for(pos, state);
                }
                state
            } else {
                head_to_body(ctx, state, body)
            }
        }
        Rule::GrowingBody { up, head, fluid } => {
            let growth = if up { Direction::Up } else { Direction::Down };
            if direction == growth.opposite() && !survives(ctx) {
                ctx.region.schedule_block(pos, block, 1);
            }
            if direction == growth && !same_block(ctx, neighbor) && !ctx.is(neighbor, head) {
                // GrowingPlantHeadBlock.getStateForPlacement: a random age from the region random.
                let head_state = ctx.registries().blocks.parse_state(head).expect("head exists");
                let age = ctx.region.random().next_i32_bound(25);
                let head_state = ctx.with(head_state, "age", &age.to_string());
                // updateHeadAfterConvertedFromBody keeps the body's berries.
                return match ctx.property(state, "berries") {
                    Some(b) => ctx.with(head_state, "berries", b),
                    None => head_state,
                };
            }
            if fluid {
                ctx.schedule_fluid_tick_for(pos, state);
            }
            state
        }
        Rule::Bamboo => {
            if !survives(ctx) {
                ctx.region.schedule_block(pos, block, 1);
            }
            let age = |ctx: &Ctx<W>, s: BlockStateId| ctx.property(s, "age").and_then(|a| a.parse::<i32>().ok()).unwrap_or(0);
            if direction == Direction::Up && same_block(ctx, neighbor) && age(ctx, neighbor) > age(ctx, state) {
                let next = (age(ctx, state) + 1) % 2;
                ctx.with(state, "age", &next.to_string())
            } else {
                state
            }
        }
        // `ConcretePowderBlock.updateShape`: water beside it sets it,
        // otherwise it falls like any `FallingBlock`.
        Rule::ConcretePowder => {
            if concrete_touches_liquid(ctx, pos) {
                let name = ctx.name(state).trim_end_matches("_powder").to_owned();
                ctx.registries().blocks.parse_state(&name).unwrap_or(state)
            } else {
                ctx.region.schedule_block(pos, block, 2);
                state
            }
        }
        // `BambooSaplingBlock.updateShape`: bamboo above turns it into bamboo.
        Rule::BambooSapling => {
            if !survives(ctx) {
                air
            } else if direction == Direction::Up && ctx.name(neighbor) == "minecraft:bamboo" {
                ctx.registries().blocks.block(ctx.registries().blocks.block_of(neighbor)).default_state()
            } else {
                state
            }
        }
        Rule::HangingMoss => {
            if !survives(ctx) {
                ctx.region.schedule_block(pos, block, 1);
            }
            let below = ctx.block(pos.below());
            ctx.with(state, "tip", if same_block(ctx, below) { "false" } else { "true" })
        }
        Rule::MangrovePropagule => {
            if waterlogged(ctx, state) {
                ctx.schedule_fluid_tick_for(pos, state);
            }
            if direction == Direction::Up && !survives(ctx) {
                air
            } else if !survives(ctx) {
                // VegetationBlock.updateShape through super.
                air
            } else {
                state
            }
        }
        Rule::ShelfMushroom => {
            if direction == facing(ctx).opposite() && !survives(ctx) {
                air
            } else {
                state
            }
        }
        Rule::SporeBlossom => {
            if direction == Direction::Up && !survives(ctx) {
                air
            } else {
                state
            }
        }
        Rule::AmethystCluster => {
            if waterlogged(ctx, state) {
                ctx.schedule_fluid_tick_for(pos, state);
            }
            if direction == facing(ctx).opposite() && !survives(ctx) {
                air
            } else {
                state
            }
        }
        Rule::HugeMushroom => {
            if same_block(ctx, neighbor) {
                ctx.with(state, direction.name(), "false")
            } else {
                state
            }
        }
        Rule::Leaves => {
            if waterlogged(ctx, state) {
                ctx.schedule_fluid_tick_for(pos, state);
            }
            let neighbor_distance = if ctx.in_tag(neighbor, lib.tags.logs) {
                0
            } else {
                ctx.property(neighbor, "distance").and_then(|d| d.parse().ok()).unwrap_or(7)
            };
            let distance = neighbor_distance + 1;
            let own: i32 = ctx.property(state, "distance").and_then(|d| d.parse().ok()).unwrap_or(7);
            if distance != 1 || own != distance {
                ctx.region.schedule_block(pos, block, 1);
            }
            state
        }
        Rule::Liquid => {
            let source = |ctx: &Ctx<W>, s: BlockStateId| matches!(ctx.fluid(s), FluidType::Water | FluidType::Lava);
            if source(ctx, state) || source(ctx, neighbor) {
                ctx.schedule_fluid_tick_for(pos, state);
            }
            state
        }
        Rule::Fire => {
            if survives(ctx) {
                let age = ctx.property(state, "age").and_then(|a| a.parse().ok()).unwrap_or(0);
                fire_state_with_age(ctx, pos, age)
            } else {
                air
            }
        }
        Rule::SoulFire => {
            if survives(ctx) {
                ctx.registries().blocks.block(block).default_state()
            } else {
                air
            }
        }
        Rule::ChorusPlant => {
            if !survives(ctx) {
                ctx.region.schedule_block(pos, block, 1);
                return state;
            }
            let connect = same_block(ctx, neighbor)
                || ctx.is(neighbor, "minecraft:chorus_flower")
                || direction == Direction::Down && ctx.in_tag(neighbor, lib.update_rules.supports_chorus_plant);
            ctx.with(state, direction.name(), if connect { "true" } else { "false" })
        }
        Rule::ChorusFlower => {
            if direction != Direction::Up && !survives(ctx) {
                ctx.region.schedule_block(pos, block, 1);
            }
            state
        }
        Rule::Vine => {
            if direction == Direction::Down {
                return state;
            }
            vine_updated(ctx, state, pos)
        }
        Rule::Multiface => {
            if waterlogged(ctx, state) {
                ctx.schedule_fluid_tick_for(pos, state);
            }
            let has = ctx.property(state, direction.name()) == Some("true");
            if has && !super::kinds::simple::can_attach_to(ctx, direction, neighbor) {
                let removed = ctx.with(state, direction.name(), "false");
                let any = Direction::ALL.iter().any(|d| ctx.property(removed, d.name()) == Some("true"));
                if any {
                    removed
                } else if waterlogged(ctx, state) {
                    ctx.lib.blocks.water
                } else {
                    air
                }
            } else {
                state
            }
        }
        Rule::MossyCarpet => {
            if !survives(ctx) {
                return air;
            }
            let updated = super::kinds::simple::mossy_updated(ctx, state, pos, false);
            if super::kinds::simple::mossy_has_faces(ctx, updated) {
                updated
            } else {
                air
            }
        }
        Rule::CreakingHeart => {
            ctx.region.schedule_block(pos, block, 1);
            state
        }
        Rule::Speleothem => {
            if waterlogged(ctx, state) {
                ctx.schedule_fluid_tick_for(pos, state);
            }
            if !matches!(direction, Direction::Up | Direction::Down) {
                return state;
            }
            // Thickness recalculation is left to the feature that placed it.
            state
        }
        Rule::Stairs => {
            tick_if_waterlogged(ctx, state, pos);
            if direction.is_horizontal() {
                let shape = stairs_shape(ctx, state, pos);
                ctx.with(state, "shape", shape)
            } else {
                state
            }
        }
        Rule::Fence | Rule::Bars => {
            tick_if_waterlogged(ctx, state, pos);
            if !direction.is_horizontal() {
                return state;
            }
            let from = direction.opposite();
            let face_solid = ctx.registries().blocks.is_face_sturdy(neighbor, from, SupportType::Full);
            let rules = &lib.update_rules;
            let connects = if matches!(rule, Rule::Fence) {
                let same_fence = ctx.in_tag(neighbor, rules.fences) && ctx.in_tag(neighbor, rules.wooden_fences) == ctx.in_tag(state, rules.wooden_fences);
                let gate = is_a(ctx, neighbor, "FenceGateBlock") && gate_connects(ctx, neighbor, from);
                !is_exception_for_connection(ctx, neighbor) && face_solid || same_fence || gate
            } else {
                !is_exception_for_connection(ctx, neighbor) && face_solid || is_a(ctx, neighbor, "IronBarsBlock") || ctx.in_tag(neighbor, rules.walls)
            };
            ctx.with(state, direction.name(), bool_name(connects))
        }
        Rule::Wall => {
            tick_if_waterlogged(ctx, state, pos);
            match direction {
                Direction::Down => state,
                Direction::Up => {
                    let sides = Direction::HORIZONTAL.map(|d| ctx.property(state, d.name()) != Some("none"));
                    wall_state(ctx, state, neighbor, sides)
                }
                _ => {
                    let from = direction.opposite();
                    let connects = wall_connects_to(ctx, neighbor, ctx.registries().blocks.is_face_sturdy(neighbor, from, SupportType::Full), from);
                    let sides = Direction::HORIZONTAL.map(|d| if d == direction { connects } else { ctx.property(state, d.name()) != Some("none") });
                    let above = ctx.block(pos.above());
                    wall_state(ctx, state, above, sides)
                }
            }
        }
        Rule::FenceGate => {
            if facing(ctx).clockwise().axis() != direction.axis() {
                return state;
            }
            let walls = lib.update_rules.walls;
            let in_wall = ctx.in_tag(neighbor, walls) || ctx.in_tag(ctx.block(pos.relative(direction.opposite(), 1)), walls);
            ctx.with(state, "in_wall", bool_name(in_wall))
        }
        Rule::Chest { copper } => {
            tick_if_waterlogged(ctx, state, pos);
            let can_connect = |ctx: &Ctx<W>, s: BlockStateId| {
                if copper {
                    ctx.in_tag(s, lib.update_rules.copper_chests) && ctx.property(s, "type").is_some()
                } else {
                    same_block(ctx, s)
                }
            };
            let mut result = state;
            if can_connect(ctx, neighbor) && direction.is_horizontal() {
                let neighbor_type = ctx.property(neighbor, "type").unwrap_or("single");
                if ctx.property(state, "type") == Some("single")
                    && neighbor_type != "single"
                    && ctx.property(state, "facing") == ctx.property(neighbor, "facing")
                    && chest_connected_direction(ctx, neighbor) == direction.opposite()
                {
                    let opposite = if neighbor_type == "left" { "right" } else { "left" };
                    result = ctx.with(state, "type", opposite);
                }
            } else if chest_connected_direction(ctx, state) == direction {
                result = ctx.with(state, "type", "single");
            }
            // `CopperChestBlock.updateShape`: a connected half takes on its
            // partner's block (oxidation), keeping its own properties.
            if copper && can_connect(ctx, neighbor) && ctx.property(result, "type") != Some("single") && chest_connected_direction(ctx, result) == direction {
                let other = ctx.registries().blocks.block_of(neighbor);
                let base = ctx.registries().blocks.block(other).default_state();
                return super::state::copy_properties(ctx.registries(), base, result);
            }
            result
        }
        Rule::Door => {
            let lower = ctx.property(state, "half") == Some("lower");
            if direction.axis() != minecraftoss_core::pos::Axis::Y || lower != (direction == Direction::Up) {
                if lower && direction == Direction::Down && !survives(ctx) {
                    air
                } else {
                    state
                }
            } else if is_a(ctx, neighbor, "DoorBlock") && ctx.property(neighbor, "half") != ctx.property(state, "half") {
                ctx.with(neighbor, "half", if lower { "lower" } else { "upper" })
            } else {
                air
            }
        }
        Rule::Bed => {
            let foot = ctx.property(state, "part") == Some("foot");
            let toward = if foot { facing(ctx) } else { facing(ctx).opposite() };
            if direction != toward {
                return state;
            }
            if same_block(ctx, neighbor) && ctx.property(neighbor, "part") != ctx.property(state, "part") {
                let occupied = ctx.property(neighbor, "occupied").unwrap_or("false");
                ctx.with(state, "occupied", occupied)
            } else {
                air
            }
        }
        Rule::Tripwire => {
            if !direction.is_horizontal() {
                return state;
            }
            let connects = if ctx.is(neighbor, "minecraft:tripwire_hook") {
                ctx.property(neighbor, "facing") == Some(direction.opposite().name())
            } else {
                same_block(ctx, neighbor)
            };
            ctx.with(state, direction.name(), bool_name(connects))
        }
        Rule::RedstoneWire => match direction {
            Direction::Down => {
                if wire_can_survive_on(ctx, neighbor) {
                    state
                } else {
                    air
                }
            }
            Direction::Up => wire_connection_state(ctx, state, pos),
            _ => {
                let side = wire_connecting_side(ctx, pos, direction, wire_can_connect_up(ctx, pos));
                let current = ctx.property(state, direction.name()).unwrap_or("none");
                if (side != "none") == (current != "none") && !wire_is_cross(ctx, state) {
                    ctx.with(state, direction.name(), side)
                } else {
                    let cross = wire_cross(ctx, state);
                    let cross = ctx.with(cross, direction.name(), side);
                    wire_connection_state(ctx, cross, pos)
                }
            }
        },
        Rule::NoteBlock => {
            if direction.axis() != minecraftoss_core::pos::Axis::Y {
                return state;
            }
            let blocks = &ctx.registries().blocks;
            let above = blocks.state(ctx.block(pos.above())).instrument;
            let instrument = if above >= FIRST_HEAD_INSTRUMENT {
                above
            } else {
                let below = blocks.state(ctx.block(pos.below())).instrument;
                if below >= FIRST_HEAD_INSTRUMENT { 0 } else { below }
            };
            ctx.with(state, "instrument", INSTRUMENTS[usize::from(instrument)])
        }
        Rule::Campfire => {
            tick_if_waterlogged(ctx, state, pos);
            if direction == Direction::Down {
                ctx.with(state, "signal_fire", bool_name(ctx.is(neighbor, "minecraft:hay_block")))
            } else {
                state
            }
        }
        Rule::AttachedStem { fruit, stem } => {
            if !ctx.is(neighbor, fruit) && direction == facing(ctx) {
                let stem = ctx.registries().blocks.parse_state(stem).expect("stem exists");
                return ctx.with(stem, "age", "7");
            }
            if survives(ctx) {
                state
            } else {
                air
            }
        }
        Rule::Lantern => {
            tick_if_waterlogged(ctx, state, pos);
            let support = if ctx.property(state, "hanging") == Some("true") { Direction::Up } else { Direction::Down };
            if direction == support && !survives(ctx) {
                air
            } else {
                state
            }
        }
        Rule::DownSurvive => {
            if direction == Direction::Down && !survives(ctx) {
                return air;
            }
            tick_if_waterlogged(ctx, state, pos);
            state
        }
        Rule::BehindSurvive => {
            if direction.opposite() == facing(ctx) && !survives(ctx) {
                return air;
            }
            tick_if_waterlogged(ctx, state, pos);
            state
        }
        Rule::Repeater => {
            if direction == Direction::Down && !diode_can_survive_on(ctx, neighbor) {
                air
            } else if direction.axis() != facing(ctx).axis() {
                ctx.with(state, "locked", bool_name(repeater_locked(ctx, pos, state)))
            } else {
                state
            }
        }
        Rule::Comparator => {
            if direction == Direction::Down && !diode_can_survive_on(ctx, neighbor) {
                air
            } else {
                state
            }
        }
        Rule::Observer => {
            if facing(ctx) == direction && ctx.property(state, "powered") == Some("false") && !ctx.region.has_block_tick(pos, block) {
                ctx.region.schedule_block(pos, block, 2);
            }
            state
        }
        Rule::FaceAttached => {
            let support = match ctx.property(state, "face") {
                Some("ceiling") => Direction::Up,
                Some("floor") => Direction::Down,
                _ => facing(ctx).opposite(),
            };
            if direction == support && !survives(ctx) {
                air
            } else {
                state
            }
        }
        Rule::Bell => {
            let attachment = ctx.property(state, "attachment").unwrap_or("floor");
            let facing = facing(ctx);
            // `getConnectedDirection(state).getOpposite()`.
            let connected = match attachment {
                "floor" => Direction::Down,
                "ceiling" => Direction::Up,
                _ => facing,
            };
            if connected == direction && !survives(ctx) && attachment != "double_wall" {
                return air;
            }
            if direction.axis() == facing.axis() {
                let blocks = &ctx.registries().blocks;
                if attachment == "double_wall" && !blocks.is_face_sturdy(neighbor, direction, SupportType::Full) {
                    let single = ctx.with(state, "attachment", "single_wall");
                    return ctx.with(single, "facing", direction.opposite().name());
                }
                if attachment == "single_wall" && connected.opposite() == direction && blocks.is_face_sturdy(neighbor, facing, SupportType::Full) {
                    return ctx.with(state, "attachment", "double_wall");
                }
            }
            state
        }
        Rule::TickIfCovered => {
            if direction == Direction::Up {
                let above = ctx.block(pos.above());
                let solid = ctx.registries().blocks.is(above, flags::LEGACY_SOLID);
                let keeps = if ctx.is(state, "minecraft:farmland") {
                    lib.update_rules.maintains_farmland.is_some_and(|tag| ctx.in_tag(above, tag))
                } else {
                    is_a(ctx, above, "FenceGateBlock")
                };
                if solid && !keeps {
                    ctx.region.schedule_block(pos, block, 1);
                }
            }
            state
        }
        Rule::Tick => {
            // `FallingBlock.getDelayAfterPlace` and `BrushableBlock`: 2 ticks.
            ctx.region.schedule_block(pos, block, 2);
            state
        }
    }
}

fn bool_name(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn tick_if_waterlogged<W: World + ?Sized>(ctx: &mut Ctx<W>, state: BlockStateId, pos: BlockPos) {
    if waterlogged(ctx, state) {
        ctx.schedule_fluid_tick_for(pos, state);
    }
}

/// Whether a state's block is an instance of a vanilla class.
fn is_a<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId, class: &str) -> bool {
    let blocks = &ctx.registries().blocks;
    blocks.block(blocks.block_of(state)).is_a(class)
}

fn state_facing<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId) -> Direction {
    ctx.property(state, "facing").and_then(Direction::from_name).unwrap_or(Direction::North)
}

/// `StairBlock.getStairsShape`.
fn stairs_shape<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId, pos: BlockPos) -> &'static str {
    let facing = state_facing(ctx, state);
    let half = ctx.property(state, "half");
    let is_stairs = |s: BlockStateId| is_a(ctx, s, "StairBlock");
    // `canTakeShape`: the side neighbour is not a stair continuing this one.
    let can_take_shape = |side: Direction| {
        let neighbor = ctx.block(pos.relative(side, 1));
        !is_stairs(neighbor) || state_facing(ctx, neighbor) != facing || ctx.property(neighbor, "half") != half
    };
    let behind = ctx.block(pos.relative(facing, 1));
    if is_stairs(behind) && ctx.property(behind, "half") == half {
        let behind_facing = state_facing(ctx, behind);
        if behind_facing.axis() != facing.axis() && can_take_shape(behind_facing.opposite()) {
            return if behind_facing == facing.counter_clockwise() { "outer_left" } else { "outer_right" };
        }
    }
    let front = ctx.block(pos.relative(facing.opposite(), 1));
    if is_stairs(front) && ctx.property(front, "half") == half {
        let front_facing = state_facing(ctx, front);
        if front_facing.axis() != facing.axis() && can_take_shape(front_facing) {
            return if front_facing == facing.counter_clockwise() { "inner_left" } else { "inner_right" };
        }
    }
    "straight"
}

/// `Block.isExceptionForConnection`.
fn is_exception_for_connection<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId) -> bool {
    is_a(ctx, state, "LeavesBlock")
        || ["minecraft:barrier", "minecraft:carved_pumpkin", "minecraft:jack_o_lantern", "minecraft:melon", "minecraft:pumpkin"]
            .iter()
            .any(|name| ctx.is(state, name))
        || ctx.in_tag(state, ctx.lib.update_rules.shulker_boxes)
}

/// `FenceGateBlock.connectsToDirection`.
fn gate_connects<W: World + ?Sized>(ctx: &Ctx<W>, gate: BlockStateId, direction: Direction) -> bool {
    state_facing(ctx, gate).axis() == direction.clockwise().axis()
}

/// `WallBlock.connectsTo`.
fn wall_connects_to<W: World + ?Sized>(ctx: &Ctx<W>, neighbor: BlockStateId, face_solid: bool, direction: Direction) -> bool {
    let gate = is_a(ctx, neighbor, "FenceGateBlock") && gate_connects(ctx, neighbor, direction);
    ctx.in_tag(neighbor, ctx.lib.update_rules.walls) || !is_exception_for_connection(ctx, neighbor) && face_solid || is_a(ctx, neighbor, "IronBarsBlock") || gate
}

/// `WallBlock.updateShape(level, state, topPos, topNeighbour, n, e, s, w)`:
/// side heights under the block above and whether the post rises. Sides
/// are in `Direction::HORIZONTAL` order.
fn wall_state<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId, above: BlockStateId, sides: [bool; 4]) -> BlockStateId {
    // Test shapes in sixteenths as (min_x, min_z, max_x, max_z): the post
    // column and each side's strip toward its face (`TEST_SHAPES_WALL`).
    const POST: [f64; 4] = [7.0, 7.0, 9.0, 9.0];
    const SIDE: [[f64; 4]; 4] = [[7.0, 0.0, 9.0, 9.0], [7.0, 7.0, 16.0, 9.0], [7.0, 7.0, 9.0, 16.0], [0.0, 7.0, 9.0, 9.0]];
    let face = ctx.registries().blocks.collision_shape(above);
    let covered = |rect: [f64; 4]| bottom_face_covers(face, rect.map(|v| v / 16.0));
    let mut out = state;
    let mut heights = ["none"; 4];
    for (i, direction) in Direction::HORIZONTAL.into_iter().enumerate() {
        heights[i] = if !sides[i] {
            "none"
        } else if covered(SIDE[i]) {
            "tall"
        } else {
            "low"
        };
        out = ctx.with(out, direction.name(), heights[i]);
    }
    // `shouldRaisePost`.
    let [north, east, south, west] = heights;
    let up = if is_a(ctx, above, "WallBlock") && ctx.property(above, "up") == Some("true") {
        true
    } else {
        let (n, e, s, w) = (north == "none", east == "none", south == "none", west == "none");
        if n && s && w && e || n != s || w != e {
            true
        } else if north == "tall" && south == "tall" || east == "tall" && west == "tall" {
            false
        } else {
            ctx.in_tag(above, ctx.lib.update_rules.wall_post_override) || covered(POST)
        }
    };
    ctx.with(out, "up", bool_name(up))
}

/// Whether the bottom face of a collision shape (`getFaceShape(DOWN)`)
/// covers a rectangle given as (min_x, min_z, max_x, max_z).
fn bottom_face_covers(shape: Option<&FaceShape>, rect: [f64; 4]) -> bool {
    let boxes = match shape {
        Some(FaceShape::Full) => return true,
        None | Some(FaceShape::Empty) => return false,
        Some(FaceShape::Boxes(boxes)) => boxes,
    };
    let bottom: Vec<&[f64; 6]> = boxes.iter().filter(|b| b[1] <= 1.0e-7).collect();
    let mut xs = vec![rect[0], rect[2]];
    let mut zs = vec![rect[1], rect[3]];
    for b in &bottom {
        xs.extend([b[0], b[3]].into_iter().filter(|&x| x > rect[0] && x < rect[2]));
        zs.extend([b[2], b[5]].into_iter().filter(|&z| z > rect[1] && z < rect[3]));
    }
    xs.sort_by(f64::total_cmp);
    zs.sort_by(f64::total_cmp);
    xs.dedup();
    zs.dedup();
    xs.windows(2).all(|x| {
        let cx = (x[0] + x[1]) / 2.0;
        zs.windows(2).all(|z| {
            let cz = (z[0] + z[1]) / 2.0;
            bottom.iter().any(|b| b[0] <= cx && cx <= b[3] && b[2] <= cz && cz <= b[5])
        })
    })
}

/// `ChestBlock.getConnectedDirection`.
fn chest_connected_direction<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId) -> Direction {
    let facing = state_facing(ctx, state);
    if ctx.property(state, "type") == Some("left") {
        facing.clockwise()
    } else {
        facing.counter_clockwise()
    }
}

/// `DiodeBlock.canSurviveOn`: a rigid top face.
fn diode_can_survive_on<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId) -> bool {
    ctx.registries().blocks.is_face_sturdy(state, Direction::Up, SupportType::Rigid)
}

/// `RepeaterBlock.isLocked`: a powered diode faces into either side
/// (`getAlternateSignal` with `sideInputDiodesOnly`).
pub fn repeater_locked<W: World + ?Sized>(ctx: &Ctx<W>, pos: BlockPos, state: BlockStateId) -> bool {
    let facing = state_facing(ctx, state);
    [facing.clockwise(), facing.counter_clockwise()].into_iter().any(|side| {
        let side_pos = pos.relative(side, 1);
        let side_state = ctx.block(side_pos);
        diode_direct_signal(ctx, side_pos, side_state, side) > 0
    })
}

/// `DiodeBlock.getDirectSignal` (0 for other blocks): its output towards
/// `direction` (the direction from the reader to the diode).
pub fn diode_direct_signal<W: World + ?Sized>(ctx: &Ctx<W>, pos: BlockPos, state: BlockStateId, direction: Direction) -> i32 {
    let repeater = is_a(ctx, state, "RepeaterBlock");
    if !repeater && !is_a(ctx, state, "ComparatorBlock") {
        return 0;
    }
    if state_facing(ctx, state) != direction || ctx.property(state, "powered") != Some("true") {
        return 0;
    }
    if repeater {
        15
    } else {
        ctx.region.comparator_output(pos)
    }
}

/// `RedstoneWireBlock.canSurviveOn`.
pub fn wire_can_survive_on<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId) -> bool {
    ctx.registries().blocks.is_face_sturdy(state, Direction::Up, SupportType::Full) || ctx.is(state, "minecraft:hopper")
}

/// `!level.getBlockState(pos.above()).isRedstoneConductor(level, pos)`.
fn wire_can_connect_up<W: World + ?Sized>(ctx: &Ctx<W>, pos: BlockPos) -> bool {
    !ctx.registries().blocks.is(ctx.block(pos.above()), flags::REDSTONE_CONDUCTOR)
}

/// `BlockState.shouldRedstoneWireConnectTo(level, pos, direction)`.
pub fn wire_should_connect_to<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId, direction: Option<Direction>) -> bool {
    if is_a(ctx, state, "RedstoneWireBlock") {
        true
    } else if is_a(ctx, state, "ObserverBlock") {
        direction == Some(state_facing(ctx, state))
    } else if is_a(ctx, state, "RepeaterBlock") {
        let facing = state_facing(ctx, state);
        direction == Some(facing) || direction == Some(facing.opposite())
    } else {
        ctx.registries().blocks.is(state, flags::SIGNAL_SOURCE) && direction.is_some()
    }
}

/// `RedstoneWireBlock.getConnectingSide(level, pos, direction, canConnectUp)`.
fn wire_connecting_side<W: World + ?Sized>(ctx: &Ctx<W>, pos: BlockPos, direction: Direction, can_connect_up: bool) -> &'static str {
    let relative_pos = pos.relative(direction, 1);
    let relative = ctx.block(relative_pos);
    let blocks = &ctx.registries().blocks;
    if can_connect_up {
        let placeable_above = is_a(ctx, relative, "TrapDoorBlock") || wire_can_survive_on(ctx, relative);
        if placeable_above && wire_should_connect_to(ctx, ctx.block(relative_pos.above()), None) {
            return if blocks.is_face_sturdy(relative, direction.opposite(), SupportType::Full) { "up" } else { "side" };
        }
    }
    let conductor = blocks.is(relative, flags::REDSTONE_CONDUCTOR);
    if !wire_should_connect_to(ctx, relative, Some(direction)) && (conductor || !wire_should_connect_to(ctx, ctx.block(relative_pos.below()), None)) {
        "none"
    } else {
        "side"
    }
}

/// Connected sides in `Direction::HORIZONTAL` order.
fn wire_sides<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId) -> [bool; 4] {
    Direction::HORIZONTAL.map(|d| ctx.property(state, d.name()).is_some_and(|v| v != "none"))
}

fn wire_is_cross<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId) -> bool {
    wire_sides(ctx, state).iter().all(|&c| c)
}

/// `crossState` with this state's power.
fn wire_cross<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId) -> BlockStateId {
    let blocks = &ctx.registries().blocks;
    let power = ctx.property(state, "power").unwrap_or("0");
    let mut cross = blocks.block(blocks.block_of(state)).default_state();
    for direction in Direction::HORIZONTAL {
        cross = ctx.with(cross, direction.name(), "side");
    }
    ctx.with(cross, "power", power)
}

/// `RedstoneWireBlock.getConnectionState`.
pub fn wire_connection_state<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId, pos: BlockPos) -> BlockStateId {
    let blocks = &ctx.registries().blocks;
    let was_dot = !wire_sides(ctx, state).iter().any(|&c| c);
    let power = ctx.property(state, "power").unwrap_or("0");
    let mut out = ctx.with(blocks.block(blocks.block_of(state)).default_state(), "power", power);
    // `getMissingConnections`.
    let up = wire_can_connect_up(ctx, pos);
    for direction in Direction::HORIZONTAL {
        if ctx.property(out, direction.name()) == Some("none") {
            out = ctx.with(out, direction.name(), wire_connecting_side(ctx, pos, direction, up));
        }
    }
    let [north, east, south, west] = wire_sides(ctx, out);
    if was_dot && !(north || east || south || west) {
        return out;
    }
    let north_south_empty = !north && !south;
    let east_west_empty = !east && !west;
    if !west && north_south_empty {
        out = ctx.with(out, "west", "side");
    }
    if !east && north_south_empty {
        out = ctx.with(out, "east", "side");
    }
    if !north && east_west_empty {
        out = ctx.with(out, "north", "side");
    }
    if !south && east_west_empty {
        out = ctx.with(out, "south", "side");
    }
    out
}

fn block_is_kelp<W: World + ?Sized>(ctx: &Ctx<W>, block: BlockId) -> bool {
    ctx.registries().blocks.block(block).name.as_str() == "minecraft:kelp"
}

/// `GrowingPlantHeadBlock.updateBodyAfterConvertedFromHead`.
fn head_to_body<W: World + ?Sized>(ctx: &Ctx<W>, head: BlockStateId, body: &str) -> BlockStateId {
    let body_state = ctx.registries().blocks.parse_state(body).expect("body exists");
    match ctx.property(head, "berries") {
        Some(b) => ctx.with(body_state, "berries", b),
        None => body_state,
    }
}

/// `VineBlock.getUpdatedState` and the air check of its `updateShape`.
fn vine_updated<W: World + ?Sized>(ctx: &Ctx<W>, mut state: BlockStateId, pos: BlockPos) -> BlockStateId {
    use super::kinds::simple::can_attach_to;
    let above_pos = pos.above();
    if ctx.property(state, "up") == Some("true") {
        let ok = can_attach_to(ctx, Direction::Down, ctx.block(above_pos));
        state = ctx.with(state, "up", if ok { "true" } else { "false" });
    }
    let above = ctx.block(above_pos);
    let is_vine_with = |face: &str| ctx.is(above, "minecraft:vine") && ctx.property(above, face) == Some("true");
    for direction in Direction::HORIZONTAL {
        let face = direction.name();
        if ctx.property(state, face) != Some("true") {
            continue;
        }
        let support = can_attach_to(ctx, direction, ctx.block(pos.relative(direction, 1))) || is_vine_with(face);
        state = ctx.with(state, face, if support { "true" } else { "false" });
    }
    let faces = ["up", "north", "east", "south", "west"].iter().any(|f| ctx.property(state, f) == Some("true"));
    if faces {
        state
    } else {
        ctx.lib.blocks.air
    }
}

/// `BaseCoralPlantTypeBlock.scanForWater` (waterlogged, or water beside)
/// and `CoralBlock.scanForWater` (water beside).
pub fn coral_scan_for_water<W: World + ?Sized>(ctx: &Ctx<W>, state: BlockStateId, pos: BlockPos) -> bool {
    if ctx.property(state, "waterlogged") == Some("true") {
        return true;
    }
    Direction::ALL.iter().any(|&d| matches!(ctx.fluid(ctx.block(pos.relative(d, 1))), FluidType::Water | FluidType::FlowingWater))
}

/// `BaseCoralPlantTypeBlock.tryScheduleDieTick` for living coral: 60 to 99
/// ticks from the level random when no water is found.
pub fn coral_try_schedule_die_tick<W: World + ?Sized>(ctx: &mut Ctx<W>, state: BlockStateId, pos: BlockPos, block: minecraftoss_core::BlockId) {
    if ctx.name(state).contains("dead_") || coral_scan_for_water(ctx, state, pos) {
        return;
    }
    let delay = 60 + ctx.region.random().next_i32_bound(40);
    ctx.region.schedule_block(pos, block, delay);
}

/// `ConcretePowderBlock.touchesLiquid`, quirk included: the check below reads
/// the block at the previous test position (the powder itself), and faces
/// are tested at the powder's position.
pub fn concrete_touches_liquid<W: World + ?Sized>(ctx: &Ctx<W>, pos: BlockPos) -> bool {
    let water = |s: BlockStateId| matches!(ctx.fluid(s), FluidType::Water | FluidType::FlowingWater);
    let mut test = pos;
    for direction in Direction::ALL {
        let state = ctx.block(test);
        if direction != Direction::Down || water(state) {
            test = pos.relative(direction, 1);
            let state = ctx.block(test);
            if water(state) && !ctx.registries().blocks.is_face_sturdy(state, direction.opposite(), minecraftoss_core::SupportType::Full) {
                return true;
            }
        }
    }
    false
}

/// `FireBlock.getIgniteOdds(state)` / `getBurnOdds(state)`: nothing burns
/// while waterlogged.
pub fn fire_odds(registries: &Registries, state: BlockStateId, burn: bool) -> i32 {
    let blocks = &registries.blocks;
    if blocks.property(state, "waterlogged") == Some("true") {
        return 0;
    }
    let info = blocks.state(state);
    i32::from(if burn { info.burn_odds } else { info.ignite_odds })
}

/// `FireBlock.canBurn`.
pub fn fire_can_burn(registries: &Registries, state: BlockStateId) -> bool {
    fire_odds(registries, state, false) > 0
}

/// `FireBlock.getStateForPlacement(level, pos)`: on an unburnable,
/// non-sturdy base the fire clings to the flammable sides.
pub fn fire_placement<W: World + ?Sized>(ctx: &Ctx<W>, pos: BlockPos) -> BlockStateId {
    let registries = ctx.registries();
    let fire = registries.blocks.parse_state("minecraft:fire").expect("vanilla block");
    let below = ctx.block(pos.below());
    if !fire_can_burn(registries, below) && !registries.blocks.is_face_sturdy(below, Direction::Up, minecraftoss_core::SupportType::Full) {
        let mut result = fire;
        for direction in Direction::ALL {
            if direction == Direction::Down {
                continue;
            }
            let burns = fire_can_burn(registries, ctx.block(pos.relative(direction, 1)));
            result = ctx.with(result, direction.name(), if burns { "true" } else { "false" });
        }
        result
    } else {
        fire
    }
}

/// `BaseFireBlock.getState`: soul fire on soul fire bases, fire otherwise.
pub fn base_fire_state<W: World + ?Sized>(ctx: &Ctx<W>, pos: BlockPos) -> BlockStateId {
    let registries = ctx.registries();
    let below = ctx.block(pos.below());
    if registries.block_in_tag(below, ctx.lib.update_rules.soul_fire_base) {
        registries.blocks.parse_state("minecraft:soul_fire").expect("vanilla block")
    } else {
        fire_placement(ctx, pos)
    }
}

/// `FireBlock.getStateWithAge`.
pub fn fire_state_with_age<W: World + ?Sized>(ctx: &Ctx<W>, pos: BlockPos, age: i32) -> BlockStateId {
    let state = base_fire_state(ctx, pos);
    if ctx.name(state) == "minecraft:fire" {
        ctx.with(state, "age", &age.to_string())
    } else {
        state
    }
}

/// `StructureTemplate.updateShapeAtEdge` over a shape's faces, in
/// `DiscreteVoxelShape.forAllFaces` order: Z faces by x, y, z; then Y faces
/// by z, x, y; then X faces by y, z, x.
pub fn shape_at_edge(ctx: &mut Ctx, shape: &VoxelShape, flags: u32) {
    let (sx, sy, sz) = shape.size;
    let visit = |ctx: &mut Ctx, direction: Direction, x: i32, y: i32, z: i32| {
        let pos = shape.origin.offset(x, y, z);
        let neighbor_pos = pos.relative(direction, 1);
        let state = ctx.block(pos);
        let neighbor = ctx.block(neighbor_pos);
        let new_state = update_shape(ctx, state, pos, direction, neighbor);
        if new_state != state {
            ctx.set_block_flags(pos, new_state, flags & !1);
        }
        let new_neighbor = update_shape(ctx, neighbor, neighbor_pos, direction.opposite(), new_state);
        if new_neighbor != neighbor {
            ctx.set_block_flags(neighbor_pos, new_neighbor, flags & !1);
        }
    };
    // Faces along one axis: runs of full cells produce a negative face at
    // their start and a positive face after their end.
    let axis_pass = |ctx: &mut Ctx, a_size: i32, b_size: i32, c_size: i32, map: &dyn Fn(i32, i32, i32) -> (i32, i32, i32), neg: Direction, pos_dir: Direction| {
        for a in 0..a_size {
            for b in 0..b_size {
                let mut last = false;
                for c in 0..=c_size {
                    let full = c != c_size && {
                        let (x, y, z) = map(a, b, c);
                        shape.is_full(x, y, z)
                    };
                    if !last && full {
                        let (x, y, z) = map(a, b, c);
                        visit(ctx, neg, x, y, z);
                    }
                    if last && !full {
                        let (x, y, z) = map(a, b, c - 1);
                        visit(ctx, pos_dir, x, y, z);
                    }
                    last = full;
                }
            }
        }
    };
    axis_pass(ctx, sx, sy, sz, &|a, b, c| (a, b, c), Direction::North, Direction::South);
    axis_pass(ctx, sz, sx, sy, &|a, b, c| (b, c, a), Direction::Down, Direction::Up);
    axis_pass(ctx, sy, sz, sx, &|a, b, c| (c, a, b), Direction::West, Direction::East);
}
