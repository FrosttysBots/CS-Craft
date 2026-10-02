//! Terrain adaptation around structures (vanilla `Beardifier`): a density
//! term added per block at the top of the Overworld's final density.
//!
//! Source-informed from the pinned 26.3 JAR, with float arithmetic in the
//! same order (the beard kernel, `Mth.fastInvSqrt` and the bury falloff).

use super::jigsaw::Projection;
use super::{References, Structures, TerrainAdjustment};
use crate::feature::template::BoundingBox;
use crate::noise::Volume;
use minecraftoss_core::{BlockPos, ChunkPos};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug)]
struct Rigid {
    bbox: BoundingBox,
    adaptation: TerrainAdjustment,
    ground_level_delta: i32,
}

#[derive(Clone, Copy, Debug)]
struct Junction {
    x: i32,
    ground_y: i32,
    z: i32,
}

/// `Beardifier.forStructuresInChunk`.
#[derive(Clone, Debug)]
pub struct Beardifier {
    rigids: Vec<Rigid>,
    junctions: Vec<Junction>,
    affected: BoundingBox,
}

/// `BEARD_KERNEL`: 24³ precomputed falloff values, z-major then x then y.
fn kernel() -> &'static [f32] {
    static KERNEL: OnceLock<Vec<f32>> = OnceLock::new();
    KERNEL.get_or_init(|| {
        let mut k = vec![0.0f32; 24 * 24 * 24];
        for zi in 0..24 {
            for xi in 0..24 {
                for yi in 0..24 {
                    let (dx, dy, dz) = (f64::from(xi - 12), f64::from(yi - 12) + 0.5, f64::from(zi - 12));
                    let distance_sqr = dx * dx + dy * dy + dz * dz;
                    k[(zi * 24 * 24 + xi * 24 + yi) as usize] = std::f64::consts::E.powf(-distance_sqr / 16.0) as f32;
                }
            }
        }
        k
    })
}

/// `Mth.fastInvSqrt`.
fn fast_inv_sqrt(x: f64) -> f64 {
    let half = 0.5 * x;
    let i = 6_910_469_410_427_058_090_i64 - (x.to_bits() as i64 >> 1);
    let y = f64::from_bits(i as u64);
    y * (1.5 - half * y * y)
}

fn beard_contribution(dx: i32, dy: i32, dz: i32, y_to_ground: i32) -> f32 {
    let (xi, yi, zi) = (dx + 12, dy + 12, dz + 12);
    if !(0..24).contains(&xi) || !(0..24).contains(&yi) || !(0..24).contains(&zi) {
        return 0.0;
    }
    let dy_offset = y_to_ground as f32 + 0.5;
    let (fx, fz) = (dx as f32, dz as f32);
    let distance_sqr = fx * fx + dy_offset * dy_offset + fz * fz;
    let value = -dy_offset * (fast_inv_sqrt(f64::from(distance_sqr / 2.0)) as f32) / 2.0;
    value * kernel()[(zi * 24 * 24 + xi * 24 + yi) as usize]
}

fn bury_contribution(dx: f32, dy: f32, dz: f32) -> f32 {
    let distance_sq = dx * dx + dy * dy + dz * dz;
    if distance_sq >= 36.0 { 0.0 } else { 1.0 - (f64::from(distance_sq).sqrt() as f32) / 6.0 }
}

impl Beardifier {
    /// The pieces and junctions near a chunk from the starts referencing it
    /// whose structure adapts the terrain; `None` when nothing does.
    pub fn for_chunk(structures: &Structures, references: &References, chunk: ChunkPos) -> Option<Self> {
        let (cx, cz) = (chunk.min_block_x(), chunk.min_block_z());
        let mut rigids = Vec::new();
        let mut junctions = Vec::new();
        let mut any: Option<BoundingBox> = None;
        let include = |any: &mut Option<BoundingBox>, b: BoundingBox| {
            *any = Some(match any {
                Some(a) => BoundingBox::encapsulating(a, &b),
                None => b,
            });
        };
        for (sid, starts) in references {
            let adaptation = structures.defs[usize::from(sid.0)].adaptation;
            if adaptation == TerrainAdjustment::None {
                continue;
            }
            for start in starts {
                let pieces = start.pieces.lock().expect("structure pieces");
                for piece in pieces.iter() {
                    if !piece.base().is_close_to_chunk(chunk, 12) {
                        continue;
                    }
                    if let Some(pool) = piece.pool_piece() {
                        if pool.element.projection() == Projection::Rigid {
                            rigids.push(Rigid { bbox: piece.base().bbox, adaptation, ground_level_delta: pool.ground_level_delta });
                            include(&mut any, piece.base().bbox);
                        }
                        for j in &pool.junctions {
                            if j.source_x > cx - 12 && j.source_z > cz - 12 && j.source_x < cx + 15 + 12 && j.source_z < cz + 15 + 12 {
                                junctions.push(Junction { x: j.source_x, ground_y: j.source_ground_y, z: j.source_z });
                                include(&mut any, BoundingBox::at(BlockPos::new(j.source_x, j.source_ground_y, j.source_z)));
                            }
                        }
                    } else {
                        rigids.push(Rigid { bbox: piece.base().bbox, adaptation, ground_level_delta: 0 });
                        include(&mut any, piece.base().bbox);
                    }
                }
            }
        }
        let affected = any?.inflated(24, 24, 24);
        Some(Self { rigids, junctions, affected })
    }

    /// `sampleValue`.
    pub fn value(&self, x: i32, y: i32, z: i32) -> f32 {
        if self.affected.is_inside(BlockPos::new(x, y, z)) { self.unchecked(x, y, z) } else { 0.0 }
    }

    /// `sampleVolume`: zero outside the affected box.
    pub fn volume(&self, out: &mut [f32], v: &Volume) {
        out.fill(0.0);
        let a = &self.affected;
        let (vx1, vy1, vz1) = (v.block_x(v.size[0] - 1), v.block_y(v.size[1] - 1), v.block_z(v.size[2] - 1));
        if a.max_x < v.min[0] || a.min_x > vx1 || a.max_y < v.min[1] || a.min_y > vy1 || a.max_z < v.min[2] || a.min_z > vz1 {
            return;
        }
        let lo = |a_min: i32, v_min: i32, step: i32| (a_min - v_min).max(0).div_euclid(step);
        let hi = |a_max: i32, v_min: i32, step: i32, size: i32| (size - 1).min((a_max - v_min).div_euclid(step));
        let (x0, y0, z0) = (lo(a.min_x, v.min[0], v.step[0]), lo(a.min_y, v.min[1], v.step[1]), lo(a.min_z, v.min[2], v.step[2]));
        let (x1, y1, z1) = (hi(a.max_x, v.min[0], v.step[0], v.size[0]), hi(a.max_y, v.min[1], v.step[1], v.size[1]), hi(a.max_z, v.min[2], v.step[2], v.size[2]));
        // Pieces and junctions 12 or more blocks away horizontally add
        // exactly +0.0 to a column (every kernel is zero there), and adding
        // +0.0 to a sum that started at +0.0 changes nothing, so each column
        // only visits the ones within reach, in the same order.
        let mut rigids: Vec<&Rigid> = Vec::with_capacity(self.rigids.len());
        let mut junctions: Vec<&Junction> = Vec::with_capacity(self.junctions.len());
        for z in z0..=z1 {
            let bz = v.block_z(z);
            for x in x0..=x1 {
                let bx = v.block_x(x);
                rigids.clear();
                rigids.extend(self.rigids.iter().filter(|r| {
                    let b = &r.bbox;
                    0.max((b.min_x - bx).max(bx - b.max_x)) < 12 && 0.max((b.min_z - bz).max(bz - b.max_z)) < 12
                }));
                junctions.clear();
                junctions.extend(self.junctions.iter().filter(|j| (-12..12).contains(&(bx - j.x)) && (-12..12).contains(&(bz - j.z))));
                for y in y0..=y1 {
                    out[v.index(x, y, z)] = Self::sum(rigids.iter().copied(), junctions.iter().copied(), bx, v.block_y(y), bz);
                }
            }
        }
    }

    fn unchecked(&self, x: i32, y: i32, z: i32) -> f32 {
        Self::sum(self.rigids.iter(), self.junctions.iter(), x, y, z)
    }

    fn sum<'a>(rigids: impl Iterator<Item = &'a Rigid>, junctions: impl Iterator<Item = &'a Junction>, x: i32, y: i32, z: i32) -> f32 {
        let mut value = 0.0f32;
        for rigid in rigids {
            let b = &rigid.bbox;
            let dx = 0.max((b.min_x - x).max(x - b.max_x));
            let dz = 0.max((b.min_z - z).max(z - b.max_z));
            let ground_y = b.min_y + rigid.ground_level_delta;
            let dy_to_ground = y - ground_y;
            let dy = match rigid.adaptation {
                TerrainAdjustment::None => 0,
                TerrainAdjustment::Bury | TerrainAdjustment::BeardThin => dy_to_ground,
                TerrainAdjustment::BeardBox => 0.max((ground_y - y).max(y - b.max_y)),
                TerrainAdjustment::Encapsulate => 0.max((b.min_y - y).max(y - b.max_y)),
            };
            value += match rigid.adaptation {
                TerrainAdjustment::None => 0.0,
                TerrainAdjustment::Bury => bury_contribution(dx as f32, dy as f32 / 2.0, dz as f32),
                TerrainAdjustment::BeardThin | TerrainAdjustment::BeardBox => beard_contribution(dx, dy, dz, dy_to_ground) * 0.8,
                TerrainAdjustment::Encapsulate => bury_contribution(dx as f32 / 2.0, dy as f32 / 2.0, dz as f32 / 2.0) * 0.8,
            };
        }
        for j in junctions {
            let (dx, dy, dz) = (x - j.x, y - j.ground_y, z - j.z);
            value += beard_contribution(dx, dy, dz, dy) * 0.4;
        }
        value
    }
}
