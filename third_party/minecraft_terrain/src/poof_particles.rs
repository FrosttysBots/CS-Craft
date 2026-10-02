//! Copied from MinecraftOSS (`engine/viewer/src/poof_particles.rs`); the camera is its forward vector.
//! The puff a dead mob leaves (`LivingEntity.makePoofParticles` on entity
//! event 60, as the server removes it): twenty `POOF` particles
//! (`ExplodeParticle`), drifting up and fading through the generic
//! sprites. Particles draw from their own unseeded randoms in vanilla, so
//! the puff matches in kind, not particle for particle.
use crate::{
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh, Vertex},
    pack::ResourceId,
    scene::Scene,
};
use glam::{DVec3, Vec2, Vec3};
use minecraftoss_player::{collision_shape_boxes, rng::LegacyRandom, Block as PlayerBlock};
use std::time::{SystemTime, UNIX_EPOCH};

/// One `ExplodeParticle`.
#[derive(Clone, Debug)]
struct PoofParticle {
    position: DVec3,
    previous: DVec3,
    velocity: DVec3,
    color: f32,
    quad_size: f32,
    age: i32,
    lifetime: i32,
    on_ground: bool,
    stopped_by_collision: bool,
}

impl PoofParticle {
    /// `Particle(level, x, y, z)` and `SingleQuadParticle` (whose lifetime
    /// and size draws `ExplodeParticle` overrides), then `ExplodeParticle`.
    fn new(position: DVec3, motion: DVec3, random: &mut LegacyRandom) -> Self {
        let _base_lifetime = random.next_float();
        let _base_quad_size = random.next_float();
        let jitter = |random: &mut LegacyRandom| f64::from((random.next_float() * 2.0 - 1.0) * 0.05);
        let velocity = DVec3::new(motion.x + jitter(random), motion.y + jitter(random), motion.z + jitter(random));
        let color = random.next_float() * 0.3 + 0.7;
        let quad_size = 0.1 * (random.next_float() * random.next_float() * 6.0 + 1.0);
        let lifetime = (16.0 / f64::from(random.next_float() * 0.8 + 0.2)) as i32 + 2;
        Self { position, previous: position, velocity, color, quad_size, age: 0, lifetime, on_ground: false, stopped_by_collision: false }
    }

    /// `Particle.tick` with gravity -0.1 and friction 0.9: it rises, slows
    /// and stops against blocks (`Particle.move` for its 0.2 box).
    fn tick(&mut self, boxes: &[([f64; 3], [f64; 3])]) -> bool {
        self.previous = self.position;
        let prior_age = self.age;
        self.age += 1;
        if prior_age >= self.lifetime {
            return false;
        }
        self.velocity.y -= 0.04 * f64::from(-0.1_f32);
        if !self.stopped_by_collision {
            let requested = self.velocity.to_array();
            let half = f64::from(0.2_f32) / 2.0;
            let p = self.position;
            let mut bounds = [[p.x - half, p.y, p.z - half], [p.x + half, p.y + f64::from(0.2_f32), p.z + half]];
            let mut moved = requested;
            if requested != [0.0; 3] && requested.iter().map(|v| v * v).sum::<f64>() < 10000.0 {
                for axis in [1, 0, 2] {
                    for &(near, far) in boxes {
                        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
                        if bounds[0][a] >= far[a] || bounds[1][a] <= near[a] || bounds[0][b] >= far[b] || bounds[1][b] <= near[b] {
                            continue;
                        }
                        if moved[axis] > 0.0 && bounds[1][axis] <= near[axis] {
                            moved[axis] = moved[axis].min(near[axis] - bounds[1][axis]);
                        } else if moved[axis] < 0.0 && bounds[0][axis] >= far[axis] {
                            moved[axis] = moved[axis].max(far[axis] - bounds[0][axis]);
                        }
                    }
                    bounds[0][axis] += moved[axis];
                    bounds[1][axis] += moved[axis];
                }
            } else {
                for axis in 0..3 {
                    bounds[0][axis] += moved[axis];
                    bounds[1][axis] += moved[axis];
                }
            }
            if moved != [0.0; 3] {
                self.position = DVec3::new((bounds[0][0] + bounds[1][0]) / 2.0, bounds[0][1], (bounds[0][2] + bounds[1][2]) / 2.0);
            }
            self.stopped_by_collision = requested[1].abs() >= f64::from(1.0e-5_f32) && moved[1].abs() < f64::from(1.0e-5_f32);
            self.on_ground = requested[1] != moved[1] && requested[1] < 0.0;
            if requested[0] != moved[0] {
                self.velocity.x = 0.0;
            }
            if requested[2] != moved[2] {
                self.velocity.z = 0.0;
            }
        }
        self.velocity *= f64::from(0.9_f32);
        if self.on_ground {
            self.velocity.x *= f64::from(0.7_f32);
            self.velocity.z *= f64::from(0.7_f32);
        }
        true
    }
}

/// The client's poof particles.
pub struct PoofParticles {
    particles: Vec<PoofParticle>,
    random: LegacyRandom,
}

impl Default for PoofParticles {
    fn default() -> Self {
        let seed = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        Self { particles: Vec::new(), random: LegacyRandom::new(seed) }
    }
}

impl PoofParticles {
    /// `makePoofParticles` for a mob with these feet and box: each particle
    /// starts at a random spot of the box, pulled back along a small
    /// gaussian motion (`getRandomX(1)`, `getRandomY()`, `getRandomZ(1)`).
    pub fn spawn(&mut self, feet: DVec3, width: f32, height: f32) {
        let (width, height) = (f64::from(width), f64::from(height));
        for _ in 0..20 {
            let r = &mut self.random;
            let motion = DVec3::new(r.next_gaussian() * 0.02, r.next_gaussian() * 0.02, r.next_gaussian() * 0.02);
            let x = feet.x + width * (2.0 * r.next_double() - 1.0);
            let y = feet.y + height * r.next_double();
            let z = feet.z + width * (2.0 * r.next_double() - 1.0);
            let particle = PoofParticle::new(DVec3::new(x, y, z) - motion * 10.0, motion, &mut self.random);
            self.particles.push(particle);
        }
    }

    pub fn tick<S: Scene>(&mut self, scene: &S) {
        self.particles.retain_mut(|particle| {
            let [x, y, z] = particle.position.to_array().map(|v| v.floor() as i32);
            let mut boxes = Vec::new();
            for bx in x - 1..=x + 1 {
                for by in y - 1..=y + 1 {
                    for bz in z - 1..=z + 1 {
                        if let Some(block) = scene.block((bx, by, bz)) {
                            let shape = PlayerBlock { id: block.id.key(), properties: block.properties.clone() };
                            for (mut min, mut max) in collision_shape_boxes(&shape) {
                                for (axis, offset) in [bx, by, bz].into_iter().enumerate() {
                                    min[axis] += f64::from(offset);
                                    max[axis] += f64::from(offset);
                                }
                                boxes.push((min, max));
                            }
                        }
                    }
                }
            }
            particle.tick(&boxes)
        });
    }

    /// Camera-facing quads (`SingleQuadParticle`, `LOOKAT_XYZ`) with the
    /// sprite for their age (`SpriteSet.get`: generic_7 down to generic_0).
    pub fn append_mesh(&self, mesh: &mut ChunkMesh, atlas: &Atlas, forward: Vec3, partial: f32, light: &SkyLight) {
        let partial = f64::from(partial.clamp(0.0, 1.0));
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward).normalize();
        for particle in &self.particles {
            let index = (particle.age * 7 / particle.lifetime.max(1)).clamp(0, 7);
            let Ok(sprite) = ResourceId::parse(&format!("minecraft:particle/generic_{}", 7 - index)) else { continue };
            if !atlas.contains(&sprite) {
                continue;
            }
            let [u0, v0, u1, v1] = atlas.region(&sprite);
            let center = particle.previous.lerp(particle.position, partial);
            let cell = (center.x.floor() as i32, center.y.floor() as i32, center.z.floor() as i32);
            let (sky, block) = (light.get(cell) as f32, light.get_block(cell) as f32);
            let center = center.as_vec3();
            let start = mesh.vertices.len() as u32;
            for (corner, uv) in [(Vec2::new(-1.0, -1.0), [u0, v1]), (Vec2::new(-1.0, 1.0), [u0, v0]), (Vec2::new(1.0, 1.0), [u1, v0]), (Vec2::new(1.0, -1.0), [u1, v1])] {
                let point = center + (right * corner.x + up * corner.y) * particle.quad_size;
                mesh.vertices.push(Vertex { position: point.to_array(), uv, color: [particle.color, particle.color, particle.color, 1.0], sky_light: sky, block_light: block });
            }
            mesh.indices.extend_from_slice(&[start, start + 2, start + 1, start, start + 3, start + 2]);
            mesh.faces += 1;
        }
    }

    pub fn clear(&mut self) {
        self.particles.clear();
    }

    pub fn len(&self) -> usize {
        self.particles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.particles.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_puff_rises_and_fades_within_its_lifetime() {
        let mut poofs = PoofParticles::default();
        poofs.spawn(DVec3::new(0.5, 64.0, 0.5), 0.9, 1.4);
        assert_eq!(poofs.len(), 20);
        let start: f64 = poofs.particles.iter().map(|p| p.position.y).sum();
        for _ in 0..5 {
            for particle in &mut poofs.particles {
                particle.tick(&[]);
            }
        }
        let later: f64 = poofs.particles.iter().map(|p| p.position.y).sum();
        assert!(later > start, "gravity -0.1 lifts them");
        assert!(poofs.particles.iter().all(|p| (3..=82).contains(&p.lifetime)));
    }
}
