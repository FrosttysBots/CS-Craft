//! Foliage placers (vanilla `feature.foliageplacers`).

use super::{Attachment, Sets, Tree};
use crate::feature::blocks::FluidType;
use crate::feature::kinds::{float, int};
use crate::feature::state::try_with;
use crate::feature::Ctx;
use crate::providers::IntProvider;
use minecraftoss_core::pos::{Axis, Direction};
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::BlockPos;
use serde_json::Value;

#[derive(Debug)]
pub enum Kind {
    Blob { height: i32 },
    Bush { height: i32 },
    Fancy { height: i32 },
    Acacia,
    Cherry { height: IntProvider, wide_hole: f32, corner_hole: f32, hanging: f32, hanging_extension: f32 },
    DarkOak,
    MegaJungle { height: i32 },
    MegaPine { crown_height: IntProvider },
    Pine { height: IntProvider },
    Poplar { height: IntProvider, side_hole: f32 },
    RandomSpread { foliage_height: IntProvider, attempts: i32 },
    Spruce { trunk_height: IntProvider },
}

#[derive(Debug)]
pub struct FoliagePlacer {
    radius: IntProvider,
    offset: IntProvider,
    kind: Kind,
}

impl FoliagePlacer {
    pub fn parse(json: &Value) -> Result<Self, String> {
        let name = json["type"].as_str().ok_or("foliage placer lacks a type")?;
        let provider = |key: &str| IntProvider::parse(&json[key]);
        let kind = match name.trim_start_matches("minecraft:") {
            "blob_foliage_placer" => Kind::Blob { height: int(json, "height")? },
            "bush_foliage_placer" => Kind::Bush { height: int(json, "height")? },
            "fancy_foliage_placer" => Kind::Fancy { height: int(json, "height")? },
            "acacia_foliage_placer" => Kind::Acacia,
            "cherry_foliage_placer" => Kind::Cherry {
                height: provider("height")?,
                wide_hole: float(json, "wide_bottom_layer_hole_chance")?,
                corner_hole: float(json, "corner_hole_chance")?,
                hanging: float(json, "hanging_leaves_chance")?,
                hanging_extension: float(json, "hanging_leaves_extension_chance")?,
            },
            "dark_oak_foliage_placer" => Kind::DarkOak,
            "jungle_foliage_placer" | "mega_jungle_foliage_placer" => Kind::MegaJungle { height: int(json, "height")? },
            "mega_pine_foliage_placer" => Kind::MegaPine { crown_height: provider("crown_height")? },
            "pine_foliage_placer" => Kind::Pine { height: provider("height")? },
            "poplar_foliage_placer" => Kind::Poplar { height: provider("height")?, side_hole: float(json, "side_hole_chance")? },
            "random_spread_foliage_placer" => Kind::RandomSpread { foliage_height: provider("foliage_height")?, attempts: int(json, "leaf_placement_attempts")? },
            "spruce_foliage_placer" => Kind::Spruce { trunk_height: provider("trunk_height")? },
            other => return Err(format!("unknown foliage placer {other}")),
        };
        Ok(Self { radius: provider("radius")?, offset: provider("offset")?, kind })
    }

    /// `foliageHeight`.
    pub fn foliage_height(&self, random: &mut WorldgenRandom, tree_height: i32) -> i32 {
        match &self.kind {
            Kind::Blob { height } | Kind::Bush { height } | Kind::Fancy { height } | Kind::MegaJungle { height } => *height,
            Kind::Acacia => 0,
            Kind::DarkOak => 4,
            Kind::Cherry { height, .. } | Kind::Pine { height } | Kind::Poplar { height, .. } => height.sample(random),
            Kind::MegaPine { crown_height } => crown_height.sample(random),
            Kind::RandomSpread { foliage_height, .. } => foliage_height.sample(random),
            Kind::Spruce { trunk_height } => 4.max(tree_height - trunk_height.sample(random)),
        }
    }

    /// `foliageRadius`.
    pub fn foliage_radius(&self, random: &mut WorldgenRandom, trunk_height: i32) -> i32 {
        let base = self.radius.sample(random);
        match self.kind {
            Kind::Pine { .. } => base + random.next_i32_bound((trunk_height + 1).max(1)),
            _ => base,
        }
    }

    /// `shouldSkipLocation` for the placers that use the shared row code.
    fn skip(&self, random: &mut WorldgenRandom, dx: i32, y: i32, dz: i32, r: i32, double: bool) -> bool {
        match &self.kind {
            Kind::Blob { .. } => dx == r && dz == r && (random.next_i32_bound(2) == 0 || y == 0),
            Kind::Bush { .. } => dx == r && dz == r && random.next_i32_bound(2) == 0,
            Kind::Fancy { .. } => {
                let (a, b) = (dx as f32 + 0.5, dz as f32 + 0.5);
                a * a + b * b > (r * r) as f32
            }
            Kind::Acacia => {
                if y == 0 {
                    (dx > 1 || dz > 1) && dx != 0 && dz != 0
                } else {
                    dx == r && dz == r && r > 0
                }
            }
            Kind::Cherry { wide_hole, corner_hole, .. } => {
                if y == -1 && (dx == r || dz == r) && random.next_f32() < *wide_hole {
                    return true;
                }
                let corner = dx == r && dz == r;
                if r > 2 {
                    corner || dx + dz > r * 2 - 2 && random.next_f32() < *corner_hole
                } else {
                    corner && random.next_f32() < *corner_hole
                }
            }
            Kind::DarkOak => {
                if y == -1 && !double {
                    dx == r && dz == r
                } else {
                    y == 1 && dx + dz > r * 2 - 2
                }
            }
            Kind::MegaJungle { .. } | Kind::MegaPine { .. } => dx + dz >= 7 || dx * dx + dz * dz > r * r,
            Kind::Pine { .. } | Kind::Spruce { .. } => dx == r && dz == r && r > 0,
            Kind::RandomSpread { .. } => false,
            Kind::Poplar { .. } => unreachable!("poplar rows use their own skip rule"),
        }
    }

    /// `shouldSkipLocationSigned`.
    fn skip_signed(&self, random: &mut WorldgenRandom, dx: i32, y: i32, dz: i32, r: i32, double: bool) -> bool {
        if matches!(self.kind, Kind::DarkOak) && y == 0 && double && (dx == -r || dx >= r) && (dz == -r || dz >= r) {
            return true;
        }
        let (mdx, mdz) = if double { (dx.abs().min((dx - 1).abs()), dz.abs().min((dz - 1).abs())) } else { (dx.abs(), dz.abs()) };
        self.skip(random, mdx, y, mdz, r, double)
    }

    fn row(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, origin: BlockPos, r: i32, y: i32, double: bool) {
        let offset = i32::from(double);
        for dx in -r..=r + offset {
            for dz in -r..=r + offset {
                if !self.skip_signed(random, dx, y, dz, r, double) {
                    try_place_leaf(ctx, random, sets, tree, origin.offset(dx, y, dz));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn row_with_hanging(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, origin: BlockPos, r: i32, y: i32, double: bool, chance: f32, extension: f32) {
        self.row(ctx, random, sets, tree, origin, r, y, double);
        let offset = i32::from(double);
        let log = origin.below();
        for along in Direction::HORIZONTAL {
            let to_edge = along.clockwise();
            let positive = matches!(to_edge, Direction::East | Direction::South | Direction::Up);
            let to_edge_offset = if positive { r + offset } else { r };
            let mut pos = origin.offset(0, y - 1, 0).relative(to_edge, to_edge_offset).relative(along, -r);
            let mut i = -r;
            while i < r + offset {
                let above = sets.foliage.contains(pos.above());
                if above && try_place_extension(ctx, random, sets, tree, chance, log, pos) {
                    try_place_extension(ctx, random, sets, tree, extension, log, pos.below());
                }
                i += 1;
                pos = pos.relative(along, 1);
            }
        }
    }

    /// `FoliagePlacer.createFoliage`.
    #[allow(clippy::too_many_arguments)]
    pub fn create(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, attachment: Attachment, foliage_height: i32, leaf_radius: i32) {
        let offset = self.offset.sample(random);
        let a = attachment;
        let double = a.double_trunk();
        let height = foliage_height + a.foliage_height_offset;
        match &self.kind {
            Kind::Blob { .. } => {
                let mut yo = offset;
                while yo >= offset - height {
                    let r = (leaf_radius + a.radius_offset - 1 - yo / 2).max(0);
                    self.row(ctx, random, sets, tree, a.pos, r, yo, double);
                    yo -= 1;
                }
            }
            Kind::Bush { .. } => {
                let mut yo = offset;
                while yo >= offset - height {
                    let r = leaf_radius + a.radius_offset - 1 - yo;
                    self.row(ctx, random, sets, tree, a.pos, r, yo, double);
                    yo -= 1;
                }
            }
            Kind::Fancy { .. } => {
                let mut yo = offset;
                while yo >= offset - foliage_height {
                    let r = leaf_radius + i32::from(yo != offset && yo != offset - foliage_height);
                    self.row(ctx, random, sets, tree, a.pos, r, yo, double);
                    yo -= 1;
                }
            }
            Kind::Acacia => {
                let pos = a.pos.offset(0, offset, 0);
                self.row(ctx, random, sets, tree, pos, leaf_radius + a.radius_offset, -1 - height, double);
                self.row(ctx, random, sets, tree, pos, leaf_radius - 1, -height, double);
                self.row(ctx, random, sets, tree, pos, leaf_radius + a.radius_offset - 1, 0, double);
            }
            Kind::Cherry { hanging, hanging_extension, .. } => {
                let pos = a.pos.offset(0, offset, 0);
                let r = leaf_radius + a.radius_offset - 1;
                self.row(ctx, random, sets, tree, pos, r - 2, height - 3, double);
                self.row(ctx, random, sets, tree, pos, r - 1, height - 4, double);
                let mut y = height - 5;
                while y >= 0 {
                    self.row(ctx, random, sets, tree, pos, r, y, double);
                    y -= 1;
                }
                self.row_with_hanging(ctx, random, sets, tree, pos, r, -1, double, *hanging, *hanging_extension);
                self.row_with_hanging(ctx, random, sets, tree, pos, r - 1, -2, double, *hanging, *hanging_extension);
            }
            Kind::DarkOak => {
                let pos = a.pos.offset(0, offset, 0);
                if double {
                    self.row(ctx, random, sets, tree, pos, leaf_radius + 2, -1, double);
                    self.row(ctx, random, sets, tree, pos, leaf_radius + 3, 0, double);
                    self.row(ctx, random, sets, tree, pos, leaf_radius + 2, 1, double);
                    if random.next_bool() {
                        self.row(ctx, random, sets, tree, pos, leaf_radius, 2, double);
                    }
                } else {
                    self.row(ctx, random, sets, tree, pos, leaf_radius + 2, -1, double);
                    self.row(ctx, random, sets, tree, pos, leaf_radius + 1, 0, double);
                }
            }
            Kind::MegaJungle { .. } => {
                let leaf_height = (if double { foliage_height } else { 1 + random.next_i32_bound(2) }) + a.foliage_height_offset;
                let mut yo = offset;
                while yo >= offset - leaf_height {
                    let r = leaf_radius + a.radius_offset + 1 - yo;
                    self.row(ctx, random, sets, tree, a.pos, r, yo, double);
                    yo -= 1;
                }
            }
            Kind::MegaPine { .. } => {
                let mut prev = 0;
                let mut yy = a.pos.y - height + offset;
                while yy <= a.pos.y + offset {
                    let yo = a.pos.y - yy;
                    let smooth = leaf_radius + a.radius_offset + crate::mth::floor_f32(yo as f32 / height as f32 * 3.5);
                    let jagged = if yo > 0 && smooth == prev && yy & 1 == 0 { smooth + 1 } else { smooth };
                    self.row(ctx, random, sets, tree, BlockPos::new(a.pos.x, yy, a.pos.z), jagged, 0, double);
                    prev = smooth;
                    yy += 1;
                }
            }
            Kind::Pine { .. } => {
                let mut r = 0;
                let mut yo = offset;
                while yo >= offset - height {
                    self.row(ctx, random, sets, tree, a.pos, r, yo, double);
                    if r >= 1 && yo == offset - height + 1 {
                        r -= 1;
                    } else if r < leaf_radius + a.radius_offset {
                        r += 1;
                    }
                    yo -= 1;
                }
            }
            Kind::Poplar { side_hole, .. } => self.poplar(ctx, random, sets, tree, a, offset, height, leaf_radius, *side_hole),
            Kind::RandomSpread { attempts, .. } => {
                for _ in 0..*attempts {
                    let dx = random.next_i32_bound(leaf_radius) - random.next_i32_bound(leaf_radius);
                    let dy = random.next_i32_bound(foliage_height) - random.next_i32_bound(foliage_height);
                    let dz = random.next_i32_bound(leaf_radius) - random.next_i32_bound(leaf_radius);
                    try_place_leaf(ctx, random, sets, tree, a.pos.offset(dx, dy, dz));
                }
            }
            Kind::Spruce { .. } => {
                let mut r = random.next_i32_bound(2);
                let (mut max_r, mut min_r) = (1, 0);
                let mut yo = offset;
                while yo >= -height {
                    self.row(ctx, random, sets, tree, a.pos, r, yo, double);
                    if r >= max_r {
                        r = min_r;
                        min_r = 1;
                        max_r = (max_r + 1).min(leaf_radius + a.radius_offset);
                    } else {
                        r += 1;
                    }
                    yo -= 1;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn poplar(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, a: Attachment, offset: i32, height: i32, leaf_radius: i32, side_hole: f32) {
        let double = a.double_trunk();
        let pos = a.pos.offset(0, offset, 0);
        let r = leaf_radius + a.radius_offset - 1;
        let flip = random.next_bool();
        let rows = [(r - 2, height - 1), (r - 1, height - 2), (r - 1, height - 3)];
        for (radius, y) in rows {
            poplar_row(ctx, random, sets, tree, pos, radius, y, double, height, flip, side_hole);
        }
        let mut y = height - 4;
        while y >= 1 {
            poplar_row(ctx, random, sets, tree, pos, r, y, double, height, flip, side_hole);
            y -= 1;
        }
        // replaceLeavesWithLog
        let off = i32::from(double);
        let y = height - 4;
        for dx in -r..=r + off {
            for dz in -r..=r + off {
                let (adz, adx) = (dz.abs(), dx.abs());
                let cut = poplar_corner_cut(dx, dz, r, poplar_partial_row(height, y), flip);
                if within_rhombus(r, adx, adz, cut, 2) && (adz == 0 && r - adx >= 4 || adx == 0 && r - adz >= 4) {
                    let p = pos.offset(dx, y, dz);
                    let axis = if adz == 0 { Axis::X } else { Axis::Z };
                    let foliage = tree.foliage_provider.get(ctx, random, p);
                    if ctx.block(p) == foliage {
                        let log = tree.trunk_provider.get(ctx, random, p);
                        let log = try_with(ctx.registries(), log, "axis", axis.name());
                        sets.set_foliage(ctx, p, log);
                    }
                }
            }
        }
        poplar_row(ctx, random, sets, tree, pos, r - 1, 0, double, height, flip, side_hole);
        poplar_row(ctx, random, sets, tree, pos, (r - 2).clamp(1, 2), -1, double, height, flip, side_hole);
    }
}

fn poplar_partial_row(height: i32, y: i32) -> bool {
    height - 1 == y || height - 2 == y
}

fn poplar_corner_cut(dx: i32, dz: i32, r: i32, partial: bool, flip: bool) -> i32 {
    let small = if flip { dx > 0 && dz > 0 || dz < 0 && dx < 0 } else { dx > 0 && dz < 0 || dz > 0 && dx < 0 };
    if small {
        r - 1
    } else if partial {
        r + 1
    } else {
        r
    }
}

fn within_rhombus(r: i32, adx: i32, adz: i32, cut: i32, extra: i32) -> bool {
    adx + adz <= r * 2 - (cut + extra)
}

#[allow(clippy::too_many_arguments)]
fn poplar_row(ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, origin: BlockPos, r: i32, y: i32, double: bool, height: i32, flip: bool, side_hole: f32) {
    let offset = i32::from(double);
    for dx in -r..=r + offset {
        for dz in -r..=r + offset {
            let partial = poplar_partial_row(height, y);
            let cut = poplar_corner_cut(dx, dz, r, partial, flip);
            let (adx, adz) = (dx.abs(), dz.abs());
            let edge = adx == r || adz == r;
            let skip = if partial && edge {
                true
            } else {
                let extra = i32::from(random.next_f32() <= side_hole);
                !within_rhombus(r, adx, adz, cut, extra)
            };
            if !skip {
                try_place_leaf(ctx, random, sets, tree, origin.offset(dx, y, dz));
            }
        }
    }
}

fn try_place_extension(ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, chance: f32, log: BlockPos, pos: BlockPos) -> bool {
    if pos.dist_manhattan(log) >= 7 {
        return false;
    }
    if random.next_f32() > chance {
        return false;
    }
    try_place_leaf(ctx, random, sets, tree, pos)
}

/// `FoliagePlacer.tryPlaceLeaf`.
pub fn try_place_leaf(ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, pos: BlockPos) -> bool {
    let current = ctx.block(pos);
    let persistent = ctx.property(current, "persistent") == Some("true");
    if persistent || !super::valid_tree_pos(ctx, pos) {
        return false;
    }
    let mut state = tree.foliage_provider.get(ctx, random, pos);
    if ctx.property(state, "waterlogged").is_some() {
        let water = ctx.fluid_at(pos) == FluidType::Water;
        state = ctx.with(state, "waterlogged", if water { "true" } else { "false" });
    }
    sets.set_foliage(ctx, pos, state);
    true
}
