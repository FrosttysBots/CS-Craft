//! Terrain-shaping features: monster rooms, springs, disks, lakes, geodes,
//! icebergs, ice spikes and block blobs.

use super::{bool_or, int, int_or, Placeable};
use crate::feature::blocks::{parse_state, BlockSet, FluidType};
use crate::feature::predicate::BlockPredicate;
use crate::feature::state::StateProvider;
use crate::feature::{Ctx, Library};
use crate::providers::IntProvider;
use minecraftoss_core::block::flags;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::{BlockPos, BlockStateId};
use serde_json::Value;
use std::sync::Arc;

pub fn parse(lib: &mut Library, kind: &str, json: &Value) -> Option<Result<Box<dyn Placeable>, String>> {
    Some(match kind {
        "monster_room" => MonsterRoom::parse(lib).map(|f| Box::new(f) as Box<dyn Placeable>),
        "spring_feature" => Spring::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        "disk" => Disk::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        "lake" => Lake::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        "geode" => Geode::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        "iceberg" => Iceberg::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        _ => return None,
    })
}

#[derive(Debug)]
struct MonsterRoom {
    cannot_replace: minecraftoss_core::tags::TagId,
    cobblestone: BlockStateId,
    mossy: BlockStateId,
    chest: BlockStateId,
    spawner: BlockStateId,
}

impl MonsterRoom {
    fn parse(lib: &mut Library) -> Result<Self, String> {
        let s = |n: &str| lib.registries.blocks.parse_state(n);
        Ok(Self {
            cannot_replace: lib.registries.block_tags.require("minecraft:features_cannot_replace")?,
            cobblestone: s("minecraft:cobblestone")?,
            mossy: s("minecraft:mossy_cobblestone")?,
            chest: s("minecraft:chest")?,
            spawner: s("minecraft:spawner")?,
        })
    }

    fn safe_set(&self, ctx: &mut Ctx, pos: BlockPos, state: BlockStateId) {
        let tag = self.cannot_replace;
        ctx.safe_set_block(pos, state, |ctx, s| !ctx.in_tag(s, tag));
    }

    /// `StructurePiece.reorient` for a chest.
    fn reorient(&self, ctx: &Ctx, pos: BlockPos, state: BlockStateId) -> BlockStateId {
        let blocks = &ctx.registries().blocks;
        let solid_render = |p: BlockPos| blocks.is(ctx.block(p), flags::SOLID_RENDER);
        let mut solid = None;
        for d in Direction::HORIZONTAL {
            let s = ctx.block(pos.relative(d, 1));
            if ctx.is(s, "minecraft:chest") {
                return state;
            }
            if blocks.is(s, flags::SOLID_RENDER) {
                if solid.is_some() {
                    solid = None;
                    break;
                }
                solid = Some(d);
            }
        }
        if let Some(d) = solid {
            return ctx.with(state, "facing", d.opposite().name());
        }
        let mut lock = ctx.property(state, "facing").and_then(Direction::from_name).unwrap_or(Direction::North);
        if solid_render(pos.relative(lock, 1)) {
            lock = lock.opposite();
        }
        if solid_render(pos.relative(lock, 1)) {
            lock = lock.clockwise();
        }
        if solid_render(pos.relative(lock, 1)) {
            lock = lock.opposite();
        }
        ctx.with(state, "facing", lock.name())
    }
}

impl Placeable for MonsterRoom {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let air = ctx.lib.blocks.cave_air;
        let xr = random.next_i32_bound(2) + 2;
        let (min_x, max_x) = (-xr - 1, xr + 1);
        let zr = random.next_i32_bound(2) + 2;
        let (min_z, max_z) = (-zr - 1, zr + 1);
        let mut holes = 0;
        for dx in min_x..=max_x {
            for dy in -1..=4 {
                for dz in min_z..=max_z {
                    let pos = origin.offset(dx, dy, dz);
                    let solid = ctx.is_solid(ctx.block(pos));
                    if (dy == -1 || dy == 4) && !solid {
                        return false;
                    }
                    if (dx == min_x || dx == max_x || dz == min_z || dz == max_z) && dy == 0 && ctx.is_empty_block(pos) && ctx.is_empty_block(pos.above()) {
                        holes += 1;
                    }
                }
            }
        }
        if !(1..=5).contains(&holes) {
            return false;
        }
        for dx in min_x..=max_x {
            for dy in (-1..=3).rev() {
                for dz in min_z..=max_z {
                    let pos = origin.offset(dx, dy, dz);
                    let state = ctx.block(pos);
                    if dx == min_x || dy == -1 || dz == min_z || dx == max_x || dy == 4 || dz == max_z {
                        if pos.y >= ctx.min_y() && !ctx.is_solid(ctx.block(pos.below())) {
                            ctx.set_block(pos, air);
                        } else if ctx.is_solid(state) && !ctx.is(state, "minecraft:chest") {
                            if dy == -1 && random.next_i32_bound(4) != 0 {
                                self.safe_set(ctx, pos, self.mossy);
                            } else {
                                self.safe_set(ctx, pos, self.cobblestone);
                            }
                        }
                    } else if !ctx.is(state, "minecraft:chest") && !ctx.is(state, "minecraft:spawner") {
                        self.safe_set(ctx, pos, air);
                    }
                }
            }
        }
        for _ in 0..2 {
            for _ in 0..3 {
                let xc = origin.x + random.next_i32_bound(xr * 2 + 1) - xr;
                let zc = origin.z + random.next_i32_bound(zr * 2 + 1) - zr;
                let pos = BlockPos::new(xc, origin.y, zc);
                if !ctx.is_empty_block(pos) {
                    continue;
                }
                let walls = Direction::HORIZONTAL.iter().filter(|&&d| ctx.is_solid(ctx.block(pos.relative(d, 1)))).count();
                if walls == 1 {
                    let chest = self.reorient(ctx, pos, self.chest);
                    self.safe_set(ctx, pos, chest);
                    // RandomizableContainer.setBlockEntityLootTable: a loot seed.
                    if ctx.is(ctx.block(pos), "minecraft:chest") {
                        let seed = random.next_i64();
                        ctx.region.set_loot_table(pos.x, pos.y, pos.z, "minecraft:chests/simple_dungeon", seed);
                    }
                    break;
                }
            }
        }
        self.safe_set(ctx, origin, self.spawner);
        // SpawnerBlockEntity.setEntityId(Util.getRandom(MOBS, random), random).
        let mob = ["minecraft:skeleton", "minecraft:zombie", "minecraft:zombie", "minecraft:spider"][random.next_i32_bound(4) as usize];
        if ctx.is(ctx.block(origin), "minecraft:spawner") {
            ctx.region.set_spawner_entity(origin.x, origin.y, origin.z, mob);
        }
        true
    }
}

#[derive(Debug)]
struct Spring {
    state: BlockStateId,
    requires_block_below: bool,
    rock_count: i32,
    hole_count: i32,
    valid: BlockSet,
}

impl Spring {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        // A FluidState codec: {"Name": "minecraft:water", "Properties": {"falling": ...}}.
        let name = json["state"]["id"].as_str().or_else(|| json["state"]["Name"].as_str()).ok_or("spring lacks a fluid state")?;
        let block = match name.trim_start_matches("minecraft:") {
            "water" | "flowing_water" => "minecraft:water",
            "lava" | "flowing_lava" => "minecraft:lava",
            other => return Err(format!("unknown spring fluid {other}")),
        };
        Ok(Self {
            state: lib.registries.blocks.parse_state(block)?,
            requires_block_below: bool_or(json, "requires_block_below", true),
            rock_count: int_or(json, "rock_count", 4),
            hole_count: int_or(json, "hole_count", 1),
            valid: BlockSet::parse(&lib.registries, &json["valid_blocks"])?,
        })
    }
}

impl Placeable for Spring {
    fn place(&self, ctx: &mut Ctx, _random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let registries = ctx.registries();
        let valid = |p: BlockPos| self.valid.contains(registries, ctx.block(p));
        if !valid(origin.above()) {
            return false;
        }
        if self.requires_block_below && !valid(origin.below()) {
            return false;
        }
        let current = ctx.block(origin);
        if !ctx.is_air(current) && !self.valid.contains(registries, current) {
            return false;
        }
        let around = [origin.offset(-1, 0, 0), origin.offset(1, 0, 0), origin.offset(0, 0, -1), origin.offset(0, 0, 1), origin.below()];
        let rocks = around.iter().filter(|&&p| valid(p)).count() as i32;
        let holes = around.iter().filter(|&&p| ctx.is_empty_block(p)).count() as i32;
        if rocks == self.rock_count && holes == self.hole_count {
            ctx.set_block(origin, self.state);
            ctx.schedule_fluid_tick(origin);
            return true;
        }
        false
    }
}

#[derive(Debug)]
struct Disk {
    provider: Arc<StateProvider>,
    target: BlockPredicate,
    radius: IntProvider,
    half_height: i32,
}

impl Disk {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            provider: StateProvider::parse(lib, &json["state_provider"])?,
            target: BlockPredicate::parse(&lib.registries, &json["target"])?,
            radius: IntProvider::parse(&json["radius"])?,
            half_height: int(json, "half_height")?,
        })
    }
}

impl Placeable for Disk {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut any = false;
        let top = origin.y + self.half_height;
        let bottom = origin.y - self.half_height - 1;
        let r = self.radius.sample(random);
        for z in -r..=r {
            for x in -r..=r {
                if x * x + z * z > r * r {
                    continue;
                }
                let mut placed_above = false;
                let mut y = top;
                while y > bottom {
                    let pos = BlockPos::new(origin.x + x, y, origin.z + z);
                    if self.target.test(ctx, pos) {
                        if let Some(state) = self.provider.get_optional(ctx, random, pos) {
                            ctx.set_block(pos, state);
                            if !placed_above {
                                ctx.mark_above_for_post_processing(pos);
                            }
                            any = true;
                            placed_above = true;
                        }
                    } else {
                        placed_above = false;
                    }
                    y -= 1;
                }
            }
        }
        any
    }
}

#[derive(Debug)]
struct Lake {
    fluid: Arc<StateProvider>,
    barrier: Arc<StateProvider>,
    can_place: BlockPredicate,
    can_replace_air_or_fluid: BlockPredicate,
    can_replace_barrier: BlockPredicate,
}

impl Lake {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            fluid: StateProvider::parse(lib, &json["fluid"])?,
            barrier: StateProvider::parse(lib, &json["barrier"])?,
            can_place: BlockPredicate::parse(&lib.registries, &json["can_place_feature"])?,
            can_replace_air_or_fluid: BlockPredicate::parse(&lib.registries, &json["can_replace_with_air_or_fluid"])?,
            can_replace_barrier: BlockPredicate::parse(&lib.registries, &json["can_replace_with_barrier"])?,
        })
    }
}

impl Placeable for Lake {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if origin.y <= ctx.min_y() + 4 {
            return false;
        }
        let origin = origin.offset(-8, -4, -8);
        let mut grid = [false; 2048];
        let idx = |x: i32, z: i32, y: i32| ((x * 16 + z) * 8 + y) as usize;
        let spots = random.next_i32_bound(4) + 4;
        for _ in 0..spots {
            let xr = random.next_f64() * 6.0 + 3.0;
            let yr = random.next_f64() * 4.0 + 2.0;
            let zr = random.next_f64() * 6.0 + 3.0;
            let xp = random.next_f64() * (16.0 - xr - 2.0) + 1.0 + xr / 2.0;
            let yp = random.next_f64() * (8.0 - yr - 4.0) + 2.0 + yr / 2.0;
            let zp = random.next_f64() * (16.0 - zr - 2.0) + 1.0 + zr / 2.0;
            for xx in 1..15 {
                for zz in 1..15 {
                    for yy in 1..7 {
                        let xd = (f64::from(xx) - xp) / (xr / 2.0);
                        let yd = (f64::from(yy) - yp) / (yr / 2.0);
                        let zd = (f64::from(zz) - zp) / (zr / 2.0);
                        if xd * xd + yd * yd + zd * zd < 1.0 {
                            grid[idx(xx, zz, yy)] = true;
                        }
                    }
                }
            }
        }
        let edge = |grid: &[bool; 2048], xx: i32, zz: i32, yy: i32| {
            !grid[idx(xx, zz, yy)]
                && (xx < 15 && grid[idx(xx + 1, zz, yy)]
                    || xx > 0 && grid[idx(xx - 1, zz, yy)]
                    || zz < 15 && grid[idx(xx, zz + 1, yy)]
                    || zz > 0 && grid[idx(xx, zz - 1, yy)]
                    || yy < 7 && grid[idx(xx, zz, yy + 1)]
                    || yy > 0 && grid[idx(xx, zz, yy - 1)])
        };
        let fluid = self.fluid.get(ctx, random, origin);
        for xx in 0..16 {
            for zz in 0..16 {
                for yy in 0..8 {
                    if !edge(&grid, xx, zz, yy) {
                        continue;
                    }
                    let pos = origin.offset(xx, yy, zz);
                    let state = ctx.block(pos);
                    if yy >= 4 && ctx.registries().blocks.is(state, flags::LIQUID) {
                        return false;
                    }
                    if yy < 4 && !ctx.is_solid(state) && state != fluid {
                        return false;
                    }
                    if !self.can_place.test(ctx, pos) {
                        return false;
                    }
                }
            }
        }
        let air = ctx.lib.blocks.cave_air;
        for xx in 0..16 {
            for zz in 0..16 {
                for yy in 0..8 {
                    if grid[idx(xx, zz, yy)] {
                        let pos = origin.offset(xx, yy, zz);
                        if self.can_replace_air_or_fluid.test(ctx, pos) {
                            let place_air = yy >= 4;
                            ctx.set_block(pos, if place_air { air } else { fluid });
                            if place_air {
                                ctx.schedule_block_tick(pos);
                                ctx.mark_above_for_post_processing(pos);
                            }
                        }
                    }
                }
            }
        }
        let barrier = self.barrier.get(ctx, random, origin);
        if !ctx.is_air(barrier) {
            for xx in 0..16 {
                for zz in 0..16 {
                    for yy in 0..8 {
                        if edge(&grid, xx, zz, yy) && (yy < 4 || random.next_i32_bound(2) != 0) {
                            let pos = origin.offset(xx, yy, zz);
                            if ctx.is_solid(ctx.block(pos)) && self.can_replace_barrier.test(ctx, pos) {
                                ctx.set_block(pos, barrier);
                                ctx.mark_above_for_post_processing(pos);
                            }
                        }
                    }
                }
            }
        }
        if matches!(ctx.fluid(fluid), FluidType::Water | FluidType::FlowingWater) {
            for xx in 0..16 {
                for zz in 0..16 {
                    let pos = origin.offset(xx, 4, zz);
                    let Some(biome) = ctx.biome(pos) else { continue };
                    if super::simple::should_freeze(ctx, biome, pos) && self.can_replace_air_or_fluid.test(ctx, pos) {
                        ctx.set_block(pos, ctx.lib.blocks.ice);
                    }
                }
            }
        }
        true
    }
}

#[allow(dead_code)]
fn state(lib: &Library, json: &Value) -> Result<BlockStateId, String> {
    parse_state(&lib.registries, json)
}

#[derive(Debug)]
struct Geode {
    filling: Arc<StateProvider>,
    inner: Arc<StateProvider>,
    alternate_inner: Arc<StateProvider>,
    middle: Arc<StateProvider>,
    outer: Arc<StateProvider>,
    inner_placements: Vec<BlockStateId>,
    cannot_replace: BlockSet,
    invalid: BlockSet,
    layers: [f64; 4],
    crack_chance: f64,
    base_crack_size: f64,
    crack_point_offset: i32,
    potential_chance: f64,
    alternate_chance: f64,
    require_alternate: bool,
    outer_wall_distance: IntProvider,
    distribution_points: IntProvider,
    point_offset: IntProvider,
    min_offset: i32,
    max_offset: i32,
    noise_multiplier: f64,
    invalid_threshold: i32,
    noise: crate::noise::NormalNoise,
}

impl Geode {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let b = &json["blocks"];
        let f = |v: &Value, k: &str, d: f64| v.get(k).and_then(Value::as_f64).unwrap_or(d);
        let provider_or = |v: &Value, d: IntProvider| if v.is_null() { Ok(d) } else { IntProvider::parse(v) };
        let registries = lib.registries.clone();
        Ok(Self {
            filling: StateProvider::parse(lib, &b["filling_provider"])?,
            inner: StateProvider::parse(lib, &b["inner_layer_provider"])?,
            alternate_inner: StateProvider::parse(lib, &b["alternate_inner_layer_provider"])?,
            middle: StateProvider::parse(lib, &b["middle_layer_provider"])?,
            outer: StateProvider::parse(lib, &b["outer_layer_provider"])?,
            inner_placements: b["inner_placements"]
                .as_array()
                .ok_or("geode lacks inner_placements")?
                .iter()
                .map(|s| parse_state(&registries, s))
                .collect::<Result<_, _>>()?,
            cannot_replace: BlockSet::parse(&registries, &b["cannot_replace"])?,
            invalid: BlockSet::parse(&registries, &b["invalid_blocks"])?,
            layers: [
                f(&json["layers"], "filling", 1.7),
                f(&json["layers"], "inner_layer", 2.2),
                f(&json["layers"], "middle_layer", 3.2),
                f(&json["layers"], "outer_layer", 4.2),
            ],
            crack_chance: f(&json["crack"], "generate_crack_chance", 1.0),
            base_crack_size: f(&json["crack"], "base_crack_size", 2.0),
            crack_point_offset: json["crack"].get("crack_point_offset").and_then(Value::as_i64).map_or(2, |v| v as i32),
            potential_chance: f(json, "use_potential_placements_chance", 0.35),
            alternate_chance: f(json, "use_alternate_layer0_chance", 0.0),
            require_alternate: bool_or(json, "placements_require_layer0_alternate", true),
            outer_wall_distance: provider_or(&json["outer_wall_distance"], IntProvider::Uniform(4, 5))?,
            distribution_points: provider_or(&json["distribution_points"], IntProvider::Uniform(3, 4))?,
            point_offset: provider_or(&json["point_offset"], IntProvider::Uniform(1, 2))?,
            min_offset: int_or(json, "min_gen_offset", -16),
            max_offset: int_or(json, "max_gen_offset", 16),
            noise_multiplier: f(json, "noise_multiplier", 0.05),
            invalid_threshold: int(json, "invalid_blocks_threshold")?,
            noise: crate::noise::NormalNoise::create_parity(-4, &[1.0]),
        })
    }
}

impl Placeable for Geode {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut points: Vec<(BlockPos, i32)> = Vec::new();
        let num_points = self.distribution_points.sample(random);
        let noise = self.noise.create(&mut minecraftoss_core::random::AnyRandom::new(true, ctx.region.world_seed()));
        let crack_adjust = f64::from(num_points) / f64::from(self.outer_wall_distance.max_inclusive());
        let [filling, inner_layer, middle_layer, outer_layer] = self.layers;
        let inner_air = 1.0 / filling.sqrt();
        let innermost = 1.0 / (inner_layer + crack_adjust).sqrt();
        let inner_crust = 1.0 / (middle_layer + crack_adjust).sqrt();
        let outer_crust = 1.0 / (outer_layer + crack_adjust).sqrt();
        let crack_size = 1.0 / (self.base_crack_size + random.next_f64() / 2.0 + if num_points > 3 { crack_adjust } else { 0.0 }).sqrt();
        let crack = f64::from(random.next_f32()) < self.crack_chance;
        let mut invalid = 0;
        for _ in 0..num_points {
            let x = self.outer_wall_distance.sample(random);
            let y = self.outer_wall_distance.sample(random);
            let z = self.outer_wall_distance.sample(random);
            let pos = origin.offset(x, y, z);
            let state = ctx.block(pos);
            if ctx.is_air(state) || self.invalid.contains(ctx.registries(), state) {
                invalid += 1;
                if invalid > self.invalid_threshold {
                    return false;
                }
            }
            points.push((pos, self.point_offset.sample(random)));
        }
        let mut crack_points = Vec::new();
        if crack {
            let index = random.next_i32_bound(4);
            let o = num_points * 2 + 1;
            let (dx, dz) = match index {
                0 => (o, 0),
                1 => (0, o),
                2 => (o, o),
                _ => (0, 0),
            };
            for dy in [7, 5, 1] {
                crack_points.push(origin.offset(dx, dy, dz));
            }
        }
        let cannot = &self.cannot_replace;
        let can_replace = |ctx: &Ctx, s: BlockStateId| !cannot.contains(ctx.registries(), s);
        let mut potential = Vec::new();
        let (lo, hi) = (self.min_offset, self.max_offset);
        for z in lo..=hi {
            for y in lo..=hi {
                for x in lo..=hi {
                    let p = origin.offset(x, y, z);
                    let noise_offset = f64::from(noise.get(f64::from(p.x), f64::from(p.y), f64::from(p.z))) * self.noise_multiplier;
                    let mut shell = 0.0;
                    for &(point, offset) in &points {
                        shell += 1.0 / (p.dist_sqr(point) + f64::from(offset)).sqrt() + noise_offset;
                    }
                    if shell < outer_crust {
                        continue;
                    }
                    if shell >= inner_air {
                        let state = self.filling.get(ctx, random, p);
                        ctx.safe_set_block(p, state, can_replace);
                        continue;
                    }
                    let mut crack_sum = 0.0;
                    for &point in &crack_points {
                        crack_sum += 1.0 / (p.dist_sqr(point) + f64::from(self.crack_point_offset)).sqrt() + noise_offset;
                    }
                    if crack && crack_sum >= crack_size {
                        ctx.safe_set_block(p, ctx.lib.blocks.air, can_replace);
                        for d in Direction::ALL {
                            let adjacent = p.relative(d, 1);
                            if ctx.fluid_at(adjacent) != FluidType::Empty {
                                ctx.schedule_fluid_tick(adjacent);
                            }
                        }
                    } else if shell >= innermost {
                        let alternate = f64::from(random.next_f32()) < self.alternate_chance;
                        let state = if alternate { self.alternate_inner.get(ctx, random, p) } else { self.inner.get(ctx, random, p) };
                        ctx.safe_set_block(p, state, can_replace);
                        if (!self.require_alternate || alternate) && f64::from(random.next_f32()) < self.potential_chance {
                            potential.push(p);
                        }
                    } else if shell >= inner_crust {
                        let state = self.middle.get(ctx, random, p);
                        ctx.safe_set_block(p, state, can_replace);
                    } else {
                        let state = self.outer.get(ctx, random, p);
                        ctx.safe_set_block(p, state, can_replace);
                    }
                }
            }
        }
        for crystal in potential {
            let mut state = self.inner_placements[random.next_i32_bound(self.inner_placements.len() as i32) as usize];
            for d in Direction::ALL {
                if ctx.property(state, "facing").is_some() {
                    state = ctx.with(state, "facing", d.name());
                }
                let place = crystal.relative(d, 1);
                let place_state = ctx.block(place);
                if ctx.property(state, "waterlogged").is_some() {
                    let source = matches!(ctx.fluid(place_state), FluidType::Water | FluidType::Lava);
                    state = ctx.with(state, "waterlogged", if source { "true" } else { "false" });
                }
                // BuddingAmethystBlock.canClusterGrowAtState: air or a full water block.
                let full_water = ctx.is(place_state, "minecraft:water") && ctx.registries().blocks.state(place_state).fluid.is_some_and(|f| f.amount == 8);
                if ctx.is_air(place_state) || full_water {
                    ctx.safe_set_block(place, state, can_replace);
                    break;
                }
            }
        }
        true
    }
}

#[derive(Debug)]
struct Iceberg {
    state: BlockStateId,
    snow_block: BlockStateId,
}

impl Iceberg {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self { state: parse_state(&lib.registries, &json["state"])?, snow_block: lib.blocks.snow_block })
    }

    fn is_iceberg(ctx: &Ctx, s: BlockStateId) -> bool {
        ctx.is(s, "minecraft:packed_ice") || ctx.is(s, "minecraft:snow_block") || ctx.is(s, "minecraft:blue_ice")
    }

    fn radius_round(random: &mut WorldgenRandom, y_off: i32, height: i32, width: i32) -> i32 {
        let k = 3.5f32 - random.next_f32();
        let mut scale = (1.0f32 - (y_off as f64).powi(2) as f32 / (height as f32 * k)) * width as f32;
        if height > 15 + random.next_i32_bound(5) {
            let temp = if y_off < 3 + random.next_i32_bound(6) { y_off / 2 } else { y_off };
            scale = (1.0f32 - temp as f32 / (height as f32 * k * 0.4)) * width as f32;
        }
        f64::from(scale / 2.0).ceil() as i32
    }

    fn radius_ellipse(y_off: i32, height: i32, width: i32) -> i32 {
        let scale = (1.0f32 - (y_off as f64).powi(2) as f32 / (height as f32 * 1.0)) * width as f32;
        f64::from(scale / 2.0).ceil() as i32
    }

    fn radius_steep(random: &mut WorldgenRandom, y_off: i32, height: i32, width: i32) -> i32 {
        let k = 1.0f32 + random.next_f32() / 2.0;
        let scale = (1.0f32 - y_off as f32 / (height as f32 * k)) * width as f32;
        f64::from(scale / 2.0).ceil() as i32
    }

    fn ellipse_c(y_off: i32, height: i32, c: i32) -> i32 {
        if y_off > 0 && height - y_off <= 3 { c - (4 - (height - y_off)) } else { c }
    }

    fn circle(xo: i32, zo: i32, radius: i32, random: &mut WorldgenRandom) -> f64 {
        let off = 10.0f32 * crate::mth::clamp(random.next_f32(), 0.2, 0.8) / radius as f32;
        f64::from(off) + f64::from(xo).powi(2) + f64::from(zo).powi(2) - f64::from(radius).powi(2)
    }

    fn ellipse(xo: i32, zo: i32, origin: (i32, i32), a: i32, c: i32, angle: f64) -> f64 {
        let (dx, dz) = (f64::from(xo - origin.0), f64::from(zo - origin.1));
        ((dx * angle.cos() - dz * angle.sin()) / f64::from(a)).powi(2) + ((dx * angle.sin() + dz * angle.cos()) / f64::from(c)).powi(2) - 1.0
    }

    #[allow(clippy::too_many_arguments)]
    fn block(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos, height: i32, xo: i32, y_off: i32, zo: i32, radius: i32, a: i32, ellipse: bool, c: i32, angle: f64, snow: bool) {
        let dist = if ellipse { Self::ellipse(xo, zo, (0, 0), a, Self::ellipse_c(y_off, height, c), angle) } else { Self::circle(xo, zo, radius, random) };
        if dist >= 0.0 {
            return;
        }
        let pos = origin.offset(xo, y_off, zo);
        let compare = if ellipse { -0.5 } else { f64::from(-6 - random.next_i32_bound(3)) };
        if dist > compare && random.next_f64() > 0.9 {
            return;
        }
        let h_diff = height - y_off;
        let state = ctx.block(pos);
        if ctx.is_air(state) || ctx.is(state, "minecraft:snow_block") || ctx.is(state, "minecraft:ice") || ctx.is(state, "minecraft:water") {
            let randomness = !ellipse || random.next_f64() > 0.05;
            let divisor = if ellipse { 3 } else { 2 };
            let is_water = ctx.is(state, "minecraft:water");
            if snow && !is_water && f64::from(h_diff) <= f64::from(random.next_i32_bound(1.max(height / divisor))) + f64::from(height) * 0.6 && randomness {
                ctx.set_block_update(pos, self.snow_block);
            } else {
                ctx.set_block_update(pos, self.state);
            }
        }
    }

    fn smooth(&self, ctx: &mut Ctx, origin: BlockPos, width: i32, height: i32, ellipse: bool, ellipse_a: i32) {
        let a = if ellipse { ellipse_a } else { width / 2 };
        for x in -a..=a {
            for z in -a..=a {
                for y_off in 0..=height {
                    let pos = origin.offset(x, y_off, z);
                    let state = ctx.block(pos);
                    if !(Self::is_iceberg(ctx, state) || ctx.is(state, "minecraft:snow")) {
                        continue;
                    }
                    if ctx.is_empty_block(pos.below()) {
                        ctx.set_block_update(pos, ctx.lib.blocks.air);
                        ctx.set_block_update(pos.above(), ctx.lib.blocks.air);
                    } else if Self::is_iceberg(ctx, state) {
                        let sides = [pos.offset(-1, 0, 0), pos.offset(1, 0, 0), pos.offset(0, 0, -1), pos.offset(0, 0, 1)];
                        let open = sides.iter().filter(|&&p| !Self::is_iceberg(ctx, ctx.block(p))).count();
                        if open >= 3 {
                            ctx.set_block_update(pos, ctx.lib.blocks.air);
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn carve(&self, ctx: &mut Ctx, radius: i32, y_off: i32, origin: BlockPos, under_water: bool, angle: f64, local: (i32, i32), ellipse_a: i32, ellipse_c: i32) {
        let a = radius + 1 + ellipse_a / 3;
        let c = (radius - 3).min(3) + ellipse_c / 2 - 1;
        for xo in -a..a {
            for zo in -a..a {
                if Self::ellipse(xo, zo, local, a, c, angle) >= 0.0 {
                    continue;
                }
                let pos = origin.offset(xo, y_off, zo);
                let state = ctx.block(pos);
                if Self::is_iceberg(ctx, state) || ctx.is(state, "minecraft:snow_block") {
                    if under_water {
                        ctx.set_block_update(pos, ctx.lib.blocks.water);
                    } else {
                        ctx.set_block_update(pos, ctx.lib.blocks.air);
                        if ctx.is(ctx.block(pos.above()), "minecraft:snow") {
                            ctx.set_block_update(pos.above(), ctx.lib.blocks.air);
                        }
                    }
                }
            }
        }
    }
}

impl Placeable for Iceberg {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let origin = origin.at_y(ctx.lib.generation.sea_level);
        let snow = random.next_f64() > 0.7;
        let angle = random.next_f64() * 2.0 * std::f64::consts::PI;
        let ellipse_a = 11 - random.next_i32_bound(5);
        let ellipse_c = 3 + random.next_i32_bound(3);
        let ellipse = random.next_f64() > 0.7;
        let mut over = if ellipse { random.next_i32_bound(6) + 6 } else { random.next_i32_bound(15) + 3 };
        if !ellipse && random.next_f64() > 0.9 {
            over += random.next_i32_bound(19) + 7;
        }
        let under = (over + random.next_i32_bound(11)).min(18);
        let width = (over + random.next_i32_bound(7) - random.next_i32_bound(5)).min(11);
        let a = if ellipse { ellipse_a } else { 11 };
        for xo in -a..a {
            for zo in -a..a {
                for y_off in 0..over {
                    let radius = if ellipse { Self::radius_ellipse(y_off, over, width) } else { Self::radius_round(random, y_off, over, width) };
                    if ellipse || xo < radius {
                        self.block(ctx, random, origin, over, xo, y_off, zo, radius, a, ellipse, ellipse_c, angle, snow);
                    }
                }
            }
        }
        self.smooth(ctx, origin, width, over, ellipse, ellipse_a);
        for xo in -a..a {
            for zo in -a..a {
                let mut y_off = -1;
                while y_off > -under {
                    let new_a = if ellipse {
                        let v = a as f32 * (1.0f32 - (y_off as f64).powi(2) as f32 / (under as f32 * 8.0));
                        f64::from(v).ceil() as i32
                    } else {
                        a
                    };
                    let radius = Self::radius_steep(random, -y_off, under, width);
                    if xo < radius {
                        self.block(ctx, random, origin, under, xo, y_off, zo, radius, new_a, ellipse, ellipse_c, angle, snow);
                    }
                    y_off -= 1;
                }
            }
        }
        let cut = if ellipse { random.next_f64() > 0.1 } else { random.next_f64() > 0.7 };
        if cut {
            let sign_x = if random.next_bool() { -1 } else { 1 };
            let sign_z = if random.next_bool() { -1 } else { 1 };
            let mut x_off = random.next_i32_bound((width / 2 - 2).max(1));
            if random.next_bool() {
                x_off = width / 2 + 1 - random.next_i32_bound((width - width / 2 - 1).max(1));
            }
            let mut z_off = random.next_i32_bound((width / 2 - 2).max(1));
            if random.next_bool() {
                z_off = width / 2 + 1 - random.next_i32_bound((width - width / 2 - 1).max(1));
            }
            if ellipse {
                x_off = random.next_i32_bound((ellipse_a - 5).max(1));
                z_off = x_off;
            }
            let local = (sign_x * x_off, sign_z * z_off);
            let cut_angle = if ellipse { angle + std::f64::consts::FRAC_PI_2 } else { random.next_f64() * 2.0 * std::f64::consts::PI };
            for y_off in 0..over - 3 {
                let radius = Self::radius_round(random, y_off, over, width);
                self.carve(ctx, radius, y_off, origin, false, cut_angle, local, ellipse_a, ellipse_c);
            }
            let mut y_off = -1;
            while y_off > -over + random.next_i32_bound(5) {
                let radius = Self::radius_steep(random, -y_off, over, width);
                self.carve(ctx, radius, y_off, origin, true, cut_angle, local, ellipse_a, ellipse_c);
                y_off -= 1;
            }
        }
        true
    }
}
