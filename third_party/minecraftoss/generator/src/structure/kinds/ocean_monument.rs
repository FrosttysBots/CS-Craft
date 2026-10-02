//! Ocean monuments (vanilla `OceanMonumentStructure`, `OceanMonumentPieces`).
//!
//! Source-informed from the pinned 26.3 JAR. The start is one building
//! piece that owns its rooms: a 5x5x3 room graph whose openings are closed
//! at random while every room still reaches the entrance, then filled by
//! room fitters in a fixed order. Grid rooms connect along z with the
//! opposite direction, so "north" is +z in grid space, as vanilla has it.
//! The whole building places from its one piece; rooms draw from the same
//! chunk random when they intersect the chunk.

use crate::feature::template::BoundingBox;
use crate::feature::Ctx;
use crate::structure::piece::{Piece, PieceBase};
use crate::structure::{GenerationContext, StructureKind, Stub};
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::{LegacyRandom, WorldgenRandom};
use minecraftoss_core::{BlockPos, BlockStateId, ChunkPos};

#[derive(Debug)]
pub struct OceanMonument;

impl StructureKind for OceanMonument {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        let chunk = ctx.chunk;
        // `getBiomesWithin(x, seaLevel, z, 29)`: every quart must be in the tag.
        let (x, y, z, r) = (chunk.min_block_x() + 9, ctx.terrain.sea_level, chunk.min_block_z() + 9, 29);
        let tags = &ctx.lib.registries.biome_tags;
        let tag = tags.require("minecraft:required_ocean_monument_surrounding").ok()?;
        for qz in (z - r) >> 2..=(z + r) >> 2 {
            for qx in (x - r) >> 2..=(x + r) >> 2 {
                for qy in (y - r) >> 2..=(y + r) >> 2 {
                    if !tags.contains(tag, usize::from(ctx.terrain.biome_at_quart(qx, qy, qz).0)) {
                        return None;
                    }
                }
            }
        }
        ctx.on_top_of_chunk_center(HeightmapKind::OceanFloorWg, move |ctx: &mut GenerationContext| {
            let direction = PieceBase::random_horizontal_direction(&mut ctx.random);
            vec![Box::new(Monument::new(&mut ctx.random, chunk.min_block_x() - 29, chunk.min_block_z() - 29, direction)) as Box<dyn Piece>]
        })
    }
}

/// `Direction.get3DDataValue` indices.
const DOWN: usize = 0;
const UP: usize = 1;
const NORTH: usize = 2;
const SOUTH: usize = 3;
const WEST: usize = 4;
const EAST: usize = 5;
const STEPS: [(i32, i32, i32); 6] = [(0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)];

/// `OceanMonumentPieces.RoomDefinition`.
#[derive(Clone, Debug)]
struct Room {
    index: i32,
    connections: [Option<usize>; 6],
    has_opening: [bool; 6],
    claimed: bool,
    is_source: bool,
    scan_index: i32,
}

impl Room {
    fn new(index: i32) -> Self {
        Self { index, connections: [None; 6], has_opening: [false; 6], claimed: false, is_source: false, scan_index: 0 }
    }

    fn is_special(&self) -> bool {
        self.index >= 75
    }

    fn count_openings(&self) -> usize {
        self.has_opening.iter().filter(|&&o| o).count()
    }
}

fn room_index(x: i32, y: i32, z: i32) -> i32 {
    y * 25 + z * 5 + x
}

fn set_connection(rooms: &mut [Room], a: usize, direction: usize, b: usize) {
    rooms[a].connections[direction] = Some(b);
    rooms[b].connections[direction ^ 1] = Some(a);
}

fn update_openings(room: &mut Room) {
    for i in 0..6 {
        room.has_opening[i] = room.connections[i].is_some();
    }
}

/// `RoomDefinition.findSource`.
fn find_source(rooms: &mut [Room], r: usize, scan_index: i32) -> bool {
    if rooms[r].is_source {
        return true;
    }
    rooms[r].scan_index = scan_index;
    for i in 0..6 {
        if let Some(c) = rooms[r].connections[i] {
            if rooms[r].has_opening[i] && rooms[c].scan_index != scan_index && find_source(rooms, c, scan_index) {
                return true;
            }
        }
    }
    false
}

/// A connected room, which `has_opening` guarantees exists.
fn conn(rooms: &[Room], r: usize, direction: usize) -> usize {
    rooms[r].connections[direction].expect("opening has a room")
}

/// `MonumentBuilding.generateRoomGraph`: the room arena, the rooms in fitting
/// order, and the source and core rooms.
fn room_graph(random: &mut LegacyRandom) -> (Vec<Room>, Vec<usize>, usize, usize) {
    let mut rooms: Vec<Room> = Vec::new();
    let mut grid: [Option<usize>; 75] = [None; 75];
    let add = |rooms: &mut Vec<Room>, grid: &mut [Option<usize>; 75], x: i32, y: i32, z: i32| {
        let pos = room_index(x, y, z);
        rooms.push(Room::new(pos));
        grid[pos as usize] = Some(rooms.len() - 1);
    };
    for x in 0..5 {
        for z in 0..4 {
            add(&mut rooms, &mut grid, x, 0, z);
        }
    }
    for x in 0..5 {
        for z in 0..4 {
            add(&mut rooms, &mut grid, x, 1, z);
        }
    }
    for x in 1..4 {
        for z in 0..2 {
            add(&mut rooms, &mut grid, x, 2, z);
        }
    }
    let source = grid[room_index(2, 0, 0) as usize].expect("source room");
    for x in 0..5 {
        for z in 0..5 {
            for y in 0..3 {
                let Some(room) = grid[room_index(x, y, z) as usize] else { continue };
                for (direction, (dx, dy, dz)) in STEPS.into_iter().enumerate() {
                    let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                    if !(0..5).contains(&nx) || !(0..5).contains(&nz) || !(0..3).contains(&ny) {
                        continue;
                    }
                    if let Some(neighbor) = grid[room_index(nx, ny, nz) as usize] {
                        let d = if nz == z { direction } else { direction ^ 1 };
                        set_connection(&mut rooms, room, d, neighbor);
                    }
                }
            }
        }
    }
    let (roof, left, right) = (rooms.len(), rooms.len() + 1, rooms.len() + 2);
    rooms.push(Room::new(1003));
    rooms.push(Room::new(1001));
    rooms.push(Room::new(1002));
    let at = |x: i32, y: i32, z: i32| grid[room_index(x, y, z) as usize].expect("grid room");
    set_connection(&mut rooms, at(2, 2, 0), UP, roof);
    set_connection(&mut rooms, at(0, 1, 0), SOUTH, left);
    set_connection(&mut rooms, at(4, 1, 0), SOUTH, right);
    for r in [roof, left, right] {
        rooms[r].claimed = true;
    }
    rooms[source].is_source = true;
    let core = at(random.next_i32_bound(4), 0, 2);
    let east = conn(&rooms, core, EAST);
    let north = conn(&rooms, core, NORTH);
    let up = conn(&rooms, core, UP);
    let east_north = conn(&rooms, east, NORTH);
    let east_up = conn(&rooms, east, UP);
    let north_up = conn(&rooms, north, UP);
    let east_north_up = conn(&rooms, east_north, UP);
    for r in [core, east, north, east_north, up, east_up, north_up, east_north_up] {
        rooms[r].claimed = true;
    }
    let mut defs: Vec<usize> = Vec::new();
    for room in grid.into_iter().flatten() {
        update_openings(&mut rooms[room]);
        defs.push(room);
    }
    update_openings(&mut rooms[roof]);
    // `Util.shuffle`.
    for i in (2..=defs.len()).rev() {
        let swap_to = random.next_i32_bound(i as i32) as usize;
        defs.swap(i - 1, swap_to);
    }
    let mut scan_index = 1;
    for &room in &defs {
        let (mut closed, mut attempts) = (0, 0);
        while closed < 2 && attempts < 5 {
            attempts += 1;
            let f = random.next_i32_bound(6) as usize;
            if !rooms[room].has_opening[f] {
                continue;
            }
            let other = conn(&rooms, room, f);
            rooms[room].has_opening[f] = false;
            rooms[other].has_opening[f ^ 1] = false;
            let from_room = find_source(&mut rooms, room, scan_index);
            scan_index += 1;
            let reachable = from_room && {
                let from_other = find_source(&mut rooms, other, scan_index);
                scan_index += 1;
                from_other
            };
            if reachable {
                closed += 1;
            } else {
                rooms[room].has_opening[f] = true;
                rooms[other].has_opening[f ^ 1] = true;
            }
        }
    }
    defs.extend([roof, left, right]);
    (rooms, defs, source, core)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Entry,
    Core,
    DoubleX,
    DoubleXY,
    DoubleY,
    DoubleYZ,
    DoubleZ,
    Simple { design: i32 },
    SimpleTop,
    Wing { design: i32 },
    Penthouse,
}

/// One room piece of the building.
#[derive(Clone, Debug)]
struct Child {
    base: PieceBase,
    kind: Kind,
    room: usize,
}

/// `OceanMonumentPiece.makeBoundingBox(orientation, definition, w, h, d)`.
fn room_box(orientation: Direction, index: i32, width: i32, height: i32, depth: i32) -> BoundingBox {
    let (room_x, room_z, room_y) = (index % 5, index / 5 % 5, index / 25);
    let b = PieceBase::make_bounding_box(0, 0, 0, orientation, width * 8, height * 4, depth * 8);
    match orientation {
        Direction::North => b.moved(room_x * 8, room_y * 4, -(room_z + depth) * 8 + 1),
        Direction::South => b.moved(room_x * 8, room_y * 4, room_z * 8),
        Direction::West => b.moved(-(room_z + depth) * 8 + 1, room_y * 4, room_x * 8),
        _ => b.moved(room_z * 8, room_y * 4, room_x * 8),
    }
}

fn room_child(orientation: Direction, rooms: &[Room], room: usize, kind: Kind, size: (i32, i32, i32)) -> Child {
    let mut base = PieceBase::new(1, room_box(orientation, rooms[room].index, size.0, size.1, size.2));
    base.set_orientation(Some(orientation));
    Child { base, kind, room }
}

/// Tries the room fitters in vanilla order on one unclaimed room.
fn fit(rooms: &mut [Room], room: usize, orientation: Direction, random: &mut LegacyRandom) -> Child {
    let open = |rooms: &[Room], r: usize, d: usize| rooms[r].has_opening[d] && !rooms[conn(rooms, r, d)].claimed;
    let claim = |rooms: &mut [Room], list: &[usize]| {
        for &r in list {
            rooms[r].claimed = true;
        }
    };
    if open(rooms, room, EAST) && open(rooms, room, UP) && open(rooms, conn(rooms, room, EAST), UP) {
        let east = conn(rooms, room, EAST);
        let list = [room, east, conn(rooms, room, UP), conn(rooms, east, UP)];
        claim(rooms, &list);
        return room_child(orientation, rooms, room, Kind::DoubleXY, (2, 2, 1));
    }
    if open(rooms, room, NORTH) && open(rooms, room, UP) && open(rooms, conn(rooms, room, NORTH), UP) {
        let north = conn(rooms, room, NORTH);
        let list = [room, north, conn(rooms, room, UP), conn(rooms, north, UP)];
        claim(rooms, &list);
        return room_child(orientation, rooms, room, Kind::DoubleYZ, (1, 2, 2));
    }
    if open(rooms, room, NORTH) {
        let list = [room, conn(rooms, room, NORTH)];
        claim(rooms, &list);
        return room_child(orientation, rooms, room, Kind::DoubleZ, (1, 1, 2));
    }
    if open(rooms, room, EAST) {
        let list = [room, conn(rooms, room, EAST)];
        claim(rooms, &list);
        return room_child(orientation, rooms, room, Kind::DoubleX, (2, 1, 1));
    }
    if open(rooms, room, UP) {
        let list = [room, conn(rooms, room, UP)];
        claim(rooms, &list);
        return room_child(orientation, rooms, room, Kind::DoubleY, (1, 2, 1));
    }
    rooms[room].claimed = true;
    let h = &rooms[room].has_opening;
    if !h[WEST] && !h[EAST] && !h[NORTH] && !h[SOUTH] && !h[UP] {
        return room_child(orientation, rooms, room, Kind::SimpleTop, (1, 1, 1));
    }
    let design = random.next_i32_bound(3);
    room_child(orientation, rooms, room, Kind::Simple { design }, (1, 1, 1))
}

/// `OceanMonumentPieces.MonumentBuilding`.
#[derive(Debug)]
pub struct Monument {
    base: PieceBase,
    rooms: Vec<Room>,
    children: Vec<Child>,
}

impl Monument {
    fn new(random: &mut LegacyRandom, west: i32, north: i32, direction: Direction) -> Self {
        let mut base = PieceBase::new(0, PieceBase::make_bounding_box(west, 39, north, direction, 58, 23, 58));
        base.set_orientation(Some(direction));
        let (mut rooms, defs, source, core) = room_graph(random);
        rooms[source].claimed = true;
        let mut children = vec![
            room_child(direction, &rooms, source, Kind::Entry, (1, 1, 1)),
            room_child(direction, &rooms, core, Kind::Core, (2, 2, 2)),
        ];
        for room in defs {
            if !rooms[room].claimed && !rooms[room].is_special() {
                children.push(fit(&mut rooms, room, direction, random));
            }
        }
        let offset = base.world_pos(9, 0, 22);
        for child in &mut children {
            child.base.bbox = child.base.bbox.moved(offset.x, offset.y, offset.z);
        }
        let corners = |a: (i32, i32, i32), b: (i32, i32, i32)| BoundingBox::from_corners(base.world_pos(a.0, a.1, a.2), base.world_pos(b.0, b.1, b.2));
        let left_wing = corners((1, 1, 1), (23, 8, 21));
        let right_wing = corners((34, 1, 1), (56, 8, 21));
        let penthouse = corners((22, 13, 22), (35, 17, 35));
        let wing_random = random.next_i32();
        let boxed = |bbox: BoundingBox, kind: Kind| {
            let mut base = PieceBase::new(1, bbox);
            base.set_orientation(Some(direction));
            Child { base, kind, room: usize::MAX }
        };
        children.push(boxed(left_wing, Kind::Wing { design: wing_random & 1 }));
        children.push(boxed(right_wing, Kind::Wing { design: wing_random.wrapping_add(1) & 1 }));
        children.push(boxed(penthouse, Kind::Penthouse));
        Self { base, rooms, children }
    }
}

/// Block states the monument uses.
struct B {
    gray: BlockStateId,
    light: BlockStateId,
    black: BlockStateId,
    lamp: BlockStateId,
    water: BlockStateId,
    air: BlockStateId,
    gold: BlockStateId,
    sponge: BlockStateId,
    /// `FILL_KEEP`: ice, packed ice, blue ice and water.
    keep: [minecraftoss_core::BlockId; 4],
    sea_level: i32,
}

impl B {
    fn load(ctx: &Ctx) -> Self {
        let blocks = &ctx.registries().blocks;
        let s = |name: &str| blocks.parse_state(name).expect("monument block");
        let keep = ["minecraft:ice", "minecraft:packed_ice", "minecraft:blue_ice", "minecraft:water"].map(|n| blocks.block_of(s(n)));
        Self {
            gray: s("minecraft:prismarine"),
            light: s("minecraft:prismarine_bricks"),
            black: s("minecraft:dark_prismarine"),
            lamp: s("minecraft:sea_lantern"),
            water: s("minecraft:water"),
            air: s("minecraft:air"),
            gold: s("minecraft:gold_block"),
            sponge: s("minecraft:wet_sponge"),
            keep,
            sea_level: ctx.lib.generation.sea_level,
        }
    }
}

/// The drawing helpers every monument piece shares.
struct Draw<'p> {
    p: &'p PieceBase,
    bb: &'p BoundingBox,
    b: &'p B,
}

impl Draw<'_> {
    fn bx(&self, ctx: &mut Ctx, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32, s: BlockStateId) {
        self.p.generate_box(ctx, self.bb, x0, y0, z0, x1, y1, z1, s, s, false);
    }

    fn put(&self, ctx: &mut Ctx, s: BlockStateId, x: i32, y: i32, z: i32) {
        self.p.place_block(ctx, s, x, y, z, self.bb);
    }

    /// `generateWaterBox`: water below sea level, air above, keeping ice and water.
    fn water(&self, ctx: &mut Ctx, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    let block = self.p.get_block(ctx, x, y, z, self.bb);
                    if self.b.keep.contains(&ctx.registries().blocks.block_of(block)) {
                        continue;
                    }
                    let state = if self.p.world_y(y) >= self.b.sea_level && block != self.b.water { self.b.air } else { self.b.water };
                    self.put(ctx, state, x, y, z);
                }
            }
        }
    }

    /// `generateBoxOnFillOnly`: replaces only still water.
    fn on_fill(&self, ctx: &mut Ctx, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32, s: BlockStateId) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if self.p.get_block(ctx, x, y, z, self.bb) == self.b.water {
                        self.put(ctx, s, x, y, z);
                    }
                }
            }
        }
    }

    /// `generateDefaultFloor`.
    fn floor(&self, ctx: &mut Ctx, xo: i32, zo: i32, down_opening: bool) {
        let (g, l) = (self.b.gray, self.b.light);
        if down_opening {
            self.bx(ctx, xo, 0, zo, xo + 2, 0, zo + 7, g);
            self.bx(ctx, xo + 5, 0, zo, xo + 7, 0, zo + 7, g);
            self.bx(ctx, xo + 3, 0, zo, xo + 4, 0, zo + 2, g);
            self.bx(ctx, xo + 3, 0, zo + 5, xo + 4, 0, zo + 7, g);
            self.bx(ctx, xo + 3, 0, zo + 2, xo + 4, 0, zo + 2, l);
            self.bx(ctx, xo + 3, 0, zo + 5, xo + 4, 0, zo + 5, l);
            self.bx(ctx, xo + 2, 0, zo + 3, xo + 2, 0, zo + 4, l);
            self.bx(ctx, xo + 5, 0, zo + 3, xo + 5, 0, zo + 4, l);
        } else {
            self.bx(ctx, xo, 0, zo, xo + 7, 0, zo + 7, g);
        }
    }

    /// `chunkIntersects` over a piece-local rectangle.
    fn chunk_intersects(&self, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
        let (wx0, wz0) = (self.p.world_x(x0, z0), self.p.world_z(x0, z0));
        let (wx1, wz1) = (self.p.world_x(x1, z1), self.p.world_z(x1, z1));
        self.bb.intersects_xz(wx0.min(wx1), wz0.min(wz1), wx0.max(wx1), wz0.max(wz1))
    }

    fn spawn_elder(&self, ctx: &mut Ctx, x: i32, y: i32, z: i32) {
        let pos = self.p.world_pos(x, y, z);
        if self.bb.is_inside(pos) && ctx.lib.can_spawn("minecraft:elder_guardian") {
            let at = [f64::from(pos.x) + 0.5, f64::from(pos.y), f64::from(pos.z) + 0.5];
            if let Some(mut entity) = crate::feature::entities::create(ctx, "minecraft:elder_guardian", at, 0.0, 0.0) {
                crate::feature::entities::finalize_spawn(ctx, &mut entity, false);
                ctx.region.add_entity(entity);
            }
        }
    }
}

impl Piece for Monument {
    fn base(&self) -> &PieceBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.base
    }

    fn type_name(&self) -> &'static str {
        "minecraft:omb"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base.bbox = self.base.bbox.moved(dx, dy, dz);
        for child in &mut self.children {
            child.base.bbox = child.base.bbox.moved(dx, dy, dz);
        }
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, _reference: BlockPos) {
        let b = B::load(ctx);
        let d = Draw { p: &self.base, bb: chunk_bb, b: &b };
        let water_height = b.sea_level.max(64) - self.base.bbox.min_y;
        d.water(ctx, 0, 0, 0, 58, water_height, 58);
        building_wing(&d, ctx, false, 0);
        building_wing(&d, ctx, true, 33);
        building_entrance_archs(&d, ctx);
        building_entrance_wall(&d, ctx);
        building_roof(&d, ctx);
        building_lower_wall(&d, ctx);
        building_middle_wall(&d, ctx);
        building_upper_wall(&d, ctx);
        for pillar_x in 0..7 {
            let mut pillar_z = 0;
            while pillar_z < 7 {
                if pillar_z == 0 && pillar_x == 3 {
                    pillar_z = 6;
                }
                let (bx, bz) = (pillar_x * 9, pillar_z * 9);
                for w in 0..4 {
                    for dd in 0..4 {
                        d.put(ctx, b.light, bx + w, 0, bz + dd);
                        self.base.fill_column_down(ctx, b.light, bx + w, -1, bz + dd, chunk_bb);
                    }
                }
                pillar_z += if pillar_x != 0 && pillar_x != 6 { 6 } else { 1 };
            }
        }
        for i in 0..5 {
            d.water(ctx, -1 - i, i * 2, -1 - i, -1 - i, 23, 58 + i);
            d.water(ctx, 58 + i, i * 2, -1 - i, 58 + i, 23, 58 + i);
            d.water(ctx, -i, i * 2, -1 - i, 57 + i, 23, -1 - i);
            d.water(ctx, -i, i * 2, 58 + i, 57 + i, 23, 58 + i);
        }
        for child in &self.children {
            if child.base.bbox.intersects(chunk_bb) {
                let d = Draw { p: &child.base, bb: chunk_bb, b: &b };
                draw_child(&d, ctx, random, &self.rooms, child);
            }
        }
    }
}

fn building_wing(d: &Draw, ctx: &mut Ctx, flipped: bool, xoff: i32) {
    if !d.chunk_intersects(xoff, 0, xoff + 23, 20) {
        return;
    }
    let (g, l) = (d.b.gray, d.b.light);
    d.bx(ctx, xoff, 0, 0, xoff + 24, 0, 20, g);
    d.water(ctx, xoff, 1, 0, xoff + 24, 10, 20);
    for i in 0..4 {
        d.bx(ctx, xoff + i, i + 1, i, xoff + i, i + 1, 20, l);
        d.bx(ctx, xoff + i + 7, i + 5, i + 7, xoff + i + 7, i + 5, 20, l);
        d.bx(ctx, xoff + 17 - i, i + 5, i + 7, xoff + 17 - i, i + 5, 20, l);
        d.bx(ctx, xoff + 24 - i, i + 1, i, xoff + 24 - i, i + 1, 20, l);
        d.bx(ctx, xoff + i + 1, i + 1, i, xoff + 23 - i, i + 1, i, l);
        d.bx(ctx, xoff + i + 8, i + 5, i + 7, xoff + 16 - i, i + 5, i + 7, l);
    }
    d.bx(ctx, xoff + 4, 4, 4, xoff + 6, 4, 20, g);
    d.bx(ctx, xoff + 7, 4, 4, xoff + 17, 4, 6, g);
    d.bx(ctx, xoff + 18, 4, 4, xoff + 20, 4, 20, g);
    d.bx(ctx, xoff + 11, 8, 11, xoff + 13, 8, 20, g);
    d.put(ctx, l, xoff + 12, 9, 12);
    d.put(ctx, l, xoff + 12, 9, 15);
    d.put(ctx, l, xoff + 12, 9, 18);
    let left = xoff + if flipped { 19 } else { 5 };
    let right = xoff + if flipped { 5 } else { 19 };
    for z in (5..=20).rev().step_by(3) {
        d.put(ctx, l, left, 5, z);
    }
    for z in (7..=19).rev().step_by(3) {
        d.put(ctx, l, right, 5, z);
    }
    for i in 0..4 {
        let x = if flipped { xoff + 24 - (17 - i * 3) } else { xoff + 17 - i * 3 };
        d.put(ctx, l, x, 5, 5);
    }
    d.put(ctx, l, right, 5, 5);
    d.bx(ctx, xoff + 11, 1, 12, xoff + 13, 7, 12, g);
    d.bx(ctx, xoff + 12, 1, 11, xoff + 12, 7, 13, g);
}

fn building_entrance_archs(d: &Draw, ctx: &mut Ctx) {
    if !d.chunk_intersects(22, 5, 35, 17) {
        return;
    }
    let (g, l, lamp) = (d.b.gray, d.b.light, d.b.lamp);
    d.water(ctx, 25, 0, 0, 32, 8, 20);
    for i in 0..4 {
        let z = 5 + i * 4;
        d.bx(ctx, 24, 2, z, 24, 4, z, l);
        d.bx(ctx, 22, 4, z, 23, 4, z, l);
        d.put(ctx, l, 25, 5, z);
        d.put(ctx, l, 26, 6, z);
        d.put(ctx, lamp, 26, 5, z);
        d.bx(ctx, 33, 2, z, 33, 4, z, l);
        d.bx(ctx, 34, 4, z, 35, 4, z, l);
        d.put(ctx, l, 32, 5, z);
        d.put(ctx, l, 31, 6, z);
        d.put(ctx, lamp, 31, 5, z);
        d.bx(ctx, 27, 6, z, 30, 6, z, g);
    }
}

fn building_entrance_wall(d: &Draw, ctx: &mut Ctx) {
    if !d.chunk_intersects(15, 20, 42, 21) {
        return;
    }
    let (g, l, k) = (d.b.gray, d.b.light, d.b.black);
    d.bx(ctx, 15, 0, 21, 42, 0, 21, g);
    d.water(ctx, 26, 1, 21, 31, 3, 21);
    d.bx(ctx, 21, 12, 21, 36, 12, 21, g);
    d.bx(ctx, 17, 11, 21, 40, 11, 21, g);
    d.bx(ctx, 16, 10, 21, 41, 10, 21, g);
    d.bx(ctx, 15, 7, 21, 42, 9, 21, g);
    d.bx(ctx, 16, 6, 21, 41, 6, 21, g);
    d.bx(ctx, 17, 5, 21, 40, 5, 21, g);
    d.bx(ctx, 21, 4, 21, 36, 4, 21, g);
    d.bx(ctx, 22, 3, 21, 26, 3, 21, g);
    d.bx(ctx, 31, 3, 21, 35, 3, 21, g);
    d.bx(ctx, 23, 2, 21, 25, 2, 21, g);
    d.bx(ctx, 32, 2, 21, 34, 2, 21, g);
    d.bx(ctx, 28, 4, 20, 29, 4, 21, l);
    for (x, y) in [(27, 3), (30, 3), (26, 2), (31, 2), (25, 1), (32, 1)] {
        d.put(ctx, l, x, y, 21);
    }
    for i in 0..7 {
        d.put(ctx, k, 28 - i, 6 + i, 21);
        d.put(ctx, k, 29 + i, 6 + i, 21);
    }
    for i in 0..4 {
        d.put(ctx, k, 28 - i, 9 + i, 21);
        d.put(ctx, k, 29 + i, 9 + i, 21);
    }
    d.put(ctx, k, 28, 12, 21);
    d.put(ctx, k, 29, 12, 21);
    for i in 0..3 {
        d.put(ctx, k, 22 - i * 2, 8, 21);
        d.put(ctx, k, 22 - i * 2, 9, 21);
        d.put(ctx, k, 35 + i * 2, 8, 21);
        d.put(ctx, k, 35 + i * 2, 9, 21);
    }
    d.water(ctx, 15, 13, 21, 42, 15, 21);
    d.water(ctx, 15, 1, 21, 15, 6, 21);
    d.water(ctx, 16, 1, 21, 16, 5, 21);
    d.water(ctx, 17, 1, 21, 20, 4, 21);
    d.water(ctx, 21, 1, 21, 21, 3, 21);
    d.water(ctx, 22, 1, 21, 22, 2, 21);
    d.water(ctx, 23, 1, 21, 24, 1, 21);
    d.water(ctx, 42, 1, 21, 42, 6, 21);
    d.water(ctx, 41, 1, 21, 41, 5, 21);
    d.water(ctx, 37, 1, 21, 40, 4, 21);
    d.water(ctx, 36, 1, 21, 36, 3, 21);
    d.water(ctx, 33, 1, 21, 34, 1, 21);
    d.water(ctx, 35, 1, 21, 35, 2, 21);
}

fn building_roof(d: &Draw, ctx: &mut Ctx) {
    if !d.chunk_intersects(21, 21, 36, 36) {
        return;
    }
    let (g, l, lamp) = (d.b.gray, d.b.light, d.b.lamp);
    d.bx(ctx, 21, 0, 22, 36, 0, 36, g);
    d.water(ctx, 21, 1, 22, 36, 23, 36);
    for i in 0..4 {
        d.bx(ctx, 21 + i, 13 + i, 21 + i, 36 - i, 13 + i, 21 + i, l);
        d.bx(ctx, 21 + i, 13 + i, 36 - i, 36 - i, 13 + i, 36 - i, l);
        d.bx(ctx, 21 + i, 13 + i, 22 + i, 21 + i, 13 + i, 35 - i, l);
        d.bx(ctx, 36 - i, 13 + i, 22 + i, 36 - i, 13 + i, 35 - i, l);
    }
    d.bx(ctx, 25, 16, 25, 32, 16, 32, g);
    d.bx(ctx, 25, 17, 25, 25, 19, 25, l);
    d.bx(ctx, 32, 17, 25, 32, 19, 25, l);
    d.bx(ctx, 25, 17, 32, 25, 19, 32, l);
    d.bx(ctx, 32, 17, 32, 32, 19, 32, l);
    d.put(ctx, l, 26, 20, 26);
    d.put(ctx, l, 27, 21, 27);
    d.put(ctx, lamp, 27, 20, 27);
    d.put(ctx, l, 26, 20, 31);
    d.put(ctx, l, 27, 21, 30);
    d.put(ctx, lamp, 27, 20, 30);
    d.put(ctx, l, 31, 20, 31);
    d.put(ctx, l, 30, 21, 30);
    d.put(ctx, lamp, 30, 20, 30);
    d.put(ctx, l, 31, 20, 26);
    d.put(ctx, l, 30, 21, 27);
    d.put(ctx, lamp, 30, 20, 27);
    d.bx(ctx, 28, 21, 27, 29, 21, 27, g);
    d.bx(ctx, 27, 21, 28, 27, 21, 29, g);
    d.bx(ctx, 28, 21, 30, 29, 21, 30, g);
    d.bx(ctx, 30, 21, 28, 30, 21, 29, g);
}

fn building_lower_wall(d: &Draw, ctx: &mut Ctx) {
    let (g, l) = (d.b.gray, d.b.light);
    if d.chunk_intersects(0, 21, 6, 58) {
        d.bx(ctx, 0, 0, 21, 6, 0, 57, g);
        d.water(ctx, 0, 1, 21, 6, 7, 57);
        d.bx(ctx, 4, 4, 21, 6, 4, 53, g);
        for i in 0..4 {
            d.bx(ctx, i, i + 1, 21, i, i + 1, 57 - i, l);
        }
        for z in (23..53).step_by(3) {
            d.put(ctx, l, 5, 5, z);
        }
        d.put(ctx, l, 5, 5, 52);
        for i in 0..4 {
            d.bx(ctx, i, i + 1, 21, i, i + 1, 57 - i, l);
        }
        d.bx(ctx, 4, 1, 52, 6, 3, 52, g);
        d.bx(ctx, 5, 1, 51, 5, 3, 53, g);
    }
    if d.chunk_intersects(51, 21, 58, 58) {
        d.bx(ctx, 51, 0, 21, 57, 0, 57, g);
        d.water(ctx, 51, 1, 21, 57, 7, 57);
        d.bx(ctx, 51, 4, 21, 53, 4, 53, g);
        for i in 0..4 {
            d.bx(ctx, 57 - i, i + 1, 21, 57 - i, i + 1, 57 - i, l);
        }
        for z in (23..53).step_by(3) {
            d.put(ctx, l, 52, 5, z);
        }
        d.put(ctx, l, 52, 5, 52);
        d.bx(ctx, 51, 1, 52, 53, 3, 52, g);
        d.bx(ctx, 52, 1, 51, 52, 3, 53, g);
    }
    if d.chunk_intersects(0, 51, 57, 57) {
        d.bx(ctx, 7, 0, 51, 50, 0, 57, g);
        d.water(ctx, 7, 1, 51, 50, 10, 57);
        for i in 0..4 {
            d.bx(ctx, i + 1, i + 1, 57 - i, 56 - i, i + 1, 57 - i, l);
        }
    }
}

fn building_middle_wall(d: &Draw, ctx: &mut Ctx) {
    let (g, l) = (d.b.gray, d.b.light);
    if d.chunk_intersects(7, 21, 13, 50) {
        d.bx(ctx, 7, 0, 21, 13, 0, 50, g);
        d.water(ctx, 7, 1, 21, 13, 10, 50);
        d.bx(ctx, 11, 8, 21, 13, 8, 53, g);
        for i in 0..4 {
            d.bx(ctx, i + 7, i + 5, 21, i + 7, i + 5, 54, l);
        }
        for z in (21..=45).step_by(3) {
            d.put(ctx, l, 12, 9, z);
        }
    }
    if d.chunk_intersects(44, 21, 50, 54) {
        d.bx(ctx, 44, 0, 21, 50, 0, 50, g);
        d.water(ctx, 44, 1, 21, 50, 10, 50);
        d.bx(ctx, 44, 8, 21, 46, 8, 53, g);
        for i in 0..4 {
            d.bx(ctx, 50 - i, i + 5, 21, 50 - i, i + 5, 54, l);
        }
        for z in (21..=45).step_by(3) {
            d.put(ctx, l, 45, 9, z);
        }
    }
    if d.chunk_intersects(8, 44, 49, 54) {
        d.bx(ctx, 14, 0, 44, 43, 0, 50, g);
        d.water(ctx, 14, 1, 44, 43, 10, 50);
        for x in (12..=45).step_by(3) {
            d.put(ctx, l, x, 9, 45);
            d.put(ctx, l, x, 9, 52);
            if matches!(x, 12 | 18 | 24 | 33 | 39 | 45) {
                for (y, z) in [(9, 47), (9, 50), (10, 45), (10, 46), (10, 51), (10, 52), (11, 47), (11, 50), (12, 48), (12, 49)] {
                    d.put(ctx, l, x, y, z);
                }
            }
        }
        for i in 0..3 {
            d.bx(ctx, 8 + i, 5 + i, 54, 49 - i, 5 + i, 54, g);
        }
        d.bx(ctx, 11, 8, 54, 46, 8, 54, l);
        d.bx(ctx, 14, 8, 44, 43, 8, 53, g);
    }
}

fn building_upper_wall(d: &Draw, ctx: &mut Ctx) {
    let (g, l) = (d.b.gray, d.b.light);
    if d.chunk_intersects(14, 21, 20, 43) {
        d.bx(ctx, 14, 0, 21, 20, 0, 43, g);
        d.water(ctx, 14, 1, 22, 20, 14, 43);
        d.bx(ctx, 18, 12, 22, 20, 12, 39, g);
        d.bx(ctx, 18, 12, 21, 20, 12, 21, l);
        for i in 0..4 {
            d.bx(ctx, i + 14, i + 9, 21, i + 14, i + 9, 43 - i, l);
        }
        for z in (23..=39).step_by(3) {
            d.put(ctx, l, 19, 13, z);
        }
    }
    if d.chunk_intersects(37, 21, 43, 43) {
        d.bx(ctx, 37, 0, 21, 43, 0, 43, g);
        d.water(ctx, 37, 1, 22, 43, 14, 43);
        d.bx(ctx, 37, 12, 22, 39, 12, 39, g);
        d.bx(ctx, 37, 12, 21, 39, 12, 21, l);
        for i in 0..4 {
            d.bx(ctx, 43 - i, i + 9, 21, 43 - i, i + 9, 43 - i, l);
        }
        for z in (23..=39).step_by(3) {
            d.put(ctx, l, 38, 13, z);
        }
    }
    if d.chunk_intersects(15, 37, 42, 43) {
        d.bx(ctx, 21, 0, 37, 36, 0, 43, g);
        d.water(ctx, 21, 1, 37, 36, 14, 43);
        d.bx(ctx, 21, 12, 37, 36, 12, 39, g);
        for i in 0..4 {
            d.bx(ctx, 15 + i, i + 9, 43 - i, 42 - i, i + 9, 43 - i, l);
        }
        for x in (21..=36).step_by(3) {
            d.put(ctx, l, x, 13, 38);
        }
    }
}

/// One room piece's `postProcess`.
fn draw_child(d: &Draw, ctx: &mut Ctx, random: &mut WorldgenRandom, rooms: &[Room], child: &Child) {
    let (g, l, k, lamp) = (d.b.gray, d.b.light, d.b.black, d.b.lamp);
    let room = || &rooms[child.room];
    let lower = || room().index / 25 > 0;
    match child.kind {
        Kind::Core => {
            d.on_fill(ctx, 1, 8, 0, 14, 8, 14, g);
            d.bx(ctx, 0, 7, 0, 0, 7, 15, l);
            d.bx(ctx, 15, 7, 0, 15, 7, 15, l);
            d.bx(ctx, 1, 7, 0, 15, 7, 0, l);
            d.bx(ctx, 1, 7, 15, 14, 7, 15, l);
            for yx in 1..=6 {
                let s = if yx == 2 || yx == 6 { g } else { l };
                for x in [0, 15] {
                    d.bx(ctx, x, yx, 0, x, yx, 1, s);
                    d.bx(ctx, x, yx, 6, x, yx, 9, s);
                    d.bx(ctx, x, yx, 14, x, yx, 15, s);
                }
                d.bx(ctx, 1, yx, 0, 1, yx, 0, s);
                d.bx(ctx, 6, yx, 0, 9, yx, 0, s);
                d.bx(ctx, 14, yx, 0, 14, yx, 0, s);
                d.bx(ctx, 1, yx, 15, 14, yx, 15, s);
            }
            d.bx(ctx, 6, 3, 6, 9, 6, 9, k);
            d.bx(ctx, 7, 4, 7, 8, 5, 8, d.b.gold);
            for yx in [3, 6] {
                for x in [6, 9] {
                    d.put(ctx, lamp, x, yx, 6);
                    d.put(ctx, lamp, x, yx, 9);
                }
            }
            let boxes: [(i32, i32, i32, i32, i32, i32); 28] = [
                (5, 1, 6, 5, 2, 6),
                (5, 1, 9, 5, 2, 9),
                (10, 1, 6, 10, 2, 6),
                (10, 1, 9, 10, 2, 9),
                (6, 1, 5, 6, 2, 5),
                (9, 1, 5, 9, 2, 5),
                (6, 1, 10, 6, 2, 10),
                (9, 1, 10, 9, 2, 10),
                (5, 2, 5, 5, 6, 5),
                (5, 2, 10, 5, 6, 10),
                (10, 2, 5, 10, 6, 5),
                (10, 2, 10, 10, 6, 10),
                (5, 7, 1, 5, 7, 6),
                (10, 7, 1, 10, 7, 6),
                (5, 7, 9, 5, 7, 14),
                (10, 7, 9, 10, 7, 14),
                (1, 7, 5, 6, 7, 5),
                (1, 7, 10, 6, 7, 10),
                (9, 7, 5, 14, 7, 5),
                (9, 7, 10, 14, 7, 10),
                (2, 1, 2, 2, 1, 3),
                (3, 1, 2, 3, 1, 2),
                (13, 1, 2, 13, 1, 3),
                (12, 1, 2, 12, 1, 2),
                (2, 1, 12, 2, 1, 13),
                (3, 1, 13, 3, 1, 13),
                (13, 1, 12, 13, 1, 13),
                (12, 1, 13, 12, 1, 13),
            ];
            for (x0, y0, z0, x1, y1, z1) in boxes {
                d.bx(ctx, x0, y0, z0, x1, y1, z1, l);
            }
        }
        Kind::DoubleX => {
            let west = child.room;
            let east = conn(rooms, west, EAST);
            if lower() {
                d.floor(ctx, 8, 0, rooms[east].has_opening[DOWN]);
                d.floor(ctx, 0, 0, rooms[west].has_opening[DOWN]);
            }
            if rooms[west].connections[UP].is_none() {
                d.on_fill(ctx, 1, 4, 1, 7, 4, 6, g);
            }
            if rooms[east].connections[UP].is_none() {
                d.on_fill(ctx, 8, 4, 1, 14, 4, 6, g);
            }
            for (y, s) in [(3, l), (2, g), (1, l)] {
                d.bx(ctx, 0, y, 0, 0, y, 7, s);
                d.bx(ctx, 15, y, 0, 15, y, 7, s);
                d.bx(ctx, 1, y, 0, 15, y, 0, s);
                d.bx(ctx, 1, y, 7, 14, y, 7, s);
            }
            d.bx(ctx, 5, 1, 0, 10, 1, 4, l);
            d.bx(ctx, 6, 2, 0, 9, 2, 3, g);
            d.bx(ctx, 5, 3, 0, 10, 3, 4, l);
            d.put(ctx, lamp, 6, 2, 3);
            d.put(ctx, lamp, 9, 2, 3);
            let (w, e) = (&rooms[west].has_opening, &rooms[east].has_opening);
            if w[SOUTH] {
                d.water(ctx, 3, 1, 0, 4, 2, 0);
            }
            if w[NORTH] {
                d.water(ctx, 3, 1, 7, 4, 2, 7);
            }
            if w[WEST] {
                d.water(ctx, 0, 1, 3, 0, 2, 4);
            }
            if e[SOUTH] {
                d.water(ctx, 11, 1, 0, 12, 2, 0);
            }
            if e[NORTH] {
                d.water(ctx, 11, 1, 7, 12, 2, 7);
            }
            if e[EAST] {
                d.water(ctx, 15, 1, 3, 15, 2, 4);
            }
        }
        Kind::DoubleXY => {
            let west = child.room;
            let east = conn(rooms, west, EAST);
            let west_up = conn(rooms, west, UP);
            let east_up = conn(rooms, east, UP);
            if lower() {
                d.floor(ctx, 8, 0, rooms[east].has_opening[DOWN]);
                d.floor(ctx, 0, 0, rooms[west].has_opening[DOWN]);
            }
            if rooms[west_up].connections[UP].is_none() {
                d.on_fill(ctx, 1, 8, 1, 7, 8, 6, g);
            }
            if rooms[east_up].connections[UP].is_none() {
                d.on_fill(ctx, 8, 8, 1, 14, 8, 6, g);
            }
            for y in 1..=7 {
                let s = if y == 2 || y == 6 { g } else { l };
                d.bx(ctx, 0, y, 0, 0, y, 7, s);
                d.bx(ctx, 15, y, 0, 15, y, 7, s);
                d.bx(ctx, 1, y, 0, 15, y, 0, s);
                d.bx(ctx, 1, y, 7, 14, y, 7, s);
            }
            let boxes: [(i32, i32, i32, i32, i32, i32); 13] = [
                (2, 1, 3, 2, 7, 4),
                (3, 1, 2, 4, 7, 2),
                (3, 1, 5, 4, 7, 5),
                (13, 1, 3, 13, 7, 4),
                (11, 1, 2, 12, 7, 2),
                (11, 1, 5, 12, 7, 5),
                (5, 1, 3, 5, 3, 4),
                (10, 1, 3, 10, 3, 4),
                (5, 7, 2, 10, 7, 5),
                (5, 5, 2, 5, 7, 2),
                (10, 5, 2, 10, 7, 2),
                (5, 5, 5, 5, 7, 5),
                (10, 5, 5, 10, 7, 5),
            ];
            for (x0, y0, z0, x1, y1, z1) in boxes {
                d.bx(ctx, x0, y0, z0, x1, y1, z1, l);
            }
            d.put(ctx, l, 6, 6, 2);
            d.put(ctx, l, 9, 6, 2);
            d.put(ctx, l, 6, 6, 5);
            d.put(ctx, l, 9, 6, 5);
            d.bx(ctx, 5, 4, 3, 6, 4, 4, l);
            d.bx(ctx, 9, 4, 3, 10, 4, 4, l);
            d.put(ctx, lamp, 5, 4, 2);
            d.put(ctx, lamp, 5, 4, 5);
            d.put(ctx, lamp, 10, 4, 2);
            d.put(ctx, lamp, 10, 4, 5);
            let (w, e) = (&rooms[west].has_opening, &rooms[east].has_opening);
            let (wu, eu) = (&rooms[west_up].has_opening, &rooms[east_up].has_opening);
            if w[SOUTH] {
                d.water(ctx, 3, 1, 0, 4, 2, 0);
            }
            if w[NORTH] {
                d.water(ctx, 3, 1, 7, 4, 2, 7);
            }
            if w[WEST] {
                d.water(ctx, 0, 1, 3, 0, 2, 4);
            }
            if e[SOUTH] {
                d.water(ctx, 11, 1, 0, 12, 2, 0);
            }
            if e[NORTH] {
                d.water(ctx, 11, 1, 7, 12, 2, 7);
            }
            if e[EAST] {
                d.water(ctx, 15, 1, 3, 15, 2, 4);
            }
            if wu[SOUTH] {
                d.water(ctx, 3, 5, 0, 4, 6, 0);
            }
            if wu[NORTH] {
                d.water(ctx, 3, 5, 7, 4, 6, 7);
            }
            if wu[WEST] {
                d.water(ctx, 0, 5, 3, 0, 6, 4);
            }
            if eu[SOUTH] {
                d.water(ctx, 11, 5, 0, 12, 6, 0);
            }
            if eu[NORTH] {
                d.water(ctx, 11, 5, 7, 12, 6, 7);
            }
            if eu[EAST] {
                d.water(ctx, 15, 5, 3, 15, 6, 4);
            }
        }
        Kind::DoubleY => {
            if lower() {
                d.floor(ctx, 0, 0, room().has_opening[DOWN]);
            }
            let above = conn(rooms, child.room, UP);
            if rooms[above].connections[UP].is_none() {
                d.on_fill(ctx, 1, 8, 1, 6, 8, 6, g);
            }
            let boxes: [(i32, i32, i32, i32, i32, i32); 12] = [
                (0, 4, 0, 0, 4, 7),
                (7, 4, 0, 7, 4, 7),
                (1, 4, 0, 6, 4, 0),
                (1, 4, 7, 6, 4, 7),
                (2, 4, 1, 2, 4, 2),
                (1, 4, 2, 1, 4, 2),
                (5, 4, 1, 5, 4, 2),
                (6, 4, 2, 6, 4, 2),
                (2, 4, 5, 2, 4, 6),
                (1, 4, 5, 1, 4, 5),
                (5, 4, 5, 5, 4, 6),
                (6, 4, 5, 6, 4, 5),
            ];
            for (x0, y0, z0, x1, y1, z1) in boxes {
                d.bx(ctx, x0, y0, z0, x1, y1, z1, l);
            }
            let mut definition = child.room;
            for y in [1, 5] {
                let h = &rooms[definition].has_opening;
                let z = 0;
                if h[SOUTH] {
                    d.bx(ctx, 2, y, z, 2, y + 2, z, l);
                    d.bx(ctx, 5, y, z, 5, y + 2, z, l);
                    d.bx(ctx, 3, y + 2, z, 4, y + 2, z, l);
                } else {
                    d.bx(ctx, 0, y, z, 7, y + 2, z, l);
                    d.bx(ctx, 0, y + 1, z, 7, y + 1, z, g);
                }
                let z = 7;
                if h[NORTH] {
                    d.bx(ctx, 2, y, z, 2, y + 2, z, l);
                    d.bx(ctx, 5, y, z, 5, y + 2, z, l);
                    d.bx(ctx, 3, y + 2, z, 4, y + 2, z, l);
                } else {
                    d.bx(ctx, 0, y, z, 7, y + 2, z, l);
                    d.bx(ctx, 0, y + 1, z, 7, y + 1, z, g);
                }
                let x = 0;
                if h[WEST] {
                    d.bx(ctx, x, y, 2, x, y + 2, 2, l);
                    d.bx(ctx, x, y, 5, x, y + 2, 5, l);
                    d.bx(ctx, x, y + 2, 3, x, y + 2, 4, l);
                } else {
                    d.bx(ctx, x, y, 0, x, y + 2, 7, l);
                    d.bx(ctx, x, y + 1, 0, x, y + 1, 7, g);
                }
                let x = 7;
                if h[EAST] {
                    d.bx(ctx, x, y, 2, x, y + 2, 2, l);
                    d.bx(ctx, x, y, 5, x, y + 2, 5, l);
                    d.bx(ctx, x, y + 2, 3, x, y + 2, 4, l);
                } else {
                    d.bx(ctx, x, y, 0, x, y + 2, 7, l);
                    d.bx(ctx, x, y + 1, 0, x, y + 1, 7, g);
                }
                definition = above;
            }
        }
        Kind::DoubleYZ => {
            let south = child.room;
            let north = conn(rooms, south, NORTH);
            let north_up = conn(rooms, north, UP);
            let south_up = conn(rooms, south, UP);
            if lower() {
                d.floor(ctx, 0, 8, rooms[north].has_opening[DOWN]);
                d.floor(ctx, 0, 0, rooms[south].has_opening[DOWN]);
            }
            if rooms[south_up].connections[UP].is_none() {
                d.on_fill(ctx, 1, 8, 1, 6, 8, 7, g);
            }
            if rooms[north_up].connections[UP].is_none() {
                d.on_fill(ctx, 1, 8, 8, 6, 8, 14, g);
            }
            for y in 1..=7 {
                let s = if y == 2 || y == 6 { g } else { l };
                d.bx(ctx, 0, y, 0, 0, y, 15, s);
                d.bx(ctx, 7, y, 0, 7, y, 15, s);
                d.bx(ctx, 1, y, 0, 6, y, 0, s);
                d.bx(ctx, 1, y, 15, 6, y, 15, s);
            }
            for y in 1..=7 {
                let s = if y == 2 || y == 6 { lamp } else { k };
                d.bx(ctx, 3, y, 7, 4, y, 8, s);
            }
            let (s, n) = (&rooms[south].has_opening, &rooms[north].has_opening);
            let (su, nu) = (&rooms[south_up].has_opening, &rooms[north_up].has_opening);
            if s[SOUTH] {
                d.water(ctx, 3, 1, 0, 4, 2, 0);
            }
            if s[EAST] {
                d.water(ctx, 7, 1, 3, 7, 2, 4);
            }
            if s[WEST] {
                d.water(ctx, 0, 1, 3, 0, 2, 4);
            }
            if n[NORTH] {
                d.water(ctx, 3, 1, 15, 4, 2, 15);
            }
            if n[WEST] {
                d.water(ctx, 0, 1, 11, 0, 2, 12);
            }
            if n[EAST] {
                d.water(ctx, 7, 1, 11, 7, 2, 12);
            }
            if su[SOUTH] {
                d.water(ctx, 3, 5, 0, 4, 6, 0);
            }
            if su[EAST] {
                d.water(ctx, 7, 5, 3, 7, 6, 4);
                d.bx(ctx, 5, 4, 2, 6, 4, 5, l);
                d.bx(ctx, 6, 1, 2, 6, 3, 2, l);
                d.bx(ctx, 6, 1, 5, 6, 3, 5, l);
            }
            if su[WEST] {
                d.water(ctx, 0, 5, 3, 0, 6, 4);
                d.bx(ctx, 1, 4, 2, 2, 4, 5, l);
                d.bx(ctx, 1, 1, 2, 1, 3, 2, l);
                d.bx(ctx, 1, 1, 5, 1, 3, 5, l);
            }
            if nu[NORTH] {
                d.water(ctx, 3, 5, 15, 4, 6, 15);
            }
            if nu[WEST] {
                d.water(ctx, 0, 5, 11, 0, 6, 12);
                d.bx(ctx, 1, 4, 10, 2, 4, 13, l);
                d.bx(ctx, 1, 1, 10, 1, 3, 10, l);
                d.bx(ctx, 1, 1, 13, 1, 3, 13, l);
            }
            if nu[EAST] {
                d.water(ctx, 7, 5, 11, 7, 6, 12);
                d.bx(ctx, 5, 4, 10, 6, 4, 13, l);
                d.bx(ctx, 6, 1, 10, 6, 3, 10, l);
                d.bx(ctx, 6, 1, 13, 6, 3, 13, l);
            }
        }
        Kind::DoubleZ => {
            let south = child.room;
            let north = conn(rooms, south, NORTH);
            if lower() {
                d.floor(ctx, 0, 8, rooms[north].has_opening[DOWN]);
                d.floor(ctx, 0, 0, rooms[south].has_opening[DOWN]);
            }
            if rooms[south].connections[UP].is_none() {
                d.on_fill(ctx, 1, 4, 1, 6, 4, 7, g);
            }
            if rooms[north].connections[UP].is_none() {
                d.on_fill(ctx, 1, 4, 8, 6, 4, 14, g);
            }
            for (y, s) in [(3, l), (2, g), (1, l)] {
                d.bx(ctx, 0, y, 0, 0, y, 15, s);
                d.bx(ctx, 7, y, 0, 7, y, 15, s);
                d.bx(ctx, 1, y, 0, 7, y, 0, s);
                d.bx(ctx, 1, y, 15, 6, y, 15, s);
            }
            let boxes: [(i32, i32, i32, i32, i32, i32); 16] = [
                (1, 1, 1, 1, 1, 2),
                (6, 1, 1, 6, 1, 2),
                (1, 3, 1, 1, 3, 2),
                (6, 3, 1, 6, 3, 2),
                (1, 1, 13, 1, 1, 14),
                (6, 1, 13, 6, 1, 14),
                (1, 3, 13, 1, 3, 14),
                (6, 3, 13, 6, 3, 14),
                (2, 1, 6, 2, 3, 6),
                (5, 1, 6, 5, 3, 6),
                (2, 1, 9, 2, 3, 9),
                (5, 1, 9, 5, 3, 9),
                (3, 2, 6, 4, 2, 6),
                (3, 2, 9, 4, 2, 9),
                (2, 2, 7, 2, 2, 8),
                (5, 2, 7, 5, 2, 8),
            ];
            for (x0, y0, z0, x1, y1, z1) in boxes {
                d.bx(ctx, x0, y0, z0, x1, y1, z1, l);
            }
            d.put(ctx, lamp, 2, 2, 5);
            d.put(ctx, lamp, 5, 2, 5);
            d.put(ctx, lamp, 2, 2, 10);
            d.put(ctx, lamp, 5, 2, 10);
            d.put(ctx, l, 2, 3, 5);
            d.put(ctx, l, 5, 3, 5);
            d.put(ctx, l, 2, 3, 10);
            d.put(ctx, l, 5, 3, 10);
            let (s, n) = (&rooms[south].has_opening, &rooms[north].has_opening);
            if s[SOUTH] {
                d.water(ctx, 3, 1, 0, 4, 2, 0);
            }
            if s[EAST] {
                d.water(ctx, 7, 1, 3, 7, 2, 4);
            }
            if s[WEST] {
                d.water(ctx, 0, 1, 3, 0, 2, 4);
            }
            if n[NORTH] {
                d.water(ctx, 3, 1, 15, 4, 2, 15);
            }
            if n[WEST] {
                d.water(ctx, 0, 1, 11, 0, 2, 12);
            }
            if n[EAST] {
                d.water(ctx, 7, 1, 11, 7, 2, 12);
            }
        }
        Kind::Entry => {
            d.bx(ctx, 0, 3, 0, 2, 3, 7, l);
            d.bx(ctx, 5, 3, 0, 7, 3, 7, l);
            d.bx(ctx, 0, 2, 0, 1, 2, 7, l);
            d.bx(ctx, 6, 2, 0, 7, 2, 7, l);
            d.bx(ctx, 0, 1, 0, 0, 1, 7, l);
            d.bx(ctx, 7, 1, 0, 7, 1, 7, l);
            d.bx(ctx, 0, 1, 7, 7, 3, 7, l);
            d.bx(ctx, 1, 1, 0, 2, 3, 0, l);
            d.bx(ctx, 5, 1, 0, 6, 3, 0, l);
            let h = &room().has_opening;
            if h[NORTH] {
                d.water(ctx, 3, 1, 7, 4, 2, 7);
            }
            if h[WEST] {
                d.water(ctx, 0, 1, 3, 1, 2, 4);
            }
            if h[EAST] {
                d.water(ctx, 6, 1, 3, 7, 2, 4);
            }
        }
        Kind::Penthouse => {
            d.bx(ctx, 2, -1, 2, 11, -1, 11, l);
            d.bx(ctx, 0, -1, 0, 1, -1, 11, g);
            d.bx(ctx, 12, -1, 0, 13, -1, 11, g);
            d.bx(ctx, 2, -1, 0, 11, -1, 1, g);
            d.bx(ctx, 2, -1, 12, 11, -1, 13, g);
            d.bx(ctx, 0, 0, 0, 0, 0, 13, l);
            d.bx(ctx, 13, 0, 0, 13, 0, 13, l);
            d.bx(ctx, 1, 0, 0, 12, 0, 0, l);
            d.bx(ctx, 1, 0, 13, 12, 0, 13, l);
            for i in (2..=11).step_by(3) {
                d.put(ctx, lamp, 0, 0, i);
                d.put(ctx, lamp, 13, 0, i);
                d.put(ctx, lamp, i, 0, 0);
            }
            d.bx(ctx, 2, 0, 3, 4, 0, 9, l);
            d.bx(ctx, 9, 0, 3, 11, 0, 9, l);
            d.bx(ctx, 4, 0, 9, 9, 0, 11, l);
            d.put(ctx, l, 5, 0, 8);
            d.put(ctx, l, 8, 0, 8);
            d.put(ctx, l, 10, 0, 10);
            d.put(ctx, l, 3, 0, 10);
            d.bx(ctx, 3, 0, 3, 3, 0, 7, k);
            d.bx(ctx, 10, 0, 3, 10, 0, 7, k);
            d.bx(ctx, 6, 0, 10, 7, 0, 10, k);
            for x in [3, 10] {
                for z in (2..=8).step_by(3) {
                    d.bx(ctx, x, 0, z, x, 2, z, l);
                }
            }
            d.bx(ctx, 5, 0, 10, 5, 2, 10, l);
            d.bx(ctx, 8, 0, 10, 8, 2, 10, l);
            d.bx(ctx, 6, -1, 7, 7, -1, 8, k);
            d.water(ctx, 6, -1, 3, 7, -1, 4);
            d.spawn_elder(ctx, 6, 1, 6);
        }
        Kind::Simple { design } => {
            let h = room().has_opening;
            if lower() {
                d.floor(ctx, 0, 0, h[DOWN]);
            }
            if room().connections[UP].is_none() {
                d.on_fill(ctx, 1, 4, 1, 6, 4, 6, g);
            }
            let center_pillar = design != 0 && random.next_bool() && !h[DOWN] && !h[UP] && room().count_openings() > 1;
            match design {
                0 => {
                    d.bx(ctx, 0, 1, 0, 2, 1, 2, l);
                    d.bx(ctx, 0, 3, 0, 2, 3, 2, l);
                    d.bx(ctx, 0, 2, 0, 0, 2, 2, g);
                    d.bx(ctx, 1, 2, 0, 2, 2, 0, g);
                    d.put(ctx, lamp, 1, 2, 1);
                    d.bx(ctx, 5, 1, 0, 7, 1, 2, l);
                    d.bx(ctx, 5, 3, 0, 7, 3, 2, l);
                    d.bx(ctx, 7, 2, 0, 7, 2, 2, g);
                    d.bx(ctx, 5, 2, 0, 6, 2, 0, g);
                    d.put(ctx, lamp, 6, 2, 1);
                    d.bx(ctx, 0, 1, 5, 2, 1, 7, l);
                    d.bx(ctx, 0, 3, 5, 2, 3, 7, l);
                    d.bx(ctx, 0, 2, 5, 0, 2, 7, g);
                    d.bx(ctx, 1, 2, 7, 2, 2, 7, g);
                    d.put(ctx, lamp, 1, 2, 6);
                    d.bx(ctx, 5, 1, 5, 7, 1, 7, l);
                    d.bx(ctx, 5, 3, 5, 7, 3, 7, l);
                    d.bx(ctx, 7, 2, 5, 7, 2, 7, g);
                    d.bx(ctx, 5, 2, 7, 6, 2, 7, g);
                    d.put(ctx, lamp, 6, 2, 6);
                    if h[SOUTH] {
                        d.bx(ctx, 3, 3, 0, 4, 3, 0, l);
                    } else {
                        d.bx(ctx, 3, 3, 0, 4, 3, 1, l);
                        d.bx(ctx, 3, 2, 0, 4, 2, 0, g);
                        d.bx(ctx, 3, 1, 0, 4, 1, 1, l);
                    }
                    if h[NORTH] {
                        d.bx(ctx, 3, 3, 7, 4, 3, 7, l);
                    } else {
                        d.bx(ctx, 3, 3, 6, 4, 3, 7, l);
                        d.bx(ctx, 3, 2, 7, 4, 2, 7, g);
                        d.bx(ctx, 3, 1, 6, 4, 1, 7, l);
                    }
                    if h[WEST] {
                        d.bx(ctx, 0, 3, 3, 0, 3, 4, l);
                    } else {
                        d.bx(ctx, 0, 3, 3, 1, 3, 4, l);
                        d.bx(ctx, 0, 2, 3, 0, 2, 4, g);
                        d.bx(ctx, 0, 1, 3, 1, 1, 4, l);
                    }
                    if h[EAST] {
                        d.bx(ctx, 7, 3, 3, 7, 3, 4, l);
                    } else {
                        d.bx(ctx, 6, 3, 3, 7, 3, 4, l);
                        d.bx(ctx, 7, 2, 3, 7, 2, 4, g);
                        d.bx(ctx, 6, 1, 3, 7, 1, 4, l);
                    }
                }
                1 => {
                    d.bx(ctx, 2, 1, 2, 2, 3, 2, l);
                    d.bx(ctx, 2, 1, 5, 2, 3, 5, l);
                    d.bx(ctx, 5, 1, 5, 5, 3, 5, l);
                    d.bx(ctx, 5, 1, 2, 5, 3, 2, l);
                    d.put(ctx, lamp, 2, 2, 2);
                    d.put(ctx, lamp, 2, 2, 5);
                    d.put(ctx, lamp, 5, 2, 5);
                    d.put(ctx, lamp, 5, 2, 2);
                    d.bx(ctx, 0, 1, 0, 1, 3, 0, l);
                    d.bx(ctx, 0, 1, 1, 0, 3, 1, l);
                    d.bx(ctx, 0, 1, 7, 1, 3, 7, l);
                    d.bx(ctx, 0, 1, 6, 0, 3, 6, l);
                    d.bx(ctx, 6, 1, 7, 7, 3, 7, l);
                    d.bx(ctx, 7, 1, 6, 7, 3, 6, l);
                    d.bx(ctx, 6, 1, 0, 7, 3, 0, l);
                    d.bx(ctx, 7, 1, 1, 7, 3, 1, l);
                    for (x, z) in [(1, 0), (0, 1), (1, 7), (0, 6), (6, 7), (7, 6), (6, 0), (7, 1)] {
                        d.put(ctx, g, x, 2, z);
                    }
                    if !h[SOUTH] {
                        d.bx(ctx, 1, 3, 0, 6, 3, 0, l);
                        d.bx(ctx, 1, 2, 0, 6, 2, 0, g);
                        d.bx(ctx, 1, 1, 0, 6, 1, 0, l);
                    }
                    if !h[NORTH] {
                        d.bx(ctx, 1, 3, 7, 6, 3, 7, l);
                        d.bx(ctx, 1, 2, 7, 6, 2, 7, g);
                        d.bx(ctx, 1, 1, 7, 6, 1, 7, l);
                    }
                    if !h[WEST] {
                        d.bx(ctx, 0, 3, 1, 0, 3, 6, l);
                        d.bx(ctx, 0, 2, 1, 0, 2, 6, g);
                        d.bx(ctx, 0, 1, 1, 0, 1, 6, l);
                    }
                    if !h[EAST] {
                        d.bx(ctx, 7, 3, 1, 7, 3, 6, l);
                        d.bx(ctx, 7, 2, 1, 7, 2, 6, g);
                        d.bx(ctx, 7, 1, 1, 7, 1, 6, l);
                    }
                }
                2 => {
                    for (y, s) in [(1, l), (2, k), (3, l)] {
                        d.bx(ctx, 0, y, 0, 0, y, 7, s);
                        d.bx(ctx, 7, y, 0, 7, y, 7, s);
                        d.bx(ctx, 1, y, 0, 6, y, 0, s);
                        d.bx(ctx, 1, y, 7, 6, y, 7, s);
                    }
                    d.bx(ctx, 0, 1, 3, 0, 2, 4, k);
                    d.bx(ctx, 7, 1, 3, 7, 2, 4, k);
                    d.bx(ctx, 3, 1, 0, 4, 2, 0, k);
                    d.bx(ctx, 3, 1, 7, 4, 2, 7, k);
                    if h[SOUTH] {
                        d.water(ctx, 3, 1, 0, 4, 2, 0);
                    }
                    if h[NORTH] {
                        d.water(ctx, 3, 1, 7, 4, 2, 7);
                    }
                    if h[WEST] {
                        d.water(ctx, 0, 1, 3, 0, 2, 4);
                    }
                    if h[EAST] {
                        d.water(ctx, 7, 1, 3, 7, 2, 4);
                    }
                }
                _ => {}
            }
            if center_pillar {
                d.bx(ctx, 3, 1, 3, 4, 1, 4, l);
                d.bx(ctx, 3, 2, 3, 4, 2, 4, g);
                d.bx(ctx, 3, 3, 3, 4, 3, 4, l);
            }
        }
        Kind::SimpleTop => {
            let h = room().has_opening;
            if lower() {
                d.floor(ctx, 0, 0, h[DOWN]);
            }
            if room().connections[UP].is_none() {
                d.on_fill(ctx, 1, 4, 1, 6, 4, 6, g);
            }
            for x in 1..=6 {
                for z in 1..=6 {
                    if random.next_i32_bound(3) != 0 {
                        let y0 = 2 + if random.next_i32_bound(4) == 0 { 0 } else { 1 };
                        d.bx(ctx, x, y0, z, x, 3, z, d.b.sponge);
                    }
                }
            }
            for (y, s) in [(1, l), (2, k), (3, l)] {
                d.bx(ctx, 0, y, 0, 0, y, 7, s);
                d.bx(ctx, 7, y, 0, 7, y, 7, s);
                d.bx(ctx, 1, y, 0, 6, y, 0, s);
                d.bx(ctx, 1, y, 7, 6, y, 7, s);
            }
            d.bx(ctx, 0, 1, 3, 0, 2, 4, k);
            d.bx(ctx, 7, 1, 3, 7, 2, 4, k);
            d.bx(ctx, 3, 1, 0, 4, 2, 0, k);
            d.bx(ctx, 3, 1, 7, 4, 2, 7, k);
            if h[SOUTH] {
                d.water(ctx, 3, 1, 0, 4, 2, 0);
            }
        }
        Kind::Wing { design: 0 } => {
            for i in 0..4 {
                d.bx(ctx, 10 - i, 3 - i, 20 - i, 12 + i, 3 - i, 20, l);
            }
            d.bx(ctx, 7, 0, 6, 15, 0, 16, l);
            d.bx(ctx, 6, 0, 6, 6, 3, 20, l);
            d.bx(ctx, 16, 0, 6, 16, 3, 20, l);
            d.bx(ctx, 7, 1, 7, 7, 1, 20, l);
            d.bx(ctx, 15, 1, 7, 15, 1, 20, l);
            d.bx(ctx, 7, 1, 6, 9, 3, 6, l);
            d.bx(ctx, 13, 1, 6, 15, 3, 6, l);
            d.bx(ctx, 8, 1, 7, 9, 1, 7, l);
            d.bx(ctx, 13, 1, 7, 14, 1, 7, l);
            d.bx(ctx, 9, 0, 5, 13, 0, 5, l);
            d.bx(ctx, 10, 0, 7, 12, 0, 7, k);
            d.bx(ctx, 8, 0, 10, 8, 0, 12, k);
            d.bx(ctx, 14, 0, 10, 14, 0, 12, k);
            for z in (7..=18).rev().step_by(3) {
                d.put(ctx, lamp, 6, 3, z);
                d.put(ctx, lamp, 16, 3, z);
            }
            d.put(ctx, lamp, 10, 0, 10);
            d.put(ctx, lamp, 12, 0, 10);
            d.put(ctx, lamp, 10, 0, 12);
            d.put(ctx, lamp, 12, 0, 12);
            d.put(ctx, lamp, 8, 3, 6);
            d.put(ctx, lamp, 14, 3, 6);
            for (x, z) in [(4, 4), (18, 4), (4, 18), (18, 18)] {
                d.put(ctx, l, x, 2, z);
                d.put(ctx, lamp, x, 1, z);
                d.put(ctx, l, x, 0, z);
            }
            d.put(ctx, l, 9, 7, 20);
            d.put(ctx, l, 13, 7, 20);
            d.bx(ctx, 6, 0, 21, 7, 4, 21, l);
            d.bx(ctx, 15, 0, 21, 16, 4, 21, l);
            d.spawn_elder(ctx, 11, 2, 16);
        }
        Kind::Wing { .. } => {
            d.bx(ctx, 9, 3, 18, 13, 3, 20, l);
            d.bx(ctx, 9, 0, 18, 9, 2, 18, l);
            d.bx(ctx, 13, 0, 18, 13, 2, 18, l);
            for x in [9, 13] {
                d.put(ctx, l, x, 6, 20);
                d.put(ctx, lamp, x, 5, 20);
                d.put(ctx, l, x, 4, 20);
            }
            d.bx(ctx, 7, 3, 7, 15, 3, 14, l);
            for x in [10, 12] {
                d.bx(ctx, x, 0, 10, x, 6, 10, l);
                d.bx(ctx, x, 0, 12, x, 6, 12, l);
                d.put(ctx, lamp, x, 0, 10);
                d.put(ctx, lamp, x, 0, 12);
                d.put(ctx, lamp, x, 4, 10);
                d.put(ctx, lamp, x, 4, 12);
            }
            for x in [8, 14] {
                d.bx(ctx, x, 0, 7, x, 2, 7, l);
                d.bx(ctx, x, 0, 14, x, 2, 14, l);
            }
            d.bx(ctx, 8, 3, 8, 8, 3, 13, k);
            d.bx(ctx, 14, 3, 8, 14, 3, 13, k);
            d.spawn_elder(ctx, 11, 5, 13);
        }
    }
}
