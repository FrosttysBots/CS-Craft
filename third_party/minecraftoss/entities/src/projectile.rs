//! Source-informed Java 26.3 arrow launch, flight, block and simple entity hits.
//! Provenance: Projectile.getMovementToShoot/shoot,
//! AbstractArrow.tick/onHitBlock/shouldFall/startFalling/tickDespawn,
//! AbstractArrow.onHitEntity, ProjectileUtil.getManyEntityHitResult,
//! ProjectileDeflection.REVERSE, AABB.clip, Level.clipIncludingBorder,
//! and AbstractSkeleton.performRangedAttack.
//! This slice models a direct living villager hit; broader entity impacts follow.

use glam::DVec3;
use minecraftoss_player::{collision_boxes_at, rng::LegacyRandom, Block, Pos, World};

#[derive(Clone, Debug)]
pub struct Arrow {
    pub position: DVec3,
    pub velocity: DVec3,
    pub in_ground: bool,
    pub in_ground_time: i32,
    pub life: i32,
    pub shake_time: i32,
    pub alive: bool,
    pub impact_block: Option<Pos>,
    pub base_damage: f64,
    impact_state: Option<Block>,
    random: LegacyRandom,
    /// Where it played `entity.arrow.hit` this tick, and the pitch.
    hit_sound: Option<(DVec3, f32)>,
}

#[derive(Clone, Copy, Debug)]
pub struct ArrowTarget {
    pub id: u64,
    pub min: DVec3,
    pub max: DVec3,
}

#[derive(Clone, Copy, Debug)]
pub struct ArrowImpact {
    pub target_id: u64,
    pub damage: f32,
    pub velocity: DVec3,
}

impl Arrow {
    /// AbstractArrow.setBaseDamageFromMob with Normal difficulty's ID 2 in
    /// the measured skeleton fixture. The damage stream precedes shoot's
    /// independent stream reset in the harness.
    pub fn set_base_damage_from_mob(
        &mut self,
        power: f32,
        difficulty_id: u32,
        random: &mut LegacyRandom,
    ) {
        // `power * 2.0F + random.triangle(difficulty * 0.11, 0.57425)`, the
        // triangle (`mode + deviation * (a - b)`) summed on its own first.
        let mode = f64::from(difficulty_id) * 0.11;
        let triangle = mode + 0.57425 * (random.next_double() - random.next_double());
        self.base_damage = f64::from(power * 2.0) + triangle;
    }

    /// An arrow in flight at `velocity` (`summon` with `Motion`), with its
    /// default base damage.
    pub fn in_flight(position: DVec3, velocity: DVec3, random: LegacyRandom) -> Self {
        Self {
            position,
            velocity,
            in_ground: false,
            in_ground_time: 0,
            life: 0,
            shake_time: 0,
            alive: true,
            impact_block: None,
            base_damage: 2.0,
            impact_state: None,
            random,
            hit_sound: None,
        }
    }

    pub fn shoot(
        position: DVec3,
        direction: DVec3,
        power: f32,
        uncertainty: f32,
        random: &mut LegacyRandom,
    ) -> Self {
        let length_squared =
            direction.x * direction.x + direction.y * direction.y + direction.z * direction.z;
        let unit = if length_squared < 1.0e-8 {
            DVec3::ZERO
        } else {
            let length = length_squared.sqrt();
            DVec3::new(
                direction.x / length,
                direction.y / length,
                direction.z / length,
            )
        };
        let spread = 0.0172275 * uncertainty as f64;
        let velocity = DVec3::new(
            unit.x + (random.next_double() - random.next_double()) * spread,
            unit.y + (random.next_double() - random.next_double()) * spread,
            unit.z + (random.next_double() - random.next_double()) * spread,
        ) * power as f64;
        Self {
            position,
            velocity,
            in_ground: false,
            in_ground_time: 0,
            life: 0,
            shake_time: 0,
            alive: true,
            impact_block: None,
            base_damage: 2.0,
            impact_state: None,
            random: random.clone(),
            hit_sound: None,
        }
    }

    /// A skeleton's ordinary bow shot at a living target. The caller supplies
    /// the target's bounding-box height and difficulty's ranged uncertainty.
    pub fn skeleton_shot(
        shooter: DVec3,
        shooter_eye_height: f32,
        target: DVec3,
        target_height: f32,
        uncertainty: f32,
        random: &mut LegacyRandom,
    ) -> Self {
        let position = DVec3::new(
            shooter.x,
            shooter.y + shooter_eye_height as f64 - 0.1_f32 as f64,
            shooter.z,
        );
        let dx = target.x - shooter.x;
        let dy = target.y + target_height as f64 * (1.0 / 3.0) - position.y;
        let dz = target.z - shooter.z;
        let horizontal = (dx * dx + dz * dz).sqrt();
        Self::shoot(
            position,
            DVec3::new(dx, dy + horizontal * 0.2_f32 as f64, dz),
            1.6,
            uncertainty,
            random,
        )
    }

    /// One server tick in open air; new arrows first tick on the following
    /// world tick, after being inserted during the shooter's tick.
    pub fn tick_open_air(&mut self) {
        if self.in_ground {
            return;
        }
        self.position += self.velocity;
        self.velocity *= 0.99_f32 as f64;
        self.velocity.y -= 0.05;
    }

    /// AbstractArrow.tick for an ordinary physical arrow against the shared
    /// block collision shapes and direct living target boxes. Exotic block
    /// callbacks and broader projectile interactions follow in later slices.
    pub fn tick_world(
        &mut self,
        world: &impl World,
        targets: &[ArrowTarget],
    ) -> Option<ArrowImpact> {
        if !self.alive {
            return None;
        }
        if self.shake_time > 0 {
            self.shake_time -= 1;
        }
        if self.in_ground {
            let current_pos = (
                self.position.x.floor() as i32,
                self.position.y.floor() as i32,
                self.position.z.floor() as i32,
            );
            if world.block(current_pos) != self.impact_state && self.should_fall(world) {
                self.in_ground = false;
                self.velocity *= DVec3::new(
                    f64::from(self.random.next_float() * 0.2),
                    f64::from(self.random.next_float() * 0.2),
                    f64::from(self.random.next_float() * 0.2),
                );
                self.life = 0;
            } else {
                self.life += 1;
                if self.life >= 1200 {
                    self.alive = false;
                }
            }
            self.in_ground_time += 1;
            return None;
        }
        self.in_ground_time = 0;
        let block_hit = first_block_hit(world, self.position, self.velocity);
        let target_end = block_hit.map_or(self.position + self.velocity, |(_, hit, _)| hit);
        let ray = target_end - self.position;
        if let Some((fraction, target)) = targets
            .iter()
            .filter_map(|target| {
                clip_fraction(self.position, ray, target.min, target.max)
                    .map(|fraction| (fraction, target))
            })
            .min_by(|(a, _), (b, _)| a.total_cmp(b))
        {
            self.position += ray * fraction;
            let velocity = self.velocity;
            let speed = f64::from(velocity.length() as f32);
            let damage = (speed * self.base_damage).max(0.0).ceil() as f32;
            return Some(ArrowImpact {
                target_id: target.id,
                damage,
                velocity,
            });
        }
        if let Some((_, hit, block)) = block_hit {
            // `Math.signum`: zero stays zero.
            let signum = |v: f64| if v == 0.0 { v } else { v.signum() };
            let sign = DVec3::new(signum(self.velocity.x), signum(self.velocity.y), signum(self.velocity.z));
            self.position = hit - sign * 0.05_f32 as f64;
            self.velocity = DVec3::ZERO;
            self.play_hit_sound();
            self.in_ground = true;
            self.impact_block = Some(block);
            self.impact_state = world.block(block);
            self.shake_time = 7;
        } else {
            self.tick_open_air();
        }
        // The flight ends with `super.tick()`: `Entity.checkBelowWorld`
        // discards an arrow 64 blocks below the world.
        if self.position.y < f64::from(world.min_y() - 64) {
            self.alive = false;
        }
        None
    }

    /// Complete `AbstractArrow.onHitEntity` after the ordered world applies
    /// damage to the struck target. Rejected damage reverses the arrow and
    /// still receives the current tick's air drag and gravity.
    pub fn resolve_entity_hit(&mut self, accepted: bool) {
        if accepted {
            self.play_hit_sound();
            self.alive = false;
        } else {
            let _ = self.random.next_float(); // REVERSE yaw variation.
            self.velocity *= -0.1;
            self.velocity *= 0.99_f32 as f64;
            self.velocity.y -= 0.05;
        }
    }

    /// `playSound(ARROW_HIT, 1.0F, 1.2F / (random.nextFloat() * 0.2F +
    /// 0.9F))` where it stands, on a block or an entity it hurt.
    fn play_hit_sound(&mut self) {
        let pitch = 1.2 / (self.random.next_float() * 0.2 + 0.9);
        self.hit_sound = Some((self.position, pitch));
    }

    /// The hit sound it played since the last call.
    pub fn take_hit_sound(&mut self) -> Option<(DVec3, f32)> {
        self.hit_sound.take()
    }

    fn should_fall(&self, world: &impl World) -> bool {
        let low = self.position - DVec3::splat(0.06);
        let high = self.position + DVec3::splat(0.06);
        let min = low.floor();
        let max = high.floor();
        for x in min.x as i32..=max.x as i32 {
            for y in min.y as i32..=max.y as i32 {
                for z in min.z as i32..=max.z as i32 {
                    for (box_min, box_max) in collision_boxes_at(world, (x, y, z)) {
                        if low.x < box_max.x
                            && high.x > box_min.x
                            && low.y < box_max.y
                            && high.y > box_min.y
                            && low.z < box_max.z
                            && high.z > box_min.z
                        {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
}

fn first_block_hit(
    world: &impl World,
    origin: DVec3,
    movement: DVec3,
) -> Option<(f64, DVec3, Pos)> {
    let end = origin + movement;
    let ray = end - origin;
    let min = origin.min(end).floor();
    let max = origin.max(end).floor();
    let mut best: Option<(f64, Pos)> = None;
    for x in min.x as i32..=max.x as i32 {
        for y in min.y as i32..=max.y as i32 {
            for z in min.z as i32..=max.z as i32 {
                let pos = (x, y, z);
                for (box_min, box_max) in collision_boxes_at(world, pos) {
                    if let Some(fraction) = clip_fraction(origin, ray, box_min, box_max) {
                        if best.is_none_or(|(previous, _)| fraction < previous) {
                            best = Some((fraction, pos));
                        }
                    }
                }
            }
        }
    }
    best.map(|(fraction, pos)| (fraction, origin + ray * fraction, pos))
}

fn clip_fraction(origin: DVec3, movement: DVec3, box_min: DVec3, box_max: DVec3) -> Option<f64> {
    let mut near: f64 = 0.0;
    let mut far: f64 = 1.0;
    for axis in 0..3 {
        if (origin[axis] <= box_min[axis] && movement[axis] < 0.0)
            || (origin[axis] >= box_max[axis] && movement[axis] > 0.0)
        {
            return None;
        }
        if movement[axis] == 0.0 {
            if origin[axis] < box_min[axis] || origin[axis] > box_max[axis] {
                return None;
            }
        } else {
            let a = (box_min[axis] - origin[axis]) / movement[axis];
            let b = (box_max[axis] - origin[axis]) / movement[axis];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
    }
    (near <= far && (0.0..=1.0).contains(&near)).then_some(near)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_normal_skeleton_bow_open_air_trace() {
        // scenarios/mobs/skeleton-player-bow.json; 26.3, shoot seed 881.
        let mut random = LegacyRandom::new(881);
        let mut arrow = Arrow::skeleton_shot(
            DVec3::new(4.7811648174303185, 1.0, 4.53125),
            1.74,
            DVec3::new(14.5, 1.0, 4.5),
            1.8,
            6.0,
            &mut random,
        );
        assert_eq!(arrow.position.y.to_bits(), 2.640000008046627_f64.to_bits());
        assert_eq!(arrow.velocity.x.to_bits(), 1.6466665306024024_f64.to_bits());
        assert_eq!(
            arrow.velocity.y.to_bits(),
            0.25545528392154754_f64.to_bits()
        );
        assert_eq!(
            arrow.velocity.z.to_bits(),
            0.03761536055610391_f64.to_bits()
        );
        arrow.tick_open_air();
        assert_eq!(arrow.position.x.to_bits(), 6.427831348032721_f64.to_bits());
        assert_eq!(arrow.position.y.to_bits(), 2.8954552919681746_f64.to_bits());
        assert_eq!(
            arrow.velocity.y.to_bits(),
            0.20290073351854349_f64.to_bits()
        );
        arrow.set_base_damage_from_mob(1.0, 2, &mut LegacyRandom::new(419));
        assert_eq!(
            arrow.base_damage.to_bits(),
            2.4331762215570514_f64.to_bits()
        );
    }
}
