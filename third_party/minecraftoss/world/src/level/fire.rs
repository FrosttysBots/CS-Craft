//! Fire on the server level (26.3 `FireBlock`, `BaseFireBlock`,
//! `SoulFireBlock`): scheduled fire ticks that age, spread and burn blocks
//! with the level random, flammability from the block state catalog
//! (`FireBlock.bootStrap`'s `setFlammable` tables), and the shared
//! placement and survival rules in `minecraftoss_generator::feature::update`.

use super::update;
use super::Level;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::{BlockPos, BlockStateId};
use minecraftoss_generator::feature::update::{fire_can_burn, fire_odds, fire_state_with_age};
use minecraftoss_generator::feature::Ctx;

impl Level<'_> {
    fn fire_with_age(&mut self, pos: BlockPos, age: i32) -> BlockStateId {
        let lib = self.lib;
        let ctx: Ctx = Ctx { lib, region: self };
        fire_state_with_age(&ctx, pos, age)
    }

    fn can_burn(&self, state: BlockStateId) -> bool {
        fire_can_burn(self.registries(), state)
    }

    /// `FireBlock.isValidFireLocation`.
    fn valid_fire_location(&self, pos: BlockPos) -> bool {
        Direction::ALL.iter().any(|&d| self.can_burn(self.block(pos.relative(d, 1))))
    }

    fn sturdy_below(&self, pos: BlockPos) -> bool {
        self.registries().blocks.is_face_sturdy(self.block(pos.below()), Direction::Up, minecraftoss_core::SupportType::Full)
    }

    /// `FireBlock.canSurvive`.
    fn fire_survives(&self, pos: BlockPos) -> bool {
        self.sturdy_below(pos) || self.valid_fire_location(pos)
    }

    /// `FireBlock.getIgniteOdds(level, pos)`: for air, the most flammable
    /// neighbour's odds.
    fn ignite_odds_at(&self, pos: BlockPos) -> i32 {
        if !self.registries().blocks.is_air(self.block(pos)) {
            return 0;
        }
        Direction::ALL.iter().map(|&d| fire_odds(self.registries(), self.block(pos.relative(d, 1)), false)).max().unwrap_or(0)
    }

    /// `ServerLevel.canSpreadFireAround`: within the game rule's radius of
    /// a player, or everywhere at -1.
    fn can_spread_fire_around(&self, pos: BlockPos) -> bool {
        let radius = self.fire_spread_radius;
        if radius == -1 {
            return true;
        }
        let target = [f64::from(pos.x), f64::from(pos.y), f64::from(pos.z)];
        self.players.iter().any(|p| {
            let d = [p[0] - target[0], p[1] - target[1], p[2] - target[2]];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() < f64::from(radius)
        })
    }

    /// `Level.isRaining`.
    pub(super) fn is_raining(&self) -> bool {
        self.sky.as_ref().is_some_and(|s| s.weather.rain_level > 0.2)
    }

    /// `Level.isRainingAt`: raining, open to the sky, at or above the
    /// motion-blocking surface, in a biome where it rains there.
    pub fn is_raining_at(&mut self, pos: BlockPos) -> bool {
        use minecraftoss_generator::feature::World;
        if !self.is_raining() || self.sky_light(pos) < 15 {
            return false;
        }
        if self.height_at(minecraftoss_core::HeightmapKind::MotionBlocking, pos.x, pos.z) > pos.y {
            return false;
        }
        World::biome(self, pos.x, pos.y, pos.z).is_some_and(|b| self.precipitation_at(b, pos) == super::random_tick::Precipitation::Rain)
    }

    fn is_near_rain(&mut self, pos: BlockPos) -> bool {
        self.is_raining_at(pos) || self.is_raining_at(pos.west()) || self.is_raining_at(pos.east()) || self.is_raining_at(pos.north()) || self.is_raining_at(pos.south())
    }

    /// The environment attribute `gameplay/increased_fire_burnout` at a
    /// block (the noise biome there).
    fn increased_fire_burnout(&self, pos: BlockPos) -> bool {
        let Some(sky) = &self.sky else { return false };
        let Some(chunk) = self.chunk(pos.chunk()) else { return false };
        let biome = chunk.biome(((pos.x >> 2) & 3) as usize, pos.y >> 2, ((pos.z >> 2) & 3) as usize);
        let clocks = |clock: &str| sky.clocks.get(clock).copied().unwrap_or(0);
        let sample = minecraftoss_core::environment::Sample {
            clock_ticks: &clocks,
            rain_level: sky.weather.rain_level,
            thunder_level: sky.weather.thunder_level,
            biome_weights: None,
            biome: biome.0,
        };
        sky.environment.value_named("gameplay/increased_fire_burnout", &sample).as_bool()
    }

    /// `BaseFireBlock.onPlace` (and `FireBlock.onPlace`'s first tick).
    pub(super) fn fire_on_place(&mut self, state: BlockStateId, pos: BlockPos, old: BlockStateId) {
        let fire = self.name(state) == "minecraft:fire";
        if self.block_id(old) != self.block_id(state) {
            let portal_dimension = matches!(self.dimension.as_deref(), Some("minecraft:overworld" | "minecraft:the_nether"));
            if portal_dimension && Direction::ALL.iter().any(|&d| self.name(self.block(pos.relative(d, 1))) == "minecraft:obsidian") {
                self.unsupported.push("nether portal lighting".to_owned());
            }
            let survives = if fire { self.fire_survives(pos) } else { self.lib.registries.block_in_tag(self.block(pos.below()), self.lib.update_rules.soul_fire_base) };
            if !survives {
                self.remove_block(pos, false);
            }
        }
        if fire {
            let delay = 30 + self.random.next_i32_bound(10);
            let block = self.block_id(state);
            self.schedule_block_tick_priority(pos, block, delay, super::redstone::priority::NORMAL);
        }
    }

    /// `FireBlock.tick`.
    pub(super) fn fire_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let delay = 30 + self.random.next_i32_bound(10);
        let block = self.block_id(state);
        self.schedule_block_tick_priority(pos, block, delay, super::redstone::priority::NORMAL);
        if !self.can_spread_fire_around(pos) {
            return;
        }
        if !self.fire_survives(pos) {
            self.remove_block(pos, false);
        }
        let below = self.block(pos.below());
        let infiniburn = self.infiniburn.is_some_and(|tag| self.lib.registries.block_in_tag(below, tag));
        let age: i32 = self.registries().blocks.property(state, "age").and_then(|a| a.parse().ok()).unwrap_or(0);
        if !infiniburn && self.is_raining() && self.is_near_rain(pos) && self.random.next_f32() < 0.2 + age as f32 * 0.03 {
            self.remove_block(pos, false);
            return;
        }
        let new_age = (age + self.random.next_i32_bound(3) / 2).min(15);
        if age != new_age {
            let next = self.with(state, "age", &new_age.to_string());
            self.set_block(pos, next, update::SKIP_BLOCK_ENTITY_SIDEEFFECTS | update::INVISIBLE, update::LIMIT);
        }
        if !infiniburn {
            if !self.valid_fire_location(pos) {
                if !self.sturdy_below(pos) || age > 3 {
                    self.remove_block(pos, false);
                }
                return;
            }
            if age == 15 && self.random.next_i32_bound(4) == 0 && !self.can_burn(self.block(pos.below())) {
                self.remove_block(pos, false);
                return;
            }
        }
        let increased = self.increased_fire_burnout(pos);
        let extra = if increased { -50 } else { 0 };
        self.check_burn_out(pos.east(), 300 + extra, age);
        self.check_burn_out(pos.west(), 300 + extra, age);
        self.check_burn_out(pos.below(), 250 + extra, age);
        self.check_burn_out(pos.above(), 250 + extra, age);
        self.check_burn_out(pos.north(), 300 + extra, age);
        self.check_burn_out(pos.south(), 300 + extra, age);
        for xx in -1..=1 {
            for zz in -1..=1 {
                for yy in -1..=4 {
                    if xx == 0 && yy == 0 && zz == 0 {
                        continue;
                    }
                    let rate = if yy > 1 { 100 + (yy - 1) * 100 } else { 100 };
                    let test = pos.offset(xx, yy, zz);
                    let ignite = self.ignite_odds_at(test);
                    if ignite <= 0 {
                        continue;
                    }
                    let mut odds = (ignite + 40 + self.difficulty * 7) / (age + 30);
                    if increased {
                        odds /= 2;
                    }
                    if odds > 0 && self.random.next_i32_bound(rate) <= odds && (!self.is_raining() || !self.is_near_rain(test)) {
                        let spread_age = (age + self.random.next_i32_bound(5) / 4).min(15);
                        let next = self.fire_with_age(test, spread_age);
                        self.set_block_and_update(test, next);
                    }
                }
            }
        }
    }

    /// `FireBlock.checkBurnOut`.
    fn check_burn_out(&mut self, pos: BlockPos, chance: i32, age: i32) {
        let old = self.block(pos);
        let odds = fire_odds(self.registries(), old, true);
        if self.random.next_i32_bound(chance) < odds {
            if self.random.next_i32_bound(age + 10) < 5 && !self.is_raining_at(pos) {
                let new_age = (age + self.random.next_i32_bound(5) / 4).min(15);
                let next = self.fire_with_age(pos, new_age);
                self.set_block_and_update(pos, next);
            } else {
                self.remove_block(pos, false);
            }
            if self.is_a(old, "TntBlock") {
                self.prime_tnt(pos);
            }
        }
    }
}
