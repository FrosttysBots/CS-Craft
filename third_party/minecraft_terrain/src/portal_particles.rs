//! Copied from MinecraftOSS (`engine/viewer/src/portal_particles.rs`); the camera is its forward vector.
//! `PORTAL` particles (`PortalParticle`): an enderman sheds two a client
//! tick (`EnderMan.aiStep`). Each flies out along its motion and eases
//! back towards where it started, shrinking in and glowing brighter
//! (`addSmoothBlockEmission`) as it ages. Their randoms are unseeded in
//! vanilla, so they match in kind, not particle for particle.
use crate::{
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh, Vertex},
    pack::ResourceId,
};
use glam::{DVec3, Vec2, Vec3};
use minecraftoss_player::rng::LegacyRandom;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
struct PortalParticle {
    start: DVec3,
    motion: DVec3,
    position: DVec3,
    previous: DVec3,
    color: [f32; 3],
    quad_size: f32,
    sprite: i32,
    age: i32,
    lifetime: i32,
}

impl PortalParticle {
    /// `Particle(level, x, y, z)` and `SingleQuadParticle` (draws the
    /// portal overrides), then `PortalParticle`'s size, colour and life.
    fn new(start: DVec3, motion: DVec3, sprite: i32, random: &mut LegacyRandom) -> Self {
        let _base_lifetime = random.next_float();
        let _base_quad_size = random.next_float();
        let quad_size = 0.1 * (random.next_float() * 0.2 + 0.5);
        let brightness = random.next_float() * 0.6 + 0.4;
        let lifetime = (random.next_float() * 10.0) as i32 + 40;
        Self { start, motion, position: start, previous: start, color: [brightness * 0.9, brightness * 0.3, brightness], quad_size, sprite, age: 0, lifetime }
    }

    /// `PortalParticle.tick`: out along its motion and back, dropping from
    /// a block above.
    fn tick(&mut self) -> bool {
        self.previous = self.position;
        let prior_age = self.age;
        self.age += 1;
        if prior_age >= self.lifetime {
            return false;
        }
        let a = self.age as f32 / self.lifetime as f32;
        let pos = 1.0 - (-a + a * a * 2.0);
        let pos = f64::from(pos);
        self.position = DVec3::new(
            self.start.x + self.motion.x * pos,
            self.start.y + self.motion.y * pos + f64::from(1.0 - a),
            self.start.z + self.motion.z * pos,
        );
        true
    }
}

/// The client's portal particles.
pub struct PortalParticles {
    particles: Vec<PortalParticle>,
    random: LegacyRandom,
}

impl Default for PortalParticles {
    fn default() -> Self {
        let seed = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        Self { particles: Vec::new(), random: LegacyRandom::new(seed ^ 0x9e37_79b9) }
    }
}

impl PortalParticles {
    /// `EnderMan.aiStep` on the client: two particles about its box
    /// (`getRandomX(0.5)`, `getRandomY() - 0.25`, `getRandomZ(0.5)`),
    /// each with a random outward motion and a random sprite.
    pub fn emit_enderman(&mut self, feet: DVec3, width: f32, height: f32) {
        let (width, height) = (f64::from(width), f64::from(height));
        for _ in 0..2 {
            let r = &mut self.random;
            let x = feet.x + width * (2.0 * r.next_double() - 1.0) * 0.5;
            let y = feet.y + height * r.next_double() - 0.25;
            let z = feet.z + width * (2.0 * r.next_double() - 1.0) * 0.5;
            let motion = DVec3::new((r.next_double() - 0.5) * 2.0, -r.next_double(), (r.next_double() - 0.5) * 2.0);
            let sprite = r.next_int(8) as i32;
            let particle = PortalParticle::new(DVec3::new(x, y, z), motion, sprite, &mut self.random);
            self.particles.push(particle);
        }
    }

    pub fn tick(&mut self) {
        self.particles.retain_mut(PortalParticle::tick);
    }

    /// Camera-facing quads: `getQuadSize` grows in as `1 - (1 - t)²`, and
    /// the block light rises by `(age / lifetime)⁴` of full.
    pub fn append_mesh(&self, mesh: &mut ChunkMesh, atlas: &Atlas, forward: Vec3, partial: f32, light: &SkyLight) {
        let partial = partial.clamp(0.0, 1.0);
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward).normalize();
        for particle in &self.particles {
            let Ok(sprite) = ResourceId::parse(&format!("minecraft:particle/generic_{}", particle.sprite)) else { continue };
            if !atlas.contains(&sprite) {
                continue;
            }
            let [u0, v0, u1, v1] = atlas.region(&sprite);
            let center = particle.previous.lerp(particle.position, f64::from(partial));
            let cell = (center.x.floor() as i32, center.y.floor() as i32, center.z.floor() as i32);
            let glow = {
                let b = particle.age as f32 / particle.lifetime as f32;
                (b * b) * (b * b)
            };
            let sky = light.get(cell) as f32;
            let block = (f32::from(light.get_block(cell)) + (glow.clamp(0.0, 1.0) * 240.0) as i32 as f32 / 16.0).min(15.0);
            let grown = {
                let s = 1.0 - (particle.age as f32 + partial) / particle.lifetime as f32;
                1.0 - s * s
            };
            let size = particle.quad_size * grown;
            let center = center.as_vec3();
            let start = mesh.vertices.len() as u32;
            let [r, g, b] = particle.color;
            for (corner, uv) in [(Vec2::new(-1.0, -1.0), [u0, v1]), (Vec2::new(-1.0, 1.0), [u0, v0]), (Vec2::new(1.0, 1.0), [u1, v0]), (Vec2::new(1.0, -1.0), [u1, v1])] {
                let point = center + (right * corner.x + up * corner.y) * size;
                mesh.vertices.push(Vertex { position: point.to_array(), uv, color: [r, g, b, 1.0], sky_light: sky, block_light: block });
            }
            mesh.indices.extend_from_slice(&[start, start + 2, start + 1, start, start + 3, start + 2]);
            mesh.faces += 1;
        }
    }

    pub fn clear(&mut self) {
        self.particles.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_particles_return_to_their_start() {
        let mut portals = PortalParticles::default();
        portals.emit_enderman(DVec3::new(0.5, 64.0, 0.5), 0.6, 2.9);
        let particle = portals.particles[0].clone();
        let mut p = particle.clone();
        for _ in 0..p.lifetime {
            assert!(p.tick());
        }
        // At the end it is back at its start (the drop from above spent).
        assert!((p.position - p.start).length() < 1.0e-6);
        assert!(!p.tick());
    }
}
