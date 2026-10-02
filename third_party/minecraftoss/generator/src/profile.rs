//! Profiling: summed time per generation phase over the whole process
//! (read by the chunk map's load reports and `gen_bench`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub const DENSITY: usize = 0;
pub const FILL_BLOCKS: usize = 1;
pub const SURFACE: usize = 2;
pub const CARVERS: usize = 3;
pub const AQUIFER_NEW: usize = 4;
pub const HEIGHTMAPS: usize = 5;
pub const SURFACE_RULES: usize = 6;
const NAMES: [&str; 7] = ["density", "fill blocks (aquifer)", "surface", "carvers", "aquifer setup", "heightmaps", "  surface: rule setup"];
static MICROS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
static COUNTS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];

pub fn add(phase: usize, since: Instant) {
    MICROS[phase].fetch_add(since.elapsed().as_micros() as u64, Ordering::Relaxed);
    COUNTS[phase].fetch_add(1, Ordering::Relaxed);
}

pub fn reset() {
    for i in 0..NAMES.len() {
        MICROS[i].store(0, Ordering::Relaxed);
        COUNTS[i].store(0, Ordering::Relaxed);
    }
}

/// Each phase's total seconds, count and mean.
pub fn report() -> String {
    let mut out = String::new();
    for (i, name) in NAMES.iter().enumerate() {
        let (micros, count) = (MICROS[i].load(Ordering::Relaxed), COUNTS[i].load(Ordering::Relaxed));
        out += &format!("    {name:32} {:8.2}s {count:7} x {:8.0} us\n", micros as f64 / 1e6, micros.checked_div(count).unwrap_or(0));
    }
    out
}
