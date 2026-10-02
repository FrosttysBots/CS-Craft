//! Trunk placers (vanilla `feature.trunkplacers`).

use super::{Attachment, Sets, Tree};
use crate::feature::blocks::BlockSet;
use crate::feature::kinds::{float, int, int_or, shuffle};
use crate::feature::state::try_with;
use crate::feature::Ctx;
use crate::providers::IntProvider;
use minecraftoss_core::pos::{Axis, Direction};
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::{BlockPos, Registries};
use serde_json::Value;

#[derive(Debug)]
pub enum Kind {
    Straight,
    Forking,
    Giant,
    MegaJungle,
    DarkOak,
    Fancy,
    Bending { min_height_for_leaves: i32, bend_length: IntProvider },
    UpwardsBranching { extra_branch_steps: IntProvider, branch_probability: f32, extra_branch_length: IntProvider, can_grow_through: BlockSet },
    Cherry { branch_count: IntProvider, horizontal_length: IntProvider, start_offset: (i32, i32), end_offset: IntProvider },
    Poplar { height_above_branches: IntProvider, branch_amount: IntProvider },
}

#[derive(Debug)]
pub struct TrunkPlacer {
    base_height: i32,
    rand_a: i32,
    rand_b: i32,
    kind: Kind,
}

fn uniform_bounds(json: &Value) -> Result<(i32, i32), String> {
    // CherryTrunkPlacer reads a bare UniformInt map without a type.
    if json.get("type").is_none() {
        let get = |k: &str| json[k].as_i64().map(|v| v as i32).ok_or_else(|| format!("uniform lacks {k}"));
        return Ok((get("min_inclusive")?, get("max_inclusive")?));
    }
    match IntProvider::parse(json)? {
        IntProvider::Uniform(lo, hi) => Ok((lo, hi)),
        IntProvider::Constant(v) => Ok((v, v)),
        other => Err(format!("expected a uniform int, got {other:?}")),
    }
}

impl TrunkPlacer {
    pub fn parse(registries: &Registries, json: &Value) -> Result<Self, String> {
        let kind = json["type"].as_str().ok_or("trunk placer lacks a type")?;
        let kind = match kind.trim_start_matches("minecraft:") {
            "straight_trunk_placer" => Kind::Straight,
            "forking_trunk_placer" => Kind::Forking,
            "giant_trunk_placer" => Kind::Giant,
            "mega_jungle_trunk_placer" => Kind::MegaJungle,
            "dark_oak_trunk_placer" => Kind::DarkOak,
            "fancy_trunk_placer" => Kind::Fancy,
            "bending_trunk_placer" => Kind::Bending {
                min_height_for_leaves: int_or(json, "min_height_for_leaves", 1),
                bend_length: IntProvider::parse(&json["bend_length"])?,
            },
            "upwards_branching_trunk_placer" => Kind::UpwardsBranching {
                extra_branch_steps: IntProvider::parse(&json["extra_branch_steps"])?,
                branch_probability: float(json, "place_branch_per_log_probability")?,
                extra_branch_length: IntProvider::parse(&json["extra_branch_length"])?,
                can_grow_through: BlockSet::parse(registries, &json["can_grow_through"])?,
            },
            "cherry_trunk_placer" => Kind::Cherry {
                branch_count: IntProvider::parse(&json["branch_count"])?,
                horizontal_length: IntProvider::parse(&json["branch_horizontal_length"])?,
                start_offset: uniform_bounds(&json["branch_start_offset_from_top"])?,
                end_offset: IntProvider::parse(&json["branch_end_offset_from_top"])?,
            },
            "poplar_trunk_placer" => Kind::Poplar {
                height_above_branches: IntProvider::parse(&json["trunk_height_above_branches"])?,
                branch_amount: IntProvider::parse(&json["branch_amount"])?,
            },
            other => return Err(format!("unknown trunk placer {other}")),
        };
        Ok(Self { base_height: int(json, "base_height")?, rand_a: int(json, "height_rand_a")?, rand_b: int(json, "height_rand_b")?, kind })
    }

    pub fn tree_height(&self, random: &mut WorldgenRandom) -> i32 {
        self.base_height + random.next_i32_bound(self.rand_a + 1) + random.next_i32_bound(self.rand_b + 1)
    }

    /// `TrunkPlacer.validTreePos`, with the upwards-branching override.
    pub fn valid_tree_pos(&self, ctx: &Ctx, pos: BlockPos) -> bool {
        if super::valid_tree_pos(ctx, pos) {
            return true;
        }
        match &self.kind {
            Kind::UpwardsBranching { can_grow_through, .. } => can_grow_through.contains(ctx.registries(), ctx.block(pos)),
            _ => false,
        }
    }

    pub fn is_free(&self, ctx: &Ctx, pos: BlockPos) -> bool {
        self.valid_tree_pos(ctx, pos) || ctx.in_tag(ctx.block(pos), ctx.lib.tags.logs)
    }

    fn place_log_with(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, pos: BlockPos, axis: Option<Axis>) -> bool {
        if !self.valid_tree_pos(ctx, pos) {
            return false;
        }
        let mut state = tree.trunk_provider.get(ctx, random, pos);
        if let Some(axis) = axis {
            state = try_with(ctx.registries(), state, "axis", axis.name());
        }
        sets.set_trunk(ctx, pos, state);
        true
    }

    fn place_log(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, pos: BlockPos) -> bool {
        self.place_log_with(ctx, random, sets, tree, pos, None)
    }

    fn place_log_if_free(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, pos: BlockPos) {
        if self.is_free(ctx, pos) {
            self.place_log(ctx, random, sets, tree, pos);
        }
    }

    fn below_trunk(ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, pos: BlockPos) {
        if let Some(state) = tree.below_trunk_provider.get_optional(ctx, random, pos) {
            sets.set_trunk(ctx, pos, state);
        }
    }

    pub fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, height: i32, origin: BlockPos) -> Vec<Attachment> {
        match &self.kind {
            Kind::Straight => {
                Self::below_trunk(ctx, random, sets, tree, origin.below());
                for y in 0..height {
                    self.place_log(ctx, random, sets, tree, origin.offset(0, y, 0));
                }
                vec![Attachment::new(origin.offset(0, height, 0), 0, false)]
            }
            Kind::Forking => self.forking(ctx, random, sets, tree, height, origin),
            Kind::Giant => self.giant(ctx, random, sets, tree, height, origin),
            Kind::MegaJungle => {
                let mut out = self.giant(ctx, random, sets, tree, height, origin);
                let mut branch = height - 2 - random.next_i32_bound(4);
                while branch > height / 2 {
                    let angle = random.next_f32() * std::f32::consts::TAU;
                    let (mut bx, mut bz) = (0, 0);
                    for b in 0..5 {
                        bx = (1.5 + crate::providers::cos(f64::from(angle)) * b as f32) as i32;
                        bz = (1.5 + crate::providers::sin(f64::from(angle)) * b as f32) as i32;
                        self.place_log(ctx, random, sets, tree, origin.offset(bx, branch - 3 + b / 2, bz));
                    }
                    out.push(Attachment::new(origin.offset(bx, branch, bz), -2, false));
                    branch -= 2 + random.next_i32_bound(4);
                }
                out
            }
            Kind::DarkOak => self.dark_oak(ctx, random, sets, tree, height, origin),
            Kind::Fancy => self.fancy(ctx, random, sets, tree, height, origin),
            Kind::Bending { min_height_for_leaves, bend_length } => {
                let direction = Direction::HORIZONTAL[random.next_i32_bound(4) as usize];
                let log_height = height - 1;
                let mut pos = origin;
                Self::below_trunk(ctx, random, sets, tree, pos.below());
                let mut out = Vec::new();
                for i in 0..=log_height {
                    if i + 1 >= log_height + random.next_i32_bound(2) {
                        pos = pos.relative(direction, 1);
                    }
                    if super::valid_tree_pos(ctx, pos) {
                        self.place_log(ctx, random, sets, tree, pos);
                    }
                    if i >= *min_height_for_leaves {
                        out.push(Attachment::new(pos, 0, false));
                    }
                    pos = pos.above();
                }
                let length = bend_length.sample(random);
                for _ in 0..=length {
                    if super::valid_tree_pos(ctx, pos) {
                        self.place_log(ctx, random, sets, tree, pos);
                    }
                    out.push(Attachment::new(pos, 0, false));
                    pos = pos.relative(direction, 1);
                }
                out
            }
            Kind::UpwardsBranching { extra_branch_steps, branch_probability, extra_branch_length, .. } => {
                let mut out = Vec::new();
                for h in 0..height {
                    let y = origin.y + h;
                    let log = BlockPos::new(origin.x, y, origin.z);
                    if self.place_log(ctx, random, sets, tree, log) && h < height - 1 && random.next_f32() < *branch_probability {
                        let dir = Direction::HORIZONTAL[random.next_i32_bound(4) as usize];
                        let len = extra_branch_length.sample(random);
                        let branch_pos = (len - extra_branch_length.sample(random) - 1).max(0);
                        let steps = extra_branch_steps.sample(random);
                        self.upwards_branch(ctx, random, sets, tree, height, &mut out, log, y, dir, branch_pos, steps);
                    }
                    if h == height - 1 {
                        out.push(Attachment::new(BlockPos::new(origin.x, y + 1, origin.z), 0, false));
                    }
                }
                out
            }
            Kind::Cherry { branch_count, horizontal_length, start_offset, end_offset } => {
                Self::below_trunk(ctx, random, sets, tree, origin.below());
                let sample_uniform = |random: &mut WorldgenRandom, (lo, hi): (i32, i32)| lo + random.next_i32_bound(hi - lo + 1);
                let first = (height - 1 + sample_uniform(random, *start_offset)).max(0);
                let mut second = (height - 1 + sample_uniform(random, (start_offset.0, start_offset.1 - 1))).max(0);
                if second >= first {
                    second += 1;
                }
                let count = branch_count.sample(random);
                let middle = count == 3;
                let both = count >= 2;
                let trunk_height = if middle {
                    height
                } else if both {
                    first.max(second) + 1
                } else {
                    first + 1
                };
                for y in 0..trunk_height {
                    self.place_log(ctx, random, sets, tree, origin.offset(0, y, 0));
                }
                let mut out = Vec::new();
                if middle {
                    out.push(Attachment::new(origin.offset(0, trunk_height, 0), 0, false));
                }
                let direction = Direction::HORIZONTAL[random.next_i32_bound(4) as usize];
                let args = (horizontal_length, end_offset);
                out.push(self.cherry_branch(ctx, random, sets, tree, height, origin, direction, first, first < trunk_height - 1, args));
                if both {
                    out.push(self.cherry_branch(ctx, random, sets, tree, height, origin, direction.opposite(), second, second < trunk_height - 1, args));
                }
                out
            }
            Kind::Poplar { height_above_branches, branch_amount } => {
                Self::below_trunk(ctx, random, sets, tree, origin.below());
                let up_to_branches = height - height_above_branches.sample(random);
                for y in 0..height {
                    self.place_log(ctx, random, sets, tree, origin.offset(0, y, 0));
                    let mut all = Direction::ALL;
                    shuffle(&mut all, random);
                    let directions: Vec<Direction> = all.into_iter().filter(|d| d.is_horizontal()).collect();
                    if up_to_branches - 1 == y {
                        let branches = branch_amount.sample(random);
                        for &dir in directions.iter().take(branches.max(0) as usize) {
                            self.place_log_with(ctx, random, sets, tree, origin.offset(0, y, 0).relative(dir, 1), Some(dir.axis()));
                        }
                    }
                }
                vec![Attachment::new(origin.offset(0, up_to_branches, 0), 0, false)]
            }
        }
    }

    fn forking(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, height: i32, origin: BlockPos) -> Vec<Attachment> {
        Self::below_trunk(ctx, random, sets, tree, origin.below());
        let mut out = Vec::new();
        let lean = Direction::HORIZONTAL[random.next_i32_bound(4) as usize];
        let lean_height = height - random.next_i32_bound(4) - 1;
        let mut lean_steps = 3 - random.next_i32_bound(3);
        let (mut tx, mut tz) = (origin.x, origin.z);
        let mut ey = None;
        for yo in 0..height {
            let yy = origin.y + yo;
            if yo >= lean_height && lean_steps > 0 {
                let (dx, _, dz) = lean.offset();
                tx += dx;
                tz += dz;
                lean_steps -= 1;
            }
            if self.place_log(ctx, random, sets, tree, BlockPos::new(tx, yy, tz)) {
                ey = Some(yy + 1);
            }
        }
        if let Some(ey) = ey {
            out.push(Attachment::new(BlockPos::new(tx, ey, tz), 1, false));
        }
        let (mut tx, mut tz) = (origin.x, origin.z);
        let branch = Direction::HORIZONTAL[random.next_i32_bound(4) as usize];
        if branch != lean {
            let branch_pos = lean_height - random.next_i32_bound(2) - 1;
            let mut steps = 1 + random.next_i32_bound(3);
            let mut ey = None;
            let mut yo = branch_pos;
            while yo < height && steps > 0 {
                if yo >= 1 {
                    let yy = origin.y + yo;
                    let (dx, _, dz) = branch.offset();
                    tx += dx;
                    tz += dz;
                    if self.place_log(ctx, random, sets, tree, BlockPos::new(tx, yy, tz)) {
                        ey = Some(yy + 1);
                    }
                }
                yo += 1;
                steps -= 1;
            }
            if let Some(ey) = ey {
                out.push(Attachment::new(BlockPos::new(tx, ey, tz), 0, false));
            }
        }
        out
    }

    fn giant(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, height: i32, origin: BlockPos) -> Vec<Attachment> {
        let below = origin.below();
        for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            Self::below_trunk(ctx, random, sets, tree, below.offset(dx, 0, dz));
        }
        for h in 0..height {
            self.place_log_if_free(ctx, random, sets, tree, origin.offset(0, h, 0));
            if h < height - 1 {
                self.place_log_if_free(ctx, random, sets, tree, origin.offset(1, h, 0));
                self.place_log_if_free(ctx, random, sets, tree, origin.offset(1, h, 1));
                self.place_log_if_free(ctx, random, sets, tree, origin.offset(0, h, 1));
            }
        }
        vec![Attachment::new(origin.offset(0, height, 0), 0, true)]
    }

    fn dark_oak(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, height: i32, origin: BlockPos) -> Vec<Attachment> {
        let below = origin.below();
        for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            Self::below_trunk(ctx, random, sets, tree, below.offset(dx, 0, dz));
        }
        let lean = Direction::HORIZONTAL[random.next_i32_bound(4) as usize];
        let lean_height = height - random.next_i32_bound(4);
        let mut lean_steps = 2 - random.next_i32_bound(3);
        let (x, y, z) = (origin.x, origin.y, origin.z);
        let (mut tx, mut tz) = (x, z);
        let ey = y + height - 1;
        for dy in 0..height {
            if dy >= lean_height && lean_steps > 0 {
                let (dx, _, dz) = lean.offset();
                tx += dx;
                tz += dz;
                lean_steps -= 1;
            }
            let pos = BlockPos::new(tx, y + dy, tz);
            if super::is_air_or_leaves(ctx, pos) {
                self.place_log(ctx, random, sets, tree, pos);
                self.place_log(ctx, random, sets, tree, pos.offset(1, 0, 0));
                self.place_log(ctx, random, sets, tree, pos.offset(0, 0, 1));
                self.place_log(ctx, random, sets, tree, pos.offset(1, 0, 1));
            }
        }
        let mut out = vec![Attachment::new(BlockPos::new(tx, ey, tz), 0, true)];
        for ox in -1..=2 {
            for oz in -1..=2 {
                if (ox < 0 || ox > 1 || oz < 0 || oz > 1) && random.next_i32_bound(3) <= 0 {
                    let length = random.next_i32_bound(3) + 2;
                    for by in 0..length {
                        self.place_log(ctx, random, sets, tree, BlockPos::new(x + ox, ey - by - 1, z + oz));
                    }
                    out.push(Attachment::new(BlockPos::new(x + ox, ey, z + oz), 0, false));
                }
            }
        }
        out
    }

    fn fancy(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, tree_height: i32, origin: BlockPos) -> Vec<Attachment> {
        let height = tree_height + 2;
        let trunk_height = (f64::from(height) * 0.618).floor() as i32;
        Self::below_trunk(ctx, random, sets, tree, origin.below());
        let clusters_per_y = 1.min((1.382 + (f64::from(height) / 13.0).powf(2.0)).floor() as i32);
        let trunk_top = origin.y + trunk_height;
        let mut relative_y = height - 5;
        let mut coords: Vec<(Attachment, i32)> = vec![(Attachment::new(origin.offset(0, relative_y, 0), 0, false), trunk_top)];
        while relative_y >= 0 {
            let shape = fancy_tree_shape(height, relative_y);
            if shape >= 0.0 {
                for _ in 0..clusters_per_y {
                    let radius = f64::from(shape) * (f64::from(random.next_f32()) + 0.328);
                    let angle = f64::from(random.next_f32() * 2.0) * std::f64::consts::PI;
                    let x = radius * angle.sin() + 0.5;
                    let z = radius * angle.cos() + 0.5;
                    let check_start = origin.offset(x.floor() as i32, relative_y - 1, z.floor() as i32);
                    let check_end = check_start.offset(0, 5, 0);
                    if self.make_limb(ctx, random, sets, tree, check_start, check_end, false) {
                        let dx = origin.x - check_start.x;
                        let dz = origin.z - check_start.z;
                        let branch_height = f64::from(check_start.y) - f64::from(dx * dx + dz * dz).sqrt() * 0.381;
                        let branch_top = if branch_height > f64::from(trunk_top) { trunk_top } else { branch_height as i32 };
                        let base = BlockPos::new(origin.x, branch_top, origin.z);
                        if self.make_limb(ctx, random, sets, tree, base, check_start, false) {
                            coords.push((Attachment::new(check_start, 0, false), base.y));
                        }
                    }
                }
            }
            relative_y -= 1;
        }
        self.make_limb(ctx, random, sets, tree, origin, origin.offset(0, trunk_height, 0), true);
        // makeBranches
        for &(attachment, branch_base) in &coords {
            let base = BlockPos::new(origin.x, branch_base, origin.z);
            if base != attachment.pos && f64::from(branch_base - origin.y) >= f64::from(height) * 0.2 {
                self.make_limb(ctx, random, sets, tree, base, attachment.pos, true);
            }
        }
        coords.into_iter().filter(|&(_, base)| f64::from(base - origin.y) >= f64::from(height) * 0.2).map(|(a, _)| a).collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn make_limb(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, sets: &mut Sets, tree: &Tree, start: BlockPos, end: BlockPos, place: bool) -> bool {
        if !place && start == end {
            return true;
        }
        let delta = (end.x - start.x, end.y - start.y, end.z - start.z);
        let steps = delta.0.abs().max(delta.1.abs().max(delta.2.abs()));
        let dx = delta.0 as f32 / steps as f32;
        let dy = delta.1 as f32 / steps as f32;
        let dz = delta.2 as f32 / steps as f32;
        for i in 0..=steps {
            let pos = start.offset(
                crate::mth::floor_f32(0.5 + i as f32 * dx),
                crate::mth::floor_f32(0.5 + i as f32 * dy),
                crate::mth::floor_f32(0.5 + i as f32 * dz),
            );
            if place {
                let xd = (pos.x - start.x).abs();
                let zd = (pos.z - start.z).abs();
                let max = xd.max(zd);
                let axis = if max > 0 { if xd == max { Axis::X } else { Axis::Z } } else { Axis::Y };
                self.place_log_with(ctx, random, sets, tree, pos, Some(axis));
            } else if !self.is_free(ctx, pos) {
                return false;
            }
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn upwards_branch(
        &self,
        ctx: &mut Ctx,
        random: &mut WorldgenRandom,
        sets: &mut Sets,
        tree: &Tree,
        height: i32,
        out: &mut Vec<Attachment>,
        log: BlockPos,
        current: i32,
        dir: Direction,
        branch_pos: i32,
        mut steps: i32,
    ) {
        let mut along = current + branch_pos;
        let (mut lx, mut lz) = (log.x, log.z);
        let mut index = branch_pos;
        while index < height && steps > 0 {
            if index >= 1 {
                let y = current + index;
                let (dx, _, dz) = dir.offset();
                lx += dx;
                lz += dz;
                along = y;
                let pos = BlockPos::new(lx, y, lz);
                if self.place_log(ctx, random, sets, tree, pos) {
                    along += 1;
                }
                out.push(Attachment::new(pos, 0, false));
            }
            index += 1;
            steps -= 1;
        }
        if along - current > 1 {
            let foliage = BlockPos::new(lx, along, lz);
            out.push(Attachment::new(foliage, 0, false));
            out.push(Attachment::new(foliage.offset(0, -2, 0), 0, false));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn cherry_branch(
        &self,
        ctx: &mut Ctx,
        random: &mut WorldgenRandom,
        sets: &mut Sets,
        tree: &Tree,
        height: i32,
        origin: BlockPos,
        direction: Direction,
        offset: i32,
        middle_continues: bool,
        (horizontal_length, end_offset): (&IntProvider, &IntProvider),
    ) -> Attachment {
        let sideways = Some(direction.axis());
        let mut log = origin.offset(0, offset, 0);
        let end_y = height - 1 + end_offset.sample(random);
        let extend = middle_continues || end_y < offset;
        let distance = horizontal_length.sample(random) + i32::from(extend);
        let end = origin.relative(direction, distance).offset(0, end_y, 0);
        let horizontal = if extend { 2 } else { 1 };
        for _ in 0..horizontal {
            log = log.relative(direction, 1);
            self.place_log_with(ctx, random, sets, tree, log, sideways);
        }
        let vertical = if end.y > log.y { Direction::Up } else { Direction::Down };
        loop {
            let distance = log.dist_manhattan(end);
            if distance == 0 {
                return Attachment::new(end.above(), 0, false);
            }
            let chance = (end.y - log.y).abs() as f32 / distance as f32;
            let grow_vertically = random.next_f32() < chance;
            log = log.relative(if grow_vertically { vertical } else { direction }, 1);
            self.place_log_with(ctx, random, sets, tree, log, if grow_vertically { None } else { sideways });
        }
    }
}

/// `FancyTrunkPlacer.treeShape`.
fn fancy_tree_shape(height: i32, y: i32) -> f32 {
    if (y as f32) < height as f32 * 0.3 {
        return -1.0;
    }
    let radius = height as f32 / 2.0;
    let adjacent = radius - y as f32;
    let mut distance = (radius * radius - adjacent * adjacent).sqrt();
    if adjacent == 0.0 {
        distance = radius;
    } else if adjacent.abs() >= radius {
        return 0.0;
    }
    distance * 0.5
}
