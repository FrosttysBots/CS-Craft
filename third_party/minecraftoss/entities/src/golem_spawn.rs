//! Where a villager's summoned iron golem appears (pinned 26.3
//! `SpawnUtil.trySpawnMob(IRON_GOLEM, MOB_SUMMONED, level, villager, 10, 8,
//! 6, Strategy.LEGACY_IRON_GOLEM, false)`, `IronGolem.checkSpawnObstruction`
//! and `NaturalSpawner.isValidEmptySpawnBlock`): up to ten tries, each a
//! column eight blocks about (two draws from the level's random) searched
//! down from six above for a solid block with air or liquid over it; a
//! column that has one makes a golem, drawing its facing and
//! `Mob.finalizeSpawn`'s follow-range bonus and left-handed roll from the
//! level's random, and the golem stays there if it stands on a full top
//! face with room for its height and no living thing in the way.
use glam::DVec3;
use minecraftoss_player::rng::LegacyRandom;
use minecraftoss_player::{Block, Pos, World};

/// Tries, reach across and up or down (`spawnAttempts`, `spawnRangeXZ`,
/// `spawnRangeY`).
const ATTEMPTS: u32 = 10;
const RANGE_XZ: i32 = 8;
const RANGE_Y: i32 = 6;

fn is_air(block: Option<&Block>) -> bool {
    block.is_none_or(|b| matches!(b.id.as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"))
}

/// `BlockState.liquid()`: water, lava and bubble columns.
fn is_liquid(block: Option<&Block>) -> bool {
    block.is_some_and(|b| matches!(b.id.as_str(), "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column"))
}

/// `Strategy.LEGACY_IRON_GOLEM`: not a see-through or special block (glass
/// and panes, leaves, ice, cobwebs, cactus, TNT, glowstone, beacons, sea
/// lanterns, conduits), solid (or powder snow), with air or liquid above.
fn legacy_golem_ground(world: &dyn World, pos: Pos, block: Option<&Block>, above: Option<&Block>) -> bool {
    if let Some(b) = block {
        let id = b.id.as_str();
        let excluded = matches!(
            id,
            "minecraft:cobweb"
                | "minecraft:cactus"
                | "minecraft:glass_pane"
                | "minecraft:conduit"
                | "minecraft:ice"
                | "minecraft:tnt"
                | "minecraft:glowstone"
                | "minecraft:beacon"
                | "minecraft:sea_lantern"
                | "minecraft:frosted_ice"
                | "minecraft:tinted_glass"
                | "minecraft:glass"
        ) || id.ends_with("_stained_glass_pane")
            || id.ends_with("_stained_glass")
            || id.ends_with("_leaves");
        if excluded {
            return false;
        }
    }
    (is_air(above) || is_liquid(above)) && (world.solid(pos) || block.is_some_and(|b| b.id == "minecraft:powder_snow"))
}

/// `moveToPossibleSpawnPosition`: down from `pos` (six above the start)
/// through twelve more blocks for ground; the spot above it.
fn possible_spawn_position(world: &dyn World, pos: Pos) -> Option<Pos> {
    let (x, mut y, z) = pos;
    let mut above = world.block((x, y, z));
    for _ in -RANGE_Y..=RANGE_Y {
        y -= 1;
        let current = world.block((x, y, z));
        if legacy_golem_ground(world, (x, y, z), current.as_ref(), above.as_ref()) {
            return Some((x, y + 1, z));
        }
        above = current;
    }
    None
}

/// Whether a block's collision boxes fill its whole cube.
fn full_cube(boxes: &[[f64; 6]]) -> bool {
    boxes.len() == 1 && boxes[0] == [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]
}

/// `Block.isFaceFull(collisionShape, UP)`: the boxes reaching the top cover
/// it (checked on a sixteenth grid).
fn top_face_full(boxes: &[[f64; 6]]) -> bool {
    let top: Vec<&[f64; 6]> = boxes.iter().filter(|b| b[4] >= 1.0).collect();
    (0..16).all(|i| {
        (0..16).all(|k| {
            let (x, z) = ((f64::from(i) + 0.5) / 16.0, (f64::from(k) + 0.5) / 16.0);
            top.iter().any(|b| b[0] <= x && x <= b[3] && b[2] <= z && z <= b[5])
        })
    })
}

/// A block holding a fluid (`getFluidState` not empty).
fn holds_fluid(block: Option<&Block>) -> bool {
    block.is_some_and(|b| {
        matches!(
            b.id.as_str(),
            "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column" | "minecraft:kelp" | "minecraft:kelp_plant" | "minecraft:seagrass" | "minecraft:tall_seagrass"
        ) || b.property("waterlogged") == Some("true")
    })
}

/// `EntityType.isBlockDangerous` for the iron golem (not fire immune):
/// burning blocks (`NodeEvaluator.isBurningBlock`), wither roses, berry
/// bushes, cactus and powder snow.
fn dangerous(block: Option<&Block>) -> bool {
    block.is_some_and(|b| {
        let lit_campfire = matches!(b.id.as_str(), "minecraft:campfire" | "minecraft:soul_campfire") && b.property("lit") == Some("true");
        lit_campfire
            || matches!(
                b.id.as_str(),
                "minecraft:fire"
                    | "minecraft:soul_fire"
                    | "minecraft:lava"
                    | "minecraft:magma_block"
                    | "minecraft:lava_cauldron"
                    | "minecraft:wither_rose"
                    | "minecraft:sweet_berry_bush"
                    | "minecraft:cactus"
                    | "minecraft:powder_snow"
            )
    })
}

/// `NaturalSpawner.isValidEmptySpawnBlock` for the iron golem; the fluid
/// is not asked where the golem's feet go (`Fluids.EMPTY`).
fn valid_empty(world: &dyn World, pos: Pos, ask_fluid: bool) -> bool {
    let block = world.block(pos);
    if full_cube(&world.collision_boxes(pos)) {
        return false;
    }
    if block.as_ref().is_some_and(minecraftoss_player::redstone::is_signal_source) {
        return false;
    }
    if ask_fluid && holds_fluid(block.as_ref()) {
        return false;
    }
    // `#prevent_mob_spawning_inside`: rails.
    if block.as_ref().is_some_and(|b| b.id.ends_with("rail")) {
        return false;
    }
    !dangerous(block.as_ref())
}

/// `Mth.wrapDegrees(float)`.
fn wrap_degrees(degrees: f32) -> f32 {
    let mut r = degrees % 360.0;
    if r >= 180.0 {
        r -= 360.0;
    }
    if r < -180.0 {
        r += 360.0;
    }
    r
}

/// A summoned golem as it appears: where, its facing and its follow
/// range's spawn bonus (`RANDOM_SPAWN_BONUS_ID`, multiplying the base).
#[derive(Clone, Copy, Debug)]
pub struct SummonedGolem {
    pub at: DVec3,
    pub yaw: f32,
    pub follow_bonus: f64,
}

/// Where a summoned golem appears around `start` (the villager's block),
/// drawing from the level's random; `blocked` says whether a living thing's
/// box would meet the golem's (`isUnobstructed`).
pub fn try_spawn_golem(world: &dyn World, start: Pos, level_random: &mut LegacyRandom, blocked: impl Fn(DVec3, DVec3) -> bool) -> Option<SummonedGolem> {
    for _ in 0..ATTEMPTS {
        // `Mth.randomBetweenInclusive(random, -8, 8)` twice.
        let dx = level_random.next_int((RANGE_XZ * 2 + 1) as u32) as i32 - RANGE_XZ;
        let dz = level_random.next_int((RANGE_XZ * 2 + 1) as u32) as i32 - RANGE_XZ;
        let Some(pos) = possible_spawn_position(world, (start.0 + dx, start.1 + RANGE_Y, start.2 + dz)) else { continue };
        // `EntityType.create`: the facing, then `Mob.finalizeSpawn`'s
        // `random.triangle(0, 0.11485)` and left-handed roll, all from the
        // level's random.
        let yaw = wrap_degrees(level_random.next_float() * 360.0);
        let follow_bonus = 0.114_850_000_000_000_01 * (level_random.next_double() - level_random.next_double());
        let _left_handed = level_random.next_float() < 0.05;
        let at = DVec3::new(f64::from(pos.0) + 0.5, f64::from(pos.1), f64::from(pos.2) + 0.5);
        // `checkSpawnRules` holds (its walk values are all 0);
        // `IronGolem.checkSpawnObstruction`.
        let below = (pos.0, pos.1 - 1, pos.2);
        if !top_face_full(&world.collision_boxes(below)) {
            continue;
        }
        if !(1..3).all(|i| valid_empty(world, (pos.0, pos.1 + i, pos.2), true)) {
            continue;
        }
        if !valid_empty(world, pos, false) {
            continue;
        }
        let half = f64::from(crate::iron_golem::WIDTH) / 2.0;
        let (min, max) = (at - DVec3::new(half, 0.0, half), at + DVec3::new(half, f64::from(crate::iron_golem::HEIGHT), half));
        if blocked(min, max) {
            continue;
        }
        return Some(SummonedGolem { at, yaw, follow_bonus });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_degrees_as_mth_does() {
        assert_eq!(wrap_degrees(190.0), -170.0);
        assert_eq!(wrap_degrees(-190.0), 170.0);
        assert_eq!(wrap_degrees(359.0), -1.0);
        assert_eq!(wrap_degrees(90.0), 90.0);
    }

    #[test]
    fn top_faces_need_full_cover() {
        assert!(top_face_full(&[[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]));
        assert!(!top_face_full(&[[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]));
        assert!(top_face_full(&[[0.0, 0.5, 0.0, 0.5, 1.0, 1.0], [0.5, 0.5, 0.0, 1.0, 1.0, 1.0]]));
    }
}
