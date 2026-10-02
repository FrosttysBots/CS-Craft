//! Block-position biome lookup (vanilla `BiomeManager` fuzzy zoom).

use sha2::{Digest, Sha256};

/// `BiomeManager.obfuscateSeed`: the first eight SHA-256 bytes of the seed, little endian.
pub fn zoom_seed(world_seed: i64) -> i64 {
    let digest = Sha256::digest(world_seed.to_le_bytes());
    i64::from_le_bytes(digest[..8].try_into().expect("SHA-256 prefix"))
}

/// `LinearCongruentialGenerator.next`.
fn lcg(seed: i64, coordinate: i64) -> i64 {
    seed.wrapping_mul(seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407)).wrapping_add(coordinate)
}

fn fiddle(seed: i64) -> f64 {
    ((seed >> 24).rem_euclid(1024) as f64 / 1024.0 - 0.5) * 0.9
}

fn fiddled_distance(seed: i64, quart: [i32; 3], offset: [f64; 3]) -> f64 {
    let mut v = seed;
    for c in quart.into_iter().chain(quart) {
        v = lcg(v, i64::from(c));
    }
    let dx = offset[0] + fiddle(v);
    v = lcg(v, seed);
    let dy = offset[1] + fiddle(v);
    v = lcg(v, seed);
    let dz = offset[2] + fiddle(v);
    dz * dz + dy * dy + dx * dx
}

/// The quart whose noise biome `BiomeManager.getBiome` returns for a block.
pub fn quart_for_block(zoom_seed: i64, x: i32, y: i32, z: i32) -> [i32; 3] {
    let source = [x.wrapping_sub(2), y.wrapping_sub(2), z.wrapping_sub(2)];
    let parent = source.map(|c| c >> 2);
    let fraction = source.map(|c| f64::from(c & 3) / 4.0);
    let mut nearest = parent;
    let mut nearest_distance = f64::INFINITY;
    for corner in 0..8 {
        let right = [corner & 4 != 0, corner & 2 != 0, corner & 1 != 0];
        let quart = std::array::from_fn(|a| parent[a] + i32::from(right[a]));
        let offset = std::array::from_fn(|a| fraction[a] - f64::from(u8::from(right[a])));
        let d = fiddled_distance(zoom_seed, quart, offset);
        if nearest_distance > d {
            nearest = quart;
            nearest_distance = d;
        }
    }
    nearest
}

/// `quart_for_block` for runs of nearby blocks: the eight corner offsets
/// depend only on the 4x4x4 cell a block falls in, so they are kept until
/// the cell changes. Results are identical to `quart_for_block`.
pub struct ZoomCache {
    seed: i64,
    parent: Option<[i32; 3]>,
    /// Each corner's fiddled X, Y and Z offsets.
    fiddles: [[f64; 3]; 8],
}

impl ZoomCache {
    pub fn new(zoom_seed: i64) -> Self {
        Self { seed: zoom_seed, parent: None, fiddles: [[0.0; 3]; 8] }
    }

    pub fn quart(&mut self, x: i32, y: i32, z: i32) -> [i32; 3] {
        let source = [x.wrapping_sub(2), y.wrapping_sub(2), z.wrapping_sub(2)];
        let parent = source.map(|c| c >> 2);
        let fraction = source.map(|c| f64::from(c & 3) / 4.0);
        if self.parent != Some(parent) {
            self.parent = Some(parent);
            for corner in 0..8 {
                let right = [corner & 4 != 0, corner & 2 != 0, corner & 1 != 0];
                let quart: [i32; 3] = std::array::from_fn(|a| parent[a] + i32::from(right[a]));
                let mut v = self.seed;
                for c in quart.into_iter().chain(quart) {
                    v = lcg(v, i64::from(c));
                }
                let fx = fiddle(v);
                v = lcg(v, self.seed);
                let fy = fiddle(v);
                v = lcg(v, self.seed);
                let fz = fiddle(v);
                self.fiddles[corner] = [fx, fy, fz];
            }
        }
        let mut nearest = parent;
        let mut nearest_distance = f64::INFINITY;
        for corner in 0..8 {
            let right = [corner & 4 != 0, corner & 2 != 0, corner & 1 != 0];
            let [fx, fy, fz] = self.fiddles[corner];
            let dx = fraction[0] - f64::from(u8::from(right[0])) + fx;
            let dy = fraction[1] - f64::from(u8::from(right[1])) + fy;
            let dz = fraction[2] - f64::from(u8::from(right[2])) + fz;
            let d = dz * dz + dy * dy + dx * dx;
            if nearest_distance > d {
                nearest = std::array::from_fn(|a| parent[a] + i32::from(right[a]));
                nearest_distance = d;
            }
        }
        nearest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_matches_direct_lookup() {
        let seed = zoom_seed(1234);
        let mut cache = ZoomCache::new(seed);
        for x in -9..9 {
            for z in -9..9 {
                for y in (-70..40).rev() {
                    assert_eq!(cache.quart(x, y, z), quart_for_block(seed, x, y, z), "at {x} {y} {z}");
                }
            }
        }
    }
}
