//! Chunk tracking views (`ChunkTrackingView`, 26.3).

use minecraftoss_core::ChunkPos;

/// `ChunkMap.MIN_VIEW_DISTANCE` / `MAX_VIEW_DISTANCE`.
pub const MIN_VIEW_DISTANCE: i32 = 2;
pub const MAX_VIEW_DISTANCE: i32 = 32;
/// The farthest this client's options let a player see: vanilla stops at
/// `MAX_VIEW_DISTANCE`, the render distance slider goes on to 64.
pub const EXTENDED_VIEW_DISTANCE: i32 = 64;

/// `ChunkTrackingView.isWithinDistance`. The rounded view is a circle of
/// radius `view_distance` grown by one chunk (`include_neighbors = false`,
/// entity tracking) or two chunks (`true`, the chunks sent to the client).
pub fn is_within_distance(center: ChunkPos, view_distance: i32, pos: ChunkPos, include_neighbors: bool) -> bool {
    let buffer = if include_neighbors { 2 } else { 1 };
    let dx = i64::from(0.max((pos.x - center.x).abs() - buffer));
    let dz = i64::from(0.max((pos.z - center.z).abs() - buffer));
    dx * dx + dz * dz < i64::from(view_distance) * i64::from(view_distance)
}

/// `ChunkTrackingView.Positioned`: the chunks one player is tracking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackingView {
    pub center: ChunkPos,
    pub view_distance: i32,
}

impl TrackingView {
    pub const fn new(center: ChunkPos, view_distance: i32) -> Self {
        Self { center, view_distance }
    }

    fn min_x(self) -> i32 {
        self.center.x - self.view_distance - 1
    }

    fn min_z(self) -> i32 {
        self.center.z - self.view_distance - 1
    }

    fn max_x(self) -> i32 {
        self.center.x + self.view_distance + 1
    }

    fn max_z(self) -> i32 {
        self.center.z + self.view_distance + 1
    }

    /// Whether the chunk is tracked (sent to the client).
    pub fn contains(self, pos: ChunkPos) -> bool {
        is_within_distance(self.center, self.view_distance, pos, true)
    }

    /// The narrower view used for entity tracking and chunk rendering.
    pub fn is_in_view_distance(self, pos: ChunkPos) -> bool {
        is_within_distance(self.center, self.view_distance, pos, false)
    }

    fn square_intersects(self, other: Self) -> bool {
        self.min_x() <= other.max_x() && self.max_x() >= other.min_x() && self.min_z() <= other.max_z() && self.max_z() >= other.min_z()
    }

    /// Every tracked chunk, X-major like `Positioned.forEach`.
    pub fn for_each(self, mut f: impl FnMut(ChunkPos)) {
        for x in self.min_x()..=self.max_x() {
            for z in self.min_z()..=self.max_z() {
                let pos = ChunkPos::new(x, z);
                if self.contains(pos) {
                    f(pos);
                }
            }
        }
    }

    /// `ChunkTrackingView.difference`. `None` is `ChunkTrackingView.EMPTY`.
    pub fn difference(from: Option<Self>, to: Option<Self>, mut enter: impl FnMut(ChunkPos), mut leave: impl FnMut(ChunkPos)) {
        if from == to {
            return;
        }
        match (from, to) {
            (Some(last), Some(next)) if last.square_intersects(next) => {
                let min_x = last.min_x().min(next.min_x());
                let min_z = last.min_z().min(next.min_z());
                let max_x = last.max_x().max(next.max_x());
                let max_z = last.max_z().max(next.max_z());
                for x in min_x..=max_x {
                    for z in min_z..=max_z {
                        let pos = ChunkPos::new(x, z);
                        let saw = last.contains(pos);
                        let sees = next.contains(pos);
                        if saw != sees {
                            if sees { enter(pos) } else { leave(pos) }
                        }
                    }
                }
            }
            _ => {
                if let Some(last) = from {
                    last.for_each(&mut leave);
                }
                if let Some(next) = to {
                    next.for_each(&mut enter);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracked_area_is_the_rounded_circle() {
        let view = TrackingView::new(ChunkPos::new(0, 0), 12);
        // Along an axis the tracked area reaches view distance + 1.
        assert!(view.contains(ChunkPos::new(13, 0)));
        assert!(!view.contains(ChunkPos::new(14, 0)));
        // (d - 2)^2 * 2 < 144 holds up to d = 10 on the diagonal.
        assert!(view.contains(ChunkPos::new(10, 10)));
        assert!(!view.contains(ChunkPos::new(11, 11)));
        assert!(view.is_in_view_distance(ChunkPos::new(12, 0)));
        assert!(!view.is_in_view_distance(ChunkPos::new(13, 0)));
        let mut count = 0;
        view.for_each(|_| count += 1);
        assert_eq!(count, 637);
    }

    #[test]
    fn difference_reports_only_changed_chunks() {
        let a = TrackingView::new(ChunkPos::new(0, 0), 4);
        let b = TrackingView::new(ChunkPos::new(1, 0), 4);
        let (mut entered, mut left) = (Vec::new(), Vec::new());
        TrackingView::difference(Some(a), Some(b), |p| entered.push(p), |p| left.push(p));
        assert!(!entered.is_empty() && !left.is_empty());
        assert!(entered.iter().all(|&p| b.contains(p) && !a.contains(p)));
        assert!(left.iter().all(|&p| a.contains(p) && !b.contains(p)));
        let mut all = 0;
        TrackingView::difference(None, Some(a), |_| all += 1, |_| panic!("nothing to leave"));
        let mut expected = 0;
        a.for_each(|_| expected += 1);
        assert_eq!(all, expected);
    }
}
