//! Pinned 26.3 PanicGoal dry-ground damage branch shared by animals.
//! Source: PanicGoal, DefaultRandomPos and DamageTypeTags.PANIC_CAUSES.
use crate::stroll::default_random_position;
use crate::walk_path::WalkProfile;
use glam::DVec3;
use minecraftoss_player::{rng::LegacyRandom, World};

#[derive(Clone, Debug, Default)]
pub struct PanicState {
    pub wanted: Option<DVec3>,
    pub running: bool,
}

impl PanicState {
    #[allow(clippy::too_many_arguments)]
    pub fn can_use(
        &mut self,
        cause_is_panic: bool,
        on_fire: bool,
        position: DVec3,
        world: &dyn World,
        profile: &WalkProfile,
        random: &mut LegacyRandom,
    ) -> bool {
        if !cause_is_panic {
            return false;
        }
        // Burning, it runs for water within five blocks (one up or down)
        // when nothing solid holds its feet (`lookForWater`); the target is
        // the water block's corner.
        if on_fire {
            let feet = (position.x.floor() as i32, position.y.floor() as i32, position.z.floor() as i32);
            if world.collision_boxes(feet).is_empty() {
                let water = manhattan_ordered(feet, 5, 1, 5, 11).into_iter().find(|&pos| {
                    world.block(pos).is_some_and(|b| b.id == "minecraft:water" || b.property("waterlogged") == Some("true"))
                });
                if let Some((x, y, z)) = water {
                    self.wanted = Some(DVec3::new(f64::from(x), f64::from(y), f64::from(z)));
                    return true;
                }
            }
        }
        // `PanicGoal.findRandomPosition`.
        self.wanted = default_random_position(world, profile, position, 5, 4, random);
        self.wanted.is_some()
    }
}

/// `BlockPos.manhattanOrdered` (`withinBoxByManhattanDistance`): the
/// blocks within reach of `origin`, nearest by Manhattan distance first,
/// each depth from low x to high x, low y to high y, a positive z before
/// its mirror.
pub fn manhattan_ordered(origin: (i32, i32, i32), reach_x: i32, reach_y: i32, reach_z: i32, max_depth: i32) -> Vec<(i32, i32, i32)> {
    let mut out = Vec::new();
    for depth in 0..=max_depth {
        let max_x = reach_x.min(depth);
        for x in -max_x..=max_x {
            let max_y = reach_y.min(depth - x.abs());
            for y in -max_y..=max_y {
                let z = depth - x.abs() - y.abs();
                if z <= reach_z {
                    out.push((origin.0 + x, origin.1 + y, origin.2 + z));
                    if z != 0 {
                        out.push((origin.0 + x, origin.1 + y, origin.2 - z));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manhattan_order_starts_at_the_origin_and_mirrors_z() {
        let order = manhattan_ordered((0, 0, 0), 5, 1, 5, 11);
        assert_eq!(&order[..6], &[(0, 0, 0), (-1, 0, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1), (0, 1, 0)]);
    }
}
