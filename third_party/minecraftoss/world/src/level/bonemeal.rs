//! Bone meal on the server level (26.3 `BoneMealItem.growCrop` and
//! `growWaterPlant`, the `BonemealableBlock` implementations of crops,
//! stems, berries, cocoa, saplings, azaleas, bamboo, growing plants, cave
//! vine berries, grass and ferns, tall flowers, flower beds, mushrooms and
//! seagrass). Every draw comes from the level random.

use super::update;
use super::Level;
use minecraftoss_core::block::FluidKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::{BlockPos, BlockStateId};

/// `Mth.nextInt(random, min, max)`.
fn next_int_between(level: &mut Level<'_>, min: i32, max: i32) -> i32 {
    if min >= max {
        min
    } else {
        level.random.next_i32_bound(max - min + 1) + min
    }
}

impl Level<'_> {
    fn bm_int(&self, state: BlockStateId, name: &str) -> i32 {
        self.registries().blocks.property(state, name).and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    fn inside_height(&self, y: i32) -> bool {
        y >= self.min_y && y < self.min_y + self.height
    }

    fn air_at(&self, pos: BlockPos) -> bool {
        self.registries().blocks.is_air(self.block(pos))
    }

    /// A configured feature's trunk base height (`TreeGrower.getMinimumHeight`)
    /// or huge mushroom foliage radius, from the data pack.
    fn feature_config_int(&self, feature: &str, path: &[&str]) -> Option<i32> {
        let id = minecraftoss_core::ident::Identifier::parse(feature).ok()?;
        let json = self.registries().datapack.read_json("worldgen/feature", &id).ok()?;
        let mut value = &json["config"];
        for key in path {
            value = &value[*key];
        }
        value.as_i64().map(|v| v as i32)
    }

    /// `BoneMealItem.growCrop`: true when the block took the bone meal (the
    /// item is used up then, grown or not).
    pub fn grow_crop(&mut self, pos: BlockPos) -> bool {
        let state = self.block(pos);
        let is = |c: &str| self.is_a(state, c);
        let name = self.name(state).to_owned();
        if is("CropBlock") {
            let max = match name.as_str() {
                "minecraft:beetroots" => 3,
                "minecraft:torchflower_crop" => 2,
                _ => 7,
            };
            let age = self.bm_int(state, "age");
            if age >= max {
                return false;
            }
            // `getBonemealAgeIncrease`: 2-5, a third of it for beetroots, 1
            // for torchflowers.
            let increase = match name.as_str() {
                "minecraft:torchflower_crop" => 1,
                "minecraft:beetroots" => next_int_between(self, 2, 5) / 3,
                _ => next_int_between(self, 2, 5),
            };
            let grown = (age + increase).min(max);
            // `getStateForAge`: the default state at that age.
            let default = self.registries().blocks.block(self.block_id(state)).default_state();
            let mut next = self.with(default, "age", &grown.to_string());
            if name == "minecraft:torchflower_crop" && grown >= 2 {
                next = self.registries().blocks.parse_state("minecraft:torchflower").expect("vanilla block");
            }
            self.set_block(pos, next, update::CLIENTS, update::LIMIT);
            return true;
        }
        if is("StemBlock") {
            let age = self.bm_int(state, "age");
            if age == 7 {
                return false;
            }
            let grown = (age + next_int_between(self, 2, 5)).min(7);
            let next = self.with(state, "age", &grown.to_string());
            self.set_block(pos, next, update::CLIENTS, update::LIMIT);
            if grown == 7 {
                // `newState.randomTick(level, pos, random)`.
                self.plant_random_tick(next, pos);
            }
            return true;
        }
        if is("SweetBerryBushBlock") {
            let age = self.bm_int(state, "age");
            if age >= 3 {
                return false;
            }
            let next = self.with(state, "age", &(age + 1).min(3).to_string());
            self.set_block(pos, next, update::CLIENTS, update::LIMIT);
            return true;
        }
        if is("CocoaBlock") {
            let age = self.bm_int(state, "age");
            if age >= 2 {
                return false;
            }
            let next = self.with(state, "age", &(age + 1).to_string());
            self.set_block(pos, next, update::CLIENTS, update::LIMIT);
            return true;
        }
        if is("SaplingBlock") {
            return self.bonemeal_sapling(state, pos);
        }
        if is("AzaleaBlock") {
            let min = self.feature_config_int("minecraft:azalea_tree", &["trunk_placer", "base_height"]).unwrap_or(0);
            let fluid_above = self.fluid_state(self.block(pos.above())).is_some();
            if !self.inside_height(pos.y + min + 2) || fluid_above {
                return false;
            }
            if self.random.next_f32() < 0.45 {
                self.grow_azalea(state, pos);
            }
            return true;
        }
        if is("BambooSaplingBlock") {
            if !self.air_at(pos.above()) || !self.inside_height(pos.y + 1) {
                return false;
            }
            let bamboo = self.registries().blocks.parse_state("minecraft:bamboo[leaves=small]").expect("vanilla block");
            self.set_block(pos.above(), bamboo, update::ALL, update::LIMIT);
            return true;
        }
        if is("BambooStalkBlock") {
            return self.bonemeal_bamboo(pos);
        }
        if name == "minecraft:cave_vines" || name == "minecraft:cave_vines_plant" {
            if self.registries().blocks.property(state, "berries") == Some("true") {
                return false;
            }
            let next = self.with(state, "berries", "true");
            self.set_block(pos, next, update::CLIENTS, update::LIMIT);
            return true;
        }
        if is("GrowingPlantHeadBlock") {
            return self.bonemeal_growing_head(state, pos);
        }
        if is("GrowingPlantBodyBlock") {
            // `getHeadPos`: along the growth direction past this block.
            let (direction, head) = match name.as_str() {
                "minecraft:kelp_plant" => (Direction::Up, "minecraft:kelp"),
                "minecraft:twisting_vines_plant" => (Direction::Up, "minecraft:twisting_vines"),
                "minecraft:weeping_vines_plant" => (Direction::Down, "minecraft:weeping_vines"),
                _ => {
                    self.unsupported.push(format!("bone meal on {name}"));
                    return false;
                }
            };
            let block = self.block_id(state);
            let mut at = pos.relative(direction, 1);
            while self.block_id(self.block(at)) == block {
                at = at.relative(direction, 1);
            }
            let head_state = self.block(at);
            if self.name(head_state) != head {
                return false;
            }
            return self.bonemeal_growing_head(head_state, at);
        }
        if is("TallGrassBlock") {
            let grown = match name.as_str() {
                "minecraft:fern" => "minecraft:large_fern",
                _ => "minecraft:tall_grass",
            };
            let grown = self.registries().blocks.parse_state(grown).expect("vanilla block");
            if !(self.can_survive_state(grown, pos) && self.air_at(pos.above()) && self.inside_height(pos.y + 1)) {
                return false;
            }
            self.place_double_plant(grown, pos, update::CLIENTS);
            return true;
        }
        if is("TallFlowerBlock") {
            // `popResource(level, pos, new ItemStack(this))`.
            self.pop_resource(pos, minecraftoss_core::item::ItemStack::new(&name, 1));
            return true;
        }
        if is("FlowerBedBlock") {
            let amount = self.bm_int(state, "flower_amount");
            if amount < 4 {
                let next = self.with(state, "flower_amount", &(amount + 1).to_string());
                self.set_block(pos, next, update::CLIENTS, update::LIMIT);
            } else {
                self.pop_resource(pos, minecraftoss_core::item::ItemStack::new(&name, 1));
            }
            return true;
        }
        if is("MushroomBlock") {
            let feature = if name == "minecraft:red_mushroom" { "minecraft:huge_red_mushroom" } else { "minecraft:huge_brown_mushroom" };
            let radius = self.feature_config_int(feature, &["foliage_radius"]).unwrap_or(0);
            if !self.inside_height(pos.y + 4 + radius) {
                return false;
            }
            if self.random.next_f32() < 0.4 {
                // `growMushroom`.
                self.remove_block(pos, false);
                if !self.place_gameplay_feature(feature, pos) {
                    self.set_block_and_update(pos, state);
                }
            }
            return true;
        }
        if name == "minecraft:seagrass" {
            if self.name(self.block(pos.above())) != "minecraft:water" {
                return false;
            }
            let lower = self.registries().blocks.parse_state("minecraft:tall_seagrass[half=lower]").expect("vanilla block");
            let upper = self.registries().blocks.parse_state("minecraft:tall_seagrass[half=upper]").expect("vanilla block");
            self.set_block(pos, lower, update::CLIENTS, update::LIMIT);
            self.set_block(pos.above(), upper, update::CLIENTS, update::LIMIT);
            return true;
        }
        if is("GrassBlock") {
            if !self.air_at(pos.above()) || !self.inside_height(pos.y + 1) {
                return false;
            }
            self.bonemeal_grass(state, pos);
            return true;
        }
        if is("NyliumBlock")
            || is("MossyCarpetBlock")
            || is("RootedDirtBlock")
            || is("NetherrackBlock")
            || is("BonemealableFeaturePlacerBlock")
            || is("NetherFungusBlock")
            || is("BigDripleafBlock")
            || is("BigDripleafStemBlock")
            || is("SmallDripleafBlock")
            || is("GlowLichenBlock")
            || is("SeaPickleBlock")
            || is("PitcherCropBlock")
            || is("BushBlock")
            || is("FireflyBushBlock")
            || is("ShortDryGrassBlock")
            || is("TallDryGrassBlock")
            || is("HangingMossBlock")
            || is("MangroveLeavesBlock")
            || is("ShelfMushroomBlock")
        {
            self.unsupported.push(format!("bone meal on {name}"));
        }
        false
    }

    /// `GrassBlock.performBonemeal`: 128 short random walks over grass,
    /// each ending in grass, the biome's bone-meal flowers or taller grass.
    fn bonemeal_grass(&mut self, state: BlockStateId, pos: BlockPos) {
        let block = self.block_id(state);
        let above = pos.above();
        'attempts: for attempt in 0..128 {
            let mut test = above;
            for _ in 0..attempt / 16 {
                let dx = self.random.next_i32_bound(3) - 1;
                let dy = (self.random.next_i32_bound(3) - 1) * self.random.next_i32_bound(3) / 2;
                let dz = self.random.next_i32_bound(3) - 1;
                test = test.offset(dx, dy, dz);
                // `stopBonemealSpread`.
                if self.block_id(self.block(test.below())) != block || self.registries().blocks.state(self.block(test)).collision_full_block {
                    continue 'attempts;
                }
            }
            self.place_bonemeal_effect(test);
        }
    }

    /// `GrassBlock.placeBonemealEffect`.
    fn place_bonemeal_effect(&mut self, test: BlockPos) {
        use minecraftoss_generator::feature::World;
        let test_state = self.block(test);
        if self.name(test_state) == "minecraft:short_grass" && self.random.next_f32() < 0.1 {
            let tall = self.registries().blocks.parse_state("minecraft:tall_grass").expect("vanilla block");
            if self.can_survive_state(tall, test) && self.air_at(test.above()) && self.inside_height(test.y + 1) {
                self.place_double_plant(tall, test, update::CLIENTS);
            }
        }
        if self.registries().blocks.is_air(test_state) && self.inside_height(test.y) {
            if self.random.next_f32() < 0.125 {
                let Some(biome) = World::biome(self, test.x, test.y, test.z) else { return };
                let features = self.lib.bone_meal_features(biome);
                if features.is_empty() {
                    return;
                }
                let pick = features[self.random.next_i32_bound(features.len() as i32) as usize];
                self.with_feature_random(|ctx, random| minecraftoss_generator::feature::place_feature(ctx, random, pick, test));
            } else {
                self.place_gameplay_placed("minecraft:grass_bonemeal", test);
            }
        }
    }

    /// `SaplingBlock` (and `MangrovePropaguleBlock`) bone meal.
    fn bonemeal_sapling(&mut self, state: BlockStateId, pos: BlockPos) -> bool {
        let name = self.name(state).to_owned();
        if name == "minecraft:mangrove_propagule" && self.registries().blocks.property(state, "hanging") == Some("true") {
            let age = self.bm_int(state, "age");
            if age >= 4 {
                return false;
            }
            let next = self.with(state, "age", &(age + 1).to_string());
            self.set_block(pos, next, update::CLIENTS, update::LIMIT);
            return true;
        }
        // `TreeGrower.canGrow` draws the tree choices from the level random.
        let Some(can) = self.tree_grower_can_grow(state, pos) else {
            self.unsupported.push(format!("bone meal on {name}"));
            return false;
        };
        if !can {
            return false;
        }
        let shortest = match name.as_str() {
            "minecraft:oak_sapling" => Some("minecraft:oak"),
            "minecraft:spruce_sapling" => Some("minecraft:spruce"),
            "minecraft:mangrove_propagule" => Some("minecraft:mangrove"),
            "minecraft:birch_sapling" => Some("minecraft:birch"),
            "minecraft:jungle_sapling" => Some("minecraft:jungle_tree_no_vine"),
            "minecraft:acacia_sapling" => Some("minecraft:acacia"),
            "minecraft:cherry_sapling" => Some("minecraft:cherry"),
            "minecraft:poplar_sapling" => Some("minecraft:red_poplar"),
            _ => None,
        };
        let min = shortest.and_then(|f| self.feature_config_int(f, &["trunk_placer", "base_height"])).unwrap_or(0);
        if !self.inside_height(pos.y + min) {
            return false;
        }
        if self.random.next_f32() < 0.45 {
            self.advance_tree(state, pos);
        }
        true
    }

    /// `BambooStalkBlock` bone meal.
    fn bonemeal_bamboo(&mut self, pos: BlockPos) -> bool {
        let is_bamboo = |level: &Self, p: BlockPos| level.name(level.block(p)) == "minecraft:bamboo";
        let count = |level: &Self, step: i32| {
            let mut h = 0;
            while h < 16 && is_bamboo(level, pos.offset(0, step * (h + 1), 0)) {
                h += 1;
            }
            h
        };
        let (mut above, below) = (count(self, 1), count(self, -1));
        let growth = pos.offset(0, above + 1, 0);
        let top_stage = self.bm_int(self.block(pos.offset(0, above, 0)), "stage");
        if !(above + below + 1 < 16 && top_stage != 1 && self.inside_height(growth.y) && self.air_at(growth)) {
            return false;
        }
        let mut total = above + below + 1;
        let new_bamboo = 1 + self.random.next_i32_bound(2);
        for _ in 0..new_bamboo {
            let top = pos.offset(0, above, 0);
            let top_state = self.block(top);
            let growth = top.above();
            if total >= 16 || self.bm_int(top_state, "stage") == 1 || !self.air_at(growth) || !self.inside_height(growth.y) {
                break;
            }
            self.grow_bamboo_from(top_state, top, total);
            above += 1;
            total += 1;
        }
        true
    }

    /// `GrowingPlantHeadBlock` bone meal: kelp and the nether vines.
    fn bonemeal_growing_head(&mut self, state: BlockStateId, pos: BlockPos) -> bool {
        let name = self.name(state).to_owned();
        let (direction, water) = match name.as_str() {
            "minecraft:kelp" => (Direction::Up, true),
            "minecraft:twisting_vines" => (Direction::Up, false),
            "minecraft:weeping_vines" => (Direction::Down, false),
            _ => {
                self.unsupported.push(format!("bone meal on {name}"));
                return false;
            }
        };
        let grows_into = |level: &Self, p: BlockPos| if water { level.name(level.block(p)) == "minecraft:water" } else { level.air_at(p) };
        let forward = pos.relative(direction, 1);
        if !(grows_into(self, forward) && self.inside_height(forward.y)) {
            return false;
        }
        let blocks = if water {
            1
        } else {
            // `NetherVines.getBlocksToGrowWhenBonemealed`.
            let mut chance = 1.0;
            let mut count = 0;
            while self.random.next_f64() < chance {
                chance *= 0.826;
                count += 1;
            }
            count
        };
        let mut at = forward;
        let mut age = (self.bm_int(state, "age") + 1).min(25);
        let mut grown = 0;
        while grown < blocks && grows_into(self, at) && self.inside_height(at.y) {
            let next = self.with(state, "age", &age.to_string());
            self.set_block_and_update(at, next);
            at = at.relative(direction, 1);
            age = (age + 1).min(25);
            grown += 1;
        }
        true
    }

    /// `DoublePlantBlock.placeAt`, copying water into waterloggable halves.
    fn place_double_plant(&mut self, state: BlockStateId, lower: BlockPos, flags: u32) {
        let upper = lower.above();
        for (pos, half) in [(lower, "lower"), (upper, "upper")] {
            let mut s = self.with(state, "half", half);
            let water = self.fluid_state(self.block(pos)).is_some_and(|f| f.kind == FluidKind::Water && f.source);
            if self.registries().blocks.property(s, "waterlogged").is_some() {
                s = self.with(s, "waterlogged", if water { "true" } else { "false" });
            }
            self.set_block(pos, s, flags, update::LIMIT);
        }
    }

    /// `BoneMealItem.growWaterPlant` outside coral biomes: seagrass spread
    /// through full water around the target.
    pub fn grow_water_plant(&mut self, pos: BlockPos) -> bool {
        use minecraftoss_generator::feature::World;
        let full_water = |level: &Self, p: BlockPos| {
            let s = level.block(p);
            level.name(s) == "minecraft:water" && level.fluid_state(s).is_some_and(|f| f.amount == 8)
        };
        if !full_water(self, pos) {
            return false;
        }
        let corals = self.lib.registries.biome_tags.require("minecraft:produces_corals_from_bonemeal").ok();
        let seagrass = self.registries().blocks.parse_state("minecraft:seagrass").expect("vanilla block");
        'attempts: for j in 0..128 {
            let mut test = pos;
            for _ in 0..j / 16 {
                let dx = self.random.next_i32_bound(3) - 1;
                let dy = (self.random.next_i32_bound(3) - 1) * self.random.next_i32_bound(3) / 2;
                let dz = self.random.next_i32_bound(3) - 1;
                test = test.offset(dx, dy, dz);
                if self.registries().blocks.state(self.block(test)).collision_full_block {
                    continue 'attempts;
                }
            }
            if let (Some(tag), Some(biome)) = (corals, World::biome(self, test.x, test.y, test.z)) {
                if self.lib.registries.biome_in_tag(biome, tag) {
                    self.unsupported.push("bone meal corals".to_owned());
                    return true;
                }
            }
            if self.can_survive_state(seagrass, test) {
                let there = self.block(test);
                if full_water(self, test) {
                    self.set_block_and_update(test, seagrass);
                } else if self.name(there) == "minecraft:seagrass" && self.name(self.block(test.above())) == "minecraft:water" && self.random.next_i32_bound(10) == 0 {
                    let lower = self.registries().blocks.parse_state("minecraft:tall_seagrass[half=lower]").expect("vanilla block");
                    let upper = self.registries().blocks.parse_state("minecraft:tall_seagrass[half=upper]").expect("vanilla block");
                    self.set_block(test, lower, update::CLIENTS, update::LIMIT);
                    self.set_block(test.above(), upper, update::CLIENTS, update::LIMIT);
                }
            }
        }
        true
    }
}
