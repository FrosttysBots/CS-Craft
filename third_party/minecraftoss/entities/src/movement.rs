//! LivingEntity.aiStep/travelInAir/travelInWater/travelInLava and Entity.move, pinned Java 26.3.
//! Measured full-block collision, simple fluid travel and a spider's wall
//! climbing are present; currents, ladders, effects, block callbacks,
//! entity/world-border collision and nonzero restitution require integration.
use crate::fluid::FluidFrame;
use glam::DVec3;
use minecraftoss_player::{
    collision::{find_supporting_block, move_body, Bounds},
    minecraft_sin_cos, Block, Pos, World,
};

/// What a move sounded (`applyMovementEmissionAndPlaySound`).
#[derive(Clone, Debug, PartialEq)]
pub enum Emission {
    /// A step on a block: its step event at 0.15 of its volume, and its
    /// pitch (`playStepSound`; none where the world knows no sound for it),
    /// and whether the block chimes (`#minecraft:crystal_sound_blocks`).
    Step(Option<(String, f32, f32)>, bool),
    /// `waterSwimSound`: the mob's swim sound at this volume (its pitch
    /// takes two draws, `playSwimSound`).
    Swim(f32),
}

#[derive(Clone, Debug)]
pub struct Body {
    pub position: DVec3,
    /// `Entity.needsSync`: a jump, push or knockback this tick, which its
    /// tracker sends at once instead of waiting for its interval. Cleared
    /// as the next tick starts, where vanilla's trackers run.
    pub needs_sync: bool,
    pub velocity: DVec3,
    pub width: f32,
    pub height: f32,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub vertical_collision: bool,
    pub fall_distance: f64,
    pub step_height: f32,
    pub gravity: f64,
    /// `LivingEntity.onClimbable` for a spider: it met a wall last tick
    /// (`Spider.isClimbing`).
    pub climbing: bool,
    /// `Entity.moveDist` and `nextStep`: how far it has walked, and where
    /// its next step sounds.
    pub move_dist: f32,
    pub next_step: f32,
    /// `Entity.mainSupportingBlockPos` and `onGroundNoBlocks`: the block it
    /// stands on, the nearest one under its feet.
    pub supporting_block: Option<Pos>,
    pub on_ground_no_blocks: bool,
    /// What its last move sounded, for the mob to play.
    pub emission: Option<Emission>,
    /// `Entity.wasTouchingWater` (`isInWater`) as the fluid interaction
    /// last found it.
    pub touching_water: bool,
    /// The volume of a splash its last water check made
    /// (`doWaterSplashEffect`), for the mob to play.
    pub splash: Option<f32>,
    /// `Entity.isSwimming`: a swimming body makes no steps.
    pub swimming: bool,
    /// `crystalSoundIntensity` and `lastCrystalSoundPlayTick`: amethyst
    /// chimes fade in and out as it walks on and off.
    pub crystal_sound_intensity: f32,
    pub last_crystal_sound_tick: i32,
    /// `Entity.getAirSupply`: breath left under water (300 full).
    pub air: i32,
    /// `Entity.remainingFireTicks`: ticks of burning left (0 when not).
    pub fire_ticks: i32,
    /// `Entity.movementThisTick`: the moves made since the blocks last
    /// acted on it (`applyEffectsFromBlocks`).
    pub moves: Vec<crate::inside_blocks::Move>,
    /// `Entity.stuckSpeedMultiplier`: a cobweb's or bush's hold on the
    /// next move.
    pub stuck: Option<DVec3>,
}

impl Body {
    pub fn new(position: DVec3, width: f32, height: f32) -> Self {
        Self {
            position,
            needs_sync: false,
            velocity: DVec3::ZERO,
            width,
            height,
            on_ground: false,
            horizontal_collision: false,
            vertical_collision: false,
            fall_distance: 0.0,
            step_height: 0.6,
            gravity: 0.08,
            climbing: false,
            move_dist: 0.0,
            next_step: 1.0,
            supporting_block: None,
            on_ground_no_blocks: false,
            emission: None,
            touching_water: false,
            splash: None,
            swimming: false,
            crystal_sound_intensity: 0.0,
            last_crystal_sound_tick: 0,
            air: 300,
            fire_ticks: 0,
            moves: Vec::new(),
            stuck: None,
        }
    }

    /// `Entity.updateFluidInteraction`'s water state: in water the body
    /// forgets its fall, and entering it after its first tick splashes at
    /// a fifth of its speed (sideways motion counting a fifth).
    pub fn update_fluid(&mut self, world: &impl World, first_tick: bool) -> FluidFrame {
        let frame = FluidFrame::sample(world, self.position, self.width, self.height);
        if frame.in_water() {
            self.fall_distance = 0.0;
            if !self.touching_water && !first_tick {
                self.splash = Some((motion_speed(self.velocity) * 0.2).min(1.0));
            }
        }
        self.touching_water = frame.in_water();
        frame
    }

    /// `Entity.getOnPos(offset)`: the block `offset` under the feet, in the
    /// column of the supporting block when there is one (whose own block
    /// stands in for a wall, a fence gate, or a fence close under it).
    pub fn on_pos(&self, world: &impl World, offset: f32) -> Pos {
        let y = (self.position.y - f64::from(offset)).floor() as i32;
        let Some(pos) = self.supporting_block else {
            return (self.position.x.floor() as i32, y, self.position.z.floor() as i32);
        };
        if offset <= 1.0e-5 {
            return pos;
        }
        let below = world.block(pos);
        let id = below.as_ref().map_or("", |b| b.id.as_str());
        let fence = id.ends_with("_fence");
        if (offset > 0.5 || !fence) && !id.ends_with("_wall") && !id.ends_with("_fence_gate") {
            (pos.0, y, pos.2)
        } else {
            pos
        }
    }

    /// `Entity.checkSupportingBlock`: on the ground, the nearest block under
    /// the feet (or, finding none, under where they were before a move
    /// sideways, unless it already stood on no block).
    fn check_supporting_block(&mut self, world: &impl World, movement: DVec3) {
        if !self.on_ground {
            self.on_ground_no_blocks = false;
            self.supporting_block = None;
            return;
        }
        let bounds = Bounds::standing(self.position, self.width, self.height);
        let min = DVec3::new(bounds.min.x, bounds.min.y - 1.0e-6, bounds.min.z);
        let max = DVec3::new(bounds.max.x, bounds.min.y, bounds.max.z);
        let mut found = find_supporting_block(world, self.position, min, max);
        if found.is_some() || self.on_ground_no_blocks {
            self.supporting_block = found;
        } else {
            let back = DVec3::new(-movement.x, 0.0, -movement.z);
            found = find_supporting_block(world, self.position, min + back, max + back);
            self.supporting_block = found;
        }
        self.on_ground_no_blocks = found.is_none();
    }

    /// `Entity.applyMovementEmissionAndPlaySound`: the walked distance grows
    /// by 0.6 of the move (horizontally, or all of it on a climbable block),
    /// and once it passes the next step over a block that isn't air, the
    /// block at the feet (`getOnPosLegacy`) sounds its step when the body
    /// stands on it or climbs it, or else one in water swims; either way the
    /// next step is a block further (`nextStep`).
    fn movement_emission(&mut self, world: &impl World, movement: DVec3, in_water: bool) -> Option<Emission> {
        let moved = (movement.length() * f64::from(0.6_f32)) as f32;
        let horizontal = (movement.x.hypot(movement.z) * f64::from(0.6_f32)) as f32;
        let supporting_pos = self.on_pos(world, 1.0e-5);
        let supporting = world.block(supporting_pos);
        self.move_dist += if climbable(supporting.as_ref()) { moved } else { horizontal };
        if !(self.move_dist > self.next_step) || is_air(supporting.as_ref()) {
            return None;
        }
        let effect_pos = self.on_pos(world, 0.2);
        let effect = world.block(effect_pos);
        // `vibrationAndSoundEffectsFromBlock`: a mob neither crouches nor
        // rides rails.
        let stands_on = |block: Option<&Block>| !is_air(block) && (self.on_ground || climbable(block)) && !self.swimming;
        let sounds = stands_on(effect.as_ref());
        if sounds || supporting_pos != effect_pos && stands_on(supporting.as_ref()) {
            self.next_step = self.move_dist as i32 as f32 + 1.0;
            if !sounds {
                return None;
            }
            let block = world.step_sound(effect_pos).map(|(event, volume, pitch)| (event, volume * 0.15, pitch));
            let crystal = effect.as_ref().is_some_and(|b| matches!(b.id.as_str(), "minecraft:amethyst_block" | "minecraft:budding_amethyst"));
            return Some(Emission::Step(block, crystal));
        }
        if !in_water {
            return None;
        }
        self.next_step = self.move_dist as i32 as f32 + 1.0;
        // `waterSwimSound` for a mob nothing rides.
        Some(Emission::Swim((motion_speed(self.velocity) * 0.35).min(1.0)))
    }

    /// `LivingEntity.aiStep`'s jump for a mob whose jump control fired: it
    /// swims up in water above its jump threshold or in deep lava
    /// (`jumpInLiquid`), and otherwise jumps from the ground or shallow
    /// water at most every ten ticks (`jumpFromGround`; honey halves it).
    pub fn living_jump(&mut self, world: &impl World, fluid: FluidFrame, jumping: bool, no_jump_delay: &mut i32, threshold: f64) {
        if !jumping {
            *no_jump_delay = 0;
            return;
        }
        let height = if fluid.in_lava() { fluid.lava_height } else { fluid.water_height };
        let in_water = fluid.in_water() && height > 0.0;
        if in_water && !(self.on_ground && height <= threshold) {
            self.velocity.y += f64::from(0.04_f32);
        } else if fluid.in_lava() && !(self.on_ground && fluid.lava_height <= threshold) {
            self.velocity.y += f64::from(0.04_f32);
        } else if (self.on_ground || in_water && height <= threshold) && *no_jump_delay == 0 {
            // `jumpFromGround` does nothing for a jump too weak to count.
            let power = 0.42_f32 * jump_factor(world, self.position, self.on_pos(world, 0.500001));
            if power > 1.0e-5 {
                self.velocity.y = f64::from(power).max(self.velocity.y);
                self.needs_sync = true;
            }
            *no_jump_delay = 10;
        }
    }

    /// Called at the start of living aiStep before AI/controller input changes.
    /// `Entity.move`'s requested movement: the motion, unless a cobweb or
    /// bush holds it (`stuckSpeedMultiplier`), which scales this move and
    /// stops the motion.
    fn requested_move(&mut self) -> DVec3 {
        match self.stuck.take() {
            Some(stuck) if stuck.length_squared() > 1.0e-7 => {
                let requested = self.velocity * stuck;
                self.velocity = DVec3::ZERO;
                requested
            }
            _ => self.velocity,
        }
    }

    pub fn trim_small_velocity(&mut self) {
        for axis in 0..3 {
            if self.velocity[axis].abs() < 0.003 {
                self.velocity[axis] = 0.0;
            }
        }
    }

    /// LivingEntity.travelInWater for an unsprinting, unenchanted ground mob.
    pub fn travel_water(&mut self, world: &impl World, input: DVec3, yaw: f32) {
        self.travel_water_mode(world, input, yaw, 0.02_f32 as f64, 0.8_f32 as f64, true);
    }

    /// Drowned.travelInWater while underwater and pursuing a waterborne target.
    pub fn travel_drowned_swimming(&mut self, world: &impl World, input: DVec3, yaw: f32) {
        self.travel_water_mode(world, input, yaw, 0.01_f32 as f64, 0.9, false);
    }

    fn travel_water_mode(
        &mut self,
        world: &impl World,
        input: DVec3,
        yaw: f32,
        acceleration: f64,
        drag: f64,
        gravity: bool,
    ) {
        let is_falling = self.velocity.y <= 0.0;
        let old_y = self.position.y;
        let length = input.length_squared();
        if length >= 1.0e-7 {
            let movement = (if length > 1.0 {
                input / length.sqrt()
            } else {
                input
            }) * acceleration;
            let (sin, cos) = minecraft_sin_cos(yaw as f64);
            self.velocity += DVec3::new(
                movement.x * cos - movement.z * sin,
                movement.y,
                movement.z * cos + movement.x * sin,
            );
        }
        let requested = self.requested_move();
        let displacement = move_body(
            world,
            Bounds::standing(self.position, self.width, self.height),
            requested,
            self.on_ground,
            self.step_height,
        );
        let length = displacement.length_squared();
        if length > 1.0e-7 || requested.length_squared() - length < 1.0e-7 {
            let from = self.position;
            self.position += displacement;
            self.moves.push(crate::inside_blocks::Move { from, to: self.position, delta: Some(requested) });
        }
        self.horizontal_collision = (requested.x - displacement.x).abs() >= 1.0e-5_f32 as f64
            || (requested.z - displacement.z).abs() >= 1.0e-5_f32 as f64;
        self.vertical_collision = requested.y != displacement.y;
        self.on_ground = self.vertical_collision && requested.y < 0.0;
        self.fall_distance = 0.0;
        self.check_supporting_block(world, displacement);
        // Entity.move restitutes collided velocity before LivingEntity applies
        // water slowdown/gravity. A floor hit turns -0.005 into +0.0, so the
        // next water-travel velocity is -0.005 again instead of -0.009.
        self.restitute(world, requested, displacement);
        self.emission = self.movement_emission(world, displacement, true);
        // A climber against a wall rises.
        if self.horizontal_collision && self.climbing {
            self.velocity.y = 0.2;
        }
        let movement = self.velocity * DVec3::splat(drag);
        if !gravity {
            self.velocity = movement;
            return;
        }
        let gravity_step = self.gravity / 16.0;
        let yd = if is_falling
            && (movement.y - 0.005).abs() >= 0.003
            && (movement.y - gravity_step).abs() < 0.003
        {
            -0.003
        } else {
            movement.y - gravity_step
        };
        self.velocity = DVec3::new(movement.x, yd, movement.z);
        self.jump_out_of_fluid(world, old_y);
    }

    /// `LivingEntity.jumpOutOfFluid`: against a wall in water or lava, a
    /// mob leaps (0.3 up) when its box, carried by its motion and lifted to
    /// 0.6 above where it began the tick, meets no collision and no liquid
    /// (`isFree`).
    fn jump_out_of_fluid(&mut self, world: &impl World, old_y: f64) {
        if !self.horizontal_collision {
            return;
        }
        let v = self.velocity;
        let offset = DVec3::new(v.x, v.y + f64::from(0.6_f32) - self.position.y + old_y, v.z);
        let bounds = Bounds::standing(self.position, self.width, self.height);
        if is_free(world, bounds.min + offset, bounds.max + offset) {
            self.velocity.y = f64::from(0.3_f32);
        }
    }

    /// LivingEntity.travelInLava, using the same moveRelative input as water.
    pub fn travel_lava(&mut self, world: &impl World, input: DVec3, yaw: f32, fluid_height: f64) {
        let is_falling = self.velocity.y <= 0.0;
        let old_y = self.position.y;
        let length = input.length_squared();
        if length >= 1.0e-7 {
            let movement = (if length > 1.0 {
                input / length.sqrt()
            } else {
                input
            }) * 0.02_f32 as f64;
            let (sin, cos) = minecraft_sin_cos(yaw as f64);
            self.velocity += DVec3::new(
                movement.x * cos - movement.z * sin,
                movement.y,
                movement.z * cos + movement.x * sin,
            );
        }
        let requested = self.requested_move();
        let displacement = move_body(
            world,
            Bounds::standing(self.position, self.width, self.height),
            requested,
            self.on_ground,
            self.step_height,
        );
        let length = displacement.length_squared();
        if length > 1.0e-7 || requested.length_squared() - length < 1.0e-7 {
            let from = self.position;
            self.position += displacement;
            self.moves.push(crate::inside_blocks::Move { from, to: self.position, delta: Some(requested) });
        }
        self.horizontal_collision = (requested.x - displacement.x).abs() >= 1.0e-5_f32 as f64
            || (requested.z - displacement.z).abs() >= 1.0e-5_f32 as f64;
        self.vertical_collision = requested.y != displacement.y;
        self.on_ground = self.vertical_collision && requested.y < 0.0;
        self.check_supporting_block(world, displacement);
        // `LivingEntity.checkFallDamage` looks for water it moved into, and
        // with it measures the lava again: `isInShallowFluid` reads the
        // depth after the move.
        let mut fluid_height = fluid_height;
        if !self.touching_water {
            self.update_fluid(world, false);
            fluid_height = crate::fluid::FluidFrame::sample(world, self.position, self.width, self.height).lava_height;
        }
        // `Entity.checkFallDamage`: out of water a fall adds up, and a
        // landing ends it (the lava halves it in `baseTick`).
        if !self.touching_water && displacement.y < 0.0 {
            self.fall_distance -= displacement.y as f32 as f64;
        }
        if self.on_ground {
            self.fall_distance = 0.0;
        }
        self.restitute(world, requested, displacement);
        self.emission = self.movement_emission(world, displacement, self.touching_water);
        if fluid_height <= 0.4 {
            let movement = self.velocity * DVec3::new(0.5, 0.8_f32 as f64, 0.5);
            let gravity_step = self.gravity / 16.0;
            let yd = if is_falling
                && (movement.y - 0.005).abs() >= 0.003
                && (movement.y - gravity_step).abs() < 0.003
            {
                -0.003
            } else {
                movement.y - gravity_step
            };
            self.velocity = DVec3::new(movement.x, yd, movement.z);
        } else {
            self.velocity *= 0.5;
        }
        self.velocity.y -= self.gravity / 4.0;
        self.jump_out_of_fluid(world, old_y);
    }

    /// Performs dry travel after AI/controller input. Returns the accumulated
    /// fall distance on landing so the shared damage pipeline can handle it.
    pub fn travel_air(
        &mut self,
        world: &impl World,
        input: DVec3,
        speed: f32,
        yaw: f32,
    ) -> Option<f64> {
        self.travel_air_jumping(world, input, speed, yaw, false)
    }

    /// [`Body::travel_air`] for a mob whose jump control fired
    /// (`LivingEntity.jumping`): a climber
    /// (`handleRelativeFrictionAndCalculateMovement` with `onClimbable`)
    /// moves at most 0.15 a side and falls at most 0.15, forgets its fall,
    /// and against a wall or jumping rises at 0.2.
    pub fn travel_air_jumping(
        &mut self,
        world: &impl World,
        input: DVec3,
        speed: f32,
        yaw: f32,
        jumping: bool,
    ) -> Option<f64> {
        let friction = if self.on_ground {
            ground_friction(world, self.on_pos(world, 0.500001))
        } else {
            1.0_f32
        };
        let acceleration = if self.on_ground {
            if friction as f64 > 0.6 {
                speed * (0.21600002_f32 / (friction * friction * friction))
            } else {
                speed
            }
        } else {
            0.02_f32
        };
        let length = input.length_squared();
        let added = if length < 1.0e-7 {
            DVec3::ZERO
        } else {
            let movement = (if length > 1.0 {
                input / length.sqrt()
            } else {
                input
            }) * acceleration as f64;
            let (sin, cos) = minecraft_sin_cos(yaw as f64);
            DVec3::new(
                movement.x * cos - movement.z * sin,
                movement.y,
                movement.z * cos + movement.x * sin,
            )
        };
        self.velocity += added;
        // `handleOnClimbable`.
        if self.climbing {
            self.fall_distance = 0.0;
            let max = f64::from(0.15_f32);
            self.velocity.x = self.velocity.x.clamp(-max, max);
            self.velocity.z = self.velocity.z.clamp(-max, max);
            self.velocity.y = self.velocity.y.max(-max);
        }
        let requested = self.requested_move();
        let displacement = move_body(
            world,
            Bounds::standing(self.position, self.width, self.height),
            requested,
            self.on_ground,
            self.step_height,
        );
        let length = displacement.length_squared();
        if length > 1.0e-7 || requested.length_squared() - length < 1.0e-7 {
            let from = self.position;
            self.position += displacement;
            self.moves.push(crate::inside_blocks::Move { from, to: self.position, delta: Some(requested) });
        }
        let x_collision = (requested.x - displacement.x).abs() >= 1.0e-5_f32 as f64;
        let z_collision = (requested.z - displacement.z).abs() >= 1.0e-5_f32 as f64;
        self.horizontal_collision = x_collision || z_collision;
        self.vertical_collision = requested.y != displacement.y;
        self.on_ground = self.vertical_collision && requested.y < 0.0;
        self.check_supporting_block(world, displacement);
        // `LivingEntity.checkFallDamage` looks for water it moved into
        // before the fall counts; in water it does not.
        if !self.touching_water {
            self.update_fluid(world, false);
        }
        if !self.touching_water && displacement.y < 0.0 {
            self.fall_distance -= displacement.y as f32 as f64;
        }
        let landed = if self.on_ground {
            let distance = self.fall_distance;
            self.fall_distance = 0.0;
            Some(distance)
        } else {
            None
        };
        let _ = (x_collision, z_collision);
        self.restitute(world, requested, displacement);
        self.emission = self.movement_emission(world, displacement, self.touching_water);
        if (self.horizontal_collision || jumping) && self.climbing {
            self.velocity.y = 0.2;
        }
        let horizontal_drag = (friction * 0.91_f32) as f64;
        self.velocity = DVec3::new(
            self.velocity.x * horizontal_drag,
            (self.velocity.y - self.gravity) * 0.98_f32 as f64,
            self.velocity.z * horizontal_drag,
        );
        landed
    }
}

/// `Block.getBounceRestitution`: beds and shelf mushrooms 0.75, slime 1.
fn bounce_restitution(id: &str) -> f64 {
    match id {
        "minecraft:slime_block" => 1.0,
        "minecraft:shelf_mushroom" => f64::from(0.75_f32),
        id if id.ends_with("_bed") => f64::from(0.75_f32),
        _ => 0.0,
    }
}

impl Body {
    /// `Entity.restituteMovementAfterCollisions` for a mob (no bounciness
    /// of its own): collided motion stops (keeping zero's sign); landing
    /// faster than gravity on a bouncy block (`getOnPosLegacy`, not honey)
    /// throws it back up by the block's restitution, less the share of the
    /// tick's gravity and air drag the move had not used.
    fn restitute(&mut self, world: &impl World, requested: DVec3, displacement: DVec3) {
        let x_collision = (requested.x - displacement.x).abs() >= 1.0e-5_f32 as f64;
        let z_collision = (requested.z - displacement.z).abs() >= 1.0e-5_f32 as f64;
        let vertical = requested.y != displacement.y;
        if !((requested.y.abs() > 0.0 && vertical) || x_collision || z_collision) {
            return;
        }
        let current = self.velocity;
        if x_collision {
            self.velocity.x = -current.x * 0.0;
        }
        if z_collision {
            self.velocity.z = -current.z * 0.0;
        }
        if vertical {
            let mut restitution = 0.0;
            if requested.y < 0.0 {
                let below = world.block(self.on_pos(world, 0.2));
                let honey = below.as_ref().is_some_and(|b| b.id == "minecraft:honey_block");
                restitution = if !(-current.y <= self.gravity) && !honey { f64::max(0.0, below.map_or(0.0, |b| bounce_restitution(&b.id))) } else { 0.0 };
            }
            let (compensation, drag) = if restitution > 0.0 {
                let portion = displacement.y / current.y;
                // `Mth.lerp(portion, 1, getAirDrag())`.
                (portion * self.gravity, 1.0 + portion * (f64::from(0.98_f32) - 1.0))
            } else {
                (0.0, 1.0)
            };
            self.velocity.y = (compensation - current.y) * drag * restitution;
        }
    }
}

/// `Entity.isFree`: no block collision shape overlaps the box
/// (`noCollision`) and no block it reaches holds a fluid
/// (`containsAnyLiquid`).
fn is_free(world: &impl World, min: DVec3, max: DVec3) -> bool {
    // Shapes can stand out of their cell (fences reach 1.5 up), so the
    // neighbours count too.
    for x in min.x.floor() as i32 - 1..=max.x.floor() as i32 + 1 {
        for y in min.y.floor() as i32 - 1..=max.y.floor() as i32 + 1 {
            for z in min.z.floor() as i32 - 1..=max.z.floor() as i32 + 1 {
                let (bx, by, bz) = (f64::from(x), f64::from(y), f64::from(z));
                let overlaps = world.collision_boxes((x, y, z)).iter().any(|b| {
                    b[0] + bx < max.x && b[3] + bx > min.x && b[1] + by < max.y && b[4] + by > min.y && b[2] + bz < max.z && b[5] + bz > min.z
                });
                if overlaps {
                    return false;
                }
            }
        }
    }
    for x in min.x.floor() as i32..max.x.ceil() as i32 {
        for y in min.y.floor() as i32..max.y.ceil() as i32 {
            for z in min.z.floor() as i32..max.z.ceil() as i32 {
                if world.block((x, y, z)).is_some_and(|b| has_fluid(&b)) {
                    return false;
                }
            }
        }
    }
    true
}

/// Whether a block state holds a fluid (`getFluidState` not empty).
fn has_fluid(block: &Block) -> bool {
    matches!(
        block.id.as_str(),
        "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column" | "minecraft:kelp" | "minecraft:kelp_plant" | "minecraft:seagrass" | "minecraft:tall_seagrass"
    ) || block.property("waterlogged") == Some("true")
}

/// `Entity.getBlockJumpFactor`: the block at the feet, or when that one
/// is neutral the block below that affects movement.
fn jump_factor(world: &impl World, position: DVec3, below: Pos) -> f32 {
    let factor = |pos| if world.block(pos).is_some_and(|b| b.id == "minecraft:honey_block") { 0.5 } else { 1.0 };
    let here = factor((position.x.floor() as i32, position.y.floor() as i32, position.z.floor() as i32));
    if here == 1.0 {
        factor(below)
    } else {
        here
    }
}

/// `Block.getFriction` of the block under the feet that affects movement
/// (`getBlockPosBelowThatAffectsMyMovement`).
fn ground_friction(world: &impl World, pos: Pos) -> f32 {
    match world.block(pos).as_ref().map(|b| b.id.as_str()) {
        Some("minecraft:ice" | "minecraft:packed_ice" | "minecraft:frosted_ice") => 0.98,
        Some("minecraft:blue_ice") => 0.989,
        Some("minecraft:slime_block") => 0.8,
        _ => 0.6,
    }
}

/// `BlockState.isAir` (unloaded blocks read as air).
fn is_air(block: Option<&Block>) -> bool {
    block.is_none_or(|b| matches!(b.id.as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"))
}

/// `Entity.isStateClimbable`: `#minecraft:climbable` or powder snow.
fn climbable(block: Option<&Block>) -> bool {
    block.is_some_and(|b| {
        matches!(
            b.id.as_str(),
            "minecraft:ladder"
                | "minecraft:vine"
                | "minecraft:scaffolding"
                | "minecraft:weeping_vines"
                | "minecraft:weeping_vines_plant"
                | "minecraft:twisting_vines"
                | "minecraft:twisting_vines_plant"
                | "minecraft:cave_vines"
                | "minecraft:cave_vines_plant"
                | "minecraft:powder_snow"
        )
    })
}

/// The speed `waterSwimSound` and `doWaterSplashEffect` hear: sideways
/// motion counts a fifth (`0.2F`).
fn motion_speed(v: DVec3) -> f32 {
    let scale = f64::from(0.2_f32);
    (v.x * v.x * scale + v.y * v.y + v.z * v.z * scale).sqrt() as f32
}
