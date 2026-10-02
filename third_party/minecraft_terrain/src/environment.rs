//! The camera's view of a dimension's environment attributes
//! (`EnvironmentAttributeProbe`) and the sky, fog and lightmap inputs the
//! renderer derives from them (`SkyRenderer.extractRenderState`,
//! `FogRenderer`/`AtmosphericFogEnvironment`, `LightmapRenderStateExtractor`).
//!
//! Attribute values come from `minecraftoss_core::environment`, which
//! matches vanilla captures exactly. Each tick samples the attributes at the
//! camera with Gaussian biome blending; frames interpolate between the last
//! two ticks with each attribute's partial-tick lerp.

use crate::day_cycle::{SkyState, BLOCK_FACTOR};
use glam::Vec3;
use minecraftoss_core::environment::{biome_weights, lerp_value, DimensionPresentation, EnvironmentSystem, Purpose, Sample, Value};
use minecraftoss_core::Registries;

/// The attributes the renderer reads, in `Used` order.
const USED: [&str; 18] = [
    "visual/fog_color",
    "visual/fog_start_distance",
    "visual/fog_end_distance",
    "visual/sky_fog_end_distance",
    "visual/cloud_fog_end_distance",
    "visual/sky_color",
    "visual/sunrise_sunset_color",
    "visual/cloud_color",
    "visual/sun_angle",
    "visual/moon_angle",
    "visual/star_angle",
    "visual/moon_phase",
    "visual/star_brightness",
    "visual/block_light_tint",
    "visual/sky_light_color",
    "visual/sky_light_factor",
    "visual/ambient_light_color",
    "gameplay/sky_light_level",
];

#[derive(Clone, Copy)]
enum Used {
    FogColor,
    FogStart,
    FogEnd,
    SkyFogEnd,
    CloudFogEnd,
    SkyColor,
    SunriseSunset,
    CloudColor,
    SunAngle,
    MoonAngle,
    StarAngle,
    MoonPhase,
    StarBrightness,
    BlockLightTint,
    SkyLightColor,
    SkyLightFactor,
    AmbientLightColor,
    SkyLightLevel,
}

/// `MoonPhase` in index order.
const MOON_PHASES: [&str; 8] = [
    "full_moon",
    "waning_gibbous",
    "third_quarter",
    "waning_crescent",
    "new_moon",
    "waxing_crescent",
    "first_quarter",
    "waxing_gibbous",
];

/// `ClientLevelData.voidDarknessOnsetRange` outside flat worlds.
const VOID_DARKNESS_ONSET_RANGE: f32 = 32.0;

pub struct DimensionEnvironment {
    system: EnvironmentSystem,
    pub presentation: DimensionPresentation,
    indices: [usize; USED.len()],
    last: Vec<Value>,
    current: Vec<Value>,
    /// `AtmosphericFogEnvironment.rainFogMultiplier`.
    rain_fog: f32,
}

/// What the camera sees this frame.
pub struct View {
    pub partial_tick: f32,
    pub forward: Vec3,
    pub camera_y: f32,
    pub render_distance: u32,
    pub rain_level: f32,
    pub thunder_level: f32,
}

impl DimensionEnvironment {
    /// The environment of a dimension type (`minecraft:the_nether`).
    pub fn load(registries: &Registries, dimension_type: &str) -> Result<Self, String> {
        let (system, presentation) = EnvironmentSystem::for_dimension(registries, dimension_type)?;
        let mut indices = [0; USED.len()];
        for (slot, name) in indices.iter_mut().zip(USED) {
            *slot = system.registry.index(name).ok_or_else(|| format!("unknown attribute {name}"))?;
        }
        let defaults: Vec<Value> = indices.iter().map(|&i| system.registry.attributes[i].default.clone()).collect();
        Ok(Self { system, presentation, indices, last: defaults.clone(), current: defaults, rain_fog: 0.0 })
    }

    /// `EnvironmentAttributeProbe.tick`: samples every used attribute at the
    /// camera. The first tick after loading fills both ends.
    pub fn tick(&mut self, clock_ticks: i64, rain_level: f32, thunder_level: f32, camera: [f64; 3], noise_biome: impl Fn(i32, i32, i32) -> u16, first: bool) {
        let weights = biome_weights([camera[0] * 0.25, camera[1] * 0.25, camera[2] * 0.25], &noise_biome);
        let own = noise_biome((camera[0].floor() as i32) >> 2, (camera[1].floor() as i32) >> 2, (camera[2].floor() as i32) >> 2);
        let clock = move |_: &str| clock_ticks;
        let sample = Sample { clock_ticks: &clock, rain_level, thunder_level, biome_weights: Some(&weights), biome: own };
        let values: Vec<Value> = self.indices.iter().map(|&i| self.system.value(i, &sample)).collect();
        self.last = if first { values.clone() } else { std::mem::replace(&mut self.current, values.clone()) };
        self.current = values;
    }

    /// `AtmosphericFogEnvironment.updateRainFogState`, once per frame:
    /// `sky_light` is the camera's sky light and `delta_ticks` the frame time.
    pub fn update_rain_fog(&mut self, rain_level: f32, sky_light: u8, rains_in_biome: bool, delta_ticks: f32) {
        let exposure = ((f32::from(sky_light) - 8.0) / 7.0).clamp(0.0, 1.0);
        let target = rain_level * exposure * if rains_in_biome { 1.0 } else { 0.5 };
        self.rain_fog += (target - self.rain_fog) * delta_ticks * 0.2;
    }

    fn value(&self, used: Used, partial_tick: f32) -> Value {
        let slot = used as usize;
        let kind = self.system.registry.attributes[self.indices[slot]].kind;
        lerp_value(kind, Purpose::PartialTick, partial_tick, &self.last[slot], &self.current[slot])
    }

    fn float(&self, used: Used, partial_tick: f32) -> f32 {
        self.value(used, partial_tick).as_f32()
    }

    fn rgb(&self, used: Used, partial_tick: f32) -> Vec3 {
        Vec3::from(self.value(used, partial_tick).as_rgb())
    }

    /// `gameplay/sky_light_level` at the end of the last tick, for sky darkening.
    pub fn sky_light_level(&self) -> f32 {
        self.current[Used::SkyLightLevel as usize].as_f32()
    }

    /// `visual/sun_angle` in degrees.
    pub fn sun_angle_degrees(&self, partial_tick: f32) -> f32 {
        self.float(Used::SunAngle, partial_tick)
    }

    pub fn sky_state(&self, view: &View) -> SkyState {
        let t = view.partial_tick;
        let sun = self.float(Used::SunAngle, t).to_radians();
        let moon = self.float(Used::MoonAngle, t).to_radians();
        let star = self.float(Used::StarAngle, t).to_radians();
        let sunset = self.value(Used::SunriseSunset, t).as_argb();
        let sky = self.rgb(Used::SkyColor, t);
        let phase = self.value(Used::MoonPhase, t);
        let moon_phase = MOON_PHASES.iter().position(|p| *p == phase.as_text()).unwrap_or(0);

        // AtmosphericFogEnvironment.getBaseColor.
        let mut fog = self.rgb(Used::FogColor, t);
        if view.render_distance >= 4 {
            let sun_x = if sun.sin() > 0.0 { -1.0 } else { 1.0 };
            let facing = view.forward.x * sun_x;
            let alpha = sunset[3];
            if facing > 0.0 && alpha > 0.0 {
                fog = joml_lerp(fog, Vec3::new(sunset[0], sunset[1], sunset[2]), facing * alpha);
            }
        }
        let darkened_sky = weather_darken(sky, view.rain_level, view.thunder_level);
        let sky_fog_end = self.float(Used::SkyFogEnd, t);
        let sky_fog_chunks = (sky_fog_end / 16.0).min(view.render_distance as f32);
        let mix = 1.0 - clamped_lerp(sky_fog_chunks / 32.0, 0.25, 1.0).powf(0.25);
        fog = joml_lerp(fog, darkened_sky, mix);
        // FogRenderer.computeFogColor: darkness toward the void.
        let min_y = self.presentation.min_y as f32;
        let darkness = ((VOID_DARKNESS_ONSET_RANGE + min_y - view.camera_y) / VOID_DARKNESS_ONSET_RANGE).clamp(0.0, 1.0);
        if darkness > 0.0 {
            fog *= (1.0 - darkness) * (1.0 - darkness);
        }

        // AtmosphericFogEnvironment.setupFog.
        let fog_end = self.float(Used::FogEnd, t);
        let fog_start = self.float(Used::FogStart, t) - 160.0 * self.rain_fog;
        let fog_end = fog_end.min(96.0).max(fog_end - 256.0 * self.rain_fog);

        let sky_light_color = self.rgb(Used::SkyLightColor, t);
        let sky_light_factor = self.float(Used::SkyLightFactor, t);
        let ambient = self.rgb(Used::AmbientLightColor, t);
        let cloud = self.value(Used::CloudColor, t).as_argb();
        SkyState {
            sky,
            fog,
            light: open_sky_light(ambient, sky_light_color, sky_light_factor),
            sky_light_color,
            sky_light_factor,
            cloud: Vec3::new(cloud[0], cloud[1], cloud[2]),
            cloud_alpha: cloud[3],
            sunset,
            sun_direction: Vec3::new(sun.sin(), sun.cos(), 0.0),
            moon_direction: Vec3::new(moon.sin(), moon.cos(), 0.0),
            star_brightness: self.float(Used::StarBrightness, t),
            star_angle: -star,
            moon_phase,
            rain_brightness: 1.0 - view.rain_level,
            ambient,
            block_light_tint: self.rgb(Used::BlockLightTint, t),
            block_factor: BLOCK_FACTOR,
            fog_start,
            fog_end,
            sky_fog_end,
            cloud_fog_end: self.float(Used::CloudFogEnd, t),
            skybox: self.presentation.skybox,
        }
    }
}

/// `AtmosphericFogEnvironment.applyWeatherDarken`.
fn weather_darken(color: Vec3, rain: f32, thunder: f32) -> Vec3 {
    let mut color = color;
    if rain > 0.0 {
        let scale = 1.0 - rain * 0.5;
        color = (color * Vec3::new(scale, scale, 1.0 - rain * 0.4)).clamp(Vec3::ZERO, Vec3::ONE);
    }
    if thunder > 0.0 {
        color = (color * (1.0 - thunder * 0.5)).clamp(Vec3::ZERO, Vec3::ONE);
    }
    color
}

/// The lightmap at open sky and no block light, at the default brightness 0.5.
fn open_sky_light(ambient: Vec3, sky_light_color: Vec3, sky_light_factor: f32) -> Vec3 {
    let raw = (ambient + sky_light_color * sky_light_factor).clamp(Vec3::ZERO, Vec3::ONE);
    let max = raw.max_element();
    let gamma = if max > 0.0 { raw * ((1.0 - (1.0 - max).powi(4)) / max) } else { Vec3::ZERO };
    raw.lerp(gamma, 0.5)
}

/// `Mth.clampedLerp`.
fn clamped_lerp(factor: f32, min: f32, max: f32) -> f32 {
    if factor < 0.0 {
        min
    } else if factor > 1.0 {
        max
    } else {
        min + factor * (max - min)
    }
}

/// `ARGB.srgbLerp` (JOML lerp without fused multiply-add).
fn joml_lerp(from: Vec3, to: Vec3, alpha: f32) -> Vec3 {
    Vec3::new((to.x - from.x) * alpha + from.x, (to.y - from.y) * alpha + from.y, (to.z - from.z) * alpha + from.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::day_cycle::{DayCycle, Skybox};
    use minecraftoss_core::registries::DataPaths;

    fn registries() -> Option<Registries> {
        let paths = DataPaths::discover().ok()?;
        paths.block_catalog.is_file().then(|| Registries::load(&paths).unwrap())
    }

    /// The data-driven Overworld reproduces the authored clock's keyframed
    /// values (sky, stars, light) away from the approximated sunset.
    #[test]
    fn overworld_matches_the_authored_clock() {
        let Some(registries) = registries() else { return };
        let plains = registries.biomes.id("minecraft:plains").unwrap().0;
        let mut env = DimensionEnvironment::load(&registries, "minecraft:overworld").unwrap();
        for ticks in (0..24_000).step_by(250) {
            env.tick(ticks, 0.0, 0.0, [0.5, 80.0, 0.5], |_, _, _| plains, true);
            let view = View { partial_tick: 1.0, forward: Vec3::Z, camera_y: 80.0, render_distance: 16, rain_level: 0.0, thunder_level: 0.0 };
            let ours = env.sky_state(&view);
            let old = DayCycle { ticks: ticks as f64, paused: true }.sample_for_biome_at(crate::terrain::plains_sample(&registries), 16);
            assert!((ours.sky - old.sky).abs().max_element() < 1e-5, "sky at {ticks}: {:?} vs {:?}", ours.sky, old.sky);
            assert!((ours.sky_light_factor - old.sky_light_factor).abs() < 1e-5, "light factor at {ticks}");
            assert!((ours.star_brightness - old.star_brightness).abs() < 1e-5, "stars at {ticks}");
            assert!((ours.sun_direction - old.sun_direction).abs().max_element() < 1e-5, "sun at {ticks}");
        }
    }

    #[test]
    fn nether_and_end_skyboxes() {
        let Some(registries) = registries() else { return };
        let nether = DimensionEnvironment::load(&registries, "minecraft:the_nether").unwrap();
        assert_eq!(nether.presentation.skybox, Skybox::None);
        let end = DimensionEnvironment::load(&registries, "minecraft:the_end").unwrap();
        assert_eq!(end.presentation.skybox, Skybox::End);
    }
}
