//! Rails on the server level (26.3 `BaseRailBlock`, `RailBlock`,
//! `PoweredRailBlock` for powered and activator rails, and `RailState`'s
//! connection logic). Detector rails wait for minecarts.

use super::redstone::Kind;
use super::Level;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::{BlockId, BlockPos, BlockStateId, SupportType};

/// `RailShape` by its serialized name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    NorthSouth,
    EastWest,
    AscendingEast,
    AscendingWest,
    AscendingNorth,
    AscendingSouth,
    SouthEast,
    SouthWest,
    NorthWest,
    NorthEast,
}

impl Shape {
    fn parse(name: &str) -> Self {
        match name {
            "east_west" => Self::EastWest,
            "ascending_east" => Self::AscendingEast,
            "ascending_west" => Self::AscendingWest,
            "ascending_north" => Self::AscendingNorth,
            "ascending_south" => Self::AscendingSouth,
            "south_east" => Self::SouthEast,
            "south_west" => Self::SouthWest,
            "north_west" => Self::NorthWest,
            "north_east" => Self::NorthEast,
            _ => Self::NorthSouth,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::NorthSouth => "north_south",
            Self::EastWest => "east_west",
            Self::AscendingEast => "ascending_east",
            Self::AscendingWest => "ascending_west",
            Self::AscendingNorth => "ascending_north",
            Self::AscendingSouth => "ascending_south",
            Self::SouthEast => "south_east",
            Self::SouthWest => "south_west",
            Self::NorthWest => "north_west",
            Self::NorthEast => "north_east",
        }
    }

    fn is_slope(self) -> bool {
        matches!(self, Self::AscendingEast | Self::AscendingWest | Self::AscendingNorth | Self::AscendingSouth)
    }
}

/// `RailState`.
struct RailState {
    pos: BlockPos,
    state: BlockStateId,
    straight: bool,
    connections: Vec<BlockPos>,
}

impl Level<'_> {
    /// `BaseRailBlock.isRail`.
    fn is_rail_state(&self, state: BlockStateId) -> bool {
        matches!(self.redstone_kind(state), Some(Kind::Rail { .. })) && self.lib.registries.block_in_tag(state, self.rails_tag)
    }

    fn is_rail_at(&self, pos: BlockPos) -> bool {
        self.is_rail_state(self.block(pos))
    }

    fn rail_shape(&self, state: BlockStateId) -> Shape {
        Shape::parse(self.registries().blocks.property(state, "shape").unwrap_or("north_south"))
    }

    fn rail_state(&self, pos: BlockPos, state: BlockStateId) -> RailState {
        let straight = matches!(self.redstone_kind(state), Some(Kind::Rail { straight: true, .. }));
        let mut rail = RailState { pos, state, straight, connections: Vec::new() };
        rail.update_connections(self.rail_shape(state));
        rail
    }

    /// `RailState.getRail`: a rail at the position, above or below it.
    fn get_rail(&self, pos: BlockPos) -> Option<RailState> {
        for p in [pos, pos.above(), pos.below()] {
            let state = self.block(p);
            if self.is_rail_state(state) {
                return Some(self.rail_state(p, state));
            }
        }
        None
    }

    fn has_rail(&self, pos: BlockPos) -> bool {
        self.is_rail_at(pos) || self.is_rail_at(pos.above()) || self.is_rail_at(pos.below())
    }

    fn remove_soft_connections(&self, rail: &mut RailState) {
        let mut i = 0;
        while i < rail.connections.len() {
            match self.get_rail(rail.connections[i]) {
                Some(other) if other.has_connection(rail.pos) => {
                    rail.connections[i] = other.pos;
                    i += 1;
                }
                _ => {
                    rail.connections.remove(i);
                }
            }
        }
    }

    /// `RailState.countPotentialConnections`.
    fn count_potential_connections(&self, pos: BlockPos) -> usize {
        Direction::HORIZONTAL.into_iter().filter(|&d| self.has_rail(pos.relative(d, 1))).count()
    }

    /// `RailState.hasNeighborRail`.
    fn has_neighbor_rail(&self, rail: &RailState, pos: BlockPos) -> bool {
        let Some(mut neighbor) = self.get_rail(pos) else { return false };
        self.remove_soft_connections(&mut neighbor);
        neighbor.can_connect_to(rail)
    }

    /// Ascending variants when a rail sits one higher at an end.
    fn slope(&self, shape: Option<Shape>, pos: BlockPos) -> Option<Shape> {
        let mut shape = shape;
        if shape == Some(Shape::NorthSouth) {
            if self.is_rail_at(pos.north().above()) {
                shape = Some(Shape::AscendingNorth);
            }
            if self.is_rail_at(pos.south().above()) {
                shape = Some(Shape::AscendingSouth);
            }
        }
        if shape == Some(Shape::EastWest) {
            if self.is_rail_at(pos.east().above()) {
                shape = Some(Shape::AscendingEast);
            }
            if self.is_rail_at(pos.west().above()) {
                shape = Some(Shape::AscendingWest);
            }
        }
        shape
    }

    /// `RailState.connectTo`.
    fn connect_to(&mut self, rail: &mut RailState, other: BlockPos) {
        rail.connections.push(other);
        let pos = rail.pos;
        let (n, s, w, e) = (rail.has_connection(pos.north()), rail.has_connection(pos.south()), rail.has_connection(pos.west()), rail.has_connection(pos.east()));
        let mut shape = None;
        if n || s {
            shape = Some(Shape::NorthSouth);
        }
        if w || e {
            shape = Some(Shape::EastWest);
        }
        if !rail.straight {
            if s && e && !n && !w {
                shape = Some(Shape::SouthEast);
            }
            if s && w && !n && !e {
                shape = Some(Shape::SouthWest);
            }
            if n && w && !s && !e {
                shape = Some(Shape::NorthWest);
            }
            if n && e && !s && !w {
                shape = Some(Shape::NorthEast);
            }
        }
        let shape = self.slope(shape, pos).unwrap_or(Shape::NorthSouth);
        rail.state = self.with(rail.state, "shape", shape.name());
        self.set_block_and_update(pos, rail.state);
    }

    /// `RailState.place`.
    fn rail_place(&mut self, mut rail: RailState, has_signal: bool, first: bool, default: Shape) -> RailState {
        let pos = rail.pos;
        let n = self.has_neighbor_rail(&rail, pos.north());
        let s = self.has_neighbor_rail(&rail, pos.south());
        let w = self.has_neighbor_rail(&rail, pos.west());
        let e = self.has_neighbor_rail(&rail, pos.east());
        let mut shape = None;
        let (north_or_south, west_or_east) = (n || s, w || e);
        if north_or_south && !west_or_east {
            shape = Some(Shape::NorthSouth);
        }
        if west_or_east && !north_or_south {
            shape = Some(Shape::EastWest);
        }
        let (se, sw, ne, nw) = (s && e, s && w, n && e, n && w);
        if !rail.straight {
            if se && !n && !w {
                shape = Some(Shape::SouthEast);
            }
            if sw && !n && !e {
                shape = Some(Shape::SouthWest);
            }
            if nw && !s && !e {
                shape = Some(Shape::NorthWest);
            }
            if ne && !s && !w {
                shape = Some(Shape::NorthEast);
            }
        }
        if shape.is_none() {
            if north_or_south && west_or_east {
                shape = Some(default);
            } else if north_or_south {
                shape = Some(Shape::NorthSouth);
            } else if west_or_east {
                shape = Some(Shape::EastWest);
            }
            if !rail.straight {
                let order: [(bool, Shape); 4] = if has_signal {
                    [(se, Shape::SouthEast), (sw, Shape::SouthWest), (ne, Shape::NorthEast), (nw, Shape::NorthWest)]
                } else {
                    [(nw, Shape::NorthWest), (ne, Shape::NorthEast), (sw, Shape::SouthWest), (se, Shape::SouthEast)]
                };
                for (applies, corner) in order {
                    if applies {
                        shape = Some(corner);
                    }
                }
            }
        }
        let shape = self.slope(shape, pos).unwrap_or(default);
        rail.update_connections(shape);
        rail.state = self.with(rail.state, "shape", shape.name());
        if first || self.block(pos) != rail.state {
            self.set_block_and_update(pos, rail.state);
            let mut i = 0;
            while i < rail.connections.len() {
                if let Some(mut neighbor) = self.get_rail(rail.connections[i]) {
                    self.remove_soft_connections(&mut neighbor);
                    if neighbor.can_connect_to(&rail) {
                        let target = rail.pos;
                        self.connect_to(&mut neighbor, target);
                    }
                }
                i += 1;
            }
        }
        rail
    }

    /// `BaseRailBlock.updateDir`.
    fn rail_update_dir(&mut self, pos: BlockPos, state: BlockStateId, first: bool) -> BlockStateId {
        let current = self.rail_shape(state);
        let rail = self.rail_state(pos, state);
        let signal = self.has_neighbor_signal(pos);
        self.rail_place(rail, signal, first, current).state
    }

    /// `BaseRailBlock.onPlace` / `updateState(state, level, pos, movedByPiston)`.
    pub(super) fn rail_on_place(&mut self, state: BlockStateId, pos: BlockPos, old: BlockStateId, moved: bool) {
        if self.block_id(old) == self.block_id(state) {
            return;
        }
        let state = self.rail_update_dir(pos, state, true);
        if matches!(self.redstone_kind(state), Some(Kind::Rail { straight: true, .. })) {
            let _ = moved;
            let block = self.block_id(state);
            self.add_and_run(super::Update::Full { state, pos, block });
        }
    }

    fn can_support_rigid(&self, pos: BlockPos) -> bool {
        self.registries().blocks.is_face_sturdy(self.block(pos), Direction::Up, SupportType::Rigid)
    }

    /// `BaseRailBlock.neighborChanged`.
    pub(super) fn rail_neighbor_changed(&mut self, state: BlockStateId, pos: BlockPos, source: BlockId) {
        if self.block_id(self.block(pos)) != self.block_id(state) {
            return;
        }
        let shape = self.rail_shape(state);
        let removed = !self.can_support_rigid(pos.below())
            || match shape {
                Shape::AscendingEast => !self.can_support_rigid(pos.east()),
                Shape::AscendingWest => !self.can_support_rigid(pos.west()),
                Shape::AscendingNorth => !self.can_support_rigid(pos.north()),
                Shape::AscendingSouth => !self.can_support_rigid(pos.south()),
                _ => false,
            };
        if removed {
            self.drop_resources(state, pos);
            self.remove_block(pos, false);
            return;
        }
        match self.redstone_kind(state) {
            Some(Kind::Rail { straight: false, .. }) => {
                // `RailBlock.updateState`: a signal source nearby can flip a junction.
                let source_default = self.registries().blocks.block(source).default_state();
                if self.is_signal_source(source_default) && self.count_potential_connections(pos) == 3 {
                    self.rail_update_dir(pos, state, false);
                }
            }
            Some(Kind::Rail { powered: true, .. }) => self.powered_rail_update(state, pos),
            _ => {}
        }
    }

    /// `BaseRailBlock.affectNeighborsAfterRemoval`.
    pub(super) fn rail_after_removal(&mut self, state: BlockStateId, pos: BlockPos, moved: bool) {
        if moved {
            return;
        }
        let block = self.block_id(state);
        if self.rail_shape(state).is_slope() {
            self.update_neighbors_at(pos.above(), block);
        }
        if matches!(self.redstone_kind(state), Some(Kind::Rail { straight: true, .. })) {
            self.update_neighbors_at(pos, block);
            self.update_neighbors_at(pos.below(), block);
        }
    }

    /// `PoweredRailBlock.updateState`.
    fn powered_rail_update(&mut self, state: BlockStateId, pos: BlockPos) {
        let powered = self.registries().blocks.property(state, "powered") == Some("true");
        let should = self.has_neighbor_signal(pos) || self.find_powered_rail_signal(pos, state, true, 0) || self.find_powered_rail_signal(pos, state, false, 0);
        if should != powered {
            let next = self.with(state, "powered", if should { "true" } else { "false" });
            self.set_block_and_update(pos, next);
            let block = self.block_id(state);
            self.update_neighbors_at(pos.below(), block);
            if self.rail_shape(state).is_slope() {
                self.update_neighbors_at(pos.above(), block);
            }
        }
    }

    /// `PoweredRailBlock.findPoweredRailSignal`.
    fn find_powered_rail_signal(&self, pos: BlockPos, state: BlockStateId, forward: bool, depth: i32) -> bool {
        if depth >= 8 {
            return false;
        }
        let (mut x, mut y, mut z) = (pos.x, pos.y, pos.z);
        let mut check_below = true;
        let mut shape = self.rail_shape(state);
        match shape {
            Shape::NorthSouth => z += if forward { 1 } else { -1 },
            Shape::EastWest => x += if forward { -1 } else { 1 },
            Shape::AscendingEast => {
                if forward {
                    x -= 1;
                } else {
                    x += 1;
                    y += 1;
                    check_below = false;
                }
                shape = Shape::EastWest;
            }
            Shape::AscendingWest => {
                if forward {
                    x -= 1;
                    y += 1;
                    check_below = false;
                } else {
                    x += 1;
                }
                shape = Shape::EastWest;
            }
            Shape::AscendingNorth => {
                if forward {
                    z += 1;
                } else {
                    z -= 1;
                    y += 1;
                    check_below = false;
                }
                shape = Shape::NorthSouth;
            }
            Shape::AscendingSouth => {
                if forward {
                    z += 1;
                    y += 1;
                    check_below = false;
                } else {
                    z -= 1;
                }
                shape = Shape::NorthSouth;
            }
            _ => {}
        }
        let block = self.block_id(state);
        self.same_rail_with_power(BlockPos::new(x, y, z), block, forward, depth, shape)
            || check_below && self.same_rail_with_power(BlockPos::new(x, y - 1, z), block, forward, depth, shape)
    }

    /// `PoweredRailBlock.isSameRailWithPower`.
    fn same_rail_with_power(&self, pos: BlockPos, block: BlockId, forward: bool, depth: i32, direction: Shape) -> bool {
        let state = self.block(pos);
        if self.block_id(state) != block {
            return false;
        }
        let mine = self.rail_shape(state);
        if direction == Shape::EastWest && matches!(mine, Shape::NorthSouth | Shape::AscendingNorth | Shape::AscendingSouth) {
            return false;
        }
        if direction == Shape::NorthSouth && matches!(mine, Shape::EastWest | Shape::AscendingEast | Shape::AscendingWest) {
            return false;
        }
        if self.registries().blocks.property(state, "powered") != Some("true") {
            return false;
        }
        self.has_neighbor_signal(pos) || self.find_powered_rail_signal(pos, state, forward, depth + 1)
    }
}

impl RailState {
    fn update_connections(&mut self, shape: Shape) {
        let p = self.pos;
        self.connections = match shape {
            Shape::NorthSouth => vec![p.north(), p.south()],
            Shape::EastWest => vec![p.west(), p.east()],
            Shape::AscendingEast => vec![p.west(), p.east().above()],
            Shape::AscendingWest => vec![p.west().above(), p.east()],
            Shape::AscendingNorth => vec![p.north().above(), p.south()],
            Shape::AscendingSouth => vec![p.north(), p.south().above()],
            Shape::SouthEast => vec![p.east(), p.south()],
            Shape::SouthWest => vec![p.west(), p.south()],
            Shape::NorthWest => vec![p.west(), p.north()],
            Shape::NorthEast => vec![p.east(), p.north()],
        };
    }

    fn has_connection(&self, pos: BlockPos) -> bool {
        self.connections.iter().any(|c| c.x == pos.x && c.z == pos.z)
    }

    fn can_connect_to(&self, other: &RailState) -> bool {
        self.has_connection(other.pos) || self.connections.len() != 2
    }
}
