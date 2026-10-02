//! Block-resolution volumes one column at a time.
//!
//! Above the interpolated cells, a final-density tree is elementwise
//! arithmetic over 16x384x16 buffers: every node is a full pass over
//! several hundred kilobytes. Evaluated a column at a time the same nodes
//! work in buffers that stay in the L1 cache.
//!
//! Only the cell-level evaluations under `Interpolated` touch caches, so
//! they run first, over the same cell volumes and in the same order as the
//! whole-volume evaluation would run them; the columns then only read their
//! corners. Everything above them is position independent, so a column
//! gets exactly the values the whole volume would. Trees with anything
//! else at block level (caches, noises, splines, slices) are not fused.

use super::{Context, Id, Sampler, Volume};
use crate::density::sampler::ContextField;
use crate::mth;

/// The corner values of one `Interpolated` occurrence over the whole
/// volume's cells.
pub(super) struct Corners {
    cells: Volume,
    values: Vec<f32>,
}

/// Prepared corners, consumed in evaluation order by each column.
pub(super) struct Fused {
    corners: Vec<Corners>,
    next: usize,
}

impl Context<'_> {
    /// `volume` for a block volume, one column at a time where the tree
    /// allows it (with identical results either way).
    pub fn sample_columns(&mut self, id: Id, volume: &Volume) -> Vec<f32> {
        let mut out = vec![0.0; volume.len()];
        if volume.step != [1, 1, 1] || volume.size[1] < 2 || self.fused.is_some() || !self.fusable(id) {
            self.volume(id, &mut out, volume);
            return out;
        }
        let mut corners = Vec::new();
        self.prepare(id, volume, &mut corners);
        self.fused = Some(Fused { corners, next: 0 });
        // Tiles of TILE x TILE columns.
        const TILE: i32 = 4;
        let height = volume.size[1];
        let mut tile = self.acquire(0);
        for tz in (0..volume.size[2]).step_by(TILE as usize) {
            for tx in (0..volume.size[0]).step_by(TILE as usize) {
                let (sx, sz) = (TILE.min(volume.size[0] - tx), TILE.min(volume.size[2] - tz));
                let v = Volume::blocks([sx, height, sz], [volume.block_x(tx), volume.min[1], volume.block_z(tz)]);
                tile.clear();
                tile.resize(v.len(), 0.0);
                self.fused.as_mut().expect("set above").next = 0;
                self.volume(id, &mut tile, &v);
                for z in 0..sz {
                    for x in 0..sx {
                        let from = v.index(x, 0, z);
                        let to = volume.index(tx + x, 0, tz + z);
                        out[to..to + height as usize].copy_from_slice(&tile[from..from + height as usize]);
                    }
                }
            }
        }
        let column = tile;
        self.release(column);
        self.fused = None;
        out
    }

    /// Whether everything in `id`'s tree down to its `Interpolated` nodes
    /// is position independent.
    fn fusable(&self, id: Id) -> bool {
        match &self.program.samplers[id] {
            Sampler::Constant(_) | Sampler::GradientClamped { .. } | Sampler::GradientRepeat { .. } | Sampler::GradientMirrored { .. } => true,
            Sampler::Interpolated { .. } => true,
            Sampler::ContextBound { field: ContextField::Beardifier, .. } if self.beardifier.is_some() => true,
            Sampler::ContextBound { fallback, .. } => self.fusable(*fallback),
            Sampler::Abs(i)
            | Sampler::Square(i)
            | Sampler::Cube(i)
            | Sampler::Sqrt(i)
            | Sampler::LeakyRelu(i, _)
            | Sampler::Reciprocal(i)
            | Sampler::Negate(i)
            | Sampler::Squeeze(i)
            | Sampler::Log(i)
            | Sampler::Sign(i)
            | Sampler::RoundInteger(_, i)
            | Sampler::ConstAdd(i, _)
            | Sampler::ConstSub(_, i)
            | Sampler::ConstMul(i, _)
            | Sampler::ConstDiv(_, i)
            | Sampler::ConstMin(i, _)
            | Sampler::ConstMax(i, _)
            | Sampler::PowConstBase(_, i)
            | Sampler::PowConstExponent(i, _)
            | Sampler::Clamp(i, _, _)
            | Sampler::RangeChoiceConst { input: i, .. }
            | Sampler::BlendDensity(i) => self.fusable(*i),
            Sampler::Round(_, a, b)
            | Sampler::Add(a, b)
            | Sampler::Sub(a, b)
            | Sampler::Mul(a, b)
            | Sampler::Div(a, b)
            | Sampler::Min(a, b, _)
            | Sampler::Max(a, b, _)
            | Sampler::Pow(a, b)
            | Sampler::LerpConstFirst(a, _, b)
            | Sampler::LerpConstSecond(a, b, _) => self.fusable(*a) && self.fusable(*b),
            Sampler::Lerp(a, b, c) => self.fusable(*a) && self.fusable(*b) && self.fusable(*c),
            Sampler::RangeChoice { input, in_range, out_of_range, .. } => self.fusable(*input) && self.fusable(*in_range) && self.fusable(*out_of_range),
            Sampler::IntervalSelectSingle { input, below, above, .. } => self.fusable(*input) && self.fusable(*below) && self.fusable(*above),
            Sampler::IntervalSelect { input, samplers, .. } => self.fusable(*input) && samplers.iter().all(|s| self.fusable(*s)),
            _ => false,
        }
    }

    /// Evaluates each `Interpolated` input over the whole volume's cells,
    /// visiting children in the order `volume` evaluates them.
    fn prepare(&mut self, id: Id, v: &Volume, corners: &mut Vec<Corners>) {
        let program = self.program;
        match &program.samplers[id] {
            Sampler::Constant(_) | Sampler::GradientClamped { .. } | Sampler::GradientRepeat { .. } | Sampler::GradientMirrored { .. } => {}
            Sampler::ContextBound { field: ContextField::Beardifier, .. } if self.beardifier.is_some() => {}
            Sampler::ContextBound { fallback, .. } => self.prepare(*fallback, v, corners),
            Sampler::Interpolated { input, cell_xz, cell_y, .. } => {
                let cells = cell_volume(v, *cell_xz, *cell_y);
                let mut values = vec![0.0; cells.len()];
                self.volume(*input, &mut values, &cells);
                corners.push(Corners { cells, values });
            }
            Sampler::Abs(i)
            | Sampler::Square(i)
            | Sampler::Cube(i)
            | Sampler::Sqrt(i)
            | Sampler::LeakyRelu(i, _)
            | Sampler::Reciprocal(i)
            | Sampler::Negate(i)
            | Sampler::Squeeze(i)
            | Sampler::Log(i)
            | Sampler::Sign(i)
            | Sampler::RoundInteger(_, i)
            | Sampler::ConstAdd(i, _)
            | Sampler::ConstSub(_, i)
            | Sampler::ConstMul(i, _)
            | Sampler::ConstDiv(_, i)
            | Sampler::ConstMin(i, _)
            | Sampler::ConstMax(i, _)
            | Sampler::PowConstBase(_, i)
            | Sampler::PowConstExponent(i, _)
            | Sampler::Clamp(i, _, _)
            | Sampler::RangeChoiceConst { input: i, .. }
            | Sampler::BlendDensity(i) => self.prepare(*i, v, corners),
            // `combine`: left, then right.
            Sampler::Round(_, a, b)
            | Sampler::Add(a, b)
            | Sampler::Sub(a, b)
            | Sampler::Mul(a, b)
            | Sampler::Div(a, b)
            | Sampler::Min(a, b, _)
            | Sampler::Max(a, b, _)
            | Sampler::Pow(a, b)
            | Sampler::LerpConstFirst(a, _, b)
            | Sampler::LerpConstSecond(a, b, _) => {
                self.prepare(*a, v, corners);
                self.prepare(*b, v, corners);
            }
            Sampler::Lerp(a, f, s) => {
                self.prepare(*a, v, corners);
                self.prepare(*f, v, corners);
                self.prepare(*s, v, corners);
            }
            Sampler::RangeChoice { input, in_range, out_of_range, .. } => {
                self.prepare(*in_range, v, corners);
                self.prepare(*input, v, corners);
                self.prepare(*out_of_range, v, corners);
            }
            Sampler::IntervalSelectSingle { input, below, above, .. } => {
                self.prepare(*input, v, corners);
                self.prepare(*below, v, corners);
                self.prepare(*above, v, corners);
            }
            Sampler::IntervalSelect { input, samplers, .. } => {
                self.prepare(*input, v, corners);
                for s in samplers {
                    self.prepare(*s, v, corners);
                }
            }
            other => unreachable!("not fusable: {other:?}"),
        }
    }

    /// Inside `sample_columns`: the next prepared `Interpolated` for the
    /// tile `v`, as `interpolated_blocks` computes it over the whole volume.
    pub(super) fn fused_interpolated(&mut self, cell_xz: i32, cell_y: i32, inv_xz: f32, inv_y: f32, out: &mut [f32], v: &Volume) -> bool {
        let Some(fused) = &mut self.fused else { return false };
        let corners = &fused.corners[fused.next];
        fused.next += 1;
        let (cv, cb) = (&corners.cells, &corners.values);
        let (first_x, first_z) = (mth::floor_div(cv.min[0], cell_xz), mth::floor_div(cv.min[2], cell_xz));
        let min_cell_y = mth::floor_div(v.min[1], cell_y);
        let count_y = mth::floor_div(v.max_block(1), cell_y) - min_cell_y + 1;
        for cell_z in mth::floor_div(v.min[2], cell_xz)..=mth::floor_div(v.max_block(2), cell_xz) {
            let cz = cell_z - first_z;
            let nz = (cz + 1).min(cv.size[2] - 1);
            let z_origin = cv.block_z(cz);
            for cell_x in mth::floor_div(v.min[0], cell_xz)..=mth::floor_div(v.max_block(0), cell_xz) {
                let cx = cell_x - first_x;
                let nx = (cx + 1).min(cv.size[0] - 1);
                let x_origin = cv.block_x(cx);
                for cy in 0..count_y {
                    let ny = (cy + 1).min(cv.size[1] - 1);
                    let v000 = cb[cv.index(cx, cy, cz)];
                    let v100 = cb[cv.index(nx, cy, cz)];
                    let v001 = cb[cv.index(cx, cy, nz)];
                    let v101 = cb[cv.index(nx, cy, nz)];
                    let v010 = cb[cv.index(cx, ny, cz)];
                    let v110 = cb[cv.index(nx, ny, cz)];
                    let v011 = cb[cv.index(cx, ny, nz)];
                    let v111 = cb[cv.index(nx, ny, nz)];
                    let oy = cv.block_y(cy) - v.min[1];
                    let y0 = (-oy).max(0);
                    let y1 = cell_y.min(v.size[1] - oy) - 1;
                    for bz in z_origin.max(v.min[2])..=(z_origin + cell_xz - 1).min(v.max_block(2)) {
                        let alpha_z = (bz - z_origin) as f32 * inv_xz;
                        let v00 = mth::lerp(alpha_z, v000, v001);
                        let v01 = mth::lerp(alpha_z, v010, v011);
                        let v10 = mth::lerp(alpha_z, v100, v101);
                        let v11 = mth::lerp(alpha_z, v110, v111);
                        for bx in x_origin.max(v.min[0])..=(x_origin + cell_xz - 1).min(v.max_block(0)) {
                            let alpha_x = (bx - x_origin) as f32 * inv_xz;
                            let low = mth::lerp(alpha_x, v00, v10);
                            let high = mth::lerp(alpha_x, v01, v11);
                            let step = (high - low) * inv_y;
                            let mut value = low + step * y0 as f32;
                            let mut index = v.index(bx - v.min[0], oy + y0, bz - v.min[2]);
                            for _ in y0..=y1 {
                                out[index] = value;
                                index += 1;
                                value += step;
                            }
                        }
                    }
                }
            }
        }
        true
    }
}

/// The cell-corner volume `interpolated_blocks` samples for `v`.
pub(super) fn cell_volume(v: &Volume, cell_xz: i32, cell_y: i32) -> Volume {
    let min_cell = [mth::floor_div(v.min[0], cell_xz), mth::floor_div(v.min[1], cell_y), mth::floor_div(v.min[2], cell_xz)];
    let max_cell = [mth::floor_div(v.max_block(0), cell_xz), mth::floor_div(v.max_block(1), cell_y), mth::floor_div(v.max_block(2), cell_xz)];
    let count = [max_cell[0] - min_cell[0] + 1, max_cell[1] - min_cell[1] + 1, max_cell[2] - min_cell[2] + 1];
    let cells = [cell_xz, cell_y, cell_xz];
    let size = [0, 1, 2].map(|a| if mth::floor_mod(v.max_block(a), cells[a]) == 0 { count[a] } else { count[a] + 1 });
    Volume::new(size, [min_cell[0] * cell_xz, min_cell[1] * cell_y, min_cell[2] * cell_xz], cells)
}
