//! Time, weather and sky light on the server level (26.3
//! `ServerClockManager`, `ServerLevel.advanceWeatherCycle`,
//! `Level.updateSkyBrightness`, `LevelReader.getEffectiveSkyBrightness`)
//! and daylight detectors (`DaylightDetectorBlock`).
//!
//! Environment attributes come from `minecraftoss_core::environment`, which
//! matches vanilla exactly. Sky light is solved for a chunk when first asked
//! after a change, with the same solver as generation.

use super::Level;
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::environment::{EnvironmentSystem, Sample};
use minecraftoss_core::light::{light_chunk, LightTable};
use minecraftoss_core::{BlockPos, BlockStateId, Chunk, ChunkPos};
use std::collections::BTreeMap;
use std::sync::Arc;

/// `WeatherData` with the level's rain and thunder levels.
#[derive(Clone, Debug, Default)]
pub struct Weather {
    pub clear_time: i32,
    pub rain_time: i32,
    pub thunder_time: i32,
    pub raining: bool,
    pub thundering: bool,
    pub rain_level: f32,
    pub thunder_level: f32,
}

/// A dimension's time and sky: its environment, clocks and weather.
pub struct Sky {
    pub environment: EnvironmentSystem,
    pub default_clock: Option<String>,
    /// `ServerClockInstance.totalTicks` by clock name.
    pub clocks: BTreeMap<String, i64>,
    pub weather: Weather,
    /// `Level.canHaveWeather`.
    pub can_have_weather: bool,
    /// `DimensionType.hasSkyLight`.
    pub has_sky_light: bool,
    /// `Level.skyDarken`.
    pub sky_darken: i32,
    /// `DimensionType.ambientLight`.
    pub ambient_light: f32,
    /// Game rules `advance_time` and `advance_weather`.
    pub advance_time: bool,
    pub advance_weather: bool,
    pub light: LightTable,
}

impl Level<'_> {
    /// Sets up the dimension's environment, clocks and weather.
    pub fn set_dimension(&mut self, dimension_type: &str) -> Result<(), String> {
        let registries = &self.lib.registries;
        let (environment, presentation) = EnvironmentSystem::for_dimension(registries, dimension_type)?;
        let json = registries
            .datapack
            .read_json("dimension_type", &minecraftoss_core::ident::Identifier::parse(dimension_type)?)?;
        let flag = |key: &str| json.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
        let has_sky_light = flag("has_skylight");
        let can_have_weather = has_sky_light && !flag("has_ceiling") && dimension_type != "minecraft:the_end";
        let mut clocks = BTreeMap::new();
        for name in ["minecraft:overworld", "minecraft:the_end"] {
            clocks.insert(name.to_owned(), 0);
        }
        self.sky = Some(Sky {
            environment,
            default_clock: presentation.default_clock,
            clocks,
            weather: Weather::default(),
            can_have_weather,
            has_sky_light,
            sky_darken: 0,
            ambient_light: json.get("ambient_light").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
            advance_time: true,
            advance_weather: true,
            light: LightTable::new(&registries.blocks),
        });
        self.update_sky_brightness();
        self.dimension = Some(dimension_type.to_owned());
        self.infiniburn = json.get("infiniburn").and_then(|v| v.as_str()).and_then(|t| registries.block_tags.require(t.trim_start_matches('#')).ok());
        Ok(())
    }

    /// Game rule `advance_weather`.
    pub fn set_advance_weather(&mut self, on: bool) {
        if let Some(sky) = &mut self.sky {
            sky.advance_weather = on;
        }
    }

    /// `/time set`: the default clock's total ticks.
    pub fn set_time(&mut self, ticks: i64) {
        if let Some(sky) = &mut self.sky {
            if let Some(clock) = sky.default_clock.clone() {
                sky.clocks.insert(clock, ticks);
            }
        }
    }

    /// `/weather clear|rain|thunder <duration>` (`setWeatherParameters`).
    pub fn set_weather(&mut self, clear_time: i32, rain_time: i32, raining: bool, thundering: bool) {
        if let Some(sky) = &mut self.sky {
            let w = &mut sky.weather;
            w.clear_time = clear_time;
            w.rain_time = rain_time;
            w.thunder_time = rain_time;
            w.raining = raining;
            w.thundering = thundering;
        }
    }

    /// `ServerClockManager.tick` (runs before the levels tick).
    pub(super) fn tick_clocks(&mut self) {
        if let Some(sky) = &mut self.sky {
            if sky.advance_time {
                for ticks in sky.clocks.values_mut() {
                    *ticks += 1;
                }
            }
        }
    }

    /// `ServerLevel.advanceWeatherCycle`: new durations are drawn from the
    /// level random when one ends (`RAIN_DELAY`, `THUNDER_DURATION`, ...).
    pub(super) fn advance_weather_cycle(&mut self) {
        let Some(sky) = &mut self.sky else { return };
        let random = &mut self.random;
        let mut uniform = |min: i32, max: i32| random.next_i32_bound(max - min + 1) + min;
        if !sky.can_have_weather {
            return;
        }
        let w = &mut sky.weather;
        if sky.advance_weather {
            if w.clear_time > 0 {
                w.clear_time -= 1;
                w.thunder_time = if w.thundering { 0 } else { 1 };
                w.rain_time = if w.raining { 0 } else { 1 };
                w.thundering = false;
                w.raining = false;
            } else {
                if w.thunder_time > 0 {
                    w.thunder_time -= 1;
                    if w.thunder_time == 0 {
                        w.thundering = !w.thundering;
                    }
                } else if w.thundering {
                    w.thunder_time = uniform(3600, 15600);
                } else {
                    w.thunder_time = uniform(12000, 180000);
                }
                if w.rain_time > 0 {
                    w.rain_time -= 1;
                    if w.rain_time == 0 {
                        w.raining = !w.raining;
                    }
                } else if w.raining {
                    w.rain_time = uniform(12000, 24000);
                } else {
                    w.rain_time = uniform(12000, 180000);
                }
            }
        }
        w.thunder_level = (w.thunder_level + if w.thundering { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
        w.rain_level = (w.rain_level + if w.raining { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
    }

    fn environment_value(&self, name: &str, biome: u16) -> f32 {
        let Some(sky) = &self.sky else { return 0.0 };
        let clocks = |clock: &str| sky.clocks.get(clock).copied().unwrap_or(0);
        let sample = Sample { clock_ticks: &clocks, rain_level: sky.weather.rain_level, thunder_level: sky.weather.thunder_level, biome_weights: None, biome };
        sky.environment.value_named(name, &sample).as_f32()
    }

    /// The `visual/moon_phase` attribute's index (`MoonPhase.index`).
    pub(super) fn moon_phase(&self) -> usize {
        const PHASES: [&str; 8] =
            ["full_moon", "waning_gibbous", "third_quarter", "waning_crescent", "new_moon", "waxing_crescent", "first_quarter", "waxing_gibbous"];
        let Some(sky) = &self.sky else { return 0 };
        let clocks = |clock: &str| sky.clocks.get(clock).copied().unwrap_or(0);
        let sample = Sample { clock_ticks: &clocks, rain_level: sky.weather.rain_level, thunder_level: sky.weather.thunder_level, biome_weights: None, biome: 0 };
        match sky.environment.value_named("visual/moon_phase", &sample) {
            minecraftoss_core::environment::Value::Text(name) => PHASES.iter().position(|p| *p == name).unwrap_or(0),
            _ => 0,
        }
    }

    /// `ServerLevel.getCurrentDifficultyAt`'s inputs at a column besides the
    /// difficulty: the overworld clock, and the chunk's inhabited time and
    /// the moon's brightness when the chunk is loaded.
    pub fn difficulty_inputs(&self, x: i32, z: i32) -> (i64, i64, f32) {
        let clock = self.sky.as_ref().and_then(|s| s.clocks.get("minecraft:overworld").copied()).unwrap_or(0);
        match self.chunk(ChunkPos::new(x >> 4, z >> 4)) {
            Some(chunk) => (clock, chunk.inhabited_time, crate::natural_spawner::spawning::MOON_BRIGHTNESS_PER_PHASE[self.moon_phase()]),
            None => (clock, 0, 0.0),
        }
    }

    /// The overworld clock (`Level.getDayTime` for the villagers' schedule).
    pub fn overworld_clock(&self) -> i64 {
        self.sky.as_ref().and_then(|s| s.clocks.get("minecraft:overworld").copied()).unwrap_or(0)
    }

    /// The `gameplay/monsters_burn` attribute (daytime in the Overworld).
    pub fn monsters_burn(&self) -> bool {
        let Some(sky) = &self.sky else { return false };
        let clocks = |clock: &str| sky.clocks.get(clock).copied().unwrap_or(0);
        let sample = Sample { clock_ticks: &clocks, rain_level: sky.weather.rain_level, thunder_level: sky.weather.thunder_level, biome_weights: None, biome: 0 };
        sky.environment.value_named("gameplay/monsters_burn", &sample) == minecraftoss_core::environment::Value::Bool(true)
    }

    /// A float attribute's value in a biome.
    pub(super) fn biome_value(&self, name: &str, biome: u16) -> f32 {
        self.environment_value(name, biome)
    }

    /// `Level.updateSkyBrightness`.
    pub fn update_sky_brightness(&mut self) {
        let level = self.environment_value("gameplay/sky_light_level", 0);
        if let Some(sky) = &mut self.sky {
            sky.sky_darken = (15.0 - level) as i32;
        }
    }

    fn biome_at(&self, pos: BlockPos) -> u16 {
        self.chunk(pos.chunk()).map_or(0, |c| c.biome(((pos.x & 15) >> 2) as usize, pos.y >> 2, ((pos.z & 15) >> 2) as usize).0)
    }

    /// Solves light for a chunk from its 3x3 neighbourhood when missing.
    fn ensure_light(&mut self, pos: ChunkPos) -> bool {
        let Some(center) = self.chunks.get(&pos) else { return false };
        if center.light.is_some() {
            return true;
        }
        if let Some(lit) = self.lazy_light.borrow_mut().remove(&pos) {
            self.chunks.get_mut(&pos).expect("loaded").light = Some(lit);
            return true;
        }
        let Some(lit) = self.compute_light(pos) else { return false };
        self.chunks.get_mut(&pos).expect("loaded").light = Some(lit);
        true
    }

    /// A chunk's light from it and its eight neighbours, when all are loaded.
    fn compute_light(&self, pos: ChunkPos) -> Option<Arc<minecraftoss_core::light::ChunkLight>> {
        let started = std::time::Instant::now();
        let out = self.compute_light_inner(pos);
        let (count, ms) = self.light_solves.get();
        self.light_solves.set((count + 1, ms + started.elapsed().as_secs_f64() * 1000.0));
        out
    }

    fn compute_light_inner(&self, pos: ChunkPos) -> Option<Arc<minecraftoss_core::light::ChunkLight>> {
        let sky = self.sky.as_ref()?;
        let center = self.chunks.get(&pos)?;
        let min_section = center.min_section_y();
        let mut around: Vec<&Chunk> = Vec::with_capacity(9);
        for dz in -1..=1 {
            for dx in -1..=1 {
                around.push(self.chunks.get(&ChunkPos::new(pos.x + dx, pos.z + dz))?);
            }
        }
        let chunks: [&Chunk; 9] = std::array::from_fn(|i| around[i]);
        let non_empty = |dx: i32, dz: i32, sy: i32| {
            self.chunks
                .get(&ChunkPos::new(pos.x + dx, pos.z + dz))
                .and_then(|c| c.sections().get((sy - min_section) as usize))
                .is_some_and(|s| !s.is_empty())
        };
        Some(Arc::new(light_chunk(&self.lib.registries.blocks, &sky.light, chunks, non_empty, sky.has_sky_light)))
    }

    /// The chunk's light for readers without `&mut`: solved and kept aside
    /// until the chunk takes it (`ensure_light`) or a change invalidates it.
    fn light_for_read(&self, pos: ChunkPos) -> Option<Arc<minecraftoss_core::light::ChunkLight>> {
        let chunk = self.chunks.get(&pos)?;
        if let Some(light) = &chunk.light {
            return Some(light.clone());
        }
        if let Some(light) = self.lazy_light.borrow().get(&pos) {
            return Some(light.clone());
        }
        let light = self.compute_light(pos)?;
        self.lazy_light.borrow_mut().insert(pos, light.clone());
        Some(light)
    }

    /// Drops the light of the chunks a changed block can affect.
    pub(super) fn invalidate_light(&mut self, pos: BlockPos) {
        let center = pos.chunk();
        for dz in -1..=1 {
            for dx in -1..=1 {
                let at = ChunkPos::new(center.x + dx, center.z + dz);
                if let Some(chunk) = self.chunks.get_mut(&at) {
                    chunk.light = None;
                }
                self.lazy_light.get_mut().remove(&at);
            }
        }
    }

    /// `getBrightness(LightLayer.SKY, pos)`: sections without data read the
    /// next stored section above, or full sky above them all.
    /// `LevelReader.getMaxLocalRawBrightness`: sky light less the sky's
    /// darkening, or block light, whichever is brighter.
    pub fn max_local_raw_brightness(&mut self, pos: BlockPos) -> i32 {
        let darken = self.sky.as_ref().map_or(0, |s| s.sky_darken);
        let sky = self.sky_light(pos) - darken;
        sky.max(self.block_light(pos))
    }

    /// `DimensionType.ambientLight` (0 without a dimension).
    pub fn ambient_light(&self) -> f32 {
        self.sky.as_ref().map_or(0.0, |s| s.ambient_light)
    }

    pub fn sky_light(&mut self, pos: BlockPos) -> i32 {
        if !self.sky.as_ref().is_some_and(|s| s.has_sky_light) {
            return 0;
        }
        if !self.ensure_light(pos.chunk()) {
            return 15;
        }
        self.solved_sky_light(pos).unwrap_or(15)
    }

    /// Sky light where the chunk's light is already solved.
    pub(super) fn solved_sky_light(&self, pos: BlockPos) -> Option<i32> {
        if !self.sky.as_ref().is_some_and(|s| s.has_sky_light) {
            return Some(0);
        }
        let light = self.light_for_read(pos.chunk())?;
        let nibble = |data: &[u8; 2048], x: i32, y: i32, z: i32| {
            let index = ((y & 15) << 8 | (z & 15) << 4 | (x & 15)) as usize;
            i32::from(data[index >> 1] >> ((index & 1) * 4) & 15)
        };
        let mut section = (pos.y >> 4) - light.min_section;
        if section < 0 {
            return Some(0);
        }
        let mut y = pos.y;
        while (section as usize) < light.sky.len() {
            if let Some(data) = &light.sky[section as usize] {
                return Some(nibble(data, pos.x, y, pos.z));
            }
            section += 1;
            y = 0;
        }
        Some(15)
    }

    /// `getBrightness(LightLayer.BLOCK, pos)`: 0 where no data is stored.
    pub fn block_light(&mut self, pos: BlockPos) -> i32 {
        if self.sky.is_none() || !self.ensure_light(pos.chunk()) {
            return 0;
        }
        self.solved_block_light(pos).unwrap_or(0)
    }

    /// Block light where the chunk's light is already solved.
    pub(super) fn solved_block_light(&self, pos: BlockPos) -> Option<i32> {
        self.sky.as_ref()?;
        let light = self.light_for_read(pos.chunk())?;
        let section = (pos.y >> 4) - light.min_section;
        if section < 0 {
            return Some(0);
        }
        Some(match light.block.get(section as usize).and_then(|d| d.as_ref()) {
            Some(data) => {
                let index = ((pos.y & 15) << 8 | (pos.z & 15) << 4 | (pos.x & 15)) as usize;
                i32::from(data[index >> 1] >> ((index & 1) * 4) & 15)
            }
            None => 0,
        })
    }

    /// Solves light for the chunks around a position, so rules that read
    /// light through `World::raw_brightness` see it.
    pub(super) fn ensure_light_around(&mut self, pos: BlockPos) {
        let center = pos.chunk();
        for dz in -1..=1 {
            for dx in -1..=1 {
                self.ensure_light(minecraftoss_core::ChunkPos::new(center.x + dx, center.z + dz));
            }
        }
    }

    /// `LevelReader.getEffectiveSkyBrightness`.
    pub fn effective_sky_brightness(&mut self, pos: BlockPos) -> i32 {
        let darken = self.sky.as_ref().map_or(0, |s| s.sky_darken);
        (self.sky_light(pos) - darken).max(0)
    }

    // ---- daylight detectors -----------------------------------------------------

    /// `DaylightDetectorBlock.updateSignalStrength`.
    pub(super) fn daylight_update(&mut self, state: BlockStateId, pos: BlockPos) {
        let mut target = self.effective_sky_brightness(pos);
        let mut sun_angle = self.environment_value("visual/sun_angle", self.biome_at(pos)) * (std::f32::consts::PI / 180.0);
        let blocks = &self.registries().blocks;
        if blocks.property(state, "inverted") == Some("true") {
            target = 15 - target;
        } else if target > 0 {
            let offset = if sun_angle < std::f32::consts::PI { 0.0 } else { std::f32::consts::PI * 2.0 };
            sun_angle += (offset - sun_angle) * 0.2;
            // `Math.round(float)`.
            target = (target as f32 * minecraftoss_generator::providers::cos(f64::from(sun_angle)) + 0.5).floor() as i32;
        }
        let target = target.clamp(0, 15);
        if blocks.property(state, "power").and_then(|p| p.parse::<i32>().ok()) != Some(target) {
            let next = self.with(state, "power", &target.to_string());
            self.set_block_and_update(pos, next);
        }
    }

    /// The daylight detector's block entity tick.
    pub(super) fn daylight_tick(&mut self, pos: BlockPos) {
        if self.game_time % 20 == 0 {
            let state = self.block(pos);
            self.daylight_update(state, pos);
        }
    }

    /// `DaylightDetectorBlock.useWithoutItem`.
    pub(super) fn daylight_use(&mut self, pos: BlockPos) {
        let state = self.block(pos);
        let inverted = self.registries().blocks.property(state, "inverted") == Some("true");
        let next = self.with(state, "inverted", if inverted { "false" } else { "true" });
        self.set_block(pos, next, super::update::CLIENTS, super::update::LIMIT);
        self.daylight_update(next, pos);
    }
}
