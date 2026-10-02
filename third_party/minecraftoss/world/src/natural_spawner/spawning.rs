//! Natural spawning around players each tick (`NaturalSpawner.spawnForChunk`
//! and `spawnCategoryForPosition` with their checks), `DifficultyInstance`,
//! the monster spawn rules (`Monster.checkMonsterSpawnRules`,
//! `Slime.checkSlimeSpawnRules`) and the monsters' `finalizeSpawn` (zombies,
//! zombie villagers, skeletons, spiders, creepers, endermen, witches, slimes
//! and zombie horses), from the pinned 26.3 common JAR.
//!
//! Draws from an entity's own random (unseeded in vanilla: a zombie's
//! reinforcement chance and knockback bonus, a zombie villager's profession)
//! come from a random seeded by the entity's UUID here; vanilla runs differ
//! in them too. Structure spawn overrides (swamp huts, ocean monuments,
//! outposts, fortresses), spawn costs and enchanted spawn equipment are not
//! ported and are noted when reached.

use super::{place, set, set_attribute_base, spawn_aabb, type_info, CreatureSpawns, Creatures, Group, RandomRef, Rules, SpawnLevel, SpawnerData, TypeInfo, Walk};
use minecraftoss_core::block::flags;
use minecraftoss_core::entity_data::finalize_mob;
use minecraftoss_core::nbt::Tag;
use minecraftoss_core::random::LegacyRandom;
use minecraftoss_core::{BiomeId, BlockPos, ChunkPos};
use std::collections::BTreeMap;

/// `MobCategory` (the spawning categories, in declaration order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MobCategory {
    Monster,
    Creature,
    Ambient,
    Axolotls,
    UndergroundWaterCreature,
    WaterCreature,
    WaterAmbient,
}

impl MobCategory {
    pub const COUNT: usize = 7;
    /// `NaturalSpawner.SPAWNING_CATEGORIES`.
    pub const SPAWNING: [Self; Self::COUNT] = [
        Self::Monster,
        Self::Creature,
        Self::Ambient,
        Self::Axolotls,
        Self::UndergroundWaterCreature,
        Self::WaterCreature,
        Self::WaterAmbient,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Monster => "monster",
            Self::Creature => "creature",
            Self::Ambient => "ambient",
            Self::Axolotls => "axolotls",
            Self::UndergroundWaterCreature => "underground_water_creature",
            Self::WaterCreature => "water_creature",
            Self::WaterAmbient => "water_ambient",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::SPAWNING.into_iter().find(|c| c.name() == name)
    }

    /// `getMaxInstancesPerChunk`.
    pub fn max_instances_per_chunk(self) -> i32 {
        match self {
            Self::Monster => 70,
            Self::Creature => 10,
            Self::Ambient => 15,
            Self::WaterAmbient => 20,
            Self::Axolotls | Self::UndergroundWaterCreature | Self::WaterCreature => 5,
        }
    }

    /// `isFriendly`.
    pub fn friendly(self) -> bool {
        self != Self::Monster
    }

    /// `isPersistent`: spawned only every 400 ticks.
    pub fn persistent(self) -> bool {
        self == Self::Creature
    }

    /// `getDespawnDistance`.
    pub fn despawn_distance(self) -> i32 {
        if self == Self::WaterAmbient { 64 } else { 128 }
    }

    /// `getNoDespawnDistance`.
    pub fn no_despawn_distance(self) -> i32 {
        32
    }

    /// `EntityType.getCategory` for the mob types spawning names; none for
    /// `MISC` and types not listed.
    pub fn of_type(kind: &str) -> Option<Self> {
        Some(match kind.trim_start_matches("minecraft:") {
            "zombie" | "zombie_villager" | "zombie_horse" | "skeleton" | "creeper" | "spider" | "enderman" | "witch" | "slime" | "husk" | "stray" | "drowned"
            | "bogged" | "parched" | "camel_husk" | "cave_spider" | "silverfish" | "phantom" | "blaze" | "ghast" | "magma_cube" | "piglin" | "hoglin"
            | "zombified_piglin" | "pillager" | "shulker" => Self::Monster,
            "cow" | "pig" | "sheep" | "chicken" | "horse" | "donkey" | "mule" | "mooshroom" | "rabbit" | "wolf" | "fox" | "goat" | "llama" | "panda"
            | "polar_bear" | "cat" | "ocelot" | "parrot" | "frog" | "camel" | "armadillo" | "turtle" | "strider" => Self::Creature,
            "bat" => Self::Ambient,
            "axolotl" => Self::Axolotls,
            "glow_squid" => Self::UndergroundWaterCreature,
            "squid" | "dolphin" => Self::WaterCreature,
            "cod" | "salmon" | "tropical_fish" | "pufferfish" => Self::WaterAmbient,
            _ => return None,
        })
    }
}

/// `EntityType.canSpawnFarFromPlayer`: creature (and misc) types, pillagers
/// and shulkers.
fn can_spawn_far_from_player(kind: &str) -> bool {
    MobCategory::of_type(kind) == Some(MobCategory::Creature) || matches!(kind, "minecraft:pillager" | "minecraft:shulker")
}

/// `DifficultyInstance`.
#[derive(Clone, Copy, Debug)]
pub struct DifficultyInstance {
    /// `Difficulty.getId`: 0 peaceful to 3 hard.
    pub base: i32,
    pub effective: f32,
}

impl DifficultyInstance {
    /// `DifficultyInstance(base, totalGameTime, localGameTime, moonBrightness)`.
    pub fn new(base: i32, total_game_time: i64, local_game_time: i64, moon_brightness: f32) -> Self {
        if base == 0 {
            return Self { base, effective: 0.0 };
        }
        let hard = base == 3;
        let mut scale = 0.75f32;
        let global = ((total_game_time as f32 + -72000.0) / 1_440_000.0).clamp(0.0, 1.0) * 0.25;
        scale += global;
        let mut local = 0.0f32;
        local += (local_game_time as f32 / 3_600_000.0).clamp(0.0, 1.0) * if hard { 1.0 } else { 0.75 };
        local += (moon_brightness * 0.25).clamp(0.0, global);
        if base == 1 {
            local *= 0.5;
        }
        scale += local;
        Self { base, effective: base as f32 * scale }
    }

    /// `getSpecialMultiplier`.
    pub fn special_multiplier(self) -> f32 {
        if self.effective < 2.0 {
            0.0
        } else if self.effective > 4.0 {
            1.0
        } else {
            (self.effective - 2.0) / 2.0
        }
    }
}

/// `DimensionType.MOON_BRIGHTNESS_PER_PHASE`, by moon phase.
pub const MOON_BRIGHTNESS_PER_PHASE: [f32; 8] = [1.0, 0.75, 0.5, 0.25, 0.0, 0.25, 0.5, 0.75];

/// What spawning reads besides the level's blocks and light.
pub struct SpawnContext<'a> {
    /// The feet of every player that is not a spectator.
    pub players: &'a [[f64; 3]],
    /// The respawn block when it is in this dimension (`getRespawnData`).
    pub respawn: Option<BlockPos>,
    /// `Difficulty.getId`.
    pub difficulty: i32,
    /// `getOverworldClockTime`.
    pub overworld_time: i64,
    /// `getMoonBrightness` (by the moon phase).
    pub moon_brightness: f32,
    /// `Level.isRaining` and `isThundering`.
    pub raining: bool,
    pub thundering: bool,
    /// The world seed (slime chunks).
    pub seed: i64,
    /// `SURFACE_SLIME_SPAWN_CHANCE` in a biome (swamp surfaces).
    pub surface_slime_chance: &'a dyn Fn(BiomeId) -> f32,
    /// `SpecialDates.isHalloween`.
    pub halloween: bool,
    /// `canSpawnEntitiesInChunk`: the chunk's entities tick.
    pub can_spawn_in_chunk: &'a dyn Fn(ChunkPos) -> bool,
    /// Boxes of entities that block building (living entities): they
    /// obstruct spawns (`isUnobstructed`), and spawned mobs join them.
    pub obstacles: Vec<[f64; 6]>,
    /// Boxes of chickens nobody rides (a baby zombie's jockey search).
    pub chickens: Vec<[f64; 6]>,
}

/// `SpawnState`'s tests around one spawn: the spawn potential (`canSpawn`)
/// and the counts after it (`afterSpawn`), given the type's spawn cost at
/// the position (energy budget and charge).
pub trait SpawnCallbacks {
    fn can_spawn(&mut self, _kind: &str, _pos: BlockPos, _cost: Option<(f64, f64)>) -> bool {
        true
    }
    fn after_spawn(&mut self, _kind: &str, _pos: BlockPos, _cost: Option<(f64, f64)>) {}
}

/// No mob cap or potential (`spawnCategoryForPosition`'s debug entry).
pub struct Unlimited;
impl SpawnCallbacks for Unlimited {}

/// `SpawnGroupData` a monster type carries through its group.
#[derive(Clone, Copy, Debug)]
enum MonsterGroup {
    None,
    /// `Zombie.ZombieGroupData`.
    Zombie { baby: bool, can_spawn_jockey: bool },
    /// `Spider.SpiderEffectsGroupData` (no effect below Hard).
    Spider,
    /// `AgeableMobGroupData` (slimes, zombie horses).
    Ageable { should_spawn_baby: bool, chance: f32, size: i32 },
}

fn byte(value: bool) -> Tag {
    Tag::Byte(i8::from(value))
}

/// An equipment slot holding one item (`setItemSlot(slot, new ItemStack(item))`).
pub(super) fn equip(tag: &mut Tag, slot: &str, item: &str) {
    let Tag::Compound(map) = tag else { return };
    let equipment = map.entry("equipment".to_owned()).or_insert_with(|| Tag::Compound(BTreeMap::new()));
    if let Tag::Compound(slots) = equipment {
        let mut stack = BTreeMap::new();
        stack.insert("count".to_owned(), Tag::Int(1));
        stack.insert("id".to_owned(), Tag::String(item.to_owned()));
        slots.insert(slot.to_owned(), Tag::Compound(stack));
    }
}

fn has_equipment(tag: &Tag, slot: &str) -> bool {
    tag.get("equipment").and_then(|e| e.get(slot)).is_some()
}

/// `AttributeMap.getInstance`: the attribute's saved entry, created with
/// the type's default base when first touched.
pub(super) fn attribute<'t>(tag: &'t mut Tag, id: &str, default_base: f64) -> Option<&'t mut BTreeMap<String, Tag>> {
    let Tag::Compound(map) = tag else { return None };
    let Tag::List(list) = map.entry("attributes".to_owned()).or_insert_with(|| Tag::List(Vec::new())) else { return None };
    let index = match list.iter().position(|a| a.get("id").and_then(Tag::as_str) == Some(id)) {
        Some(i) => i,
        None => {
            let mut entry = BTreeMap::new();
            entry.insert("id".to_owned(), Tag::String(id.to_owned()));
            entry.insert("base".to_owned(), Tag::Double(default_base));
            list.push(Tag::Compound(entry));
            list.len() - 1
        }
    };
    match &mut list[index] {
        Tag::Compound(entry) => Some(entry),
        _ => None,
    }
}

/// `addOrReplacePermanentModifier`.
fn add_modifier(tag: &mut Tag, id: &str, default_base: f64, modifier_id: &str, amount: f64, operation: &str) {
    let Some(entry) = attribute(tag, id, default_base) else { return };
    let Tag::List(modifiers) = entry.entry("modifiers".to_owned()).or_insert_with(|| Tag::List(Vec::new())) else { return };
    modifiers.retain(|m| m.get("id").and_then(Tag::as_str) != Some(modifier_id));
    let mut modifier = BTreeMap::new();
    modifier.insert("amount".to_owned(), Tag::Double(amount));
    modifier.insert("id".to_owned(), Tag::String(modifier_id.to_owned()));
    modifier.insert("operation".to_owned(), Tag::String(operation.to_owned()));
    modifiers.push(Tag::Compound(modifier));
}

/// The follow range base a type's attribute supplier gives.
fn follow_range_base(kind: &str) -> f64 {
    match kind {
        "minecraft:zombie" | "minecraft:zombie_villager" | "minecraft:husk" | "minecraft:drowned" => 35.0,
        "minecraft:enderman" => 64.0,
        _ => 16.0,
    }
}

/// An entity's own random, seeded from its UUID (vanilla's is unseeded).
fn entity_random(uuid: [i32; 4]) -> LegacyRandom {
    let seed = uuid.iter().fold(0x5DEE_CE66_Du64, |h, &w| h.rotate_left(17) ^ u64::from(w as u32));
    LegacyRandom::new(seed as i64)
}

/// `WorldgenRandom.seedSlimeChunk(x, z, seed, 987234911).nextInt(10) == 0`.
pub fn is_slime_chunk(chunk: ChunkPos, seed: i64) -> bool {
    let (x, z) = (chunk.x, chunk.z);
    let mixed = seed
        .wrapping_add(i64::from(x.wrapping_mul(x).wrapping_mul(4_987_142)))
        .wrapping_add(i64::from(x.wrapping_mul(5_947_611)))
        .wrapping_add(i64::from(z.wrapping_mul(z)).wrapping_mul(4_392_871))
        .wrapping_add(i64::from(z.wrapping_mul(389_711)))
        ^ 987_234_911;
    LegacyRandom::new(mixed).next_i32_bound(10) == 0
}

/// `VillagerType.BY_BIOME` (plains otherwise).
fn villager_type(biome: &str) -> &'static str {
    match biome.trim_start_matches("minecraft:") {
        "badlands" | "desert" | "eroded_badlands" | "wooded_badlands" => "minecraft:desert",
        "bamboo_jungle" | "jungle" | "sparse_jungle" => "minecraft:jungle",
        "savanna_plateau" | "savanna" | "windswept_savanna" => "minecraft:savanna",
        "deep_frozen_ocean" | "frozen_ocean" | "frozen_river" | "ice_spikes" | "snowy_beach" | "snowy_taiga" | "snowy_plains" | "grove" | "snowy_slopes"
        | "frozen_peaks" | "jagged_peaks" => "minecraft:snow",
        "swamp" | "mangrove_swamp" => "minecraft:swamp",
        "old_growth_spruce_taiga" | "old_growth_pine_taiga" | "windswept_gravelly_hills" | "windswept_hills" | "taiga" | "windswept_forest" => "minecraft:taiga",
        _ => "minecraft:plains",
    }
}

/// `VillagerProfession` in registration order.
const PROFESSIONS: [&str; 15] = [
    "none",
    "armorer",
    "butcher",
    "cartographer",
    "cleric",
    "farmer",
    "fisherman",
    "fletcher",
    "leatherworker",
    "librarian",
    "mason",
    "nitwit",
    "shepherd",
    "toolsmith",
    "weaponsmith",
];

/// `Mob.getEquipmentForSlot`: leather, copper, gold, chainmail, iron, diamond.
fn equipment_for_slot(slot: &str, armor_type: i32) -> Option<String> {
    let material = ["leather", "copper", "golden", "chainmail", "iron", "diamond"].get(usize::try_from(armor_type).ok()?)?;
    let piece = match slot {
        "head" => "helmet",
        "chest" => "chestplate",
        "legs" => "leggings",
        "feet" => "boots",
        _ => return None,
    };
    Some(format!("minecraft:{material}_{piece}"))
}

/// The box of a mob after `finalizeSpawn` (babies and slimes resize).
pub(super) fn mob_box(tag: &Tag, info: &TypeInfo, at: [f64; 3]) -> [f64; 6] {
    let (mut width, mut height) = (info.width, info.height);
    if tag.get("IsBaby").and_then(Tag::as_i64) == Some(1) {
        (width, height) = (0.49, 0.98);
    }
    if let Some(size) = tag.get("Size").and_then(Tag::as_i64) {
        let scale = (size + 1) as f32;
        (width, height) = (width * scale, height * scale);
    }
    let half = f64::from(width / 2.0);
    [at[0] - half, at[1], at[2] - half, at[0] + half, at[1] + f64::from(height), at[2] + half]
}

fn overlaps(a: &[f64; 6], b: &[f64; 6]) -> bool {
    a[0] < b[3] && a[3] > b[0] && a[1] < b[4] && a[4] > b[1] && a[2] < b[5] && a[5] > b[2]
}

impl CreatureSpawns {
    /// `MobSpawnSettings.getMobSpawnCost` of the biome at a position.
    pub(super) fn cost_at(&self, level: &dyn SpawnLevel, kind: &str, pos: BlockPos) -> Option<(f64, f64)> {
        self.costs[usize::from(level.biome(pos).0)].get(kind).copied()
    }

    /// A biome's spawn list for a category (`mobsAt` without structures).
    fn mobs_at(&self, level: &dyn SpawnLevel, category: MobCategory, pos: BlockPos) -> &Creatures {
        let biome = level.biome(pos);
        &self.by_category[usize::from(biome.0)][category as usize]
    }

    /// `NaturalSpawner.spawnCategoryForPosition`: up to three groups
    /// around `start`, each a random walk that spawns the first type drawn
    /// where it may. `chunk` is the chunk spawning (the start's chunk).
    pub fn spawn_category_for_position(
        &self,
        level: &mut dyn SpawnLevel,
        context: &mut SpawnContext,
        callbacks: &mut dyn SpawnCallbacks,
        category: MobCategory,
        chunk: ChunkPos,
        start: BlockPos,
    ) {
        let y_start = start.y;
        if self.registries.blocks.is(level.block(start), flags::REDSTONE_CONDUCTOR) {
            return;
        }
        let mut cluster_size = 0;
        for _ in 0..3 {
            let (mut x, mut z) = (start.x, start.z);
            let mut current: Option<SpawnerData> = None;
            let mut monster_group = MonsterGroup::None;
            let mut animal_group = Group::None;
            let mut max = (level.random().next_f32() * 4.0).ceil() as i32;
            let mut ll = 0;
            while ll < max {
                ll += 1;
                x += level.random().next_i32_bound(6) - level.random().next_i32_bound(6);
                z += level.random().next_i32_bound(6) - level.random().next_i32_bound(6);
                let pos = BlockPos::new(x, y_start, z);
                let (xx, zz) = (f64::from(x) + 0.5, f64::from(z) + 0.5);
                // `getNearestPlayer(xx, yStart, zz, -1, false)`.
                let Some(nearest) = context
                    .players
                    .iter()
                    .map(|p| {
                        let (dx, dy, dz) = (p[0] - xx, p[1] - f64::from(y_start), p[2] - zz);
                        dx * dx + dy * dy + dz * dz
                    })
                    .min_by(f64::total_cmp)
                else {
                    continue;
                };
                if !Self::right_distance(context, chunk, pos, nearest) {
                    continue;
                }
                if current.is_none() {
                    let Some(data) = self.random_spawn_mob_at(level, category, pos) else { break };
                    // `count.sample`: a constant draws nothing.
                    max = if data.constant { data.min } else { level.random().next_i32_bound(data.max - data.min + 1) + data.min };
                    current = Some(data);
                }
                let data = current.clone().expect("drawn above");
                let Some(info) = type_info(&data.kind) else {
                    level.note_unsupported(&format!("natural spawning of {}", data.kind));
                    continue;
                };
                if !self.valid_spawn_position_for_type(level, context, &data, &info, pos, nearest) {
                    continue;
                }
                let cost = self.cost_at(level, &data.kind, pos);
                if !callbacks.can_spawn(&data.kind, pos, cost) {
                    continue;
                }
                // `getMobForSpawn`: types not allowed in peaceful are not created.
                if context.difficulty == 0 && MobCategory::of_type(&data.kind) == Some(MobCategory::Monster) {
                    return;
                }
                let Some(mut tag) = self.registries.entities.as_ref().and_then(|c| c.default_tag(&data.kind)) else {
                    level.note_unsupported(&format!("creating {}", data.kind));
                    return;
                };
                let y_rot = level.random().next_f32() * 360.0;
                let uuid = level.next_uuid();
                let at = [xx, f64::from(y_start), zz];
                place(&mut tag, at, y_rot, 0.0, uuid);
                if !self.valid_position_for_mob(level, context, &data.kind, &info, pos, at, nearest) {
                    continue;
                }
                let difficulty = Self::difficulty_at(level, context, pos);
                let root = if info.rules.creature() {
                    self.finalize_spawn(level, &data.kind, &mut tag, pos, &mut animal_group);
                    tag
                } else {
                    self.finalize_monster(level, context, &data.kind, tag, pos, difficulty, &mut monster_group, true)
                };
                cluster_size += 1;
                let riding = root.get("Passengers").is_some();
                for (entity, kind) in Self::self_and_passengers(&root) {
                    let entity_info = type_info(kind).unwrap_or(info);
                    let bb = mob_box(entity, &entity_info, at);
                    context.obstacles.push(bb);
                    if kind == "minecraft:chicken" && !riding {
                        context.chickens.push(bb);
                    }
                }
                level.add_entity(root);
                callbacks.after_spawn(&data.kind, pos, cost);
                if cluster_size >= info.cluster {
                    return;
                }
            }
        }
    }

    /// An entity tag and its passengers' tags, with their types.
    fn self_and_passengers(tag: &Tag) -> Vec<(&Tag, &str)> {
        let mut out = vec![(tag, tag.get("id").and_then(Tag::as_str).unwrap_or(""))];
        for passenger in tag.get("Passengers").and_then(Tag::as_list).into_iter().flatten() {
            out.extend(Self::self_and_passengers(passenger));
        }
        out
    }

    /// `isRightDistanceToPlayerAndSpawnPoint`.
    fn right_distance(context: &SpawnContext, chunk: ChunkPos, pos: BlockPos, nearest: f64) -> bool {
        if nearest <= 576.0 {
            return false;
        }
        if let Some(r) = context.respawn {
            // `closerToCenterThan`: from the respawn block's centre.
            let (dx, dy, dz) = (
                f64::from(r.x) + 0.5 - (f64::from(pos.x) + 0.5),
                f64::from(r.y) + 0.5 - f64::from(pos.y),
                f64::from(r.z) + 0.5 - (f64::from(pos.z) + 0.5),
            );
            if dx * dx + dy * dy + dz * dz < 24.0 * 24.0 {
                return false;
            }
        }
        let at = pos.chunk();
        at == chunk || (context.can_spawn_in_chunk)(at)
    }

    /// `getRandomSpawnMobAt`.
    fn random_spawn_mob_at(&self, level: &mut dyn SpawnLevel, category: MobCategory, pos: BlockPos) -> Option<SpawnerData> {
        let biome = level.biome(pos);
        if category == MobCategory::WaterAmbient
            && self.reduced_water_ambient.is_some_and(|t| self.registries.biome_in_tag(biome, t))
            && level.random().next_f32() < 0.98
        {
            return None;
        }
        let list = self.mobs_at(level, category, pos).clone();
        list.pick(level.random()).cloned()
    }

    /// `isValidSpawnPostitionForType`.
    fn valid_spawn_position_for_type(
        &self,
        level: &mut dyn SpawnLevel,
        context: &SpawnContext,
        data: &SpawnerData,
        info: &TypeInfo,
        pos: BlockPos,
        nearest: f64,
    ) -> bool {
        let Some(type_category) = MobCategory::of_type(&data.kind) else { return false };
        let despawn = f64::from(type_category.despawn_distance());
        if !can_spawn_far_from_player(&data.kind) && nearest > despawn * despawn {
            return false;
        }
        // `canSpawnMobAt`: the list here holds the same `SpawnerData`.
        let here = self.mobs_at(level, type_category, pos);
        if !here.entries.iter().any(|e| e.kind == data.kind && e.min == data.min && e.max == data.max && e.constant == data.constant) {
            return false;
        }
        if !self.is_spawn_position_ok(level, &data.kind, info, pos) {
            return false;
        }
        if !self.natural_spawn_rules(level, context, &data.kind, info, pos) {
            return false;
        }
        self.no_collision(level, spawn_aabb(info, f64::from(pos.x) + 0.5, f64::from(pos.y), f64::from(pos.z) + 0.5))
    }

    /// `SpawnPlacements.checkSpawnRules` with reason `NATURAL`.
    fn natural_spawn_rules(&self, level: &mut dyn SpawnLevel, context: &SpawnContext, kind: &str, info: &TypeInfo, pos: BlockPos) -> bool {
        match info.rules {
            Rules::Animal | Rules::SpawnableOn(_) => self.check_spawn_rules(level, info, pos),
            Rules::Monster => self.dark_enough(level, context, pos) && self.mob_spawn_rules(level, kind, info, pos),
            // `canSeeSky`: full sky light, tested after the monster rules.
            Rules::SurfaceMonster => self.dark_enough(level, context, pos) && self.mob_spawn_rules(level, kind, info, pos) && level.sky_brightness(pos) >= 15,
            Rules::Stray => {
                // The sky is seen from the top of any powder snow above.
                let mut sky = pos.above();
                while self.registries.blocks.block(self.registries.blocks.block_of(level.block(sky))).name.as_str() == "minecraft:powder_snow" {
                    sky = sky.above();
                }
                self.dark_enough(level, context, pos) && self.mob_spawn_rules(level, kind, info, pos) && level.sky_brightness(sky.below()) >= 15
            }
            Rules::Slime => self.slime_spawn_rules(level, context, kind, info, pos),
            Rules::Bat => self.bat_spawn_rules(level, kind, info, pos),
            // `pos.y <= seaLevel - 33`, unlit, in water: no random draws.
            Rules::GlowSquid => {
                let state = level.block(pos);
                let water = self.registries.blocks.block(self.registries.blocks.block_of(state)).name.as_str() == "minecraft:water";
                pos.y <= level.sea_level() - 33 && level.raw_brightness(pos, 0) == 0 && water
            }
            Rules::Other => {
                level.note_unsupported(&format!("spawn rules of {kind}"));
                false
            }
        }
    }

    /// `Monster.isDarkEnoughToSpawn`.
    fn dark_enough(&self, level: &mut dyn SpawnLevel, context: &SpawnContext, pos: BlockPos) -> bool {
        let sky = level.sky_brightness(pos);
        if sky > level.random().next_i32_bound(32) {
            return false;
        }
        if self.monster_block_light_limit < 15 && level.block_brightness(pos) > self.monster_block_light_limit {
            return false;
        }
        let darkening = if context.thundering { 10 } else { level.sky_darken() };
        let brightness = level.raw_brightness(pos, darkening);
        brightness <= level.random().next_i32_bound(self.monster_light_test_max + 1)
    }

    /// `Bat.checkBatSpawnRules`.
    fn bat_spawn_rules(&self, level: &mut dyn SpawnLevel, kind: &str, info: &TypeInfo, pos: BlockPos) -> bool {
        if pos.y >= level.height(minecraftoss_core::chunk::HeightmapKind::WorldSurface, pos.x, pos.z) {
            return false;
        }
        if level.random().next_bool() {
            return false;
        }
        let darkening = level.sky_darken();
        if level.raw_brightness(pos, darkening) > level.random().next_i32_bound(4) {
            return false;
        }
        let below = level.block(pos.below());
        if !self.bats_spawnable_on.is_some_and(|t| self.registries.block_in_tag(below, t)) {
            return false;
        }
        self.mob_spawn_rules(level, kind, info, pos)
    }

    /// `Mob.checkMobSpawnRules`: a valid spawn block below.
    fn mob_spawn_rules(&self, level: &mut dyn SpawnLevel, kind: &str, info: &TypeInfo, pos: BlockPos) -> bool {
        let below = level.block(pos.below());
        self.is_valid_spawn(&self.registries, below, kind, info)
    }

    /// `Slime.checkSlimeSpawnRules`: swamp surfaces by the moon, or slime
    /// chunks below 40.
    fn slime_spawn_rules(&self, level: &mut dyn SpawnLevel, context: &SpawnContext, kind: &str, info: &TypeInfo, pos: BlockPos) -> bool {
        if context.difficulty == 0 {
            return false;
        }
        let biome = level.biome(pos);
        let surface = self.surface_slimes.is_some_and(|t| self.registries.biome_in_tag(biome, t));
        if surface && pos.y > 50 && pos.y < 70 {
            let chance = (context.surface_slime_chance)(biome);
            if level.random().next_f32() < chance {
                let darkening = level.sky_darken();
                let brightness = level.raw_brightness(pos, darkening);
                if brightness <= level.random().next_i32_bound(8) {
                    return self.mob_spawn_rules(level, kind, info, pos);
                }
            }
        }
        let slime_chunk = is_slime_chunk(pos.chunk(), context.seed);
        if level.random().next_i32_bound(10) == 0 && slime_chunk && pos.y < 40 {
            return self.mob_spawn_rules(level, kind, info, pos);
        }
        false
    }

    /// `isValidPositionForMob`: close enough to stay, the mob's own spawn
    /// rules (its walk target) and nothing in the way.
    #[allow(clippy::too_many_arguments)]
    fn valid_position_for_mob(
        &self,
        level: &mut dyn SpawnLevel,
        context: &SpawnContext,
        kind: &str,
        info: &TypeInfo,
        block: BlockPos,
        at: [f64; 3],
        nearest: f64,
    ) -> bool {
        let despawn = f64::from(MobCategory::of_type(kind).map_or(128, MobCategory::despawn_distance));
        if nearest > despawn * despawn && info.remove_far {
            return false;
        }
        // `PathfinderMob.checkSpawnRules`: a walk target value of at least 0.
        let walk_target_ok = match info.walk {
            Walk::Animal => self.mob_check_spawn_rules(level, info, block),
            Walk::Monster => -self.pathfinding_cost_from_light(level, block) >= 0.0,
            Walk::Neutral => true,
        };
        if !walk_target_ok || !self.check_spawn_obstruction(level, info, at[0], at[1], at[2]) {
            return false;
        }
        // `isUnobstructed`: no entity that blocks building overlaps.
        let bb = spawn_aabb(info, at[0], at[1], at[2]);
        !context.obstacles.iter().any(|o| overlaps(&bb, o))
    }

    /// `SummonCommand.createEntity`'s mob: the type's fresh tag at `at` (the
    /// new entity's yaw, and a UUID from the level's entity random) with the
    /// command's NBT loaded over it, keeping the command's position; or,
    /// without NBT, `finalizeSpawn(COMMAND)` as the first of its group.
    #[allow(clippy::too_many_arguments)]
    pub fn summon(&self, level: &mut dyn SpawnLevel, context: &mut SpawnContext, kind: &str, at: [f64; 3], nbt: Option<&Tag>, y_rot: f32) -> Result<Tag, String> {
        let Some(mut tag) = self.registries.entities.as_ref().and_then(|c| c.default_tag(kind)) else {
            return Err(format!("Unable to summon {kind}"));
        };
        let uuid = level.next_uuid();
        place(&mut tag, at, y_rot, 0.0, uuid);
        // `Mob.readAdditionalSaveData`: the command's tag is loaded, which
        // reads an absent `CanPickUpLoot` as false (though a villager's
        // constructor set it).
        if let Tag::Compound(map) = &mut tag {
            if map.contains_key("CanPickUpLoot") {
                map.insert("CanPickUpLoot".to_owned(), Tag::Byte(0));
            }
        }
        if let Some(Tag::Compound(nbt)) = nbt {
            if let Tag::Compound(map) = &mut tag {
                for (key, value) in nbt {
                    if key != "id" && key != "Pos" {
                        map.insert(key.clone(), value.clone());
                    }
                }
            }
            return Ok(tag);
        }
        let pos = BlockPos::new(at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32);
        match type_info(kind) {
            Some(info) if info.rules.creature() => {
                self.finalize_spawn(level, kind, &mut tag, pos, &mut Group::None);
                Ok(tag)
            }
            Some(_) => {
                let difficulty = Self::difficulty_at(level, context, pos);
                Ok(self.finalize_monster(level, context, kind, tag, pos, difficulty, &mut MonsterGroup::None, false))
            }
            // Types without a spawn port come as their fresh tag.
            None => Ok(tag),
        }
    }

    /// `ServerLevel.getCurrentDifficultyAt`.
    fn difficulty_at(level: &dyn SpawnLevel, context: &SpawnContext, pos: BlockPos) -> DifficultyInstance {
        let (local, moon) = match level.inhabited_time(pos.chunk()) {
            Some(time) => (time, context.moon_brightness),
            None => (0, 0.0),
        };
        DifficultyInstance::new(context.difficulty, context.overworld_time, local, moon)
    }

    /// `Mob.populateDefaultEquipmentSlots`: an armor set, most likely
    /// leather, one piece after another until a partial-set roll stops it.
    fn populate_armor(level: &mut dyn SpawnLevel, tag: &mut Tag, difficulty: DifficultyInstance) {
        if level.random().next_f32() < 0.15 * difficulty.special_multiplier() {
            let mut armor_type = level.random().next_i32_bound(3);
            for _ in 1..=3 {
                if level.random().next_f32() < 0.1087 {
                    armor_type += 1;
                }
            }
            let partial = if difficulty.base == 3 { 0.1 } else { 0.25 };
            let mut first = true;
            // `EQUIPMENT_POPULATION_ORDER`.
            for slot in ["head", "chest", "legs", "feet"] {
                if !first && level.random().next_f32() < partial {
                    break;
                }
                first = false;
                if !has_equipment(tag, slot) {
                    if let Some(item) = equipment_for_slot(slot, armor_type) {
                        equip(tag, slot, &item);
                    }
                }
            }
        }
    }

    /// `Mob.populateDefaultEquipmentEnchantments`: a roll per equipped item
    /// (the main hand at 0.25, armor at 0.5, times the special multiplier)
    /// in `EquipmentSlot.VALUES` order.
    fn roll_enchantments(level: &mut dyn SpawnLevel, tag: &Tag, difficulty: DifficultyInstance) {
        let special = difficulty.special_multiplier();
        for (slot, chance) in [("mainhand", 0.25f32), ("feet", 0.5), ("legs", 0.5), ("chest", 0.5), ("head", 0.5)] {
            if has_equipment(tag, slot) && level.random().next_f32() < chance * special {
                level.note_unsupported("enchanted spawn equipment");
            }
        }
    }

    /// The Halloween pumpkin head of zombies and skeletons.
    fn halloween_head(level: &mut dyn SpawnLevel, context: &SpawnContext, tag: &mut Tag) {
        if !has_equipment(tag, "head") && context.halloween && level.random().next_f32() < 0.25 {
            let item = if level.random().next_f32() < 0.1 { "minecraft:jack_o_lantern" } else { "minecraft:carved_pumpkin" };
            equip(tag, "head", item);
            level.note_unsupported("pumpkin head drop chance");
        }
    }

    /// The monsters' `finalizeSpawn` (with `Mob.finalizeSpawn`'s common
    /// part), returning the entity to add: the mob, or the chicken it rides.
    #[allow(clippy::too_many_arguments)]
    fn finalize_monster(
        &self,
        level: &mut dyn SpawnLevel,
        context: &SpawnContext,
        kind: &str,
        mut tag: Tag,
        pos: BlockPos,
        difficulty: DifficultyInstance,
        group: &mut MonsterGroup,
        natural: bool,
    ) -> Tag {
        let special = difficulty.special_multiplier();
        let uuid = Self::uuid(&tag);
        let mut own = entity_random(uuid);
        // Created before `Mob.finalizeSpawn` adds its bonus.
        let _ = attribute(&mut tag, "minecraft:follow_range", follow_range_base(kind));
        match kind {
            "minecraft:husk" => {
                finalize_mob(&mut tag, &mut RandomRef(level.random()));
                let root = self.finalize_zombie(level, context, tag, pos, difficulty, group, &mut own);
                return self.finalize_husk(level, context, root, pos, difficulty, natural);
            }
            "minecraft:zombie" | "minecraft:zombie_villager" => {
                if kind == "minecraft:zombie_villager" {
                    // The constructor's profession, then `finalizeVillagerType`.
                    let profession = PROFESSIONS[own.next_i32_bound(PROFESSIONS.len() as i32) as usize];
                    let biome = self.biome_name(level.biome(pos));
                    let mut data = BTreeMap::new();
                    data.insert("level".to_owned(), Tag::Int(1));
                    data.insert("profession".to_owned(), Tag::String(format!("minecraft:{profession}")));
                    data.insert("type".to_owned(), Tag::String(villager_type(&biome).to_owned()));
                    set(&mut tag, "VillagerData", Tag::Compound(data));
                    set(&mut tag, "VillagerDataFinalized", byte(true));
                }
                finalize_mob(&mut tag, &mut RandomRef(level.random()));
                return self.finalize_zombie(level, context, tag, pos, difficulty, group, &mut own);
            }
            // `AbstractSkeleton.finalizeSpawn`, which strays, bogged and
            // parched do not override.
            "minecraft:skeleton" | "minecraft:stray" | "minecraft:bogged" | "minecraft:parched" => {
                finalize_mob(&mut tag, &mut RandomRef(level.random()));
                self.finalize_skeleton(level, context, &mut tag, difficulty);
            }
            "minecraft:spider" => {
                finalize_mob(&mut tag, &mut RandomRef(level.random()));
                if level.random().next_i32_bound(100) == 0 {
                    // A skeleton jockey: the skeleton finalizes as a spawn of its own.
                    if let Some(mut skeleton) = self.registries.entities.as_ref().and_then(|c| c.default_tag("minecraft:skeleton")) {
                        let skeleton_uuid = level.next_uuid();
                        place(&mut skeleton, Self::position(&tag), Self::y_rot(&tag), 0.0, skeleton_uuid);
                        let _ = attribute(&mut skeleton, "minecraft:follow_range", 16.0);
                        finalize_mob(&mut skeleton, &mut RandomRef(level.random()));
                        self.finalize_skeleton(level, context, &mut skeleton, difficulty);
                        set(&mut tag, "Passengers", Tag::List(vec![skeleton]));
                    }
                }
                if matches!(group, MonsterGroup::None) {
                    *group = MonsterGroup::Spider;
                    if difficulty.base == 3 && level.random().next_f32() < 0.1 * special {
                        level.note_unsupported("spider spawn effect");
                    }
                }
            }
            "minecraft:witch" => {
                // `Raider.finalizeSpawn`, then `PatrollingMonster`'s leader
                // roll (witches never lead), then `Mob`'s part.
                set(&mut tag, "CanJoinRaid", byte(false));
                level.random().next_f32();
                finalize_mob(&mut tag, &mut RandomRef(level.random()));
            }
            "minecraft:slime" => {
                // `AgeableMob.finalizeSpawn` without babies, `Mob`'s part,
                // then `AbstractCubeMob.setSpawnSize`.
                if matches!(group, MonsterGroup::None) {
                    *group = MonsterGroup::Ageable { should_spawn_baby: false, chance: 0.05, size: 0 };
                }
                Self::ageable_monster(level, &mut tag, group);
                finalize_mob(&mut tag, &mut RandomRef(level.random()));
                let mut scale = level.random().next_i32_bound(3);
                if scale < 2 && level.random().next_f32() < 0.5 * special {
                    scale += 1;
                }
                Self::set_cube_size(&mut tag, 1 << scale);
                // A size other than the default 1 changes the synced size:
                // `onSyncedDataUpdated` turns the cube to its head yaw, the
                // constructor's roll from its own unseeded random.
                if scale > 0 {
                    let yaw = own.next_f32() * std::f64::consts::TAU as f32;
                    set(&mut tag, "Rotation", Tag::List(vec![Tag::Float(yaw), Tag::Float(0.0)]));
                }
            }
            "minecraft:zombie_horse" => {
                // A zombie rider with an iron spear, finalized first.
                if let Some(mut zombie) = self.registries.entities.as_ref().and_then(|c| c.default_tag("minecraft:zombie")) {
                    let zombie_uuid = level.next_uuid();
                    place(&mut zombie, Self::position(&tag), Self::y_rot(&tag), 0.0, zombie_uuid);
                    let mut rider_random = entity_random(zombie_uuid);
                    let _ = attribute(&mut zombie, "minecraft:follow_range", 35.0);
                    finalize_mob(&mut zombie, &mut RandomRef(level.random()));
                    let mut rider_group = MonsterGroup::None;
                    let mut zombie = self.finalize_zombie(level, context, zombie, pos, difficulty, &mut rider_group, &mut rider_random);
                    if zombie.get("id").and_then(Tag::as_str) == Some("minecraft:chicken") {
                        level.note_unsupported("chicken jockey on a zombie horse");
                    }
                    equip(&mut zombie, "mainhand", "minecraft:iron_spear");
                    set(&mut tag, "Passengers", Tag::List(vec![zombie]));
                }
                // `AbstractHorse.finalizeSpawn`: the horse's attributes, then
                // `AgeableMob` and `Mob`.
                if matches!(group, MonsterGroup::None) {
                    *group = MonsterGroup::Ageable { should_spawn_baby: true, chance: 0.2, size: 0 };
                }
                let random = level.random();
                let third = 0.066_666_666_666_666_67;
                let jump = 0.5 + random.next_f64() * third + random.next_f64() * third + random.next_f64() * third;
                let speed = (9.0 + random.next_f64() * 1.0 + random.next_f64() * 1.0 + random.next_f64() * 1.0) / f64::from(42.16f32);
                set_attribute_base(&mut tag, "minecraft:jump_strength", jump);
                set_attribute_base(&mut tag, "minecraft:movement_speed", speed);
                Self::ageable_monster(level, &mut tag, group);
                finalize_mob(&mut tag, &mut RandomRef(level.random()));
            }
            "minecraft:creeper" | "minecraft:enderman" | "minecraft:bat" => finalize_mob(&mut tag, &mut RandomRef(level.random())),
            _ => {
                level.note_unsupported(&format!("finalizeSpawn of {kind}"));
                finalize_mob(&mut tag, &mut RandomRef(level.random()));
            }
        }
        tag
    }

    /// `AgeableMob.finalizeSpawn` for the ageable monsters.
    fn ageable_monster(level: &mut dyn SpawnLevel, tag: &mut Tag, group: &mut MonsterGroup) {
        if let MonsterGroup::Ageable { should_spawn_baby, chance, size } = group {
            if *should_spawn_baby && *size > 0 && level.random().next_f32() <= *chance {
                set(tag, "Age", Tag::Int(-24000));
            }
            *size += 1;
        }
    }

    /// `AbstractCubeMob.setSize(size, true)`: attributes by size, full health.
    fn set_cube_size(tag: &mut Tag, size: i32) {
        set(tag, "Size", Tag::Int(size - 1));
        let health = f64::from(size * size);
        set_attribute_base(tag, "minecraft:max_health", health);
        set_attribute_base(tag, "minecraft:movement_speed", f64::from(0.2f32 + 0.1 * size as f32));
        set_attribute_base(tag, "minecraft:attack_damage", f64::from(size));
        set(tag, "Health", Tag::Float(health as f32));
    }

    /// `AbstractSkeleton.finalizeSpawn` after `Mob`'s part.
    fn finalize_skeleton(&self, level: &mut dyn SpawnLevel, context: &SpawnContext, tag: &mut Tag, difficulty: DifficultyInstance) {
        Self::populate_armor(level, tag, difficulty);
        equip(tag, "mainhand", "minecraft:bow");
        Self::roll_enchantments(level, tag, difficulty);
        let loot = level.random().next_f32() < 0.55 * difficulty.special_multiplier();
        set(tag, "CanPickUpLoot", byte(loot));
        Self::halloween_head(level, context, tag);
    }

    /// `Zombie.finalizeSpawn` after `Mob`'s part: the entity to add is the
    /// zombie, or a new chicken it rides.
    #[allow(clippy::too_many_arguments)]
    fn finalize_zombie(
        &self,
        level: &mut dyn SpawnLevel,
        context: &SpawnContext,
        mut tag: Tag,
        pos: BlockPos,
        difficulty: DifficultyInstance,
        group: &mut MonsterGroup,
        own: &mut LegacyRandom,
    ) -> Tag {
        let special = difficulty.special_multiplier();
        let loot = level.random().next_f32() < 0.55 * special;
        set(&mut tag, "CanPickUpLoot", byte(loot));
        if matches!(group, MonsterGroup::None) {
            // `getSpawnAsBabyOdds`.
            let baby = level.random().next_f32() < 0.05;
            *group = MonsterGroup::Zombie { baby, can_spawn_jockey: true };
        }
        let mut vehicle = None;
        if let MonsterGroup::Zombie { baby, can_spawn_jockey } = *group {
            if baby {
                set(&mut tag, "IsBaby", byte(true));
                // `setBaby` touches the speed (the baby modifier is transient).
                let _ = attribute(&mut tag, "minecraft:movement_speed", f64::from(0.23f32));
                if can_spawn_jockey {
                    if f64::from(level.random().next_f32()) < 0.05 {
                        let at = Self::position(&tag);
                        let half = f64::from(0.49f32 / 2.0);
                        let search = [at[0] - half - 5.0, at[1] - 3.0, at[2] - half - 5.0, at[0] + half + 5.0, at[1] + f64::from(0.98f32) + 3.0, at[2] + half + 5.0];
                        if context.chickens.iter().any(|c| overlaps(&search, c)) {
                            level.note_unsupported("baby zombie riding a nearby chicken");
                        }
                    } else if f64::from(level.random().next_f32()) < 0.05 {
                        vehicle = self.jockey_chicken(level, &tag, pos);
                    }
                }
            }
            let doors = level.random().next_f32() < special * 0.1;
            set(&mut tag, "CanBreakDoors", byte(doors));
            Self::populate_armor(level, &mut tag, difficulty);
            // `Zombie.populateDefaultEquipmentSlots`: a rare iron weapon.
            let weapon_chance = if difficulty.base == 3 { 0.05 } else { 0.01 };
            if level.random().next_f32() < weapon_chance {
                let item = match level.random().next_i32_bound(6) {
                    0 => "minecraft:iron_sword",
                    1 => "minecraft:iron_spear",
                    _ => "minecraft:iron_shovel",
                };
                equip(&mut tag, "mainhand", item);
            }
            Self::roll_enchantments(level, &tag, difficulty);
        }
        Self::halloween_head(level, context, &mut tag);
        // `handleAttributes`, from the zombie's own random.
        set_attribute_base(&mut tag, "minecraft:spawn_reinforcements", own.next_f64() * f64::from(0.1f32));
        add_modifier(&mut tag, "minecraft:knockback_resistance", 0.0, "minecraft:random_spawn_bonus", own.next_f64() * f64::from(0.05f32), "add_value");
        let follow = own.next_f64() * 1.5 * f64::from(special);
        if follow > 1.0 {
            add_modifier(&mut tag, "minecraft:follow_range", 35.0, "minecraft:zombie_random_spawn_bonus", follow, "add_multiplied_total");
        }
        if own.next_f32() < special * 0.05 {
            add_modifier(&mut tag, "minecraft:spawn_reinforcements", 0.0, "minecraft:leader_zombie_bonus", own.next_f64() * 0.25 + 0.5, "add_value");
            let bonus = own.next_f64() * 3.0 + 1.0;
            add_modifier(&mut tag, "minecraft:max_health", 20.0, "minecraft:leader_zombie_bonus", bonus, "add_multiplied_total");
            set(&mut tag, "Health", Tag::Float((20.0 * (1.0 + bonus)).clamp(1.0, 1024.0) as f32));
            set(&mut tag, "CanBreakDoors", byte(true));
        }
        match vehicle {
            Some(mut chicken) => {
                set(&mut chicken, "Passengers", Tag::List(vec![tag]));
                chicken
            }
            None => tag,
        }
    }

    /// `Husk.finalizeSpawn` after the zombie's part: a second loot roll,
    /// then, for a natural spawn with room for one, a one-in-ten camel
    /// husk the husk rides with an iron spear, a parched riding behind
    /// (each finalized as a spawn of its own).
    fn finalize_husk(&self, level: &mut dyn SpawnLevel, context: &SpawnContext, mut root: Tag, pos: BlockPos, difficulty: DifficultyInstance, natural: bool) -> Tag {
        let loot = level.random().next_f32() < 0.55 * difficulty.special_multiplier();
        let on_its_own = root.get("id").and_then(Tag::as_str) == Some("minecraft:husk");
        if !on_its_own {
            // A baby husk on a chicken: the roll lands on the rider.
            if let Some(Tag::List(riders)) = root.get("Passengers").cloned() {
                if let Some(mut husk) = riders.into_iter().next() {
                    set(&mut husk, "CanPickUpLoot", byte(loot));
                    set(&mut root, "Passengers", Tag::List(vec![husk]));
                }
            }
            level.note_unsupported("camel husk for a husk jockey");
            return root;
        }
        set(&mut root, "CanPickUpLoot", byte(loot));
        if !natural {
            return root;
        }
        let at = Self::position(&root);
        let (x, z) = (f64::from(pos.x) + 0.5, f64::from(pos.z) + 0.5);
        let Some(camel_info) = type_info("minecraft:camel_husk") else { return root };
        if !self.no_collision(level, spawn_aabb(&camel_info, x, f64::from(pos.y), z)) {
            return root;
        }
        if level.random().next_f32() >= 0.1 {
            return root;
        }
        let catalog = self.registries.entities.as_ref();
        let (Some(mut camel), Some(mut parched)) = (catalog.and_then(|c| c.default_tag("minecraft:camel_husk")), catalog.and_then(|c| c.default_tag("minecraft:parched"))) else {
            return root;
        };
        equip(&mut root, "mainhand", "minecraft:iron_spear");
        // `Camel.finalizeSpawn`: no memories to draw, then `AbstractHorse`
        // (no attributes to roll), `AgeableMob` (a first-of-group, no baby
        // roll) and `Mob`. The camel is only `setPos`: it keeps its
        // constructor's yaw (its own unseeded random), and its riders take
        // its previous-tick rotation, still 0 (`AbstractHorse.addPassenger`'s
        // `absSnapRotationTo(getViewYRot(0))`).
        let camel_uuid = level.next_uuid();
        let camel_yaw = entity_random(camel_uuid).next_f32() * std::f64::consts::TAU as f32;
        place(&mut camel, at, camel_yaw, 0.0, camel_uuid);
        set(&mut root, "Rotation", Tag::List(vec![Tag::Float(0.0), Tag::Float(0.0)]));
        let _ = attribute(&mut camel, "minecraft:follow_range", 16.0);
        finalize_mob(&mut camel, &mut RandomRef(level.random()));
        // The parched: a skeleton's finalize.
        let parched_uuid = level.next_uuid();
        place(&mut parched, at, 0.0, 0.0, parched_uuid);
        let _ = attribute(&mut parched, "minecraft:follow_range", 16.0);
        finalize_mob(&mut parched, &mut RandomRef(level.random()));
        self.finalize_skeleton(level, context, &mut parched, difficulty);
        set(&mut camel, "Passengers", Tag::List(vec![root, parched]));
        camel
    }

    /// A chicken jockey's new chicken at the zombie, finalized as a spawn
    /// of its own.
    fn jockey_chicken(&self, level: &mut dyn SpawnLevel, zombie: &Tag, pos: BlockPos) -> Option<Tag> {
        let mut chicken = self.registries.entities.as_ref().and_then(|c| c.default_tag("minecraft:chicken"))?;
        let uuid = level.next_uuid();
        place(&mut chicken, Self::position(zombie), Self::y_rot(zombie), 0.0, uuid);
        self.finalize_spawn(level, "minecraft:chicken", &mut chicken, pos, &mut Group::None);
        set(&mut chicken, "IsChickenJockey", byte(true));
        Some(chicken)
    }

    fn uuid(tag: &Tag) -> [i32; 4] {
        match tag.get("UUID") {
            Some(Tag::IntArray(v)) if v.len() == 4 => [v[0], v[1], v[2], v[3]],
            _ => [0; 4],
        }
    }

    fn position(tag: &Tag) -> [f64; 3] {
        let at = |i: usize| match tag.get("Pos") {
            Some(Tag::List(p)) => match p.get(i) {
                Some(Tag::Double(v)) => *v,
                _ => 0.0,
            },
            _ => 0.0,
        };
        [at(0), at(1), at(2)]
    }

    fn y_rot(tag: &Tag) -> f32 {
        match tag.get("Rotation") {
            Some(Tag::List(r)) => match r.first() {
                Some(Tag::Float(v)) => *v,
                _ => 0.0,
            },
            _ => 0.0,
        }
    }

    fn biome_name(&self, biome: BiomeId) -> String {
        self.registries.biomes.get(biome).name.as_str().to_owned()
    }
}
