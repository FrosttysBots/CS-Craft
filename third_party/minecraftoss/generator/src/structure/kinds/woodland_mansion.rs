//! Woodland mansions (vanilla `WoodlandMansionStructure`,
//! `WoodlandMansionPieces`).
//!
//! Source-informed from the pinned 26.3 JAR. An 11x11 floor plan grows
//! corridors from the entrance, rooms are cut from what is left, and the
//! outline, roofs and rooms become template pieces. Grid "y" runs along
//! the structure's south axis. After placing, cobblestone fills down from
//! the lowest floor to the ground under the mansion.

use super::template_piece::TemplatePiece;
use crate::feature::template::processor::Processor;
use crate::feature::template::transform::zero_position_with_transform;
use crate::feature::template::{BoundingBox, Mirror, PlaceSettings, Rotation};
use crate::feature::{Ctx, Library};
use crate::structure::piece::{create_chest, Piece, PieceBase};
use crate::structure::{GenerationContext, PieceList, StructureKind, Stub};
use minecraftoss_core::block::flags;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::{LegacyRandom, RandomSource, WorldgenRandom};
use minecraftoss_core::{BlockPos, ChunkPos};

#[derive(Debug)]
pub struct WoodlandMansion;

impl StructureKind for WoodlandMansion {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        let (x, z) = (ctx.chunk.min_block_x() + 7, ctx.chunk.min_block_z() + 7);
        if !ctx.could_valid_biome_exist_in_terrain_column(x, z) {
            return None;
        }
        let rotation = Rotation::random(&mut ctx.random);
        // `getLowestYIn5by5Box`.
        let (ox, oz) = match rotation {
            Rotation::None => (5, 5),
            Rotation::Clockwise90 => (-5, 5),
            Rotation::Clockwise180 => (-5, -5),
            Rotation::Counterclockwise90 => (5, -5),
        };
        let start = BlockPos::new(x, ctx.lowest_y_at(x, z, ox, oz), z);
        if start.y < 60 {
            return None;
        }
        Some(Stub::deferred(start, move |ctx: &mut GenerationContext| {
            let grid = MansionGrid::new(&mut ctx.random);
            let mut placer = Placer { lib: ctx.lib, random: &mut ctx.random, start_x: 0, start_y: 0, pieces: Vec::new() };
            placer.create_mansion(start, rotation, &grid);
            placer.pieces
        }))
    }

    /// `WoodlandMansionStructure.afterPlace`: cobblestone under the mansion.
    fn after_place(&self, ctx: &mut Ctx, _random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, pieces: &[Box<dyn Piece>]) {
        let Some(bounds) = BoundingBox::encapsulating_all(pieces.iter().map(|p| &p.base().bbox)) else { return };
        let cobblestone = ctx.registries().blocks.parse_state("minecraft:cobblestone").expect("cobblestone");
        let (min_y, y_start) = (ctx.min_y(), bounds.min_y);
        for x in chunk_bb.min_x..=chunk_bb.max_x {
            for z in chunk_bb.min_z..=chunk_bb.max_z {
                let pos = BlockPos::new(x, y_start, z);
                if ctx.is_air(ctx.block(pos)) || !bounds.is_inside(pos) || !pieces.iter().any(|p| p.base().bbox.is_inside(pos)) {
                    continue;
                }
                let mut y = y_start - 1;
                while y > min_y {
                    let below = BlockPos::new(x, y, z);
                    let state = ctx.block(below);
                    if !ctx.is_air(state) && !ctx.registries().blocks.is(state, flags::LIQUID) {
                        break;
                    }
                    ctx.set_block(below, cobblestone);
                    y -= 1;
                }
            }
        }
    }
}

/// `WoodlandMansionPieces.SimpleGrid`: indexed `[x][y]`, a fixed value outside.
#[derive(Clone, Debug)]
struct SimpleGrid {
    width: i32,
    height: i32,
    outside: i32,
    cells: Vec<i32>,
}

impl SimpleGrid {
    fn new(width: i32, height: i32, outside: i32) -> Self {
        Self { width, height, outside, cells: vec![0; (width * height) as usize] }
    }

    fn inside(&self, x: i32, y: i32) -> bool {
        x >= 0 && x < self.width && y >= 0 && y < self.height
    }

    fn set(&mut self, x: i32, y: i32, value: i32) {
        if self.inside(x, y) {
            self.cells[(x * self.height + y) as usize] = value;
        }
    }

    fn set_box(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, value: i32) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.set(x, y, value);
            }
        }
    }

    fn get(&self, x: i32, y: i32) -> i32 {
        if self.inside(x, y) { self.cells[(x * self.height + y) as usize] } else { self.outside }
    }

    fn set_if(&mut self, x: i32, y: i32, if_value: i32, value: i32) {
        if self.get(x, y) == if_value {
            self.set(x, y, value);
        }
    }

    fn edges_to(&self, x: i32, y: i32, value: i32) -> bool {
        self.get(x - 1, y) == value || self.get(x + 1, y) == value || self.get(x, y + 1) == value || self.get(x, y - 1) == value
    }
}

const CORRIDOR: i32 = 1;
const ROOM: i32 = 2;
const START_ROOM: i32 = 3;
const BLOCKED: i32 = 5;
const ROOM_1X1: i32 = 65536;
const ROOM_1X2: i32 = 131072;
const ROOM_2X2: i32 = 262144;
const ROOM_ORIGIN_FLAG: i32 = 1048576;
const ROOM_DOOR_FLAG: i32 = 2097152;
const ROOM_STAIRS_FLAG: i32 = 4194304;
const ROOM_CORRIDOR_FLAG: i32 = 8388608;
const ROOM_TYPE_MASK: i32 = 983040;
const ROOM_ID_MASK: i32 = 65535;

/// Horizontal steps as (x, grid y) — a direction's x and z steps.
fn step(direction: Direction) -> (i32, i32) {
    let (x, _, z) = direction.offset();
    (x, z)
}

/// `Direction.from2DDataValue`.
fn from_2d(value: i32) -> Direction {
    Direction::BY_2D[value as usize]
}

fn is_house(grid: &SimpleGrid, x: i32, y: i32) -> bool {
    matches!(grid.get(x, y), 1..=4)
}

/// `WoodlandMansionPieces.MansionGrid`.
struct MansionGrid {
    base: SimpleGrid,
    third: SimpleGrid,
    floors: [SimpleGrid; 3],
    entrance_x: i32,
    entrance_y: i32,
}

impl MansionGrid {
    fn new(random: &mut LegacyRandom) -> Self {
        let (ex, ey) = (7, 4);
        let mut base = SimpleGrid::new(11, 11, BLOCKED);
        base.set_box(ex, ey, ex + 1, ey + 1, START_ROOM);
        base.set_box(ex - 1, ey, ex - 1, ey + 1, ROOM);
        base.set_box(ex + 2, ey - 2, ex + 3, ey + 3, BLOCKED);
        base.set_box(ex + 1, ey - 2, ex + 1, ey - 1, CORRIDOR);
        base.set_box(ex + 1, ey + 2, ex + 1, ey + 3, CORRIDOR);
        base.set(ex - 1, ey - 1, CORRIDOR);
        base.set(ex - 1, ey + 2, CORRIDOR);
        base.set_box(0, 0, 11, 1, BLOCKED);
        base.set_box(0, 9, 11, 11, BLOCKED);
        recursive_corridor(random, &mut base, ex, ey - 2, Direction::West, 6);
        recursive_corridor(random, &mut base, ex, ey + 3, Direction::West, 6);
        recursive_corridor(random, &mut base, ex - 2, ey - 1, Direction::West, 3);
        recursive_corridor(random, &mut base, ex - 2, ey + 2, Direction::West, 3);
        while clean_edges(&mut base) {}
        let mut floors = [SimpleGrid::new(11, 11, BLOCKED), SimpleGrid::new(11, 11, BLOCKED), SimpleGrid::new(11, 11, BLOCKED)];
        identify_rooms(random, &base, &mut floors[0]);
        identify_rooms(random, &base, &mut floors[1]);
        floors[0].set_box(ex + 1, ey, ex + 1, ey + 1, ROOM_CORRIDOR_FLAG);
        floors[1].set_box(ex + 1, ey, ex + 1, ey + 1, ROOM_CORRIDOR_FLAG);
        let third = SimpleGrid::new(base.width, base.height, BLOCKED);
        let mut grid = Self { base, third, floors, entrance_x: ex, entrance_y: ey };
        grid.setup_third_floor(random);
        let third = grid.third.clone();
        identify_rooms(random, &third, &mut grid.floors[2]);
        grid
    }

    fn is_room_id(&self, x: i32, y: i32, floor: usize, room_id: i32) -> bool {
        self.floors[floor].get(x, y) & ROOM_ID_MASK == room_id
    }

    fn room_1x2_direction(&self, x: i32, y: i32, floor: usize, room_id: i32) -> Option<Direction> {
        Direction::HORIZONTAL.into_iter().find(|&d| {
            let (dx, dy) = step(d);
            self.is_room_id(x + dx, y + dy, floor, room_id)
        })
    }

    /// `setupThirdFloor`.
    fn setup_third_floor(&mut self, random: &mut LegacyRandom) {
        let mut potential = Vec::new();
        for y in 0..self.third.height {
            for x in 0..self.third.width {
                let data = self.floors[1].get(x, y);
                if data & ROOM_TYPE_MASK == ROOM_1X2 && data & ROOM_DOOR_FLAG == ROOM_DOOR_FLAG {
                    potential.push((x, y));
                }
            }
        }
        let (w, h) = (self.third.width, self.third.height);
        if potential.is_empty() {
            self.third.set_box(0, 0, w, h, BLOCKED);
            return;
        }
        let (rx, ry) = potential[random.next_i32_bound(potential.len() as i32) as usize];
        let data = self.floors[1].get(rx, ry);
        self.floors[1].set(rx, ry, data | ROOM_STAIRS_FLAG);
        let dir = self.room_1x2_direction(rx, ry, 1, data & ROOM_ID_MASK).expect("1x2 room has a second cell");
        let (dx, dy) = step(dir);
        let (end_x, end_y) = (rx + dx, ry + dy);
        for y in 0..h {
            for x in 0..w {
                if !is_house(&self.base, x, y) {
                    self.third.set(x, y, BLOCKED);
                } else if x == rx && y == ry {
                    self.third.set(x, y, START_ROOM);
                } else if x == end_x && y == end_y {
                    self.third.set(x, y, START_ROOM);
                    self.floors[2].set(x, y, ROOM_CORRIDOR_FLAG);
                }
            }
        }
        let corridors: Vec<Direction> = Direction::HORIZONTAL
            .into_iter()
            .filter(|&d| {
                let (dx, dy) = step(d);
                self.third.get(end_x + dx, end_y + dy) == 0
            })
            .collect();
        if corridors.is_empty() {
            self.third.set_box(0, 0, w, h, BLOCKED);
            self.floors[1].set(rx, ry, data);
        } else {
            let dir = corridors[random.next_i32_bound(corridors.len() as i32) as usize];
            let (dx, dy) = step(dir);
            recursive_corridor(random, &mut self.third, end_x + dx, end_y + dy, dir, 4);
            while clean_edges(&mut self.third) {}
        }
    }
}

/// `MansionGrid.recursiveCorridor`.
fn recursive_corridor(random: &mut LegacyRandom, grid: &mut SimpleGrid, x: i32, y: i32, heading: Direction, depth: i32) {
    if depth <= 0 {
        return;
    }
    let (hx, hy) = step(heading);
    grid.set(x, y, CORRIDOR);
    grid.set_if(x + hx, y + hy, 0, CORRIDOR);
    for _ in 0..8 {
        let next = from_2d(random.next_i32_bound(4));
        if next != heading.opposite() && (next != Direction::East || !random.next_bool()) {
            let (nx, ny) = (x + hx, y + hy);
            let (sx, sy) = step(next);
            if grid.get(nx + sx, ny + sy) == 0 && grid.get(nx + sx * 2, ny + sy * 2) == 0 {
                recursive_corridor(random, grid, x + hx + sx, y + hy + sy, next, depth - 1);
                break;
            }
        }
    }
    let (cx, cy) = step(heading.clockwise());
    let (ccx, ccy) = step(heading.counter_clockwise());
    grid.set_if(x + cx, y + cy, 0, ROOM);
    grid.set_if(x + ccx, y + ccy, 0, ROOM);
    grid.set_if(x + hx + cx, y + hy + cy, 0, ROOM);
    grid.set_if(x + hx + ccx, y + hy + ccy, 0, ROOM);
    grid.set_if(x + hx * 2, y + hy * 2, 0, ROOM);
    grid.set_if(x + cx * 2, y + cy * 2, 0, ROOM);
    grid.set_if(x + ccx * 2, y + ccy * 2, 0, ROOM);
}

/// `MansionGrid.cleanEdges`.
fn clean_edges(grid: &mut SimpleGrid) -> bool {
    let mut touched = false;
    for y in 0..grid.height {
        for x in 0..grid.width {
            if grid.get(x, y) != 0 {
                continue;
            }
            let direct = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().filter(|(dx, dy)| is_house(grid, x + dx, y + dy)).count();
            if direct >= 3 {
                grid.set(x, y, ROOM);
                touched = true;
            } else if direct == 2 {
                let diagonal = [(1, 1), (-1, 1), (1, -1), (-1, -1)].iter().filter(|(dx, dy)| is_house(grid, x + dx, y + dy)).count();
                if diagonal <= 1 {
                    grid.set(x, y, ROOM);
                    touched = true;
                }
            }
        }
    }
    touched
}

/// `MansionGrid.identifyRooms`.
fn identify_rooms(random: &mut LegacyRandom, from: &SimpleGrid, rooms: &mut SimpleGrid) {
    let mut positions = Vec::new();
    for y in 0..from.height {
        for x in 0..from.width {
            if from.get(x, y) == ROOM {
                positions.push((x, y));
            }
        }
    }
    // `Util.shuffle`.
    for i in (2..=positions.len()).rev() {
        let swap_to = random.next_i32_bound(i as i32) as usize;
        positions.swap(i - 1, swap_to);
    }
    let mut room_id = 10;
    for (x, y) in positions {
        if rooms.get(x, y) != 0 {
            continue;
        }
        let (mut x0, mut x1, mut y0, mut y1) = (x, x, y, y);
        let free = |dx: i32, dy: i32| rooms.get(x + dx, y + dy) == 0 && from.get(x + dx, y + dy) == ROOM;
        let mut kind = ROOM_1X1;
        if free(1, 0) && free(0, 1) && free(1, 1) {
            x1 += 1;
            y1 += 1;
            kind = ROOM_2X2;
        } else if free(-1, 0) && free(0, 1) && free(-1, 1) {
            x0 -= 1;
            y1 += 1;
            kind = ROOM_2X2;
        } else if free(-1, 0) && free(0, -1) && free(-1, -1) {
            x0 -= 1;
            y0 -= 1;
            kind = ROOM_2X2;
        } else if free(1, 0) {
            x1 += 1;
            kind = ROOM_1X2;
        } else if free(0, 1) {
            y1 += 1;
            kind = ROOM_1X2;
        } else if free(-1, 0) {
            x0 -= 1;
            kind = ROOM_1X2;
        } else if free(0, -1) {
            y0 -= 1;
            kind = ROOM_1X2;
        }
        let mut door_x = if random.next_bool() { x0 } else { x1 };
        let mut door_y = if random.next_bool() { y0 } else { y1 };
        let mut door_flag = ROOM_DOOR_FLAG;
        if !from.edges_to(door_x, door_y, CORRIDOR) {
            door_x = if door_x == x0 { x1 } else { x0 };
            door_y = if door_y == y0 { y1 } else { y0 };
            if !from.edges_to(door_x, door_y, CORRIDOR) {
                door_y = if door_y == y0 { y1 } else { y0 };
                if !from.edges_to(door_x, door_y, CORRIDOR) {
                    door_x = if door_x == x0 { x1 } else { x0 };
                    door_y = if door_y == y0 { y1 } else { y0 };
                    if !from.edges_to(door_x, door_y, CORRIDOR) {
                        door_flag = 0;
                        door_x = x0;
                        door_y = y0;
                    }
                }
            }
        }
        for ry in y0..=y1 {
            for rx in x0..=x1 {
                if rx == door_x && ry == door_y {
                    rooms.set(rx, ry, ROOM_ORIGIN_FLAG | door_flag | kind | room_id);
                } else {
                    rooms.set(rx, ry, kind | room_id);
                }
            }
        }
        room_id += 1;
    }
}

/// `FloorRoomCollection`: the first floor, or the second and third.
#[derive(Clone, Copy)]
struct Rooms {
    first: bool,
}

impl Rooms {
    fn one_by_one(self, random: &mut LegacyRandom) -> String {
        if self.first {
            format!("1x1_a{}", random.next_i32_bound(5) + 1)
        } else {
            format!("1x1_b{}", random.next_i32_bound(5) + 1)
        }
    }

    fn one_by_one_secret(self, random: &mut LegacyRandom) -> String {
        format!("1x1_as{}", random.next_i32_bound(4) + 1)
    }

    fn side_entrance(self, random: &mut LegacyRandom, stairs: bool) -> String {
        if self.first {
            format!("1x2_a{}", random.next_i32_bound(9) + 1)
        } else if stairs {
            "1x2_c_stairs".to_owned()
        } else {
            format!("1x2_c{}", random.next_i32_bound(4) + 1)
        }
    }

    fn front_entrance(self, random: &mut LegacyRandom, stairs: bool) -> String {
        if self.first {
            format!("1x2_b{}", random.next_i32_bound(5) + 1)
        } else if stairs {
            "1x2_d_stairs".to_owned()
        } else {
            format!("1x2_d{}", random.next_i32_bound(5) + 1)
        }
    }

    fn one_by_two_secret(self, random: &mut LegacyRandom) -> String {
        if self.first {
            format!("1x2_s{}", random.next_i32_bound(2) + 1)
        } else {
            format!("1x2_se{}", random.next_i32_bound(1) + 1)
        }
    }

    fn two_by_two(self, random: &mut LegacyRandom) -> String {
        if self.first {
            format!("2x2_a{}", random.next_i32_bound(4) + 1)
        } else {
            format!("2x2_b{}", random.next_i32_bound(5) + 1)
        }
    }
}

/// `PlacementData`.
#[derive(Clone, Copy)]
struct Walker {
    rotation: Rotation,
    position: BlockPos,
    wall: &'static str,
}

fn above(pos: BlockPos, n: i32) -> BlockPos {
    pos.offset(0, n, 0)
}

/// `BlockPos.rotate(Rotation)` about the origin.
fn rotate_pos(pos: BlockPos, rotation: Rotation) -> BlockPos {
    match rotation {
        Rotation::None => pos,
        Rotation::Clockwise90 => BlockPos::new(-pos.z, pos.y, pos.x),
        Rotation::Clockwise180 => BlockPos::new(-pos.x, pos.y, -pos.z),
        Rotation::Counterclockwise90 => BlockPos::new(pos.z, pos.y, -pos.x),
    }
}

/// `MansionPiecePlacer`.
struct Placer<'a> {
    lib: &'a Library,
    random: &'a mut LegacyRandom,
    start_x: i32,
    start_y: i32,
    pieces: PieceList,
}

impl Placer<'_> {
    fn add(&mut self, name: &str, position: BlockPos, rotation: Rotation) {
        self.add_mirrored(name, position, rotation, Mirror::None);
    }

    fn add_mirrored(&mut self, name: &str, position: BlockPos, rotation: Rotation, mirror: Mirror) {
        self.pieces.push(Box::new(MansionPiece::new(self.lib, name, position, rotation, mirror)));
    }

    /// The position of grid cell (x, y) relative to `origin`.
    fn cell(&self, origin: BlockPos, rotation: Rotation, x: i32, y: i32, east_extra: i32) -> BlockPos {
        let pos = origin.relative(rotation.rotate(Direction::South), 8 + (y - self.start_y) * 8);
        pos.relative(rotation.rotate(Direction::East), east_extra + (x - self.start_x) * 8)
    }

    fn create_mansion(&mut self, origin: BlockPos, rotation: Rotation, mansion: &MansionGrid) {
        let mut data = Walker { rotation, position: origin, wall: "wall_flat" };
        // `entrance`.
        let west = data.rotation.rotate(Direction::West);
        self.add("entrance", data.position.relative(west, 9), data.rotation);
        data.position = data.position.relative(data.rotation.rotate(Direction::South), 16);
        let mut second = Walker { rotation: data.rotation, position: above(data.position, 8), wall: "wall_window" };
        let (base, third) = (&mansion.base, &mansion.third);
        self.start_x = mansion.entrance_x + 1;
        self.start_y = mansion.entrance_y + 1;
        let (end_x, end_y) = (mansion.entrance_x + 1, mansion.entrance_y);
        let (sx, sy) = (self.start_x, self.start_y);
        self.traverse_outer_walls(&mut data, base, Direction::South, sx, sy, end_x, end_y);
        self.traverse_outer_walls(&mut second, base, Direction::South, sx, sy, end_x, end_y);
        let mut third_data = Walker { rotation: data.rotation, position: above(data.position, 19), wall: "wall_window" };
        'find: for y in 0..third.height {
            for x in (0..third.width).rev() {
                if is_house(third, x, y) {
                    third_data.position = third_data.position.relative(rotation.rotate(Direction::South), 8 + (y - self.start_y) * 8);
                    third_data.position = third_data.position.relative(rotation.rotate(Direction::East), (x - self.start_x) * 8);
                    self.traverse_wall_piece(&mut third_data);
                    self.traverse_outer_walls(&mut third_data, third, Direction::South, x, y, x, y);
                    break 'find;
                }
            }
        }
        self.create_roof(above(origin, 16), rotation, base, Some(third));
        self.create_roof(above(origin, 27), rotation, third, None);
        for floor in 0..3usize {
            let floor_origin = above(origin, 8 * floor as i32 + if floor == 2 { 3 } else { 0 });
            let rooms = &mansion.floors[floor];
            let grid = if floor == 2 { third } else { base };
            let collection = Rooms { first: floor == 0 };
            let (south_piece, west_piece) = if floor == 0 { ("carpet_south_1", "carpet_west_1") } else { ("carpet_south_2", "carpet_west_2") };
            let corridor_like = |x: i32, y: i32| grid.get(x, y) == CORRIDOR || rooms.get(x, y) & ROOM_CORRIDOR_FLAG == ROOM_CORRIDOR_FLAG;
            for y in 0..grid.height {
                for x in 0..grid.width {
                    if grid.get(x, y) != CORRIDOR {
                        continue;
                    }
                    let pos = self.cell(floor_origin, rotation, x, y, 0);
                    let r = |d: Direction| rotation.rotate(d);
                    self.add("corridor_floor", pos, rotation);
                    if corridor_like(x, y - 1) {
                        self.add("carpet_north", above(pos.relative(r(Direction::East), 1), 1), rotation);
                    }
                    if corridor_like(x + 1, y) {
                        self.add("carpet_east", above(pos.relative(r(Direction::South), 1).relative(r(Direction::East), 5), 1), rotation);
                    }
                    if corridor_like(x, y + 1) {
                        self.add(south_piece, pos.relative(r(Direction::South), 5).relative(r(Direction::West), 1), rotation);
                    }
                    if corridor_like(x - 1, y) {
                        self.add(west_piece, pos.relative(r(Direction::West), 1).relative(r(Direction::North), 1), rotation);
                    }
                }
            }
            let (wall_piece, door_piece) = if floor == 0 { ("indoors_wall_1", "indoors_door_1") } else { ("indoors_wall_2", "indoors_door_2") };
            for y in 0..grid.height {
                for x in 0..grid.width {
                    let mut third_start = floor == 2 && grid.get(x, y) == START_ROOM;
                    if grid.get(x, y) != ROOM && !third_start {
                        continue;
                    }
                    let data = rooms.get(x, y);
                    let kind = data & ROOM_TYPE_MASK;
                    let room_id = data & ROOM_ID_MASK;
                    third_start = third_start && data & ROOM_CORRIDOR_FLAG == ROOM_CORRIDOR_FLAG;
                    let mut door_dirs = Vec::new();
                    if data & ROOM_DOOR_FLAG == ROOM_DOOR_FLAG {
                        for d in Direction::HORIZONTAL {
                            let (dx, dy) = step(d);
                            if grid.get(x + dx, y + dy) == CORRIDOR {
                                door_dirs.push(d);
                            }
                        }
                    }
                    let door = if !door_dirs.is_empty() {
                        Some(door_dirs[self.random.next_i32_bound(door_dirs.len() as i32) as usize])
                    } else if data & ROOM_ORIGIN_FLAG == ROOM_ORIGIN_FLAG {
                        Some(Direction::Up)
                    } else {
                        None
                    };
                    let r = |d: Direction| rotation.rotate(d);
                    let room_pos = self.cell(floor_origin, rotation, x, y, -1);
                    let pick = |d: Direction| if door == Some(d) { door_piece } else { wall_piece };
                    if is_house(grid, x - 1, y) && !mansion.is_room_id(x - 1, y, floor, room_id) {
                        self.add(pick(Direction::West), room_pos, rotation);
                    }
                    if grid.get(x + 1, y) == CORRIDOR && !third_start {
                        self.add(pick(Direction::East), room_pos.relative(r(Direction::East), 8), rotation);
                    }
                    if is_house(grid, x, y + 1) && !mansion.is_room_id(x, y + 1, floor, room_id) {
                        let pos = room_pos.relative(r(Direction::South), 7).relative(r(Direction::East), 7);
                        self.add(pick(Direction::South), pos, rotation.then(Rotation::Clockwise90));
                    }
                    if grid.get(x, y - 1) == CORRIDOR && !third_start {
                        let pos = room_pos.relative(r(Direction::North), 1).relative(r(Direction::East), 7);
                        self.add(pick(Direction::North), pos, rotation.then(Rotation::Clockwise90));
                    }
                    if kind == ROOM_1X1 {
                        self.add_room_1x1(room_pos, rotation, door, collection);
                    } else if kind == ROOM_1X2 && door.is_some() {
                        let room_dir = mansion.room_1x2_direction(x, y, floor, room_id);
                        let stairs = data & ROOM_STAIRS_FLAG == ROOM_STAIRS_FLAG;
                        self.add_room_1x2(room_pos, rotation, room_dir, door.expect("checked"), collection, stairs);
                    } else if kind == ROOM_2X2 && door.is_some() && door != Some(Direction::Up) {
                        let door = door.expect("checked");
                        let mut room_dir = door.clockwise();
                        let (dx, dy) = step(room_dir);
                        if !mansion.is_room_id(x + dx, y + dy, floor, room_id) {
                            room_dir = room_dir.opposite();
                        }
                        self.add_room_2x2(room_pos, rotation, room_dir, door, collection);
                    } else if kind == ROOM_2X2 && door == Some(Direction::Up) {
                        self.add_mirrored("2x2_s1", room_pos.relative(r(Direction::East), 1), rotation, Mirror::None);
                    }
                }
            }
        }
    }

    /// `traverseOuterWalls`.
    #[allow(clippy::too_many_arguments)]
    fn traverse_outer_walls(&mut self, data: &mut Walker, grid: &SimpleGrid, mut direction: Direction, start_x: i32, start_y: i32, end_x: i32, end_y: i32) {
        let (mut x, mut y) = (start_x, start_y);
        let start_direction = direction;
        loop {
            let (dx, dy) = step(direction);
            let (cx, cy) = step(direction.counter_clockwise());
            if !is_house(grid, x + dx, y + dy) {
                self.traverse_turn(data);
                direction = direction.clockwise();
                if x != end_x || y != end_y || start_direction != direction {
                    self.traverse_wall_piece(data);
                }
            } else if is_house(grid, x + dx + cx, y + dy + cy) {
                self.traverse_inner_turn(data);
                x += dx;
                y += dy;
                direction = direction.counter_clockwise();
            } else {
                x += dx;
                y += dy;
                if x != end_x || y != end_y || start_direction != direction {
                    self.traverse_wall_piece(data);
                }
            }
            if x == end_x && y == end_y && start_direction == direction {
                break;
            }
        }
    }

    fn traverse_wall_piece(&mut self, data: &mut Walker) {
        let pos = data.position.relative(data.rotation.rotate(Direction::East), 7);
        self.add(data.wall, pos, data.rotation);
        data.position = data.position.relative(data.rotation.rotate(Direction::South), 8);
    }

    fn traverse_turn(&mut self, data: &mut Walker) {
        data.position = data.position.relative(data.rotation.rotate(Direction::South), -1);
        self.add("wall_corner", data.position, data.rotation);
        data.position = data.position.relative(data.rotation.rotate(Direction::South), -7);
        data.position = data.position.relative(data.rotation.rotate(Direction::West), -6);
        data.rotation = data.rotation.then(Rotation::Clockwise90);
    }

    fn traverse_inner_turn(&mut self, data: &mut Walker) {
        data.position = data.position.relative(data.rotation.rotate(Direction::South), 6);
        data.position = data.position.relative(data.rotation.rotate(Direction::East), 8);
        data.rotation = data.rotation.then(Rotation::Counterclockwise90);
    }

    /// `createRoof`.
    fn create_roof(&mut self, roof_origin: BlockPos, rotation: Rotation, grid: &SimpleGrid, above_grid: Option<&SimpleGrid>) {
        let r = |d: Direction| rotation.rotate(d);
        let cw90 = rotation.then(Rotation::Clockwise90);
        let cw180 = rotation.then(Rotation::Clockwise180);
        let ccw90 = rotation.then(Rotation::Counterclockwise90);
        let house = |x: i32, y: i32| is_house(grid, x, y);
        for y in 0..grid.height {
            for x in 0..grid.width {
                let position = self.cell(roof_origin, rotation, x, y, 0);
                let is_above = above_grid.is_some_and(|g| is_house(g, x, y));
                if !house(x, y) || is_above {
                    continue;
                }
                self.add("roof", above(position, 3), rotation);
                if !house(x + 1, y) {
                    self.add("roof_front", position.relative(r(Direction::East), 6), rotation);
                }
                if !house(x - 1, y) {
                    let p = position.relative(r(Direction::East), 0).relative(r(Direction::South), 7);
                    self.add("roof_front", p, cw180);
                }
                if !house(x, y - 1) {
                    self.add("roof_front", position.relative(r(Direction::West), 1), ccw90);
                }
                if !house(x, y + 1) {
                    let p = position.relative(r(Direction::East), 6).relative(r(Direction::South), 6);
                    self.add("roof_front", p, cw90);
                }
            }
        }
        if let Some(above_grid) = above_grid {
            for y in 0..grid.height {
                for x in 0..grid.width {
                    let position = self.cell(roof_origin, rotation, x, y, 0);
                    if !house(x, y) || !is_house(above_grid, x, y) {
                        continue;
                    }
                    if !house(x + 1, y) {
                        self.add("small_wall", position.relative(r(Direction::East), 7), rotation);
                    }
                    if !house(x - 1, y) {
                        let p = position.relative(r(Direction::West), 1).relative(r(Direction::South), 6);
                        self.add("small_wall", p, cw180);
                    }
                    if !house(x, y - 1) {
                        let p = position.relative(r(Direction::West), 0).relative(r(Direction::North), 1);
                        self.add("small_wall", p, ccw90);
                    }
                    if !house(x, y + 1) {
                        let p = position.relative(r(Direction::East), 6).relative(r(Direction::South), 7);
                        self.add("small_wall", p, cw90);
                    }
                    if !house(x + 1, y) {
                        if !house(x, y - 1) {
                            let p = position.relative(r(Direction::East), 7).relative(r(Direction::North), 2);
                            self.add("small_wall_corner", p, rotation);
                        }
                        if !house(x, y + 1) {
                            let p = position.relative(r(Direction::East), 8).relative(r(Direction::South), 7);
                            self.add("small_wall_corner", p, cw90);
                        }
                    }
                    if !house(x - 1, y) {
                        if !house(x, y - 1) {
                            let p = position.relative(r(Direction::West), 2).relative(r(Direction::North), 1);
                            self.add("small_wall_corner", p, ccw90);
                        }
                        if !house(x, y + 1) {
                            let p = position.relative(r(Direction::West), 1).relative(r(Direction::South), 8);
                            self.add("small_wall_corner", p, cw180);
                        }
                    }
                }
            }
        }
        for y in 0..grid.height {
            for x in 0..grid.width {
                let position = self.cell(roof_origin, rotation, x, y, 0);
                let is_above = above_grid.is_some_and(|g| is_house(g, x, y));
                if !house(x, y) || is_above {
                    continue;
                }
                if !house(x + 1, y) {
                    let p2 = position.relative(r(Direction::East), 6);
                    if !house(x, y + 1) {
                        self.add("roof_corner", p2.relative(r(Direction::South), 6), rotation);
                    } else if house(x + 1, y + 1) {
                        self.add("roof_inner_corner", p2.relative(r(Direction::South), 5), rotation);
                    }
                    if !house(x, y - 1) {
                        self.add("roof_corner", p2, ccw90);
                    } else if house(x + 1, y - 1) {
                        let p3 = position.relative(r(Direction::East), 9).relative(r(Direction::North), 2);
                        self.add("roof_inner_corner", p3, cw90);
                    }
                }
                if !house(x - 1, y) {
                    let p2 = position.relative(r(Direction::East), 0).relative(r(Direction::South), 0);
                    if !house(x, y + 1) {
                        self.add("roof_corner", p2.relative(r(Direction::South), 6), cw90);
                    } else if house(x - 1, y + 1) {
                        let p3 = p2.relative(r(Direction::South), 8).relative(r(Direction::West), 3);
                        self.add("roof_inner_corner", p3, ccw90);
                    }
                    if !house(x, y - 1) {
                        self.add("roof_corner", p2, cw180);
                    } else if house(x - 1, y - 1) {
                        self.add("roof_inner_corner", p2.relative(r(Direction::South), 1), cw180);
                    }
                }
            }
        }
    }

    /// `addRoom1x1`.
    fn add_room_1x1(&mut self, room_pos: BlockPos, rotation: Rotation, door: Option<Direction>, rooms: Rooms) {
        let mut piece_rot = Rotation::None;
        let mut name = rooms.one_by_one(self.random);
        if door != Some(Direction::East) {
            match door {
                Some(Direction::North) => piece_rot = piece_rot.then(Rotation::Counterclockwise90),
                Some(Direction::West) => piece_rot = piece_rot.then(Rotation::Clockwise180),
                Some(Direction::South) => piece_rot = piece_rot.then(Rotation::Clockwise90),
                _ => name = rooms.one_by_one_secret(self.random),
            }
        }
        let zero = zero_position_with_transform(BlockPos::new(1, 0, 0), Mirror::None, piece_rot, 7, 7);
        let piece_rot = piece_rot.then(rotation);
        let offset = rotate_pos(zero, rotation);
        self.add(&name, room_pos.offset(offset.x, 0, offset.z), piece_rot);
    }

    /// `addRoom1x2`.
    fn add_room_1x2(&mut self, room_pos: BlockPos, rotation: Rotation, room_dir: Option<Direction>, door: Direction, rooms: Rooms, stairs: bool) {
        use Direction::{East, North, South, Up, West};
        let r = |d: Direction| rotation.rotate(d);
        let at = |east: i32, south: i32| room_pos.relative(r(East), east).relative(r(South), south);
        let Some(room_dir) = room_dir else { return };
        let (name, pos, rot, mirror) = match (door, room_dir) {
            (East, South) => (rooms.side_entrance(self.random, stairs), at(1, 0), rotation, Mirror::None),
            (East, North) => (rooms.side_entrance(self.random, stairs), at(1, 6), rotation, Mirror::LeftRight),
            (West, North) => (rooms.side_entrance(self.random, stairs), at(7, 6), rotation.then(Rotation::Clockwise180), Mirror::None),
            (West, South) => (rooms.side_entrance(self.random, stairs), at(7, 0), rotation, Mirror::FrontBack),
            (South, East) => (rooms.side_entrance(self.random, stairs), at(1, 0), rotation.then(Rotation::Clockwise90), Mirror::LeftRight),
            (South, West) => (rooms.side_entrance(self.random, stairs), at(7, 0), rotation.then(Rotation::Clockwise90), Mirror::None),
            (North, West) => (rooms.side_entrance(self.random, stairs), at(7, 6), rotation.then(Rotation::Clockwise90), Mirror::FrontBack),
            (North, East) => (rooms.side_entrance(self.random, stairs), at(1, 6), rotation.then(Rotation::Counterclockwise90), Mirror::None),
            (South, North) => {
                let pos = room_pos.relative(r(East), 1).relative(r(North), 8);
                (rooms.front_entrance(self.random, stairs), pos, rotation, Mirror::None)
            }
            (North, South) => (rooms.front_entrance(self.random, stairs), at(7, 14), rotation.then(Rotation::Clockwise180), Mirror::None),
            (West, East) => (rooms.front_entrance(self.random, stairs), at(15, 0), rotation.then(Rotation::Clockwise90), Mirror::None),
            (East, West) => {
                let pos = room_pos.relative(r(West), 7).relative(r(South), 6);
                (rooms.front_entrance(self.random, stairs), pos, rotation.then(Rotation::Counterclockwise90), Mirror::None)
            }
            (Up, East) => (rooms.one_by_two_secret(self.random), at(15, 0), rotation.then(Rotation::Clockwise90), Mirror::None),
            (Up, South) => {
                let pos = room_pos.relative(r(East), 1).relative(r(North), 0);
                (rooms.one_by_two_secret(self.random), pos, rotation, Mirror::None)
            }
            _ => return,
        };
        self.add_mirrored(&name, pos, rot, mirror);
    }

    /// `addRoom2x2`.
    fn add_room_2x2(&mut self, room_pos: BlockPos, rotation: Rotation, room_dir: Direction, door: Direction, rooms: Rooms) {
        use Direction::{East, North, South, West};
        let (east, south, rot, mirror) = match (door, room_dir) {
            (East, South) => (-7, 0, rotation, Mirror::None),
            (East, North) => (-7, 6, rotation, Mirror::LeftRight),
            (North, East) => (1, 14, rotation.then(Rotation::Counterclockwise90), Mirror::None),
            (North, West) => (7, 14, rotation.then(Rotation::Counterclockwise90), Mirror::LeftRight),
            (South, West) => (7, -8, rotation.then(Rotation::Clockwise90), Mirror::None),
            (South, East) => (1, -8, rotation.then(Rotation::Clockwise90), Mirror::LeftRight),
            (West, North) => (15, 6, rotation.then(Rotation::Clockwise180), Mirror::None),
            (West, South) => (15, 0, rotation, Mirror::FrontBack),
            _ => (0, 0, rotation, Mirror::None),
        };
        let pos = room_pos.relative(rotation.rotate(East), east).relative(rotation.rotate(South), south);
        let name = rooms.two_by_two(self.random);
        self.add_mirrored(&name, pos, rot, mirror);
    }
}

/// `WoodlandMansionPieces.WoodlandMansionPiece`.
#[derive(Debug)]
pub struct MansionPiece {
    t: TemplatePiece,
}

impl MansionPiece {
    fn new(lib: &Library, name: &str, position: BlockPos, rotation: Rotation, mirror: Mirror) -> Self {
        let settings = PlaceSettings {
            ignore_entities: true,
            rotation,
            mirror,
            processors: vec![Processor::BlockIgnore(lib.processor_blocks.structure_block())],
            ..PlaceSettings::default()
        };
        Self { t: TemplatePiece::new(lib, 0, &format!("minecraft:woodland_mansion/{name}"), settings, position) }
    }
}

impl Piece for MansionPiece {
    fn base(&self) -> &PieceBase {
        &self.t.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.t.base
    }

    fn type_name(&self) -> &'static str {
        "minecraft:wmp"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.t.move_by(dx, dy, dz);
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, reference: BlockPos) {
        let rotation = self.t.settings.rotation;
        self.t.place(ctx, random, chunk_bb, reference, &mut |ctx, marker, pos, random, bb| {
            if marker.starts_with("Chest") {
                let chest = ctx.registries().blocks.parse_state("minecraft:chest").expect("chest");
                let facing = match marker {
                    "ChestWest" => Some(Direction::West),
                    "ChestEast" => Some(Direction::East),
                    "ChestSouth" => Some(Direction::South),
                    "ChestNorth" => Some(Direction::North),
                    _ => None,
                };
                let state = match facing {
                    Some(d) => ctx.with(chest, "facing", rotation.rotate(d).name()),
                    None => chest,
                };
                create_chest(ctx, bb, random, pos, "minecraft:chests/woodland_mansion", Some(state));
                return;
            }
            let (entity, count) = match marker {
                "Mage" => ("minecraft:evoker", 1),
                "Warrior" => ("minecraft:vindicator", 1),
                "Group of Allays" => ("minecraft:allay", ctx.region.level_random().next_i32_bound(3) + 1),
                _ => return,
            };
            // `EntityType.create` is null for hostile mobs on peaceful; the
            // marker keeps its block then.
            if !ctx.lib.can_spawn(entity) {
                return;
            }
            let air = ctx.lib.blocks.air;
            for _ in 0..count {
                // `setPersistenceRequired`, `snapTo(position)` and `finalizeSpawn`.
                let at = [f64::from(pos.x) + 0.5, f64::from(pos.y), f64::from(pos.z) + 0.5];
                if let Some(mut mob) = crate::feature::entities::create(ctx, entity, at, 0.0, 0.0) {
                    crate::feature::entities::set(&mut mob, [("PersistenceRequired", minecraftoss_core::nbt::Tag::Byte(1))]);
                    crate::feature::entities::finalize_spawn(ctx, &mut mob, false);
                    ctx.region.add_entity(mob);
                }
                ctx.set_block(pos, air);
            }
        });
    }
}
