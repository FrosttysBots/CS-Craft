//! Cave features: speleothems (pointed dripstone and sulfur spikes),
//! speleothem clusters, large dripstone, vegetation patches and root systems.

use super::ore::column_scan;
use super::{float, float_or, int, int_or, Placeable};
use crate::feature::blocks::{parse_state, BlockSet, FluidType};
use crate::feature::java_set::JavaHashSet;
use crate::feature::predicate::BlockPredicate;
use crate::feature::state::StateProvider;
use crate::feature::{place_placed, Ctx, Library, PlacedId};
use crate::providers::{FloatProvider, IntProvider};
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::tags::TagId;
use minecraftoss_core::{BlockId, BlockPos, BlockStateId, SupportType};
use serde_json::Value;
use std::sync::Arc;

pub fn parse(lib: &mut Library, kind: &str, json: &Value) -> Option<Result<Box<dyn Placeable>, String>> {
    Some(match kind {
        "speleothem" => Speleothem::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        "speleothem_cluster" => Cluster::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        "large_dripstone" => LargeDripstone::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        "vegetation_patch" => VegetationPatch::parse(lib, json, false).map(|f| Box::new(f) as Box<dyn Placeable>),
        "waterlogged_vegetation_patch" => VegetationPatch::parse(lib, json, true).map(|f| Box::new(f) as Box<dyn Placeable>),
        "root_system" => RootSystem::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        _ => return None,
    })
}

fn is_empty_or_water(ctx: &Ctx, s: BlockStateId) -> bool {
    ctx.is_air(s) || s == ctx.lib.blocks.water
}

fn is_empty_or_water_or_lava(ctx: &Ctx, s: BlockStateId) -> bool {
    ctx.is_air(s) || s == ctx.lib.blocks.water || s == ctx.lib.blocks.lava
}

/// The blocks a speleothem grows from and the pointed block it grows.
#[derive(Debug)]
struct Blocks {
    base: BlockId,
    base_state: BlockStateId,
    pointed: BlockId,
    pointed_state: BlockStateId,
    replaceable: BlockSet,
}

impl Blocks {
    fn parse(lib: &Library, json: &Value) -> Result<Self, String> {
        let base_state = parse_state(&lib.registries, &json["base_block"])?;
        let pointed_state = parse_state(&lib.registries, &json["pointed_block"])?;
        let blocks = &lib.registries.blocks;
        Ok(Self {
            base: blocks.block_of(base_state),
            base_state: blocks.block(blocks.block_of(base_state)).default_state(),
            pointed: blocks.block_of(pointed_state),
            pointed_state: blocks.block(blocks.block_of(pointed_state)).default_state(),
            replaceable: BlockSet::parse(&lib.registries, &json["replaceable_blocks"])?,
        })
    }

    fn is_base(&self, ctx: &Ctx, s: BlockStateId) -> bool {
        ctx.registries().blocks.block_of(s) == self.base || self.replaceable.contains(ctx.registries(), s)
    }

    fn place_base(&self, ctx: &mut Ctx, pos: BlockPos) -> bool {
        if self.replaceable.contains(ctx.registries(), ctx.block(pos)) {
            ctx.set_block(pos, self.base_state);
            true
        } else {
            false
        }
    }

    fn pointed(&self, ctx: &Ctx, direction: Direction, thickness: &str) -> BlockStateId {
        ctx.with(ctx.with(self.pointed_state, "vertical_direction", direction.name()), "thickness", thickness)
    }

    /// `SpeleothemUtils.growSpeleothem`.
    fn grow(&self, ctx: &mut Ctx, start: BlockPos, tip: Direction, height: i32, merged: bool) {
        if !self.is_base(ctx, ctx.block(start.relative(tip.opposite(), 1))) {
            return;
        }
        let mut states = Vec::new();
        if height >= 3 {
            states.push("base");
            for _ in 0..height - 3 {
                states.push("middle");
            }
        }
        if height >= 2 {
            states.push("frustum");
        }
        if height >= 1 {
            states.push(if merged { "tip_merge" } else { "tip" });
        }
        let mut pos = start;
        for thickness in states {
            let mut state = self.pointed(ctx, tip, thickness);
            let water = matches!(ctx.fluid_at(pos), FluidType::Water | FluidType::FlowingWater);
            state = ctx.with(state, "waterlogged", if water { "true" } else { "false" });
            ctx.set_block(pos, state);
            pos = pos.relative(tip, 1);
        }
    }
}

#[derive(Debug)]
struct Speleothem {
    blocks: Blocks,
    taller: f32,
    directional: f32,
    radius2: f32,
    radius3: f32,
}

impl Speleothem {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            blocks: Blocks::parse(lib, json)?,
            taller: float_or(json, "chance_of_taller_generation", 0.2),
            directional: float_or(json, "chance_of_directional_spread", 0.7),
            radius2: float_or(json, "chance_of_spread_radius2", 0.5),
            radius3: float_or(json, "chance_of_spread_radius3", 0.5),
        })
    }
}

impl Placeable for Speleothem {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let above = self.blocks.is_base(ctx, ctx.block(origin.above()));
        let below = self.blocks.is_base(ctx, ctx.block(origin.below()));
        let tip = if above && below {
            if random.next_bool() { Direction::Down } else { Direction::Up }
        } else if above {
            Direction::Down
        } else if below {
            Direction::Up
        } else {
            return false;
        };
        let root = origin.relative(tip.opposite(), 1);
        self.blocks.place_base(ctx, root);
        for direction in Direction::HORIZONTAL {
            if random.next_f32() > self.directional {
                continue;
            }
            let p1 = root.relative(direction, 1);
            self.blocks.place_base(ctx, p1);
            if random.next_f32() > self.radius2 {
                continue;
            }
            let p2 = p1.relative(Direction::ALL[random.next_i32_bound(6) as usize], 1);
            self.blocks.place_base(ctx, p2);
            if random.next_f32() > self.radius3 {
                continue;
            }
            let p3 = p2.relative(Direction::ALL[random.next_i32_bound(6) as usize], 1);
            self.blocks.place_base(ctx, p3);
        }
        let height = if random.next_f32() < self.taller && is_empty_or_water(ctx, ctx.block(origin.relative(tip, 1))) { 2 } else { 1 };
        self.blocks.grow(ctx, origin, tip, height, false);
        true
    }
}

#[derive(Debug)]
struct Cluster {
    blocks: Blocks,
    search_range: i32,
    height: IntProvider,
    radius: IntProvider,
    max_height_diff: i32,
    height_deviation: i32,
    layer_thickness: IntProvider,
    density: FloatProvider,
    wetness: FloatProvider,
    chance_at_max_distance: f32,
    max_distance_from_edge: i32,
    max_distance_from_center: i32,
    base_stone: TagId,
}

impl Cluster {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            blocks: Blocks::parse(lib, json)?,
            search_range: int(json, "floor_to_ceiling_search_range")?,
            height: IntProvider::parse(&json["height"])?,
            radius: IntProvider::parse(&json["radius"])?,
            max_height_diff: int(json, "max_stalagmite_stalactite_height_diff")?,
            height_deviation: int(json, "height_deviation")?,
            layer_thickness: IntProvider::parse(&json["speleothem_block_layer_thickness"])?,
            density: FloatProvider::parse(&json["density"])?,
            wetness: FloatProvider::parse(&json["wetness"])?,
            chance_at_max_distance: float(json, "chance_of_speleothem_at_max_distance_from_center")?,
            max_distance_from_edge: int(json, "max_distance_from_edge_affecting_chance_of_speleothem")?,
            max_distance_from_center: int(json, "max_distance_from_center_affecting_height_bias")?,
            base_stone: lib.registries.block_tags.require("minecraft:base_stone_overworld")?,
        })
    }

    fn speleothem_height(&self, random: &mut WorldgenRandom, dx: i32, dz: i32, density: f32, max_height: i32) -> i32 {
        if random.next_f32() > density {
            return 0;
        }
        let distance = dx.abs() + dz.abs();
        let mean = crate::mth::clamped_map(f64::from(distance), 0.0, f64::from(self.max_distance_from_center), f64::from(max_height) / 2.0, 0.0) as f32;
        let normal = mean + random.next_gaussian() as f32 * self.height_deviation as f32;
        crate::mth::clamp(normal, 0.0, max_height as f32) as i32
    }

    fn can_place_pool(&self, ctx: &Ctx, pos: BlockPos) -> bool {
        let state = ctx.block(pos);
        let blocks = &ctx.registries().blocks;
        if state == ctx.lib.blocks.water || blocks.block_of(state) == self.blocks.base || blocks.block_of(state) == self.blocks.pointed {
            return false;
        }
        if matches!(ctx.fluid_at(pos.above()), FluidType::Water | FluidType::FlowingWater) {
            return false;
        }
        let adjacent = |p: BlockPos| {
            let s = ctx.block(p);
            ctx.in_tag(s, self.base_stone) || matches!(ctx.fluid(s), FluidType::Water | FluidType::FlowingWater)
        };
        Direction::HORIZONTAL.iter().all(|&d| adjacent(pos.relative(d, 1))) && adjacent(pos.below())
    }

    fn replace_with_base(&self, ctx: &mut Ctx, first: BlockPos, count: i32, direction: Direction) {
        let mut pos = first;
        for _ in 0..count {
            if !self.blocks.place_base(ctx, pos) {
                return;
            }
            pos = pos.relative(direction, 1);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn column(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, pos: BlockPos, dx: i32, dz: i32, wetness: f32, chance: f64, cluster_height: i32, density: f32) {
        let inside = |s: BlockStateId| ctx.is_air(s) || s == ctx.lib.blocks.water;
        let edge = |s: BlockStateId| !ctx.is_air(s) && s != ctx.lib.blocks.water;
        let Some((base_floor, ceiling)) = column_scan(ctx, pos, self.search_range, &inside, &edge) else {
            return;
        };
        if ceiling.is_none() && base_floor.is_none() {
            return;
        }
        let want_pool = random.next_f32() < wetness;
        let floor = match base_floor {
            Some(f) if want_pool && self.can_place_pool(ctx, pos.at_y(f)) => {
                ctx.set_block(pos.at_y(f), ctx.lib.blocks.water);
                Some(f - 1)
            }
            other => other,
        };
        let lava = |ctx: &Ctx, y: i32| ctx.block(pos.at_y(y)) == ctx.lib.blocks.lava;
        let want_stalactite = random.next_f64() < chance;
        let stalactite = match ceiling {
            Some(c) if want_stalactite && !lava(ctx, c) => {
                let thickness = self.layer_thickness.sample(random);
                self.replace_with_base(ctx, pos.at_y(c), thickness, Direction::Up);
                let max = match floor {
                    Some(f) => cluster_height.min(c - f),
                    None => cluster_height,
                };
                self.speleothem_height(random, dx, dz, density, max)
            }
            _ => 0,
        };
        let want_stalagmite = random.next_f64() < chance;
        let stalagmite = match floor {
            Some(f) if want_stalagmite && !lava(ctx, f) => {
                let thickness = self.layer_thickness.sample(random);
                self.replace_with_base(ctx, pos.at_y(f), thickness, Direction::Down);
                if ceiling.is_some() {
                    0.max(stalactite + crate::providers::random_between_inclusive(random, -self.max_height_diff, self.max_height_diff))
                } else {
                    self.speleothem_height(random, dx, dz, density, cluster_height)
                }
            }
            _ => 0,
        };
        let (actual_stalactite, actual_stalagmite) = match (ceiling, floor) {
            (Some(c), Some(f)) if c - stalactite <= f + stalagmite => {
                let lowest_bottom = (c - stalactite).max(f + 1);
                let highest_top = (f + stalagmite).min(c - 1);
                let bottom = crate::providers::random_between_inclusive(random, lowest_bottom, highest_top + 1);
                (c - bottom, bottom - 1 - f)
            }
            _ => (stalactite, stalagmite),
        };
        let column_height = match (ceiling, floor) {
            (Some(c), Some(f)) => Some(c - f - 1),
            _ => None,
        };
        let merge = random.next_bool() && actual_stalactite > 0 && actual_stalagmite > 0 && column_height == Some(actual_stalactite + actual_stalagmite);
        if let Some(c) = ceiling {
            self.blocks.grow(ctx, pos.at_y(c - 1), Direction::Down, actual_stalactite, merge);
        }
        if let Some(f) = floor {
            self.blocks.grow(ctx, pos.at_y(f + 1), Direction::Up, actual_stalagmite, merge);
        }
    }
}

impl Placeable for Cluster {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !is_empty_or_water(ctx, ctx.block(origin)) {
            return false;
        }
        let height = self.height.sample(random);
        let wetness = self.wetness.sample(random);
        let density = self.density.sample(random);
        let xr = self.radius.sample(random);
        let zr = self.radius.sample(random);
        for dx in -xr..=xr {
            for dz in -zr..=zr {
                let from_edge = (xr - dx.abs()).min(zr - dz.abs());
                // Mth.clampedMap(float, ...) in float precision.
                let t = (from_edge as f32 - 0.0) / (self.max_distance_from_edge as f32 - 0.0);
                let chance = if t < 0.0 {
                    self.chance_at_max_distance
                } else if t > 1.0 {
                    1.0
                } else {
                    self.chance_at_max_distance + t * (1.0 - self.chance_at_max_distance)
                };
                self.column(ctx, random, origin.offset(dx, 0, dz), dx, dz, wetness, f64::from(chance), height, density);
            }
        }
        true
    }
}

/// `SpeleothemUtils.getSpeleothemHeight`.
fn speleothem_height(mut distance: f64, radius: f64, scale: f64, bluntness: f64) -> f64 {
    if distance < bluntness {
        distance = bluntness;
    }
    let r = distance / radius * 0.384;
    let part1 = 0.75 * r.powf(1.333_333_333_333_333_3);
    let part2 = r.powf(0.666_666_666_666_666_6);
    let part3 = 0.333_333_333_333_333_3 * r.ln();
    let height = (scale * (part1 - part2 - part3)).max(0.0);
    height / 0.384 * radius
}

#[derive(Debug)]
struct LargeDripstone {
    replaceable: BlockSet,
    search_range: i32,
    column_radius: IntProvider,
    height_scale: FloatProvider,
    max_ratio: f32,
    stalactite_bluntness: FloatProvider,
    stalagmite_bluntness: FloatProvider,
    wind_speed: FloatProvider,
    min_radius_for_wind: i32,
    min_bluntness_for_wind: f32,
    dripstone: BlockStateId,
    base_stone: TagId,
}

struct Dripstone {
    root: BlockPos,
    up: bool,
    radius: i32,
    bluntness: f64,
    scale: f64,
}

struct Wind {
    origin_y: i32,
    speed: Option<(f64, f64)>,
    max_offset: i32,
}

impl Wind {
    fn offset(&self, pos: BlockPos) -> BlockPos {
        let Some((sx, sz)) = self.speed else { return pos };
        let dy = f64::from(self.origin_y - pos.y);
        let dx = ((sx * dy).floor() as i32).clamp(-self.max_offset, self.max_offset);
        let dz = ((sz * dy).floor() as i32).clamp(-self.max_offset, self.max_offset);
        pos.offset(dx, 0, dz)
    }
}

impl Dripstone {
    fn height_at(&self, radius: f32) -> i32 {
        speleothem_height(f64::from(radius), f64::from(self.radius), self.scale, self.bluntness) as i32
    }

    fn embedded(ctx: &Ctx, center: BlockPos, radius: i32) -> bool {
        if is_empty_or_water_or_lava(ctx, ctx.block(center)) {
            return false;
        }
        let increment = 6.0f32 / radius as f32;
        let mut angle = 0.0f32;
        while angle < std::f32::consts::TAU {
            let dx = (crate::providers::cos(f64::from(angle)) * radius as f32) as i32;
            let dz = (crate::providers::sin(f64::from(angle)) * radius as f32) as i32;
            if is_empty_or_water_or_lava(ctx, ctx.block(center.offset(dx, 0, dz))) {
                return false;
            }
            angle += increment;
        }
        true
    }

    fn settle(&mut self, ctx: &Ctx, wind: &Wind) -> bool {
        while self.radius > 1 {
            let mut root = self.root;
            let tries = 10.min(self.height_at(0.0));
            for _ in 0..tries {
                if ctx.block(root) == ctx.lib.blocks.lava {
                    return false;
                }
                if Self::embedded(ctx, wind.offset(root), self.radius) {
                    self.root = root;
                    return true;
                }
                root = root.relative(if self.up { Direction::Down } else { Direction::Up }, 1);
            }
            self.radius /= 2;
        }
        false
    }

    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, wind: &Wind, dripstone: BlockStateId, base_stone: TagId) {
        for dx in -self.radius..=self.radius {
            for dz in -self.radius..=self.radius {
                let current = ((dx * dx + dz * dz) as f32).sqrt();
                if current > self.radius as f32 {
                    continue;
                }
                let mut height = self.height_at(current);
                if height <= 0 {
                    continue;
                }
                if f64::from(random.next_f32()) < 0.2 {
                    height = (height as f32 * crate::providers::random_between(random, 0.8, 1.0)) as i32;
                }
                let mut pos = self.root.offset(dx, 0, dz);
                let mut out_of_stone = false;
                let max_y = if self.up { ctx.height(HeightmapKind::WorldSurfaceWg, pos.x, pos.z) } else { i32::MAX };
                let mut i = 0;
                while i < height && pos.y < max_y {
                    let adjusted = wind.offset(pos);
                    let state = ctx.block(adjusted);
                    if is_empty_or_water_or_lava(ctx, state) {
                        out_of_stone = true;
                        ctx.set_block(adjusted, dripstone);
                    } else if out_of_stone && ctx.in_tag(state, base_stone) {
                        break;
                    }
                    pos = pos.relative(if self.up { Direction::Up } else { Direction::Down }, 1);
                    i += 1;
                }
            }
        }
    }

    fn suits_wind(&self, min_radius: i32, min_bluntness: f32) -> bool {
        self.radius >= min_radius && self.bluntness >= f64::from(min_bluntness)
    }
}

impl LargeDripstone {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            replaceable: BlockSet::parse(&lib.registries, &json["replaceable_blocks"])?,
            search_range: int_or(json, "floor_to_ceiling_search_range", 30),
            column_radius: IntProvider::parse(&json["column_radius"])?,
            height_scale: FloatProvider::parse(&json["height_scale"])?,
            max_ratio: float(json, "max_column_radius_to_cave_height_ratio")?,
            stalactite_bluntness: FloatProvider::parse(&json["stalactite_bluntness"])?,
            stalagmite_bluntness: FloatProvider::parse(&json["stalagmite_bluntness"])?,
            wind_speed: FloatProvider::parse(&json["wind_speed"])?,
            min_radius_for_wind: int(json, "min_radius_for_wind")?,
            min_bluntness_for_wind: float(json, "min_bluntness_for_wind")?,
            dripstone: lib.registries.blocks.parse_state("minecraft:dripstone_block")?,
            base_stone: lib.registries.block_tags.require("minecraft:base_stone_overworld")?,
        })
    }
}

impl Placeable for LargeDripstone {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !is_empty_or_water(ctx, ctx.block(origin)) {
            return false;
        }
        let dripstone_block = ctx.registries().blocks.block_of(self.dripstone);
        let inside = |s: BlockStateId| ctx.is_air(s) || s == ctx.lib.blocks.water;
        let edge = |s: BlockStateId| {
            ctx.registries().blocks.block_of(s) == dripstone_block || self.replaceable.contains(ctx.registries(), s) || s == ctx.lib.blocks.lava
        };
        let Some((Some(floor), Some(ceiling))) = column_scan(ctx, origin, self.search_range, &inside, &edge) else {
            return false;
        };
        let column_height = ceiling - floor - 1;
        if column_height < 4 {
            return false;
        }
        let by_height = (column_height as f32 * self.max_ratio) as i32;
        let max_radius = by_height.clamp(self.column_radius.min_inclusive(), self.column_radius.max_inclusive());
        let radius = crate::providers::random_between_inclusive(random, self.column_radius.min_inclusive(), max_radius);
        let make = |random: &mut WorldgenRandom, root: BlockPos, up: bool, bluntness: &FloatProvider| {
            let b = f64::from(bluntness.sample(random));
            let s = f64::from(self.height_scale.sample(random));
            Dripstone { root, up, radius, bluntness: b, scale: s }
        };
        let mut stalactite = make(random, origin.at_y(ceiling - 1), false, &self.stalactite_bluntness);
        let mut stalagmite = make(random, origin.at_y(floor + 1), true, &self.stalagmite_bluntness);
        let wind = if stalactite.suits_wind(self.min_radius_for_wind, self.min_bluntness_for_wind) && stalagmite.suits_wind(self.min_radius_for_wind, self.min_bluntness_for_wind) {
            let speed = self.wind_speed.sample(random);
            let direction = crate::providers::random_between(random, 0.0, std::f32::consts::PI);
            let sx = f64::from(crate::providers::cos(f64::from(direction)) * speed);
            let sz = f64::from(crate::providers::sin(f64::from(direction)) * speed);
            Wind { origin_y: origin.y, speed: Some((sx, sz)), max_offset: 16 - radius }
        } else {
            Wind { origin_y: 0, speed: None, max_offset: 0 }
        };
        let top_ok = stalactite.settle(ctx, &wind);
        let bottom_ok = stalagmite.settle(ctx, &wind);
        if top_ok {
            stalactite.place(ctx, random, &wind, self.dripstone, self.base_stone);
        }
        if bottom_ok {
            stalagmite.place(ctx, random, &wind, self.dripstone, self.base_stone);
        }
        true
    }
}

#[derive(Debug)]
struct VegetationPatch {
    replaceable: BlockSet,
    ground: Arc<StateProvider>,
    vegetation: PlacedId,
    /// `CaveSurface`: FLOOR grows into the block below, CEILING above.
    inwards: Direction,
    depth: IntProvider,
    extra_bottom: f32,
    vertical_range: i32,
    vegetation_chance: f32,
    xz_radius: IntProvider,
    extra_edge: f32,
    waterlogged: bool,
}

impl VegetationPatch {
    fn parse(lib: &mut Library, json: &Value, waterlogged: bool) -> Result<Self, String> {
        Ok(Self {
            replaceable: BlockSet::parse(&lib.registries, &json["replaceable"])?,
            ground: StateProvider::parse(lib, &json["ground_state"])?,
            vegetation: lib.placed_ref(&json["vegetation_feature"])?,
            inwards: match json["surface"].as_str() {
                Some("ceiling") => Direction::Up,
                _ => Direction::Down,
            },
            depth: IntProvider::parse(&json["depth"])?,
            extra_bottom: float(json, "extra_bottom_block_chance")?,
            vertical_range: int(json, "vertical_range")?,
            vegetation_chance: float(json, "vegetation_chance")?,
            xz_radius: IntProvider::parse(&json["xz_radius"])?,
            extra_edge: float(json, "extra_edge_column_chance")?,
            waterlogged,
        })
    }

    fn place_ground(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, mut below: BlockPos, depth: i32) -> bool {
        for i in 0..depth {
            let state = self.ground.get(ctx, random, below);
            let current = ctx.block(below);
            if ctx.registries().blocks.block_of(state) != ctx.registries().blocks.block_of(current) {
                if !self.replaceable.contains(ctx.registries(), current) {
                    return i != 0;
                }
                ctx.set_block(below, state);
                below = below.relative(self.inwards, 1);
            }
        }
        true
    }

    fn ground_patch(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos, xr: i32, zr: i32) -> JavaHashSet {
        let outwards = self.inwards.opposite();
        let mut surface = JavaHashSet::new();
        for dx in -xr..=xr {
            let x_edge = dx == -xr || dx == xr;
            for dz in -zr..=zr {
                let z_edge = dz == -zr || dz == zr;
                let edge = x_edge || z_edge;
                let corner = x_edge && z_edge;
                let edge_not_corner = edge && !corner;
                if corner || edge_not_corner && (self.extra_edge == 0.0 || random.next_f32() > self.extra_edge) {
                    continue;
                }
                let mut pos = origin.offset(dx, 0, dz);
                let mut offset = 0;
                while ctx.is_empty_block(pos) && offset < self.vertical_range {
                    pos = pos.relative(self.inwards, 1);
                    offset += 1;
                }
                let mut offset = 0;
                while !ctx.is_empty_block(pos) && offset < self.vertical_range {
                    pos = pos.relative(outwards, 1);
                    offset += 1;
                }
                let below = pos.relative(self.inwards, 1);
                let below_state = ctx.block(below);
                if ctx.is_empty_block(pos) && ctx.registries().blocks.is_face_sturdy(below_state, outwards, SupportType::Full) {
                    let extra = i32::from(self.extra_bottom > 0.0 && random.next_f32() < self.extra_bottom);
                    let depth = self.depth.sample(random) + extra;
                    if self.place_ground(ctx, random, below, depth) {
                        surface.insert(below);
                    }
                }
            }
        }
        if !self.waterlogged {
            return surface;
        }
        let mut water = JavaHashSet::new();
        for pos in surface.iter() {
            let exposed = [Direction::North, Direction::East, Direction::South, Direction::West, Direction::Down]
                .iter()
                .any(|&d| !ctx.registries().blocks.is_face_sturdy(ctx.block(pos.relative(d, 1)), d.opposite(), SupportType::Full));
            if !exposed {
                water.insert(pos);
            }
        }
        for pos in water.iter() {
            ctx.set_block(pos, ctx.lib.blocks.water);
        }
        water
    }
}

impl Placeable for VegetationPatch {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let xr = self.xz_radius.sample(random) + 1;
        let zr = self.xz_radius.sample(random) + 1;
        let surface = self.ground_patch(ctx, random, origin, xr, zr);
        for pos in surface.iter() {
            if self.vegetation_chance > 0.0 && random.next_f32() < self.vegetation_chance {
                let target = if self.waterlogged { pos.below() } else { pos };
                let placed = place_placed(ctx, random, self.vegetation, target.relative(self.inwards.opposite(), 1));
                if self.waterlogged && placed {
                    let state = ctx.block(pos);
                    if ctx.property(state, "waterlogged") == Some("false") {
                        let wet = ctx.with(state, "waterlogged", "true");
                        ctx.set_block(pos, wet);
                    }
                }
            }
        }
        !surface.is_empty()
    }
}

#[derive(Debug)]
struct RootSystem {
    tree: PlacedId,
    vertical_space: i32,
    level_test_distance: i32,
    max_level_deviation: i32,
    root_radius: i32,
    root_replaceable: BlockSet,
    root_state: Arc<StateProvider>,
    root_attempts: i32,
    root_column_max_height: i32,
    hanging_radius: i32,
    hanging_span: i32,
    hanging_state: Arc<StateProvider>,
    hanging_attempts: i32,
    allowed_water: i32,
    allowed_position: BlockPredicate,
}

impl RootSystem {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            tree: lib.placed_ref(&json["feature"])?,
            vertical_space: int(json, "required_vertical_space_for_tree")?,
            level_test_distance: int(json, "level_test_distance")?,
            max_level_deviation: int(json, "max_level_deviation")?,
            root_radius: int(json, "root_radius")?,
            root_replaceable: BlockSet::parse(&lib.registries, &json["root_replaceable"])?,
            root_state: StateProvider::parse(lib, &json["root_state_provider"])?,
            root_attempts: int(json, "root_placement_attempts")?,
            root_column_max_height: int(json, "root_column_max_height")?,
            hanging_radius: int(json, "hanging_root_radius")?,
            hanging_span: int(json, "hanging_roots_vertical_span")?,
            hanging_state: StateProvider::parse(lib, &json["hanging_root_state_provider"])?,
            hanging_attempts: int(json, "hanging_root_placement_attempts")?,
            allowed_water: int(json, "allowed_vertical_water_for_tree")?,
            allowed_position: BlockPredicate::parse(&lib.registries, &json["allowed_tree_position"])?,
        })
    }

    fn space_for_tree(&self, ctx: &Ctx, pos: BlockPos) -> bool {
        for i in 1..=self.vertical_space {
            let state = ctx.block(pos.offset(0, i, 0));
            let allowed = ctx.is_air(state) || i + 1 <= self.allowed_water && matches!(ctx.fluid(state), FluidType::Water | FluidType::FlowingWater);
            if !allowed {
                return false;
            }
        }
        if self.level_test_distance > 0 {
            for d in Direction::BY_2D {
                let corner = pos.relative(d, self.level_test_distance);
                let below = ctx.block(corner.offset(0, -self.max_level_deviation, 0));
                let above = ctx.block(corner.offset(0, self.max_level_deviation, 0));
                if ctx.is_air(below) || !ctx.is_air(above) {
                    return false;
                }
            }
        }
        true
    }
}

impl Placeable for RootSystem {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !ctx.is_empty_block(origin) {
            return false;
        }
        let mut working = origin;
        let mut placed_tree = false;
        for y in 0..self.root_column_max_height {
            working = working.above();
            if ctx.height(HeightmapKind::WorldSurface, working.x, working.z) < working.y {
                break;
            }
            if self.allowed_position.test(ctx, working) && self.space_for_tree(ctx, working) {
                let below = working.below();
                if matches!(ctx.fluid_at(below), FluidType::Lava | FluidType::FlowingLava) || !ctx.is_solid(ctx.block(below)) {
                    break;
                }
                if place_placed(ctx, random, self.tree, working) {
                    // placeDirt
                    for dy in origin.y..origin.y + y {
                        for _ in 0..self.root_attempts {
                            let dx = random.next_i32_bound(self.root_radius) - random.next_i32_bound(self.root_radius);
                            let dz = random.next_i32_bound(self.root_radius) - random.next_i32_bound(self.root_radius);
                            let p = BlockPos::new(origin.x + dx, dy, origin.z + dz);
                            if self.root_replaceable.contains(ctx.registries(), ctx.block(p)) {
                                let state = self.root_state.get(ctx, random, p);
                                ctx.set_block(p, state);
                            }
                        }
                    }
                    placed_tree = true;
                    break;
                }
            }
        }
        if placed_tree {
            for _ in 0..self.hanging_attempts {
                let dx = random.next_i32_bound(self.hanging_radius) - random.next_i32_bound(self.hanging_radius);
                let dy = random.next_i32_bound(self.hanging_span) - random.next_i32_bound(self.hanging_span);
                let dz = random.next_i32_bound(self.hanging_radius) - random.next_i32_bound(self.hanging_radius);
                let p = origin.offset(dx, dy, dz);
                if ctx.is_empty_block(p) {
                    let state = self.hanging_state.get(ctx, random, p);
                    if ctx.can_survive(state, p) && ctx.registries().blocks.is_face_sturdy(ctx.block(p.above()), Direction::Down, SupportType::Full) {
                        ctx.set_block(p, state);
                    }
                }
            }
        }
        true
    }
}
