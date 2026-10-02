//! Rotation and mirroring for structure templates: vanilla `Rotation`,
//! `Mirror`, `BoundingBox`, the position transforms of `StructureTemplate`
//! and every block's `rotate`/`mirror`.
//!
//! Source-informed from the pinned 26.3 JAR. Block transforms follow the
//! class that overrides `rotate`/`mirror` for each block (facing blocks,
//! stairs, doors, pillars, 16-step rotations, side properties, rails,
//! jigsaw orientation), including their asymmetries: anvils and pillars do
//! not mirror, and stairs mirror only along their facing axis.

use minecraftoss_core::pos::{Axis, Direction};
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::{BlockPos, BlockStateId, Registries};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rotation {
    None,
    Clockwise90,
    Clockwise180,
    Counterclockwise90,
}

impl Rotation {
    /// `Rotation.values()` order.
    pub const ALL: [Self; 4] = [Self::None, Self::Clockwise90, Self::Clockwise180, Self::Counterclockwise90];

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.trim_start_matches("minecraft:") {
            "none" => Self::None,
            "clockwise_90" => Self::Clockwise90,
            "180" => Self::Clockwise180,
            "counterclockwise_90" => Self::Counterclockwise90,
            _ => return None,
        })
    }

    /// `Rotation.getRotated`: this rotation followed by `other`.
    pub fn then(self, other: Self) -> Self {
        Self::ALL[(self as usize + other as usize) % 4]
    }

    /// `Rotation.getRandom`: `Util.getRandom(values(), random)`.
    pub fn random(random: &mut impl RandomSource) -> Self {
        Self::ALL[random.next_i32_bound(4) as usize]
    }

    pub fn rotate(self, direction: Direction) -> Direction {
        if !direction.is_horizontal() {
            return direction;
        }
        match self {
            Self::None => direction,
            Self::Clockwise90 => direction.clockwise(),
            Self::Clockwise180 => direction.opposite(),
            Self::Counterclockwise90 => direction.counter_clockwise(),
        }
    }

    /// `Rotation.rotate(int, int)` for 16-step rotation properties.
    pub fn rotate_steps(self, value: i32, steps: i32) -> i32 {
        match self {
            Self::None => value,
            Self::Clockwise90 => (value + steps / 4) % steps,
            Self::Clockwise180 => (value + steps / 2) % steps,
            Self::Counterclockwise90 => (value + steps * 3 / 4) % steps,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mirror {
    None,
    /// Inverts Z.
    LeftRight,
    /// Inverts X.
    FrontBack,
}

impl Mirror {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "none" => Self::None,
            "left_right" => Self::LeftRight,
            "front_back" => Self::FrontBack,
            _ => return None,
        })
    }

    /// `Mirror.mirror(int, int)`.
    pub fn mirror_steps(self, value: i32, steps: i32) -> i32 {
        let half = steps / 2;
        let corrected = if value > half { value - steps } else { value };
        match self {
            Self::None => value,
            Self::LeftRight => (half - corrected + steps) % steps,
            Self::FrontBack => (steps - corrected) % steps,
        }
    }

    /// `Mirror.getRotation`: a half turn when the mirror flips the direction's axis.
    pub fn rotation_for(self, direction: Direction) -> Rotation {
        match (self, direction.axis()) {
            (Self::LeftRight, Axis::Z) | (Self::FrontBack, Axis::X) => Rotation::Clockwise180,
            _ => Rotation::None,
        }
    }

    pub fn mirror(self, direction: Direction) -> Direction {
        match (self, direction.axis()) {
            (Self::FrontBack, Axis::X) | (Self::LeftRight, Axis::Z) => direction.opposite(),
            _ => direction,
        }
    }
}

/// `StructureTemplate.transform(BlockPos, Mirror, Rotation, BlockPos)`.
pub fn transform(pos: BlockPos, mirror: Mirror, rotation: Rotation, pivot: BlockPos) -> BlockPos {
    let (mut x, y, mut z) = (pos.x, pos.y, pos.z);
    match mirror {
        Mirror::LeftRight => z = -z,
        Mirror::FrontBack => x = -x,
        Mirror::None => {}
    }
    let (px, pz) = (pivot.x, pivot.z);
    match rotation {
        Rotation::Counterclockwise90 => BlockPos::new(px - pz + z, y, px + pz - x),
        Rotation::Clockwise90 => BlockPos::new(px + pz - z, y, pz - px + x),
        Rotation::Clockwise180 => BlockPos::new(px + px - x, y, pz + pz - z),
        Rotation::None => BlockPos::new(x, y, z),
    }
}

/// `StructureTemplate.transform(Vec3, Mirror, Rotation, BlockPos)`.
pub fn transform_vec(pos: [f64; 3], mirror: Mirror, rotation: Rotation, pivot: BlockPos) -> [f64; 3] {
    let [mut x, y, mut z] = pos;
    match mirror {
        Mirror::LeftRight => z = 1.0 - z,
        Mirror::FrontBack => x = 1.0 - x,
        Mirror::None => {}
    }
    let (px, pz) = (f64::from(pivot.x), f64::from(pivot.z));
    match rotation {
        Rotation::Counterclockwise90 => [px - pz + z, y, px + pz + 1.0 - x],
        Rotation::Clockwise90 => [px + pz + 1.0 - z, y, pz - px + x],
        Rotation::Clockwise180 => [px + px + 1.0 - x, y, pz + pz + 1.0 - z],
        Rotation::None => [x, y, z],
    }
}

/// `StructureTemplate.getZeroPositionWithTransform`.
pub fn zero_position_with_transform(zero: BlockPos, mirror: Mirror, rotation: Rotation, size_x: i32, size_z: i32) -> BlockPos {
    let (sx, sz) = (size_x - 1, size_z - 1);
    let mx = if mirror == Mirror::FrontBack { sx } else { 0 };
    let mz = if mirror == Mirror::LeftRight { sz } else { 0 };
    match rotation {
        Rotation::Counterclockwise90 => zero.offset(mz, 0, sx - mx),
        Rotation::Clockwise90 => zero.offset(sz - mz, 0, mx),
        Rotation::Clockwise180 => zero.offset(sx - mx, 0, sz - mz),
        Rotation::None => zero.offset(mx, 0, mz),
    }
}

/// Vanilla `BoundingBox`: inclusive block bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BoundingBox {
    pub min_x: i32,
    pub min_y: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub max_z: i32,
}

impl BoundingBox {
    /// The constructor, which orders inverted bounds.
    pub fn new(min_x: i32, min_y: i32, min_z: i32, max_x: i32, max_y: i32, max_z: i32) -> Self {
        Self {
            min_x: min_x.min(max_x),
            min_y: min_y.min(max_y),
            min_z: min_z.min(max_z),
            max_x: min_x.max(max_x),
            max_y: min_y.max(max_y),
            max_z: min_z.max(max_z),
        }
    }

    pub fn from_corners(a: BlockPos, b: BlockPos) -> Self {
        Self::new(a.x, a.y, a.z, b.x, b.y, b.z)
    }

    pub fn moved(self, dx: i32, dy: i32, dz: i32) -> Self {
        Self {
            min_x: self.min_x + dx,
            min_y: self.min_y + dy,
            min_z: self.min_z + dz,
            max_x: self.max_x + dx,
            max_y: self.max_y + dy,
            max_z: self.max_z + dz,
        }
    }

    pub fn is_inside(&self, pos: BlockPos) -> bool {
        pos.x >= self.min_x && pos.x <= self.max_x && pos.z >= self.min_z && pos.z <= self.max_z && pos.y >= self.min_y && pos.y <= self.max_y
    }

    pub fn intersects(&self, other: &Self) -> bool {
        self.max_x >= other.min_x
            && self.min_x <= other.max_x
            && self.max_z >= other.min_z
            && self.min_z <= other.max_z
            && self.max_y >= other.min_y
            && self.min_y <= other.max_y
    }

    /// `BoundingBox.forAllCorners` order.
    pub fn corners(&self) -> [BlockPos; 8] {
        let (x0, y0, z0, x1, y1, z1) = (self.min_x, self.min_y, self.min_z, self.max_x, self.max_y, self.max_z);
        [
            BlockPos::new(x1, y1, z1),
            BlockPos::new(x0, y1, z1),
            BlockPos::new(x1, y0, z1),
            BlockPos::new(x0, y0, z1),
            BlockPos::new(x1, y1, z0),
            BlockPos::new(x0, y1, z0),
            BlockPos::new(x1, y0, z0),
            BlockPos::new(x0, y0, z0),
        ]
    }

    pub fn infinite() -> Self {
        Self { min_x: i32::MIN, min_y: i32::MIN, min_z: i32::MIN, max_x: i32::MAX, max_y: i32::MAX, max_z: i32::MAX }
    }

    pub fn at(pos: BlockPos) -> Self {
        Self { min_x: pos.x, min_y: pos.y, min_z: pos.z, max_x: pos.x, max_y: pos.y, max_z: pos.z }
    }

    /// `intersects(minX, minZ, maxX, maxZ)`: the XZ footprint only.
    pub fn intersects_xz(&self, min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> bool {
        self.max_x >= min_x && self.min_x <= max_x && self.max_z >= min_z && self.min_z <= max_z
    }

    pub fn x_span(&self) -> i32 {
        self.max_x - self.min_x + 1
    }

    pub fn y_span(&self) -> i32 {
        self.max_y - self.min_y + 1
    }

    pub fn z_span(&self) -> i32 {
        self.max_z - self.min_z + 1
    }

    /// `BoundingBox.getCenter`.
    pub fn center(&self) -> BlockPos {
        BlockPos::new(
            self.min_x + (self.max_x - self.min_x + 1) / 2,
            self.min_y + (self.max_y - self.min_y + 1) / 2,
            self.min_z + (self.max_z - self.min_z + 1) / 2,
        )
    }

    pub fn inflated(&self, x: i32, y: i32, z: i32) -> Self {
        Self::new(self.min_x - x, self.min_y - y, self.min_z - z, self.max_x + x, self.max_y + y, self.max_z + z)
    }

    /// `BoundingBox.encapsulating(a, b)`.
    pub fn encapsulating(a: &Self, b: &Self) -> Self {
        Self::new(a.min_x.min(b.min_x), a.min_y.min(b.min_y), a.min_z.min(b.min_z), a.max_x.max(b.max_x), a.max_y.max(b.max_y), a.max_z.max(b.max_z))
    }

    /// The mutating `encapsulate(BlockPos)`.
    pub fn encapsulate_pos(&mut self, pos: BlockPos) {
        self.min_x = self.min_x.min(pos.x);
        self.min_y = self.min_y.min(pos.y);
        self.min_z = self.min_z.min(pos.z);
        self.max_x = self.max_x.max(pos.x);
        self.max_y = self.max_y.max(pos.y);
        self.max_z = self.max_z.max(pos.z);
    }

    /// `BoundingBox.encapsulatingBoxes`.
    pub fn encapsulating_all<'a>(boxes: impl IntoIterator<Item = &'a Self>) -> Option<Self> {
        let mut it = boxes.into_iter();
        let mut out = *it.next()?;
        for b in it {
            out = Self {
                min_x: out.min_x.min(b.min_x),
                min_y: out.min_y.min(b.min_y),
                min_z: out.min_z.min(b.min_z),
                max_x: out.max_x.max(b.max_x),
                max_y: out.max_y.max(b.max_y),
                max_z: out.max_z.max(b.max_z),
            };
        }
        Some(out)
    }

    /// `BoundingBox.orientBox`.
    #[allow(clippy::too_many_arguments)]
    pub fn orient_box(foot_x: i32, foot_y: i32, foot_z: i32, off_x: i32, off_y: i32, off_z: i32, width: i32, height: i32, depth: i32, direction: Direction) -> Self {
        match direction {
            Direction::North => Self::new(foot_x + off_x, foot_y + off_y, foot_z - depth + 1 + off_z, foot_x + width - 1 + off_x, foot_y + height - 1 + off_y, foot_z + off_z),
            Direction::West => Self::new(foot_x - depth + 1 + off_z, foot_y + off_y, foot_z + off_x, foot_x + off_z, foot_y + height - 1 + off_y, foot_z + width - 1 + off_x),
            Direction::East => Self::new(foot_x + off_z, foot_y + off_y, foot_z + off_x, foot_x + depth - 1 + off_z, foot_y + height - 1 + off_y, foot_z + width - 1 + off_x),
            _ => Self::new(foot_x + off_x, foot_y + off_y, foot_z + off_z, foot_x + width - 1 + off_x, foot_y + height - 1 + off_y, foot_z + depth - 1 + off_z),
        }
    }
}

/// Which `rotate`/`mirror` implementation a block uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Identity,
    /// `facing` rotates; mirroring turns it half around when the mirror
    /// flips its axis (`state.rotate(mirror.getRotation(facing))`).
    Facing,
    /// `AnvilBlock`: rotates but inherits the identity mirror.
    FacingNoMirror,
    Stairs,
    Door,
    /// `RotatedPillarBlock.rotatePillar` and `NetherPortalBlock`: X and Z
    /// swap on quarter turns; no mirror.
    Pillar,
    /// Banners, signs, hanging signs and skulls: 16 steps.
    Rotation16,
    /// Blocks with `north`/`east`/`south`/`west` properties that remap them
    /// (fences, panes, bars, walls, redstone wire, tripwire, vines, mossy
    /// carpet, huge mushroom blocks, multiface blocks).
    Sides,
    Rail,
    /// `JigsawBlock` and `CrafterBlock`: `FrontAndTop` through the octahedral group.
    Orientation,
}

fn classify(name: &str, has: impl Fn(&str) -> bool) -> Kind {
    match name {
        "minecraft:anvil" | "minecraft:chipped_anvil" | "minecraft:damaged_anvil" => Kind::FacingNoMirror,
        // FireBlock and ChorusPlantBlock keep the identity transforms.
        "minecraft:fire" | "minecraft:chorus_plant" => Kind::Identity,
        "minecraft:jigsaw" | "minecraft:crafter" => Kind::Orientation,
        _ if has("facing") && has("hinge") => Kind::Door,
        _ if has("facing") && has("shape") => Kind::Stairs,
        _ if has("facing") => Kind::Facing,
        _ if has("axis") => Kind::Pillar,
        _ if has("rotation") => Kind::Rotation16,
        _ if has("shape") => Kind::Rail,
        _ if has("north") && has("east") && has("south") && has("west") => Kind::Sides,
        _ => Kind::Identity,
    }
}

/// Every block state's rotated and mirrored states, precomputed.
pub struct StateTransforms {
    /// Per state: clockwise 90, 180, counterclockwise 90.
    rotated: Vec<[BlockStateId; 3]>,
    /// Per state: left-right, front-back.
    mirrored: Vec<[BlockStateId; 2]>,
}

impl StateTransforms {
    pub fn build(registries: &Registries) -> Self {
        let blocks = &registries.blocks;
        let count = blocks.state_count();
        let mut rotated = vec![[BlockStateId::AIR; 3]; count];
        let mut mirrored = vec![[BlockStateId::AIR; 2]; count];
        for (_, info) in blocks.blocks() {
            let kind = classify(info.name.as_str(), |p| info.properties().iter().any(|q| &*q.name == p));
            for state in info.states() {
                let t = Transformer { registries, kind };
                let i = state.0 as usize;
                rotated[i] = [
                    t.rotate(state, Rotation::Clockwise90),
                    t.rotate(state, Rotation::Clockwise180),
                    t.rotate(state, Rotation::Counterclockwise90),
                ];
                mirrored[i] = [t.mirror(state, Mirror::LeftRight), t.mirror(state, Mirror::FrontBack)];
            }
        }
        Self { rotated, mirrored }
    }

    /// `BlockState.rotate`.
    pub fn rotate(&self, state: BlockStateId, rotation: Rotation) -> BlockStateId {
        match rotation {
            Rotation::None => state,
            Rotation::Clockwise90 => self.rotated[state.0 as usize][0],
            Rotation::Clockwise180 => self.rotated[state.0 as usize][1],
            Rotation::Counterclockwise90 => self.rotated[state.0 as usize][2],
        }
    }

    /// `BlockState.mirror`.
    pub fn mirror(&self, state: BlockStateId, mirror: Mirror) -> BlockStateId {
        match mirror {
            Mirror::None => state,
            Mirror::LeftRight => self.mirrored[state.0 as usize][0],
            Mirror::FrontBack => self.mirrored[state.0 as usize][1],
        }
    }
}

struct Transformer<'r> {
    registries: &'r Registries,
    kind: Kind,
}

const SIDES: [Direction; 4] = [Direction::North, Direction::East, Direction::South, Direction::West];

impl Transformer<'_> {
    fn get(&self, state: BlockStateId, name: &str) -> Option<&str> {
        self.registries.blocks.property(state, name)
    }

    fn set(&self, state: BlockStateId, name: &str, value: &str) -> BlockStateId {
        self.registries.blocks.with_property(state, name, value).unwrap_or(state)
    }

    fn facing(&self, state: BlockStateId) -> Option<Direction> {
        self.get(state, "facing").and_then(Direction::from_name)
    }

    fn with_facing(&self, state: BlockStateId, f: impl Fn(Direction) -> Direction) -> BlockStateId {
        match self.facing(state) {
            Some(d) => self.set(state, "facing", f(d).name()),
            None => state,
        }
    }

    /// Remaps side properties: the new value on `map(side)` is the old value on `side`.
    fn map_sides(&self, state: BlockStateId, map: impl Fn(Direction) -> Direction) -> BlockStateId {
        let old: Vec<Option<String>> = SIDES.iter().map(|d| self.get(state, d.name()).map(str::to_owned)).collect();
        let mut out = state;
        for (side, value) in SIDES.iter().zip(&old) {
            if let Some(value) = value {
                out = self.set(out, map(*side).name(), value);
            }
        }
        out
    }

    fn rotate(&self, state: BlockStateId, rotation: Rotation) -> BlockStateId {
        if rotation == Rotation::None {
            return state;
        }
        match self.kind {
            Kind::Identity => state,
            Kind::Facing | Kind::FacingNoMirror | Kind::Stairs | Kind::Door => self.with_facing(state, |d| rotation.rotate(d)),
            Kind::Pillar => match (rotation, self.get(state, "axis")) {
                (Rotation::Clockwise90 | Rotation::Counterclockwise90, Some("x")) => self.set(state, "axis", "z"),
                (Rotation::Clockwise90 | Rotation::Counterclockwise90, Some("z")) => self.set(state, "axis", "x"),
                _ => state,
            },
            Kind::Rotation16 => match self.get(state, "rotation").and_then(|v| v.parse::<i32>().ok()) {
                Some(v) => self.set(state, "rotation", &rotation.rotate_steps(v, 16).to_string()),
                None => state,
            },
            Kind::Sides => self.map_sides(state, |d| rotation.rotate(d)),
            Kind::Rail => self.map_rail(state, |d| rotation.rotate(d)),
            Kind::Orientation => self.map_orientation(state, |d| rotation.rotate(d)),
        }
    }

    fn mirror(&self, state: BlockStateId, mirror: Mirror) -> BlockStateId {
        if mirror == Mirror::None {
            return state;
        }
        match self.kind {
            Kind::Identity | Kind::FacingNoMirror | Kind::Pillar => state,
            Kind::Facing => match self.facing(state) {
                Some(d) => self.rotate(state, mirror.rotation_for(d)),
                None => state,
            },
            Kind::Door => match self.facing(state) {
                Some(d) => {
                    let turned = self.rotate(state, mirror.rotation_for(d));
                    let hinge = if self.get(turned, "hinge") == Some("left") { "right" } else { "left" };
                    self.set(turned, "hinge", hinge)
                }
                None => state,
            },
            Kind::Stairs => self.mirror_stairs(state, mirror),
            Kind::Rotation16 => match self.get(state, "rotation").and_then(|v| v.parse::<i32>().ok()) {
                Some(v) => self.set(state, "rotation", &mirror.mirror_steps(v, 16).to_string()),
                None => state,
            },
            Kind::Sides => self.map_sides(state, |d| mirror.mirror(d)),
            Kind::Rail => self.map_rail(state, |d| mirror.mirror(d)),
            Kind::Orientation => self.map_orientation(state, |d| mirror.mirror(d)),
        }
    }

    /// `StairBlock.mirror`: only a mirror across the facing axis changes the
    /// state, and a front-back mirror keeps inner shapes as they are.
    fn mirror_stairs(&self, state: BlockStateId, mirror: Mirror) -> BlockStateId {
        let Some(facing) = self.facing(state) else { return state };
        let shape = self.get(state, "shape").unwrap_or("straight").to_owned();
        let turned = || self.rotate(state, Rotation::Clockwise180);
        match (mirror, facing.axis()) {
            (Mirror::LeftRight, Axis::Z) => match shape.as_str() {
                "outer_left" => self.set(turned(), "shape", "outer_right"),
                "inner_right" => self.set(turned(), "shape", "inner_left"),
                "inner_left" => self.set(turned(), "shape", "inner_right"),
                "outer_right" => self.set(turned(), "shape", "outer_left"),
                _ => turned(),
            },
            (Mirror::FrontBack, Axis::X) => match shape.as_str() {
                "outer_left" => self.set(turned(), "shape", "outer_right"),
                "inner_right" => self.set(turned(), "shape", "inner_right"),
                "inner_left" => self.set(turned(), "shape", "inner_left"),
                "outer_right" => self.set(turned(), "shape", "outer_left"),
                _ => turned(),
            },
            _ => state,
        }
    }

    /// `BaseRailBlock.rotate`/`mirror`: every rail shape transforms geometrically.
    fn map_rail(&self, state: BlockStateId, map: impl Fn(Direction) -> Direction) -> BlockStateId {
        let Some(shape) = self.get(state, "shape") else { return state };
        let new = if let Some(dir) = shape.strip_prefix("ascending_").and_then(Direction::from_name) {
            format!("ascending_{}", map(dir).name())
        } else {
            let mut parts = shape.split('_').filter_map(Direction::from_name);
            let (Some(a), Some(b)) = (parts.next(), parts.next()) else { return state };
            let (a, b) = (map(a), map(b));
            if a.axis() == b.axis() {
                if a.axis() == Axis::Z { "north_south".to_owned() } else { "east_west".to_owned() }
            } else {
                let (ns, ew) = if a.axis() == Axis::Z { (a, b) } else { (b, a) };
                format!("{}_{}", ns.name(), ew.name())
            }
        };
        self.set(state, "shape", &new)
    }

    /// `FrontAndTop` rotated by the octahedral group of a rotation or mirror.
    fn map_orientation(&self, state: BlockStateId, map: impl Fn(Direction) -> Direction) -> BlockStateId {
        let Some(value) = self.get(state, "orientation") else { return state };
        let mut parts = value.split('_').filter_map(Direction::from_name);
        let (Some(front), Some(top)) = (parts.next(), parts.next()) else { return state };
        let new = format!("{}_{}", map(front).name(), map(top).name());
        self.set(state, "orientation", &new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_steps_match_vanilla() {
        assert_eq!(Rotation::Clockwise90.rotate_steps(15, 16), 3);
        assert_eq!(Rotation::Counterclockwise90.rotate_steps(2, 16), 14);
        assert_eq!(Mirror::LeftRight.mirror_steps(0, 16), 8);
        assert_eq!(Mirror::LeftRight.mirror_steps(12, 16), 12);
        assert_eq!(Mirror::FrontBack.mirror_steps(4, 16), 12);
        assert_eq!(Mirror::FrontBack.mirror_steps(0, 16), 0);
    }

    #[test]
    fn positions_transform_like_vanilla() {
        let p = BlockPos::new(2, 5, 7);
        let zero = BlockPos::new(0, 0, 0);
        assert_eq!(transform(p, Mirror::None, Rotation::Clockwise90, zero), BlockPos::new(-7, 5, 2));
        assert_eq!(transform(p, Mirror::None, Rotation::Counterclockwise90, zero), BlockPos::new(7, 5, -2));
        assert_eq!(transform(p, Mirror::LeftRight, Rotation::Clockwise180, zero), BlockPos::new(-2, 5, 7));
        assert_eq!(zero_position_with_transform(zero, Mirror::None, Rotation::Clockwise90, 3, 13), BlockPos::new(12, 0, 0));
    }
}
