//! Ore-like features: `OreFeature`, `ScatteredOreFeature`,
//! `ReplaceBlobsFeature` and `UnderwaterMagmaFeature`.

use super::{float, int};
use crate::feature::blocks::parse_state;
use crate::feature::rule_test::Replacement;
use crate::feature::{Ctx, Library, World};
use crate::providers::{sin, IntProvider};
use minecraftoss_core::block::FaceShape;
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::{BlockPos, BlockStateId};
use serde_json::Value;

#[derive(Debug)]
pub struct Ore {
    targets: Vec<Replacement>,
    size: i32,
    discard_chance: f32,
}

/// `AbstractOreFeature.shouldSkipAirCheck`.
fn skip_air_check(random: &mut WorldgenRandom, chance: f32) -> bool {
    if chance <= 0.0 {
        true
    } else if chance >= 1.0 {
        false
    } else {
        random.next_f32() >= chance
    }
}

fn adjacent_to_air<W: World + ?Sized>(get: impl Fn(BlockPos) -> BlockStateId, ctx: &Ctx<W>, pos: BlockPos) -> bool {
    Direction::ALL.iter().any(|&d| ctx.is_air(get(pos.relative(d, 1))))
}

impl Ore {
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            targets: Replacement::parse_list(&lib.registries, &json["targets"])?,
            size: int(json, "size")?,
            discard_chance: float(json, "discard_chance_on_air_exposure")?,
        })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let dir = random.next_f32() * std::f32::consts::PI;
        let spread_xy = self.size as f32 / 8.0;
        let max_radius = (((self.size as f32 / 16.0 * 2.0 + 1.0) / 2.0) as f64).ceil() as i32;
        let (dsin, dcos) = (f64::from(dir).sin(), f64::from(dir).cos());
        let x0 = f64::from(origin.x) + dsin * f64::from(spread_xy);
        let x1 = f64::from(origin.x) - dsin * f64::from(spread_xy);
        let z0 = f64::from(origin.z) + dcos * f64::from(spread_xy);
        let z1 = f64::from(origin.z) - dcos * f64::from(spread_xy);
        let y0 = f64::from(origin.y + random.next_i32_bound(3) - 2);
        let y1 = f64::from(origin.y + random.next_i32_bound(3) - 2);
        let ceil_spread = f64::from(spread_xy).ceil() as i32;
        let x_start = origin.x - ceil_spread - max_radius;
        let y_start = origin.y - 2 - max_radius;
        let z_start = origin.z - ceil_spread - max_radius;
        let size_xz = 2 * (ceil_spread + max_radius);
        let size_y = 2 * (2 + max_radius);
        // Vanilla walks the square for any column at or above `y_start`.
        if ctx.any_height_at_least(HeightmapKind::OceanFloorWg, x_start, z_start, x_start + size_xz, z_start + size_xz, y_start) {
            let c = [x0, x1, z0, z1, y0, y1];
            // The region itself when it is one, for inlined block access.
            let lib = ctx.lib;
            if let Some(region) = ctx.region.as_region_mut() {
                return self.do_place(&mut Ctx { lib, region }, random, c, x_start, y_start, z_start, size_xz, size_y);
            }
            return self.do_place(ctx, random, c, x_start, y_start, z_start, size_xz, size_y);
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn do_place<W: World + ?Sized>(&self, ctx: &mut Ctx<W>, random: &mut WorldgenRandom, c: [f64; 6], x_start: i32, y_start: i32, z_start: i32, size_xz: i32, size_y: i32) -> bool {
        let [x0, x1, z0, z1, y0, y1] = c;
        let size = self.size as usize;
        let mut data = vec![0.0f64; size * 4];
        for i in 0..size {
            let step = i as f32 / self.size as f32;
            let xx = x0 + f64::from(step) * (x1 - x0);
            let yy = y0 + f64::from(step) * (y1 - y0);
            let zz = z0 + f64::from(step) * (z1 - z0);
            let ss = random.next_f64() * f64::from(self.size) / 16.0;
            let r = (f64::from(sin(f64::from(std::f32::consts::PI * step)) + 1.0) * ss + 1.0) / 2.0;
            data[i * 4..i * 4 + 4].copy_from_slice(&[xx, yy, zz, r]);
        }
        for i1 in 0..size.saturating_sub(1) {
            if data[i1 * 4 + 3] <= 0.0 {
                continue;
            }
            for i2 in i1 + 1..size {
                if data[i2 * 4 + 3] <= 0.0 {
                    continue;
                }
                let dx = data[i1 * 4] - data[i2 * 4];
                let dy = data[i1 * 4 + 1] - data[i2 * 4 + 1];
                let dz = data[i1 * 4 + 2] - data[i2 * 4 + 2];
                let dr = data[i1 * 4 + 3] - data[i2 * 4 + 3];
                if dr * dr > dx * dx + dy * dy + dz * dz {
                    if dr > 0.0 {
                        data[i2 * 4 + 3] = -1.0;
                    } else {
                        data[i1 * 4 + 3] = -1.0;
                    }
                }
            }
        }
        let mut tested = vec![false; (size_xz * size_y * size_xz).max(0) as usize + size_xz.max(0) as usize * 2 + size_y.max(0) as usize + 8];
        let mut placed = 0;
        let mut zd_squares: Vec<f64> = Vec::new();
        for i in 0..size {
            let r = data[i * 4 + 3];
            if r < 0.0 {
                continue;
            }
            let (xx, yy, zz) = (data[i * 4], data[i * 4 + 1], data[i * 4 + 2]);
            let x_min = ((xx - r).floor() as i32).max(x_start);
            let y_min = ((yy - r).floor() as i32).max(y_start);
            let z_min = ((zz - r).floor() as i32).max(z_start);
            let x_max = ((xx + r).floor() as i32).max(x_min);
            let y_max = ((yy + r).floor() as i32).max(y_min);
            let z_max = ((zz + r).floor() as i32).max(z_min);
            // The same quotients as per block, once per sphere.
            zd_squares.clear();
            zd_squares.extend((z_min..=z_max).map(|z| {
                let zd = (f64::from(z) + 0.5 - zz) / r;
                zd * zd
            }));
            for x in x_min..=x_max {
                let xd = (f64::from(x) + 0.5 - xx) / r;
                if xd * xd >= 1.0 {
                    continue;
                }
                for y in y_min..=y_max {
                    let yd = (f64::from(y) + 0.5 - yy) / r;
                    if xd * xd + yd * yd >= 1.0 {
                        continue;
                    }
                    let outside = ctx.region.is_outside_build_height(y);
                    let xy = xd * xd + yd * yd;
                    for (z, &zd2) in (z_min..=z_max).zip(&zd_squares) {
                        if xy + zd2 >= 1.0 || outside {
                            continue;
                        }
                        let index = (x - x_start + (y - y_start) * size_xz + (z - z_start) * size_xz * size_y) as usize;
                        if index >= tested.len() {
                            tested.resize(index + 1, false);
                        }
                        if tested[index] {
                            continue;
                        }
                        tested[index] = true;
                        let pos = BlockPos::new(x, y, z);
                        if !ctx.region.contains(x, z) {
                            continue;
                        }
                        let state = ctx.block(pos);
                        for target in &self.targets {
                            if self.can_place(ctx, random, state, target, pos) {
                                ctx.region.set_block_section(x, y, z, target.state);
                                placed += 1;
                                break;
                            }
                        }
                    }
                }
            }
        }
        placed > 0
    }

    fn can_place<W: World + ?Sized>(&self, ctx: &Ctx<W>, random: &mut WorldgenRandom, state: BlockStateId, target: &Replacement, pos: BlockPos) -> bool {
        if !target.target.test(ctx.registries(), state, pos, random) {
            return false;
        }
        skip_air_check(random, self.discard_chance) || !adjacent_to_air(|p| ctx.block(p), ctx, pos)
    }
}

#[derive(Debug)]
pub struct ScatteredOre {
    ore: Ore,
}

impl ScatteredOre {
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self { ore: Ore::parse(lib, json)? })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let tries = random.next_i32_bound(self.ore.size + 1);
        for i in 0..tries {
            let max = i.min(7);
            let mut axis = || java_round((random.next_f32() - random.next_f32()) * max as f32);
            let (xd, yd, zd) = (axis(), axis(), axis());
            let pos = origin.offset(xd, yd, zd);
            let state = ctx.block(pos);
            for target in &self.ore.targets {
                if self.ore.can_place(ctx, random, state, target, pos) {
                    ctx.set_block(pos, target.state);
                    break;
                }
            }
        }
        true
    }
}

#[derive(Debug)]
pub struct ReplaceBlobs {
    target: BlockStateId,
    state: BlockStateId,
    radius: IntProvider,
}

/// `BlockPos.manhattanOrdered`: shells of growing Manhattan distance.
pub struct Manhattan {
    origin: BlockPos,
    reach: (i32, i32, i32),
    max_depth: i32,
    depth: i32,
    max_x: i32,
    max_y: i32,
    x: i32,
    y: i32,
    mirror: Option<BlockPos>,
}

impl Manhattan {
    pub fn new(origin: BlockPos, rx: i32, ry: i32, rz: i32, max_depth: i32) -> Self {
        Self { origin, reach: (rx, ry, rz), max_depth, depth: 0, max_x: 0, max_y: 0, x: 0, y: 0, mirror: None }
    }
}

impl Iterator for Manhattan {
    type Item = BlockPos;

    fn next(&mut self) -> Option<BlockPos> {
        if let Some(m) = self.mirror.take() {
            return Some(m);
        }
        loop {
            if self.y > self.max_y {
                self.x += 1;
                if self.x > self.max_x {
                    self.depth += 1;
                    if self.depth > self.max_depth {
                        return None;
                    }
                    self.max_x = self.reach.0.min(self.depth);
                    self.x = -self.max_x;
                }
                self.max_y = self.reach.1.min(self.depth - self.x.abs());
                self.y = -self.max_y;
            }
            let (xx, yy) = (self.x, self.y);
            self.y += 1;
            let zz = self.depth - xx.abs() - yy.abs();
            if zz <= self.reach.2 {
                let found = self.origin.offset(xx, yy, zz);
                if zz != 0 {
                    self.mirror = Some(self.origin.offset(xx, yy, -zz));
                }
                return Some(found);
            }
        }
    }
}

impl ReplaceBlobs {
    pub fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            target: parse_state(&lib.registries, &json["target"])?,
            state: parse_state(&lib.registries, &json["state"])?,
            radius: IntProvider::parse(&json["radius"])?,
        })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let blocks = &ctx.registries().blocks;
        let target_block = blocks.block_of(self.target);
        let mut cursor = origin.at_y(origin.y.clamp(ctx.min_y() + 1, ctx.max_y()));
        let center = loop {
            if cursor.y <= ctx.min_y() + 1 {
                return false;
            }
            if ctx.registries().blocks.block_of(ctx.block(cursor)) == target_block {
                break cursor;
            }
            cursor = cursor.below();
        };
        let rx = self.radius.sample(random);
        let ry = self.radius.sample(random);
        let rz = self.radius.sample(random);
        let max = rx.max(ry.max(rz));
        let mut any = false;
        for pos in Manhattan::new(center, rx, ry, rz, rx + ry + rz) {
            if pos.dist_manhattan(center) > max {
                break;
            }
            if ctx.registries().blocks.block_of(ctx.block(pos)) == target_block {
                ctx.set_block_update(pos, self.state);
                any = true;
            }
        }
        any
    }
}

#[derive(Debug)]
pub struct UnderwaterMagma {
    floor_search_range: i32,
    radius: i32,
    probability: f32,
}

impl UnderwaterMagma {
    pub fn parse(_lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            floor_search_range: int(json, "floor_search_range")?,
            radius: int(json, "placement_radius_around_floor")?,
            probability: float(json, "placement_probability_per_valid_position")?,
        })
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let water = ctx.lib.blocks.water;
        let Some(floor) = column_floor(ctx, origin, self.floor_search_range, |s| s == water, |s| s != water) else {
            return false;
        };
        let magma = ctx.registries().blocks.parse_state("minecraft:magma_block").expect("magma block exists");
        let floor_pos = origin.at_y(floor);
        let r = self.radius;
        let mut any = false;
        for z in -r..=r {
            for y in -r..=r {
                for x in -r..=r {
                    let pos = floor_pos.offset(x, y, z);
                    if random.next_f32() < self.probability && self.valid(ctx, pos) {
                        ctx.set_block(pos, magma);
                        any = true;
                    }
                }
            }
        }
        any
    }

    fn valid(&self, ctx: &Ctx, pos: BlockPos) -> bool {
        let state = ctx.block(pos);
        if state == ctx.lib.blocks.water || ctx.is_air(state) || visible_from_outside(ctx, pos.below(), Direction::Up) {
            return false;
        }
        !Direction::HORIZONTAL.iter().any(|&d| visible_from_outside(ctx, pos.relative(d, 1), d.opposite()))
    }
}

fn visible_from_outside(ctx: &Ctx, pos: BlockPos, covered: Direction) -> bool {
    !matches!(ctx.registries().blocks.face_shape(ctx.block(pos), covered), FaceShape::Full)
}

/// `Column.scan(...).getFloor()`.
pub fn column_floor(ctx: &Ctx, pos: BlockPos, range: i32, inside: impl Fn(BlockStateId) -> bool, edge: impl Fn(BlockStateId) -> bool) -> Option<i32> {
    column_scan(ctx, pos, range, &inside, &edge).map(|(floor, _)| floor).flatten()
}

/// `Column.scan`: `(floor, ceiling)` when the start is inside the column.
pub fn column_scan(ctx: &Ctx, pos: BlockPos, range: i32, inside: &dyn Fn(BlockStateId) -> bool, edge: &dyn Fn(BlockStateId) -> bool) -> Option<(Option<i32>, Option<i32>)> {
    if !inside(ctx.block(pos)) {
        return None;
    }
    let scan = |direction: Direction| {
        let mut p = pos;
        let mut i = 1;
        while i < range && inside(ctx.block(p)) {
            p = p.relative(direction, 1);
            i += 1;
        }
        edge(ctx.block(p)).then_some(p.y)
    };
    let ceiling = scan(Direction::Up);
    let floor = scan(Direction::Down);
    Some((floor, ceiling))
}

/// `Math.round(float)`: nearest, ties toward positive infinity.
pub fn java_round(v: f32) -> i32 {
    if v.is_nan() {
        return 0;
    }
    let f = v.floor();
    (if v - f >= 0.5 { f + 1.0 } else { f }) as i32
}
