//! Random-tick growth of plants and crystals on the server level (26.3
//! `GrowingPlantHeadBlock` with kelp, cave vines and the nether vines,
//! `BambooStalkBlock`, `BambooSaplingBlock`, `SweetBerryBushBlock`,
//! `CocoaBlock`, `StemBlock`, `MushroomBlock`, `NetherWartBlock`,
//! `BuddingAmethystBlock`).

use super::update;
use super::Level;
use minecraftoss_core::block::FluidKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::{BlockPos, BlockStateId};

impl Level<'_> {
    fn int_property(&self, state: BlockStateId, name: &str) -> i32 {
        self.registries().blocks.property(state, name).and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    fn state_named(&self, name: &str) -> BlockStateId {
        self.registries().blocks.parse_state(name).expect("vanilla block")
    }

    /// Plant random ticks this module handles; false for other blocks.
    pub(super) fn plant_random_tick(&mut self, state: BlockStateId, pos: BlockPos) -> bool {
        let is = |c: &str| self.is_a(state, c);
        if is("GrowingPlantHeadBlock") {
            self.growing_head_random_tick(state, pos);
        } else if is("BambooStalkBlock") {
            self.bamboo_random_tick(state, pos);
        } else if is("BambooSaplingBlock") {
            // `BambooSaplingBlock.randomTick`.
            if self.random.next_i32_bound(3) == 0 && self.registries().blocks.is_air(self.block(pos.above())) && self.raw_brightness(pos.above(), 0) >= 9 {
                let bamboo = self.state_named("minecraft:bamboo[leaves=small]");
                self.set_block(pos.above(), bamboo, update::ALL, update::LIMIT);
            }
        } else if is("SweetBerryBushBlock") {
            let age = self.int_property(state, "age");
            if age < 3 && self.random.next_i32_bound(5) == 0 && self.raw_brightness(pos.above(), 0) >= 9 {
                let next = self.with(state, "age", &(age + 1).to_string());
                self.set_block(pos, next, update::CLIENTS, update::LIMIT);
            }
        } else if is("CocoaBlock") {
            if self.random.next_i32_bound(5) == 0 {
                let age = self.int_property(state, "age");
                if age < 2 {
                    let next = self.with(state, "age", &(age + 1).to_string());
                    self.set_block(pos, next, update::CLIENTS, update::LIMIT);
                }
            }
        } else if is("StemBlock") {
            self.stem_random_tick(state, pos);
        } else if is("MushroomBlock") {
            self.mushroom_random_tick(state, pos);
        } else if is("NetherWartBlock") {
            let age = self.int_property(state, "age");
            if age < 3 && self.random.next_i32_bound(10) == 0 {
                let next = self.with(state, "age", &(age + 1).to_string());
                self.set_block(pos, next, update::CLIENTS, update::LIMIT);
            }
        } else if is("BuddingAmethystBlock") {
            self.budding_amethyst_random_tick(pos);
        } else {
            return false;
        }
        true
    }

    /// `GrowingPlantHeadBlock.randomTick`: kelp up into water, cave vines
    /// down and the nether vines into air.
    fn growing_head_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let name = self.name(state).to_owned();
        let (direction, chance) = match name.as_str() {
            "minecraft:kelp" => (Direction::Up, 0.14),
            "minecraft:cave_vines" => (Direction::Down, 0.1),
            "minecraft:twisting_vines" => (Direction::Up, 0.1),
            "minecraft:weeping_vines" => (Direction::Down, 0.1),
            _ => {
                self.unsupported.push(format!("random tick of {name}"));
                return;
            }
        };
        let age = self.int_property(state, "age");
        if age >= 25 || self.random.next_f64() >= chance {
            return;
        }
        let growth = pos.relative(direction, 1);
        let target = self.block(growth);
        let grows = if name == "minecraft:kelp" { self.name(target) == "minecraft:water" } else { self.registries().blocks.is_air(target) };
        if !grows {
            return;
        }
        // `getGrowIntoState`: the next age; cave vines roll their berries.
        let mut next = self.with(state, "age", &(age + 1).to_string());
        if name == "minecraft:cave_vines" {
            let berries = self.random.next_f32() < 0.11;
            next = self.with(next, "berries", if berries { "true" } else { "false" });
        }
        self.set_block_and_update(growth, next);
    }

    /// `BambooStalkBlock.randomTick` and `growBamboo`.
    fn bamboo_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        if self.int_property(state, "stage") != 0 {
            return;
        }
        if !(self.random.next_i32_bound(3) == 0 && self.registries().blocks.is_air(self.block(pos.above())) && self.raw_brightness(pos.above(), 0) >= 9) {
            return;
        }
        let is_bamboo = |level: &Self, s: BlockStateId| level.name(s) == "minecraft:bamboo";
        let mut below_count = 0;
        while below_count < 16 && is_bamboo(self, self.block(pos.offset(0, -(below_count + 1), 0))) {
            below_count += 1;
        }
        let height = below_count + 1;
        if height >= 16 {
            return;
        }
        self.grow_bamboo_from(state, pos, height);
    }

    /// `BambooStalkBlock.growBamboo`: a new stalk above, with leaves by the
    /// stalks below.
    pub(super) fn grow_bamboo_from(&mut self, state: BlockStateId, pos: BlockPos, height: i32) {
        let is_bamboo = |level: &Self, s: BlockStateId| level.name(s) == "minecraft:bamboo";
        let below = self.block(pos.below());
        let two_below_pos = pos.offset(0, -2, 0);
        let two_below = self.block(two_below_pos);
        let leaves_of = |level: &Self, s: BlockStateId| level.registries().blocks.property(s, "leaves").unwrap_or("none").to_owned();
        let mut leaves = "none";
        if height >= 1 {
            if !is_bamboo(self, below) || leaves_of(self, below) == "none" {
                leaves = "small";
            } else {
                leaves = "large";
                if is_bamboo(self, two_below) {
                    let small = self.with(below, "leaves", "small");
                    self.set_block_and_update(pos.below(), small);
                    let none = self.with(two_below, "leaves", "none");
                    self.set_block_and_update(two_below_pos, none);
                }
            }
        }
        let age = if self.int_property(state, "age") != 1 && !is_bamboo(self, two_below) { 0 } else { 1 };
        let stage = if (height < 11 || !(self.random.next_f32() < 0.25)) && height != 15 { 0 } else { 1 };
        let mut grown = self.registries().blocks.block(self.block_id(state)).default_state();
        grown = self.with(grown, "age", &age.to_string());
        grown = self.with(grown, "leaves", leaves);
        grown = self.with(grown, "stage", &stage.to_string());
        self.set_block_and_update(pos.above(), grown);
    }

    /// `StemBlock.randomTick`: grow, then set a fruit beside the stem.
    fn stem_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        if self.raw_brightness(pos, 0) < 9 {
            return;
        }
        let speed = self.crop_growth_speed(state, pos);
        if self.random.next_i32_bound((25.0f32 / speed) as i32 + 1) != 0 {
            return;
        }
        let age = self.int_property(state, "age");
        if age < 7 {
            let next = self.with(state, "age", &(age + 1).to_string());
            self.set_block(pos, next, update::CLIENTS, update::LIMIT);
            return;
        }
        let direction = Direction::HORIZONTAL[self.random.next_i32_bound(4) as usize];
        let relative = pos.relative(direction, 1);
        let (fruit, attached, support) = match self.name(state) {
            "minecraft:pumpkin_stem" => ("minecraft:pumpkin", "minecraft:attached_pumpkin_stem", "minecraft:supports_pumpkin_stem_fruit"),
            _ => ("minecraft:melon", "minecraft:attached_melon_stem", "minecraft:supports_melon_stem_fruit"),
        };
        let tag = self.lib.registries.block_tags.require(support).expect("tag exists");
        if self.registries().blocks.is_air(self.block(relative)) && self.lib.registries.block_in_tag(self.block(relative.below()), tag) {
            let fruit = self.state_named(fruit);
            self.set_block_and_update(relative, fruit);
            let stem = self.state_named(&format!("{attached}[facing={}]", direction.name()));
            self.set_block_and_update(pos, stem);
        }
    }

    /// `MushroomBlock.randomTick`: a short random walk to a dark spot.
    fn mushroom_random_tick(&mut self, state: BlockStateId, mut pos: BlockPos) {
        if self.random.next_i32_bound(25) != 0 {
            return;
        }
        // `canSpread`: at most four of the same mushroom within 4 across.
        let block = self.block_id(state);
        let mut count = 0;
        for y in -1..=1 {
            for z in -4..=4 {
                for x in -4..=4 {
                    if self.block_id(self.block(pos.offset(x, y, z))) == block {
                        count += 1;
                    }
                }
            }
        }
        if count > 4 {
            return;
        }
        self.ensure_light_around(pos);
        let step = |level: &mut Self, from: BlockPos| {
            let dx = level.random.next_i32_bound(3) - 1;
            let dy = level.random.next_i32_bound(2) - level.random.next_i32_bound(2);
            let dz = level.random.next_i32_bound(3) - 1;
            from.offset(dx, dy, dz)
        };
        let fits = |level: &Self, p: BlockPos| level.registries().blocks.is_air(level.block(p)) && level.can_survive_state(state, p);
        let mut offset = step(self, pos);
        for _ in 0..4 {
            if fits(self, offset) {
                pos = offset;
            }
            offset = step(self, pos);
        }
        if fits(self, offset) {
            self.set_block(offset, state, update::CLIENTS, update::LIMIT);
        }
    }

    /// `SpongeBlock.tryAbsorbWater`: a breadth-first search (depth 6, at most
    /// 65 nodes) that drains water, picks up waterlogging and washes kelp
    /// and seagrass away; the sponge turns wet if anything was taken.
    pub(super) fn sponge_try_absorb(&mut self, start: BlockPos) {
        let mut queue = std::collections::VecDeque::from([(start, 0)]);
        let mut visited = std::collections::HashSet::new();
        let mut count = 0;
        while let Some((pos, depth)) = queue.pop_front() {
            if !visited.insert(pos) {
                continue;
            }
            let accept = if pos == start {
                true
            } else {
                let state = self.block(pos);
                let water = self.fluid_state(state).is_some_and(|f| f.kind == FluidKind::Water);
                if !water {
                    false
                } else if self.bucket_pickup(pos).is_some() {
                    true
                } else if self.is_a(state, "LiquidBlock") {
                    self.set_block_and_update(pos, BlockStateId::AIR);
                    true
                } else if matches!(self.name(state), "minecraft:kelp" | "minecraft:kelp_plant" | "minecraft:seagrass" | "minecraft:tall_seagrass") {
                    self.drop_resources(state, pos);
                    self.set_block_and_update(pos, BlockStateId::AIR);
                    true
                } else {
                    false
                }
            };
            if !accept {
                continue;
            }
            count += 1;
            if count >= 65 {
                break;
            }
            if depth < 6 {
                for direction in Direction::ALL {
                    queue.push_back((pos.relative(direction, 1), depth + 1));
                }
            }
        }
        if count > 1 {
            let wet = self.state_named("minecraft:wet_sponge");
            self.set_block(start, wet, update::CLIENTS, update::LIMIT);
        }
    }

    /// `WetSpongeBlock.onPlace`: where water evaporates, it dries at once.
    pub(super) fn wet_sponge_on_place(&mut self, pos: BlockPos) {
        if self.dimension.as_deref() == Some("minecraft:the_nether") {
            let sponge = self.state_named("minecraft:sponge");
            self.set_block(pos, sponge, update::ALL, update::LIMIT);
            // The extinguish sound's pitch comes from the level random.
            let _ = self.random.next_f32();
        }
    }

    /// The scheduled tick of living coral: dead, dry, where no water is found.
    pub(super) fn coral_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let lib = self.lib;
        let wet = {
            let ctx: minecraftoss_generator::feature::Ctx = minecraftoss_generator::feature::Ctx { lib, region: self };
            minecraftoss_generator::feature::update::coral_scan_for_water(&ctx, state, pos)
        };
        if wet {
            return;
        }
        let dead_name = format!("minecraft:dead_{}", self.name(state).trim_start_matches("minecraft:"));
        let mut dead = self.state_named(&dead_name);
        if self.registries().blocks.property(dead, "waterlogged").is_some() {
            dead = self.with(dead, "waterlogged", "false");
        }
        if let Some(facing) = self.registries().blocks.property(state, "facing").map(str::to_owned) {
            dead = self.with(dead, "facing", &facing);
        }
        self.set_block(pos, dead, update::CLIENTS, update::LIMIT);
    }

    /// `BuddingAmethystBlock.randomTick`.
    fn budding_amethyst_random_tick(&mut self, pos: BlockPos) {
        if self.random.next_i32_bound(5) != 0 {
            return;
        }
        let direction = Direction::ALL[self.random.next_i32_bound(6) as usize];
        let grow = pos.relative(direction, 1);
        let there = self.block(grow);
        let fluid = self.fluid_state(there);
        let facing_here = self.registries().blocks.property(there, "facing") == Some(direction.name());
        let source_water = fluid.is_some_and(|f| f.kind == FluidKind::Water && f.source);
        // `canClusterGrowAtState`: air, or a water block that is full.
        let full_water = self.name(there) == "minecraft:water" && fluid.is_some_and(|f| f.amount == 8);
        let next = if self.registries().blocks.is_air(there) || full_water {
            Some("minecraft:small_amethyst_bud")
        } else if facing_here {
            match self.name(there) {
                "minecraft:small_amethyst_bud" => Some("minecraft:medium_amethyst_bud"),
                "minecraft:medium_amethyst_bud" => Some("minecraft:large_amethyst_bud"),
                "minecraft:large_amethyst_bud" => Some("minecraft:amethyst_cluster"),
                _ => None,
            }
        } else {
            None
        };
        if let Some(name) = next {
            let waterlogged = if source_water { "true" } else { "false" };
            let target = self.state_named(&format!("{name}[facing={},waterlogged={waterlogged}]", direction.name()));
            self.set_block_and_update(grow, target);
        }
    }
}
