//! The integrated server's mobs. Entities generation created (structure
//! mobs and the SPAWN step's creatures, saved as `ProtoChunk.getEntities`
//! keeps them) join the entity world when their chunk arrives, as vanilla
//! loads a FULL chunk's proto entities (`EntityType.loadEntitiesRecursive`),
//! and leave with it. They tick only inside the entity-ticking range.

use crate::scene::Block;
use crate::terrain::BlockStates;
use glam::DVec3;
use minecraftoss_entities::zombie::ZombieKind;
use minecraftoss_core::nbt::Tag;
use minecraftoss_core::{BlockStateId, ChunkPos};
use minecraftoss_entities::chicken::{Chicken, ChickenSoundVariant, ChickenVariant};
use minecraftoss_entities::cow::{Cow, CowSoundVariant, CowVariant};
use minecraftoss_entities::mooshroom::{MushroomCow, MushroomVariant};
use minecraftoss_entities::pig::{Pig, PigSoundVariant, PigVariant};
use minecraftoss_entities::sheep::Sheep;
use minecraftoss_entities::villager::Villager;
use minecraftoss_entities::world::EntityWorld;
use minecraftoss_player::rng::LegacyRandom;
use minecraftoss_core::block::{flags, FluidKind};
use minecraftoss_core::tags::TagId;
use minecraftoss_core::Registries;
use minecraftoss_player::path_type::PathType;
use minecraftoss_player::{Block as PlayerBlock, World as PlayerWorld};
use std::cell::RefCell;
use minecraftoss_world::level::{update, Level};

/// Per block state, what mobs look up often: the name form (none for air)
/// and the walk path type.
pub struct MobTables {
    pub names: Vec<Option<PlayerBlock>>,
    pub path_types: Vec<PathType>,
    /// `PoiTypes.forState`.
    pub pois: Vec<Option<minecraftoss_entities::poi::PoiType>>,
}

impl MobTables {
    pub fn new(level: &Level<'static>, states: &BlockStates) -> Self {
        let registries = level.registries();
        let count = registries.blocks.state_count();
        let tags = PathTags::new(registries);
        let names: Vec<Option<PlayerBlock>> = (0..count).map(|i| name_of(level, states, BlockStateId(i as u16))).collect();
        let pois = names.iter().map(|b| b.as_ref().and_then(minecraftoss_entities::poi::PoiType::of_block)).collect();
        Self {
            names,
            path_types: (0..count).map(|i| path_type_of_state(registries, &tags, BlockStateId(i as u16))).collect(),
            pois,
        }
    }
}

/// The block tags `getPathTypeFromState` consults.
struct PathTags {
    trapdoors: Option<TagId>,
    speleothems: Option<TagId>,
    fire: Option<TagId>,
    campfires: Option<TagId>,
    fences: Option<TagId>,
    walls: Option<TagId>,
}

impl PathTags {
    fn new(registries: &Registries) -> Self {
        let tag = |name: &str| registries.block_tags.id(name);
        Self {
            trapdoors: tag("minecraft:trapdoors"),
            speleothems: tag("minecraft:speleothems"),
            fire: tag("minecraft:fire"),
            campfires: tag("minecraft:campfires"),
            fences: tag("minecraft:fences"),
            walls: tag("minecraft:walls"),
        }
    }
}

/// `WalkNodeEvaluator.getPathTypeFromState` for a block state.
fn path_type_of_state(registries: &Registries, tags: &PathTags, state: BlockStateId) -> PathType {
    let blocks = &registries.blocks;
    if blocks.is_air(state) {
        return PathType::Open;
    }
    let info = blocks.block(blocks.block_of(state));
    let name = info.name.as_str();
    let is = |tag: Option<TagId>| tag.is_some_and(|t| registries.block_in_tag(state, t));
    let class = |wanted: &str| info.classes().iter().any(|c| &**c == wanted);
    let prop = |key: &str| blocks.property(state, key) == Some("true");
    if is(tags.trapdoors) || name == "minecraft:lily_pad" || name == "minecraft:big_dripleaf" {
        return PathType::Trapdoor;
    }
    match name {
        "minecraft:powder_snow" => return PathType::PowderSnow,
        "minecraft:cactus" | "minecraft:sweet_berry_bush" => return PathType::Damaging,
        "minecraft:honey_block" => return PathType::StickyHoney,
        "minecraft:cocoa" => return PathType::Cocoa,
        _ => {}
    }
    if name == "minecraft:wither_rose" || is(tags.speleothems) {
        return PathType::DamageCautious;
    }
    let fluid = blocks.state(state).fluid.map(|f| f.kind);
    if fluid == Some(FluidKind::Lava) {
        return PathType::Lava;
    }
    // `NodeEvaluator.isBurningBlock`.
    let lit_campfire = is(tags.campfires) && prop("lit");
    if is(tags.fire) || matches!(name, "minecraft:lava" | "minecraft:magma_block" | "minecraft:lava_cauldron") || lit_campfire {
        return PathType::Fire;
    }
    if class("DoorBlock") {
        return if prop("open") {
            PathType::DoorOpen
        } else if name == "minecraft:iron_door" {
            PathType::DoorIronClosed
        } else {
            PathType::DoorWoodClosed
        };
    }
    if class("BaseRailBlock") {
        return PathType::Rail;
    }
    if class("LeavesBlock") {
        return PathType::Leaves;
    }
    if is(tags.fences) || is(tags.walls) || (class("FenceGateBlock") && !prop("open")) {
        return PathType::Fence;
    }
    if !blocks.is(state, flags::PATHFINDABLE_LAND) {
        return PathType::Blocked;
    }
    if fluid == Some(FluidKind::Water) {
        PathType::Water
    } else {
        PathType::Open
    }
}

/// The level as mobs read and write it: blocks by name, air as none, and
/// the catalog's shapes, path types and flags. The level sits in a cell
/// because reading light may solve it.
pub struct MobWorld<'a> {
    pub level: RefCell<&'a mut Level<'static>>,
    pub states: &'a BlockStates,
    pub tables: &'a MobTables,
}

impl MobWorld<'_> {
    fn state(&self, (x, y, z): (i32, i32, i32)) -> BlockStateId {
        self.level.borrow().block(minecraftoss_core::BlockPos::new(x, y, z))
    }
}

/// A level stack's components as JSON: the JSON text the server keeps for
/// stacks it made, or saved NBT.
fn stack_components(stack: &minecraftoss_core::item::ItemStack) -> Option<serde_json::Value> {
    match stack.components.as_ref()? {
        Tag::String(json) => serde_json::from_str(json).ok(),
        tag => Some(tag_json(tag)),
    }
}

fn world_item(id: i32, pos: [f64; 3], stack: &minecraftoss_core::item::ItemStack, pickup_delay: i32) -> minecraftoss_player::WorldItem {
    minecraftoss_player::WorldItem { id, position: DVec3::from_array(pos), item: stack.id.clone(), count: stack.count, components: stack_components(stack), pickup_delay }
}

impl PlayerWorld for MobWorld<'_> {
    fn items_in(&self, min: DVec3, max: DVec3) -> Vec<minecraftoss_player::WorldItem> {
        self.level.borrow().items_touching(min.to_array(), max.to_array()).iter().map(|(id, pos, stack, delay)| world_item(*id, *pos, stack, *delay)).collect()
    }

    fn item(&self, id: i32) -> Option<minecraftoss_player::WorldItem> {
        self.level.borrow().item_entity(id).map(|(pos, stack, delay)| world_item(id, pos, &stack, delay))
    }

    fn take_item(&mut self, id: i32, count: i32) {
        self.level.borrow_mut().take_item(id, count);
    }

    fn spawn_item(&mut self, position: DVec3, stack: &minecraftoss_player::inventory::ItemStack, velocity: DVec3, pickup_delay: i32) {
        let mut level_stack = minecraftoss_core::item::ItemStack::new(&stack.id, i32::from(stack.count));
        level_stack.components = stack.components.as_ref().map(|c| Tag::String(c.to_string()));
        self.level.borrow_mut().spawn_item_with(position.to_array(), level_stack, velocity.to_array(), pickup_delay, 0);
    }

    fn schedule_block_tick(&mut self, (x, y, z): (i32, i32, i32), delay: i32) {
        let pos = minecraftoss_core::BlockPos::new(x, y, z);
        let mut level = self.level.borrow_mut();
        let state = level.block(pos);
        level.schedule_block_tick(pos, state, delay);
    }

    fn trample_farmland(&mut self, (x, y, z): (i32, i32, i32), random: &mut minecraftoss_player::rng::LegacyRandom) {
        let Ok(id) = crate::pack::ResourceId::parse("minecraft:dirt") else { return };
        let Some(dirt) = self.states.state_of(&Block { id, properties: Default::default() }) else { return };
        let mut level = self.level.borrow_mut();
        // The level's own updates break what grew there, drawing from the
        // level random where the entity world has it.
        let gaussian = match &mut level.random {
            minecraftoss_core::random::AnyRandom::Legacy(r) => {
                let gaussian = r.gaussian_cache();
                r.set_seed((random.raw_state() ^ 0x5DEE_CE66D) as i64);
                Some(gaussian)
            }
            _ => None,
        };
        level.set_block(minecraftoss_core::BlockPos::new(x, y, z), dirt, update::ALL, update::LIMIT);
        if let (Some(gaussian), minecraftoss_core::random::AnyRandom::Legacy(r)) = (gaussian, &mut level.random) {
            *random = minecraftoss_player::rng::LegacyRandom::from_raw_state(r.state());
            r.set_gaussian_cache(gaussian);
        }
    }

    fn spawn_popped_item(&mut self, position: DVec3, stack: &minecraftoss_player::inventory::ItemStack) {
        let mut level_stack = minecraftoss_core::item::ItemStack::new(&stack.id, i32::from(stack.count));
        level_stack.components = stack.components.as_ref().map(|c| Tag::String(c.to_string()));
        self.level.borrow_mut().spawn_popped_item(position.to_array(), level_stack);
    }

    fn block_drops(&self, (x, y, z): (i32, i32, i32), block: &PlayerBlock) -> Vec<minecraftoss_player::inventory::ItemStack> {
        let Ok(id) = crate::pack::ResourceId::parse(&block.id) else { return Vec::new() };
        let Some(state) = self.states.state_of(&Block { id, properties: block.properties.clone() }) else { return Vec::new() };
        let mut level = self.level.borrow_mut();
        let drops = level.mob_block_drops(state, minecraftoss_core::BlockPos::new(x, y, z));
        drops
            .into_iter()
            .map(|s| {
                let max = level.lib.registries.items.max_stack(&s.id).clamp(1, 99) as u8;
                minecraftoss_player::inventory::ItemStack { id: s.id.clone(), count: s.count.clamp(0, 255) as u8, max, components: stack_components(&s) }
            })
            .collect()
    }

    fn biome(&self, (x, y, z): (i32, i32, i32)) -> Option<String> {
        let level = self.level.borrow();
        let biome = minecraftoss_generator::feature::World::biome(&**level, x, y, z)?;
        Some(level.lib.registries.biomes.get(biome).name.as_str().to_owned())
    }

    fn block(&self, pos: (i32, i32, i32)) -> Option<PlayerBlock> {
        self.tables.names.get(usize::from(self.state(pos).0)).cloned().flatten()
    }

    fn set_block(&mut self, (x, y, z): (i32, i32, i32), block: Option<PlayerBlock>) {
        let state = match block {
            None => BlockStateId::AIR,
            Some(block) => {
                let Ok(id) = crate::pack::ResourceId::parse(&block.id) else { return };
                let Some(state) = self.states.state_of(&Block { id, properties: block.properties }) else { return };
                state
            }
        };
        self.level.borrow_mut().set_block(minecraftoss_core::BlockPos::new(x, y, z), state, update::ALL, update::LIMIT);
    }

    fn collision_boxes(&self, pos: (i32, i32, i32)) -> Vec<[f64; 6]> {
        let state = self.state(pos);
        self.level.borrow().registries().blocks.collision_boxes(state)
    }

    fn path_type_from_state(&self, pos: (i32, i32, i32)) -> PathType {
        self.tables.path_types.get(usize::from(self.state(pos).0)).copied().unwrap_or(PathType::Blocked)
    }

    fn floatable_fluid(&self, pos: (i32, i32, i32)) -> bool {
        let state = self.state(pos);
        self.level.borrow().registries().blocks.state(state).fluid.is_some_and(|f| f.kind == FluidKind::Water)
    }

    fn pathfindable(&self, pos: (i32, i32, i32)) -> bool {
        let state = self.state(pos);
        self.level.borrow().registries().blocks.is(state, flags::PATHFINDABLE_LAND)
    }

    fn solid(&self, pos: (i32, i32, i32)) -> bool {
        let state = self.state(pos);
        self.level.borrow().registries().blocks.is(state, flags::LEGACY_SOLID)
    }

    /// `BlockState.isSuffocating`, from the catalog.
    fn suffocating(&self, pos: (i32, i32, i32)) -> bool {
        let state = self.state(pos);
        self.level.borrow().registries().blocks.state(state).suffocating
    }

    fn solid_render(&self, pos: (i32, i32, i32)) -> bool {
        let state = self.state(pos);
        self.level.borrow().registries().blocks.is(state, flags::SOLID_RENDER)
    }

    fn difficulty_inputs(&self, (x, _, z): (i32, i32, i32)) -> (i64, i64, f32) {
        self.level.borrow().difficulty_inputs(x, z)
    }

    fn sturdy_underside(&self, pos: (i32, i32, i32)) -> bool {
        let state = self.state(pos);
        self.level.borrow().registries().blocks.is_face_sturdy(state, minecraftoss_core::pos::Direction::Down, minecraftoss_core::block::SupportType::Full)
    }

    /// The block's fall sound: the catalog's sound type, else the step's
    /// family.
    fn fall_sound(&self, pos: (i32, i32, i32)) -> Option<(String, f32, f32)> {
        let state = self.state(pos);
        if let Some(sound) = self.level.borrow().registries().blocks.sound_type(state) {
            return Some((sound.fall.to_string(), sound.volume, sound.pitch));
        }
        let (step, volume, pitch) = self.step_sound(pos)?;
        Some((format!("{}.fall", step.strip_suffix(".step")?), volume, pitch))
    }

    /// The block's step sound: the catalog's sound type, else its measured
    /// sound type, else the family its name suggests at volume and pitch 1.
    fn step_sound(&self, pos: (i32, i32, i32)) -> Option<(String, f32, f32)> {
        let state = self.state(pos);
        if let Some(sound) = self.level.borrow().registries().blocks.sound_type(state) {
            return Some((sound.step.to_string(), sound.volume, sound.pitch));
        }
        let block = self.block(pos)?;
        let (family, volume, pitch) = match crate::audio::measured_block_sound_profile(&block.id) {
            Some(profile) => (profile.family, profile.volume, profile.pitch),
            None => (crate::audio::block_sound_family(&block.id), 1.0, 1.0),
        };
        Some((format!("block.{family}.step"), volume, pitch))
    }

    /// `getLightLevelDependentMagicValue - 0.5`: the raw brightness (sky
    /// light less the sky's darkening, or block light) on vanilla's curve,
    /// lifted by the dimension's ambient light.
    fn can_see_sky(&self, (x, y, z): (i32, i32, i32)) -> bool {
        self.level.borrow_mut().sky_light(minecraftoss_core::BlockPos::new(x, y, z)) >= 15
    }

    fn rain_at(&self, (x, y, z): (i32, i32, i32)) -> bool {
        self.level.borrow_mut().is_raining_at(minecraftoss_core::BlockPos::new(x, y, z))
    }

    fn light_path_cost(&self, (x, y, z): (i32, i32, i32)) -> f32 {
        let mut level = self.level.borrow_mut();
        let brightness = level.max_local_raw_brightness(minecraftoss_core::BlockPos::new(x, y, z));
        let v = brightness as f32 / 15.0;
        let curved = v / (4.0 - 3.0 * v);
        // `Mth.lerp(ambientLight, curvedV, 1)`.
        let magic = curved + level.ambient_light() * (1.0 - curved);
        magic - 0.5
    }

    fn min_y(&self) -> i32 {
        self.level.borrow().min_y()
    }

    fn max_y(&self) -> i32 {
        self.level.borrow().max_y()
    }
}

fn name_of(level: &Level<'static>, states: &BlockStates, state: BlockStateId) -> Option<PlayerBlock> {
    if level.registries().blocks.is_air(state) {
        return None;
    }
    states.block(state).map(|b| PlayerBlock { id: b.id.key(), properties: b.properties.clone() })
}



fn position(tag: &Tag) -> Option<DVec3> {
    let pos = tag.get("Pos")?.as_list()?;
    Some(DVec3::new(pos.first()?.as_f64()?, pos.get(1)?.as_f64()?, pos.get(2)?.as_f64()?))
}

fn yaw(tag: &Tag) -> f32 {
    tag.get("Rotation").and_then(Tag::as_list).and_then(|r| r.first()).and_then(Tag::as_f64).unwrap_or(0.0) as f32
}

fn int(tag: &Tag, key: &str) -> Option<i32> {
    tag.get(key).and_then(Tag::as_i64).map(|v| v as i32)
}

fn text<'a>(tag: &'a Tag, key: &str) -> Option<&'a str> {
    tag.get(key).and_then(Tag::as_str)
}

/// A seed for the entity's own random from its UUID: vanilla seeds entity
/// randoms from the clock, so any seed is as good; this one repeats.
fn seed(tag: &Tag) -> u64 {
    let uuid = tag.get("UUID").and_then(Tag::as_ints).unwrap_or_default();
    uuid.iter().fold(0x9E37_79B9_7F4A_7C15u64, |h, &w| (h ^ u64::from(w as u32)).wrapping_mul(0x100_0000_01B3))
}

/// Adds a saved entity the entity world simulates; its ID, or none for a
/// type it does not simulate yet.
pub fn spawn_saved(world: &mut EntityWorld, tag: &Tag) -> Option<u64> {
    let kind = text(tag, "id")?;
    let pos = position(tag)?;
    let yaw = yaw(tag);
    let age = int(tag, "Age").unwrap_or(0);
    let health = tag.get("Health").and_then(Tag::as_f64).map(|h| h as f32);
    let persistent = int(tag, "PersistenceRequired").unwrap_or(0) != 0;
    let random = LegacyRandom::new(seed(tag));
    let variant = text(tag, "variant").unwrap_or("minecraft:temperate");
    let fire = tag.get("Fire").and_then(Tag::as_i64).map_or(0, |f| f as i32);
    // `Mob.readAdditionalSaveData`: a mob without AI stands where it is.
    let no_ai = int(tag, "NoAI").unwrap_or(0) != 0;
    let id = match kind {
        "minecraft:cow" | "minecraft:mooshroom" => {
            let mut cow = Cow::new(pos);
            cow.yaw = yaw;
            cow.age.set(age);
            cow.persistence_required = persistent;
            if let Some(health) = health {
                cow.health = health;
            }
            cow.variant = match variant {
                "minecraft:warm" => CowVariant::Warm,
                "minecraft:cold" => CowVariant::Cold,
                _ => CowVariant::Temperate,
            };
            cow.sound_variant = if text(tag, "sound_variant") == Some("minecraft:moody") { CowSoundVariant::Moody } else { CowSoundVariant::Classic };
            let id = if kind == "minecraft:mooshroom" {
                let mushroom = if text(tag, "Type") == Some("brown") { MushroomVariant::Brown } else { MushroomVariant::Red };
                let mut mooshroom = MushroomCow::new(pos, mushroom);
                mooshroom.cow = cow;
                world.spawn_mooshroom(mooshroom, no_ai)
            } else {
                world.spawn_cow(cow, no_ai)
            };
            world.cow_mut(id)?.random = random;
            id
        }
        "minecraft:horse" | "minecraft:donkey" => {
            use minecraftoss_entities::horse::{HorseKind, HorseState};
            let mut cow = Cow::new(pos);
            cow.yaw = yaw;
            cow.age.set(age);
            cow.persistence_required = persistent;
            if let Some(health) = health {
                cow.health = health;
            }
            // `AbstractHorse.readAdditionalSaveData` (and `Horse`'s variant,
            // `AbstractChestedHorse`'s chest); its attributes as saved.
            let mut horse = HorseState::new(if kind == "minecraft:horse" { HorseKind::Horse } else { HorseKind::Donkey });
            horse.variant = int(tag, "Variant").unwrap_or(0);
            horse.chest = int(tag, "ChestedHorse").unwrap_or(0) != 0;
            horse.tame = int(tag, "Tame").unwrap_or(0) != 0;
            horse.bred = int(tag, "Bred").unwrap_or(0) != 0;
            horse.temper = int(tag, "Temper").unwrap_or(0);
            horse.eating = int(tag, "EatingHaystack").unwrap_or(0) != 0;
            if let Some(max_health) = attribute_base(tag, "minecraft:max_health") {
                horse.max_health = max_health as f32;
            }
            if let Some(speed) = attribute_base(tag, "minecraft:movement_speed") {
                horse.movement_speed = speed;
            }
            if let Some(jump) = attribute_base(tag, "minecraft:jump_strength") {
                horse.jump_strength = jump;
            }
            let id = world.spawn_horse(cow, horse, no_ai);
            world.cow_mut(id)?.random = random;
            id
        }
        "minecraft:pig" => {
            let mut pig = Pig::new(pos);
            pig.yaw = yaw;
            pig.age.set(age);
            pig.persistence_required = persistent;
            if let Some(health) = health {
                pig.health = health;
            }
            pig.variant = match variant {
                "minecraft:warm" => PigVariant::Warm,
                "minecraft:cold" => PigVariant::Cold,
                _ => PigVariant::Temperate,
            };
            pig.sound_variant = match text(tag, "sound_variant") {
                Some("minecraft:mini") => PigSoundVariant::Mini,
                Some("minecraft:big") => PigSoundVariant::Big,
                _ => PigSoundVariant::Classic,
            };
            // `Mob.equipment`'s saddle slot.
            pig.saddled = tag.get("equipment").and_then(|e| e.get("saddle")).and_then(|s| s.get("id")).and_then(Tag::as_str) == Some("minecraft:saddle");
            let id = world.spawn_pig(pig, no_ai);
            world.pig_mut(id)?.random = random;
            id
        }
        "minecraft:chicken" => {
            let mut chicken = Chicken::new(pos);
            chicken.yaw = yaw;
            chicken.age.set(age);
            chicken.persistence_required = persistent;
            if let Some(health) = health {
                chicken.health = health;
            }
            chicken.variant = match variant {
                "minecraft:warm" => ChickenVariant::Warm,
                "minecraft:cold" => ChickenVariant::Cold,
                _ => ChickenVariant::Temperate,
            };
            chicken.sound_variant = if text(tag, "sound_variant") == Some("minecraft:picky") { ChickenSoundVariant::Picky } else { ChickenSoundVariant::Classic };
            if let Some(egg_time) = int(tag, "EggLayTime") {
                chicken.egg_time = egg_time;
            }
            let id = world.spawn_chicken(chicken, no_ai);
            world.chicken_mut(id)?.random = random;
            id
        }
        "minecraft:sheep" => {
            let mut sheep = Sheep::default();
            sheep.age.set(age);
            sheep.persistence_required = persistent;
            sheep.wool.set_color((int(tag, "Color").unwrap_or(0) & 15) as u8);
            sheep.wool.set_sheared(int(tag, "Sheared").unwrap_or(0) != 0);
            let id = world.spawn_sheep(sheep, pos, no_ai);
            let entity = world.sheep_mut(id)?;
            entity.random = random;
            // `Entity.load`: the head and body face the saved yaw too.
            entity.yaw = yaw;
            entity.previous_yaw = yaw;
            entity.look_control.head_yaw = yaw;
            entity.body_rotation.body_yaw = yaw;
            if let Some(health) = health {
                entity.health = health;
            }
            id
        }
        // Monsters and bats natural spawning makes. Riders (jockeys) wait
        // dormant with their vehicle.
        "minecraft:zombie" | "minecraft:husk" | "minecraft:zombie_villager" if tag.get("Passengers").is_none() => {
            let mut zombie = minecraftoss_entities::zombie::Zombie::new(pos);
            zombie.kind = match kind {
                "minecraft:husk" => ZombieKind::Husk,
                "minecraft:zombie_villager" => ZombieKind::ZombieVillager,
                _ => ZombieKind::Zombie,
            };
            if zombie.kind == ZombieKind::ZombieVillager {
                let data = tag.get("VillagerData");
                let field = |key: &str, default: &str| data.and_then(|d| text(d, key)).unwrap_or(default).to_owned();
                zombie.villager = Some((field("type", "minecraft:plains"), field("profession", "minecraft:none")));
            }
            zombie.head_item = head_item(tag);
            zombie.main_hand = tag.get("equipment").and_then(|e| e.get("mainhand")).and_then(|i| i.get("id")).and_then(Tag::as_str).map(str::to_owned);
            zombie.armor = armor_slots(tag);
            zombie.set_baby(int(tag, "IsBaby").unwrap_or(0) != 0);
            zombie.can_break_doors = int(tag, "CanBreakDoors").unwrap_or(0) != 0;
            zombie.persistence_required = persistent;
            if let Some(health) = health {
                zombie.health = health;
            }
            let id = if no_ai { world.spawn_zombie(zombie, true) } else { world.spawn_zombie_active(zombie, yaw) };
            let entity = world.zombie_mut(id)?;
            entity.random = random;
            // `Entity.load` turns the body and head to the saved yaw.
            entity.yaw = yaw;
            entity.body_rotation.body_yaw = yaw;
            entity.look_control.head_yaw = yaw;
            if let (Some(range), Some(ai)) = (attribute_value(tag, "minecraft:follow_range"), entity.ai.as_deref_mut()) {
                ai.state.follow_range = range;
            }
            id
        }
        "minecraft:skeleton" | "minecraft:stray" | "minecraft:bogged" | "minecraft:parched" if tag.get("Passengers").is_none() => {
            use minecraftoss_entities::skeleton::SkeletonKind;
            let kind = match kind {
                "minecraft:stray" => SkeletonKind::Stray,
                "minecraft:bogged" => SkeletonKind::Bogged,
                "minecraft:parched" => SkeletonKind::Parched,
                _ => SkeletonKind::Skeleton,
            };
            let mut skeleton = minecraftoss_entities::skeleton::Skeleton::of_kind(kind, pos);
            // `Bogged.readAdditionalSaveData`.
            skeleton.sheared = int(tag, "sheared").unwrap_or(0) != 0;
            skeleton.head_item = head_item(tag);
            // Its bow is the main hand's (a summoned skeleton has none; a
            // natural spawn's `finalizeSpawn` gave it one).
            skeleton.holds_bow = tag.get("equipment").and_then(|e| e.get("mainhand")).and_then(|i| i.get("id")).and_then(Tag::as_str) == Some("minecraft:bow");
            skeleton.armor = armor_slots(tag);
            skeleton.persistence_required = persistent;
            if let Some(health) = health {
                skeleton.health = health;
            }
            let id = if no_ai { world.spawn_skeleton(skeleton, true) } else { world.spawn_skeleton_active(skeleton, yaw) };
            let entity = world.skeleton_mut(id)?;
            entity.random = random;
            entity.yaw = yaw;
            entity.body_rotation.body_yaw = yaw;
            entity.look_control.head_yaw = yaw;
            if let (Some(range), Some(ai)) = (attribute_value(tag, "minecraft:follow_range"), entity.ai.as_deref_mut()) {
                ai.state.follow_range = range;
            }
            id
        }
        "minecraft:creeper" => {
            let mut creeper = minecraftoss_entities::creeper::Creeper::new(pos);
            creeper.yaw = yaw;
            creeper.persistence_required = persistent;
            if let Some(health) = health {
                creeper.health = health;
            }
            creeper.powered = int(tag, "powered").unwrap_or(0) != 0;
            creeper.max_swell = int(tag, "Fuse").unwrap_or(30);
            creeper.explosion_radius = int(tag, "ExplosionRadius").unwrap_or(3);
            creeper.ignited = int(tag, "ignited").unwrap_or(0) != 0;
            let id = world.spawn_creeper(creeper, no_ai);
            let entity = world.creeper_mut(id)?;
            entity.random = random;
            if let Some(range) = attribute_value(tag, "minecraft:follow_range") {
                entity.ai.state.follow_range = range;
            }
            id
        }
        "minecraft:spider" if tag.get("Passengers").is_none() => {
            let mut spider = minecraftoss_entities::spider::Spider::new(pos);
            spider.yaw = yaw;
            spider.persistence_required = persistent;
            if let Some(health) = health {
                spider.health = health;
            }
            let id = world.spawn_spider(spider, no_ai);
            let entity = world.spider_mut(id)?;
            entity.random = random;
            if let Some(range) = attribute_value(tag, "minecraft:follow_range") {
                entity.ai.state.follow_range = range;
            }
            id
        }
        "minecraft:slime" => {
            // `AbstractCubeMob.readAdditionalSaveData`: `Size` is one short
            // of the size, and the landing state carries over.
            let mut slime = minecraftoss_entities::slime::Slime::new(pos, int(tag, "Size").unwrap_or(0) + 1);
            slime.yaw = yaw;
            slime.persistence_required = persistent;
            slime.was_on_ground = int(tag, "wasOnGround").unwrap_or(0) != 0;
            if let Some(health) = health {
                slime.health = health;
            }
            let id = world.spawn_slime(slime, no_ai);
            let entity = world.slime_mut(id)?;
            entity.random = random;
            if let Some(range) = attribute_value(tag, "minecraft:follow_range") {
                entity.ai.state.follow_range = range;
            }
            id
        }
        "minecraft:enderman" => {
            let mut enderman = minecraftoss_entities::enderman::Enderman::new(pos);
            enderman.yaw = yaw;
            enderman.persistence_required = persistent;
            if let Some(health) = health {
                enderman.health = health;
            }
            let id = world.spawn_enderman(enderman, no_ai);
            let entity = world.enderman_mut(id)?;
            entity.random = random;
            if let Some(range) = attribute_value(tag, "minecraft:follow_range") {
                entity.ai.state.follow_range = range;
            }
            // `readAdditionalSaveData`: the carried block (air carries
            // nothing) and the anger's end (who it was angry at is not kept).
            entity.ai.state.enderman.carried = tag.get("carriedBlockState").and_then(|state| {
                let name = text(state, "Name")?;
                (name != "minecraft:air").then(|| {
                    let mut block = PlayerBlock::new(name);
                    if let Some(Tag::Compound(properties)) = state.get("Properties") {
                        for (key, value) in properties {
                            if let Some(value) = value.as_str() {
                                block = block.with(key, value);
                            }
                        }
                    }
                    block
                })
            });
            if let Some(end) = tag.get("anger_end_time").and_then(Tag::as_i64) {
                entity.ai.state.enderman.anger_end_time = end;
            }
            id
        }
        "minecraft:witch" => {
            // A witch keeps no drinking state (`DATA_USING_ITEM` is not
            // saved): it wakes up not drinking.
            let mut witch = minecraftoss_entities::witch::Witch::new(pos);
            witch.yaw = yaw;
            witch.persistence_required = persistent;
            if let Some(health) = health {
                witch.health = health;
            }
            let id = world.spawn_witch(witch, no_ai);
            let entity = world.witch_mut(id)?;
            entity.random = random;
            if let Some(range) = attribute_value(tag, "minecraft:follow_range") {
                entity.ai.state.follow_range = range;
            }
            id
        }
        "minecraft:iron_golem" => {
            // `IronGolem.readAdditionalSaveData`: built by a player or not,
            // and the anger's end (whom it was angry at is not kept).
            let mut golem = minecraftoss_entities::iron_golem::IronGolem::new(pos);
            golem.yaw = yaw;
            golem.persistence_required = persistent;
            golem.player_created = int(tag, "PlayerCreated").unwrap_or(0) != 0;
            if let Some(health) = health {
                golem.health = health;
            }
            let id = world.spawn_iron_golem(golem, no_ai);
            let entity = world.iron_golem_mut(id)?;
            entity.random = random;
            if let Some(end) = tag.get("anger_end_time").and_then(Tag::as_i64) {
                entity.ai.state.enderman.anger_end_time = end;
            }
            id
        }
        "minecraft:wolf" => {
            // `Wolf.readAdditionalSaveData` and `TamableAnimal`'s: variant,
            // sound variant, collar, owner (tame with one), sitting, and the
            // anger's end (whom it was angry at is not kept yet).
            let mut wolf = minecraftoss_entities::wolf::Wolf::new(pos);
            wolf.yaw = yaw;
            wolf.persistence_required = persistent;
            wolf.set_age(age);
            let short = |key: &str, default: &str| text(tag, key).map_or(default, |v| v.trim_start_matches("minecraft:")).to_owned();
            wolf.variant = short("variant", "pale");
            wolf.sound_variant = short("sound_variant", "classic");
            wolf.collar = int(tag, "CollarColor").map_or(minecraftoss_entities::wolf::DEFAULT_COLLAR, |c| c as u8);
            wolf.owner = tag.get("Owner").and_then(Tag::as_ints).and_then(|ints| <[i32; 4]>::try_from(ints).ok()).map(minecraftoss_entities::gossip::uuid_from_ints);
            wolf.tame = wolf.owner.is_some();
            wolf.ordered_to_sit = int(tag, "Sitting").unwrap_or(0) != 0;
            wolf.sitting = wolf.ordered_to_sit;
            wolf.in_love = int(tag, "InLove").unwrap_or(0);
            // Taming raised the attribute's base to 40 (loading tame does not).
            wolf.max_health_base = attribute_base(tag, "minecraft:max_health").map_or(minecraftoss_entities::wolf::MAX_HEALTH, |b| b as f32);
            if let Some(health) = health {
                wolf.health = health;
            }
            let id = world.spawn_wolf(wolf, no_ai);
            let entity = world.wolf_mut(id)?;
            entity.random = random;
            if let Some(end) = tag.get("anger_end_time").and_then(Tag::as_i64) {
                entity.ai.state.enderman.anger_end_time = end;
            }
            if let Some(range) = attribute_value(tag, "minecraft:follow_range") {
                entity.ai.state.follow_range = range;
            }
            id
        }
        "minecraft:splash_potion" => {
            // A potion in flight (`ThrowableItemProjectile.Item`), its thrower
            // forgotten.
            let potion = tag
                .get("Item")
                .and_then(|item| item.get("components"))
                .and_then(|c| c.get("minecraft:potion_contents"))
                .and_then(|c| c.get("potion").or(Some(c)))
                .and_then(Tag::as_str)
                .and_then(minecraftoss_entities::potion::Potion::from_id)?;
            let motion = tag.get("Motion").and_then(Tag::as_list).map(|m| DVec3::new(m.first().and_then(Tag::as_f64).unwrap_or(0.0), m.get(1).and_then(Tag::as_f64).unwrap_or(0.0), m.get(2).and_then(Tag::as_f64).unwrap_or(0.0))).unwrap_or(DVec3::ZERO);
            let mut thrown = minecraftoss_entities::potion::ThrownPotion::shoot(potion, 0, pos, motion, 1.0, 0.0, &mut LegacyRandom::new(0));
            thrown.owner = None;
            thrown.velocity = motion;
            thrown.yaw = yaw;
            thrown.left_owner = int(tag, "LeftOwner").unwrap_or(1) != 0;
            return Some(world.spawn_potion(thrown));
        }
        "minecraft:bat" => {
            let mut bat = minecraftoss_entities::bat::Bat::new(pos);
            bat.persistence_required = persistent;
            bat.resting = int(tag, "BatFlags").unwrap_or(0) & 1 != 0;
            if let Some(health) = health {
                bat.health = health;
            }
            let id = world.spawn_bat(bat, no_ai);
            let entity = world.bat_mut(id)?;
            entity.random = random;
            entity.yaw = yaw;
            entity.look_control.head_yaw = yaw;
            entity.body_rotation.body_yaw = yaw;
            id
        }
        "minecraft:villager" => {
            let mut villager = Villager::new(pos);
            villager.set_age(age);
            villager.persistence_required = persistent;
            // `VillagerData` and `Xp`.
            let data = tag.get("VillagerData");
            if let Some(kind) = data.and_then(|d| text(d, "type")) {
                villager.kind = kind.to_owned();
            }
            if let Some(profession) = data.and_then(|d| text(d, "profession")).and_then(minecraftoss_entities::villager::Profession::from_id) {
                villager.profession = profession;
            }
            if let Some(level) = data.and_then(|d| int(d, "level")) {
                villager.level = level;
            }
            villager.xp = int(tag, "Xp").unwrap_or(0);
            if let Some(health) = health {
                villager.health = health;
            }
            let id = world.spawn_villager(villager, true);
            // The saved stacks' sizes, as the item catalog knows them.
            let maxes: Vec<u8> = tag.get("Inventory").and_then(Tag::as_list).into_iter().flatten().map(|s| world.item_max_stack(text(s, "id").unwrap_or("")).clamp(1, 99) as u8).collect();
            let entity = world.villager_mut(id)?;
            entity.random = random;
            entity.yaw = yaw;
            // `AbstractVillager.readAdditionalSaveData`: the offers it had
            // made (none are made until needed), and `Villager`'s restock
            // clock.
            entity.offers = tag.get("Offers").and_then(offers_of);
            // `InventoryCarrier.readInventoryFromTag` (`fromItemList`: each
            // stack added) and `FoodLevel`.
            for (saved, max) in tag.get("Inventory").and_then(Tag::as_list).into_iter().flatten().zip(maxes) {
                if let (Some(item), Some(count)) = (text(saved, "id"), int(saved, "count").or(Some(1))) {
                    let components = saved.get("components").map(tag_json);
                    let stack = minecraftoss_player::inventory::ItemStack { id: item.to_owned(), count: count.clamp(1, 99) as u8, max, components };
                    entity.inventory.add(stack);
                }
            }
            entity.food_level = int(tag, "FoodLevel").unwrap_or(0);
            // What it holds is its main hand.
            entity.held_item = tag.get("equipment").and_then(|e| e.get("mainhand")).and_then(|item| {
                let id = text(item, "id")?.to_owned();
                let components = item.get("components").map(tag_json).and_then(|v| v.as_object().cloned()).unwrap_or_default();
                Some(minecraftoss_entities::trading::TradeItem { id, count: int(item, "count").unwrap_or(1), components })
            });
            // `Mob.readAdditionalSaveData`: false unless saved (village
            // templates save true).
            entity.can_pick_up_loot = int(tag, "CanPickUpLoot").is_some_and(|b| b != 0);
            entity.last_restock = tag.get("LastRestock").and_then(Tag::as_i64).unwrap_or(0);
            entity.restocks_today = int(tag, "RestocksToday").unwrap_or(0);
            // `Villager.readAdditionalSaveData`: its gossip (the codec's
            // container, then `clear` and `putAll`) and decay time.
            let saved: Vec<_> = tag
                .get("Gossips")
                .and_then(Tag::as_list)
                .into_iter()
                .flatten()
                .filter_map(|g| {
                    let ints = g.get("Target").and_then(Tag::as_ints)?;
                    let target = minecraftoss_entities::gossip::uuid_from_ints([*ints.first()?, *ints.get(1)?, *ints.get(2)?, *ints.get(3)?]);
                    let kind = minecraftoss_entities::gossip::GossipType::from_id(text(g, "Type")?)?;
                    Some((target, kind, int(g, "Value")?))
                })
                .collect();
            std::sync::Arc::make_mut(&mut entity.gossips).replace_with(&minecraftoss_entities::gossip::Gossips::from_entries(&saved));
            entity.last_gossip_decay = tag.get("LastGossipDecay").and_then(Tag::as_i64).unwrap_or(0);
            // Its brain (idle, play and panic, with no points of interest
            // yet), unless it has no AI.
            if !no_ai {
                world.activate_villager(id, yaw);
            }
            id
        }
        _ => return None,
    };
    // `Entity.load`: the UUID gossip about it is kept by.
    if let Some(ints) = tag.get("UUID").and_then(Tag::as_ints).filter(|i| i.len() == 4) {
        world.set_uuid(id, minecraftoss_entities::gossip::uuid_from_ints([ints[0], ints[1], ints[2], ints[3]]));
    }
    // `LivingEntity.readAdditionalSaveData`: its effects as saved.
    if let (Some(list), Some((effects, _, _))) = (tag.get("active_effects").and_then(Tag::as_list), world.mob_effects_mut(id)) {
        for saved in list {
            if let Some(effect) = effect_of(saved) {
                effects.insert(effect);
            }
        }
    }
    // `Entity.load`: the motion, ground contact and fire it was saved with.
    if let Some(body) = world.body_mut(id) {
        if let Some(motion) = tag.get("Motion").and_then(Tag::as_list) {
            let at = |i: usize| motion.get(i).and_then(Tag::as_f64).unwrap_or(0.0);
            body.velocity = DVec3::new(at(0), at(1), at(2));
        }
        body.on_ground = int(tag, "OnGround").unwrap_or(0) != 0;
        body.fire_ticks = fire;
    }
    Some(id)
}

/// A saved villager's offers (`MerchantOffers.CODEC`: `{Recipes: [...]}`,
/// an empty list when absent); none when one does not read, as vanilla
/// then makes them afresh.
fn offers_of(tag: &Tag) -> Option<Vec<minecraftoss_entities::trading::MerchantOffer>> {
    let Some(list) = tag.get("Recipes") else { return Some(Vec::new()) };
    list.as_list()?.iter().map(|offer| minecraftoss_entities::trading::MerchantOffer::from_json(&tag_json(offer))).collect()
}

/// Offers as `MerchantOffers.CODEC` writes them to NBT: whole numbers as
/// ints, `rewardExp` as a byte and `priceMultiplier` as a float.
fn offers_tag(offers: &[minecraftoss_entities::trading::MerchantOffer]) -> Tag {
    let recipes = offers
        .iter()
        .map(|offer| {
            let mut tag = json_tag(&offer.to_json());
            if let Tag::Compound(map) = &mut tag {
                if let Some(Tag::Double(m)) = map.get("priceMultiplier") {
                    let m = *m as f32;
                    map.insert("priceMultiplier".to_owned(), Tag::Float(m));
                }
            }
            tag
        })
        .collect();
    Tag::Compound([("Recipes".to_owned(), Tag::List(recipes))].into_iter().collect())
}

/// NBT as the JSON the item and offer codecs read (numbers keep their
/// value; bytes stand for booleans too).
fn tag_json(tag: &Tag) -> serde_json::Value {
    use serde_json::Value;
    match tag {
        Tag::Byte(v) => Value::from(*v),
        Tag::Short(v) => Value::from(*v),
        Tag::Int(v) => Value::from(*v),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(v) => Value::from(f64::from(*v)),
        Tag::Double(v) => Value::from(*v),
        Tag::ByteArray(a) => Value::Array(a.iter().map(|&v| Value::from(v)).collect()),
        Tag::String(s) => Value::from(s.clone()),
        Tag::List(list) => Value::Array(list.iter().map(tag_json).collect()),
        Tag::Compound(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), tag_json(v))).collect()),
        Tag::IntArray(a) => Value::Array(a.iter().map(|&v| Value::from(v)).collect()),
        Tag::LongArray(a) => Value::Array(a.iter().map(|&v| Value::from(v)).collect()),
    }
}

/// JSON as NBT: whole numbers as ints, others as doubles, booleans as
/// bytes.
fn json_tag(value: &serde_json::Value) -> Tag {
    use serde_json::Value;
    match value {
        Value::Object(map) => Tag::Compound(map.iter().map(|(k, v)| (k.clone(), json_tag(v))).collect()),
        Value::Array(list) => Tag::List(list.iter().map(json_tag).collect()),
        Value::String(s) => Tag::String(s.clone()),
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(v) => Tag::Int(v as i32),
            None => Tag::Double(n.as_f64().unwrap_or(0.0)),
        },
        Value::Null => Tag::Compound(Default::default()),
    }
}

/// A saved effect (`MobEffectInstance.CODEC`: its ID, level, time, flags
/// and the effect hidden beneath), none for one this port lacks.
fn effect_of(tag: &Tag) -> Option<minecraftoss_entities::effects::EffectInstance> {
    let effect = minecraftoss_entities::effects::MobEffect::from_id(text(tag, "id")?)?;
    Some(details_of(tag, effect))
}

fn details_of(tag: &Tag, effect: minecraftoss_entities::effects::MobEffect) -> minecraftoss_entities::effects::EffectInstance {
    let flag = |key: &str| tag.get(key).and_then(Tag::as_i64).map(|v| v != 0);
    let visible = flag("show_particles").unwrap_or(true);
    minecraftoss_entities::effects::EffectInstance {
        effect,
        duration: int(tag, "duration").unwrap_or(0),
        amplifier: int(tag, "amplifier").unwrap_or(0) & 255,
        ambient: flag("ambient").unwrap_or(false),
        visible,
        show_icon: flag("show_icon").unwrap_or(visible),
        hidden: tag.get("hidden_effect").map(|hidden| Box::new(details_of(hidden, effect))),
    }
}

/// An effect as `MobEffectInstance.CODEC` writes it.
fn effect_tag(effect: &minecraftoss_entities::effects::EffectInstance, with_id: bool) -> Tag {
    let mut map = std::collections::BTreeMap::new();
    if with_id {
        map.insert("id".to_owned(), Tag::String(effect.effect.id().to_owned()));
    }
    map.insert("amplifier".to_owned(), Tag::Byte(effect.amplifier as u8 as i8));
    map.insert("duration".to_owned(), Tag::Int(effect.duration));
    map.insert("ambient".to_owned(), Tag::Byte(i8::from(effect.ambient)));
    map.insert("show_particles".to_owned(), Tag::Byte(i8::from(effect.visible)));
    map.insert("show_icon".to_owned(), Tag::Byte(i8::from(effect.show_icon)));
    if let Some(hidden) = &effect.hidden {
        map.insert("hidden_effect".to_owned(), effect_tag(hidden, false));
    }
    Tag::Compound(map)
}

/// Doubles as an NBT list.
fn doubles(values: [f64; 3]) -> Tag {
    Tag::List(values.into_iter().map(Tag::Double).collect())
}

/// A UUID for an entity saved without one, from its ID, position and the
/// session (vanilla's are random).
fn fresh_uuid(id: u64, position: DVec3) -> Tag {
    static SALT: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let salt = *SALT.get_or_init(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64));
    let mix = |mut z: u64| {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let a = mix(salt ^ id ^ position.x.to_bits().rotate_left(17) ^ position.y.to_bits() ^ position.z.to_bits().rotate_left(41));
    let b = mix(a);
    Tag::IntArray(vec![(a >> 32) as i32, a as i32, (b >> 32) as i32, b as i32])
}

/// The live mobs whose feet `keep` accepts, as saved entity tags
/// (`Entity.saveWithoutId` with the type's fields), each from the tag it
/// came from when it has one: what the entity world simulates overwrites
/// it; the rest (equipment, attributes, UUID, villager data) stays as
/// loaded. `spawn_saved` reads them back.
pub fn mob_tags(world: &EntityWorld, keep: impl Fn(DVec3) -> bool, originals: &std::collections::HashMap<u64, Tag>) -> Vec<(u64, Tag)> {
    use minecraftoss_entities::movement::Body;
    let mut out = Vec::new();
    let mut add = |id: u64, kind: &str, body: &Body, yaw: f32, health: f32, persistent: bool, fields: Vec<(&str, Tag)>| {
        if !keep(body.position) || health <= 0.0 {
            return;
        }
        let mut map = match originals.get(&id) {
            Some(Tag::Compound(map)) => map.clone(),
            _ => std::collections::BTreeMap::new(),
        };
        let p = body.position;
        map.insert("id".into(), Tag::String(kind.to_owned()));
        map.insert("Pos".into(), doubles([p.x, p.y, p.z]));
        map.insert("Motion".into(), doubles([body.velocity.x, body.velocity.y, body.velocity.z]));
        map.insert("Rotation".into(), Tag::List(vec![Tag::Float(yaw), Tag::Float(0.0)]));
        map.insert("OnGround".into(), Tag::Byte(i8::from(body.on_ground)));
        map.insert("fall_distance".into(), Tag::Double(body.fall_distance));
        map.insert("Fire".into(), Tag::Short(body.fire_ticks.clamp(-32768, 32767) as i16));
        map.insert("Health".into(), Tag::Float(health));
        map.insert("PersistenceRequired".into(), Tag::Byte(i8::from(persistent)));
        // The UUID gossip about it is kept by.
        map.entry("UUID".into()).or_insert_with(|| Tag::IntArray(minecraftoss_entities::gossip::uuid_to_ints(world.uuid_of(id)).to_vec()));
        for (key, value) in fields {
            map.insert(key.to_owned(), value);
        }
        out.push((id, Tag::Compound(map)));
    };
    let age = |age: &minecraftoss_entities::age::Age| vec![("Age", Tag::Int(age.ticks)), ("ForcedAge", Tag::Int(age.forced))];
    let text_tag = |s: &str| Tag::String(s.to_owned());
    for e in world.bats() {
        add(e.id, "minecraft:bat", &e.bat.body, e.yaw, e.bat.health, e.bat.persistence_required, vec![("BatFlags", Tag::Byte(i8::from(e.bat.resting)))]);
    }
    for e in world.zombies() {
        let kind = e.zombie.kind.type_id();
        let fields = vec![("IsBaby", Tag::Byte(i8::from(e.zombie.baby))), ("CanBreakDoors", Tag::Byte(i8::from(e.zombie.can_break_doors)))];
        add(e.id, kind, &e.zombie.body, e.yaw, e.zombie.health, e.zombie.persistence_required, fields);
    }
    for e in world.skeletons() {
        let mut fields = Vec::new();
        if e.skeleton.kind == minecraftoss_entities::skeleton::SkeletonKind::Bogged {
            fields.push(("sheared", Tag::Byte(i8::from(e.skeleton.sheared))));
        }
        add(e.id, e.skeleton.kind.type_id(), &e.skeleton.body, e.yaw, e.skeleton.health, e.skeleton.persistence_required, fields);
    }
    for e in world.endermen() {
        let mut fields = vec![("anger_end_time", Tag::Long(e.ai.state.enderman.anger_end_time))];
        if let Some(block) = e.carried() {
            let mut state: std::collections::BTreeMap<String, Tag> = [("Name".to_owned(), Tag::String(block.id.clone()))].into_iter().collect();
            if !block.properties.is_empty() {
                let properties = block.properties.iter().map(|(k, v)| (k.clone(), Tag::String(v.clone()))).collect();
                state.insert("Properties".to_owned(), Tag::Compound(properties));
            }
            fields.push(("carriedBlockState", Tag::Compound(state)));
        }
        add(e.id, "minecraft:enderman", &e.enderman.body, e.enderman.yaw, e.enderman.health, e.enderman.persistence_required, fields);
    }
    for e in world.witches() {
        let w = &e.witch;
        add(e.id, "minecraft:witch", &w.body, w.yaw, w.health, w.persistence_required, Vec::new());
    }
    for e in world.iron_golems() {
        let g = &e.golem;
        let fields = vec![("PlayerCreated", Tag::Byte(i8::from(g.player_created))), ("anger_end_time", Tag::Long(e.ai.state.enderman.anger_end_time))];
        add(e.id, "minecraft:iron_golem", &g.body, g.yaw, g.health, g.persistence_required, fields);
    }
    for e in world.wolves() {
        let w = &e.wolf;
        let mut fields = vec![
            ("variant", text_tag(&format!("minecraft:{}", w.variant))),
            ("sound_variant", text_tag(&format!("minecraft:{}", w.sound_variant))),
            ("CollarColor", Tag::Byte(w.collar as i8)),
            ("Sitting", Tag::Byte(i8::from(w.ordered_to_sit))),
            ("anger_end_time", Tag::Long(e.ai.state.enderman.anger_end_time)),
        ];
        fields.extend(age(&w.age));
        fields.push(("InLove", Tag::Int(w.in_love)));
        fields.push(("attributes", with_attribute_base(originals.get(&e.id), "minecraft:max_health", f64::from(w.max_health_base))));
        if let Some(owner) = w.owner {
            fields.push(("Owner", Tag::IntArray(minecraftoss_entities::gossip::uuid_to_ints(owner).to_vec())));
        }
        add(e.id, "minecraft:wolf", &w.body, w.yaw, w.health, w.persistence_required, fields);
    }
    for e in world.slimes() {
        let slime = &e.slime;
        let fields = vec![("Size", Tag::Int(slime.size - 1)), ("wasOnGround", Tag::Byte(i8::from(slime.was_on_ground)))];
        add(e.id, "minecraft:slime", &slime.body, slime.yaw, slime.health, slime.persistence_required, fields);
    }
    for e in world.creepers() {
        let c = &e.creeper;
        if c.exploded {
            continue;
        }
        let fields = vec![
            ("powered", Tag::Byte(i8::from(c.powered))),
            ("Fuse", Tag::Short(c.max_swell as i16)),
            ("ExplosionRadius", Tag::Byte(c.explosion_radius as i8)),
            ("ignited", Tag::Byte(i8::from(c.ignited))),
        ];
        add(e.id, "minecraft:creeper", &c.body, e.ai.yaw, c.health, c.persistence_required, fields);
    }
    for e in world.spiders() {
        add(e.id, "minecraft:spider", &e.spider.body, e.spider.yaw, e.spider.health, e.spider.persistence_required, Vec::new());
    }
    for e in world.villagers() {
        let v = &e.villager;
        let mut fields = age(&v.age);
        let mut data = std::collections::BTreeMap::new();
        data.insert("type".to_owned(), text_tag(&v.kind));
        data.insert("profession".to_owned(), text_tag(v.profession.id()));
        data.insert("level".to_owned(), Tag::Int(v.level));
        fields.push(("VillagerData", Tag::Compound(data)));
        fields.push(("Xp", Tag::Int(v.xp)));
        if let Some(item) = &e.held_item {
            let mut stack: std::collections::BTreeMap<String, Tag> = [("id".to_owned(), text_tag(&item.id)), ("count".to_owned(), Tag::Int(item.count))].into_iter().collect();
            if !item.components.is_empty() {
                stack.insert("components".to_owned(), json_tag(&serde_json::Value::Object(item.components.clone())));
            }
            fields.push(("equipment", Tag::Compound([("mainhand".to_owned(), Tag::Compound(stack))].into_iter().collect())));
        }
        add(e.id, "minecraft:villager", &v.body, e.ai.as_deref().map_or(e.yaw, |ai| ai.yaw), v.health, v.persistence_required, fields);
    }
    for e in world.cows() {
        let cow = &e.cow;
        let mut fields = age(&cow.age);
        fields.push(("variant", text_tag(match cow.variant { CowVariant::Warm => "minecraft:warm", CowVariant::Cold => "minecraft:cold", CowVariant::Temperate => "minecraft:temperate" })));
        fields.push(("sound_variant", text_tag(match cow.sound_variant { CowSoundVariant::Moody => "minecraft:moody", CowSoundVariant::Classic => "minecraft:classic" })));
        if let Some(horse) = &e.horse {
            use minecraftoss_entities::horse::HorseKind;
            let mut fields = age(&cow.age);
            fields.push(("EatingHaystack", Tag::Byte(i8::from(horse.eating))));
            fields.push(("Bred", Tag::Byte(i8::from(horse.bred))));
            fields.push(("Temper", Tag::Int(horse.temper)));
            fields.push(("Tame", Tag::Byte(i8::from(horse.tame))));
            match horse.kind {
                HorseKind::Horse => fields.push(("Variant", Tag::Int(horse.variant))),
                HorseKind::Donkey => fields.push(("ChestedHorse", Tag::Byte(i8::from(horse.chest)))),
            }
            add(e.id, horse.kind.type_id(), &cow.body, cow.yaw, cow.health, cow.persistence_required, fields);
            continue;
        }
        let kind = match &e.mooshroom {
            Some(state) => {
                fields.push(("Type", text_tag(if state.variant == MushroomVariant::Brown { "brown" } else { "red" })));
                "minecraft:mooshroom"
            }
            None => "minecraft:cow",
        };
        add(e.id, kind, &cow.body, cow.yaw, cow.health, cow.persistence_required, fields);
    }
    for e in world.sheep() {
        let mut fields = age(&e.sheep.age);
        fields.push(("Color", Tag::Byte((e.sheep.wool.data() & 15) as i8)));
        fields.push(("Sheared", Tag::Byte(i8::from(e.sheep.wool.sheared()))));
        add(e.id, "minecraft:sheep", &e.body, e.yaw, e.health, e.sheep.persistence_required, fields);
    }
    for e in world.pigs() {
        let pig = &e.pig;
        let mut fields = age(&pig.age);
        fields.push(("variant", text_tag(match pig.variant { PigVariant::Warm => "minecraft:warm", PigVariant::Cold => "minecraft:cold", PigVariant::Temperate => "minecraft:temperate" })));
        fields.push(("sound_variant", text_tag(match pig.sound_variant { PigSoundVariant::Mini => "minecraft:mini", PigSoundVariant::Big => "minecraft:big", PigSoundVariant::Classic => "minecraft:classic" })));
        if pig.saddled {
            // `Mob.equipment`'s saddle slot.
            let saddle = Tag::Compound([("id".to_owned(), text_tag("minecraft:saddle")), ("count".to_owned(), Tag::Int(1))].into_iter().collect());
            fields.push(("equipment", Tag::Compound([("saddle".to_owned(), saddle)].into_iter().collect())));
        }
        add(e.id, "minecraft:pig", &pig.body, pig.yaw, pig.health, pig.persistence_required, fields);
    }
    for e in world.chickens() {
        let chicken = &e.chicken;
        let mut fields = age(&chicken.age);
        fields.push(("variant", text_tag(match chicken.variant { ChickenVariant::Warm => "minecraft:warm", ChickenVariant::Cold => "minecraft:cold", ChickenVariant::Temperate => "minecraft:temperate" })));
        fields.push(("sound_variant", text_tag(match chicken.sound_variant { ChickenSoundVariant::Picky => "minecraft:picky", ChickenSoundVariant::Classic => "minecraft:classic" })));
        fields.push(("EggLayTime", Tag::Int(chicken.egg_time)));
        add(e.id, "minecraft:chicken", &chicken.body, chicken.yaw, chicken.health, chicken.persistence_required, fields);
    }
    // `AbstractVillager.addAdditionalSaveData`: the offers once made, and
    // `Villager`'s restock clock.
    for (id, tag) in &mut out {
        if let (Some(e), Tag::Compound(map)) = (world.villagers().iter().find(|v| v.id == *id), tag) {
            match &e.offers {
                Some(offers) => map.insert("Offers".to_owned(), offers_tag(offers)),
                None => map.remove("Offers"),
            };
            map.insert("LastRestock".to_owned(), Tag::Long(e.last_restock));
            // `writeInventoryToTag` (the stacks in slot order) and `FoodLevel`.
            let inventory = e
                .inventory
                .slots
                .iter()
                .flatten()
                .map(|s| {
                    let mut stack = vec![("id".to_owned(), Tag::String(s.id.clone())), ("count".to_owned(), Tag::Int(i32::from(s.count)))];
                    if let Some(components) = &s.components {
                        stack.push(("components".to_owned(), json_tag(components)));
                    }
                    Tag::Compound(stack.into_iter().collect())
                })
                .collect();
            map.insert("Inventory".to_owned(), Tag::List(inventory));
            map.insert("FoodLevel".to_owned(), Tag::Byte(e.food_level as i8));
            map.insert("CanPickUpLoot".to_owned(), Tag::Byte(i8::from(e.can_pick_up_loot)));
            map.insert("RestocksToday".to_owned(), Tag::Int(e.restocks_today));
            // `Villager.addAdditionalSaveData`: gossip as `GossipContainer.CODEC`
            // writes it, and its decay time.
            let gossips = e
                .gossips
                .unpack()
                .into_iter()
                .map(|(target, kind, value)| {
                    let entry = [
                        ("Target".to_owned(), Tag::IntArray(minecraftoss_entities::gossip::uuid_to_ints(target).to_vec())),
                        ("Type".to_owned(), Tag::String(kind.id().to_owned())),
                        ("Value".to_owned(), Tag::Int(value)),
                    ];
                    Tag::Compound(entry.into_iter().collect())
                })
                .collect();
            map.insert("Gossips".to_owned(), Tag::List(gossips));
            map.insert("LastGossipDecay".to_owned(), Tag::Long(e.last_gossip_decay));
        }
    }
    // `LivingEntity.addAdditionalSaveData`: the effects, when there are any.
    for (id, tag) in &mut out {
        if let (Some(effects), Tag::Compound(map)) = (world.mob_effects(*id), tag) {
            if effects.is_empty() {
                map.remove("active_effects");
            } else {
                map.insert("active_effects".to_owned(), Tag::List(effects.iter().map(|e| effect_tag(e, true)).collect()));
            }
        }
    }
    // Splash potions in flight (`ThrowableItemProjectile`).
    for p in world.potions() {
        let potion = &p.potion;
        if !keep(potion.position) || !potion.alive {
            continue;
        }
        let mut map = std::collections::BTreeMap::new();
        map.insert("id".to_owned(), Tag::String("minecraft:splash_potion".to_owned()));
        map.insert("Pos".to_owned(), doubles(potion.position.to_array()));
        map.insert("Motion".to_owned(), doubles(potion.velocity.to_array()));
        map.insert("Rotation".to_owned(), Tag::List(vec![Tag::Float(potion.yaw), Tag::Float(potion.pitch)]));
        map.insert("LeftOwner".to_owned(), Tag::Byte(i8::from(potion.left_owner)));
        let contents = Tag::Compound([("potion".to_owned(), Tag::String(potion.potion.id().to_owned()))].into_iter().collect());
        let components = Tag::Compound([("minecraft:potion_contents".to_owned(), contents)].into_iter().collect());
        let item = [("id".to_owned(), Tag::String("minecraft:splash_potion".to_owned())), ("count".to_owned(), Tag::Int(1)), ("components".to_owned(), components)];
        map.insert("Item".to_owned(), Tag::Compound(item.into_iter().collect()));
        map.insert("UUID".to_owned(), fresh_uuid(p.id, potion.position));
        out.push((p.id, Tag::Compound(map)));
    }
    out
}

/// The level's items and experience orbs whose position `keep` accepts, as
/// saved entity tags (`ItemEntity`, `ExperienceOrb.addAdditionalSaveData`).
pub fn level_entity_tags(level: &Level<'static>, keep: impl Fn(DVec3) -> bool) -> Vec<Tag> {
    let mut out = Vec::new();
    for e in &level.entities {
        let p = DVec3::from_array(e.pos);
        if e.removed || !keep(p) {
            continue;
        }
        let mut map: std::collections::BTreeMap<String, Tag> = std::collections::BTreeMap::new();
        map.insert("Pos".into(), doubles(e.pos));
        map.insert("Motion".into(), doubles(e.delta));
        map.insert("OnGround".into(), Tag::Byte(i8::from(e.on_ground)));
        map.insert("UUID".into(), fresh_uuid(e.id as u64, p));
        if let Some(item) = e.item_data() {
            if item.stack.is_empty() {
                continue;
            }
            map.insert("id".into(), Tag::String("minecraft:item".into()));
            let mut stack = std::collections::BTreeMap::new();
            stack.insert("id".to_owned(), Tag::String(item.stack.id.clone()));
            stack.insert("count".to_owned(), Tag::Int(item.stack.count));
            if let Some(components) = &item.stack.components {
                stack.insert("components".to_owned(), components.clone());
            }
            map.insert("Item".into(), Tag::Compound(stack));
            map.insert("Age".into(), Tag::Short(item.age.clamp(-32768, 32767) as i16));
            map.insert("PickupDelay".into(), Tag::Short(item.pickup_delay.clamp(-32768, 32767) as i16));
            map.insert("Health".into(), Tag::Short(item.health as i16));
        } else if let Some(orb) = e.orb_data() {
            map.insert("id".into(), Tag::String("minecraft:experience_orb".into()));
            map.insert("Value".into(), Tag::Short(orb.value as i16));
            map.insert("Count".into(), Tag::Int(orb.count));
            map.insert("Age".into(), Tag::Short(orb.age as i16));
            map.insert("Health".into(), Tag::Short(orb.health as i16));
        } else {
            continue;
        }
        out.push(Tag::Compound(map));
    }
    out
}

/// A saved item or experience orb joins the level; false for other types.
pub fn load_level_entity(level: &mut Level<'static>, tag: &Tag) -> bool {
    let Some(pos) = position(tag) else { return false };
    let motion = tag.get("Motion").and_then(Tag::as_list).map_or([0.0; 3], |m| [0, 1, 2].map(|i| m.get(i).and_then(Tag::as_f64).unwrap_or(0.0)));
    match text(tag, "id") {
        Some("minecraft:item") => {
            let Some(item) = tag.get("Item") else { return true };
            let (Some(id), count) = (text(item, "id"), int(item, "count").unwrap_or(1)) else { return true };
            let mut stack = minecraftoss_core::item::ItemStack::new(id, count);
            stack.components = item.get("components").cloned();
            level.spawn_item_with(pos.to_array(), stack, motion, int(tag, "PickupDelay").unwrap_or(0), int(tag, "Age").unwrap_or(0));
            true
        }
        Some("minecraft:experience_orb") => {
            let mut orb = minecraftoss_world::level::entity::Entity::experience_orb(pos.to_array(), int(tag, "Value").unwrap_or(0));
            orb.delta = motion;
            if let Some(data) = orb.orb_data_mut() {
                data.count = int(tag, "Count").unwrap_or(1).max(1);
                data.age = int(tag, "Age").unwrap_or(0);
                data.health = int(tag, "Health").unwrap_or(5);
            }
            level.add_entity(orb);
            true
        }
        _ => false,
    }
}

/// A saved attribute's value (`AttributeInstance.calculateValue`): the
/// base, its added values, then the multipliers of the base and the total,
/// within the attribute's range (follow range: 0 to 2048).
/// A saved attribute's base.
fn attribute_base(tag: &Tag, id: &str) -> Option<f64> {
    tag.get("attributes")?.as_list()?.iter().find(|a| a.get("id").and_then(Tag::as_str) == Some(id))?.get("base")?.as_f64()
}

/// A saved entity's attributes with one base set (the entry added when
/// missing), the rest as loaded.
fn with_attribute_base(original: Option<&Tag>, id: &str, base: f64) -> Tag {
    let mut list = original.and_then(|t| t.get("attributes")).and_then(Tag::as_list).map(<[Tag]>::to_vec).unwrap_or_default();
    match list.iter_mut().find(|a| a.get("id").and_then(Tag::as_str) == Some(id)) {
        Some(Tag::Compound(entry)) => {
            entry.insert("base".to_owned(), Tag::Double(base));
        }
        _ => {
            let entry: std::collections::BTreeMap<String, Tag> = [("id".to_owned(), Tag::String(id.to_owned())), ("base".to_owned(), Tag::Double(base))].into_iter().collect();
            list.push(Tag::Compound(entry));
        }
    }
    Tag::List(list)
}

fn attribute_value(tag: &Tag, id: &str) -> Option<f64> {
    let entry = tag.get("attributes")?.as_list()?.iter().find(|a| a.get("id").and_then(Tag::as_str) == Some(id))?;
    let mut base = entry.get("base")?.as_f64()?;
    let modifiers: Vec<(&str, f64)> = entry
        .get("modifiers")
        .and_then(Tag::as_list)
        .map(|list| list.iter().filter_map(|m| Some((m.get("operation")?.as_str()?, m.get("amount")?.as_f64()?))).collect())
        .unwrap_or_default();
    for (_, amount) in modifiers.iter().filter(|(op, _)| *op == "add_value") {
        base += amount;
    }
    let mut value = base;
    for (_, amount) in modifiers.iter().filter(|(op, _)| *op == "add_multiplied_base") {
        value += base * amount;
    }
    for (_, amount) in modifiers.iter().filter(|(op, _)| *op == "add_multiplied_total") {
        value *= 1.0 + amount;
    }
    Some(value.clamp(0.0, 2048.0))
}

/// The head slot's item, when there is one: whether it can be damaged
/// (armor can; pumpkins and heads cannot).
/// The head, chest, legs and feet slots' item IDs from `equipment`.
fn armor_slots(tag: &Tag) -> [Option<String>; 4] {
    ["head", "chest", "legs", "feet"].map(|slot| tag.get("equipment")?.get(slot)?.get("id")?.as_str().map(str::to_owned))
}

fn head_item(tag: &Tag) -> Option<bool> {
    let id = tag.get("equipment")?.get("head")?.get("id")?.as_str()?;
    Some(id.ends_with("_helmet"))
}

/// Whether a position lies in a chunk.
pub fn in_chunk(position: DVec3, chunk: ChunkPos) -> bool {
    (position.x.floor() as i32) >> 4 == chunk.x && (position.z.floor() as i32) >> 4 == chunk.z
}
