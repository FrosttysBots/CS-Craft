//! Natural mob spawning: the SPAWN chunk step's creatures
//! (`NaturalSpawner.spawnMobsForChunkGeneration`).
//!
//! Source-informed from the pinned 26.3 `NoiseBasedChunkGenerator.spawnOriginalMobs`,
//! `NaturalSpawner`, `SpawnPlacements`, `SpawnPlacementTypes`, `EntityType`,
//! `Animal`, `AgeableMob`, `Mob.finalizeSpawn`, the farm animals' and horses'
//! `finalizeSpawn`, `VariantUtils`/`PriorityProvider` and `SheepColorSpawnRules`.
//!
//! The spawner reads the level through [`SpawnLevel`], so the same rules
//! serve a generation region and the server level ([`spawning`]: the
//! per-tick spawner around players).

pub mod spawning;
pub mod tick;

use minecraftoss_core::block::flags;
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::entity_data::{finalize_mob, place};
use minecraftoss_core::nbt::Tag;
use minecraftoss_core::random::{LegacyRandom, RandomSource, WorldgenRandom};
use minecraftoss_core::tags::TagId;
use minecraftoss_core::{BiomeId, BlockPos, BlockStateId, ChunkPos, Registries};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// What natural spawning reads and writes.
pub trait SpawnLevel {
    fn block(&self, pos: BlockPos) -> BlockStateId;
    /// `getHeight(type, x, z)`: one above the highest matching block.
    fn height(&self, kind: HeightmapKind, x: i32, z: i32) -> i32;
    /// `getBiome`: the zoomed biome at a block.
    fn biome(&self, pos: BlockPos) -> BiomeId;
    /// `getNoiseBiome` at quart coordinates.
    fn noise_biome(&self, qx: i32, qy: i32, qz: i32) -> BiomeId;
    /// `getRawBrightness(pos, darkening)`.
    fn raw_brightness(&mut self, pos: BlockPos, darkening: i32) -> i32;
    /// `getBrightness(LightLayer.SKY, pos)`.
    fn sky_brightness(&mut self, pos: BlockPos) -> i32;
    /// `getBrightness(LightLayer.BLOCK, pos)`.
    fn block_brightness(&mut self, pos: BlockPos) -> i32;
    /// `getSkyDarken`.
    fn sky_darken(&self) -> i32 {
        0
    }
    /// `getSeaLevel`.
    fn sea_level(&self) -> i32 {
        63
    }
    fn min_y(&self) -> i32;
    /// The level random (`getRandom`).
    fn random(&mut self) -> &mut dyn RandomSource;
    fn next_uuid(&mut self) -> [i32; 4];
    /// `addFreshEntityWithPassengers`.
    fn add_entity(&mut self, entity: Tag);
    /// A FULL chunk's `inhabitedTime`; none when it is not loaded.
    fn inhabited_time(&self, _chunk: ChunkPos) -> Option<i64> {
        None
    }
    /// `incrementInhabitedTime`.
    fn add_inhabited_time(&mut self, _chunk: ChunkPos) {}
    /// Types the rules here do not cover yet.
    fn note_unsupported(&mut self, _what: &str) {}
}

/// `SpawnerData`: a type, its weight and its group size.
#[derive(Clone, Debug)]
struct SpawnerData {
    kind: String,
    weight: i32,
    /// `count`: a constant (`min == max`, no draw) or uniform range.
    min: i32,
    max: i32,
    constant: bool,
}

/// One biome's creature list (`WeightedList<SpawnerData>`).
#[derive(Clone, Debug, Default)]
struct Creatures {
    entries: Vec<SpawnerData>,
    total: i32,
}

impl Creatures {
    /// `WeightedList.getRandom`.
    fn pick(&self, random: &mut dyn RandomSource) -> Option<&SpawnerData> {
        if self.total == 0 {
            return None;
        }
        let mut selection = random.next_i32_bound(self.total);
        for entry in &self.entries {
            if selection < entry.weight {
                return Some(entry);
            }
            selection -= entry.weight;
        }
        None
    }
}

/// A variant registry's spawn selectors (`PriorityProvider`), in registry
/// (identifier) order.
#[derive(Debug, Default)]
struct VariantRegistry {
    variants: Vec<(String, Vec<(i32, Condition)>)>,
}

#[derive(Debug)]
enum Condition {
    Always,
    Biomes(Vec<BiomeId>),
    BiomeTag(TagId),
    /// Structure and moon brightness conditions (cats, wolves): not needed
    /// by the creatures spawning covers yet.
    Other,
}

impl VariantRegistry {
    fn load(registries: &Registries, kind: &str) -> Result<Self, String> {
        let mut out = Self::default();
        let pack = &registries.datapack;
        for id in pack.list(kind)? {
            let json = pack.read_json(kind, &id)?;
            let mut selectors = Vec::new();
            for s in json["spawn_conditions"].as_array().into_iter().flatten() {
                let priority = s["priority"].as_i64().unwrap_or(0) as i32;
                let condition = match s["condition"]["type"].as_str() {
                    None => Condition::Always,
                    Some("minecraft:biome") => match &s["condition"]["biomes"] {
                        Value::String(tag) if tag.starts_with('#') => Condition::BiomeTag(
                            registries.biome_tags.id(&tag[1..]).ok_or_else(|| format!("unknown biome tag {tag}"))?,
                        ),
                        Value::String(one) => Condition::Biomes(registries.biomes.id(one).into_iter().collect()),
                        Value::Array(list) => Condition::Biomes(list.iter().filter_map(|b: &Value| b.as_str().and_then(|b| registries.biomes.id(b))).collect()),
                        other => return Err(format!("{kind} biome condition {other}")),
                    },
                    Some(_) => Condition::Other,
                };
                selectors.push((priority, condition));
            }
            out.variants.push((id.as_str().to_owned(), selectors));
        }
        Ok(out)
    }

    /// `PriorityProvider.pick`: the variants of the highest priority whose
    /// condition holds (a stable sort, registry order kept), then
    /// `Util.getRandomSafe`.
    fn pick(&self, registries: &Registries, biome: BiomeId, random: &mut dyn RandomSource, unsupported: &mut bool) -> Option<&str> {
        let mut unpacked: Vec<(&str, i32, &Condition)> =
            self.variants.iter().flat_map(|(id, selectors)| selectors.iter().map(move |(p, c)| (id.as_str(), *p, c))).collect();
        unpacked.sort_by(|a, b| b.1.cmp(&a.1));
        let mut highest = i32::MIN;
        let mut selected = Vec::new();
        for (id, priority, condition) in unpacked {
            if priority < highest {
                continue;
            }
            let holds = match condition {
                Condition::Always => true,
                Condition::Biomes(list) => list.contains(&biome),
                Condition::BiomeTag(tag) => registries.biome_in_tag(biome, *tag),
                Condition::Other => {
                    *unsupported = true;
                    false
                }
            };
            if holds {
                highest = priority;
                selected.push(id);
            }
        }
        if selected.is_empty() {
            return None;
        }
        Some(selected[random.next_i32_bound(selected.len() as i32) as usize])
    }
}

/// `SpawnPlacements` data of a creature type, with its `EntityType` size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placement {
    OnGround,
    InWater,
    InLava,
    NoRestrictions,
}

#[derive(Clone, Copy, Debug)]
struct TypeInfo {
    width: f32,
    height: f32,
    placement: Placement,
    heightmap: HeightmapKind,
    fire_immune: bool,
    /// The type's `SpawnPlacements` predicate family.
    rules: Rules,
    /// Its `PathfinderMob.getWalkTargetValue` family.
    walk: Walk,
    /// `Mob.removeWhenFarAway` for a fresh natural spawn.
    remove_far: bool,
    /// `Mob.getMaxSpawnClusterSize`.
    cluster: i32,
}

/// Which `SpawnPlacements` predicate a type registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rules {
    /// `Animal.checkAnimalSpawnRules` (grass-like block below, light > 8).
    Animal,
    /// The creatures that need their own block tag below and light above
    /// 8 (`Wolf.checkWolfSpawnRules`, `Fox.checkFoxSpawnRules`,
    /// `Rabbit.checkRabbitSpawnRules`, `Armadillo.checkArmadilloSpawnRules`).
    SpawnableOn(&'static str),
    /// `Monster.checkMonsterSpawnRules` (dark enough, a valid block below).
    Monster,
    /// `Monster.checkSurfaceMonstersSpawnRules`: the monster rules under
    /// open sky (husks, parched).
    SurfaceMonster,
    /// `Stray.checkStraySpawnRules`: the monster rules with open sky above
    /// any powder snow.
    Stray,
    /// `Slime.checkSlimeSpawnRules` (swamp surfaces, slime chunks).
    Slime,
    /// `Bat.checkBatSpawnRules` (below the surface, in the dark).
    Bat,
    /// `GlowSquid.checkGlowSquidSpawnRules` (deep, dark water).
    GlowSquid,
    /// Rules not ported yet: the type is not spawned.
    Other,
}

impl Rules {
    /// Creatures finalize as animals (`finalize_spawn`), not as monsters.
    fn creature(self) -> bool {
        matches!(self, Self::Animal | Self::SpawnableOn(_))
    }
}

/// Which `getWalkTargetValue` a type has (a spawn needs at least 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Walk {
    /// `Animal`: 10 on grass blocks, else the light's pathfinding cost.
    Animal,
    /// `Monster`: the negated pathfinding cost (darkness).
    Monster,
    /// `PathfinderMob`'s 0 (cube mobs).
    Neutral,
}

/// The creature types spawn tables name.
fn type_info(kind: &str) -> Option<TypeInfo> {
    use HeightmapKind::{MotionBlocking, MotionBlockingNoLeaves};
    let (width, height, placement, heightmap, fire_immune, animal_rules) = match kind.trim_start_matches("minecraft:") {
        "pig" => (0.9, 0.9, Placement::OnGround, MotionBlockingNoLeaves, false, true),
        "cow" => (0.9, 1.4, Placement::OnGround, MotionBlockingNoLeaves, false, true),
        "sheep" => (0.9, 1.3, Placement::OnGround, MotionBlockingNoLeaves, false, true),
        "chicken" => (0.4, 0.7, Placement::OnGround, MotionBlockingNoLeaves, false, true),
        "horse" => (1.396_484_4, 1.6, Placement::OnGround, MotionBlockingNoLeaves, false, true),
        "donkey" => (1.396_484_4, 1.5, Placement::OnGround, MotionBlockingNoLeaves, false, true),
        "llama" => (0.9, 1.87, Placement::OnGround, MotionBlockingNoLeaves, false, true),
        "panda" => (1.3, 1.25, Placement::NoRestrictions, MotionBlockingNoLeaves, false, true),
        "rabbit" => (0.49, 0.6, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "wolf" => (0.6, 0.85, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "fox" => (0.6, 0.7, Placement::NoRestrictions, MotionBlockingNoLeaves, false, false),
        "goat" => (0.9, 1.3, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "frog" => (0.5, 0.5, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "parrot" => (0.5, 0.9, Placement::OnGround, MotionBlocking, false, false),
        "polar_bear" => (1.4, 1.4, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "turtle" => (1.2, 0.4, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "camel" => (1.7, 2.375, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "armadillo" => (0.7, 0.65, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "mooshroom" => (0.9, 1.4, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "strider" => (0.9, 1.7, Placement::InLava, MotionBlockingNoLeaves, true, false),
        "axolotl" => (0.75, 0.42, Placement::InWater, MotionBlockingNoLeaves, false, false),
        // Monsters (`EntityTypes` sizes; `Monster.checkMonsterSpawnRules`).
        "zombie" | "zombie_villager" | "witch" | "husk" => (0.6, 1.95, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "skeleton" | "stray" | "bogged" | "parched" => (0.6, 1.99, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        // A husk's mount (its room is tried; it never spawns on its own).
        "camel_husk" => (1.7, 2.375, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "creeper" => (0.6, 1.7, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "spider" => (1.4, 0.9, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "enderman" => (0.6, 2.9, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "slime" => (0.52, 0.52, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "zombie_horse" => (1.396_484_4, 1.6, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "bat" => (0.5, 0.9, Placement::OnGround, MotionBlockingNoLeaves, false, false),
        "glow_squid" => (0.8, 0.8, Placement::InWater, MotionBlockingNoLeaves, false, false),
        _ => return None,
    };
    let short = kind.trim_start_matches("minecraft:");
    let monster = matches!(short, "zombie" | "zombie_villager" | "witch" | "skeleton" | "creeper" | "spider" | "enderman" | "husk" | "stray" | "bogged" | "parched");
    let rules = match short {
        _ if animal_rules => Rules::Animal,
        "wolf" => Rules::SpawnableOn("minecraft:wolves_spawnable_on"),
        "fox" => Rules::SpawnableOn("minecraft:foxes_spawnable_on"),
        "rabbit" => Rules::SpawnableOn("minecraft:rabbits_spawnable_on"),
        "armadillo" => Rules::SpawnableOn("minecraft:armadillo_spawnable_on"),
        "zombie_horse" => Rules::Monster,
        "husk" | "parched" => Rules::SurfaceMonster,
        "stray" => Rules::Stray,
        "camel_husk" => Rules::Other,
        "slime" => Rules::Slime,
        "bat" => Rules::Bat,
        "glow_squid" => Rules::GlowSquid,
        _ if monster => Rules::Monster,
        _ => Rules::Other,
    };
    let walk = match short {
        _ if monster => Walk::Monster,
        // Cube mobs and squid keep `PathfinderMob`'s 0; bats are no
        // pathfinders (`Mob.checkSpawnRules`).
        "slime" | "bat" | "glow_squid" => Walk::Neutral,
        _ => Walk::Animal,
    };
    // Animals stay (`Animal.removeWhenFarAway`) except the overrides.
    let remove_far = monster || matches!(short, "slime" | "zombie_horse" | "axolotl" | "bat" | "glow_squid");
    let cluster = match short {
        "horse" | "donkey" | "mule" | "llama" | "camel" | "zombie_horse" => 6,
        "wolf" => 8,
        _ => 4,
    };
    Some(TypeInfo { width, height, placement, heightmap, fire_immune, rules, walk, remove_far, cluster })
}

/// Per-biome creature spawning and the data finalizing it reads.
pub struct CreatureSpawns {
    registries: Arc<Registries>,
    creatures: Vec<Creatures>,
    /// Each biome's spawn lists by `MobCategory` (spawning categories).
    by_category: Vec<[Creatures; spawning::MobCategory::COUNT]>,
    /// Each biome's `spawn_costs`: energy budget and charge by type.
    costs: Vec<std::collections::HashMap<String, (f64, f64)>>,
    reduced_water_ambient: Option<TagId>,
    /// `DimensionType.monsterSpawnBlockLightLimit` and the upper bound of
    /// the uniform `monsterSpawnLightTest` (lower bound 0).
    monster_block_light_limit: i32,
    monster_light_test_max: i32,
    /// `#minecraft:allows_surface_slime_spawns`.
    surface_slimes: Option<TagId>,
    /// `#minecraft:bats_spawnable_on`.
    bats_spawnable_on: Option<TagId>,
    probability: Vec<f32>,
    pig_variants: VariantRegistry,
    cow_variants: VariantRegistry,
    chicken_variants: VariantRegistry,
    pig_sounds: Vec<String>,
    cow_sounds: Vec<String>,
    chicken_sounds: Vec<String>,
    wolf_variants: VariantRegistry,
    wolf_sounds: Vec<String>,
    /// `#spawns_snow_foxes`, `#spawns_white_rabbits`, `#spawns_gold_rabbits`.
    snow_foxes: TagId,
    white_rabbits: TagId,
    gold_rabbits: TagId,
    warm: TagId,
    cold: TagId,
    animals_spawnable_on: TagId,
    prevent_mob_spawning_inside: TagId,
    fire: TagId,
    leaves: TagId,
    grass_block: minecraftoss_core::BlockId,
    /// `DimensionType.ambientLight` and `hasCeiling`.
    ambient_light: f32,
    has_ceiling: bool,
    /// `NoiseGeneratorSettings.disableMobGeneration`.
    disabled: bool,
}

/// An attribute value as a biome sets it: a plain value, or a modifier
/// with an argument (`overlay` and `override` both replace an unset base).
fn attribute_argument(value: &Value) -> &Value {
    match value.get("modifier") {
        Some(_) => &value["argument"],
        None => value,
    }
}

fn int_provider(value: &Value) -> Result<(i32, i32, bool), String> {
    match value {
        Value::Number(n) => {
            let v = n.as_i64().ok_or("count is not an integer")? as i32;
            Ok((v, v, true))
        }
        _ => match value["type"].as_str() {
            Some("minecraft:uniform") => {
                let min = value["min_inclusive"].as_i64().ok_or("uniform lacks min")? as i32;
                let max = value["max_inclusive"].as_i64().ok_or("uniform lacks max")? as i32;
                Ok((min, max, false))
            }
            Some("minecraft:constant") => {
                let v = value["value"].as_i64().ok_or("constant lacks value")? as i32;
                Ok((v, v, true))
            }
            other => Err(format!("spawn count provider {other:?}")),
        },
    }
}

impl CreatureSpawns {
    pub fn load(registries_arc: Arc<Registries>, dimension_type: &str, disabled: bool) -> Result<Self, String> {
        let registries = &*registries_arc;
        let pack = &registries.datapack;
        let mut creatures = Vec::new();
        let mut by_category = Vec::new();
        let mut costs = Vec::new();
        let mut probability = Vec::new();
        for (_, info) in registries.biomes.iter() {
            let json = pack.read_json("worldgen/biome", &info.name)?;
            let attributes = &json["attributes"];
            let mut lists: [Creatures; spawning::MobCategory::COUNT] = Default::default();
            let mut biome_costs = std::collections::HashMap::new();
            if let Some(spawns) = attributes.get("minecraft:gameplay/natural_mob_spawns") {
                for (kind, cost) in attribute_argument(spawns)["spawn_costs"].as_object().into_iter().flatten() {
                    let budget = cost["energy_budget"].as_f64().ok_or("spawn cost lacks a budget")?;
                    let charge = cost["charge"].as_f64().ok_or("spawn cost lacks a charge")?;
                    biome_costs.insert(kind.clone(), (budget, charge));
                }
                for (i, category) in spawning::MobCategory::SPAWNING.iter().enumerate() {
                    for entry in attribute_argument(spawns)["spawns_by_category"][category.name()].as_array().into_iter().flatten() {
                        let kind = entry["type"].as_str().ok_or("spawner lacks a type")?.to_owned();
                        let weight = entry["weight"].as_i64().ok_or("spawner lacks a weight")? as i32;
                        let (min, max, constant) = int_provider(&entry["count"])?;
                        lists[i].total += weight;
                        lists[i].entries.push(SpawnerData { kind, weight, min, max, constant });
                    }
                }
            }
            creatures.push(lists[spawning::MobCategory::Creature as usize].clone());
            by_category.push(lists);
            costs.push(biome_costs);
            let p = attributes.get("minecraft:gameplay/creature_world_gen_spawn_probability").map(attribute_argument).and_then(Value::as_f64);
            probability.push(p.map_or(0.1, |p| p as f32));
        }
        let sounds = |kind: &str| -> Result<Vec<String>, String> { Ok(pack.list(kind)?.into_iter().map(|id| id.as_str().to_owned()).collect()) };
        let tag = |tags: &minecraftoss_core::tags::Tags, name: &str| tags.id(name).ok_or_else(|| format!("unknown tag {name}"));
        let dimension = pack.read_json("dimension_type", &minecraftoss_core::Identifier::parse(dimension_type)?)?;
        // `DimensionType.monsterSpawnLightTest`: a constant or uniform int.
        let light_test = &dimension["monster_spawn_light_level"];
        let monster_light_test_max = light_test.as_i64().map(|v| v as i32).or_else(|| light_test["max_inclusive"].as_i64().map(|v| v as i32)).unwrap_or(7);
        Ok(Self {
            registries: registries_arc.clone(),
            creatures,
            by_category,
            costs,
            reduced_water_ambient: registries.biome_tags.id("minecraft:reduced_water_ambient_spawns"),
            monster_block_light_limit: dimension["monster_spawn_block_light_limit"].as_i64().unwrap_or(0) as i32,
            monster_light_test_max,
            surface_slimes: registries.biome_tags.id("minecraft:allows_surface_slime_spawns"),
            bats_spawnable_on: registries.block_tags.id("minecraft:bats_spawnable_on"),
            probability,
            pig_variants: VariantRegistry::load(registries, "pig_variant")?,
            cow_variants: VariantRegistry::load(registries, "cow_variant")?,
            chicken_variants: VariantRegistry::load(registries, "chicken_variant")?,
            pig_sounds: sounds("pig_sound_variant")?,
            cow_sounds: sounds("cow_sound_variant")?,
            chicken_sounds: sounds("chicken_sound_variant")?,
            wolf_variants: VariantRegistry::load(registries, "wolf_variant")?,
            wolf_sounds: sounds("wolf_sound_variant")?,
            snow_foxes: tag(&registries.biome_tags, "minecraft:spawns_snow_foxes")?,
            white_rabbits: tag(&registries.biome_tags, "minecraft:spawns_white_rabbits")?,
            gold_rabbits: tag(&registries.biome_tags, "minecraft:spawns_gold_rabbits")?,
            warm: tag(&registries.biome_tags, "minecraft:spawns_warm_variant_farm_animals")?,
            cold: tag(&registries.biome_tags, "minecraft:spawns_cold_variant_farm_animals")?,
            animals_spawnable_on: tag(&registries.block_tags, "minecraft:animals_spawnable_on")?,
            prevent_mob_spawning_inside: tag(&registries.block_tags, "minecraft:prevent_mob_spawning_inside")?,
            fire: tag(&registries.block_tags, "minecraft:fire")?,
            leaves: tag(&registries.block_tags, "minecraft:leaves")?,
            grass_block: registries.blocks.block_by_name("minecraft:grass_block").ok_or("no grass block")?,
            ambient_light: dimension["ambient_light"].as_f64().unwrap_or(0.0) as f32,
            has_ceiling: dimension["has_ceiling"].as_bool().unwrap_or(false),
            disabled,
        })
    }

    /// `NoiseBasedChunkGenerator.spawnOriginalMobs` for a region centred
    /// on `center`: a decoration-seeded legacy random drives
    /// `spawnMobsForChunkGeneration`.
    pub fn spawn_original_mobs(&self, level: &mut dyn SpawnLevel, center: ChunkPos, world_seed: i64, max_y: i32) {
        if self.disabled {
            return;
        }
        let mut random = WorldgenRandom::from_legacy(LegacyRandom::new(0));
        random.decoration_seed(world_seed, center.min_block_x(), center.min_block_z());
        let source = BlockPos::new(center.min_block_x(), max_y, center.min_block_z());
        self.spawn_for_chunk_generation(level, source, center, &mut random);
    }

    /// `NaturalSpawner.spawnMobsForChunkGeneration`.
    fn spawn_for_chunk_generation(&self, level: &mut dyn SpawnLevel, source: BlockPos, chunk: ChunkPos, random: &mut dyn RandomSource) {
        // `NATURAL_MOB_SPAWNS` reads the zoomed biome; the probability the
        // noise biome, both at the source block's centre.
        let biome = level.biome(source);
        let mobs = &self.creatures[usize::from(biome.0)];
        if mobs.entries.is_empty() {
            return;
        }
        let noise = level.noise_biome(source.x >> 2, source.y >> 2, source.z >> 2);
        let probability = self.probability[usize::from(noise.0)];
        let (xo, zo) = (chunk.min_block_x(), chunk.min_block_z());
        while random.next_f32() < probability {
            let Some(data) = mobs.pick(random) else { continue };
            let count = if data.constant { data.min } else { random.next_i32_bound(data.max - data.min + 1) + data.min };
            let Some(info) = type_info(&data.kind) else {
                level.note_unsupported(&format!("spawning {}", data.kind));
                return;
            };
            if !info.rules.creature() {
                level.note_unsupported(&format!("spawn rules of {}", data.kind));
            }
            let mut group = Group::None;
            let mut x = xo + random.next_i32_bound(16);
            let mut z = zo + random.next_i32_bound(16);
            let (start_x, start_z) = (x, z);
            for _ in 0..count {
                let mut success = false;
                let mut attempts = 0;
                while !success && attempts < 4 {
                    attempts += 1;
                    let pos = self.top_non_colliding_pos(level, &info, x, z);
                    if self.is_spawn_position_ok(level, &data.kind, &info, pos) {
                        let width = f64::from(info.width);
                        let fx = f64::from(x).clamp(f64::from(xo) + width, f64::from(xo) + 16.0 - width);
                        let fz = f64::from(z).clamp(f64::from(zo) + width, f64::from(zo) + 16.0 - width);
                        let y = f64::from(pos.y);
                        let at = BlockPos::new(fx.floor() as i32, pos.y, fz.floor() as i32);
                        if !self.no_collision(level, spawn_aabb(&info, fx, y, fz)) || !self.check_spawn_rules(level, &info, at) {
                            continue;
                        }
                        let Some(mut tag) = self.registries.entities.as_ref().and_then(|c| c.default_tag(&data.kind)) else {
                            level.note_unsupported(&format!("creating {}", data.kind));
                            continue;
                        };
                        let y_rot = random.next_f32() * 360.0;
                        let uuid = level.next_uuid();
                        place(&mut tag, [fx, y, fz], y_rot, 0.0, uuid);
                        if self.mob_check_spawn_rules(level, &info, at) && self.check_spawn_obstruction(level, &info, fx, y, fz) {
                            self.finalize_spawn(level, &data.kind, &mut tag, at, &mut group);
                            level.add_entity(tag);
                            success = true;
                        }
                    }
                    x += random.next_i32_bound(5) - random.next_i32_bound(5);
                    z += random.next_i32_bound(5) - random.next_i32_bound(5);
                    while x < xo || x >= xo + 16 || z < zo || z >= zo + 16 {
                        x = start_x + random.next_i32_bound(5) - random.next_i32_bound(5);
                        z = start_z + random.next_i32_bound(5) - random.next_i32_bound(5);
                    }
                }
            }
        }
    }

    /// `NaturalSpawner.getTopNonCollidingPos`.
    fn top_non_colliding_pos(&self, level: &mut dyn SpawnLevel, info: &TypeInfo, x: i32, z: i32) -> BlockPos {
        let mut pos = BlockPos::new(x, level.height(info.heightmap, x, z), z);
        if self.has_ceiling {
            let registries = &*self.registries;
            loop {
                pos = pos.below();
                if registries.blocks.is_air(level.block(pos)) {
                    break;
                }
            }
            loop {
                pos = pos.below();
                if !(registries.blocks.is_air(level.block(pos)) && pos.y > level.min_y()) {
                    break;
                }
            }
        }
        match info.placement {
            // `ON_GROUND.adjustSpawnPosition`: into a land-pathfindable block below.
            Placement::OnGround => {
                if self.registries.blocks.is(level.block(pos.below()), flags::PATHFINDABLE_LAND) {
                    pos.below()
                } else {
                    pos
                }
            }
            _ => pos,
        }
    }

    /// `SpawnPlacements.isSpawnPositionOk` (the world border is far away).
    fn is_spawn_position_ok(&self, level: &mut dyn SpawnLevel, kind: &str, info: &TypeInfo, pos: BlockPos) -> bool {
        let registries = &*self.registries;
        let fluid = |state: BlockStateId| registries.blocks.state(state).fluid;
        match info.placement {
            Placement::NoRestrictions => true,
            Placement::InLava => fluid(level.block(pos)).is_some_and(|f| f.kind == minecraftoss_core::block::FluidKind::Lava),
            Placement::InWater => {
                fluid(level.block(pos)).is_some_and(|f| f.kind == minecraftoss_core::block::FluidKind::Water)
                    && !registries.blocks.is(level.block(pos.above()), flags::REDSTONE_CONDUCTOR)
            }
            Placement::OnGround => {
                let below = level.block(pos.below());
                self.is_valid_spawn(registries, below, kind, info)
                    && self.is_valid_empty_spawn_block(registries, level.block(pos), kind, info)
                    && self.is_valid_empty_spawn_block(registries, level.block(pos.above()), kind, info)
            }
        }
    }

    /// `BlockState.isValidSpawn` for a type: the plain animal answer, with
    /// the block predicates that name types (leaves for ocelots and
    /// parrots, ice for polar bears, magma for fire-immune mobs).
    fn is_valid_spawn(&self, registries: &Registries, state: BlockStateId, kind: &str, info: &TypeInfo) -> bool {
        let name = registries.blocks.block(registries.blocks.block_of(state)).name.as_str();
        if registries.block_in_tag(state, self.leaves) && registries.blocks.block(registries.blocks.block_of(state)).is_a("LeavesBlock") {
            return matches!(kind, "minecraft:ocelot" | "minecraft:parrot");
        }
        match name {
            "minecraft:ice" | "minecraft:frosted_ice" => kind == "minecraft:polar_bear",
            "minecraft:magma_block" => info.fire_immune,
            _ => registries.blocks.is(state, flags::VALID_SPAWN_ANIMAL),
        }
    }

    /// `NaturalSpawner.isValidEmptySpawnBlock`.
    fn is_valid_empty_spawn_block(&self, registries: &Registries, state: BlockStateId, kind: &str, info: &TypeInfo) -> bool {
        let blocks = &registries.blocks;
        let s = blocks.state(state);
        !s.collision_full_block
            && !s.has(flags::SIGNAL_SOURCE)
            && s.fluid.is_none()
            && !registries.block_in_tag(state, self.prevent_mob_spawning_inside)
            && !self.is_block_dangerous(registries, state, kind, info)
    }

    /// `EntityType.isBlockDangerous`.
    fn is_block_dangerous(&self, registries: &Registries, state: BlockStateId, kind: &str, info: &TypeInfo) -> bool {
        let blocks = &registries.blocks;
        let name = blocks.block(blocks.block_of(state)).name.as_str();
        // `immuneTo`: foxes and sweet berry bushes, polar bears and powder snow.
        let immune = match kind {
            "minecraft:fox" => name == "minecraft:sweet_berry_bush",
            "minecraft:polar_bear" => name == "minecraft:powder_snow",
            _ => false,
        };
        if immune {
            return false;
        }
        let lit_campfire = blocks.block(blocks.block_of(state)).is_a("CampfireBlock") && blocks.property(state, "lit") == Some("true");
        let burning = registries.block_in_tag(state, self.fire)
            || matches!(name, "minecraft:lava" | "minecraft:magma_block" | "minecraft:lava_cauldron")
            || lit_campfire;
        (!info.fire_immune && burning) || matches!(name, "minecraft:wither_rose" | "minecraft:sweet_berry_bush" | "minecraft:cactus" | "minecraft:powder_snow")
    }

    /// `SpawnPlacements.checkSpawnRules` for `CHUNK_GENERATION`.
    fn check_spawn_rules(&self, level: &mut dyn SpawnLevel, info: &TypeInfo, pos: BlockPos) -> bool {
        // `Animal.checkAnimalSpawnRules` and the rules with their own block
        // tag (other creatures approximated by the animals' rules).
        let spawnable_on = match info.rules {
            Rules::SpawnableOn(name) => match self.registries.block_tags.id(name) {
                Some(tag) => tag,
                None => return false,
            },
            _ => self.animals_spawnable_on,
        };
        let bright = level.raw_brightness(pos, 0) > 8;
        let below = level.block(pos.below());
        self.registries.block_in_tag(below, spawnable_on) && bright
    }

    /// `Mob.checkSpawnRules`: `PathfinderMob`'s walk target value.
    fn mob_check_spawn_rules(&self, level: &mut dyn SpawnLevel, info: &TypeInfo, pos: BlockPos) -> bool {
        let _ = info;
        // `Animal.getWalkTargetValue`.
        let below = level.block(pos.below());
        if self.registries.blocks.block_of(below) == self.grass_block {
            return true;
        }
        self.pathfinding_cost_from_light(level, pos) >= 0.0
    }

    /// `LevelReader.getPathfindingCostFromLightLevels`.
    fn pathfinding_cost_from_light(&self, level: &mut dyn SpawnLevel, pos: BlockPos) -> f32 {
        let darken = level.sky_darken();
        let brightness = if (-30_000_000..30_000_000).contains(&pos.x) && (-30_000_000..30_000_000).contains(&pos.z) {
            level.raw_brightness(pos, darken)
        } else {
            15
        };
        let v = brightness as f32 / 15.0;
        let curved = v / (4.0 - 3.0 * v);
        let magic = curved + self.ambient_light * (1.0 - curved);
        magic - 0.5
    }

    /// `Mob.checkSpawnObstruction`'s `containsAnyLiquid` over the bounding
    /// box (entities never obstruct in a generation region; the server's
    /// spawner tests them itself).
    fn check_spawn_obstruction(&self, level: &mut dyn SpawnLevel, info: &TypeInfo, x: f64, y: f64, z: f64) -> bool {
        let half = f64::from(info.width) / 2.0;
        let (min, max) = ([x - half, y, z - half], [x + half, y + f64::from(info.height), z + half]);
        let registries = &*self.registries;
        for bx in min[0].floor() as i32..max[0].ceil() as i32 {
            for by in min[1].floor() as i32..max[1].ceil() as i32 {
                for bz in min[2].floor() as i32..max[2].ceil() as i32 {
                    if registries.blocks.state(level.block(BlockPos::new(bx, by, bz))).fluid.is_some() {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// `CollisionGetter.noCollision(aabb)`: no block collision shape
    /// overlaps the box (`BlockCollisions`, with its edge-cell rules).
    fn no_collision(&self, level: &mut dyn SpawnLevel, bb: [f64; 6]) -> bool {
        let registries = &*self.registries;
        let blocks = &registries.blocks;
        let lo = |v: f64| (v - 1.0e-7).floor() as i32 - 1;
        let hi = |v: f64| (v + 1.0e-7).floor() as i32 + 1;
        let (x0, x1) = (lo(bb[0]), hi(bb[3]));
        let (y0, y1) = (lo(bb[1]), hi(bb[4]));
        let (z0, z1) = (lo(bb[2]), hi(bb[5]));
        for z in z0..=z1 {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let face_type = i32::from(x == x0 || x == x1) + i32::from(y == y0 || y == y1) + i32::from(z == z0 || z == z1);
                    if face_type == 3 {
                        continue;
                    }
                    let state = level.block(BlockPos::new(x, y, z));
                    let boxes: Vec<[f64; 6]> = match blocks.collision_shape(state) {
                        None | Some(minecraftoss_core::block::FaceShape::Empty) => continue,
                        Some(minecraftoss_core::block::FaceShape::Full) => vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]],
                        Some(minecraftoss_core::block::FaceShape::Boxes(b)) => b.to_vec(),
                    };
                    let large = boxes.iter().any(|b| b[0] < 0.0 || b[1] < 0.0 || b[2] < 0.0 || b[3] > 1.0 || b[4] > 1.0 || b[5] > 1.0);
                    if face_type == 1 && !large {
                        continue;
                    }
                    if face_type == 2 && blocks.block(blocks.block_of(state)).name.as_str() != "minecraft:moving_piston" {
                        continue;
                    }
                    let (ox, oy, oz) = (f64::from(x), f64::from(y), f64::from(z));
                    let overlaps = boxes.iter().any(|b| {
                        b[0] + ox < bb[3] && b[3] + ox > bb[0] && b[1] + oy < bb[4] && b[4] + oy > bb[1] && b[2] + oz < bb[5] && b[5] + oz > bb[2]
                    });
                    if overlaps {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// `Mob.finalizeSpawn` and the type's overrides, drawing from the level random.
    fn finalize_spawn(&self, level: &mut dyn SpawnLevel, kind: &str, tag: &mut Tag, pos: BlockPos, group: &mut Group) {
        let registries = &*self.registries;
        let biome = level.biome(pos);
        let mut unsupported = false;
        match kind {
            "minecraft:pig" | "minecraft:cow" | "minecraft:chicken" => {
                let (variants, sounds) = match kind {
                    "minecraft:pig" => (&self.pig_variants, &self.pig_sounds),
                    "minecraft:cow" => (&self.cow_variants, &self.cow_sounds),
                    _ => (&self.chicken_variants, &self.chicken_sounds),
                };
                if let Some(variant) = variants.pick(registries, biome, level.random(), &mut unsupported) {
                    set(tag, "variant", Tag::String(variant.to_owned()));
                }
                if !sounds.is_empty() {
                    let sound = &sounds[level.random().next_i32_bound(sounds.len() as i32) as usize];
                    set(tag, "sound_variant", Tag::String(sound.clone()));
                }
                ageable(level, tag, group, 0.05);
            }
            // `Wolf.finalizeSpawn`: the pack shares the first wolf's variant
            // (`WolfPackData`, never babies); each draws its voice.
            "minecraft:wolf" => {
                let variant = match group {
                    Group::WolfPack { variant, .. } => Some(variant.clone()),
                    _ => self.wolf_variants.pick(registries, biome, level.random(), &mut unsupported).map(str::to_owned),
                };
                if let Some(variant) = variant {
                    set(tag, "variant", Tag::String(variant.clone()));
                    if !matches!(group, Group::WolfPack { .. }) {
                        *group = Group::WolfPack { variant, size: 0 };
                    }
                }
                if !self.wolf_sounds.is_empty() {
                    let sound = &self.wolf_sounds[level.random().next_i32_bound(self.wolf_sounds.len() as i32) as usize];
                    set(tag, "sound_variant", Tag::String(sound.clone()));
                }
                ageable(level, tag, group, 0.05);
            }
            // `Fox.finalizeSpawn`: the group's variant (snow in snowy biomes),
            // babies from the third fox on; one in five holds an item.
            "minecraft:fox" => {
                let (snow, baby) = match group {
                    Group::Fox { snow, size } => (*snow, *size >= 2),
                    _ => {
                        let snow = registries.biome_in_tag(biome, self.snow_foxes);
                        *group = Group::Fox { snow, size: 0 };
                        (snow, false)
                    }
                };
                set(tag, "Type", Tag::String(if snow { "snow" } else { "red" }.to_owned()));
                if baby {
                    set(tag, "Age", Tag::Int(-24000));
                }
                let random = level.random();
                if random.next_f32() < 0.2 {
                    let odds = random.next_f32();
                    let item = if odds < 0.05 {
                        "minecraft:emerald"
                    } else if odds < 0.2 {
                        "minecraft:egg"
                    } else if odds < 0.4 {
                        if random.next_bool() { "minecraft:rabbit_foot" } else { "minecraft:rabbit_hide" }
                    } else if odds < 0.6 {
                        "minecraft:wheat"
                    } else if odds < 0.8 {
                        "minecraft:leather"
                    } else {
                        "minecraft:feather"
                    };
                    spawning::equip(tag, "mainhand", item);
                }
                // `Fox.createAttributes`: a 32-block follow range.
                let _ = spawning::attribute(tag, "minecraft:follow_range", 32.0);
                ageable(level, tag, group, 0.05);
            }
            // `Rabbit.finalizeSpawn`: a variant drawn for each, the group's
            // kept; the rest of a group are babies.
            "minecraft:rabbit" => {
                let roll = level.random().next_i32_bound(100);
                let drawn = if registries.biome_in_tag(biome, self.white_rabbits) {
                    if roll < 80 { 1 } else { 3 }
                } else if registries.biome_in_tag(biome, self.gold_rabbits) {
                    4
                } else if roll < 50 {
                    0
                } else if roll < 90 {
                    5
                } else {
                    2
                };
                let variant = match group {
                    Group::Rabbit { variant, .. } => *variant,
                    _ => {
                        *group = Group::Rabbit { variant: drawn, size: 0 };
                        drawn
                    }
                };
                set(tag, "RabbitType", Tag::Int(variant));
                // `setVariant` touches the attack damage (to drop the killer
                // bunny's bonus), which saves it.
                let _ = spawning::attribute(tag, "minecraft:attack_damage", 3.0);
                ageable(level, tag, group, 1.0);
            }
            // `Llama.finalizeSpawn`: strength, the group's variant, then
            // `AbstractChestedHorse.randomizeAttributes` (health).
            "minecraft:llama" => {
                let random = level.random();
                let max = if random.next_f32() < 0.04 { 5 } else { 3 };
                set(tag, "Strength", Tag::Int(1 + random.next_i32_bound(max)));
                let variant = match group {
                    Group::Llama { variant, .. } => *variant,
                    _ => {
                        let v = level.random().next_i32_bound(4);
                        *group = Group::Llama { variant: v, size: 0 };
                        v
                    }
                };
                set(tag, "Variant", Tag::Int(variant));
                randomize_horse(tag, level.random(), false);
                ageable(level, tag, group, 0.05);
            }
            // `Armadillo`: no override (its scute timer is its own random's).
            "minecraft:armadillo" => ageable(level, tag, group, 0.05),
            "minecraft:sheep" => {
                let color = self.sheep_color(registries, biome, level.random());
                set(tag, "Color", Tag::Byte(color));
                ageable(level, tag, group, 0.05);
            }
            "minecraft:horse" => {
                let random = level.random();
                let variant = match group {
                    Group::Horse { variant, .. } => *variant,
                    _ => {
                        let v = random.next_i32_bound(7);
                        *group = Group::Horse { variant: v, size: 0 };
                        v
                    }
                };
                let markings = random.next_i32_bound(5);
                set(tag, "Variant", Tag::Int((variant & 255) | (markings << 8 & 65280)));
                randomize_horse(tag, random, true);
                ageable(level, tag, group, 0.05);
            }
            "minecraft:donkey" | "minecraft:mule" => {
                if matches!(group, Group::None) {
                    *group = Group::Ageable { size: 0, chance: 0.2 };
                }
                randomize_horse(tag, level.random(), false);
                ageable(level, tag, group, 0.2);
            }
            _ => {
                level.note_unsupported(&format!("finalizeSpawn of {kind}"));
                ageable(level, tag, group, 0.05);
            }
        }
        if unsupported {
            level.note_unsupported("variant conditions");
        }
        finalize_mob(tag, &mut RandomRef(level.random()));
    }

    /// `SheepColorSpawnRules.getSheepColor`: the climate's weighted colours,
    /// the common colour turning pink one time in 500.
    fn sheep_color(&self, registries: &Registries, biome: BiomeId, random: &mut dyn RandomSource) -> i8 {
        const WHITE: i8 = 0;
        const PINK: i8 = 6;
        const GRAY: i8 = 7;
        const LIGHT_GRAY: i8 = 8;
        const BROWN: i8 = 12;
        const BLACK: i8 = 15;
        let (singles, common): ([(i8, i32); 4], i8) = if registries.biome_in_tag(biome, self.warm) {
            ([(GRAY, 5), (LIGHT_GRAY, 5), (WHITE, 5), (BLACK, 3)], BROWN)
        } else if registries.biome_in_tag(biome, self.cold) {
            ([(LIGHT_GRAY, 5), (GRAY, 5), (WHITE, 5), (BROWN, 3)], BLACK)
        } else {
            ([(BLACK, 5), (GRAY, 5), (LIGHT_GRAY, 5), (BROWN, 3)], WHITE)
        };
        let mut selection = random.next_i32_bound(100);
        for (color, weight) in singles {
            if selection < weight {
                return color;
            }
            selection -= weight;
        }
        if random.next_i32_bound(500) < 499 {
            common
        } else {
            PINK
        }
    }
}

/// `SpawnGroupData` carried through a group.
#[derive(Clone, Debug)]
enum Group {
    None,
    /// `AgeableMobGroupData`.
    Ageable { size: i32, chance: f32 },
    /// `Horse.HorseGroupData` (baby chance 0.05).
    Horse { variant: i32, size: i32 },
    /// `Wolf.WolfPackData`: its variant, and no babies.
    WolfPack { variant: String, size: i32 },
    /// `Fox.FoxGroupData`: its variant; no baby draws.
    Fox { snow: bool, size: i32 },
    /// `Rabbit.RabbitGroupData`: its `RabbitType`; baby chance 1.
    Rabbit { variant: i32, size: i32 },
    /// `Llama.LlamaGroupData`: its variant.
    Llama { variant: i32, size: i32 },
}

/// `AgeableMob.finalizeSpawn`: later members of a group may be babies.
fn ageable(level: &mut dyn SpawnLevel, tag: &mut Tag, group: &mut Group, default_chance: f32) {
    if matches!(group, Group::None) {
        *group = Group::Ageable { size: 0, chance: default_chance };
    }
    let (size, chance) = match group {
        Group::Ageable { size, chance } => (size, *chance),
        Group::Horse { size, .. } => (size, 0.05),
        // `isShouldSpawnBaby` false: no draw.
        Group::WolfPack { size, .. } | Group::Fox { size, .. } => {
            *size += 1;
            return;
        }
        Group::Rabbit { size, .. } => (size, 1.0),
        Group::Llama { size, .. } => (size, 0.05),
        Group::None => unreachable!("set above"),
    };
    if *size > 0 && level.random().next_f32() <= chance {
        set(tag, "Age", Tag::Int(-24000));
    }
    *size += 1;
}

/// `AbstractHorse.randomizeAttributes`: health from two ints; horses also
/// draw speed and jump strength.
fn randomize_horse(tag: &mut Tag, random: &mut dyn RandomSource, speed_and_jump: bool) {
    let health = 15.0f32 + random.next_i32_bound(8) as f32 + random.next_i32_bound(9) as f32;
    set_attribute_base(tag, "minecraft:max_health", f64::from(health));
    if speed_and_jump {
        let speed = (f64::from(0.45f32) + random.next_f64() * 0.3 + random.next_f64() * 0.3 + random.next_f64() * 0.3) * 0.25;
        set_attribute_base(tag, "minecraft:movement_speed", speed);
        let jump = f64::from(0.4f32) + random.next_f64() * 0.2 + random.next_f64() * 0.2 + random.next_f64() * 0.2;
        set_attribute_base(tag, "minecraft:jump_strength", jump);
    }
}

fn set(tag: &mut Tag, key: &str, value: Tag) {
    if let Tag::Compound(map) = tag {
        map.insert(key.to_owned(), value);
    }
}

fn set_attribute_base(tag: &mut Tag, id: &str, base: f64) {
    let Tag::Compound(map) = tag else { return };
    let Tag::List(list) = map.entry("attributes".to_owned()).or_insert_with(|| Tag::List(Vec::new())) else { return };
    if let Some(Tag::Compound(attribute)) = list.iter_mut().find(|a| a.get("id").and_then(Tag::as_str) == Some(id)) {
        attribute.insert("base".to_owned(), Tag::Double(base));
        return;
    }
    let mut attribute = BTreeMap::new();
    attribute.insert("id".to_owned(), Tag::String(id.to_owned()));
    attribute.insert("base".to_owned(), Tag::Double(base));
    list.push(Tag::Compound(attribute));
}

/// `EntityType.getSpawnAABB` (spawn dimension scale 1).
fn spawn_aabb(info: &TypeInfo, x: f64, y: f64, z: f64) -> [f64; 6] {
    let half = f64::from(info.width / 2.0);
    let height = f64::from(info.height);
    [x - half, y, z - half, x + half, y + height, z + half]
}

/// A `&mut dyn RandomSource` as a sized `RandomSource`.
struct RandomRef<'a>(&'a mut dyn RandomSource);

impl RandomSource for RandomRef<'_> {
    fn next_i32(&mut self) -> i32 {
        self.0.next_i32()
    }
    fn next_i32_bound(&mut self, bound: i32) -> i32 {
        self.0.next_i32_bound(bound)
    }
    fn next_i64(&mut self) -> i64 {
        self.0.next_i64()
    }
    fn next_f32(&mut self) -> f32 {
        self.0.next_f32()
    }
    fn next_f64(&mut self) -> f64 {
        self.0.next_f64()
    }
    fn next_bool(&mut self) -> bool {
        self.0.next_bool()
    }
    fn next_gaussian(&mut self) -> f64 {
        self.0.next_gaussian()
    }
}

/// The SPAWN step's `WorldGenRegion`: the centre chunk's 3x3 at their
/// FEATURES state, lit on first use (the centre's LIGHT step), with the
/// region random of a fresh region at the centre.
pub struct GenerationRegion<'a> {
    chunks: [&'a minecraftoss_core::Chunk; 9],
    center: ChunkPos,
    zoom_seed: i64,
    void_air: BlockStateId,
    random: minecraftoss_core::random::AnyRandom,
    light: Option<minecraftoss_core::light::ChunkLight>,
    compute_light: &'a dyn Fn() -> minecraftoss_core::light::ChunkLight,
    sky_light: bool,
    world_seed: i64,
    created: u32,
    /// The entities spawned, in order (all in the centre chunk).
    pub entities: Vec<Tag>,
    pub unsupported: Vec<String>,
}

impl<'a> GenerationRegion<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registries: &Registries,
        chunks: [&'a minecraftoss_core::Chunk; 9],
        world_seed: i64,
        random: minecraftoss_core::random::AnyRandom,
        compute_light: &'a dyn Fn() -> minecraftoss_core::light::ChunkLight,
        sky_light: bool,
    ) -> Self {
        let center = chunks[4].pos;
        Self {
            chunks,
            center,
            zoom_seed: minecraftoss_generator::zoom::zoom_seed(world_seed),
            void_air: registries.blocks.parse_state("minecraft:void_air").expect("void_air exists"),
            random,
            light: None,
            compute_light,
            sky_light,
            world_seed,
            created: 0,
            entities: Vec::new(),
            unsupported: Vec::new(),
        }
    }

    fn chunk(&self, x: i32, z: i32) -> Option<&'a minecraftoss_core::Chunk> {
        let (dx, dz) = ((x >> 4) - self.center.x + 1, (z >> 4) - self.center.z + 1);
        ((0..3).contains(&dx) && (0..3).contains(&dz)).then(|| self.chunks[(dz * 3 + dx) as usize])
    }

    /// Sky and block light in the centre chunk (lit on first use).
    fn light_at(&mut self, pos: BlockPos) -> (i32, i32) {
        if pos.chunk() != self.center {
            self.unsupported.push("light outside the spawning chunk".to_owned());
            return (15, 15);
        }
        let light = self.light.get_or_insert_with(|| (self.compute_light)());
        let sky = if self.sky_light { light.sky_at(pos.x, pos.y, pos.z) } else { 0 };
        (sky, light.block_at(pos.x, pos.y, pos.z))
    }
}

impl SpawnLevel for GenerationRegion<'_> {
    fn block(&self, pos: BlockPos) -> BlockStateId {
        match self.chunk(pos.x, pos.z) {
            Some(chunk) if pos.y >= chunk.min_y() && pos.y < chunk.min_y() + chunk.height() => chunk.block((pos.x & 15) as usize, pos.y, (pos.z & 15) as usize),
            Some(_) => self.void_air,
            None => BlockStateId::AIR,
        }
    }

    fn height(&self, kind: HeightmapKind, x: i32, z: i32) -> i32 {
        self.chunk(x, z).map_or(self.chunks[4].min_y(), |c| c.heightmaps.get(kind, (x & 15) as usize, (z & 15) as usize))
    }

    fn biome(&self, pos: BlockPos) -> BiomeId {
        let [qx, qy, qz] = minecraftoss_generator::zoom::quart_for_block(self.zoom_seed, pos.x, pos.y, pos.z);
        self.noise_biome(qx, qy, qz)
    }

    fn noise_biome(&self, qx: i32, qy: i32, qz: i32) -> BiomeId {
        let chunk = self.chunk(qx << 2, qz << 2).unwrap_or(self.chunks[4]);
        chunk.biome((qx & 3) as usize, qy, (qz & 3) as usize)
    }

    fn raw_brightness(&mut self, pos: BlockPos, darkening: i32) -> i32 {
        let (sky, block) = self.light_at(pos);
        (sky - darkening).max(block)
    }

    fn sky_brightness(&mut self, pos: BlockPos) -> i32 {
        self.light_at(pos).0
    }

    fn block_brightness(&mut self, pos: BlockPos) -> i32 {
        self.light_at(pos).1
    }

    fn min_y(&self) -> i32 {
        self.chunks[4].min_y()
    }

    fn random(&mut self) -> &mut dyn RandomSource {
        &mut self.random
    }

    fn next_uuid(&mut self) -> [i32; 4] {
        self.created += 1;
        let mut state = (self.world_seed as u64) ^ (self.center.pack() as u64).rotate_left(29) ^ u64::from(self.created).wrapping_mul(0xD6E8_FEB8_6659_FD93);
        let mut next = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let (a, b) = (next(), next());
        // Version 4, IETF variant, as `Mth.createInsecureUUID`.
        let a = (a & !0xF000) | 0x4000;
        let b = (b & 0x3FFF_FFFF_FFFF_FFFF) | 0x8000_0000_0000_0000;
        [(a >> 32) as i32, a as i32, (b >> 32) as i32, b as i32]
    }

    fn add_entity(&mut self, entity: Tag) {
        self.entities.push(entity);
    }

    fn note_unsupported(&mut self, what: &str) {
        self.unsupported.push(what.to_owned());
    }
}
