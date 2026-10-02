//! End features: obsidian spikes, the spawn platform, gateways, small islands
//! and chorus plants.

use super::{bool_or, shuffle, Placeable};
use crate::feature::predicate::parse_vec3;
use crate::feature::{Ctx, Library};
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::{LegacyRandom, WorldgenRandom};
use minecraftoss_core::tags::TagId;
use minecraftoss_core::{BlockPos, BlockStateId};
use serde_json::Value;

pub fn parse(lib: &mut Library, kind: &str, json: &Value) -> Option<Result<Box<dyn Placeable>, String>> {
    Some(match kind {
        "end_spike" => EndSpikes::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        "end_platform" => EndPlatform::parse(lib).map(|f| Box::new(f) as Box<dyn Placeable>),
        "end_gateway" => EndGateway::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        "end_island" => EndIsland::parse(lib).map(|f| Box::new(f) as Box<dyn Placeable>),
        "chorus_plant" => ChorusPlant::parse(lib).map(|f| Box::new(f) as Box<dyn Placeable>),
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug)]
pub struct Spike {
    pub center_x: i32,
    pub center_z: i32,
    pub radius: i32,
    pub height: i32,
    pub guarded: bool,
}

/// `EndSpikeFeature.getSpikesForLevel`: ten pillars from the world seed.
pub fn spikes_for_seed(seed: i64) -> Vec<Spike> {
    let key = LegacyRandom::new(seed).next_i64() & 65535;
    let mut sizes: Vec<i32> = (0..10).collect();
    let mut random = LegacyRandom::new(key);
    for i in (2..=sizes.len()).rev() {
        let j = random.next_i32_bound(i as i32) as usize;
        sizes.swap(i - 1, j);
    }
    (0..10)
        .map(|i| {
            let angle = 2.0 * (-std::f64::consts::PI + std::f64::consts::PI / 10.0 * f64::from(i));
            let size = sizes[i as usize];
            Spike {
                center_x: (42.0 * angle.cos()).floor() as i32,
                center_z: (42.0 * angle.sin()).floor() as i32,
                radius: 2 + size / 3,
                height: 76 + size * 3,
                guarded: size == 1 || size == 2,
            }
        })
        .collect()
}

#[derive(Debug)]
struct EndSpikes {
    spikes: Vec<Spike>,
    obsidian: BlockStateId,
    iron_bars: BlockStateId,
    bedrock: BlockStateId,
    fire: BlockStateId,
}

impl EndSpikes {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let mut spikes = Vec::new();
        for s in json["spikes"].as_array().ok_or("end_spike lacks spikes")? {
            let get = |k: &str| s.get(k).and_then(Value::as_i64).map_or(0, |v| v as i32);
            spikes.push(Spike {
                center_x: get("centerX"),
                center_z: get("centerZ"),
                radius: get("radius"),
                height: get("height"),
                guarded: bool_or(s, "guarded", false),
            });
        }
        let b = |n: &str| lib.registries.blocks.parse_state(n);
        Ok(Self { spikes, obsidian: b("minecraft:obsidian")?, iron_bars: b("minecraft:iron_bars")?, bedrock: b("minecraft:bedrock")?, fire: b("minecraft:fire")? })
    }

    fn place_spike(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, spike: Spike) {
        let r = spike.radius;
        for z in spike.center_z - r..=spike.center_z + r {
            for y in ctx.min_y()..=spike.height + 10 {
                for x in spike.center_x - r..=spike.center_x + r {
                    let pos = BlockPos::new(x, y, z);
                    // distToLowCornerSqr(centerX, y, centerZ)
                    let (dx, dz) = (f64::from(x - spike.center_x), f64::from(z - spike.center_z));
                    if dx * dx + dz * dz <= f64::from(r * r + 1) && y < spike.height {
                        ctx.set_block_update(pos, self.obsidian);
                    } else if y > 65 {
                        ctx.set_block_update(pos, ctx.lib.blocks.air);
                    }
                }
            }
        }
        if spike.guarded {
            for dx in -2i32..=2 {
                for dz in -2i32..=2 {
                    for dy in 0..=3 {
                        let (side_x, side_z, top) = (dx.abs() == 2, dz.abs() == 2, dy == 3);
                        if !(side_x || side_z || top) {
                            continue;
                        }
                        let x_edge = dx == -2 || dx == 2 || top;
                        let z_edge = dz == -2 || dz == 2 || top;
                        let flag = |b: bool| if b { "true" } else { "false" };
                        let mut state = self.iron_bars;
                        state = ctx.with(state, "north", flag(x_edge && dz != -2));
                        state = ctx.with(state, "south", flag(x_edge && dz != 2));
                        state = ctx.with(state, "west", flag(z_edge && dx != -2));
                        state = ctx.with(state, "east", flag(z_edge && dx != 2));
                        ctx.set_block_update(BlockPos::new(spike.center_x + dx, spike.height + dy, spike.center_z + dz), state);
                    }
                }
            }
        }
        // The end crystal (`EndCrystal.snapTo` with a random yaw) sits on
        // bedrock with fire; generation's spikes have no beam target and
        // leave the crystal vulnerable.
        let yaw = random.next_f32() * 360.0;
        let crystal = BlockPos::new(spike.center_x, spike.height + 1, spike.center_z);
        let at = [f64::from(spike.center_x) + 0.5, f64::from(spike.height + 1), f64::from(spike.center_z) + 0.5];
        if let Some(entity) = crate::feature::entities::create(ctx, "minecraft:end_crystal", at, yaw, 0.0) {
            ctx.region.add_entity(entity);
        }
        ctx.set_block_update(crystal.below(), self.bedrock);
        ctx.set_block_update(crystal, self.fire);
    }
}

impl Placeable for EndSpikes {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let spikes = if self.spikes.is_empty() { spikes_for_seed(ctx.region.world_seed()) } else { self.spikes.clone() };
        for spike in spikes {
            if spike.center_x >> 4 == origin.x >> 4 && spike.center_z >> 4 == origin.z >> 4 {
                self.place_spike(ctx, random, spike);
            }
        }
        true
    }
}

#[derive(Debug)]
struct EndPlatform {
    obsidian: BlockStateId,
}

impl EndPlatform {
    fn parse(lib: &mut Library) -> Result<Self, String> {
        Ok(Self { obsidian: lib.registries.blocks.parse_state("minecraft:obsidian")? })
    }
}

impl Placeable for EndPlatform {
    fn place(&self, ctx: &mut Ctx, _random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        for dz in -2..=2 {
            for dx in -2..=2 {
                for dy in -1..3 {
                    let pos = origin.offset(dx, dy, dz);
                    let target = if dy == -1 { self.obsidian } else { ctx.lib.blocks.air };
                    let blocks = &ctx.registries().blocks;
                    if blocks.block_of(ctx.block(pos)) != blocks.block_of(target) {
                        ctx.set_block_update(pos, target);
                    }
                }
            }
        }
        true
    }
}

#[derive(Debug)]
struct EndGateway {
    exit: Option<BlockPos>,
    exact: bool,
    gateway: BlockStateId,
    bedrock: BlockStateId,
}

impl EndGateway {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        Ok(Self {
            exit: json.get("exit").map(parse_vec3).transpose()?,
            exact: json.get("exact").and_then(Value::as_bool).unwrap_or(false),
            gateway: lib.registries.blocks.parse_state("minecraft:end_gateway")?,
            bedrock: lib.registries.blocks.parse_state("minecraft:bedrock")?,
        })
    }
}

impl Placeable for EndGateway {
    fn place(&self, ctx: &mut Ctx, _random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let air = ctx.lib.blocks.air;
        for z in -1..=1 {
            for y in -2..=2 {
                for x in -1..=1 {
                    let pos = origin.offset(x, y, z);
                    let (same_x, same_y, same_z) = (x == 0, y == 0, z == 0);
                    let end = y.abs() == 2;
                    let state = if same_x && same_y && same_z {
                        self.gateway
                    } else if same_y {
                        air
                    } else if end && same_x && same_z || (same_x || same_z) && !end {
                        self.bedrock
                    } else {
                        air
                    };
                    ctx.set_block_update(pos, state);
                    if state == self.gateway {
                        if let Some(exit) = self.exit {
                            ctx.region.set_gateway_exit(pos.x, pos.y, pos.z, (exit.x, exit.y, exit.z), self.exact);
                        }
                    }
                }
            }
        }
        true
    }
}

#[derive(Debug)]
struct EndIsland {
    end_stone: BlockStateId,
}

impl EndIsland {
    fn parse(lib: &mut Library) -> Result<Self, String> {
        Ok(Self { end_stone: lib.registries.blocks.parse_state("minecraft:end_stone")? })
    }
}

impl Placeable for EndIsland {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let mut size = random.next_i32_bound(3) as f32 + 4.0;
        let mut y = 0;
        while size > 0.5 {
            let (lo, hi) = (crate::mth::floor_f32(-size), f64::from(size).ceil() as i32);
            for x in lo..=hi {
                for z in lo..=hi {
                    if ((x * x + z * z) as f32) <= (size + 1.0) * (size + 1.0) {
                        ctx.set_block_update(origin.offset(x, y, z), self.end_stone);
                    }
                }
            }
            size -= random.next_i32_bound(2) as f32 + 0.5;
            y -= 1;
        }
        true
    }
}

#[derive(Debug)]
struct ChorusPlant {
    plant: BlockStateId,
    flower: BlockStateId,
    supports: TagId,
}

impl ChorusPlant {
    fn parse(lib: &mut Library) -> Result<Self, String> {
        Ok(Self {
            plant: lib.registries.blocks.parse_state("minecraft:chorus_plant")?,
            flower: lib.registries.blocks.parse_state("minecraft:chorus_flower[age=5]")?,
            supports: lib.registries.block_tags.require("minecraft:supports_chorus_plant")?,
        })
    }

    /// `ChorusPlantBlock.getStateWithConnections`.
    fn connected(&self, ctx: &Ctx, pos: BlockPos) -> BlockStateId {
        let is_part = |s: BlockStateId| ctx.is(s, "minecraft:chorus_plant") || ctx.is(s, "minecraft:chorus_flower");
        let flag = |b: bool| if b { "true" } else { "false" };
        let down = ctx.block(pos.below());
        let mut state = ctx.with(self.plant, "down", flag(is_part(down) || ctx.in_tag(down, self.supports)));
        state = ctx.with(state, "up", flag(is_part(ctx.block(pos.above()))));
        for d in Direction::HORIZONTAL {
            state = ctx.with(state, d.name(), flag(is_part(ctx.block(pos.relative(d, 1)))));
        }
        state
    }

    fn neighbours_empty(ctx: &Ctx, pos: BlockPos, ignore: Option<Direction>) -> bool {
        Direction::HORIZONTAL.iter().all(|&d| Some(d) == ignore || ctx.is_empty_block(pos.relative(d, 1)))
    }

    fn grow(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, current: BlockPos, start: BlockPos, spread: i32, depth: i32) {
        let mut height = random.next_i32_bound(4) + 1;
        if depth == 0 {
            height += 1;
        }
        for i in 0..height {
            let target = current.offset(0, i + 1, 0);
            if !Self::neighbours_empty(ctx, target, None) {
                return;
            }
            let state = self.connected(ctx, target);
            ctx.set_block(target, state);
            let below = self.connected(ctx, target.below());
            ctx.set_block(target.below(), below);
        }
        let mut placed_stem = false;
        if depth < 4 {
            let mut stems = random.next_i32_bound(4);
            if depth == 0 {
                stems += 1;
            }
            for _ in 0..stems {
                let direction = Direction::HORIZONTAL[random.next_i32_bound(4) as usize];
                let target = current.offset(0, height, 0).relative(direction, 1);
                if (target.x - start.x).abs() < spread
                    && (target.z - start.z).abs() < spread
                    && ctx.is_empty_block(target)
                    && ctx.is_empty_block(target.below())
                    && Self::neighbours_empty(ctx, target, Some(direction.opposite()))
                {
                    placed_stem = true;
                    let state = self.connected(ctx, target);
                    ctx.set_block(target, state);
                    let back = target.relative(direction.opposite(), 1);
                    let back_state = self.connected(ctx, back);
                    ctx.set_block(back, back_state);
                    self.grow(ctx, random, target, start, spread, depth + 1);
                }
            }
        }
        if !placed_stem {
            ctx.set_block(current.offset(0, height, 0), self.flower);
        }
    }
}

impl Placeable for ChorusPlant {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !ctx.is_empty_block(origin) || !ctx.in_tag(ctx.block(origin.below()), self.supports) {
            return false;
        }
        let state = self.connected(ctx, origin);
        ctx.set_block(origin, state);
        self.grow(ctx, random, origin, origin, 8, 0);
        true
    }
}

#[allow(dead_code)]
fn unused(list: &mut [i32], random: &mut WorldgenRandom) {
    shuffle(list, random);
}
