//! Plant growth that places configured features on the live level (26.3
//! `SaplingBlock`, `MangrovePropaguleBlock`, `TreeGrower`).
//!
//! Vanilla hands the level random itself to `Feature.place`; the generator's
//! features run here through a `WorldgenRandom` wrapping that same legacy
//! random, so every draw lands in the level's stream, and their block
//! changes go through `Level::set_block` with the features' own flags.

use super::update;
use super::Level;
use minecraftoss_core::random::{AnyRandom, RandomSource, WorldgenRandom};
use minecraftoss_core::{BlockPos, BlockStateId};
use minecraftoss_generator::feature::{place_feature, Ctx};

/// `TreeGrower`: weighted configured features.
struct Grower {
    trees: &'static [(&'static str, i32)],
    mega: &'static [(&'static str, i32)],
    flowers: &'static [(&'static str, i32)],
}

/// The grower of a sapling block (`Blocks` registrations).
fn grower(name: &str) -> Option<Grower> {
    Some(match name {
        "minecraft:oak_sapling" => Grower {
            trees: &[("minecraft:oak", 9), ("minecraft:fancy_oak", 1)],
            mega: &[],
            flowers: &[("minecraft:oak_bees_005", 9), ("minecraft:fancy_oak_bees_005", 1)],
        },
        "minecraft:spruce_sapling" => Grower {
            trees: &[("minecraft:spruce", 1)],
            mega: &[("minecraft:mega_spruce", 1), ("minecraft:mega_pine", 1)],
            flowers: &[],
        },
        "minecraft:mangrove_propagule" => Grower { trees: &[("minecraft:mangrove", 15), ("minecraft:tall_mangrove", 85)], mega: &[], flowers: &[] },
        "minecraft:birch_sapling" => Grower { trees: &[("minecraft:birch", 1)], mega: &[], flowers: &[("minecraft:birch_bees_005", 1)] },
        "minecraft:jungle_sapling" => Grower { trees: &[("minecraft:jungle_tree_no_vine", 1)], mega: &[("minecraft:mega_jungle_tree", 1)], flowers: &[] },
        "minecraft:acacia_sapling" => Grower { trees: &[("minecraft:acacia", 1)], mega: &[], flowers: &[] },
        "minecraft:cherry_sapling" => Grower { trees: &[("minecraft:cherry", 1)], mega: &[], flowers: &[("minecraft:cherry_bees_005", 1)] },
        "minecraft:dark_oak_sapling" => Grower { trees: &[], mega: &[("minecraft:dark_oak", 1)], flowers: &[] },
        "minecraft:pale_oak_sapling" => Grower { trees: &[], mega: &[("minecraft:pale_oak_bonemeal", 1)], flowers: &[] },
        "minecraft:azalea" | "minecraft:flowering_azalea" => Grower { trees: &[("minecraft:azalea_tree", 1)], mega: &[], flowers: &[] },
        "minecraft:poplar_sapling" => Grower {
            trees: &[("minecraft:red_poplar", 1), ("minecraft:orange_poplar", 1), ("minecraft:yellow_poplar", 1)],
            mega: &[],
            flowers: &[],
        },
        _ => return None,
    })
}

impl Level<'_> {
    /// `WeightedList.getRandom`: no draw for an empty list.
    fn weighted_pick(&mut self, list: &'static [(&'static str, i32)]) -> Option<&'static str> {
        let total: i32 = list.iter().map(|(_, w)| w).sum();
        if total == 0 {
            return None;
        }
        let mut selection = self.random.next_i32_bound(total);
        for (name, weight) in list {
            if selection < *weight {
                return Some(name);
            }
            selection -= weight;
        }
        None
    }

    /// `SaplingBlock.randomTick` (and `MangrovePropaguleBlock`'s).
    pub(super) fn sapling_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        if self.is_a(state, "MangrovePropaguleBlock") {
            if self.registries().blocks.property(state, "hanging") == Some("true") {
                if self.registries().blocks.property(state, "age") != Some("4") {
                    let age: i32 = self.registries().blocks.property(state, "age").and_then(|a| a.parse().ok()).unwrap_or(0);
                    let next = self.with(state, "age", &(age + 1).to_string());
                    self.set_block(pos, next, update::CLIENTS, update::LIMIT);
                }
                return;
            }
            if self.random.next_i32_bound(7) == 0 {
                self.advance_tree(state, pos);
            }
            return;
        }
        let darken = self.sky.as_ref().map_or(0, |s| s.sky_darken);
        if self.raw_brightness(pos.above(), darken) >= 9 && self.random.next_i32_bound(7) == 0 {
            self.advance_tree(state, pos);
        }
    }

    /// `SaplingBlock.advanceTree`.
    pub(super) fn advance_tree(&mut self, state: BlockStateId, pos: BlockPos) {
        if self.registries().blocks.property(state, "stage") == Some("0") {
            let next = self.with(state, "stage", "1");
            self.set_block(pos, next, update::SKIP_BLOCK_ENTITY_SIDEEFFECTS | update::INVISIBLE, update::LIMIT);
        } else {
            self.grow_tree(state, pos);
        }
    }

    /// `TreeGrower.canGrow`: draws the tree choices; a grower with only a
    /// 2x2 tree needs four saplings. `None` for blocks without a grower.
    pub(super) fn tree_grower_can_grow(&mut self, state: BlockStateId, pos: BlockPos) -> Option<bool> {
        let grower = grower(self.name(state))?;
        let flowers = !grower.flowers.is_empty() && self.has_flowers(pos);
        let feature = self.weighted_pick(if flowers { grower.flowers } else { grower.trees });
        let mega = self.weighted_pick(grower.mega);
        Some(if feature.is_none() && mega.is_some() { self.two_by_two_saplings(state, pos).is_some() } else { true })
    }

    /// `TreeGrower.AZALEA.growTree`.
    pub(super) fn grow_azalea(&mut self, state: BlockStateId, pos: BlockPos) {
        self.grow_tree(state, pos);
    }

    /// `TreeGrower.growTree`.
    fn grow_tree(&mut self, state: BlockStateId, pos: BlockPos) -> bool {
        let name = self.name(state).to_owned();
        let Some(grower) = grower(&name) else {
            self.unsupported.push(format!("tree growth of {name}"));
            return false;
        };
        if let Some(mega) = self.weighted_pick(grower.mega) {
            if let Some((dx, dz, saplings)) = self.two_by_two_saplings(state, pos) {
                for &(_, p) in &saplings {
                    self.remove_sapling(p);
                }
                if self.place_gameplay_feature(mega, pos.offset(dx, 0, dz)) {
                    return true;
                }
                for &(s, p) in &saplings {
                    self.set_block(p, s, update::SKIP_BLOCK_ENTITY_SIDEEFFECTS | update::INVISIBLE, update::LIMIT);
                }
                return false;
            }
        }
        let flowers = !grower.flowers.is_empty() && self.has_flowers(pos);
        let Some(feature) = self.weighted_pick(if flowers { grower.flowers } else { grower.trees }) else { return false };
        self.remove_sapling(pos);
        if self.place_gameplay_feature(feature, pos) {
            return true;
        }
        self.set_block(pos, state, update::SKIP_BLOCK_ENTITY_SIDEEFFECTS | update::INVISIBLE, update::LIMIT);
        false
    }

    /// `TreeGrower.removeSapling`: the position's fluid, flags 818.
    fn remove_sapling(&mut self, pos: BlockPos) {
        let empty = self.fluid_legacy_block(self.fluid_state(self.block(pos)));
        let flags = update::SKIP_ON_PLACE | update::SKIP_BLOCK_ENTITY_SIDEEFFECTS | update::SUPPRESS_DROPS | update::KNOWN_SHAPE | update::CLIENTS;
        self.set_block(pos, empty, flags, update::LIMIT);
    }

    /// `TreeGrower.findTwoByTwoSaplingPos`.
    fn two_by_two_saplings(&self, state: BlockStateId, pos: BlockPos) -> Option<(i32, i32, Vec<(BlockStateId, BlockPos)>)> {
        let block = self.block_id(state);
        for dx in [0, -1] {
            for dz in [0, -1] {
                let around: Vec<(BlockStateId, BlockPos)> = [(dx, dz), (dx + 1, dz), (dx, dz + 1), (dx + 1, dz + 1)]
                    .into_iter()
                    .map(|(x, z)| {
                        let p = pos.offset(x, 0, z);
                        (self.block(p), p)
                    })
                    .collect();
                if around.iter().all(|&(s, _)| self.block_id(s) == block) {
                    return Some((dx, dz, around));
                }
            }
        }
        None
    }

    /// `TreeGrower.hasFlowers`: a flower within 2 blocks across, 1 up/down.
    fn has_flowers(&self, pos: BlockPos) -> bool {
        let tag = self.lib.registries.block_tags.require("minecraft:flowers").expect("tag exists");
        for y in -1..=1 {
            for z in -2..=2 {
                for x in -2..=2 {
                    if self.lib.registries.block_in_tag(self.block(pos.offset(x, y, z)), tag) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `Feature.place(level, generator, level.random, pos)` for a named
    /// configured feature, drawing from the level random.
    pub(super) fn place_gameplay_feature(&mut self, name: &str, pos: BlockPos) -> bool {
        let Some(id) = self.lib.feature_by_name(name) else {
            self.unsupported.push(format!("feature {name}"));
            return false;
        };
        self.with_feature_random(|ctx, random| place_feature(ctx, random, id, pos))
    }

    /// `PlacedFeature.place(level, generator, level.random, pos)`.
    pub(super) fn place_gameplay_placed(&mut self, name: &str, pos: BlockPos) -> bool {
        let Some(id) = self.lib.placed_by_name(name) else {
            self.unsupported.push(format!("placed feature {name}"));
            return false;
        };
        self.with_feature_random(|ctx, random| minecraftoss_generator::feature::place_placed(ctx, random, id, pos))
    }

    /// Runs feature code on this level with the level random as its random.
    pub(super) fn with_feature_random(&mut self, place: impl FnOnce(&mut Ctx, &mut WorldgenRandom) -> bool) -> bool {
        let lib = self.lib;
        let AnyRandom::Legacy(legacy) = std::mem::replace(&mut self.random, AnyRandom::new(true, 0)) else {
            unreachable!("the level random is a legacy random");
        };
        let mut random = WorldgenRandom::from_legacy(legacy);
        let placed = {
            let mut ctx: Ctx = Ctx { lib, region: self };
            place(&mut ctx, &mut random)
        };
        // Anything that drew from the level random during placement drew
        // from the placeholder instead: vanilla shares one stream.
        if let AnyRandom::Legacy(placeholder) = &self.random {
            if placeholder.state() != minecraftoss_core::random::LegacyRandom::new(0).state() {
                self.unsupported.push("level random draws during feature placement".to_owned());
            }
        }
        self.random = AnyRandom::Legacy(random.into_legacy().expect("legacy source"));
        placed
    }
}
