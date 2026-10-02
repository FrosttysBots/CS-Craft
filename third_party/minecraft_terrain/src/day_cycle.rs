//! Overworld presentation clock for the fixed scene. The key times and colors
//! follow the pinned Minecraft 26.3 `minecraft:day` timeline and Overworld
//! dimension attributes; the abbreviated sunrise ramp is a visual approximation.

use crate::scene::BiomeSample;
use glam::Vec3;
pub use minecraftoss_core::environment::Skybox;

pub const DAY_TICKS: f64 = 24_000.0;

#[derive(Clone, Copy, Debug)]
pub struct DayCycle {
    pub ticks: f64,
    pub paused: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct SkyState {
    pub sky: Vec3,
    pub fog: Vec3,
    pub light: Vec3,
    pub sky_light_color: Vec3,
    pub sky_light_factor: f32,
    pub cloud: Vec3,
    /// `cloud_color` alpha; clouds render only when it is at least 1/255.
    pub cloud_alpha: f32,
    pub sunset: [f32; 4],
    pub sun_direction: Vec3,
    pub moon_direction: Vec3,
    pub star_brightness: f32,
    pub star_angle: f32,
    pub moon_phase: usize,
    pub rain_brightness: f32,
    /// Lightmap inputs beside the sky light: `ambient_light_color`,
    /// `block_light_tint` and the flickering block factor.
    pub ambient: Vec3,
    pub block_light_tint: Vec3,
    pub block_factor: f32,
    /// `FogData` environmental start/end and the sky and cloud fog ends.
    pub fog_start: f32,
    pub fog_end: f32,
    pub sky_fog_end: f32,
    pub cloud_fog_end: f32,
    pub skybox: Skybox,
}

/// Overworld dimension constants for the authored scene's clock.
pub const OVERWORLD_AMBIENT: Vec3 = Vec3::splat(10.0 / 255.0);
/// The `block_light_tint` default, 0xFFD88C.
pub const DEFAULT_BLOCK_LIGHT_TINT: Vec3 = Vec3::new(1.0, 216.0 / 255.0, 140.0 / 255.0);
/// `LightmapRenderStateExtractor`: 1.4 plus a flicker that starts at zero.
pub const BLOCK_FACTOR: f32 = 1.4;

impl Default for DayCycle {
    fn default() -> Self {
        Self {
            ticks: 1000.0,
            paused: false,
        }
    }
}

impl DayCycle {
    /// The Overworld day timeline's `minecraft:visual/sun_angle` value.
    pub fn sun_angle_degrees(&self) -> f32 {
        360.0 * day_ease(self.time() as f32)
    }

    /// `minecraft:gameplay/sky_light_level` multiplies the dimension's
    /// default level 15 by this timeline track.
    pub fn sky_light_level(&self) -> f32 {
        15.0 * track(
            self.time() as f32,
            &[
                (133.0, 1.0),
                (11867.0, 1.0),
                (13670.0, 0.266_666_68),
                (22330.0, 0.266_666_68),
            ],
        )
    }

    pub fn advance(&mut self, seconds: f64) {
        if !self.paused {
            self.ticks += seconds * 20.0;
        }
    }

    pub fn set(&mut self, ticks: f64) {
        // Keep total clock ticks. The sky/moon sample wraps separately, while
        // DebugEntryDayCount reads the unwrapped period count.
        self.ticks = ticks;
    }

    pub fn day_count(&self) -> i32 {
        (self.ticks as i64 / 24_000) as i32
    }

    pub fn time(&self) -> f64 {
        self.ticks.rem_euclid(DAY_TICKS)
    }

    pub fn sample(&self) -> SkyState {
        self.sample_for_biome(BiomeSample::THE_VOID)
    }
    /// The sky at the graphics reference's 16-chunk render distance.
    pub fn sample_for_biome(&self, biome: BiomeSample) -> SkyState {
        self.sample_for_biome_at(biome, 16)
    }
    pub fn sample_for_biome_at(&self, biome: BiomeSample, render_distance: u32) -> SkyState {
        let t = self.time() as f32;
        let sky_factor = track(
            t,
            &[(133.0, 1.0), (11867.0, 1.0), (13670.0, 0.0), (22330.0, 0.0)],
        );
        let day_night_color = |night: f32| {
            track(
                t,
                &[
                    (133.0, 1.0),
                    (11867.0, 1.0),
                    (13670.0, night),
                    (22330.0, night),
                ],
            )
        };
        let light_factor = track(
            t,
            &[
                (730.0, 1.0),
                (11270.0, 1.0),
                (13140.0, 0.24),
                (22860.0, 0.24),
            ],
        );
        let night_blue = track(
            t,
            &[(730.0, 1.0), (11270.0, 1.0), (13140.0, 1.0), (22860.0, 1.0)],
        );
        let light_tint = Vec3::new(
            track(
                t,
                &[
                    (730.0, 1.0),
                    (11270.0, 1.0),
                    (13140.0, 122.0 / 255.0),
                    (22860.0, 122.0 / 255.0),
                ],
            ),
            track(
                t,
                &[
                    (730.0, 1.0),
                    (11270.0, 1.0),
                    (13140.0, 122.0 / 255.0),
                    (22860.0, 122.0 / 255.0),
                ],
            ),
            night_blue,
        );
        // Minecraft's default gamma option is 0.5. At open-sky level 15 the
        // pinned lightmap shader adds ambient 0x0a and applies notGamma.
        let raw_light = (Vec3::splat(10.0 / 255.0) + light_tint * light_factor).min(Vec3::ONE);
        let max_channel = raw_light.max_element();
        let gamma_light = if max_channel > 0.0 {
            raw_light * ((1.0 - (1.0 - max_channel).powi(4)) / max_channel)
        } else {
            Vec3::ZERO
        };
        let light = raw_light.lerp(gamma_light, 0.5);

        let angle = celestial_angle(t);
        let sun_direction = Vec3::new(-angle.sin(), angle.cos(), 0.0);
        let moon_direction = -sun_direction;
        let star_brightness = track(
            t,
            &[
                (92.0, 0.037),
                (627.0, 0.0),
                (11373.0, 0.0),
                (11732.0, 0.016),
                (11959.0, 0.044),
                (12399.0, 0.143),
                (12729.0, 0.258),
                (13228.0, 0.5),
                (22772.0, 0.5),
                (23032.0, 0.364),
                (23356.0, 0.225),
                (23758.0, 0.101),
            ],
        );
        let dawn = 1.0 - circular_distance(t, 0.0) / 950.0;
        let dusk = 1.0 - circular_distance(t, 12_000.0) / 1500.0;
        let sunset_alpha = dawn.max(dusk).clamp(0.0, 1.0).powi(2) * 0.55;
        let sunset_color =
            Vec3::new(1.0, 0.58, 0.23).lerp(Vec3::new(0.96, 0.36, 0.12), sunset_alpha);
        let cloud_color = Vec3::new(
            day_night_color(25.0 / 255.0),
            day_night_color(25.0 / 255.0),
            day_night_color(38.0 / 255.0),
        );
        let sky_base = biome
            .sky_color
            .unwrap_or([120, 167, 255])
            .map(|c| c as f32 / 255.0);
        let sky = Vec3::from(sky_base) * sky_factor;
        let fog_rgb = biome
            .fog_color
            .unwrap_or([192, 216, 255])
            .map(|c| c as f32 / 255.0);
        let fog_base = Vec3::from(fog_rgb)
            * Vec3::new(
                day_night_color(15.0 / 255.0),
                day_night_color(15.0 / 255.0),
                day_night_color(22.0 / 255.0),
            );
        // AtmosphericFogEnvironment.getBaseColor blends Overworld fog toward
        // the camera biome's sky color, by the smaller of the render distance
        // and the 512-block sky_fog_end_distance, in chunks.
        let sky_fog_end = (512.0_f32 / 16.0).min(render_distance as f32);
        let sky_color_mix = 1.0 - clamped_lerp(sky_fog_end / 32.0, 0.25, 1.0).powf(0.25);
        let fog = fog_base.lerp(sky, sky_color_mix);
        SkyState {
            sky,
            fog,
            light,
            sky_light_color: light_tint,
            sky_light_factor: light_factor,
            cloud: cloud_color,
            cloud_alpha: 0.8,
            sunset: [sunset_color.x, sunset_color.y, sunset_color.z, sunset_alpha],
            sun_direction,
            moon_direction,
            star_brightness,
            star_angle: angle,
            moon_phase: (self.ticks.div_euclid(DAY_TICKS) as usize) % 8,
            rain_brightness: 1.0,
            ambient: OVERWORLD_AMBIENT,
            block_light_tint: DEFAULT_BLOCK_LIGHT_TINT,
            block_factor: BLOCK_FACTOR,
            fog_start: 0.0,
            fog_end: 1024.0,
            sky_fog_end: 512.0,
            cloud_fog_end: 2048.0,
            skybox: Skybox::Overworld,
        }
    }
}

/// `Mth.clampedLerp(factor, min, max)`.
fn clamped_lerp(factor: f32, min: f32, max: f32) -> f32 {
    if factor < 0.0 {
        min
    } else if factor > 1.0 {
        max
    } else {
        min + factor * (max - min)
    }
}

fn circular_distance(t: f32, center: f32) -> f32 {
    let delta = (t - center).abs();
    delta.min(DAY_TICKS as f32 - delta)
}

fn track(t: f32, keys: &[(f32, f32)]) -> f32 {
    for i in 0..keys.len() {
        let (from_t, from_v) = keys[i];
        let (mut to_t, to_v) = keys[(i + 1) % keys.len()];
        let mut at = t;
        if i + 1 == keys.len() {
            to_t += DAY_TICKS as f32;
            if at < from_t {
                at += DAY_TICKS as f32;
            }
        }
        if at >= from_t && at <= to_t {
            let alpha = ((at - from_t) / (to_t - from_t)).clamp(0.0, 1.0);
            return from_v + (to_v - from_v) * alpha;
        }
    }
    keys[0].1
}

fn celestial_angle(t: f32) -> f32 {
    -std::f32::consts::TAU * day_ease(t)
}

/// EasingType.CubicBezier in 26.3 returns the starting guess when the X
/// sample is within 1e-5, before any Newton step. This matters near noon.
fn day_ease(t: f32) -> f32 {
    let x = ((t - 6000.0) / DAY_TICKS as f32).rem_euclid(1.0);
    let x_curve = CubicCurve::new(0.362, 0.638);
    let y_curve = CubicCurve::new(0.241, 0.759);
    let mut u = x;
    for _ in 0..4 {
        let error = x_curve.sample(u) - x;
        if error.abs() < 1.0e-5 {
            return y_curve.sample(u);
        }
        let gradient = x_curve.gradient(u);
        if gradient < 1.0e-5 {
            break;
        }
        u -= (error / gradient).clamp(-0.25, 0.25);
    }
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    while low < high {
        let error = x_curve.sample(u) - x;
        if error.abs() < 1.0e-5 {
            break;
        }
        if error < 0.0 {
            low = u;
        } else {
            high = u;
        }
        let next = (low + high) / 2.0;
        if next == u {
            break;
        }
        u = next;
    }
    y_curve.sample(u)
}

struct CubicCurve {
    a: f32,
    b: f32,
    c: f32,
}

impl CubicCurve {
    fn new(first: f32, second: f32) -> Self {
        Self {
            a: 3.0 * first - 3.0 * second + 1.0,
            b: -6.0 * first + 3.0 * second,
            c: 3.0 * first,
        }
    }
    fn sample(&self, t: f32) -> f32 {
        ((self.a * t + self.b) * t + self.c) * t
    }
    fn gradient(&self, t: f32) -> f32 {
        (3.0 * self.a * t + 2.0 * self.b) * t + self.c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noon_night_and_wrap() {
        let mut clock = DayCycle::default();
        clock.set(6000.0);
        let noon = clock.sample();
        assert!(noon.sun_direction.y > 0.99);
        assert!((noon.sky.x - 123.0 / 255.0).abs() < 1e-5);
        clock.set(18000.0);
        let night = clock.sample();
        assert!(night.sun_direction.y < -0.99);
        assert_eq!(night.sky, Vec3::ZERO);
        assert!(night.light.x < noon.light.x);
        clock.set(24000.0);
        assert!(clock.time().abs() < f64::EPSILON);
    }

    #[test]
    fn day_advances_at_twenty_ticks_per_second_until_paused() {
        let mut clock = DayCycle::default();
        let start = clock.ticks;
        clock.advance(1.0);
        assert_eq!(clock.ticks, start + 20.0);
        assert_ne!(
            clock.sample().sun_direction,
            DayCycle {
                ticks: start,
                paused: false
            }
            .sample()
            .sun_direction
        );
        clock.paused = true;
        clock.advance(1.0);
        assert_eq!(clock.ticks, start + 20.0);
    }
}
