//! The integrated server's world simulation for a streamed world: the
//! exact `minecraftoss_world::level::Level` owns the loaded FULL chunks,
//! takes the player's edits and uses, ticks at 20 Hz, and hands back the
//! positions whose blocks changed so the client scene and the saved chunks
//! follow it.

use crate::scene::{Block, BlockPos, HandcraftedScene, Scene};
use crate::terrain::BlockStates;
use minecraftoss_core::{BlockStateId, Chunk, ChunkPos};
use minecraftoss_world::chunk_map::WorldGen;
use minecraftoss_world::level::{update, Level};
use minecraftoss_world::natural_spawner::tick::{CensusMob, SpawnPlayer};
use std::sync::Arc;

/// How a player changed a block (`BlockItem.place` sets with flags 11,
/// breaking removes the block with flags 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerEdit {
    Place,
    Break,
}

pub struct ServerSim {
    level: Level<'static>,
    states: Arc<BlockStates>,
    /// Mobs, ticked after the level each tick.
    mobs: minecraftoss_entities::world::EntityWorld,
    /// Each block state as mobs read it.
    mob_tables: crate::server_mobs::MobTables,
    /// The tag each mob was loaded or spawned from, by entity ID: saving
    /// overwrites what the entity world simulates and keeps the rest.
    mob_tags: std::collections::HashMap<u64, minecraftoss_core::nbt::Tag>,
    /// The entity storage the level's entities are saved to, shared with
    /// the chunk map (`EntityStorage`).
    storage: Option<Arc<minecraftoss_world::storage::ChunkStorage>>,
    /// Server ticks since the last autosave.
    ticks_since_save: u32,
    entity_loot: Option<minecraftoss_entities::loot::EntityLootBook>,
    shearing_loot: Option<minecraftoss_entities::loot::ShearingLootBook>,
    /// Spawned or loaded mobs the entity world does not simulate yet, each
    /// with its riders, and the tag that saves them unchanged.
    dormant: Vec<(Vec<CensusMob>, minecraftoss_core::nbt::Tag)>,
    /// Creeper blasts since the client last heard of them.
    explosions: Vec<minecraftoss_entities::creeper::CreeperExplosion>,
    /// The player's `takeXpDelay`: ticks until it can take another orb.
    take_xp_delay: i32,
    /// The client was asked to close its trading screen.
    merchant_closing: bool,
    player_kills: Vec<String>,
}

/// What a player's hit or use on a mob did, for the client to present.
#[derive(Clone, Debug, Default)]
pub struct MobResult {
    /// Some only for attacks, in command order; false means no damage landed.
    pub attack_hurt: Option<bool>,
    pub sounds: Vec<crate::mob_actions::MobSound>,
    /// The acting player's inventory slots the action changed, with what
    /// they now hold.
    pub slots: Vec<(usize, Option<minecraftoss_player::inventory::ItemStack>)>,
    /// A trading screen the action opened (`openTradingScreen`).
    pub merchant: Option<MerchantView>,
}

/// A player's trading screen as the client shows it: the villager's
/// offers, profession, level and experience
/// (`ClientboundMerchantOffersPacket`) and the menu's slots.
#[derive(Clone, Debug, PartialEq)]
pub struct MerchantView {
    pub villager: u64,
    pub profession: &'static str,
    pub level: i32,
    pub xp: i32,
    pub offers: Vec<minecraftoss_entities::trading::MerchantOffer>,
    pub payment: [Option<minecraftoss_player::inventory::ItemStack>; 2],
    pub result: Option<minecraftoss_player::inventory::ItemStack>,
    pub future_xp: i32,
    /// Items' maximum stack sizes the offers name, for the client's
    /// prices.
    pub max_stacks: Vec<(String, i32)>,
}

/// What the player does on the trading screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MerchantOp {
    /// An offer picked in the list.
    Select(i32),
    /// A payment slot clicked (shift: moved back).
    Payment { slot: usize, right: bool, shift: bool },
    /// The result clicked (shift: traded as often as it goes).
    Result { shift: bool },
    Close,
}

/// A trading screen's new state after an operation, or its closing
/// (`view` none), with the acting player's inventory slots it changed and
/// the cursor.
#[derive(Clone, Debug)]
pub struct MerchantUpdate {
    pub view: Option<MerchantView>,
    pub slots: Vec<(usize, Option<minecraftoss_player::inventory::ItemStack>)>,
    pub cursor: Option<minecraftoss_player::inventory::ItemStack>,
    /// The server asks the client to close the screen (the villager went
    /// away or stopped trading); the client answers with `Close`.
    pub closing: bool,
}

/// An item entity on the server, as the client shows it. Components travel
/// as their JSON text.
#[derive(Clone, Debug)]
pub struct ServerItem {
    pub id: i32,
    pub item: String,
    pub count: i32,
    pub components: Option<String>,
    pub position: [f64; 3],
    pub previous_position: [f64; 3],
    pub velocity: [f64; 3],
    pub age: i32,
    pub pickup_delay: i32,
    pub on_ground: bool,
}

/// A falling block entity as the client shows it.
#[derive(Clone, Debug)]
pub struct ServerFallingBlock {
    pub position: [f64; 3],
    pub previous_position: [f64; 3],
    pub block: String,
}

/// An experience orb as the client shows it.
#[derive(Clone, Copy, Debug)]
pub struct ServerOrb {
    pub id: i32,
    pub position: [f64; 3],
    pub previous_position: [f64; 3],
    pub value: i32,
    /// `tickCount`, which drives its colour.
    pub tick_count: i32,
}

/// The level's entities as the client shows them after a tick.
#[derive(Clone, Debug, Default)]
pub struct EntitySnapshot {
    pub items: Vec<ServerItem>,
    pub tnt: Vec<ServerTnt>,
    pub falling: Vec<ServerFallingBlock>,
    pub orbs: Vec<ServerOrb>,
}

/// A primed TNT entity as the client shows it.
#[derive(Clone, Copy, Debug)]
pub struct ServerTnt {
    pub position: [f64; 3],
    pub previous_position: [f64; 3],
    pub fuse: i32,
}

fn stack_of(item: &str, count: i32, components: Option<&str>) -> minecraftoss_core::item::ItemStack {
    let mut stack = minecraftoss_core::item::ItemStack::new(item, count);
    stack.components = components.map(|json| minecraftoss_core::nbt::Tag::String(json.to_owned()));
    stack
}

fn components_of(stack: &minecraftoss_core::item::ItemStack) -> Option<String> {
    match &stack.components {
        Some(minecraftoss_core::nbt::Tag::String(json)) => Some(json.clone()),
        Some(other) => Some(format!("{other:?}")),
        None => None,
    }
}

impl ServerSim {
    /// A level for a dimension. The world generation data lives as long as
    /// the process (one small leak per world opened).
    pub fn new(worldgen: Arc<WorldGen>, states: Arc<BlockStates>, dimension_type: &str) -> Self {
        let leaked: &'static Arc<WorldGen> = Box::leak(Box::new(worldgen));
        let worldgen: &'static WorldGen = leaked;
        let range = states.vertical_range();
        let mut level = Level::new(&worldgen.library, range.start, range.end - range.start);
        level.random_sequences = minecraftoss_core::loot::RandomSequences::new(worldgen.terrain.seed);
        if let Err(e) = level.set_dimension(dimension_type) {
            eprintln!("world simulation without environment: {e}");
        }
        let mob_tables = crate::server_mobs::MobTables::new(&level, &states);
        // Natural spawning, when the entity catalog can create mobs.
        if level.registries().entities.is_some() {
            match minecraftoss_world::natural_spawner::CreatureSpawns::load(worldgen.terrain.registries.clone(), dimension_type, false) {
                Ok(spawns) => level.natural_spawning = Some(minecraftoss_world::level::spawning::NaturalSpawning::new(Arc::new(spawns))),
                Err(e) => eprintln!("natural spawning unavailable: {e}"),
            }
        }
        // The players are real entities: arrows hit them.
        let mut mobs = minecraftoss_entities::world::EntityWorld::default();
        mobs.set_players_pickable(true);
        // New mobs' UUIDs differ from session to session, as vanilla's
        // random ones do.
        mobs.set_uuid_salt(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64));
        mobs.pois = minecraftoss_entities::poi::PoiManager::new(range.start >> 4, (range.end - 1) >> 4);
        Self {
            level,
            states,
            mobs,
            mob_tables,
            mob_tags: std::collections::HashMap::new(),
            storage: None,
            ticks_since_save: 0,
            entity_loot: None,
            shearing_loot: None,
            dormant: Vec::new(),
            explosions: Vec::new(),
            player_kills: Vec::new(),
            take_xp_delay: 0,
            merchant_closing: false,
        }
    }

    /// Loads the entity and shearing loot tables and the villager trade
    /// sets from the data JAR, with the world seed's random sequences.
    pub fn load_loot(&mut self, jar: &std::path::Path, seed: u64) {
        self.entity_loot = minecraftoss_entities::loot::EntityLootBook::from_jar(jar, seed).map_err(|e| eprintln!("server entity loot unavailable: {e:#}")).ok();
        self.shearing_loot = minecraftoss_entities::loot::ShearingLootBook::from_jar(jar, seed).map_err(|e| eprintln!("server shearing loot unavailable: {e:#}")).ok();
        // The trade sets read the items' enchantability and stack sizes
        // from the exported item catalog: beside a JAR fetched into a
        // MinecraftOSS-shaped folder, in a checkout, or under the working
        // directory.
        const CATALOG: &str = "artifacts/item-catalog/26.3.json";
        let catalog = [
            jar.parent().map(|root| root.join(CATALOG)),
            std::env::var_os("MINECRAFTOSS_ROOT").map(|root| std::path::Path::new(&root).join(CATALOG)),
            Some(std::path::PathBuf::from(CATALOG)),
        ]
        .into_iter()
        .flatten()
        .find(|path| path.is_file());
        match catalog.map(|path| minecraftoss_entities::trading::TradeBook::from_jar(jar, &path)) {
            Some(Ok(book)) => self.mobs.set_trades(Arc::new(book), seed),
            Some(Err(e)) => eprintln!("server villager trades unavailable: {e:#}"),
            None => eprintln!("server villager trades unavailable: no item catalog"),
        }
    }

    /// A player's hit (`attack`, with what the player brings to it) or item
    /// use on a mob, with a copy of the player's inventory. Drops enter the
    /// level as item entities.
    pub fn mob_action(&mut self, hit: minecraftoss_entities::world::MobHit, attack: Option<minecraftoss_entities::world::PlayerAttack>, mut inventory: minecraftoss_player::inventory::Inventory, selected: usize, infinite: bool) -> MobResult {
        let before = inventory.slots.clone();
        let mut actor = crate::mob_actions::Actor {
            inventory: &mut inventory,
            selected,
            infinite,
            entity_loot: self.entity_loot.as_mut(),
            shearing_loot: self.shearing_loot.as_mut(),
        };
        let outcome = if let Some(attack) = &attack {
            crate::mob_actions::attack(&mut self.mobs, hit, &mut actor, attack)
        } else {
            crate::mob_actions::interact(&mut self.mobs, hit, &mut actor)
        };
        self.player_kills.extend(outcome.player_kills);
        for (stack, position) in outcome.drops {
            let components = stack.components.as_ref().map(|c| c.to_string());
            let stack = stack_of(&stack.id, i32::from(stack.count), components.as_deref());
            self.level.spawn_at_location(position.to_array(), stack);
        }
        for (position, amount) in outcome.experience {
            self.level.award_experience(position.to_array(), amount);
        }
        let slots = inventory
            .slots
            .iter()
            .zip(&before)
            .enumerate()
            .filter(|(_, (now, was))| now != was)
            .map(|(slot, (now, _))| (slot, now.clone()))
            .collect();
        self.spawn_trade_experience();
        MobResult { attack_hurt: attack.map(|_| outcome.hurt), sounds: outcome.sounds, slots, merchant: self.merchant_view(0) }
    }

    /// The player's trading screen as the client shows it.
    pub fn merchant_view(&mut self, player: u64) -> Option<MerchantView> {
        let menu = self.mobs.merchant_menu(player)?.clone();
        let offers = self.mobs.villager_offers(menu.villager).map(<[_]>::to_vec).unwrap_or_default();
        let villager = self.mobs.villagers().iter().find(|e| e.id == menu.villager)?;
        let mut max_stacks = Vec::new();
        for offer in &offers {
            for id in std::iter::once(&offer.buy.id).chain(offer.buy_b.as_ref().map(|b| &b.id)).chain(std::iter::once(&offer.sell.id)) {
                if !max_stacks.iter().any(|(item, _): &(String, i32)| item == id) {
                    max_stacks.push((id.clone(), self.mobs.item_max_stack(id)));
                }
            }
        }
        Some(MerchantView {
            villager: menu.villager,
            profession: villager.villager.profession.id(),
            level: villager.villager.level,
            xp: villager.villager.xp,
            offers,
            payment: menu.payment.clone(),
            result: menu.result.clone(),
            future_xp: menu.future_xp,
            max_stacks,
        })
    }

    /// An operation on the player's trading screen, with a copy of the
    /// player's inventory (and the hotbar slot selected): the screen's new
    /// state, the slots it changed and the cursor. Closing returns the
    /// payments and the cursor to the inventory; what does not fit drops
    /// at the player's feet.
    pub fn merchant(&mut self, op: MerchantOp, mut inventory: minecraftoss_player::inventory::Inventory, selected: usize, feet: [f64; 3]) -> MerchantUpdate {
        let before = inventory.slots.clone();
        match op {
            MerchantOp::Select(index) => self.mobs.merchant_select(0, index, &mut inventory),
            MerchantOp::Payment { slot, right, shift } => self.mobs.merchant_click_payment(0, slot, right, shift, &mut inventory),
            MerchantOp::Result { shift } => self.mobs.merchant_click_result(0, shift, &mut inventory),
            MerchantOp::Close => {
                for stack in self.mobs.merchant_close(0, &mut inventory, selected) {
                    let components = stack.components.as_ref().map(|c| c.to_string());
                    self.level.spawn_at_location(feet, stack_of(&stack.id, i32::from(stack.count), components.as_deref()));
                }
                self.merchant_closing = false;
            }
        }
        self.spawn_trade_experience();
        let slots = inventory.slots.iter().zip(&before).enumerate().filter(|(_, (now, was))| now != was).map(|(slot, (now, _))| (slot, now.clone())).collect();
        MerchantUpdate { view: self.merchant_view(0), slots, cursor: inventory.cursor.clone(), closing: false }
    }

    /// `ServerPlayer.tick`'s `stillValid` check on an open trading screen:
    /// once the villager is out of reach, dead or done trading, the client
    /// is asked (once) to close it.
    fn check_merchant(&mut self, players: &[minecraftoss_entities::tempt::PlayerCandidate]) -> Option<MerchantUpdate> {
        if self.merchant_closing || self.mobs.merchant_menu(0).is_none() {
            return None;
        }
        let player = players.iter().find(|p| p.id == 0)?;
        let eyes = player.position + glam::DVec3::Y * f64::from(player.eye_height);
        if self.mobs.merchant_still_valid(0, eyes) {
            return None;
        }
        self.merchant_closing = true;
        Some(MerchantUpdate { view: None, slots: Vec::new(), cursor: None, closing: true })
    }

    /// The orbs trades dropped, each whole into the level.
    fn spawn_trade_experience(&mut self) {
        for (position, value) in self.mobs.take_trade_experience() {
            self.level.spawn_experience_orb(position.to_array(), value);
        }
    }

    /// Where the level's entities are saved (the chunk map's storage).
    pub fn set_entity_storage(&mut self, storage: Option<Arc<minecraftoss_world::storage::ChunkStorage>>) {
        self.storage = storage;
    }

    /// A chunk joins the level with its entities (`EntityStorage`): those
    /// saved for it, or for a chunk the level never saved, the ones
    /// generation made. A chunk sent again keeps the entities it has.
    pub fn load_chunk(&mut self, chunk: &Chunk) {
        let fresh = self.level.chunk(chunk.pos).is_none();
        self.level.insert_chunk(chunk.clone());
        if !fresh {
            return;
        }
        self.load_pois(chunk);
        let saved = self.storage.as_ref().and_then(|storage| {
            storage.load_entities(chunk.pos).unwrap_or_else(|e| {
                eprintln!("entities of chunk {:?} failed to load: {e}", chunk.pos);
                None
            })
        });
        for tag in saved.unwrap_or_else(|| chunk.generation.entities.clone()) {
            self.add_saved_entity(tag);
        }
    }

    /// A chunk's points of interest (`PoiManager.checkConsistencyWithBlocks`
    /// for each section that may hold one).
    fn load_pois(&mut self, chunk: &Chunk) {
        let table = &self.mob_tables.pois;
        let kind = |state: BlockStateId| table.get(state.0 as usize).copied().flatten();
        for (i, section) in chunk.sections().iter().enumerate() {
            let sy = chunk.min_section_y() + i as i32;
            let maybe = match &section.blocks {
                minecraftoss_core::palette::PalettedContainer::Single(state) => kind(*state).is_some(),
                minecraftoss_core::palette::PalettedContainer::Direct(values) => values.iter().any(|&s| kind(s).is_some()),
            };
            if maybe {
                self.mobs.pois.load_section((chunk.pos.x, sy, chunk.pos.z), |(x, y, z)| kind(section.block((x & 15) as usize, (y & 15) as usize, (z & 15) as usize)));
            }
        }
    }

    /// The blocks set since the last call move the points of interest
    /// (`ServerLevel.updatePOIOnBlockStateChange`).
    fn sync_pois(&mut self) {
        for (x, y, z) in self.level.take_block_log() {
            let state = self.level.block(minecraftoss_core::BlockPos::new(x, y, z));
            let new = self.mob_tables.pois.get(state.0 as usize).copied().flatten();
            let old = self.mobs.pois.kind((x, y, z));
            self.mobs.pois.block_changed((x, y, z), old, new);
        }
    }

    /// A saved entity joins the level: items and experience orbs as level
    /// entities, mobs the entity world simulates into it, and the rest
    /// dormant.
    fn add_saved_entity(&mut self, tag: minecraftoss_core::nbt::Tag) {
        self.mobs.set_day_time(self.level.overworld_clock());
        if crate::server_mobs::load_level_entity(&mut self.level, &tag) {
            return;
        }
        match crate::server_mobs::spawn_saved(&mut self.mobs, &tag) {
            Some(id) => {
                self.mob_tags.insert(id, tag);
            }
            None => self.dormant.push((minecraftoss_world::natural_spawner::tick::census_of(&tag), tag)),
        }
    }

    /// Saves a chunk's entities: its mobs (from the tag each came from,
    /// with what the entity world changed), its dormant mobs as they came,
    /// and its items and experience orbs. Unloading also takes them out.
    fn save_chunk_entities(&mut self, pos: ChunkPos, unload: bool) {
        let inside = |p: glam::DVec3| crate::server_mobs::in_chunk(p, pos);
        let mut tags: Vec<minecraftoss_core::nbt::Tag> = crate::server_mobs::mob_tags(&self.mobs, inside, &self.mob_tags).into_iter().map(|(_, tag)| tag).collect();
        tags.extend(self.dormant.iter().filter(|(group, _)| inside(glam::DVec3::from_array(group[0].pos))).map(|(_, tag)| tag.clone()));
        tags.extend(crate::server_mobs::level_entity_tags(&self.level, inside));
        if unload {
            self.mobs.remove_where(inside);
            self.dormant.retain(|(group, _)| !inside(glam::DVec3::from_array(group[0].pos)));
            self.level.entities.retain(|e| !(inside(glam::DVec3::from_array(e.pos)) && (e.item_data().is_some() || e.orb_data().is_some())));
            let alive: std::collections::HashSet<u64> = self.mobs.mob_ids().collect();
            self.mob_tags.retain(|id, _| alive.contains(id));
        }
        if let Some(storage) = &self.storage {
            if let Err(e) = storage.save_entities(pos, &tags) {
                eprintln!("entities of chunk {pos:?} failed to save: {e}");
            }
        }
    }

    /// Saves every loaded chunk's entities and writes the entity storage
    /// (autosave and shutdown).
    pub fn save_all_entities(&mut self) {
        let loaded: Vec<ChunkPos> = self.level.chunks().map(|c| c.pos).collect();
        for pos in loaded {
            self.save_chunk_entities(pos, false);
        }
        if let Some(storage) = &self.storage {
            if let Err(e) = storage.flush() {
                eprintln!("entity storage write failed: {e}");
            }
        }
    }

    /// Before the level ticks: natural spawning's players (registered with
    /// its spawn counter), the mob census for its caps, and the simulation
    /// area.
    pub fn prepare_spawning(&mut self, players: &[minecraftoss_entities::tempt::PlayerCandidate]) {
        let Some(natural) = &mut self.level.natural_spawning else { return };
        let spawn_players: Vec<SpawnPlayer> = players.iter().map(|p| SpawnPlayer { pos: p.position.to_array(), spectator: p.spectator }).collect();
        natural.set_players(&spawn_players);
        let mut census: Vec<CensusMob> = self
            .mobs
            .census()
            .into_iter()
            .map(|m| {
                let half = f64::from(m.width / 2.0);
                let p = m.position;
                CensusMob {
                    kind: m.kind.to_owned(),
                    pos: p.to_array(),
                    bb: [p.x - half, p.y, p.z - half, p.x + half, p.y + f64::from(m.height), p.z + half],
                    persistent: m.persistent,
                    riding: false,
                }
            })
            .collect();
        census.extend(self.dormant.iter().flat_map(|(group, _)| group.iter().cloned()));
        natural.census = census;
        natural.simulation = self.level.ticking_chunks.iter().flatten().copied().collect();
    }

    /// Mobs natural spawning made join the entity world; types it does not
    /// simulate yet (and jockeys) wait dormant: they count toward the caps
    /// and block spawns until they despawn.
    fn take_spawned(&mut self) {
        for tag in std::mem::take(&mut self.level.spawned) {
            self.add_saved_entity(tag);
        }
    }

    /// `Mob.checkDespawn` for the mobs in loaded chunks, before they tick.
    fn despawn_mobs(&mut self, players: &[minecraftoss_entities::tempt::PlayerCandidate], difficulty: i32) {
        let feet: Vec<glam::DVec3> = players.iter().filter(|p| !p.spectator).map(|p| p.position).collect();
        let level = &self.level;
        let loaded = |p: glam::DVec3| level.chunk(ChunkPos::new((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4)).is_some();
        self.mobs.check_despawn(&feet, difficulty == 0, &loaded);
        // Dormant monsters go in peaceful and beyond 128 blocks (they never
        // idle: they do not tick).
        self.dormant.retain(|(group, _)| {
            let root = &group[0];
            if root.persistent || minecraftoss_world::natural_spawner::spawning::MobCategory::of_type(&root.kind) != Some(minecraftoss_world::natural_spawner::spawning::MobCategory::Monster) {
                return true;
            }
            if difficulty == 0 {
                return false;
            }
            let at = glam::DVec3::from_array(root.pos);
            feet.iter().map(|p| p.distance_squared(at)).min_by(f64::total_cmp).is_none_or(|d| d <= 128.0 * 128.0)
        });
    }

    /// The mobs spawned that the entity world does not simulate.
    pub fn dormant_count(&self) -> usize {
        self.dormant.len()
    }

    /// The recipe book mobs consult (breeding colours).
    pub fn set_recipe_book(&mut self, recipes: Arc<minecraftoss_player::crafting::RecipeBook>) {
        self.mobs.set_recipe_book(recipes);
    }

    /// Mobs tick after the level, inside the entity-ticking range (the
    /// level's ticking chunks, `DistanceManager.inEntityTickingRange`).
    pub fn tick_mobs(&mut self, players: &[minecraftoss_entities::tempt::PlayerCandidate], bright_outside: bool) {
        self.sync_pois();
        let ticking: std::collections::HashSet<ChunkPos> = self.level.ticking_chunks.iter().flatten().copied().collect();
        self.mobs.set_bright_outside(bright_outside);
        self.mobs.set_monsters_burn(self.level.monsters_burn());
        let Self { level, states, mobs, mob_tables, .. } = self;
        let mut world = crate::server_mobs::MobWorld { level: std::cell::RefCell::new(level), states, tables: mob_tables };
        let ticks = |p: glam::DVec3| ticking.contains(&ChunkPos::new((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4));
        // Brains draw from the level's own random, and villagers keep the
        // overworld clock's schedule.
        mobs.set_day_time(world.level.borrow().overworld_clock());
        let shared = match &world.level.borrow().random {
            minecraftoss_core::random::AnyRandom::Legacy(random) => Some((random.state(), random.gaussian_cache())),
            _ => None,
        };
        if let Some((state, _)) = shared {
            *mobs.level_random_mut() = minecraftoss_player::rng::LegacyRandom::from_raw_state(state);
        }
        mobs.tick_with_players_where(&mut world, players, &ticks);
        // Births and summons are listed for replays that tag them; the game
        // needs no list.
        let _ = (mobs.take_born_villagers(), mobs.take_born_wolves(), mobs.take_summoned_golems());
        if let Some((_, gaussian)) = shared {
            if let minecraftoss_core::random::AnyRandom::Legacy(random) = &mut world.level.borrow_mut().random {
                random.set_seed((mobs.level_random_mut().raw_state() ^ 0x5DEE_CE66D) as i64);
                random.set_gaussian_cache(gaussian);
            }
        }
        // `ServerPlayer.doTick` (the connection tick, after the levels):
        // the players push the mobs they walk into.
        mobs.push_from_players(players, &ticks);
        // Entity events (a villager's hearts, anger, happiness) are for the
        // client's particles, which it does not draw yet.
        let _ = mobs.take_entity_events();
        // A blast's victims drop their loot before its blocks break
        // (`ServerExplosion.explode`: `hurtEntities`, then the blocks).
        self.drop_death_loot();
        self.explode_creepers();
    }

    /// The loot and experience of the mobs that died, into the level as
    /// item entities and experience orbs (`dropAllDeathLoot`).
    fn drop_death_loot(&mut self) {
        let (drops, experience, kills) = crate::mob_actions::death_remains(&mut self.mobs, self.entity_loot.as_mut());
        self.player_kills.extend(kills);
        for (stack, position) in drops {
            let components = stack.components.as_ref().map(|c| c.to_string());
            let stack = stack_of(&stack.id, i32::from(stack.count), components.as_deref());
            self.level.spawn_at_location(position.to_array(), stack);
        }
        for (position, amount) in experience {
            self.level.award_experience(position.to_array(), amount);
        }
    }

    /// `Player.tick`'s `takeXpDelay` countdown, then `Player.aiStep` touching
    /// one orb (at random) when the player at `feet` can take one: returns
    /// the orb's ID, position and value.
    pub fn take_experience(&mut self, feet: Option<[f64; 3]>) -> Option<(i32, [f64; 3], i32)> {
        if self.take_xp_delay > 0 {
            self.take_xp_delay -= 1;
        }
        if self.take_xp_delay != 0 {
            return None;
        }
        let taken = self.level.player_touch_orb(feet?)?;
        self.take_xp_delay = 2;
        Some(taken)
    }

    /// The server's experience orbs.
    pub fn orbs(&self) -> Vec<ServerOrb> {
        self.level
            .entities
            .iter()
            .filter_map(|e| e.orb_data().map(|d| ServerOrb { id: e.id, position: e.pos, previous_position: e.old_position(), value: d.value, tick_count: e.tick_count }))
            .collect()
    }

    /// Experience at a position (`ExperienceOrb.award`), as orbs.
    pub fn award_experience(&mut self, position: [f64; 3], amount: i32) {
        self.level.award_experience(position, amount);
    }

    /// `summon minecraft:experience_orb` with its `Value`: a still orb.
    pub fn summon_orb(&mut self, position: [f64; 3], value: i32) {
        self.level.add_entity(minecraftoss_world::level::entity::Entity::experience_orb(position, value));
    }

    /// `/summon` for a mob: the level makes its tag (`SummonCommand`), and
    /// it joins the entity world as a loaded mob would.
    pub fn summon(&mut self, kind: &str, position: [f64; 3], nbt: Option<&minecraftoss_core::nbt::Tag>, y_rot: f32) -> Result<(), String> {
        let tag = self.level.summon_mob(kind, position, nbt, y_rot)?;
        // A new brain reads the schedule for the time it is.
        self.mobs.set_day_time(self.level.overworld_clock());
        self.add_saved_entity(tag);
        Ok(())
    }

    /// Creeper blasts reach the level (`ServerLevel.explode` with
    /// `ExplosionInteraction.MOB`): blocks break when mobs may grief, with
    /// the drops decaying (`mob_explosion_drop_decay`), and items, TNT and
    /// falling blocks are hurt and pushed. The entity world already hurt
    /// its mobs and the players.
    fn explode_creepers(&mut self) {
        use minecraftoss_world::level::explosion::{BlockInteraction, Explosion};
        for blast in self.mobs.take_creeper_explosions() {
            let interaction = if self.mobs.mob_griefing() { BlockInteraction::DestroyWithDecay } else { BlockInteraction::Keep };
            self.level.explode(Explosion { center: blast.position.to_array(), radius: blast.radius, fire: false, interaction, source: None });
            self.explosions.push(blast);
        }
    }

    /// The blasts since the last call, for the client's sound and particles.
    pub fn take_explosions(&mut self) -> Vec<minecraftoss_entities::creeper::CreeperExplosion> {
        std::mem::take(&mut self.explosions)
    }

    /// The mobs a player at `position` tracks (`ChunkMap.TrackedEntity`:
    /// within `range` blocks horizontally).
    pub fn tracked_mobs(&self, position: [f64; 3], range: f64) -> minecraftoss_entities::world::EntityWorld {
        let range_sq = range * range;
        self.mobs.clone_where(|p| {
            let (dx, dz) = (p.x - position[0], p.z - position[2]);
            dx * dx + dz * dz <= range_sq
        })
    }

    /// A chunk leaves the level; its entities are saved and taken out.
    pub fn unload_chunk(&mut self, pos: ChunkPos) {
        self.save_chunk_entities(pos, true);
        self.level.remove_chunk(pos);
        self.mobs.pois.unload_chunk(pos.x, pos.z);
    }

    fn state_of(&self, block: Option<&Block>) -> BlockStateId {
        block.and_then(|b| self.states.state_of(b)).unwrap_or(BlockStateId::AIR)
    }

    /// Applies a player's edit that the client scene already shows.
    pub fn player_edit(&mut self, scene: &HandcraftedScene, pos: BlockPos, edit: PlayerEdit) {
        self.player_edit_block(pos, Scene::block(scene, pos), edit);
    }

    /// A player's edit to `block` (`None` for air) at a position.
    pub fn player_edit_block(&mut self, pos: BlockPos, block: Option<&Block>, edit: PlayerEdit) {
        let target = self.state_of(block);
        let at = minecraftoss_core::BlockPos::new(pos.0, pos.1, pos.2);
        if self.level.block(at) == target {
            return;
        }
        let flags = match edit {
            PlayerEdit::Place => update::ALL | 8,
            PlayerEdit::Break => update::ALL,
        };
        self.level.set_block(at, target, flags, update::LIMIT);
    }

    /// `useWithoutItem` on a simulated block; false when not simulated.
    pub fn use_block(&mut self, pos: BlockPos, player_facing: &str) -> bool {
        let facing = minecraftoss_core::pos::Direction::from_name(player_facing);
        self.level.use_block_facing(minecraftoss_core::BlockPos::new(pos.0, pos.1, pos.2), facing)
    }

    /// `BoneMealItem.useOn` on a clicked face: grow the block, or water
    /// plants in front of a sturdy face. True when the bone meal is used.
    pub fn bone_meal(&mut self, pos: BlockPos, face: &str) -> bool {
        let at = minecraftoss_core::BlockPos::new(pos.0, pos.1, pos.2);
        if self.level.grow_crop(at) {
            return true;
        }
        let Some(face) = minecraftoss_core::pos::Direction::from_name(face) else { return false };
        let state = self.level.block(at);
        self.level.registries().blocks.is_face_sturdy(state, face, minecraftoss_core::SupportType::Full) && self.level.grow_water_plant(at.relative(face, 1))
    }

    pub fn attack_block(&mut self, pos: BlockPos) -> bool {
        self.level.attack_block(minecraftoss_core::BlockPos::new(pos.0, pos.1, pos.2))
    }

    /// The players the level can see (fire spreads near them), and the
    /// difficulty (`Difficulty.getId`).
    pub fn set_players(&mut self, positions: &[[f64; 3]], difficulty: i32) {
        self.level.players = positions.to_vec();
        self.level.difficulty = difficulty;
    }

    /// The chunks random ticks run in: every loaded chunk within
    /// `distance` (Chebyshev) of `center`, in X then Z order.
    pub fn set_simulation_area(&mut self, center: (i32, i32), distance: i32) {
        let mut list = Vec::with_capacity(((2 * distance + 1) * (2 * distance + 1)) as usize);
        for x in center.0 - distance..=center.0 + distance {
            for z in center.1 - distance..=center.1 + distance {
                let pos = ChunkPos::new(x, z);
                if self.level.chunk(pos).is_some() {
                    list.push(pos);
                }
            }
        }
        self.level.ticking_chunks = Some(list);
    }

    /// Keeps the level's default clock with the client's day time.
    pub fn set_time(&mut self, ticks: i64) {
        self.level.set_time(ticks);
    }

    pub fn tick(&mut self) {
        self.level.tick();
        self.level.unsupported.clear();
    }

    /// Hands an item entity the client created (a drop, a spill) to the
    /// server, which simulates it from then on; returns its entity ID.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_item(&mut self, item: &str, count: i32, components: Option<&str>, position: [f64; 3], velocity: [f64; 3], pickup_delay: i32, age: i32) -> i32 {
        self.level.spawn_item_with(position, stack_of(item, count, components), velocity, pickup_delay, age)
    }

    /// The server's item entities.
    pub fn items(&self) -> Vec<ServerItem> {
        self.level
            .entities
            .iter()
            .filter_map(|e| e.item_data().map(|d| (e, d)))
            .map(|(e, d)| ServerItem {
                id: e.id,
                item: d.stack.id.clone(),
                count: d.stack.count,
                components: components_of(&d.stack),
                position: e.pos,
                previous_position: e.old_position(),
                velocity: e.delta,
                age: d.age,
                pickup_delay: d.pickup_delay,
                on_ground: e.on_ground,
            })
            .collect()
    }

    /// The server's falling blocks.
    pub fn falling_blocks(&self) -> Vec<ServerFallingBlock> {
        self.level
            .entities
            .iter()
            .filter_map(|e| {
                e.falling_data().map(|d| ServerFallingBlock {
                    position: e.pos,
                    previous_position: e.old_position(),
                    block: self.level.registries().blocks.block(self.level.registries().blocks.block_of(d.state)).name.to_string(),
                })
            })
            .collect()
    }

    /// The server's primed TNT.
    pub fn primed_tnt(&self) -> Vec<ServerTnt> {
        self.level
            .entities
            .iter()
            .filter_map(|e| e.tnt_data().map(|d| ServerTnt { position: e.pos, previous_position: e.old_position(), fuse: d.fuse }))
            .collect()
    }

    /// `ItemEntity.playerTouch` for a player standing at `feet`: `take`
    /// offers each touching stack (item, count, components) and returns how
    /// many the inventory took. Returns (entity ID, position, item, count,
    /// components) per pickup.
    pub fn pickup(&mut self, feet: [f64; 3], mut take: impl FnMut(&str, i32, Option<&str>) -> i32) -> Vec<(i32, [f64; 3], String, i32, Option<String>)> {
        self.level
            .player_touch_items(feet, |stack| {
                let components = components_of(stack);
                take(&stack.id, stack.count, components.as_deref())
            })
            .into_iter()
            .map(|(id, position, stack)| {
                let components = components_of(&stack);
                (id, position, stack.id, stack.count, components)
            })
            .collect()
    }

    /// Positions changed since the last call, with the block now there.
    pub fn take_changes(&mut self) -> Vec<(BlockPos, Option<Block>)> {
        let changed = self.level.take_changed();
        if std::env::var_os("MINECRAFTOSS_DEBUG_SERVER").is_some() && !changed.is_empty() {
            eprintln!("server changes at tick {}: {} (first {:?})", self.level.game_time, changed.len(), &changed[..changed.len().min(4)]);
        }
        changed
            .into_iter()
            .map(|(x, y, z)| {
                let state = self.level.block(minecraftoss_core::BlockPos::new(x, y, z));
                let block = if self.level.registries().blocks.is_air(state) { None } else { self.states.block(state).cloned() };
                ((x, y, z), block)
            })
            .collect()
    }
}

/// What the client asks of the integrated server thread, in order.
pub enum Command {
    LoadChunk(Arc<Chunk>),
    UnloadChunk(ChunkPos),
    /// A block the client set, as the client scene now shows it.
    PlayerEdit { pos: BlockPos, block: Option<Block>, edit: PlayerEdit },
    UseBlock { pos: BlockPos, facing: &'static str },
    BoneMeal { pos: BlockPos, face: &'static str },
    Attack(BlockPos),
    SpawnItem { item: String, count: i32, components: Option<String>, position: [f64; 3], velocity: [f64; 3], pickup_delay: i32, age: i32 },
    /// The recipe book, for mobs.
    RecipeBook(Arc<minecraftoss_player::crafting::RecipeBook>),
    /// The data JAR and world seed the server's loot tables come from.
    LootTables { jar: std::path::PathBuf, seed: u64 },
    /// `summon minecraft:experience_orb`.
    SummonOrb { position: [f64; 3], value: i32 },
    /// `/summon` for a mob, with the command's NBT and the new mob's own
    /// random yaw.
    Summon { kind: String, position: [f64; 3], nbt: Option<minecraftoss_core::nbt::Tag>, y_rot: f32 },
    /// A player's hit (`attack`) or item use on a mob, with a copy of its
    /// inventory.
    MobAction { hit: minecraftoss_entities::world::MobHit, attack: Option<minecraftoss_entities::world::PlayerAttack>, inventory: Box<minecraftoss_player::inventory::Inventory>, selected: usize, infinite: bool },
    /// An operation on the player's trading screen.
    Merchant { op: MerchantOp, inventory: Box<minecraftoss_player::inventory::Inventory>, selected: usize, feet: [f64; 3] },
    /// One server tick, with the player's state for it.
    Tick(Box<TickInput>),
}

/// The client state a server tick reads.
pub struct TickInput {
    pub day_ticks: i64,
    pub players: Vec<[f64; 3]>,
    pub difficulty: i32,
    pub simulation_center: (i32, i32),
    pub simulation_distance: i32,
    /// The player's feet, when it can pick items up, with its inventory and
    /// selected slot (the server offers touching stacks to this copy).
    pub pickup: Option<([f64; 3], Box<minecraftoss_player::inventory::Inventory>, usize)>,
    /// The player as mobs see it (held food, whether it can be targeted).
    pub mob_players: Vec<minecraftoss_entities::tempt::PlayerCandidate>,
    /// Where each of those players looks (endermen read stares from it).
    pub mob_views: Vec<(u64, minecraftoss_entities::enderman::PlayerView)>,
    /// Each of those players' health, effects and motion (witches pick
    /// their potions by them).
    pub mob_vitals: Vec<(u64, minecraftoss_entities::monster_ai::PlayerVitals)>,
    /// `Level.isBrightOutside`.
    pub bright_outside: bool,
    /// The player's position and how far away it tracks mobs.
    pub tracking: ([f64; 3], f64),
    /// The hurts that landed on the player since the last tick: the mob
    /// behind each and the damage type (its tame wolves avenge it).
    pub player_hurts: Vec<(Option<u64>, &'static str)>,
    /// The `spawn_mobs` game rule.
    pub spawn_mobs: bool,
}

/// What the server thread sends back after handling commands.
#[derive(Default)]
pub struct Output {
    pub player_kills: Vec<String>,
    /// Blocks that changed, with the block now there.
    pub changes: Vec<(BlockPos, Option<Block>)>,
    /// Entity snapshots, after a tick.
    pub entities: Option<EntitySnapshot>,
    /// Experience orbs the player took: (entity ID, position, value).
    pub orbs_taken: Vec<(i32, [f64; 3], i32)>,
    /// The mobs the player tracks, after a tick.
    pub mobs: Option<Box<minecraftoss_entities::world::EntityWorld>>,
    /// What the player's mob actions did, in order.
    pub mob_results: Vec<MobResult>,
    /// The trading screen's updates, in order.
    pub merchant: Vec<MerchantUpdate>,
    /// Mobs' hits on the players, in order.
    pub player_hits: Vec<minecraftoss_entities::world::PlayerHit>,
    /// Splash potions that broke near players, in order.
    pub player_splashes: Vec<(u64, minecraftoss_entities::world::PlayerSplash)>,
    /// Where splash potions broke (their sound and colour).
    pub potion_breaks: Vec<minecraftoss_entities::world::PotionBreak>,
    /// Creeper blasts this tick (`ClientboundExplodePacket`).
    pub explosions: Vec<minecraftoss_entities::creeper::CreeperExplosion>,
    /// Sounds mobs made this tick.
    pub mob_sounds: Vec<minecraftoss_entities::world::MobSound>,
    /// Items the player picked up: (entity ID, position, item, count, components).
    pub picked: Vec<(i32, [f64; 3], String, i32, Option<String>)>,
    /// What each `/summon` did: the type it made, or why it failed.
    pub summoned: Vec<Result<String, String>>,
    /// Positions where bone meal was used.
    pub bone_meal_used: Vec<BlockPos>,
    /// How many commands the server has handled so far.
    pub handled: u64,
    /// The last tick's phases, light solves and total milliseconds.
    pub tick_phases: Option<([f64; 6], (u32, f64), f64)>,
}

/// The integrated server on its own thread, as vanilla runs it: the client
/// sends commands and never waits for a tick.
pub struct ServerHandle {
    commands: Option<std::sync::mpsc::Sender<Command>>,
    outputs: std::sync::mpsc::Receiver<Output>,
    states: Arc<BlockStates>,
    /// Per block state: whether the level acts on a use or an attack.
    uses: Arc<Vec<bool>>,
    attacks: Arc<Vec<bool>>,
    sent: u64,
    /// Highest acknowledgement, including outputs already consumed by poll().
    received: u64,
    /// Outputs [`Self::wait_idle`] received ahead of the next poll.
    waited: Vec<Output>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ServerHandle {
    pub fn spawn(sim: ServerSim) -> Self {
        let states = sim.states.clone();
        let count = sim.level.registries().blocks.state_count();
        let uses: Vec<bool> = (0..count).map(|i| sim.level.handles_use(BlockStateId(i as u16))).collect();
        let attacks: Vec<bool> = (0..count).map(|i| sim.level.handles_attack(BlockStateId(i as u16))).collect();
        let (commands, receiver) = std::sync::mpsc::channel::<Command>();
        let (sender, outputs) = std::sync::mpsc::channel::<Output>();
        let thread = std::thread::Builder::new()
            .name("Server thread".into())
            .spawn(move || server_loop(sim, receiver, sender))
            .expect("server thread starts");
        Self { commands: Some(commands), outputs, states, uses: Arc::new(uses), attacks: Arc::new(attacks), sent: 0, received: 0, waited: Vec::new(), thread: Some(thread) }
    }

    fn send(&mut self, command: Command) {
        self.sent += 1;
        if let Some(commands) = &self.commands {
            let _ = commands.send(command);
        }
    }

    /// Commands sent so far (compare with `Output::handled`).
    pub fn sent(&self) -> u64 {
        self.sent
    }

    pub fn load_chunk(&mut self, chunk: &Arc<Chunk>) {
        self.send(Command::LoadChunk(chunk.clone()));
    }

    pub fn unload_chunk(&mut self, pos: ChunkPos) {
        self.send(Command::UnloadChunk(pos));
    }

    pub fn player_edit(&mut self, scene: &HandcraftedScene, pos: BlockPos, edit: PlayerEdit) {
        let block = Scene::block(scene, pos).cloned();
        self.send(Command::PlayerEdit { pos, block, edit });
    }

    fn state_in(&self, scene: &HandcraftedScene, pos: BlockPos) -> Option<BlockStateId> {
        Scene::block(scene, pos).and_then(|b| self.states.state_of(b))
    }

    /// `useWithoutItem` on a simulated block: true (and sent) when the level
    /// acts on the block the scene shows.
    pub fn use_block(&mut self, scene: &HandcraftedScene, pos: BlockPos, facing: &'static str) -> bool {
        let handled = self.state_in(scene, pos).is_some_and(|s| self.uses.get(usize::from(s.0)).copied().unwrap_or(false));
        if handled {
            self.send(Command::UseBlock { pos, facing });
        }
        handled
    }

    /// Bone meal on a clicked face; whether it was used arrives in an output.
    pub fn bone_meal(&mut self, pos: BlockPos, face: &'static str) {
        self.send(Command::BoneMeal { pos, face });
    }

    /// A player's attack; true (and sent) when the level acts on the block.
    pub fn attack_block(&mut self, scene: &HandcraftedScene, pos: BlockPos) -> bool {
        let handled = self.state_in(scene, pos).is_some_and(|s| self.attacks.get(usize::from(s.0)).copied().unwrap_or(false));
        if handled {
            self.send(Command::Attack(pos));
        }
        handled
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_item(&mut self, item: &str, count: i32, components: Option<&str>, position: [f64; 3], velocity: [f64; 3], pickup_delay: i32, age: i32) {
        self.send(Command::SpawnItem { item: item.to_owned(), count, components: components.map(str::to_owned), position, velocity, pickup_delay, age });
    }

    pub fn tick(&mut self, input: TickInput) {
        self.send(Command::Tick(Box::new(input)));
    }

    pub fn set_recipe_book(&mut self, recipes: Arc<minecraftoss_player::crafting::RecipeBook>) {
        self.send(Command::RecipeBook(recipes));
    }

    pub fn load_loot(&mut self, jar: std::path::PathBuf, seed: u64) {
        self.send(Command::LootTables { jar, seed });
    }

    /// An operation on the player's trading screen, with a copy of the
    /// player's inventory.
    pub fn merchant(&mut self, op: MerchantOp, inventory: &minecraftoss_player::inventory::Inventory, selected: usize, feet: [f64; 3]) {
        self.send(Command::Merchant { op, inventory: Box::new(inventory.clone()), selected, feet });
    }

    /// Summons a still experience orb worth `value`.
    pub fn summon_orb(&mut self, position: [f64; 3], value: i32) {
        self.send(Command::SummonOrb { position, value });
    }

    /// `/summon` for a mob; what it did arrives in an output. The new
    /// mob's yaw comes from its own random (`LivingEntity`'s constructor:
    /// `nextFloat() * (float)(Math.PI * 2)`, in degrees), seeded here from
    /// the clock as a fresh entity's is.
    pub fn summon(&mut self, kind: String, position: [f64; 3], nbt: Option<minecraftoss_core::nbt::Tag>) {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as i64);
        let y_rot = minecraftoss_core::random::LegacyRandom::new(nanos).next_f32() * (std::f64::consts::PI * 2.0) as f32;
        self.send(Command::Summon { kind, position, nbt, y_rot });
    }

    /// Hands a player's hit or use on a mob to the server; its result
    /// arrives in an output.
    pub fn mob_action(&mut self, hit: minecraftoss_entities::world::MobHit, attack: Option<minecraftoss_entities::world::PlayerAttack>, inventory: &minecraftoss_player::inventory::Inventory, selected: usize, infinite: bool) {
        self.send(Command::MobAction { hit, attack, inventory: Box::new(inventory.clone()), selected, infinite });
    }

    /// Outputs the server has produced since the last call.
    pub fn poll(&mut self) -> Vec<Output> {
        let mut outputs = std::mem::take(&mut self.waited);
        outputs.extend(self.outputs.try_iter());
        if let Some(last)=outputs.last(){self.received=self.received.max(last.handled);}
        outputs
    }

    /// Blocks until the server has handled every command sent so far (a
    /// capture's settle ticks, each answered before the next), keeping the
    /// outputs for the next [`Self::poll`].
    pub fn wait_idle(&mut self) {
        while self.received < self.sent {
            match self.outputs.recv() {
                Ok(out) => {self.received=self.received.max(out.handled);self.waited.push(out);},
                Err(_) => break,
            }
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        // Closing the channel ends the loop.
        self.commands = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn make_stack(recipes: &minecraftoss_player::crafting::RecipeBook, item: &str, count: i32, components: Option<&str>) -> minecraftoss_player::inventory::ItemStack {
    let mut stack = minecraftoss_player::inventory::ItemStack::new(item, count.clamp(0, 255) as u8);
    stack.components = components.and_then(|c| serde_json::from_str(c).ok());
    if stack.components.is_none() {
        stack.max = stack.max.min(recipes.max_stack(item));
    }
    stack
}

fn server_loop(mut sim: ServerSim, commands: std::sync::mpsc::Receiver<Command>, outputs: std::sync::mpsc::Sender<Output>) {
    let mut handled = 0u64;
    while let Ok(first) = commands.recv() {
        let mut out = Output::default();
        // Everything already queued is handled before answering.
        let mut next = Some(first);
        while let Some(command) = next.take().or_else(|| commands.try_recv().ok()) {
            handled += 1;
            match command {
                Command::LoadChunk(chunk) => sim.load_chunk(&chunk),
                Command::UnloadChunk(pos) => sim.unload_chunk(pos),
                Command::PlayerEdit { pos, block, edit } => sim.player_edit_block(pos, block.as_ref(), edit),
                Command::UseBlock { pos, facing } => {
                    sim.use_block(pos, facing);
                }
                Command::BoneMeal { pos, face } => {
                    if sim.bone_meal(pos, face) {
                        out.bone_meal_used.push(pos);
                    }
                }
                Command::Attack(pos) => {
                    sim.attack_block(pos);
                }
                Command::SpawnItem { item, count, components, position, velocity, pickup_delay, age } => {
                    sim.spawn_item(&item, count, components.as_deref(), position, velocity, pickup_delay, age);
                }
                Command::RecipeBook(recipes) => sim.set_recipe_book(recipes),
                Command::LootTables { jar, seed } => sim.load_loot(&jar, seed),
                Command::SummonOrb { position, value } => sim.summon_orb(position, value),
                Command::Summon { kind, position, nbt, y_rot } => {
                    out.summoned.push(sim.summon(&kind, position, nbt.as_ref(), y_rot).map(|()| kind));
                }
                Command::MobAction { hit, attack, inventory, selected, infinite } => {
                    out.mob_results.push(sim.mob_action(hit, attack, *inventory, selected, infinite));
                }
                Command::Merchant { op, inventory, selected, feet } => {
                    out.merchant.push(sim.merchant(op, *inventory, selected, feet));
                }
                Command::Tick(input) => {
                    let started = std::time::Instant::now();
                    let input = *input;
                    // `MinecraftServer.autoSave`: every five minutes.
                    sim.ticks_since_save += 1;
                    if sim.ticks_since_save >= 6000 {
                        sim.ticks_since_save = 0;
                        sim.save_all_entities();
                    }
                    sim.set_time(input.day_ticks);
                    sim.set_players(&input.players, input.difficulty);
                    // Living, non-spectating players draw experience orbs.
                    sim.level.living_players = input.mob_players.iter().filter(|p| p.alive && !p.spectator).map(|p| (p.position.to_array(), f64::from(p.eye_height))).collect();
                    sim.set_simulation_area(input.simulation_center, input.simulation_distance);
                    if let Some(natural) = &mut sim.level.natural_spawning {
                        natural.spawn_mobs = input.spawn_mobs;
                    }
                    sim.prepare_spawning(&input.mob_players);
                    sim.tick();
                    let mobs_started = std::time::Instant::now();
                    sim.take_spawned();
                    sim.despawn_mobs(&input.mob_players, input.difficulty);
                    // The player's fight memory: its `tickCount` moves on and
                    // the hurts the client took land in it.
                    sim.mobs.tick_player(0);
                    for &(source, kind) in &input.player_hurts {
                        sim.mobs.player_hurt(0, source, kind);
                    }
                    sim.mobs.set_player_views(input.mob_views.clone());
                    sim.mobs.set_player_vitals(input.mob_vitals.clone());
                    // What the player holds, as villagers see it
                    // (`ShowTradesToPlayer`).
                    let held = input.pickup.as_ref().and_then(|(_, inventory, selected)| inventory.slots.get(*selected).and_then(Option::as_ref).map(|s| s.id.clone()));
                    sim.mobs.set_player_main_hand(0, held.as_deref());
                    sim.tick_mobs(&input.mob_players, input.bright_outside);
                    sim.spawn_trade_experience();
                    out.merchant.extend(sim.check_merchant(&input.mob_players));
                    out.player_hits.extend(sim.mobs.take_player_hits());
                    out.player_splashes.extend(sim.mobs.take_player_splashes());
                    out.potion_breaks.extend(sim.mobs.take_potion_breaks());
                    out.explosions.extend(sim.take_explosions());
                    out.player_kills.append(&mut sim.player_kills);
                    out.mob_sounds.extend(sim.mobs.take_sounds());
                    sim.level.last_tick_phases[5] += mobs_started.elapsed().as_secs_f64() * 1000.0;
                    out.mobs = Some(Box::new(sim.tracked_mobs(input.tracking.0, input.tracking.1)));
                    let pickup_feet = input.pickup.as_ref().map(|(feet, _, _)| *feet);
                    if let Some((feet, mut inventory, selected)) = input.pickup {
                        let recipes = inventory.recipes.clone();
                        let picked = sim.pickup(feet, |item, count, components| {
                            let stack = make_stack(&recipes, item, count, components);
                            match inventory.add_item(stack, selected) {
                                None => count,
                                Some(rest) => count - i32::from(rest.count),
                            }
                        });
                        out.picked.extend(picked);
                    }
                    // After the items, one experience orb.
                    out.orbs_taken.extend(sim.take_experience(pickup_feet));
                    out.entities = Some(EntitySnapshot { items: sim.items(), tnt: sim.primed_tnt(), falling: sim.falling_blocks(), orbs: sim.orbs() });
                    let (solves, ms) = sim.level.light_solves.replace((0, 0.0));
                    out.tick_phases = Some((sim.level.last_tick_phases, (solves, ms), started.elapsed().as_secs_f64() * 1000.0));
                }
            }
        }
        out.changes = sim.take_changes();
        out.handled = handled;
        if outputs.send(out).is_err() {
            break;
        }
    }
    // The client closed the world: its entities are saved.
    sim.save_all_entities();
}

#[cfg(test)]
mod tests {
    use super::*;
    use minecraftoss_core::registries::DataPaths;
    use minecraftoss_core::Registries;
    use minecraftoss_generator::terrain::TerrainGenerator;
    use minecraftoss_world::chunk_map::ChunkMap;

    /// A block the player places reaches the level, which runs the block's
    /// behaviour and reports the blocks it changed back to the scene.
    #[test]
    fn placed_water_flows_through_the_level() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 0).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 0, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 2, 4);
        let mut server = ServerSim::new(worldgen, states.clone(), "minecraft:overworld");
        let mut scene = HandcraftedScene::streamed(states);
        for x in -1..=1 {
            for z in -1..=1 {
                let chunk = map.load_now(ChunkPos::new(x, z));
                server.load_chunk(&chunk);
                scene.insert_chunk(chunk);
            }
        }
        // A water source in the air above the terrain.
        let pos = (8, 200, 8);
        scene.set(pos, Some(Block::new("minecraft:water")));
        server.player_edit(&scene, pos, PlayerEdit::Place);
        let placed = server.take_changes();
        assert_eq!(placed.len(), 1, "{placed:?}");
        let mut flowed = 0;
        for _ in 0..10 {
            server.tick();
            flowed += server.take_changes().len();
        }
        assert!(flowed > 0, "water should fall from the source");
    }

    /// Creatures the SPAWN step generated join the server's mobs with their
    /// chunk, once per session, and wander while inside the ticking area.
    #[test]
    fn generated_mobs_join_the_server_and_wander() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 6, 8);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        // Plains around chunk (20, 128), a SPAWN probe with creatures.
        let mut generated = 0;
        for x in 15..=25 {
            for z in 123..=133 {
                let chunk = map.load_now(ChunkPos::new(x, z));
                generated += chunk.generation.entities.iter().filter(|t| t.get("id").and_then(minecraftoss_core::nbt::Tag::as_str).is_some_and(|id| ["minecraft:cow", "minecraft:pig", "minecraft:chicken", "minecraft:sheep"].contains(&id))).count();
                server.load_chunk(&chunk);
            }
        }
        assert!(generated > 0, "seed 1234 generates animals there");
        let animals = |world: &minecraftoss_entities::world::EntityWorld| world.cows().iter().filter(|c| c.horse.is_none()).count() + world.pigs().len() + world.chickens().len() + world.sheep().len();
        assert_eq!(animals(&server.mobs), generated, "every generated animal joins");
        // Loading a chunk again does not add its creatures twice.
        server.load_chunk(&map.load_now(ChunkPos::new(20, 128)));
        assert_eq!(animals(&server.mobs), generated);
        let start: Vec<glam::DVec3> = server.mobs.cows().iter().map(|c| c.cow.body.position).chain(server.mobs.sheep().iter().map(|s| s.body.position)).collect();
        server.set_simulation_area((20, 128), 5);
        for _ in 0..400 {
            server.tick();
            server.tick_mobs(&[], true);
        }
        let end: Vec<glam::DVec3> = server.mobs.cows().iter().map(|c| c.cow.body.position).chain(server.mobs.sheep().iter().map(|s| s.body.position)).collect();
        let moved = start.iter().zip(&end).filter(|(a, b)| a.distance(**b) > 0.5).count();
        assert!(moved > 0, "some animal strolls within 20 seconds: {start:?} -> {end:?}");
        assert!(end.iter().all(|p| p.y > 40.0), "animals stay on the ground: {end:?}");
        // The client tracks the ones near its player.
        let near = server.tracked_mobs([328.0, 80.0, 2056.0], 32.0);
        assert!(animals(&near) <= animals(&server.mobs));
    }

    /// Generated animals wander about a player among them, climbing the
    /// terrain's steps (`jumpFromGround`); around a spectator they idle
    /// once `noActionTime` passes 100, as `Mob.checkDespawn` only resets it
    /// near a player it counts.
    #[test]
    fn idle_animals_wander_near_a_player() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 6, 8);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        for x in 15..=25 {
            for z in 123..=133 {
                server.load_chunk(&map.load_now(ChunkPos::new(x, z)));
            }
        }
        // Among the first animals generated.
        let herd = server.mobs.cows().iter().map(|e| e.cow.body.position).chain(server.mobs.sheep().iter().map(|e| e.body.position)).next().expect("animals");
        let mut player = minecraftoss_entities::tempt::PlayerCandidate {
            id: 0,
            position: herd + glam::DVec3::new(3.0, 4.0, 0.0),
            eye_height: 1.62,
            main_hand_cow_food: false,
            offhand_cow_food: false,
            main_hand_pig_food: false,
            offhand_pig_food: false,
            main_hand_chicken_food: false,
            offhand_chicken_food: false,
            main_hand_carrot_on_a_stick: false,
            offhand_carrot_on_a_stick: false,
            main_hand_wolf_interest: false,
            offhand_wolf_interest: false,
            main_hand_horse_tempt: false,
            offhand_horse_tempt: false,
            alive: true,
            spectator: false,
            attackable: true,
        };
        let mut share = Vec::new();
        for spectator in [false, true] {
            player.spectator = spectator;
            let (mut moving, mut samples) = (0, 0);
            let mut last: std::collections::HashMap<u64, glam::DVec3> = Default::default();
            for tick in 0..1200 {
                server.set_players(&[player.position.to_array()], 2);
                server.set_simulation_area((20, 128), 5);
                server.tick();
                server.despawn_mobs(&[player], 2);
                server.tick_mobs(&[player], true);
                let w = &server.mobs;
                let animals = w.cows().iter().map(|e| (e.id, e.cow.body.position)).chain(w.sheep().iter().map(|e| (e.id, e.body.position)));
                for (id, p) in animals {
                    if p.distance(player.position) > 32.0 {
                        continue;
                    }
                    if let Some(old) = last.insert(id, p) {
                        if tick >= 200 {
                            samples += 1;
                            moving += usize::from(glam::DVec2::new(old.x - p.x, old.z - p.z).length() > 0.01);
                        }
                    }
                }
            }
            share.push(moving as f64 / samples.max(1) as f64);
        }
        assert!(share[0] > 0.15, "animals walk about a player: {share:?}");
        assert!(share[1] < 0.01, "and idle about a spectator: {share:?}");
    }

    /// Mobs leave with their chunk into the entity storage and come back
    /// from it as they were: where they wandered, hurt, sheared, and one
    /// killed stays dead, not respawned from generation.
    #[test]
    fn mobs_are_saved_with_their_chunks() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 6, 8);
        let dir = std::env::temp_dir().join(format!("minecraftoss-entity-save-{}", std::process::id()));
        let storage = Arc::new(minecraftoss_world::storage::ChunkStorage::new(&dir, "minecraft:overworld", registries.clone(), -64, 384));
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        server.set_entity_storage(Some(storage));
        let chunks: Vec<std::sync::Arc<Chunk>> = (15..=25).flat_map(|x| (123..=133).map(move |z| (x, z))).map(|(x, z)| map.load_now(ChunkPos::new(x, z))).collect();
        for chunk in &chunks {
            server.load_chunk(chunk);
        }
        server.set_simulation_area((20, 128), 5);
        for _ in 0..100 {
            server.tick();
            server.tick_mobs(&[], true);
        }
        let animals = |world: &minecraftoss_entities::world::EntityWorld| world.cows().iter().filter(|c| c.horse.is_none()).count() + world.pigs().len() + world.chickens().len() + world.sheep().len();
        let before = animals(&server.mobs);
        assert!(before > 1, "seed 1234 has animals there");
        // Hurt one animal and kill another.
        let hurt_id = server.mobs.mob_ids().next().unwrap();
        let victim = server.mobs.mob_ids().nth(1).unwrap();
        let hurt_at = server.mobs.body_mut(hurt_id).unwrap().position;
        for (id, amount) in [(hurt_id, 1.0), (victim, 100.0)] {
            if let Some(e) = server.mobs.cow_mut(id) { e.hurt(amount, minecraftoss_entities::world::DamageSourceKind::Generic); }
            if let Some(e) = server.mobs.sheep_mut(id) { e.hurt(amount, minecraftoss_entities::world::DamageSourceKind::Generic); }
            if let Some(e) = server.mobs.pig_mut(id) { e.hurt(amount, minecraftoss_entities::world::DamageSourceKind::Generic); }
            if let Some(e) = server.mobs.chicken_mut(id) { e.hurt(amount); }
        }
        let positions: Vec<glam::DVec3> = server.mobs.cows().iter().map(|c| c.cow.body.position).collect();
        for chunk in &chunks {
            server.unload_chunk(chunk.pos);
        }
        assert_eq!(animals(&server.mobs), 0, "unloading takes the mobs out");
        for chunk in &chunks {
            server.load_chunk(chunk);
        }
        assert_eq!(animals(&server.mobs), before - 1, "they come back without the one killed");
        let back: Vec<glam::DVec3> = server.mobs.cows().iter().map(|c| c.cow.body.position).collect();
        assert!(positions.iter().all(|p| back.iter().any(|b| b.distance(*p) < 1.0e-9)), "where they were: {positions:?} -> {back:?}");
        let health = [server.mobs.cows().iter().map(|c| (c.cow.body.position, c.cow.health)).collect::<Vec<_>>(), server.mobs.sheep().iter().map(|s| (s.body.position, s.health)).collect(), server.mobs.pigs().iter().map(|p| (p.pig.body.position, p.pig.health)).collect(), server.mobs.chickens().iter().map(|c| (c.chicken.body.position, c.chicken.health)).collect()].concat();
        let (_, restored) = health.iter().find(|(p, _)| p.distance(hurt_at) < 2.0).copied().expect("the hurt animal is back");
        assert!(restored < 10.0, "hurt as it was: {restored}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// At midnight on Normal, natural spawning fills the dark around a
    /// player with monsters up to the cap, and they despawn once the player
    /// is far away.
    #[test]
    fn night_spawns_monsters_around_the_player() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        if registries.entities.is_none() {
            return;
        }
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 6, 8);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        for x in 11..=29 {
            for z in 119..=137 {
                server.load_chunk(&map.load_now(ChunkPos::new(x, z)));
            }
        }
        let surface = minecraftoss_generator::feature::World::height_at(&server.level, minecraftoss_core::chunk::HeightmapKind::MotionBlocking, 328, 2056);
        let mut player = minecraftoss_entities::tempt::PlayerCandidate {
            id: 0,
            position: glam::DVec3::new(328.5, f64::from(surface), 2056.5),
            eye_height: 1.62,
            main_hand_cow_food: false,
            offhand_cow_food: false,
            main_hand_pig_food: false,
            offhand_pig_food: false,
            main_hand_chicken_food: false,
            offhand_chicken_food: false,
            main_hand_carrot_on_a_stick: false,
            offhand_carrot_on_a_stick: false,
            main_hand_wolf_interest: false,
            offhand_wolf_interest: false,
            main_hand_horse_tempt: false,
            offhand_horse_tempt: false,
            alive: true,
            spectator: false,
            attackable: true,
        };
        let monsters = |server: &ServerSim| {
            let census = server.mobs.census();
            // Dormant monsters count; generated minecarts and horses wait
            // dormant too.
            let monster = |kind: &str| minecraftoss_world::natural_spawner::spawning::MobCategory::of_type(kind) == Some(minecraftoss_world::natural_spawner::spawning::MobCategory::Monster);
            census.iter().filter(|m| ["minecraft:zombie", "minecraft:skeleton"].contains(&m.kind)).count() + server.dormant.iter().filter(|(g, _)| monster(&g[0].kind)).count()
        };
        server.set_time(18000);
        for _ in 0..40 {
            server.set_players(&[player.position.to_array()], 2);
            server.set_simulation_area((20, 128), 5);
            server.prepare_spawning(&[player]);
            server.tick();
            server.take_spawned();
            server.despawn_mobs(&[player], 2);
            server.tick_mobs(&[player], false);
        }
        let spawned = monsters(&server);
        assert!(spawned > 20, "the night brings monsters: {spawned}");
        // The cap is 70 for the 289 chunks around one player, overshot at
        // most by one tick's spawning.
        assert!(spawned <= 70 + 40, "monsters stay near the cap: {spawned}");
        assert!(server.mobs.zombies().iter().all(|z| z.zombie.body.position.distance(player.position) <= 132.0 + 40.0));
        // Far away, they despawn.
        player.position.x += 3000.0;
        server.prepare_spawning(&[player]);
        server.despawn_mobs(&[player], 2);
        assert_eq!(monsters(&server), 0, "monsters beyond 128 blocks despawn");
    }

    /// `/summon`: a mob made without NBT is finalized as a fresh spawn and
    /// joins the entity world; NBT is loaded over the fresh mob; monsters
    /// are refused in peaceful.
    /// A summoned villager's brain walks it about at the time of day it
    /// idles in.
    #[test]
    fn villagers_wander_by_day() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        if registries.entities.is_none() {
            return;
        }
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 6, 8);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        for x in 19..=21 {
            for z in 127..=129 {
                server.load_chunk(&map.load_now(ChunkPos::new(x, z)));
            }
        }
        let surface = minecraftoss_generator::feature::World::height_at(&server.level, minecraftoss_core::chunk::HeightmapKind::MotionBlocking, 328, 2056);
        let at = [328.5, f64::from(surface), 2056.5];
        server.set_players(&[at], 2);
        server.set_time(1000);
        server.summon("minecraft:villager", at, None, 0.0).unwrap();
        let start = server.mobs.villagers()[0].villager.body.position;
        let mut farthest = 0.0_f64;
        for _ in 0..600 {
            server.set_players(&[at], 2);
            server.set_simulation_area((20, 128), 4);
            server.tick();
            server.tick_mobs(&[], true);
            let villager = &server.mobs.villagers()[0];
            assert!(villager.villager.health > 0.0, "it lives");
            farthest = farthest.max(villager.villager.body.position.distance(start));
        }
        assert!(farthest > 1.0, "it wandered ({farthest})");
        // At night a homeless villager rests: it heads for (any) village.
        server.set_time(13000);
        for _ in 0..40 {
            server.set_players(&[at], 2);
            server.set_simulation_area((20, 128), 4);
            server.tick();
            server.tick_mobs(&[], true);
        }
        let brain = &server.mobs.villagers()[0].ai.as_ref().unwrap().brain;
        assert!(brain.activities.active.contains(&minecraftoss_entities::villager_brain::Activity::Rest), "it rests at night");
    }

    /// A summoned iron golem strolls about (no village near: any spot),
    /// and comes back from its saved tag cracked, built by a player or not.
    #[test]
    fn iron_golems_stroll_and_keep_their_tags() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        if registries.entities.is_none() {
            return;
        }
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 6, 8);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        for x in 19..=21 {
            for z in 127..=129 {
                server.load_chunk(&map.load_now(ChunkPos::new(x, z)));
            }
        }
        let surface = minecraftoss_generator::feature::World::height_at(&server.level, minecraftoss_core::chunk::HeightmapKind::MotionBlocking, 328, 2056);
        let at = [328.5, f64::from(surface), 2056.5];
        server.set_players(&[at], 2);
        server.set_time(1000);
        let nbt = minecraftoss_core::snbt::parse_compound("{PlayerCreated:1b}").unwrap();
        server.summon("minecraft:iron_golem", at, Some(&nbt), 0.0).unwrap();
        let start = server.mobs.iron_golems()[0].golem.body.position;
        let mut farthest = 0.0_f64;
        for _ in 0..2000 {
            server.set_players(&[at], 2);
            server.set_simulation_area((20, 128), 4);
            server.tick();
            server.tick_mobs(&[], true);
            let golem = &server.mobs.iron_golems()[0];
            assert!(golem.golem.health > 0.0, "it lives");
            farthest = farthest.max(golem.golem.body.position.distance(start));
        }
        assert!(farthest > 1.0, "it strolled ({farthest})");
        let id = server.mobs.iron_golems()[0].id;
        server.mobs.iron_golem_mut(id).unwrap().hurt(55.0);
        let tags = crate::server_mobs::mob_tags(&server.mobs, |_| true, &std::collections::HashMap::new());
        let (_, tag) = tags.iter().find(|(tag_id, _)| *tag_id == id).expect("the golem is saved");
        let mut loaded = minecraftoss_entities::world::EntityWorld::default();
        crate::server_mobs::spawn_saved(&mut loaded, tag).expect("golems load");
        let golem = &loaded.iron_golems()[0].golem;
        assert_eq!(golem.health, 45.0);
        assert!(golem.player_created, "built by a player");
        assert_eq!(golem.crackiness(), minecraftoss_entities::iron_golem::Crackiness::Medium);
    }

    /// A summoned wolf wanders about, and comes back from its saved tag
    /// with its variant, voice, collar, owner and sitting order.
    #[test]
    fn wolves_wander_and_keep_their_tags() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        if registries.entities.is_none() {
            return;
        }
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 6, 8);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        for x in 19..=21 {
            for z in 127..=129 {
                server.load_chunk(&map.load_now(ChunkPos::new(x, z)));
            }
        }
        let surface = minecraftoss_generator::feature::World::height_at(&server.level, minecraftoss_core::chunk::HeightmapKind::MotionBlocking, 328, 2056);
        let at = [328.5, f64::from(surface), 2056.5];
        server.set_players(&[at], 2);
        server.set_time(1000);
        let nbt = minecraftoss_core::snbt::parse_compound("{variant:\"minecraft:woods\",sound_variant:\"minecraft:big\"}").unwrap();
        server.summon("minecraft:wolf", at, Some(&nbt), 0.0).unwrap();
        let start = server.mobs.wolves()[0].wolf.body.position;
        let mut farthest = 0.0_f64;
        for _ in 0..1200 {
            server.set_players(&[at], 2);
            server.set_simulation_area((20, 128), 4);
            server.tick();
            server.tick_mobs(&[], true);
            let wolf = &server.mobs.wolves()[0];
            assert!(wolf.wolf.health > 0.0, "it lives");
            farthest = farthest.max(wolf.wolf.body.position.distance(start));
        }
        assert!(farthest > 1.0, "it wandered ({farthest})");
        let id = server.mobs.wolves()[0].id;
        {
            // Tamed by someone and told to sit, in a blue collar.
            let wolf = &mut server.mobs.wolf_mut(id).unwrap().wolf;
            wolf.owner = Some(0x1234_5678_9abc_def0_0fed_cba9_8765_4321);
            wolf.set_tame(true, true);
            wolf.ordered_to_sit = true;
            wolf.collar = 11;
        }
        let tags = crate::server_mobs::mob_tags(&server.mobs, |_| true, &std::collections::HashMap::new());
        let (_, tag) = tags.iter().find(|(tag_id, _)| *tag_id == id).expect("the wolf is saved");
        let mut loaded = minecraftoss_entities::world::EntityWorld::default();
        crate::server_mobs::spawn_saved(&mut loaded, tag).expect("wolves load");
        let wolf = &loaded.wolves()[0].wolf;
        assert_eq!((wolf.variant.as_str(), wolf.sound_variant.as_str()), ("woods", "big"));
        assert_eq!(wolf.max_health(), 40.0, "taming's health stays");
        assert_eq!(wolf.collar, 11);
        assert_eq!(wolf.owner, Some(0x1234_5678_9abc_def0_0fed_cba9_8765_4321));
        assert!(wolf.tame && wolf.ordered_to_sit && wolf.sitting, "tame and sitting");
    }

    /// A librarian makes its offers from the data JAR's trade sets when
    /// they are first needed; they are saved with it (`Offers`, typed as
    /// vanilla writes them) and come back unchanged, while a villager that
    /// never made any saves none. Its gossip and UUID come back too.
    #[test]
    fn villagers_keep_their_offers_and_gossip() {
        use minecraftoss_core::nbt::Tag;
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let jar = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/.gradle/loom-cache/minecraftMaven/net/minecraft/minecraft-common-1fad6b3808/26.3/minecraft-common-1fad6b3808-26.3.jar");
        if !jar.exists() || registries.entities.is_none() {
            return;
        }
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        server.load_loot(&jar, 1234);
        let nbt = minecraftoss_core::snbt::parse_compound("{NoAI:1b,VillagerData:{type:\"minecraft:plains\",profession:\"minecraft:librarian\",level:3}}").unwrap();
        server.summon("minecraft:villager", [0.5, 100.0, 0.5], Some(&nbt), 0.0).unwrap();
        server.summon("minecraft:villager", [4.5, 100.0, 0.5], Some(&nbt), 0.0).unwrap();
        let (id, idle) = (server.mobs.villagers()[0].id, server.mobs.villagers()[1].id);
        let offers = server.mobs.villager_offers(id).expect("a villager").to_vec();
        let about = server.mobs.uuid_of(idle);
        server.mobs.villager_hurt_by(id, idle);
        std::sync::Arc::make_mut(&mut server.mobs.villager_mut(id).unwrap().gossips).add(7, minecraftoss_entities::gossip::GossipType::Trading, 12);
        server.mobs.villager_mut(id).unwrap().last_gossip_decay = 1234;
        let gossip = server.mobs.villager_mut(id).unwrap().gossips.unpack();
        assert_eq!(gossip.len(), 2);
        // `updateTrades`: the two of its current level's set only.
        assert_eq!(offers.len(), 2, "{offers:?}");
        let tags = crate::server_mobs::mob_tags(&server.mobs, |_| true, &std::collections::HashMap::new());
        let (_, tag) = tags.iter().find(|(tag_id, _)| *tag_id == id).expect("the villager is saved");
        let recipes = tag.get("Offers").and_then(|o| o.get("Recipes")).and_then(Tag::as_list).expect("its offers are saved");
        assert_eq!(recipes.len(), 2);
        assert!(recipes.iter().all(|r| matches!(r.get("priceMultiplier"), Some(Tag::Float(_)))), "price multipliers are floats");
        assert!(matches!(tag.get("LastRestock"), Some(Tag::Long(0))));
        let (_, idle_tag) = tags.iter().find(|(tag_id, _)| *tag_id == idle).unwrap();
        assert!(idle_tag.get("Offers").is_none(), "no offers made, none saved");
        let mut loaded = minecraftoss_entities::world::EntityWorld::default();
        let back = crate::server_mobs::spawn_saved(&mut loaded, tag).expect("villagers load");
        assert_eq!(loaded.villager_mut(back).unwrap().offers.as_deref(), Some(&offers[..]));
        assert_eq!(loaded.villager_mut(back).unwrap().gossips.unpack(), gossip);
        assert_eq!(loaded.villager_mut(back).unwrap().last_gossip_decay, 1234);
        assert_eq!(loaded.uuid_of(back), server.mobs.uuid_of(id), "it keeps its UUID");
        assert!(gossip.iter().any(|g| g.0 == about), "gossip about the other villager");
    }

    /// Using a farmer opens its trades; picking one moves the wheat in,
    /// shift-clicking the result trades until the wheat runs short (levelling
    /// the farmer up and dropping experience), and closing gives the rest
    /// back.
    #[test]
    fn players_trade_with_villagers() {
        use minecraftoss_player::inventory::{Inventory, ItemStack};
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let jar = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/.gradle/loom-cache/minecraftMaven/net/minecraft/minecraft-common-1fad6b3808/26.3/minecraft-common-1fad6b3808-26.3.jar");
        if !jar.exists() || registries.entities.is_none() {
            return;
        }
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        server.load_loot(&jar, 1234);
        let nbt = minecraftoss_core::snbt::parse_compound(
            "{NoAI:1b,Xp:8,VillagerData:{type:\"minecraft:plains\",profession:\"minecraft:farmer\",level:1},Offers:{Recipes:[{buy:{id:\"minecraft:wheat\",count:20},sell:{id:\"minecraft:emerald\",count:1},maxUses:16,xp:2,priceMultiplier:0.05f}]}}",
        )
        .unwrap();
        server.summon("minecraft:villager", [0.5, 100.0, 0.5], Some(&nbt), 0.0).unwrap();
        let id = server.mobs.villagers()[0].id;
        let mut inventory = Inventory::default();
        let mut wheat = ItemStack::new("minecraft:wheat", 50);
        wheat.max = 64;
        inventory.slots[9] = Some(wheat);
        let result = server.mob_action(minecraftoss_entities::world::MobHit::Villager(id), None, inventory.clone(), 0, false);
        let view = result.merchant.expect("the trading screen opens");
        assert_eq!((view.profession, view.level, view.offers.len()), ("minecraft:farmer", 1, 1));
        let apply = |inventory: &mut Inventory, update: &MerchantUpdate| {
            for (slot, stack) in &update.slots {
                inventory.slots[*slot] = stack.clone();
            }
            inventory.cursor = update.cursor.clone();
        };
        let update = server.merchant(MerchantOp::Select(0), inventory.clone(), 0, [0.0; 3]);
        apply(&mut inventory, &update);
        let view = update.view.expect("still open");
        assert_eq!(view.payment[0].as_ref().map(|s| s.count), Some(50), "the wheat moved in");
        assert_eq!(view.result.as_ref().map(|s| s.id.as_str()), Some("minecraft:emerald"));
        let update = server.merchant(MerchantOp::Result { shift: true }, inventory.clone(), 0, [0.0; 3]);
        apply(&mut inventory, &update);
        let view = update.view.expect("still open");
        assert_eq!(view.payment[0].as_ref().map(|s| s.count), Some(10), "two trades of twenty");
        assert_eq!(inventory.count("minecraft:emerald"), 2);
        assert_eq!((view.level, view.xp), (2, 12), "the first trade levelled the farmer up");
        assert!(view.offers.len() > 1, "with its next level's offers");
        assert!(server.level.experience_total() > 0, "trades dropped experience");
        let update = server.merchant(MerchantOp::Close, inventory.clone(), 0, [0.0; 3]);
        apply(&mut inventory, &update);
        assert!(update.view.is_none(), "closed");
        assert_eq!(inventory.count("minecraft:wheat"), 10, "the rest came back");
        assert!(server.mobs.villagers()[0].trading_player.is_none());
    }

    /// A villager claims the bed and bell placed near it by day, and sleeps
    /// in the bed at night: the level's blocks feed the points of interest.
    #[test]
    fn villagers_claim_beds_and_sleep() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        if registries.entities.is_none() {
            return;
        }
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 6, 8);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        for x in 19..=21 {
            for z in 127..=129 {
                server.load_chunk(&map.load_now(ChunkPos::new(x, z)));
            }
        }
        let height = |server: &ServerSim, x, z| minecraftoss_generator::feature::World::height_at(&server.level, minecraftoss_core::chunk::HeightmapKind::MotionBlocking, x, z);
        let surface = height(&server, 328, 2056);
        let place = |server: &mut ServerSim, x: i32, z: i32, block: Block| {
            let y = height(server, x, z);
            let state = server.state_of(Some(&block));
            server.level.set_block_and_update(minecraftoss_core::BlockPos::new(x, y, z), state);
            (x, y, z)
        };
        let bell = place(&mut server, 330, 2056, Block::new("minecraft:bell"));
        let bed = |part: &str| Block::new("minecraft:red_bed").with("facing", "east").with("part", part).with("occupied", "false");
        place(&mut server, 326, 2058, bed("foot"));
        let head = place(&mut server, 327, 2058, bed("head"));
        let at = [328.5, f64::from(surface), 2056.5];
        server.set_players(&[at], 2);
        server.set_time(1000);
        server.summon("minecraft:villager", at, None, 0.0).unwrap();
        let tick = |server: &mut ServerSim| {
            server.set_players(&[at], 2);
            server.set_simulation_area((20, 128), 4);
            server.tick();
            server.tick_mobs(&[], true);
        };
        for _ in 0..200 {
            tick(&mut server);
        }
        let memories = &server.mobs.villagers()[0].ai.as_ref().unwrap().brain.memories;
        assert_eq!(memories.home.get().copied(), Some(head), "it claims the bed");
        assert_eq!(memories.meeting_point.get().copied(), Some(bell), "it claims the bell");
        assert_eq!(server.mobs.pois.record(head).map(|r| r.free_tickets), Some(0));
        server.set_time(12500);
        let mut slept = false;
        for _ in 0..600 {
            tick(&mut server);
            slept |= server.mobs.villagers()[0].sleeping == Some(head);
        }
        assert!(slept, "it sleeps in its bed at night");
        // A lectern nearby by day: it becomes a librarian and works there.
        let lectern = place(&mut server, 331, 2054, Block::new("minecraft:lectern"));
        server.set_time(1800);
        let mut worked = false;
        for _ in 0..800 {
            tick(&mut server);
            worked |= server.mobs.villagers()[0].ai.as_ref().unwrap().brain.running().iter().any(|b| b == "RunOne:WorkAtPoi");
        }
        let villager = &server.mobs.villagers()[0];
        assert_eq!(villager.villager.profession, minecraftoss_entities::villager::Profession::Librarian, "it takes the lectern's profession");
        assert_eq!(villager.ai.as_ref().unwrap().brain.memories.job_site.get().copied(), Some(lectern));
        assert!(worked, "it works at its lectern");
    }

    /// A farmer takes the composter by its field, harvests the ripe wheat
    /// (the level drops its loot, which the farmer picks up) and sows the
    /// bare farmland again.
    #[test]
    fn farmers_harvest_and_sow() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        if registries.entities.is_none() {
            return;
        }
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 6, 8);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        for x in 19..=21 {
            for z in 127..=129 {
                server.load_chunk(&map.load_now(ChunkPos::new(x, z)));
            }
        }
        let height = |server: &ServerSim, x, z| minecraftoss_generator::feature::World::height_at(&server.level, minecraftoss_core::chunk::HeightmapKind::MotionBlocking, x, z);
        let ground = height(&server, 328, 2056);
        let set = |server: &mut ServerSim, (x, y, z): (i32, i32, i32), block: Block| {
            let state = server.state_of(Some(&block));
            server.level.set_block_and_update(minecraftoss_core::BlockPos::new(x, y, z), state);
        };
        // A flat stone yard with a three-by-three field of ripe wheat beside
        // a composter.
        for x in 322..=334 {
            for z in 2050..=2062 {
                for y in ground..ground + 4 {
                    set(&mut server, (x, y, z), Block::new("minecraft:air"));
                }
                set(&mut server, (x, ground - 1, z), Block::new("minecraft:stone"));
            }
        }
        let mut field = Vec::new();
        for x in 328..=330 {
            for z in 2055..=2057 {
                set(&mut server, (x, ground - 1, z), Block::new("minecraft:farmland").with("moisture", "7"));
                set(&mut server, (x, ground, z), Block::new("minecraft:wheat").with("age", "7"));
                field.push((x, ground, z));
            }
        }
        set(&mut server, (327, ground, 2056), Block::new("minecraft:composter").with("level", "0"));
        let at = [326.5, f64::from(ground), 2056.5];
        server.set_players(&[at], 2);
        server.set_time(3000);
        let farmer = minecraftoss_core::snbt::parse_compound(
            "{CanPickUpLoot:1b,Xp:1,VillagerData:{type:\"minecraft:plains\",profession:\"minecraft:farmer\",level:1},Inventory:[{id:\"minecraft:wheat_seeds\",count:4}]}",
        )
        .unwrap();
        server.summon("minecraft:villager", at, Some(&farmer), 0.0).unwrap();
        let age = |server: &ServerSim, (x, y, z): (i32, i32, i32)| {
            let state = server.level.block(minecraftoss_core::BlockPos::new(x, y, z));
            let blocks = &server.level.registries().blocks;
            (blocks.block(blocks.block_of(state)).name.as_str().to_owned(), blocks.property(state, "age").map(str::to_owned))
        };
        let mut sown = false;
        for _ in 0..2400 {
            server.set_players(&[at], 2);
            server.set_simulation_area((20, 128), 4);
            server.tick();
            server.tick_mobs(&[], true);
            sown |= field.iter().any(|&pos| age(&server, pos) == ("minecraft:wheat".to_owned(), Some("0".to_owned())));
            if sown {
                break;
            }
        }
        let villager = &server.mobs.villagers()[0];
        assert!(villager.ai.as_ref().unwrap().brain.memories.job_site.get().is_some(), "it takes the composter");
        assert!(sown, "it harvested ripe wheat and sowed the farmland again");
        assert!(field.iter().any(|&pos| age(&server, pos).1.as_deref() != Some("7")), "a ripe crop was harvested");
    }

    /// A villager with plenty of bread throws some to one with none as they
    /// meet, and the hungry one picks it up from the level.
    #[test]
    fn villagers_share_food() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        if registries.entities.is_none() {
            return;
        }
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 6, 8);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        for x in 19..=21 {
            for z in 127..=129 {
                server.load_chunk(&map.load_now(ChunkPos::new(x, z)));
            }
        }
        let height = |server: &ServerSim, x, z| minecraftoss_generator::feature::World::height_at(&server.level, minecraftoss_core::chunk::HeightmapKind::MotionBlocking, x, z);
        let at = [328.5, f64::from(height(&server, 328, 2056)), 2056.5];
        server.set_players(&[at], 2);
        server.set_time(1000);
        let plenty = minecraftoss_core::snbt::parse_compound("{CanPickUpLoot:1b,Inventory:[{id:\"minecraft:bread\",count:30}]}").unwrap();
        let none = minecraftoss_core::snbt::parse_compound("{CanPickUpLoot:1b}").unwrap();
        server.summon("minecraft:villager", at, Some(&plenty), 0.0).unwrap();
        server.summon("minecraft:villager", [at[0] + 2.0, at[1], at[2] + 1.0], Some(&none), 90.0).unwrap();
        let hungry = server.mobs.villagers()[1].id;
        let fed = |server: &ServerSim| server.mobs.villagers().iter().find(|v| v.id == hungry).unwrap().inventory.count("minecraft:bread");
        for _ in 0..1200 {
            server.set_players(&[at], 2);
            server.set_simulation_area((20, 128), 4);
            server.tick();
            server.tick_mobs(&[], true);
            if fed(&server) > 0 {
                break;
            }
        }
        assert_eq!(fed(&server), 6, "six loaves thrown (thirty less twenty-four) and picked up");
        assert_eq!(server.mobs.villagers()[0].inventory.count("minecraft:bread"), 24);
    }

    /// Two villagers with bread near free beds have a baby, which takes a
    /// bed; they eat their bread and rest from breeding.
    #[test]
    fn villagers_breed_into_a_free_bed() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        if registries.entities.is_none() {
            return;
        }
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 6, 8);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        for x in 19..=21 {
            for z in 127..=129 {
                server.load_chunk(&map.load_now(ChunkPos::new(x, z)));
            }
        }
        let height = |server: &ServerSim, x, z| minecraftoss_generator::feature::World::height_at(&server.level, minecraftoss_core::chunk::HeightmapKind::MotionBlocking, x, z);
        let place = |server: &mut ServerSim, x: i32, z: i32, block: Block| {
            let y = height(server, x, z);
            let state = server.state_of(Some(&block));
            server.level.set_block_and_update(minecraftoss_core::BlockPos::new(x, y, z), state);
            (x, y, z)
        };
        let bed = |part: &str| Block::new("minecraft:red_bed").with("facing", "east").with("part", part).with("occupied", "false");
        let mut beds = Vec::new();
        for z in [2052, 2055, 2058] {
            place(&mut server, 326, z, bed("foot"));
            beds.push(place(&mut server, 327, z, bed("head")));
        }
        let at = [329.5, f64::from(height(&server, 329, 2055)), 2055.5];
        server.set_players(&[at], 2);
        server.set_time(1000);
        let fed = minecraftoss_core::snbt::parse_compound("{CanPickUpLoot:1b,Inventory:[{id:\"minecraft:bread\",count:3}]}").unwrap();
        server.summon("minecraft:villager", at, Some(&fed), 0.0).unwrap();
        server.summon("minecraft:villager", [at[0] + 2.0, at[1], at[2] + 1.0], Some(&fed), 90.0).unwrap();
        let parents: Vec<u64> = server.mobs.villagers().iter().map(|v| v.id).collect();
        for _ in 0..2400 {
            server.set_players(&[at], 2);
            server.set_simulation_area((20, 128), 4);
            server.tick();
            server.tick_mobs(&[], true);
            if server.mobs.villagers().len() > 2 {
                break;
            }
        }
        let villagers = server.mobs.villagers();
        let baby = villagers.iter().find(|v| !parents.contains(&v.id)).expect("the villagers had a baby");
        assert!(baby.villager.age.baby());
        let home = baby.ai.as_ref().unwrap().brain.memories.home.get().copied();
        assert!(home.is_some_and(|h| beds.contains(&h)), "the baby has a bed: {home:?}");
        for parent in villagers.iter().filter(|v| parents.contains(&v.id)) {
            assert!(parent.villager.age.ticks > 0, "the parents rest from breeding");
            assert_eq!(parent.inventory.food_points(), 0, "the parents ate their bread");
        }
    }

    /// A villager that can pick things up walks to bread lying near it and
    /// takes it; one summoned without `CanPickUpLoot` leaves its bread be,
    /// and the flag survives saving.
    #[test]
    fn villagers_pick_up_food() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        if registries.entities.is_none() {
            return;
        }
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 6, 8);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        for x in 19..=21 {
            for z in 127..=129 {
                server.load_chunk(&map.load_now(ChunkPos::new(x, z)));
            }
        }
        let height = |server: &ServerSim, x, z| minecraftoss_generator::feature::World::height_at(&server.level, minecraftoss_core::chunk::HeightmapKind::MotionBlocking, x, z);
        let at = [328.5, f64::from(height(&server, 328, 2056)), 2056.5];
        let other = [334.5, f64::from(height(&server, 334, 2062)), 2062.5];
        server.set_players(&[at], 2);
        server.set_time(1000);
        let loot = minecraftoss_core::snbt::parse_compound("{CanPickUpLoot:1b}").unwrap();
        server.summon("minecraft:villager", at, Some(&loot), 0.0).unwrap();
        server.summon("minecraft:villager", other, None, 0.0).unwrap();
        let (picker, idle) = (server.mobs.villagers()[0].id, server.mobs.villagers()[1].id);
        let bread = |count| minecraftoss_world::level::container::Stack::new("minecraft:bread", count);
        let near = [at[0] + 2.0, f64::from(height(&server, 330, 2056)), at[2]];
        let kept = server.level.spawn_item_with([other[0] + 0.5, other[1], other[2]], bread(2), [0.0; 3], 0, 0);
        let taken = server.level.spawn_item_with(near, bread(3), [0.0; 3], 0, 0);
        for _ in 0..200 {
            server.set_players(&[at], 2);
            server.set_simulation_area((20, 128), 4);
            server.tick();
            server.tick_mobs(&[], true);
        }
        let villager = |id| server.mobs.villagers().iter().find(|v| v.id == id).unwrap();
        assert_eq!(villager(picker).inventory.food_points(), 12, "it took the three loaves");
        assert!(server.level.item_entity(taken).is_none(), "the bread is gone");
        assert!(!villager(idle).can_pick_up_loot, "a summoned villager is told whether it may");
        assert_eq!(villager(idle).inventory.food_points(), 0);
        assert_eq!(server.level.item_entity(kept).map(|(_, stack, _)| stack.count), Some(2), "its bread lies where it was");
        let tags = crate::server_mobs::mob_tags(&server.mobs, |_| true, &std::collections::HashMap::new());
        let (_, tag) = tags.iter().find(|(tag_id, _)| *tag_id == picker).unwrap();
        let mut loaded = minecraftoss_entities::world::EntityWorld::default();
        let back = crate::server_mobs::spawn_saved(&mut loaded, tag).unwrap();
        let back = loaded.villager_mut(back).unwrap();
        assert!(back.can_pick_up_loot);
        assert_eq!(back.inventory.food_points(), 12, "its inventory is saved");
    }

    #[test]
    fn summon_makes_mobs() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        if registries.entities.is_none() {
            return;
        }
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 6, 8);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        for x in 19..=21 {
            for z in 127..=129 {
                server.load_chunk(&map.load_now(ChunkPos::new(x, z)));
            }
        }
        let surface = minecraftoss_generator::feature::World::height_at(&server.level, minecraftoss_core::chunk::HeightmapKind::MotionBlocking, 328, 2056);
        let at = [328.5, f64::from(surface), 2056.5];
        server.set_players(&[at], 2);
        let before = server.mobs.zombies().len();
        server.summon("minecraft:husk", at, None, 1.5).unwrap();
        let nbt = minecraftoss_core::snbt::parse_compound(r#"{IsBaby:1b,PersistenceRequired:1b,VillagerData:{type:"minecraft:desert",profession:"minecraft:farmer",level:1}}"#).unwrap();
        server.summon("minecraft:zombie_villager", [at[0] + 2.0, at[1], at[2]], Some(&nbt), 0.0).unwrap();
        let zombies = server.mobs.zombies();
        assert_eq!(zombies.len(), before + 2, "both join the entity world");
        let husk = zombies.iter().find(|z| z.zombie.kind == minecraftoss_entities::zombie::ZombieKind::Husk).unwrap();
        assert_eq!(husk.zombie.body.position.to_array(), at, "at the command's position");
        let villager = zombies.iter().find(|z| z.zombie.kind == minecraftoss_entities::zombie::ZombieKind::ZombieVillager).unwrap();
        assert!(villager.zombie.baby && villager.zombie.persistence_required, "the NBT is loaded");
        assert_eq!(villager.zombie.villager, Some(("minecraft:desert".to_owned(), "minecraft:farmer".to_owned())));
        server.summon("minecraft:cow", at, None, 0.0).unwrap();
        assert_eq!(server.mobs.cows().len(), 1, "animals are summoned too");
        // A villager loads without its brain (not ported) instead of
        // tripping the entity world's assertion.
        server.summon("minecraft:villager", at, None, 0.0).unwrap();
        assert_eq!(server.mobs.villagers().len(), 1);
        // `NoAI` holds a mob still, turned to its saved yaw.
        let still = minecraftoss_core::snbt::parse_compound("{NoAI:1b,Rotation:[180f,0f]}").unwrap();
        server.summon("minecraft:zombie", [at[0] - 2.0, at[1], at[2]], Some(&still), 0.0).unwrap();
        let zombie = server.mobs.zombies().iter().find(|z| z.no_ai).expect("a NoAI zombie");
        assert_eq!((zombie.yaw, zombie.body_rotation.body_yaw, zombie.look_control.head_yaw), (180.0, 180.0, 180.0));
        let id = zombie.id;
        for _ in 0..20 {
            server.tick_mobs(&[], false);
        }
        let zombie = server.mobs.zombies().iter().find(|z| z.id == id).unwrap();
        assert_eq!(zombie.zombie.body.position.to_array(), [at[0] - 2.0, at[1], at[2]], "it stays put");
        // A horse loads its variant and grazing (`EatingHaystack`), which
        // lasts fifty ticks though it has no AI.
        let grazing = minecraftoss_core::snbt::parse_compound("{NoAI:1b,Variant:1027,EatingHaystack:1b,Age:-24000}").unwrap();
        server.summon("minecraft:horse", [at[0] + 4.0, at[1], at[2]], Some(&grazing), 0.0).unwrap();
        let horse = server.mobs.cows().iter().find_map(|c| c.horse.clone()).expect("a horse");
        assert_eq!((horse.variant, horse.eating), (1027, true));
        for _ in 0..40 {
            server.set_simulation_area((20, 128), 4);
            server.tick();
            server.tick_mobs(&[], false);
        }
        let horse = server.mobs.cows().iter().find_map(|c| c.horse.clone()).unwrap();
        assert!(horse.eating && horse.eat_anim == 1.0, "{horse:?}");
        server.set_players(&[at], 0);
        assert!(server.summon("minecraft:zombie", at, None, 0.0).is_err(), "no monsters in peaceful");
        assert!(server.summon("minecraft:cow", at, None, 0.0).is_ok(), "animals come in peaceful");
    }

    /// A creeper sees the player across a platform, walks up, swells and
    /// blows up: the blast breaks the planks (mobs may grief), hurts and
    /// pushes the player, and the client hears of it.
    #[test]
    fn creeper_stalks_and_blows_up_the_player() {
        use minecraftoss_core::nbt::Tag;
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 0).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 0, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 2, 4);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        for x in -1..=2 {
            for z in -1..=1 {
                server.load_chunk(&map.load_now(ChunkPos::new(x, z)));
            }
        }
        // A plank platform high above the terrain.
        for x in 0..=24 {
            for z in 0..=12 {
                server.player_edit_block((x, 200, z), Some(&Block::new("minecraft:oak_planks")), PlayerEdit::Place);
            }
        }
        server.take_changes();
        let player = minecraftoss_entities::tempt::PlayerCandidate {
            id: 0,
            position: glam::DVec3::new(18.5, 201.0, 6.5),
            eye_height: 1.62,
            main_hand_cow_food: false,
            offhand_cow_food: false,
            main_hand_pig_food: false,
            offhand_pig_food: false,
            main_hand_chicken_food: false,
            offhand_chicken_food: false,
            main_hand_carrot_on_a_stick: false,
            offhand_carrot_on_a_stick: false,
            main_hand_wolf_interest: false,
            offhand_wolf_interest: false,
            main_hand_horse_tempt: false,
            offhand_horse_tempt: false,
            alive: true,
            spectator: false,
            attackable: true,
        };
        let mut tag = std::collections::BTreeMap::new();
        tag.insert("id".to_owned(), Tag::String("minecraft:creeper".to_owned()));
        tag.insert("Pos".to_owned(), Tag::List(vec![Tag::Double(8.5), Tag::Double(201.0), Tag::Double(6.5)]));
        tag.insert("Rotation".to_owned(), Tag::List(vec![Tag::Float(0.0), Tag::Float(0.0)]));
        tag.insert("UUID".to_owned(), Tag::IntArray(vec![1, 2, 3, 4]));
        let id = crate::server_mobs::spawn_saved(&mut server.mobs, &Tag::Compound(tag)).expect("creepers are simulated");
        assert!(server.mobs.creepers().iter().any(|c| c.id == id && !c.no_ai));
        server.set_time(18000);
        let (mut hits, mut blasts, mut broken) = (Vec::new(), Vec::new(), 0);
        for _ in 0..400 {
            server.set_players(&[player.position.to_array()], 2);
            server.set_simulation_area((0, 0), 4);
            server.tick();
            server.tick_mobs(&[player], false);
            hits.extend(server.mobs.take_player_hits());
            blasts.extend(server.take_explosions());
            broken += server.take_changes().iter().filter(|(pos, block)| pos.1 == 200 && block.is_none()).count();
            if !blasts.is_empty() {
                break;
            }
        }
        assert_eq!(blasts.len(), 1, "the creeper explodes");
        assert!(server.mobs.creepers().iter().all(|c| c.id != id), "and is gone");
        let blast = blasts[0];
        assert!(blast.position.distance(player.position) < 3.5, "beside the player: {:?}", blast.position);
        assert!(broken > 0, "the blast breaks planks");
        let hit = hits.iter().find(|h| matches!(h.kind, minecraftoss_entities::world::PlayerHitKind::Explosion { .. })).expect("the blast hits the player");
        assert!(hit.damage > 10.0, "a close blast hurts: {}", hit.damage);
    }

    /// A player's wheat makes a generated cow fall in love (using up the
    /// wheat), and hits kill it, dropping its loot into the level.
    #[test]
    fn players_feed_and_hit_server_mobs() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let jar = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/.gradle/loom-cache/minecraftMaven/net/minecraft/minecraft-common-1fad6b3808/26.3/minecraft-common-1fad6b3808-26.3.jar");
        if !jar.exists() {
            return;
        }
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 4, 8);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        server.load_loot(&jar, 1234);
        let recipes = Arc::new(minecraftoss_player::crafting::RecipeBook::from_jar(&jar).unwrap());
        server.set_recipe_book(recipes.clone());
        for x in 20..=23 {
            for z in 129..=132 {
                server.load_chunk(&map.load_now(ChunkPos::new(x, z)));
            }
        }
        server.set_simulation_area((21, 130), 4);
        let cow = server.mobs.cows().iter().find(|c| !c.cow.age.baby()).map(|c| c.id).expect("an adult cow near (356, 65, 2097)");
        let hit = minecraftoss_entities::world::MobHit::Cow(cow);
        let mut inventory = minecraftoss_player::inventory::Inventory::default();
        inventory.recipes = recipes;
        inventory.slots[0] = Some(minecraftoss_player::inventory::ItemStack::new("minecraft:wheat", 3));
        let fed = server.mob_action(hit, None, inventory.clone(), 0, false);
        assert_eq!(fed.slots.len(), 1, "one wheat is used: {:?}", fed.slots);
        assert_eq!(fed.slots[0].1.as_ref().map(|s| s.count), Some(2));
        assert!(server.mobs.cow_mut(cow).unwrap().cow.in_love > 0, "the cow falls in love");
        // Full-strength bare-handed hits, one point each past each hurt
        // cooldown, until it dies.
        let items_before = server.items().len();
        let mut sounds = Vec::new();
        for _ in 0..40 {
            let at = server.mobs.cows().iter().find(|c| c.id == cow).map(|c| c.cow.body.position);
            let Some(at) = at else { break };
            let attack = minecraftoss_entities::world::PlayerAttack {
                player_id: 0,
                position: at + glam::DVec3::X,
                yaw: 90.0,
                attack_damage: 1.0,
                strength: 1.0,
                sprinting: false,
                can_critical: false,
                can_sweep: false,
            };
            let result = server.mob_action(hit, Some(attack), inventory.clone(), 0, false);
            sounds.extend(result.sounds.into_iter().map(|s| s.event).filter(|s| s.starts_with("entity.cow")));
            for _ in 0..11 {
                server.tick_mobs(&[], true);
            }
            if sounds.last().is_some_and(|s| s.ends_with("death")) {
                break;
            }
        }
        assert!(sounds.first().is_some_and(|s| s.ends_with(".hurt")), "{sounds:?}");
        assert!(sounds.last().is_some_and(|s| s.ends_with(".death")), "the cow dies: {sounds:?}");
        assert!(server.items().len() > items_before, "its loot drops into the level");
    }

    /// Five experience land as orbs of vanilla's sizes (3, 1 and 1); the
    /// orbs are drawn to a player standing beside them, who takes one every
    /// other tick until none are left.
    #[test]
    fn experience_orbs_are_awarded_and_taken() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 0).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 0, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 2, 4);
        let mut server = ServerSim::new(worldgen, states.clone(), "minecraft:overworld");
        let mut scene = HandcraftedScene::streamed(states);
        for x in -1..=1 {
            for z in -1..=1 {
                let chunk = map.load_now(ChunkPos::new(x, z));
                server.load_chunk(&chunk);
                scene.insert_chunk(chunk);
            }
        }
        // A floating platform, high above the terrain.
        for x in 2..=14 {
            for z in 2..=14 {
                scene.set((x, 200, z), Some(Block::new("minecraft:stone")));
                server.player_edit(&scene, (x, 200, z), PlayerEdit::Place);
            }
        }
        server.award_experience([8.5, 201.0, 8.5], 5);
        let mut values: Vec<i32> = server.orbs().iter().map(|o| o.value).collect();
        values.sort_unstable();
        assert_eq!(values, [1, 1, 3]);
        let feet = [10.5, 201.0, 8.5];
        server.level.living_players = vec![(feet, 1.62)];
        let mut taken = Vec::new();
        for tick in 0..60 {
            server.tick();
            if let Some((_, _, value)) = server.take_experience(Some(feet)) {
                taken.push((tick, value));
            }
        }
        assert_eq!(taken.iter().map(|(_, v)| v).sum::<i32>(), 5, "every orb is taken: {taken:?}");
        assert!(taken.windows(2).all(|w| w[1].0 - w[0].0 >= 2), "one orb every other tick at most: {taken:?}");
        assert!(server.orbs().is_empty());
    }

    /// `cargo test --release -p minecraftoss-viewer --lib mob_tick_cost -- --ignored --nocapture`
    #[test]
    #[ignore = "timing measurement"]
    fn mob_tick_cost() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 1234).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 1234, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 10, 16);
        let mut server = ServerSim::new(worldgen, states, "minecraft:overworld");
        for x in 10..=30 {
            for z in 118..=138 {
                server.load_chunk(&map.load_now(ChunkPos::new(x, z)));
            }
        }
        server.set_simulation_area((20, 128), 10);
        let mut times = Vec::new();
        for _ in 0..600 {
            let started = std::time::Instant::now();
            server.tick_mobs(&[], true);
            times.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        let p = |q: usize| times[(times.len() - 1) * q / 100];
        eprintln!("{} mobs: tick p50 {:.2} p95 {:.2} p99 {:.2} max {:.2} ms", server.mobs.len(), p(50), p(95), p(99), times[times.len() - 1]);
    }

    /// TNT primed by a placed redstone block becomes an entity the client
    /// can render, explodes, and leaves items the player can pick up.
    #[test]
    fn primed_tnt_explodes_and_drops_items() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 0).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 0, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 2, 4);
        let mut server = ServerSim::new(worldgen, states.clone(), "minecraft:overworld");
        let mut scene = HandcraftedScene::streamed(states);
        for x in -1..=1 {
            for z in -1..=1 {
                let chunk = map.load_now(ChunkPos::new(x, z));
                server.load_chunk(&chunk);
                scene.insert_chunk(chunk);
            }
        }
        // A floating platform of planks with TNT on it, high above terrain.
        for x in 4..=12 {
            for z in 4..=12 {
                scene.set((x, 200, z), Some(Block::new("minecraft:oak_planks")));
                server.player_edit(&scene, (x, 200, z), PlayerEdit::Place);
            }
        }
        scene.set((8, 201, 8), Some(Block::new("minecraft:tnt")));
        server.player_edit(&scene, (8, 201, 8), PlayerEdit::Place);
        scene.set((9, 201, 8), Some(Block::new("minecraft:redstone_block")));
        server.player_edit(&scene, (9, 201, 8), PlayerEdit::Place);
        server.take_changes();
        assert_eq!(server.primed_tnt().len(), 1, "the TNT should prime");
        for _ in 0..90 {
            server.tick();
        }
        assert!(server.primed_tnt().is_empty(), "the TNT should have exploded");
        let changes = server.take_changes();
        assert!(changes.iter().any(|(pos, block)| *pos == (8, 200, 8) && block.is_none()), "the blast should break the planks under it");
        let items = server.items();
        assert!(items.iter().any(|i| i.item == "minecraft:oak_planks"), "broken planks drop: {items:?}");
        // Let the drops land, then pick them up standing among them.
        for _ in 0..60 {
            server.tick();
        }
        let item = server.items().into_iter().find(|i| i.pickup_delay == 0).expect("a landed item");
        let mut taken = 0;
        let picked = server.pickup([item.position[0], item.position[1], item.position[2]], |_, count, _| {
            taken += count;
            count
        });
        assert!(!picked.is_empty() && taken > 0, "the player should pick up touching items");
        assert!(server.items().iter().all(|i| i.id != picked[0].0), "a fully picked-up item is removed");
    }

    /// Sand placed in the air falls as an entity and lands on the ground.
    #[test]
    fn placed_sand_falls_and_lands() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 0).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 0, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 2, 4);
        let mut server = ServerSim::new(worldgen, states.clone(), "minecraft:overworld");
        let mut scene = HandcraftedScene::streamed(states);
        for x in -1..=1 {
            for z in -1..=1 {
                let chunk = map.load_now(ChunkPos::new(x, z));
                server.load_chunk(&chunk);
                scene.insert_chunk(chunk);
            }
        }
        scene.set((8, 200, 8), Some(Block::new("minecraft:stone")));
        server.player_edit(&scene, (8, 200, 8), PlayerEdit::Place);
        scene.set((8, 205, 8), Some(Block::new("minecraft:sand")));
        server.player_edit(&scene, (8, 205, 8), PlayerEdit::Place);
        server.take_changes();
        for _ in 0..3 {
            server.tick();
        }
        assert_eq!(server.falling_blocks().len(), 1, "the sand should be falling");
        for _ in 0..40 {
            server.tick();
        }
        assert!(server.falling_blocks().is_empty(), "the sand should have landed");
        let changes = server.take_changes();
        assert!(changes.iter().any(|(pos, block)| *pos == (8, 201, 8) && block.as_ref().is_some_and(|b| *b == Block::new("minecraft:sand"))), "{changes:?}");
    }

    /// Bone meal the player uses on a crop grows it through the level.
    #[test]
    fn bone_meal_grows_a_crop() {
        let Ok(paths) = DataPaths::discover() else { return };
        let Ok(registries) = Registries::load(&paths) else { return };
        let registries = Arc::new(registries);
        let worldgen = Arc::new(WorldGen::new(Arc::new(TerrainGenerator::overworld(registries.clone(), 0).unwrap())).unwrap());
        let states = Arc::new(BlockStates::new(registries.clone(), 0, -64, 384).unwrap());
        let mut map = ChunkMap::with_worldgen(worldgen.clone(), 2, 4);
        let mut server = ServerSim::new(worldgen, states.clone(), "minecraft:overworld");
        let mut scene = HandcraftedScene::streamed(states);
        for x in -1..=1 {
            for z in -1..=1 {
                let chunk = map.load_now(ChunkPos::new(x, z));
                server.load_chunk(&chunk);
                scene.insert_chunk(chunk);
            }
        }
        scene.set((8, 200, 8), Some(Block::new("minecraft:farmland")));
        server.player_edit(&scene, (8, 200, 8), PlayerEdit::Place);
        scene.set((8, 201, 8), Some(Block::new("minecraft:wheat")));
        server.player_edit(&scene, (8, 201, 8), PlayerEdit::Place);
        server.take_changes();
        assert!(server.bone_meal((8, 201, 8), "up"), "wheat takes bone meal");
        let changes = server.take_changes();
        let grown = changes.iter().find(|(pos, _)| *pos == (8, 201, 8)).and_then(|(_, b)| b.clone()).expect("the wheat changed");
        let age: i32 = grown.properties.get("age").and_then(|a| a.parse().ok()).unwrap_or(0);
        assert!((2..=5).contains(&age), "bone meal adds 2 to 5 ages: {age}");
        assert!(!server.bone_meal((8, 200, 8), "up"), "farmland does not take bone meal");
    }
}
