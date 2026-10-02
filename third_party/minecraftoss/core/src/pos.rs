//! Block, section and chunk coordinates with vanilla packing.

/// A block position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub const fn chunk(self) -> ChunkPos {
        ChunkPos::new(self.x >> 4, self.z >> 4)
    }

    pub const fn section(self) -> SectionPos {
        SectionPos::new(self.x >> 4, self.y >> 4, self.z >> 4)
    }

    pub const fn offset(self, dx: i32, dy: i32, dz: i32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.z + dz)
    }

    /// `BlockPos.relative(direction, distance)`.
    /// `BlockPos.offset(Vec3i)`.
    pub const fn offset_pos(self, other: BlockPos) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    pub const fn relative(self, direction: Direction, distance: i32) -> Self {
        let (dx, dy, dz) = direction.offset();
        Self::new(self.x + dx * distance, self.y + dy * distance, self.z + dz * distance)
    }

    pub const fn above(self) -> Self {
        self.offset(0, 1, 0)
    }

    pub const fn below(self) -> Self {
        self.offset(0, -1, 0)
    }

    pub const fn north(self) -> Self {
        self.offset(0, 0, -1)
    }

    pub const fn south(self) -> Self {
        self.offset(0, 0, 1)
    }

    pub const fn west(self) -> Self {
        self.offset(-1, 0, 0)
    }

    pub const fn east(self) -> Self {
        self.offset(1, 0, 0)
    }

    pub const fn at_y(self, y: i32) -> Self {
        Self::new(self.x, y, self.z)
    }

    /// `Vec3i.distSqr`.
    pub fn dist_sqr(self, other: Self) -> f64 {
        let (dx, dy, dz) = (f64::from(self.x - other.x), f64::from(self.y - other.y), f64::from(self.z - other.z));
        dx * dx + dy * dy + dz * dz
    }

    /// `Vec3i.distManhattan`.
    pub const fn dist_manhattan(self, other: Self) -> i32 {
        (self.x - other.x).abs() + (self.y - other.y).abs() + (self.z - other.z).abs()
    }

    /// `BlockPos.asLong`.
    pub const fn pack(self) -> i64 {
        ((self.x as i64 & 0x3FF_FFFF) << 38) | ((self.z as i64 & 0x3FF_FFFF) << 12) | (self.y as i64 & 0xFFF)
    }
}

/// A chunk column position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkPos {
    pub x: i32,
    pub z: i32,
}

impl ChunkPos {
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    /// Vanilla `ChunkPos.pack`: low 32 bits X, high 32 bits Z.
    pub const fn pack(self) -> i64 {
        (self.x as i64 & 0xffff_ffff) | ((self.z as i64 & 0xffff_ffff) << 32)
    }

    pub const fn unpack(packed: i64) -> Self {
        Self::new(packed as i32, (packed >> 32) as i32)
    }

    pub const fn min_block_x(self) -> i32 {
        self.x << 4
    }

    pub const fn min_block_z(self) -> i32 {
        self.z << 4
    }

    /// Vanilla `ChunkPos.distanceSquared`.
    pub const fn distance_squared(self, other: Self) -> i32 {
        let dx = self.x - other.x;
        let dz = self.z - other.z;
        dx * dx + dz * dz
    }

    /// Chessboard distance, as used by chunk ticket level propagation.
    pub const fn chebyshev(self, other: Self) -> i32 {
        let dx = (self.x - other.x).abs();
        let dz = (self.z - other.z).abs();
        if dx > dz { dx } else { dz }
    }
}

/// A 16x16x16 section position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SectionPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl SectionPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub const fn chunk(self) -> ChunkPos {
        ChunkPos::new(self.x, self.z)
    }

    pub const fn origin(self) -> BlockPos {
        BlockPos::new(self.x << 4, self.y << 4, self.z << 4)
    }

    /// Vanilla `SectionPos.asLong`: 22 bits X, 22 bits Z, 20 bits Y.
    pub const fn pack(self) -> i64 {
        ((self.x as i64 & 0x3f_ffff) << 42)
            | (self.y as i64 & 0xf_ffff)
            | ((self.z as i64 & 0x3f_ffff) << 20)
    }

    pub const fn unpack(packed: i64) -> Self {
        Self::new(
            (packed >> 42) as i32,
            (packed << 44 >> 44) as i32,
            (packed << 22 >> 42) as i32,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_round_trips_including_negatives() {
        for (x, z) in [
            (0, 0),
            (-1, 5),
            (1_875_000, -1_875_000),
            (i32::MIN, i32::MAX),
        ] {
            assert_eq!(
                ChunkPos::unpack(ChunkPos::new(x, z).pack()),
                ChunkPos::new(x, z)
            );
        }
        for (x, y, z) in [(0, 0, 0), (-1, -4, 7), (1_875_000, 19, -1_875_000)] {
            assert_eq!(
                SectionPos::unpack(SectionPos::new(x, y, z).pack()),
                SectionPos::new(x, y, z)
            );
        }
        assert_eq!(
            BlockPos::new(-1, -65, 16).section(),
            SectionPos::new(-1, -5, 1)
        );
    }
}

/// The six axis directions in vanilla `Direction` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Direction {
    pub const ALL: [Self; 6] = [
        Self::Down,
        Self::Up,
        Self::North,
        Self::South,
        Self::West,
        Self::East,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn opposite(self) -> Self {
        match self {
            Self::Down => Self::Up,
            Self::Up => Self::Down,
            Self::North => Self::South,
            Self::South => Self::North,
            Self::West => Self::East,
            Self::East => Self::West,
        }
    }

    pub const fn offset(self) -> (i32, i32, i32) {
        match self {
            Self::Down => (0, -1, 0),
            Self::Up => (0, 1, 0),
            Self::North => (0, 0, -1),
            Self::South => (0, 0, 1),
            Self::West => (-1, 0, 0),
            Self::East => (1, 0, 0),
        }
    }

    /// `Direction.Plane.HORIZONTAL` order.
    pub const HORIZONTAL: [Self; 4] = [Self::North, Self::East, Self::South, Self::West];

    /// `Direction.Plane.VERTICAL` order.
    pub const VERTICAL: [Self; 2] = [Self::Up, Self::Down];

    /// `Direction.get2DDataValue` order (`BY_2D_DATA`: S, W, N, E).
    pub const BY_2D: [Self; 4] = [Self::South, Self::West, Self::North, Self::East];

    pub const fn axis(self) -> Axis {
        match self {
            Self::Down | Self::Up => Axis::Y,
            Self::North | Self::South => Axis::Z,
            Self::West | Self::East => Axis::X,
        }
    }

    pub const fn is_horizontal(self) -> bool {
        !matches!(self, Self::Down | Self::Up)
    }

    /// `Direction.getClockWise` (about the Y axis).
    pub const fn clockwise(self) -> Self {
        match self {
            Self::North => Self::East,
            Self::East => Self::South,
            Self::South => Self::West,
            Self::West => Self::North,
            other => other,
        }
    }

    /// `Direction.getCounterClockWise` (about the Y axis).
    pub const fn counter_clockwise(self) -> Self {
        match self {
            Self::North => Self::West,
            Self::West => Self::South,
            Self::South => Self::East,
            Self::East => Self::North,
            other => other,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Down => "down",
            Self::Up => "up",
            Self::North => "north",
            Self::South => "south",
            Self::West => "west",
            Self::East => "east",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "down" => Self::Down,
            "up" => Self::Up,
            "north" => Self::North,
            "south" => Self::South,
            "west" => Self::West,
            "east" => Self::East,
            _ => return None,
        })
    }

    /// `Direction.fromAxisAndDirection`.
    pub const fn from_axis(axis: Axis, positive: bool) -> Self {
        match (axis, positive) {
            (Axis::X, true) => Self::East,
            (Axis::X, false) => Self::West,
            (Axis::Y, true) => Self::Up,
            (Axis::Y, false) => Self::Down,
            (Axis::Z, true) => Self::South,
            (Axis::Z, false) => Self::North,
        }
    }
}

/// `Direction.Axis`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Self; 3] = [Self::X, Self::Y, Self::Z];

    pub const fn name(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
            Self::Z => "z",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "x" => Self::X,
            "y" => Self::Y,
            "z" => Self::Z,
            _ => return None,
        })
    }

    pub const fn is_horizontal(self) -> bool {
        !matches!(self, Self::Y)
    }
}
