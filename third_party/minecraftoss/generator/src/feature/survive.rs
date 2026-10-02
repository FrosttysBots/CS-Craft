//! `BlockState.canSurvive` for the blocks world generation places or checks.
//! Each rule follows the block's class in the pinned 26.3 JAR; 26.3 moved most
//! supports into block and fluid tags, which load from the data pack. Blocks
//! without a rule here survive anywhere, as `BlockBehaviour.canSurvive` does.

use super::blocks::FluidType;
use minecraftoss_core::block::FaceShape;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::tags::TagId;
use minecraftoss_core::{BlockId, BlockPos, BlockStateId, Registries, SupportType};
use std::collections::HashMap;

/// Whether a shape covers a whole face of the block (`Block.isFaceFull`).
pub fn is_face_full(shape: &FaceShape, direction: Direction) -> bool {
    let boxes = match shape {
        FaceShape::Full => return true,
        FaceShape::Empty => return false,
        FaceShape::Boxes(boxes) => boxes,
    };
    // Axis index of the face normal, and whether it points to the max side.
    let (axis, positive) = match direction {
        Direction::Down => (1, false),
        Direction::Up => (1, true),
        Direction::North => (2, false),
        Direction::South => (2, true),
        Direction::West => (0, false),
        Direction::East => (0, true),
    };
    let (u, v) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let touching: Vec<&[f64; 6]> = boxes
        .iter()
        .filter(|b| if positive { b[axis + 3] >= 1.0 } else { b[axis] <= 0.0 })
        .collect();
    if touching.is_empty() {
        return false;
    }
    let mut us = vec![0.0, 1.0];
    let mut vs = vec![0.0, 1.0];
    for b in &touching {
        us.extend([b[u], b[u + 3]]);
        vs.extend([b[v], b[v + 3]]);
    }
    us.sort_by(f64::total_cmp);
    vs.sort_by(f64::total_cmp);
    us.dedup();
    vs.dedup();
    for pu in us.windows(2).filter(|w| w[0] >= 0.0 && w[1] <= 1.0 && w[1] > w[0]) {
        for pv in vs.windows(2).filter(|w| w[0] >= 0.0 && w[1] <= 1.0 && w[1] > w[0]) {
            let (cu, cv) = ((pu[0] + pu[1]) / 2.0, (pv[0] + pv[1]) / 2.0);
            if !touching.iter().any(|b| b[u] <= cu && cu <= b[u + 3] && b[v] <= cv && cv <= b[v + 3]) {
                return false;
            }
        }
    }
    true
}

/// Tags and blocks survival rules test, resolved once per data pack.
pub struct Survival {
    tags: HashMap<&'static str, TagId>,
    fluid_tags: HashMap<String, Vec<FluidType>>,
    rules: super::BlockMap<Rule>,
    /// `DoublePlantBlock` and its subclasses.
    double_plants: Vec<BlockId>,
    mossy_carpet: BlockId,
    /// `getRawBrightness` in a chunk that has no light yet: full skylight in
    /// dimensions with a sky, zero without one.
    pub unlit_raw_brightness: i32,
}

#[derive(Clone, Copy, Debug)]
enum Rule {
    /// `VegetationBlock`: the block below is in a tag.
    Below(&'static str),
    /// `DoublePlantBlock`: the lower half as `Below`, the upper half on its lower half.
    DoublePlant(&'static str),
    Mushroom,
    LilyPad,
    Seagrass,
    TallSeagrass,
    SeaPickle,
    /// `GrowingPlantBlock`: attached to the block against its growth.
    Growing { up: bool, head: &'static str, body: &'static str, cannot_attach: Option<&'static str> },
    LeafLitter,
    SmallDripleaf,
    SporeBlossom,
    Carpet,
    MossyCarpet,
    Cactus,
    CactusFlower,
    SugarCane,
    MangrovePropagule,
    Fire,
    SoulFire,
    SnowLayer,
    Multiface,
    HangingRoots,
    Speleothem,
    BigDripleaf,
    BigDripleafStem,
    Bamboo,
    ChorusPlant,
    ChorusFlower,
    Cocoa,
    AmethystCluster,
    HangingMoss,
    Vine,
    /// `CropBlock`: farmland below and enough light.
    Crop,
    /// `BaseTorchBlock`, `CandleBlock`: centre support below.
    BelowCenter,
    /// `WallTorchBlock`, `LadderBlock`: a sturdy face behind, against `facing`.
    Behind,
    /// `WallBannerBlock`, `WallSignBlock`: a solid block behind.
    SolidBehind,
    /// `BannerBlock`, `StandingSignBlock`: a solid block below.
    SolidBelow,
    Lantern,
    FaceAttached,
    PressurePlate,
    Door,
    TripwireHook,
    Bell,
    PistonHead,
    RedstoneWire,
    /// `DiodeBlock`: a rigid top face below.
    Diode,
}

const TAGS: &[&str] = &[
    "minecraft:supports_vegetation",
    "minecraft:supports_dry_vegetation",
    "minecraft:supports_wither_rose",
    "minecraft:supports_azalea",
    "minecraft:supports_mangrove_propagule",
    "minecraft:supports_hanging_mangrove_propagule",
    "minecraft:overrides_mushroom_light_requirement",
    "minecraft:supports_lily_pad",
    "minecraft:cannot_support_seagrass",
    "minecraft:cannot_support_kelp",
    "minecraft:supports_crimson_fungus",
    "minecraft:supports_warped_fungus",
    "minecraft:supports_crimson_roots",
    "minecraft:supports_warped_roots",
    "minecraft:supports_nether_sprouts",
    "minecraft:supports_small_dripleaf",
    "minecraft:supports_cactus",
    "minecraft:support_override_cactus_flower",
    "minecraft:supports_sugar_cane",
    "minecraft:supports_sugar_cane_adjacently",
    "minecraft:soul_fire_base_blocks",
    "minecraft:cannot_support_snow_layer",
    "minecraft:support_override_snow_layer",
    "minecraft:supports_big_dripleaf",
    "minecraft:supports_bamboo",
    "minecraft:supports_chorus_plant",
    "minecraft:supports_chorus_flower",
    "minecraft:supports_cocoa",
    "minecraft:supports_crops",
    "minecraft:supports_pumpkin_stem",
    "minecraft:supports_melon_stem",
    "minecraft:supports_nether_wart",
];

impl Survival {
    pub fn load(registries: &Registries) -> Result<Self, String> {
        let mut tags = HashMap::new();
        for &name in TAGS {
            tags.insert(name, registries.block_tags.require(name)?);
        }
        let mut fluid_tags = HashMap::new();
        for name in registries.datapack.list("tags/fluid")? {
            let json = registries.datapack.read_json("tags/fluid", &name)?;
            let mut fluids = Vec::new();
            for value in json["values"].as_array().ok_or("fluid tag lacks values")? {
                let entry = value.as_str().or_else(|| value["id"].as_str()).ok_or("bad fluid tag entry")?;
                fluids.extend(FluidType::parse_set(&serde_json::Value::String(entry.to_owned()))?);
            }
            fluid_tags.insert(name.to_string(), fluids);
        }
        let mut rules = super::BlockMap::default();
        let mut rule = |names: &[&str], r: Rule| -> Result<(), String> {
            for name in names {
                let id = registries.blocks.block_by_name(&format!("minecraft:{name}")).ok_or_else(|| format!("unknown block {name}"))?;
                rules.insert(id, r);
            }
            Ok(())
        };
        let vegetation = "minecraft:supports_vegetation";
        rule(
            &[
                "oak_sapling", "spruce_sapling", "birch_sapling", "jungle_sapling", "acacia_sapling", "cherry_sapling",
                "dark_oak_sapling", "pale_oak_sapling", "poplar_sapling", "dandelion", "poppy", "blue_orchid", "allium",
                "azure_bluet", "red_tulip", "orange_tulip", "white_tulip", "pink_tulip", "oxeye_daisy", "cornflower",
                "lily_of_the_valley", "torchflower", "open_eyeblossom", "closed_eyeblossom", "golden_dandelion", "red_shrub", "short_grass",
                "fern", "bush", "firefly_bush", "sweet_berry_bush", "pink_petals", "wildflowers",
            ],
            Rule::Below(vegetation),
        )?;
        rule(&["tall_grass", "large_fern", "lilac", "rose_bush", "peony", "sunflower", "pitcher_plant"], Rule::DoublePlant(vegetation))?;
        rule(&["dead_bush", "short_dry_grass", "tall_dry_grass"], Rule::Below("minecraft:supports_dry_vegetation"))?;
        rule(&["wither_rose"], Rule::Below("minecraft:supports_wither_rose"))?;
        rule(&["azalea", "flowering_azalea"], Rule::Below("minecraft:supports_azalea"))?;
        rule(&["mangrove_propagule"], Rule::MangrovePropagule)?;
        rule(&["brown_mushroom", "red_mushroom"], Rule::Mushroom)?;
        rule(&["lily_pad"], Rule::LilyPad)?;
        rule(&["seagrass"], Rule::Seagrass)?;
        rule(&["tall_seagrass"], Rule::TallSeagrass)?;
        rule(&["sea_pickle"], Rule::SeaPickle)?;
        rule(&["kelp", "kelp_plant"], Rule::Growing { up: true, head: "minecraft:kelp", body: "minecraft:kelp_plant", cannot_attach: Some("minecraft:cannot_support_kelp") })?;
        rule(&["cave_vines", "cave_vines_plant"], Rule::Growing { up: false, head: "minecraft:cave_vines", body: "minecraft:cave_vines_plant", cannot_attach: None })?;
        rule(&["weeping_vines", "weeping_vines_plant"], Rule::Growing { up: false, head: "minecraft:weeping_vines", body: "minecraft:weeping_vines_plant", cannot_attach: None })?;
        rule(&["twisting_vines", "twisting_vines_plant"], Rule::Growing { up: true, head: "minecraft:twisting_vines", body: "minecraft:twisting_vines_plant", cannot_attach: None })?;
        rule(&["crimson_fungus"], Rule::Below("minecraft:supports_crimson_fungus"))?;
        rule(&["warped_fungus"], Rule::Below("minecraft:supports_warped_fungus"))?;
        rule(&["crimson_roots"], Rule::Below("minecraft:supports_crimson_roots"))?;
        rule(&["warped_roots"], Rule::Below("minecraft:supports_warped_roots"))?;
        rule(&["nether_sprouts"], Rule::Below("minecraft:supports_nether_sprouts"))?;
        rule(&["leaf_litter"], Rule::LeafLitter)?;
        rule(&["small_dripleaf"], Rule::SmallDripleaf)?;
        rule(&["spore_blossom"], Rule::SporeBlossom)?;
        rule(&["moss_carpet"], Rule::Carpet)?;
        rule(&["pale_moss_carpet"], Rule::MossyCarpet)?;
        rule(&["cactus"], Rule::Cactus)?;
        rule(&["cactus_flower"], Rule::CactusFlower)?;
        rule(&["sugar_cane"], Rule::SugarCane)?;
        rule(&["fire"], Rule::Fire)?;
        rule(&["soul_fire"], Rule::SoulFire)?;
        rule(&["snow"], Rule::SnowLayer)?;
        rule(&["glow_lichen", "sculk_vein", "resin_clump"], Rule::Multiface)?;
        rule(&["hanging_roots"], Rule::HangingRoots)?;
        rule(&["pointed_dripstone", "sulfur_spike"], Rule::Speleothem)?;
        rule(&["big_dripleaf"], Rule::BigDripleaf)?;
        rule(&["big_dripleaf_stem"], Rule::BigDripleafStem)?;
        rule(&["bamboo", "bamboo_sapling"], Rule::Bamboo)?;
        rule(&["chorus_plant"], Rule::ChorusPlant)?;
        rule(&["chorus_flower"], Rule::ChorusFlower)?;
        rule(&["cocoa"], Rule::Cocoa)?;
        rule(&["amethyst_cluster", "large_amethyst_bud", "medium_amethyst_bud", "small_amethyst_bud"], Rule::AmethystCluster)?;
        rule(&["pale_hanging_moss"], Rule::HangingMoss)?;
        rule(&["vine"], Rule::Vine)?;
        rule(&["pumpkin_stem", "attached_pumpkin_stem"], Rule::Below("minecraft:supports_pumpkin_stem"))?;
        rule(&["melon_stem", "attached_melon_stem"], Rule::Below("minecraft:supports_melon_stem"))?;
        rule(&["nether_wart"], Rule::Below("minecraft:supports_nether_wart"))?;
        // The rest by vanilla class, from the schema 4 catalog; the most
        // derived class that has a rule decides.
        let by_class: &[(&str, Option<Rule>)] = &[
            ("GrindstoneBlock", None),
            ("CropBlock", Some(Rule::Crop)),
            ("WallTorchBlock", Some(Rule::Behind)),
            ("RedstoneWallTorchBlock", Some(Rule::Behind)),
            ("LadderBlock", Some(Rule::Behind)),
            ("BaseTorchBlock", Some(Rule::BelowCenter)),
            ("CandleBlock", Some(Rule::BelowCenter)),
            ("WallBannerBlock", Some(Rule::SolidBehind)),
            ("WallSignBlock", Some(Rule::SolidBehind)),
            ("BannerBlock", Some(Rule::SolidBelow)),
            ("StandingSignBlock", Some(Rule::SolidBelow)),
            ("LanternBlock", Some(Rule::Lantern)),
            ("FaceAttachedHorizontalDirectionalBlock", Some(Rule::FaceAttached)),
            ("BasePressurePlateBlock", Some(Rule::PressurePlate)),
            ("DoorBlock", Some(Rule::Door)),
            ("CarpetBlock", Some(Rule::Carpet)),
            ("TripWireHookBlock", Some(Rule::TripwireHook)),
            ("BellBlock", Some(Rule::Bell)),
            ("PistonHeadBlock", Some(Rule::PistonHead)),
            ("RedstoneWireBlock", Some(Rule::RedstoneWire)),
            ("DiodeBlock", Some(Rule::Diode)),
        ];
        for (id, info) in registries.blocks.blocks() {
            if rules.contains_key(&id) {
                continue;
            }
            let found = info.classes().iter().find_map(|class| by_class.iter().find(|(name, _)| **name == **class));
            if let Some((_, Some(r))) = found {
                rules.insert(id, *r);
            }
        }
        let double_plants = [
            "tall_grass", "large_fern", "sunflower", "lilac", "rose_bush", "peony", "pitcher_plant", "pitcher_crop", "small_dripleaf", "tall_seagrass",
        ]
        .iter()
        .map(|n| registries.blocks.block_by_name(&format!("minecraft:{n}")).ok_or_else(|| format!("unknown block {n}")))
        .collect::<Result<_, _>>()?;
        let mossy_carpet = registries.blocks.block_by_name("minecraft:pale_moss_carpet").ok_or("unknown block pale_moss_carpet")?;
        Ok(Self { tags, fluid_tags, rules, double_plants, mossy_carpet, unlit_raw_brightness: 15 })
    }

    /// How `SimpleBlockFeature` places a state: as a double plant, a mossy carpet or a plain block.
    pub fn shape_class(&self, registries: &Registries, state: BlockStateId) -> super::kinds::simple::PlantShape {
        let block = registries.blocks.block_of(state);
        if self.double_plants.contains(&block) {
            super::kinds::simple::PlantShape::DoublePlant
        } else if block == self.mossy_carpet {
            super::kinds::simple::PlantShape::MossyCarpet
        } else {
            super::kinds::simple::PlantShape::Other
        }
    }

    fn tag(&self, name: &str) -> TagId {
        self.tags[name]
    }

    fn fluid_in(&self, tag: &str, fluid: FluidType) -> bool {
        self.fluid_tags.get(tag).is_some_and(|fluids| fluids.contains(&fluid))
    }

    /// `BlockState.canSurvive(level, pos)`.
    pub fn can_survive<W: super::World + ?Sized>(&self, registries: &Registries, region: &W, state: BlockStateId, (x, y, z): (i32, i32, i32)) -> bool {
        let blocks = &registries.blocks;
        let Some(&rule) = self.rules.get(&blocks.block_of(state)) else {
            return true;
        };
        let get = |dx: i32, dy: i32, dz: i32| region.block_at(x + dx, y + dy, z + dz);
        let tagged = |s: BlockStateId, tag: &str| registries.block_in_tag(s, self.tag(tag));
        let is = |s: BlockStateId, name: &str| blocks.block(blocks.block_of(s)).name.as_str() == name;
        let sturdy = |s: BlockStateId, d: Direction| blocks.is_face_sturdy(s, d, SupportType::Full);
        let fluid = |s: BlockStateId| super::blocks::Behaviour { registries }.fluid(s);
        let collision_full = |s: BlockStateId, d: Direction| blocks.collision_shape(s).is_some_and(|shape| is_face_full(shape, d));
        match rule {
            Rule::Below(tag) => tagged(get(0, -1, 0), tag),
            Rule::DoublePlant(tag) => {
                if blocks.property(state, "half") == Some("upper") {
                    let below = get(0, -1, 0);
                    blocks.block_of(below) == blocks.block_of(state) && blocks.property(below, "half") == Some("lower")
                } else {
                    tagged(get(0, -1, 0), tag)
                }
            }
            Rule::MangrovePropagule => {
                if blocks.property(state, "hanging") == Some("true") {
                    tagged(get(0, 1, 0), "minecraft:supports_hanging_mangrove_propagule")
                } else {
                    tagged(get(0, -1, 0), "minecraft:supports_mangrove_propagule")
                }
            }
            Rule::Mushroom => {
                let below = get(0, -1, 0);
                let raw = region.light(BlockPos::new(x, y, z), 0).map_or(self.unlit_raw_brightness, |(raw, _)| raw);
                tagged(below, "minecraft:overrides_mushroom_light_requirement")
                    || raw < 13 && blocks.is(below, minecraftoss_core::block::flags::SOLID_RENDER)
            }
            Rule::LilyPad => {
                let (below, at) = (get(0, -1, 0), get(0, 0, 0));
                (self.fluid_in("minecraft:supports_lily_pad", fluid(below)) || tagged(below, "minecraft:supports_lily_pad")) && fluid(at) == FluidType::Empty
            }
            Rule::Seagrass => {
                let below = get(0, -1, 0);
                sturdy(below, Direction::Up) && !tagged(below, "minecraft:cannot_support_seagrass")
            }
            Rule::TallSeagrass => {
                if blocks.property(state, "half") == Some("upper") {
                    let below = get(0, -1, 0);
                    is(below, "minecraft:tall_seagrass") && blocks.property(below, "half") == Some("lower")
                } else {
                    let below = get(0, -1, 0);
                    // FluidState.isFull: amount 8 (a source or a falling column).
                    let full = registries.blocks.state(get(0, 0, 0)).fluid.is_some_and(|f| f.amount == 8);
                    sturdy(below, Direction::Up) && !tagged(below, "minecraft:cannot_support_seagrass") && matches!(fluid(get(0, 0, 0)), FluidType::Water | FluidType::FlowingWater) && full
                }
            }
            Rule::SeaPickle => {
                let below = get(0, -1, 0);
                let collision_top = blocks.collision_shape(below).is_some_and(|shape| face_nonempty(shape, Direction::Up));
                collision_top || sturdy(below, Direction::Up)
            }
            Rule::Growing { up, head, body, cannot_attach } => {
                let attached = if up { get(0, -1, 0) } else { get(0, 1, 0) };
                if cannot_attach.is_some_and(|tag| tagged(attached, tag)) {
                    return false;
                }
                is(attached, head) || is(attached, body) || sturdy(attached, if up { Direction::Up } else { Direction::Down })
            }
            Rule::LeafLitter => sturdy(get(0, -1, 0), Direction::Up),
            Rule::SmallDripleaf => {
                if blocks.property(state, "half") == Some("upper") {
                    let below = get(0, -1, 0);
                    blocks.block_of(below) == blocks.block_of(state) && blocks.property(below, "half") == Some("lower")
                } else {
                    let below = get(0, -1, 0);
                    let water_source_above = fluid(get(0, 0, 0)) == FluidType::Water;
                    tagged(below, "minecraft:supports_small_dripleaf") || (water_source_above && tagged(below, "minecraft:supports_vegetation"))
                }
            }
            Rule::SporeBlossom => {
                blocks.is_face_sturdy(get(0, 1, 0), Direction::Down, SupportType::Center) && !matches!(fluid(get(0, 0, 0)), FluidType::Water | FluidType::FlowingWater)
            }
            Rule::Carpet => !blocks.is_air(get(0, -1, 0)),
            Rule::MossyCarpet => {
                let below = get(0, -1, 0);
                if blocks.property(state, "bottom") == Some("true") {
                    !blocks.is_air(below)
                } else {
                    blocks.block_of(below) == blocks.block_of(state) && blocks.property(below, "bottom") == Some("true")
                }
            }
            Rule::Cactus => {
                for (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                    let neighbor = get(dx, 0, dz);
                    if blocks.is(neighbor, minecraftoss_core::block::flags::LEGACY_SOLID) || matches!(fluid(neighbor), FluidType::Lava | FluidType::FlowingLava) {
                        return false;
                    }
                }
                let below = get(0, -1, 0);
                (is(below, "minecraft:cactus") || tagged(below, "minecraft:supports_cactus")) && !blocks.is(get(0, 1, 0), minecraftoss_core::block::flags::LIQUID)
            }
            Rule::CactusFlower => {
                let below = get(0, -1, 0);
                tagged(below, "minecraft:support_override_cactus_flower") || blocks.is_face_sturdy(below, Direction::Up, SupportType::Center)
            }
            Rule::SugarCane => {
                let below = get(0, -1, 0);
                if is(below, "minecraft:sugar_cane") {
                    return true;
                }
                if tagged(below, "minecraft:supports_sugar_cane") {
                    for (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                        let neighbor = get(dx, -1, dz);
                        if self.fluid_in("minecraft:supports_sugar_cane_adjacently", fluid(neighbor)) || tagged(neighbor, "minecraft:supports_sugar_cane_adjacently") {
                            return true;
                        }
                    }
                }
                false
            }
            // `FireBlock.canSurvive`: a sturdy block below, or something
            // flammable beside it.
            Rule::Fire => {
                sturdy(get(0, -1, 0), Direction::Up)
                    || Direction::ALL.iter().any(|d| {
                        let (dx, dy, dz) = d.offset();
                        crate::feature::update::fire_can_burn(registries, get(dx, dy, dz))
                    })
            }
            Rule::SoulFire => tagged(get(0, -1, 0), "minecraft:soul_fire_base_blocks"),
            Rule::SnowLayer => {
                let below = get(0, -1, 0);
                if tagged(below, "minecraft:cannot_support_snow_layer") {
                    return false;
                }
                if tagged(below, "minecraft:support_override_snow_layer") {
                    return true;
                }
                collision_full(below, Direction::Up) || (is(below, "minecraft:snow") && blocks.property(below, "layers") == Some("8"))
            }
            Rule::Multiface => {
                let mut any = false;
                for direction in Direction::ALL {
                    if blocks.property(state, direction_name(direction)) != Some("true") {
                        continue;
                    }
                    let (dx, dy, dz) = direction.offset();
                    let neighbor = get(dx, dy, dz);
                    if !(sturdy(neighbor, direction.opposite()) || collision_full(neighbor, direction.opposite())) {
                        return false;
                    }
                    any = true;
                }
                any
            }
            Rule::HangingRoots => sturdy(get(0, 1, 0), Direction::Down),
            Rule::Speleothem => {
                let tip_up = blocks.property(state, "vertical_direction") == Some("up");
                let tip = if tip_up { Direction::Up } else { Direction::Down };
                let behind = if tip_up { get(0, -1, 0) } else { get(0, 1, 0) };
                sturdy(behind, tip) || (blocks.block_of(behind) == blocks.block_of(state) && blocks.property(behind, "vertical_direction") == blocks.property(state, "vertical_direction"))
            }
            Rule::BigDripleaf => {
                let below = get(0, -1, 0);
                is(below, "minecraft:big_dripleaf") || is(below, "minecraft:big_dripleaf_stem") || tagged(below, "minecraft:supports_big_dripleaf")
            }
            Rule::BigDripleafStem => {
                let (below, above) = (get(0, -1, 0), get(0, 1, 0));
                (is(below, "minecraft:big_dripleaf_stem") || tagged(below, "minecraft:supports_big_dripleaf"))
                    && (is(above, "minecraft:big_dripleaf_stem") || is(above, "minecraft:big_dripleaf"))
            }
            Rule::Bamboo => tagged(get(0, -1, 0), "minecraft:supports_bamboo"),
            Rule::ChorusPlant => {
                let below = get(0, -1, 0);
                let vertical = !blocks.is_air(get(0, 1, 0)) && !blocks.is_air(below);
                for (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                    if !is(get(dx, 0, dz), "minecraft:chorus_plant") {
                        continue;
                    }
                    if vertical {
                        return false;
                    }
                    let under = get(dx, -1, dz);
                    if is(under, "minecraft:chorus_plant") || tagged(under, "minecraft:supports_chorus_plant") {
                        return true;
                    }
                }
                is(below, "minecraft:chorus_plant") || tagged(below, "minecraft:supports_chorus_plant")
            }
            Rule::ChorusFlower => {
                let below = get(0, -1, 0);
                if is(below, "minecraft:chorus_plant") || tagged(below, "minecraft:supports_chorus_flower") {
                    return true;
                }
                if !blocks.is_air(below) {
                    return false;
                }
                let mut one = false;
                for (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                    let neighbor = get(dx, 0, dz);
                    if is(neighbor, "minecraft:chorus_plant") {
                        if one {
                            return false;
                        }
                        one = true;
                    } else if !blocks.is_air(neighbor) {
                        return false;
                    }
                }
                one
            }
            // `CropBlock.hasSufficientLight`: raw brightness 8, or the sky.
            Rule::Crop => {
                let lit = region.light(BlockPos::new(x, y, z), 0).map_or(self.unlit_raw_brightness >= 8, |(raw, sky)| raw >= 8 || sky);
                tagged(get(0, -1, 0), "minecraft:supports_crops") && lit
            }
            Rule::BelowCenter => blocks.is_face_sturdy(get(0, -1, 0), Direction::Up, SupportType::Center),
            Rule::Behind | Rule::SolidBehind => {
                let facing = blocks.property(state, "facing").and_then(parse_direction).unwrap_or(Direction::North);
                let (dx, dy, dz) = facing.opposite().offset();
                let behind = get(dx, dy, dz);
                if matches!(rule, Rule::Behind) {
                    sturdy(behind, facing)
                } else {
                    blocks.is(behind, minecraftoss_core::block::flags::LEGACY_SOLID)
                }
            }
            Rule::SolidBelow => blocks.is(get(0, -1, 0), minecraftoss_core::block::flags::LEGACY_SOLID),
            Rule::Lantern => {
                if blocks.property(state, "hanging") == Some("true") {
                    blocks.is_face_sturdy(get(0, 1, 0), Direction::Down, SupportType::Center)
                } else {
                    blocks.is_face_sturdy(get(0, -1, 0), Direction::Up, SupportType::Center)
                }
            }
            Rule::FaceAttached => {
                // `canAttach(level, pos, getConnectedDirection(state).getOpposite())`.
                let toward = match blocks.property(state, "face") {
                    Some("ceiling") => Direction::Up,
                    Some("floor") => Direction::Down,
                    _ => blocks.property(state, "facing").and_then(parse_direction).unwrap_or(Direction::North).opposite(),
                };
                let (dx, dy, dz) = toward.offset();
                sturdy(get(dx, dy, dz), toward.opposite())
            }
            Rule::PressurePlate => {
                let below = get(0, -1, 0);
                blocks.is_face_sturdy(below, Direction::Up, SupportType::Rigid) || blocks.is_face_sturdy(below, Direction::Up, SupportType::Center)
            }
            Rule::Door => {
                let below = get(0, -1, 0);
                if blocks.property(state, "half") == Some("upper") {
                    blocks.block_of(below) == blocks.block_of(state)
                } else {
                    sturdy(below, Direction::Up)
                }
            }
            Rule::TripwireHook => {
                let facing = blocks.property(state, "facing").and_then(parse_direction).unwrap_or(Direction::North);
                let (dx, dy, dz) = facing.opposite().offset();
                facing.is_horizontal() && sturdy(get(dx, dy, dz), facing)
            }
            Rule::Bell => {
                // The side the bell hangs from: `getConnectedDirection(state).getOpposite()`.
                let facing = blocks.property(state, "facing").and_then(parse_direction).unwrap_or(Direction::North);
                let toward = match blocks.property(state, "attachment") {
                    Some("floor") => Direction::Down,
                    Some("ceiling") => Direction::Up,
                    _ => facing,
                };
                let (dx, dy, dz) = toward.offset();
                if toward == Direction::Up {
                    blocks.is_face_sturdy(get(dx, dy, dz), Direction::Down, SupportType::Center)
                } else {
                    sturdy(get(dx, dy, dz), toward.opposite())
                }
            }
            Rule::PistonHead => {
                let facing = blocks.property(state, "facing").and_then(parse_direction).unwrap_or(Direction::North);
                let (dx, dy, dz) = facing.opposite().offset();
                let base = get(dx, dy, dz);
                let base_name = if blocks.property(state, "type") == Some("sticky") { "minecraft:sticky_piston" } else { "minecraft:piston" };
                let same_facing = blocks.property(base, "facing") == blocks.property(state, "facing");
                (is(base, base_name) && blocks.property(base, "extended") == Some("true") || is(base, "minecraft:moving_piston")) && same_facing
            }
            Rule::Diode => blocks.is_face_sturdy(get(0, -1, 0), Direction::Up, SupportType::Rigid),
            Rule::RedstoneWire => {
                let below = get(0, -1, 0);
                sturdy(below, Direction::Up) || is(below, "minecraft:hopper")
            }
            Rule::Cocoa => {
                let facing = blocks.property(state, "facing").and_then(parse_direction).unwrap_or(Direction::North);
                let (dx, dy, dz) = facing.offset();
                tagged(get(dx, dy, dz), "minecraft:supports_cocoa")
            }
            Rule::AmethystCluster => {
                let facing = blocks.property(state, "facing").and_then(parse_direction).unwrap_or(Direction::Up);
                let (dx, dy, dz) = facing.opposite().offset();
                sturdy(get(dx, dy, dz), facing)
            }
            Rule::HangingMoss => {
                let above = get(0, 1, 0);
                sturdy(above, Direction::Down) || collision_full(above, Direction::Down) || blocks.block_of(above) == blocks.block_of(state)
            }
            Rule::Vine => {
                // VineBlock.getUpdatedState: the UP face tests the block above
                // with Direction.DOWN, so its UP face (a vanilla quirk).
                let attach = |neighbor: BlockStateId, face: Direction| sturdy(neighbor, face) || collision_full(neighbor, face);
                let above = get(0, 1, 0);
                if blocks.property(state, "up") == Some("true") && attach(above, Direction::Up) {
                    return true;
                }
                for direction in [Direction::North, Direction::East, Direction::South, Direction::West] {
                    let face = direction_name(direction);
                    if blocks.property(state, face) != Some("true") {
                        continue;
                    }
                    let (dx, dy, dz) = direction.offset();
                    if attach(get(dx, dy, dz), direction.opposite())
                        || (blocks.block_of(above) == blocks.block_of(state) && blocks.property(above, face) == Some("true"))
                    {
                        return true;
                    }
                }
                false
            }
        }
    }
}

/// Whether a shape has any area on a face (`VoxelShape.getFaceShape(...).isEmpty()`).
fn face_nonempty(shape: &FaceShape, direction: Direction) -> bool {
    match shape {
        FaceShape::Full => true,
        FaceShape::Empty => false,
        FaceShape::Boxes(boxes) => {
            let (axis, positive) = match direction {
                Direction::Down => (1, false),
                Direction::Up => (1, true),
                Direction::North => (2, false),
                Direction::South => (2, true),
                Direction::West => (0, false),
                Direction::East => (0, true),
            };
            boxes.iter().any(|b| if positive { b[axis + 3] >= 1.0 } else { b[axis] <= 0.0 })
        }
    }
}

pub fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Down => "down",
        Direction::Up => "up",
        Direction::North => "north",
        Direction::South => "south",
        Direction::West => "west",
        Direction::East => "east",
    }
}

pub fn parse_direction(name: &str) -> Option<Direction> {
    Some(match name {
        "down" => Direction::Down,
        "up" => Direction::Up,
        "north" => Direction::North,
        "south" => Direction::South,
        "west" => Direction::West,
        "east" => Direction::East,
        _ => return None,
    })
}
