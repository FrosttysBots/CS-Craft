//! What every living mob suffers (pinned 26.3 `LivingEntity`): its air
//! under water (`baseTick`: a breath a tick for those that cannot breathe
//! there, 2 drowning damage at -20, four back a tick elsewhere) and the
//! damage of a fall (`Entity.checkFallDamage` → `Block.fallOn` →
//! `causeFallDamage`: the fall less three safe blocks, times the block's
//! multiplier, with the fall sounds played first).
use super::*;

/// `LivingEntity.getMaxAirSupply`.
pub(crate) const MAX_AIR: i32 = 300;

/// `LivingEntity.baseTick`'s air: returns whether it drowns this tick. The
/// eyes must be in water (`isEyeInFluid`); `breathes_water` is
/// `#can_breathe_under_water` (the undead among these mobs).
pub(crate) fn breathe(body: &mut Body, world: &impl World, eye_height: f32, breathes_water: bool) -> bool {
    if FluidFrame::eye_in_water(world, body.position, eye_height) && !breathes_water {
        // `decreaseAirSupply`: no oxygen bonus here.
        body.air -= 1;
        if body.air <= -20 {
            body.air = 0;
            return true;
        }
    } else if body.air < MAX_AIR {
        body.air = (body.air + 4).min(MAX_AIR);
    }
    false
}

/// `LivingEntity.aiStep` for a dying mob (`isImmobile`): no input, but it
/// still falls, sinks and slides, sounds its movement, and a hard landing
/// plays the fall sounds (its hurt then fails: it is dead). Returns how far
/// it fell when it landed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dying_travel(body: &mut Body, world: &impl World, yaw: f32, tick_count: i32, random: &mut LegacyRandom, voices: &mut Vec<(Voice, DVec3)>, sounds: super::emissions::MovementSounds, hostile: Option<bool>) -> Option<f64> {
    body.trim_small_velocity();
    let fluid = FluidFrame::sample(world, body.position, body.width, body.height);
    let landed = if fluid.in_water() {
        body.travel_water(world, DVec3::ZERO, yaw);
        None
    } else if fluid.in_lava() {
        body.travel_lava(world, DVec3::ZERO, yaw, fluid.lava_height);
        None
    } else {
        body.travel_air(world, DVec3::ZERO, 0.0, yaw)
    };
    // `hostile` is none for mobs immune to falls.
    if let (Some(fallen), Some(hostile)) = (landed, hostile) {
        let _ = fall_damage(body, world, fallen, hostile, voices);
    }
    super::emissions::play_movement(body, tick_count, random, voices, sounds);
    landed
}

/// `Block.fallOn`'s damage multiplier for the block it lands on.
fn fall_multiplier(block: &minecraftoss_player::Block) -> f32 {
    match block.id.as_str() {
        "minecraft:hay_block" | "minecraft:honey_block" => 0.2,
        "minecraft:slime_block" => 0.0,
        _ => 1.0,
    }
}

/// `Block.getFallDistanceReduction`: beds and shelf mushrooms halve the
/// fall before it counts.
fn fall_reduction(block: &minecraftoss_player::Block) -> f64 {
    if block.id.ends_with("_bed") || block.id == "minecraft:shelf_mushroom" {
        f64::from(1.0_f32 - 0.5_f32)
    } else {
        1.0
    }
}

/// `FarmlandBlock.fallOn`, ahead of the fall's damage: landing on farmland
/// (`getOnPosLegacy`) after any fall draws from the level random, and a mob
/// bigger than 0.512 cubic blocks tramples it to dirt with mob griefing on
/// when the draw falls under the fall less half a block (`turnToBaseBlock`:
/// the dirt pushes it up onto its top).
pub(crate) fn fall_on_farmland(body: &mut Body, world: &mut impl World, fallen: f64, level_random: &mut LegacyRandom, griefing: bool) -> bool {
    if fallen <= 0.0 {
        return false;
    }
    let pos = body.on_pos(world, 0.2);
    if world.block(pos).is_none_or(|b| b.id != "minecraft:farmland") {
        return false;
    }
    if !(f64::from(level_random.next_float()) < fallen - 0.5 && griefing && body.width * body.width * body.height > 0.512) {
        return false;
    }
    // `pushEntitiesUp` (`teleportRelative`) onto the dirt's top, then the
    // dirt.
    let top = f64::from(pos.1) + 1.0;
    if body.position.y < top {
        body.position.y = top;
    }
    world.trample_farmland(pos, level_random);
    true
}

/// `LivingEntity.causeFallDamage` for a landing after `fallen` blocks on the
/// block at its feet (`getOnPosLegacy`): the damage, after the fall sound
/// (`getFallSounds`: small, big above 4; hostile for monsters) and the
/// block's (`playBlockFallSound`: under the feet by 0.2, at half its volume
/// and three quarters its pitch). None when the fall does no damage.
pub(crate) fn fall_damage(body: &Body, world: &impl World, fallen: f64, hostile: bool, voices: &mut Vec<(Voice, DVec3)>) -> Option<f32> {
    let landed_on = world.block(body.on_pos(world, 0.2));
    let multiplier = landed_on.as_ref().map_or(1.0, fall_multiplier);
    let fallen = fallen * landed_on.as_ref().map_or(1.0, fall_reduction);
    // `calculateFallPower` less `SAFE_FALL_DISTANCE` 3, times the multiplier
    // and `FALL_DAMAGE_MULTIPLIER` 1.
    let damage = ((fallen + 1.0e-6 - 3.0) * f64::from(multiplier)).floor() as i32;
    if damage <= 0 {
        return None;
    }
    let p = body.position;
    let event = match (hostile, damage > 4) {
        (true, false) => "entity.hostile.small_fall",
        (true, true) => "entity.hostile.big_fall",
        (false, false) => "entity.generic.small_fall",
        (false, true) => "entity.generic.big_fall",
    };
    voices.push((Voice::Event(event, 1.0, 1.0), p));
    let below = (p.x.floor() as i32, (p.y - f64::from(0.2_f32)).floor() as i32, p.z.floor() as i32);
    if let Some((event, volume, pitch)) = world.fall_sound(below) {
        voices.push((Voice::Step(event, volume * 0.5, pitch * 0.75), p));
    }
    Some(damage as f32)
}
