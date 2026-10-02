//! World and player spawn positions (vanilla 26.3
//! `MinecraftServer.setInitialSpawn` and `PlayerSpawnFinder`).
//!
//! Vanilla searches FULL chunks. Decoration is not implemented yet, so this
//! searches TERRAIN chunks: a tree or other feature at the first valid
//! column can move vanilla's result. The origin chunk from
//! `TerrainGenerator::spawn_origin` does not depend on chunk contents.
//! Collision is read from the block catalog's top occlusion face, which
//! agrees with the collision face for the full cubes TERRAIN places.

use crate::chunk_map::ChunkMap;
use minecraftoss_core::block::FaceShape;
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::{BlockStateId, Chunk, ChunkPos, Registries};

/// `GameRules.RESPAWN_RADIUS` default.
pub const DEFAULT_RESPAWN_RADIUS: i32 = 10;

/// `ChunkAccess.getHeight`: the heightmap's first free Y, minus one.
fn height(chunk: &Chunk, kind: HeightmapKind, x: i32, z: i32) -> i32 {
    chunk.heightmaps.get(kind, (x & 15) as usize, (z & 15) as usize) - 1
}

fn has_full_top(registries: &Registries, state: BlockStateId) -> bool {
    !registries.blocks.is_air(state) && *registries.blocks.face_shape(state, Direction::Up) == FaceShape::Full
}

/// `PlayerSpawnFinder.getLevelRespawnPos`: the block above the highest
/// full-topped block of a column, unless the column is flooded.
fn respawn_pos(registries: &Registries, chunk: &Chunk, x: i32, z: i32) -> Option<(i32, i32, i32)> {
    let top = height(chunk, HeightmapKind::MotionBlocking, x, z);
    if top < chunk.min_y() {
        return None;
    }
    let surface = height(chunk, HeightmapKind::WorldSurface, x, z);
    if surface <= top && surface > height(chunk, HeightmapKind::OceanFloor, x, z) {
        return None;
    }
    for y in (chunk.min_y()..=top + 1).rev() {
        let state = chunk.block((x & 15) as usize, y, (z & 15) as usize);
        if registries.blocks.has_fluid(state) {
            break;
        }
        if has_full_top(registries, state) {
            return Some((x, y + 1, z));
        }
    }
    None
}

/// `PlayerSpawnFinder.getSpawnPosInChunk`: the first valid column, X-major.
fn spawn_pos_in_chunk(registries: &Registries, chunk: &Chunk) -> Option<(i32, i32, i32)> {
    let (min_x, min_z) = (chunk.pos.min_block_x(), chunk.pos.min_block_z());
    for x in min_x..min_x + 16 {
        for z in min_z..min_z + 16 {
            if let Some(pos) = respawn_pos(registries, chunk, x, z) {
                return Some(pos);
            }
        }
    }
    None
}

/// `MinecraftServer.setInitialSpawn`: the spawn chunk's center at Y 64, then
/// the first valid position in an 11x11-chunk spiral around it.
pub fn world_spawn(map: &mut ChunkMap) -> (i32, i32, i32) {
    let generator = map.generator().clone();
    let registries = generator.registries.clone();
    let origin = generator.spawn_origin();
    let mut spawn = (origin.min_block_x() + 8, 64, origin.min_block_z() + 8);
    let (mut x, mut z, mut dx, mut dz) = (0i32, 0i32, 0i32, -1i32);
    for _ in 0..11 * 11 {
        if (-5..=5).contains(&x) && (-5..=5).contains(&z) {
            let chunk = map.load_now(ChunkPos::new(origin.x + x, origin.z + z));
            if let Some(pos) = spawn_pos_in_chunk(&registries, &chunk) {
                spawn = pos;
                break;
            }
        }
        if x == z || (x < 0 && x == -z) || (x > 0 && x == 1 - z) {
            (dx, dz) = (-dz, dx);
        }
        x += dx;
        z += dz;
    }
    spawn
}

/// Whether a standing player's box at a block fits: no solid or liquid
/// block in it (`noCollision` with liquids).
fn fits_player(registries: &Registries, map: &mut ChunkMap, (x, y, z): (i32, i32, i32)) -> bool {
    let chunk = map.load_now(ChunkPos::new(x >> 4, z >> 4));
    (y..=y + 1).all(|y| {
        let state = chunk.block((x & 15) as usize, y, (z & 15) as usize);
        registries.blocks.is_air(state) && registries.blocks.state(state).fluid.is_none()
    })
}

/// `PlayerSpawnFinder.findSpawn`: tries the positions within `radius` of the
/// world spawn in a scrambled order starting at `offset`, which vanilla
/// draws from an unseeded random. Returns the bottom center of the block.
pub fn player_spawn(map: &mut ChunkMap, world_spawn: (i32, i32, i32), radius: i32, offset: u32) -> (f64, f64, f64) {
    let generator = map.generator().clone();
    let registries = generator.registries.clone();
    let side = i64::from(radius) * 2 + 1;
    let count = (side * side).min(1024) as i32;
    let coprime = if count <= 16 { count - 1 } else { 17 };
    let offset = (offset % count as u32) as i32;
    for index in 0..count {
        let value = (offset + coprime * index) % count;
        let (x, z) = (
            world_spawn.0 + value % (radius * 2 + 1) - radius,
            world_spawn.2 + value / (radius * 2 + 1) - radius,
        );
        let chunk = map.load_now(ChunkPos::new(x >> 4, z >> 4));
        if let Some(pos) = respawn_pos(&registries, &chunk, x, z) {
            if fits_player(&registries, map, pos) {
                return (f64::from(pos.0) + 0.5, f64::from(pos.1), f64::from(pos.2) + 0.5);
            }
        }
    }
    // `fixupSpawnHeight`: up out of anything solid, then down to the ground.
    let (x, mut y, z) = world_spawn;
    let (min_y, max_y) = (generator.min_y, generator.min_y + generator.height - 1);
    while !fits_player(&registries, map, (x, y, z)) && y < max_y {
        y += 1;
    }
    y -= 1;
    while fits_player(&registries, map, (x, y, z)) && y > min_y {
        y -= 1;
    }
    (f64::from(x) + 0.5, f64::from(y + 1), f64::from(z) + 0.5)
}
