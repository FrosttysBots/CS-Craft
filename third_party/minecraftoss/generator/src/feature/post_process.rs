//! `LevelChunk.postProcessGeneration`, run when a chunk starts block
//! ticking (its whole 3x3 at FULL): every position generation marked is
//! re-shaped against its neighbours, section by section in mark order.

use super::blocks::FluidType;
use super::template::update_from_neighbour_shapes;
use super::{Ctx, Library, Region};
use minecraftoss_core::{BlockPos, Chunk};

/// `setBlock` flags `postProcessGeneration` uses: no neighbour shape
/// updates, no re-marking.
const FLAGS: u32 = 276;

/// Post-processes the center of a 3x3 of FULL chunks (in `Region` order)
/// and returns it with its marks consumed.
///
/// A marked position holding fluid gets `FluidState.tick` in vanilla, which
/// is runtime fluid flow; here it is queued as a fluid tick at the front of
/// the chunk's tick list for the simulation to run first. Liquid blocks'
/// own tick (bubble columns) is left to the simulation the same way.
pub fn post_process(lib: &Library, mut chunks: Vec<Chunk>, world_seed: i64) -> Chunk {
    let marks = std::mem::take(&mut chunks[4].generation.post_processing);
    let (center, min_y) = (chunks[4].pos, chunks[4].min_y());
    // One list per section, sections bottom to top, each in mark order.
    let mut ordered: Vec<(usize, (i32, i32, i32))> = marks.into_iter().enumerate().collect();
    ordered.sort_by_key(|&(i, (_, y, _))| ((y - min_y) >> 4, i));
    let mut region = Region::new(center, chunks, lib.registries.clone(), world_seed);
    let mut fluid_ticks = Vec::new();
    {
        let mut ctx = Ctx { lib, region: &mut region };
        for (_, (x, y, z)) in ordered {
            let pos = BlockPos::new(x, y, z);
            let state = ctx.block(pos);
            if ctx.fluid(state) != FluidType::Empty {
                let fluid = super::region::fluid_id(lib.registries.blocks.state(state).fluid.as_ref()).to_owned();
                fluid_ticks.push(minecraftoss_core::chunk::ScheduledTick { pos: (x, y, z), fluid: true, id: fluid, delay: 0, priority: 0 });
            }
            let blocks = &lib.registries.blocks;
            if blocks.block(blocks.block_of(state)).is_a("LiquidBlock") {
                continue;
            }
            let new_state = update_from_neighbour_shapes(&mut ctx, state, pos);
            if new_state != state {
                ctx.set_block_flags(pos, new_state, FLAGS);
            }
        }
    }
    let mut chunk = region.into_chunks().swap_remove(4);
    // Ticks the fluid positions first, as the simulation must run them
    // before the chunk's scheduled ticks; one per fluid and position.
    let scheduled = std::mem::take(&mut chunk.generation.ticks);
    for tick in fluid_ticks.into_iter().chain(scheduled) {
        chunk.generation.schedule_tick(tick);
    }
    chunk
}
