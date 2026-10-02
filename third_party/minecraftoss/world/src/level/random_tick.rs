//! Chunk ticking on the server level (26.3 `ServerChunkCache.tickChunks`,
//! `ServerLevel.tickChunk` with `getBlockRandomPos`, `tickPrecipitation`)
//! and the random ticks of the blocks simulated so far: crops, farmland,
//! sugar cane, cactus, grass and mycelium, ice, snow layers, leaves,
//! redstone ore and weathering copper.
//!
//! Random ticks draw positions from `Level.randValue` (an LCG vanilla seeds
//! from an unseeded random) and outcomes from the level random. Blocks whose
//! random tick is not ported are recorded in `Level::unsupported`.

use super::Level;
use minecraftoss_core::block::{flags, FluidKind};
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::{BlockPos, BlockStateId, ChunkPos, HeightmapKind};

/// `Biome.Precipitation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precipitation {
    None,
    Rain,
    Snow,
}

impl Level<'_> {
    /// `Level.getBlockRandomPos(x, y, z, 15)`.
    fn block_random_pos(&mut self, x: i32, y: i32, z: i32) -> BlockPos {
        self.rand_value = self.rand_value.wrapping_mul(3).wrapping_add(1_013_904_223);
        let value = self.rand_value >> 2;
        BlockPos::new(x + (value & 15), y + (value >> 16 & 15), z + (value >> 8 & 15))
    }

    /// Whether a state makes its section tick randomly
    /// (`BlockState.isRandomlyTicking` or a randomly ticking fluid: lava).
    pub(super) fn ticks_randomly(&self, state: BlockStateId) -> bool {
        let blocks = &self.registries().blocks;
        blocks.is(state, flags::RANDOMLY_TICKING) || blocks.state(state).fluid.as_ref().is_some_and(|f| f.kind == FluidKind::Lava)
    }

    /// `LevelChunkSection.isRandomlyTicking` for a chunk's sections.
    fn section_ticks_randomly(&mut self, pos: ChunkPos, section: usize) -> bool {
        if !self.random_counts.contains_key(&pos) {
            let Some(chunk) = self.chunks.get(&pos) else { return false };
            let counts: Vec<u32> = chunk
                .sections()
                .iter()
                .map(|section| {
                    if section.is_empty() {
                        return 0;
                    }
                    match &section.blocks {
                        minecraftoss_core::palette::PalettedContainer::Single(state) => {
                            if self.ticks_randomly(*state) {
                                4096
                            } else {
                                0
                            }
                        }
                        minecraftoss_core::palette::PalettedContainer::Direct(values) => {
                            values.iter().filter(|&&state| self.ticks_randomly(state)).count() as u32
                        }
                    }
                })
                .collect();
            self.random_counts.insert(pos, counts);
        }
        self.random_counts[&pos].get(section).is_some_and(|&c| c > 0)
    }

    /// Keeps the random-tick counts in step with a block change.
    pub(super) fn note_random_ticking(&mut self, pos: BlockPos, old: BlockStateId, new: BlockStateId) {
        let (before, after) = (self.ticks_randomly(old), self.ticks_randomly(new));
        if before == after {
            return;
        }
        let min_y = self.min_y;
        if let Some(counts) = self.random_counts.get_mut(&pos.chunk()) {
            let section = ((pos.y - min_y) >> 4) as usize;
            if let Some(count) = counts.get_mut(section) {
                if after {
                    *count += 1;
                } else {
                    *count = count.saturating_sub(1);
                }
            }
        }
    }

    /// The chunks that tick, in vanilla's order when known
    /// (`DistanceManager.forEachEntityTickingChunk`).
    fn ticking_chunk_list(&self) -> Vec<ChunkPos> {
        match &self.ticking_chunks {
            Some(list) => list.iter().copied().filter(|p| self.chunks.contains_key(p)).collect(),
            None => {
                let mut list: Vec<ChunkPos> = self.chunks.keys().copied().collect();
                list.sort_by_key(|p| (p.x, p.z));
                list
            }
        }
    }

    /// `ServerChunkCache.tickChunks` without players: every entity-ticking
    /// chunk runs `ServerLevel.tickChunk`.
    pub(super) fn tick_chunks(&mut self) {
        // Natural spawning runs before the random ticks.
        self.tick_natural_spawning();
        let speed = self.random_tick_speed;
        for pos in self.ticking_chunk_list() {
            let (min_x, min_z) = (pos.x * 16, pos.z * 16);
            for _ in 0..speed {
                if self.random.next_i32_bound(48) == 0 {
                    let at = self.block_random_pos(min_x, 0, min_z);
                    self.tick_precipitation(at);
                }
            }
            if speed <= 0 {
                continue;
            }
            let sections = match self.chunks.get(&pos) {
                Some(chunk) => chunk.sections().len(),
                None => continue,
            };
            for section in 0..sections {
                if !self.section_ticks_randomly(pos, section) {
                    continue;
                }
                let min_y = self.min_y + section as i32 * 16;
                for _ in 0..speed {
                    let at = self.block_random_pos(min_x, min_y, min_z);
                    let state = self.block(at);
                    if self.registries().blocks.is(state, flags::RANDOMLY_TICKING) {
                        let before = match &self.random {
                            minecraftoss_core::random::AnyRandom::Legacy(r) => r.state(),
                            _ => 0,
                        };
                        self.random_tick(state, at);
                        static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                        if *LOG.get_or_init(|| std::env::var_os("SIM_RANDOM_TICK_LOG").is_some()) {
                            let after = match &self.random {
                                minecraftoss_core::random::AnyRandom::Legacy(r) => r.state(),
                                _ => 0,
                            };
                            if before != after {
                                eprintln!("RTICK {} {:?} {}", self.game_time, at, self.name(state));
                            }
                        }
                    }
                    // The fluid of the state read before the block's tick.
                    if self.registries().blocks.state(state).fluid.as_ref().is_some_and(|f| f.kind == FluidKind::Lava) {
                        self.unsupported.push("lava random tick (fire)".to_owned());
                    }
                }
            }
        }
    }

    /// `ServerLevel.tickPrecipitation` for warm dry weather: nothing
    /// freezes and nothing falls. Cold biomes and rain are not simulated.
    fn tick_precipitation(&mut self, pos: BlockPos) {
        use minecraftoss_generator::feature::World;
        let top = BlockPos::new(pos.x, self.height_at(HeightmapKind::MotionBlocking, pos.x, pos.z), pos.z);
        let below = top.below();
        let Some(biome) = World::biome(self, top.x, top.y, top.z) else { return };
        if self.should_freeze(biome, below) {
            let ice = self.registries().blocks.parse_state("minecraft:ice").expect("vanilla block");
            self.set_block_and_update(below, ice);
        }
        if !self.is_raining() {
            return;
        }
        let max_height = self.max_snow_accumulation_height;
        if max_height > 0 && self.should_snow(biome, top) {
            let state = self.block(top);
            if self.name(state) == "minecraft:snow" {
                let layers: i32 = self.registries().blocks.property(state, "layers").and_then(|l| l.parse().ok()).unwrap_or(1);
                if layers < max_height.min(8) {
                    let next = self.with(state, "layers", &(layers + 1).to_string());
                    let cell = super::physics::Aabb::new(f64::from(top.x), f64::from(top.y), f64::from(top.z), f64::from(top.x) + 1.0, f64::from(top.y) + 1.0, f64::from(top.z) + 1.0);
                    if self.entities.iter().any(|e| !e.removed && e.bb.intersects(&cell)) {
                        self.unsupported.push("snow pushing entities up".to_owned());
                    }
                    self.set_block_and_update(top, next);
                }
            } else {
                let snow = self.registries().blocks.parse_state("minecraft:snow").expect("vanilla block");
                self.set_block_and_update(top, snow);
            }
        }
        let precipitation = self.precipitation_at(biome, below);
        if precipitation != Precipitation::None {
            let state = self.block(below);
            self.handle_precipitation(state, below, precipitation);
        }
    }

    /// `Biome.getTemperature` (its per-position cache changes nothing).
    fn biome_temperature(&self, biome: minecraftoss_core::BiomeId, pos: BlockPos) -> f32 {
        let info = self.registries().biomes.get(biome);
        self.lib.temperature.at(info.temperature, info.frozen_temperature_modifier, pos.x, pos.y, pos.z, self.sea_level)
    }

    /// `Biome.getPrecipitationAt`.
    pub(super) fn precipitation_at(&self, biome: minecraftoss_core::BiomeId, pos: BlockPos) -> Precipitation {
        if !self.registries().biomes.get(biome).has_precipitation {
            Precipitation::None
        } else if self.biome_temperature(biome, pos) >= 0.15 {
            Precipitation::Rain
        } else {
            Precipitation::Snow
        }
    }

    fn inside_build_height(&self, y: i32) -> bool {
        y >= self.min_y && y < self.min_y + self.height
    }

    /// `Biome.shouldFreeze(level, pos, true)`: an exposed, unlit water
    /// source in the cold.
    fn should_freeze(&mut self, biome: minecraftoss_core::BiomeId, pos: BlockPos) -> bool {
        if self.biome_temperature(biome, pos) >= 0.15 || !self.inside_build_height(pos.y) || self.block_light(pos) >= 10 {
            return false;
        }
        let state = self.block(pos);
        let source = self.fluid_state(state).is_some_and(|f| f.kind == FluidKind::Water && f.source);
        if !source || !self.is_a(state, "LiquidBlock") {
            return false;
        }
        let water = |level: &Self, p: BlockPos| level.fluid_state(level.block(p)).is_some_and(|f| f.kind == FluidKind::Water);
        !(water(self, pos.west()) && water(self, pos.east()) && water(self, pos.north()) && water(self, pos.south()))
    }

    /// `Biome.shouldSnow`.
    fn should_snow(&mut self, biome: minecraftoss_core::BiomeId, pos: BlockPos) -> bool {
        if self.precipitation_at(biome, pos) != Precipitation::Snow || !self.inside_build_height(pos.y) || self.block_light(pos) >= 10 {
            return false;
        }
        let state = self.block(pos);
        let snow = self.registries().blocks.parse_state("minecraft:snow").expect("vanilla block");
        (self.registries().blocks.is_air(state) || self.name(state) == "minecraft:snow") && self.can_survive_state(snow, pos)
    }

    /// `Block.handlePrecipitation`: cauldrons fill (`CauldronBlock`,
    /// `LayeredCauldronBlock`), drawing their chance from the level random.
    fn handle_precipitation(&mut self, state: BlockStateId, pos: BlockPos, precipitation: Precipitation) {
        let layered = self.is_a(state, "LayeredCauldronBlock");
        if !layered && !self.is_a(state, "CauldronBlock") {
            return;
        }
        let happens = match precipitation {
            Precipitation::Rain => self.random.next_f32() < 0.05,
            Precipitation::Snow => self.random.next_f32() < 0.1,
            Precipitation::None => false,
        };
        if !happens {
            return;
        }
        if layered {
            let kind = match self.name(state) {
                "minecraft:water_cauldron" => Precipitation::Rain,
                "minecraft:powder_snow_cauldron" => Precipitation::Snow,
                _ => Precipitation::None,
            };
            let level: i32 = self.registries().blocks.property(state, "level").and_then(|l| l.parse().ok()).unwrap_or(3);
            if level != 3 && precipitation == kind {
                let next = self.with(state, "level", &(level + 1).to_string());
                self.set_block_and_update(pos, next);
            }
        } else {
            let name = if precipitation == Precipitation::Rain { "minecraft:water_cauldron" } else { "minecraft:powder_snow_cauldron" };
            let next = self.registries().blocks.parse_state(name).expect("vanilla block");
            self.set_block_and_update(pos, next);
        }
    }

    /// `BlockBehaviour.randomTick`.
    fn random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        if self.plant_random_tick(state, pos) {
            return;
        }
        let blocks = &self.registries().blocks;
        let info = blocks.block(blocks.block_of(state));
        let name = info.name.as_str().to_owned();
        let class = |c: &str| info.is_a(c);
        if class("CropBlock") {
            if class("BeetrootBlock") || class("TorchflowerCropBlock") {
                if self.random.next_i32_bound(3) == 0 {
                    return;
                }
            }
            self.crop_random_tick(state, pos, &name);
        } else if class("SaplingBlock") {
            self.sapling_random_tick(state, pos);
        } else if class("VineBlock") {
            self.vine_random_tick(state, pos);
        } else if class("FarmlandBlock") {
            self.farmland_random_tick(state, pos);
        } else if class("SugarCaneBlock") {
            self.sugar_cane_random_tick(state, pos);
        } else if class("CactusBlock") {
            self.cactus_random_tick(state, pos);
        } else if class("SpreadingSnowyBlock") {
            self.spreading_random_tick(state, pos);
        } else if class("IceBlock") {
            let dampening = i32::from(self.registries().blocks.state(state).light_dampening);
            if self.block_light(pos) > 11 - dampening {
                // `IceBlock.melt` outside the Nether.
                let water = self.registries().blocks.parse_state("minecraft:water").expect("vanilla block");
                self.set_block_and_update(pos, water);
                let block = self.block_id(water);
                self.add_and_run(super::Update::Simple { pos, block });
            }
        } else if class("SnowLayerBlock") {
            if self.block_light(pos) > 11 {
                self.drop_resources(state, pos);
                self.remove_block(pos, false);
            }
        } else if class("LeavesBlock") {
            if blocks.property(state, "persistent") == Some("false") && blocks.property(state, "distance") == Some("7") {
                self.drop_resources(state, pos);
                self.remove_block(pos, false);
            }
        } else if class("RedStoneOreBlock") {
            if blocks.property(state, "lit") == Some("true") {
                let next = self.with(state, "lit", "false");
                self.set_block_and_update(pos, next);
            }
        } else if info.classes().iter().any(|c| c.starts_with("Weathering")) {
            if class("WeatheringCopperDoorBlock") && blocks.property(state, "half") != Some("lower") {
                return;
            }
            self.copper_change_over_time(state, pos);
        } else {
            self.unsupported.push(format!("random tick of {name}"));
        }
    }

    /// `VineBlock.isAcceptableNeighbour` (`MultifaceBlock.canAttachTo`).
    fn vine_attaches(&self, neighbour: BlockPos, direction: Direction) -> bool {
        let state = self.block(neighbour);
        let face = direction.opposite();
        let blocks = &self.registries().blocks;
        blocks.is_face_sturdy(state, face, minecraftoss_core::SupportType::Full)
            || blocks.collision_shape(state).is_some_and(|shape| minecraftoss_generator::feature::survive::is_face_full(shape, face))
    }

    /// `VineBlock.canSupportAtFace`.
    fn vine_supported_at_face(&self, pos: BlockPos, direction: Direction) -> bool {
        if direction == Direction::Down {
            return false;
        }
        if self.vine_attaches(pos.relative(direction, 1), direction) {
            return true;
        }
        if direction.axis() == minecraftoss_core::pos::Axis::Y {
            return false;
        }
        let above = self.block(pos.above());
        self.name(above) == "minecraft:vine" && self.registries().blocks.property(above, direction.name()) == Some("true")
    }

    fn vine_face(&self, state: BlockStateId, direction: Direction) -> bool {
        self.registries().blocks.property(state, direction.name()) == Some("true")
    }

    fn vine_with(&self, state: BlockStateId, direction: Direction, value: bool) -> BlockStateId {
        self.with(state, direction.name(), if value { "true" } else { "false" })
    }

    /// `VineBlock.canSpread`: at most four vines within 4 across, 1 up/down.
    fn vine_can_spread(&self, pos: BlockPos) -> bool {
        let mut count = 0;
        for y in -1..=1 {
            for z in -4..=4 {
                for x in -4..=4 {
                    if self.name(self.block(pos.offset(x, y, z))) == "minecraft:vine" {
                        count += 1;
                        if count > 4 {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    /// `VineBlock.randomTick` (game rule `spread_vines` on).
    fn vine_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        use super::update::CLIENTS;
        if self.random.next_i32_bound(4) != 0 {
            return;
        }
        let test = Direction::ALL[self.random.next_i32_bound(6) as usize];
        let default = self.registries().blocks.block(self.block_id(state)).default_state();
        let air = |level: &Self, p: BlockPos| level.registries().blocks.is_air(level.block(p));
        let above = pos.above();
        if test.is_horizontal() && !self.vine_face(state, test) {
            if !self.vine_can_spread(pos) {
                return;
            }
            let test_pos = pos.relative(test, 1);
            if air(self, test_pos) {
                let (cw, ccw) = (test.clockwise(), test.counter_clockwise());
                let (cw_face, ccw_face) = (self.vine_face(state, cw), self.vine_face(state, ccw));
                let (cw_pos, ccw_pos) = (test_pos.relative(cw, 1), test_pos.relative(ccw, 1));
                if cw_face && self.vine_attaches(cw_pos, cw) {
                    let next = self.vine_with(default, cw, true);
                    self.set_block(test_pos, next, CLIENTS, super::update::LIMIT);
                } else if ccw_face && self.vine_attaches(ccw_pos, ccw) {
                    let next = self.vine_with(default, ccw, true);
                    self.set_block(test_pos, next, CLIENTS, super::update::LIMIT);
                } else {
                    let opposite = test.opposite();
                    if cw_face && air(self, cw_pos) && self.vine_attaches(pos.relative(cw, 1), opposite) {
                        let next = self.vine_with(default, opposite, true);
                        self.set_block(cw_pos, next, CLIENTS, super::update::LIMIT);
                    } else if ccw_face && air(self, ccw_pos) && self.vine_attaches(pos.relative(ccw, 1), opposite) {
                        let next = self.vine_with(default, opposite, true);
                        self.set_block(ccw_pos, next, CLIENTS, super::update::LIMIT);
                    } else if f64::from(self.random.next_f32()) < 0.05 && self.vine_attaches(test_pos.above(), Direction::Up) {
                        let next = self.vine_with(default, Direction::Up, true);
                        self.set_block(test_pos, next, CLIENTS, super::update::LIMIT);
                    }
                }
            } else if self.vine_attaches(test_pos, test) {
                let next = self.vine_with(state, test, true);
                self.set_block(pos, next, CLIENTS, super::update::LIMIT);
            }
            return;
        }
        if test == Direction::Up && pos.y < self.min_y + self.height - 1 {
            if self.vine_supported_at_face(pos, test) {
                let next = self.vine_with(state, Direction::Up, true);
                self.set_block(pos, next, CLIENTS, super::update::LIMIT);
                return;
            }
            if air(self, above) {
                if !self.vine_can_spread(pos) {
                    return;
                }
                let mut above_state = state;
                for direction in Direction::HORIZONTAL {
                    if self.random.next_bool() || !self.vine_attaches(above.relative(direction, 1), direction) {
                        above_state = self.vine_with(above_state, direction, false);
                    }
                }
                if Direction::HORIZONTAL.iter().any(|&d| self.vine_face(above_state, d)) {
                    self.set_block(above, above_state, CLIENTS, super::update::LIMIT);
                }
                return;
            }
        }
        if pos.y > self.min_y {
            let below = pos.below();
            let below_state = self.block(below);
            let below_is_vine = self.block_id(below_state) == self.block_id(state);
            if self.registries().blocks.is_air(below_state) || below_is_vine {
                let before = if below_is_vine { below_state } else { default };
                // `copyRandomFaces`.
                let mut after = before;
                for direction in Direction::HORIZONTAL {
                    if self.random.next_bool() && self.vine_face(state, direction) {
                        after = self.vine_with(after, direction, true);
                    }
                }
                if before != after && Direction::HORIZONTAL.iter().any(|&d| self.vine_face(after, d)) {
                    self.set_block(below, after, CLIENTS, super::update::LIMIT);
                }
            }
        }
    }

    /// `LevelLightEngine.getRawBrightness(pos, 0)`.
    pub(super) fn raw_brightness(&mut self, pos: BlockPos, sky_dampen: i32) -> i32 {
        let sky = self.sky_light(pos) - sky_dampen;
        sky.max(self.block_light(pos))
    }

    /// `CropBlock.randomTick`.
    fn crop_random_tick(&mut self, state: BlockStateId, pos: BlockPos, name: &str) {
        if self.raw_brightness(pos, 0) < 9 {
            return;
        }
        let blocks = &self.registries().blocks;
        let age: i32 = blocks.property(state, "age").and_then(|a| a.parse().ok()).unwrap_or(0);
        let max = match name {
            "minecraft:beetroots" => 3,
            "minecraft:torchflower_crop" => 1,
            _ => 7,
        };
        if age >= max {
            return;
        }
        let speed = self.crop_growth_speed(state, pos);
        if self.random.next_i32_bound((25.0f32 / speed) as i32 + 1) == 0 {
            let next = self.with(state, "age", &(age + 1).to_string());
            self.set_block(pos, next, super::update::CLIENTS, super::update::LIMIT);
        }
    }

    /// `CropBlock.getGrowthSpeed`.
    pub(super) fn crop_growth_speed(&self, state: BlockStateId, pos: BlockPos) -> f32 {
        let mut speed = 1.0f32;
        let below = pos.below();
        let blocks = &self.registries().blocks;
        for dx in -1..=1 {
            for dz in -1..=1 {
                let s = self.block(below.offset(dx, 0, dz));
                let mut block_speed = 0.0f32;
                if self.lib.registries.block_in_tag(s, self.grows_crops_tag) {
                    block_speed = 1.0;
                    if blocks.property(s, "moisture").and_then(|m| m.parse::<i32>().ok()).unwrap_or(0) > 0 {
                        block_speed = 3.0;
                    }
                }
                if dx != 0 || dz != 0 {
                    block_speed /= 4.0;
                }
                speed += block_speed;
            }
        }
        let block = blocks.block_of(state);
        let is = |p: BlockPos| blocks.block_of(self.block(p)) == block;
        let horizontal = is(pos.west()) || is(pos.east());
        let vertical = is(pos.north()) || is(pos.south());
        if horizontal && vertical {
            speed /= 2.0;
        } else if is(pos.west().north()) || is(pos.east().north()) || is(pos.east().south()) || is(pos.west().south()) {
            speed /= 2.0;
        }
        speed
    }

    /// `FarmlandBlock.randomTick` (rain is not simulated here).
    fn farmland_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let blocks = &self.registries().blocks;
        let moisture: i32 = blocks.property(state, "moisture").and_then(|m| m.parse().ok()).unwrap_or(0);
        let mut near_water = false;
        'scan: for y in 0..=1 {
            for z in -4..=4 {
                for x in -4..=4 {
                    let s = self.block(pos.offset(x, y, z));
                    if blocks.state(s).fluid.as_ref().is_some_and(|f| f.kind == FluidKind::Water) {
                        near_water = true;
                        break 'scan;
                    }
                }
            }
        }
        if !near_water && !self.is_raining_at(pos.above()) {
            if moisture > 0 {
                let next = self.with(state, "moisture", &(moisture - 1).to_string());
                self.set_block(pos, next, super::update::CLIENTS, super::update::LIMIT);
            } else if !self.lib.registries.block_in_tag(self.block(pos.above()), self.maintains_farmland_tag) {
                self.turn_to_dirt(pos);
            }
        } else if moisture < 7 {
            let next = self.with(state, "moisture", "7");
            self.set_block(pos, next, super::update::CLIENTS, super::update::LIMIT);
        }
    }

    /// `FarmlandBlock.turnToBaseBlock` / `PathBlock.turnToBaseBlock`: back
    /// to dirt, pushing entities on top up.
    pub(super) fn turn_to_dirt(&mut self, pos: BlockPos) {
        let cell = super::physics::Aabb::new(f64::from(pos.x), f64::from(pos.y) + 0.9375, f64::from(pos.z), f64::from(pos.x) + 1.0, f64::from(pos.y) + 1.0, f64::from(pos.z) + 1.0);
        if self.entities.iter().any(|e| !e.removed && e.bb.intersects(&cell)) {
            self.unsupported.push("pushing entities up".to_owned());
        }
        let dirt = self.registries().blocks.parse_state("minecraft:dirt").expect("vanilla block");
        self.set_block_and_update(pos, dirt);
    }

    /// The scheduled ticks of farmland and paths: covered by a solid block
    /// (farmland keeps under `maintains_farmland`, paths under fence
    /// gates), they revert to dirt.
    pub(super) fn covered_ground_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let above = self.block(pos.above());
        let solid = self.registries().blocks.is(above, flags::LEGACY_SOLID);
        let survives = if self.is_a(state, "FarmlandBlock") {
            !solid || self.lib.registries.block_in_tag(above, self.maintains_farmland_tag)
        } else {
            true
        };
        if !survives || self.is_a(state, "PathBlock") {
            self.turn_to_dirt(pos);
        }
    }

    /// `SugarCaneBlock.randomTick`.
    fn sugar_cane_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        if !self.registries().blocks.is_air(self.block(pos.above())) {
            return;
        }
        let block = self.block_id(state);
        let mut height = 1;
        while self.block_id(self.block(pos.offset(0, -height, 0))) == block {
            height += 1;
        }
        if height < 3 {
            let age: i32 = self.registries().blocks.property(state, "age").and_then(|a| a.parse().ok()).unwrap_or(0);
            if age == 15 {
                let default = self.registries().blocks.block(block).default_state();
                self.set_block_and_update(pos.above(), default);
                self.set_block(pos, self.with(state, "age", "0"), 260, super::update::LIMIT);
            } else {
                self.set_block(pos, self.with(state, "age", &(age + 1).to_string()), 260, super::update::LIMIT);
            }
        }
    }

    /// `CactusBlock.randomTick`.
    fn cactus_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let above = pos.above();
        if !self.registries().blocks.is_air(self.block(above)) {
            return;
        }
        let block = self.block_id(state);
        let age: i32 = self.registries().blocks.property(state, "age").and_then(|a| a.parse().ok()).unwrap_or(0);
        let mut height = 1;
        while self.block_id(self.block(pos.offset(0, -height, 0))) == block {
            height += 1;
            if height == 3 && age == 15 {
                return;
            }
        }
        let default = self.registries().blocks.block(block).default_state();
        if age == 8 && self.can_survive_state(default, above) {
            let chance = if height >= 3 { 0.25 } else { 0.1 };
            if self.random.next_f64() <= chance {
                let flower = self.registries().blocks.parse_state("minecraft:cactus_flower").expect("vanilla block");
                self.set_block_and_update(above, flower);
            }
        } else if age == 15 && height < 3 {
            self.set_block_and_update(above, default);
            let reset = self.with(state, "age", "0");
            self.set_block(pos, reset, 260, super::update::LIMIT);
            self.add_and_run(super::Update::Full { state: reset, pos: above, block });
        }
        if age < 15 {
            self.set_block(pos, self.with(state, "age", &(age + 1).to_string()), 260, super::update::LIMIT);
        }
    }

    /// `SpreadingSnowyBlock.canStayAlive`.
    fn grass_can_stay_alive(&self, state: BlockStateId, pos: BlockPos) -> bool {
        let blocks = &self.registries().blocks;
        let above = self.block(pos.above());
        if self.name(above) == "minecraft:snow" && blocks.property(above, "layers") == Some("1") {
            return true;
        }
        if blocks.state(above).fluid.as_ref().is_some_and(|f| f.amount == 8) {
            return false;
        }
        let table = &self.sky.as_ref().expect("dimension set").light;
        let dampening = if table.shape_occludes(blocks, state, above, Direction::Up) { 16 } else { i32::from(blocks.state(above).light_dampening) };
        dampening < 15
    }

    /// `SpreadingSnowyBlock.randomTick` (grass blocks, mycelium).
    fn spreading_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let dirt = self.registries().blocks.parse_state("minecraft:dirt").expect("vanilla block");
        if !self.grass_can_stay_alive(state, pos) {
            self.set_block_and_update(pos, dirt);
            return;
        }
        let darken = self.sky.as_ref().map_or(0, |s| s.sky_darken);
        if self.raw_brightness(pos.above(), darken) < 9 {
            return;
        }
        let blocks = &self.registries().blocks;
        let default = blocks.block(blocks.block_of(state)).default_state();
        for _ in 0..4 {
            let dx = self.random.next_i32_bound(3) - 1;
            let dy = self.random.next_i32_bound(5) - 3;
            let dz = self.random.next_i32_bound(3) - 1;
            let target = pos.offset(dx, dy, dz);
            if self.block(target) == dirt
                && self.grass_can_stay_alive(default, target)
                && !self.registries().blocks.state(self.block(target.above())).fluid.as_ref().is_some_and(|f| f.kind == FluidKind::Water)
            {
                let snowy = self.lib.registries.block_in_tag(self.block(target.above()), self.snow_tag);
                let next = self.with(default, "snowy", if snowy { "true" } else { "false" });
                self.set_block_and_update(target, next);
            }
        }
    }

    /// `ChangeOverTimeBlock.changeOverTime` for weathering copper.
    fn copper_change_over_time(&mut self, state: BlockStateId, pos: BlockPos) {
        if self.random.next_f32() >= 0.056_888_89 {
            return;
        }
        let stage = |level: &Self, s: BlockStateId| -> Option<usize> {
            let blocks = &level.registries().blocks;
            let info = blocks.block(blocks.block_of(s));
            if !info.classes().iter().any(|c| c.starts_with("Weathering")) {
                return None;
            }
            let bare = info.name.as_str().trim_start_matches("minecraft:");
            Some(if bare.starts_with("exposed_") {
                1
            } else if bare.starts_with("weathered_") {
                2
            } else if bare.starts_with("oxidized_") {
                3
            } else {
                0
            })
        };
        let own = stage(self, state).expect("weathering copper");
        let (mut same, mut older) = (0, 0);
        for dx in -4i32..=4 {
            for dy in -4i32..=4 {
                for dz in -4i32..=4 {
                    if dx.abs() + dy.abs() + dz.abs() > 4 || (dx, dy, dz) == (0, 0, 0) {
                        continue;
                    }
                    if let Some(found) = stage(self, self.block(pos.offset(dx, dy, dz))) {
                        if found < own {
                            return;
                        }
                        if found > own {
                            older += 1;
                        } else {
                            same += 1;
                        }
                    }
                }
            }
        }
        let chance = (older + 1) as f32 / (older + same + 1) as f32;
        let modifier = if own == 0 { 0.75f32 } else { 1.0 };
        if self.random.next_f32() < chance * chance * modifier && own < 3 {
            let blocks = &self.registries().blocks;
            let bare = blocks.block(blocks.block_of(state)).name.as_str().trim_start_matches("minecraft:").to_owned();
            let base = ["exposed_", "weathered_", "oxidized_"].iter().find_map(|p| bare.strip_prefix(p)).unwrap_or(&bare).to_owned();
            let base = if base == "copper" { "copper_block".to_owned() } else { base };
            let prefix = ["exposed_", "weathered_", "oxidized_"][own];
            let next_name = if base == "copper_block" { format!("minecraft:{prefix}copper") } else { format!("minecraft:{prefix}{base}") };
            if let Some(block) = blocks.block_by_name(&next_name) {
                let mut next = blocks.block(block).default_state();
                for property in blocks.block(blocks.block_of(state)).properties() {
                    if let Some(value) = blocks.property(state, &property.name) {
                        if let Some(s) = blocks.with_property(next, &property.name, value) {
                            next = s;
                        }
                    }
                }
                self.set_block_and_update(pos, next);
            }
        }
    }
}
