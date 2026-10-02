//! Villagers in the entity world (pinned 26.3 `Villager`, `AbstractVillager`,
//! `AgeableMob`, `Mob.serverAiStep` and `LivingEntity.aiStep`): a living
//! villager's tick runs its brain (`villager_brain`) between the
//! navigation's tick and the controls, then moves, meets the blocks it went
//! through, pushes, and ages. A villager without AI only settles, meets its
//! blocks, pushes and ages.
use super::*;
use crate::villager_brain::{Brain, CtxParts, Me, OffersAccess, PathObjects, Remote, Seen};

/// The trade data and the level's trade sequences a brain may make offers
/// with (`ShowTradesToPlayer` reads `getOffers`).
struct TradeArgs<'a> {
    book: Option<&'a crate::trading::TradeBook>,
    sequences: &'a mut crate::trading::TradeSequences,
}

const VILLAGER_SOUNDS: MovementSounds = MovementSounds::creature(None);

/// A villager's brain and controls.
#[derive(Clone)]
pub struct VillagerAi {
    pub brain: Brain,
    pub navigation: GroundNavigation,
    pub paths: PathObjects,
    pub move_control: MoveControl,
    pub look_control: LookControl,
    pub body_rotation: BodyRotation,
    pub yaw: f32,
    pub speed: f32,
    pub forward: f32,
    pub sideways: f32,
    pub jumping: bool,
    pub no_jump_delay: i32,
    /// `Sensing`'s line-of-sight answers this tick.
    sight: HashMap<u64, bool>,
    pub last_gossip_time: i64,
    profile: WalkProfile,
    /// `Villager.die` gave back its points of interest.
    pois_released: bool,
}

impl VillagerAi {
    /// A new brain: its sensors wait from the villager's random.
    fn new(villager: &Villager, yaw: f32, random: &mut LegacyRandom, time: i64, day_time: i64) -> Self {
        let baby = villager.age.baby();
        let mut profile = WalkProfile::animal(villager.body.width, villager.body.height);
        // A `PathfinderMob`: no fire mali; doors open; paths up to 48.
        profile.clear_malus(PathType::FireInNeighbor);
        profile.clear_malus(PathType::Fire);
        profile.can_open_doors = true;
        profile.max_path_length = Some(48.0);
        profile.max_visited = Some(768);
        profile.walk_target = crate::walk_path::WalkTarget::Zero;
        Self {
            brain: Brain::new(baby, villager.profession, random, time, day_time),
            navigation: GroundNavigation::default(),
            paths: PathObjects::default(),
            move_control: MoveControl::default(),
            look_control: LookControl::new(yaw),
            body_rotation: BodyRotation::new(yaw),
            yaw,
            speed: 0.0,
            forward: 0.0,
            sideways: 0.0,
            jumping: false,
            no_jump_delay: 0,
            sight: HashMap::new(),
            last_gossip_time: 0,
            profile,
            pois_released: false,
        }
    }
}

/// An entity's eye height, by kind.
fn seen_of(kind: &'static str, category: &'static str, body: &Body, eye_height: f32, alive: bool, baby: bool, id: u64) -> Seen {
    Seen {
        id,
        kind,
        category,
        position: body.position,
        eye_height,
        width: body.width,
        height: body.height,
        alive,
        baby,
        spectator: false,
        interaction_target: None,
        sleeping: false,
        can_breed: false,
        food_points: 0,
        profession: None,
        job_site: None,
        potential_job_site: false,
        xp: 0,
        path_step: None,
        last_gossip_time: 0,
        wants_golem: false,
        gossips: None,
        main_hand: None,
    }
}

impl EntityWorld {
    /// A villager with its brain, facing `yaw`.
    pub fn spawn_villager_active(&mut self, villager: Villager, yaw: f32) -> u64 {
        let id = self.spawn_villager(villager, true);
        self.activate_villager(id, yaw);
        id
    }

    /// Gives a villager its brain (`makeBrain`: the sensors' first scans
    /// from its random as it stands), facing `yaw`.
    pub fn activate_villager(&mut self, id: u64, yaw: f32) {
        let (time, day_time) = (self.game_time, self.day_time);
        let mut uniquifier = self.seed_uniquifier;
        let Some(entity) = self.villager_mut(id) else { return };
        entity.no_ai = false;
        let mut ai = VillagerAi::new(&entity.villager, yaw, &mut entity.random, time, day_time);
        // `RandomSupport.generateUniqueSeed`, less the clock.
        ai.brain.seed_unseeded(|| {
            uniquifier = uniquifier.wrapping_mul(1_181_783_497_276_652_981);
            uniquifier
        });
        entity.ai = Some(Box::new(ai));
        self.seed_uniquifier = uniquifier;
    }

    /// The trade data villagers make offers from, and the world seed their
    /// trade sets' random sequences start from.
    pub fn set_trades(&mut self, book: Arc<crate::trading::TradeBook>, world_seed: u64) {
        self.trades = Some(book);
        self.trade_sequences = crate::trading::TradeSequences::new(world_seed);
    }

    /// `AbstractVillager.getOffers`: the villager's offers, made from its
    /// profession's trade set for its level the first time they are needed.
    pub fn villager_offers(&mut self, id: u64) -> Option<&[crate::trading::MerchantOffer]> {
        let book = self.trades.clone();
        let entity = self.villagers.iter_mut().find(|e| e.id == id)?;
        if entity.offers.is_none() {
            // Without trade data there is nothing to make them from yet.
            let Some(book) = book else { return Some(&[]) };
            let v = &entity.villager;
            entity.offers = Some(book.villager_offers(&v.kind, v.profession.id(), v.level, &mut self.trade_sequences));
        }
        entity.offers.as_deref()
    }

    /// `Villager.shouldRestock` then `restock` (`WorkAtPoi.start`): half a
    /// day since the last restock or a new overworld day restarts the
    /// count; at most two restocks a day, the second two minutes after the
    /// first, and only when some offer was used. A restock updates demand
    /// and resets every offer's uses.
    pub(super) fn villager_work_restock(&mut self, id: u64) {
        let (game_time, day) = (self.game_time, self.day_time.div_euclid(24_000));
        if self.villager_offers(id).is_none() {
            return;
        }
        let Some(entity) = self.villagers.iter_mut().find(|e| e.id == id) else { return };
        let mut new_day = game_time > entity.last_restock + 12_000;
        new_day |= entity.last_restock_check_day > 0 && day > entity.last_restock_check_day;
        entity.last_restock_check_day = day;
        let offers: &mut [crate::trading::MerchantOffer] = entity.offers.as_deref_mut().unwrap_or_default();
        if new_day {
            entity.last_restock = game_time;
            // `resetNumberOfRestocks`: `catchUpDemand` first.
            let missed = 2 - entity.restocks_today;
            if missed > 0 {
                offers.iter_mut().for_each(|o| o.uses = 0);
            }
            for _ in 0..missed {
                offers.iter_mut().for_each(|o| o.demand = o.demand + o.uses - (o.max_uses - o.uses));
            }
            entity.restocks_today = 0;
        }
        let allowed = entity.restocks_today == 0 || (entity.restocks_today < 2 && game_time > entity.last_restock + 2400);
        let needed = offers.iter().any(|o| o.uses > 0);
        if allowed && needed {
            offers.iter_mut().for_each(|o| o.demand = o.demand + o.uses - (o.max_uses - o.uses));
            offers.iter_mut().for_each(|o| o.uses = 0);
            entity.last_restock = game_time;
            entity.restocks_today += 1;
        }
    }

    /// `Mob.aiStep`'s looting for a villager: with mob griefing on, each
    /// item it touches (its box grown by 1, 0, 1) that can be picked up and
    /// that it wants goes into its inventory (`InventoryCarrier.pickUpItem`),
    /// the item keeping what does not fit.
    fn villager_pick_up(&mut self, id: u64, world: &mut impl World) {
        if !self.mob_griefing {
            return;
        }
        let Some(entity) = self.villagers.iter().find(|e| e.id == id) else { return };
        if !entity.can_pick_up_loot || entity.villager.health <= 0.0 {
            return;
        }
        let body = &entity.villager.body;
        let half = f64::from(body.width) / 2.0;
        let (min, max) = (body.position - DVec3::new(half + 1.0, 0.0, half + 1.0), body.position + DVec3::new(half + 1.0, f64::from(body.height), half + 1.0));
        for item in world.items_in(min, max) {
            if item.count <= 0 || item.pickup_delay > 0 {
                continue;
            }
            let max_stack = self.item_max_stack(&item.item).clamp(1, 99) as u8;
            let Some(entity) = self.villagers.iter_mut().find(|e| e.id == id) else { return };
            let profession = entity.villager.profession;
            if !crate::villager_inventory::wants_to_pick_up(&entity.inventory, profession, &item.item, item.components.as_ref()) {
                continue;
            }
            let count = item.count.clamp(0, 255) as u8;
            let stack = minecraftoss_player::inventory::ItemStack { id: item.item.clone(), count, max: max_stack, components: item.components.clone() };
            let left = entity.inventory.add(stack).map_or(0, |rest| rest.count);
            world.take_item(item.id, i32::from(count - left));
        }
    }

    /// Where the villager brains' clock stands (`Level.getDayTime`: the
    /// overworld clock).
    pub fn set_day_time(&mut self, day_time: i64) {
        self.day_time = day_time;
    }

    /// The level's random (shared with the world's simulation): brains
    /// draw from it.
    pub fn level_random_mut(&mut self) -> &mut LegacyRandom {
        &mut self.level_random
    }

    /// Every living entity but `except` as brains see it, with the
    /// players.
    fn seen_all(&self, except: u64, players: &[PlayerCandidate]) -> Vec<Seen> {
        let mut out = Vec::new();
        for e in &self.villagers {
            if e.id != except {
                let mut seen = seen_of("minecraft:villager", "misc", &e.villager.body, e.eye_height(), e.villager.health > 0.0, e.villager.age.baby(), e.id);
                seen.interaction_target = e.ai.as_ref().and_then(|ai| ai.brain.memories.interaction_target.get().copied());
                seen.sleeping = e.sleeping.is_some();
                seen.can_breed = crate::villager_inventory::can_breed(e.food_level, &e.inventory, seen.sleeping, e.villager.age.ticks);
                seen.food_points = e.inventory.food_points();
                seen.profession = Some(e.villager.profession);
                seen.xp = e.villager.xp;
                seen.wants_golem = self.wants_golem(e);
                seen.gossips = Some(e.gossips.clone());
                if let Some(ai) = e.ai.as_deref() {
                    seen.last_gossip_time = ai.last_gossip_time;
                    seen.job_site = ai.brain.memories.job_site.get().copied();
                    seen.potential_job_site = ai.brain.memories.potential_job_site.present();
                    let nav = &ai.navigation;
                    if ai.brain.memories.path.present() && nav.next > 0 && nav.next < nav.nodes.len() {
                        seen.path_step = Some((nav.nodes[nav.next - 1], nav.nodes[nav.next]));
                    }
                }
                out.push(seen);
            }
        }
        for e in &self.cows {
            let baby = e.cow.age.baby();
            let kind = if e.mooshroom.is_some() { "minecraft:mooshroom" } else { "minecraft:cow" };
            out.push(seen_of(kind, "creature", &e.cow.body, if baby { 0.665 } else { 1.3 }, e.cow.health > 0.0, baby, e.id));
        }
        for e in &self.pigs {
            let baby = e.pig.age.baby();
            out.push(seen_of("minecraft:pig", "creature", &e.pig.body, if baby { 0.3825 } else { 0.765 }, e.pig.health > 0.0, baby, e.id));
        }
        for e in &self.sheep {
            let baby = e.sheep.age.baby();
            out.push(seen_of("minecraft:sheep", "creature", &e.body, if baby { 0.6175 } else { 1.235 }, e.health > 0.0, baby, e.id));
        }
        for e in &self.chickens {
            let baby = e.chicken.age.baby();
            out.push(seen_of("minecraft:chicken", "creature", &e.chicken.body, if baby { 0.28125 } else { 0.644 }, e.chicken.health > 0.0, baby, e.id));
        }
        for e in &self.zombies {
            out.push(seen_of(e.zombie.kind.type_id(), "monster", &e.zombie.body, e.zombie.eye_height(), e.zombie.health > 0.0, e.zombie.baby, e.id));
        }
        for e in &self.skeletons {
            out.push(seen_of(e.skeleton.kind.type_id(), "monster", &e.skeleton.body, e.skeleton.eye_height(), e.skeleton.health > 0.0, false, e.id));
        }
        for e in self.creepers.iter().filter(|e| !e.creeper.exploded) {
            out.push(seen_of("minecraft:creeper", "monster", &e.creeper.body, e.creeper.body.height * 0.85, e.creeper.health > 0.0, false, e.id));
        }
        for e in &self.spiders {
            out.push(seen_of("minecraft:spider", "monster", &e.spider.body, e.spider.eye_height(), e.spider.health > 0.0, false, e.id));
        }
        for e in &self.slimes {
            out.push(seen_of("minecraft:slime", "monster", &e.slime.body, e.slime.eye_height(), e.slime.health > 0.0, false, e.id));
        }
        for e in &self.endermen {
            out.push(seen_of("minecraft:enderman", "monster", &e.enderman.body, e.enderman.eye_height(), e.enderman.health > 0.0, false, e.id));
        }
        for e in &self.witches {
            out.push(seen_of("minecraft:witch", "monster", &e.witch.body, crate::witch::EYE_HEIGHT, e.witch.health > 0.0, false, e.id));
        }
        for e in &self.bats {
            out.push(seen_of("minecraft:bat", "ambient", &e.bat.body, e.bat.body.height * 0.85, e.bat.health > 0.0, false, e.id));
        }
        for e in &self.iron_golems {
            out.push(seen_of("minecraft:iron_golem", "misc", &e.golem.body, crate::iron_golem::EYE_HEIGHT, e.golem.health > 0.0, false, e.id));
        }
        for e in &self.wolves {
            out.push(seen_of("minecraft:wolf", "creature", &e.wolf.body, e.wolf.eye_height(), e.wolf.health > 0.0, e.wolf.baby(), e.id));
        }
        for p in players {
            out.push(Seen {
                id: PLAYER_TARGET + p.id,
                kind: "minecraft:player",
                category: "misc",
                position: p.position,
                eye_height: p.eye_height,
                width: 0.6,
                height: 1.8,
                alive: p.alive,
                baby: false,
                spectator: p.spectator,
                interaction_target: None,
                sleeping: false,
                can_breed: false,
                food_points: 0,
                profession: None,
                job_site: None,
                potential_job_site: false,
                xp: 0,
                path_step: None,
                last_gossip_time: 0,
                wants_golem: false,
                gossips: None,
                main_hand: self.player_main_hands.get(&p.id).cloned(),
            });
        }
        out
    }

    /// One villager's tick.
    pub(super) fn tick_villager(&mut self, id: u64, world: &mut impl World, players: &[PlayerCandidate], ticks: &dyn Fn(DVec3) -> bool) {
        let (game_time, day_time, griefing) = (self.game_time, self.day_time, self.mob_griefing);
        let mut seen = self.seen_all(id, players);
        // The item entities about it, which it may track like entities, and
        // those it tracks that are gone, where they were.
        if let Some(entity) = self.villagers.iter().find(|e| e.id == id) {
            let body = &entity.villager.body;
            let half = f64::from(body.width) / 2.0;
            let (min, max) = (body.position - DVec3::new(half + 32.0, 16.0, half + 32.0), body.position + DVec3::new(half + 32.0, f64::from(body.height) + 16.0, half + 32.0));
            let items: Vec<_> = world.items_in(min, max).iter().map(crate::villager_brain::seen_item).collect();
            for tracked in &entity.tracked_items {
                match world.item((tracked.id - crate::villager_brain::ITEM_TARGET) as i32) {
                    Some(item) if !items.iter().any(|s| s.id == tracked.id) => seen.push(crate::villager_brain::seen_item(&item)),
                    None => seen.push(tracked.clone()),
                    _ => {}
                }
            }
            seen.extend(items);
        }
        // `getEntitiesOfClass(LivingEntity, box inflated by 16)`: the living
        // entities of the sections that box reaches, in their order.
        let nearby: Vec<u64> = {
            let Some(entity) = self.villagers.iter().find(|e| e.id == id) else { return };
            let body = &entity.villager.body;
            let half = f64::from(body.width) / 2.0;
            let (min, max) = (body.position - DVec3::new(half + 16.0, 16.0, half + 16.0), body.position + DVec3::new(half + 16.0, f64::from(body.height) + 16.0, half + 16.0));
            let mut nearby: Vec<u64> = self.sections.living_around(min, max).into_iter().filter(|&other| other != id).collect();
            // Real players are entities in the level's lookup too (the
            // harness's probe players are not).
            if self.players_pickable {
                for p in players.iter().filter(|p| p.alive && !p.spectator) {
                    let (half, at) = (0.3, p.position);
                    if at.x - half < max.x && at.x + half > min.x && at.y < max.y && at.y + 1.8 > min.y && at.z - half < max.z && at.z + half > min.z {
                        nearby.push(PLAYER_TARGET + p.id);
                    }
                }
            }
            nearby
        };
        let mut remote = Vec::new();
        let mut level_random = std::mem::take(&mut self.level_random);
        let Some(entity) = self.villagers.iter_mut().find(|e| e.id == id) else {
            self.level_random = level_random;
            return;
        };
        entity.previous_position = entity.villager.body.position;
        entity.tick_count += 1;
        let first = entity.tick_count == 1;
        base_tick_fluid(&mut entity.villager.body, &*world, first, &mut entity.random, &mut entity.voices, VILLAGER_SOUNDS);
        hazards::burn(entity, &*world, game_time);
        let eye = entity.eye_height();
        // A sleeper is never in a wall (`LivingEntity.isInWall`).
        if entity.sleeping.is_none() {
            hazards::suffocate(entity, &*world, eye, game_time);
        }
        let water_breathing = entity.effects.has(MobEffect::WaterBreathing);
        if entity.villager.health > 0.0 && breathe(&mut entity.villager.body, &*world, eye, water_breathing) {
            entity.hurt_from(2.0, "minecraft:drown", None, game_time);
        }
        let removed = entity.villager.damage.tick();
        for work in entity.effects.tick(false) {
            match work.resolve(entity.villager.health, 20.0) {
                Some(EffectWork::Heal(amount)) => heal(&mut entity.villager.health, 20.0, amount),
                Some(EffectWork::HurtMagic(amount)) => {
                    entity.hurt_from(amount, "minecraft:magic", None, game_time);
                }
                _ => {}
            }
        }
        if entity.villager.health > 0.0 {
            if (entity.random.next_int(1000) as i32) < entity.ambient_sound_time {
                entity.ambient_sound_time = -80;
                // A sleeper has no ambient sound; a trader's is the trading
                // murmur (`Villager.getAmbientSound`).
                if entity.sleeping.is_none() {
                    let pitch = voice_pitch(&mut entity.random, entity.villager.age.baby());
                    let voice = if entity.trading_player.is_some() { Voice::Event("entity.villager.trade", 1.0, pitch) } else { Voice::Ambient(pitch) };
                    entity.voices.push((voice, entity.position()));
                }
            } else {
                entity.ambient_sound_time += 1;
            }
        }
        if removed {
            self.level_random = level_random;
            return;
        }
        // `LivingEntity.tick`: a sleeper whose bed is gone gets up, and one
        // hurt in its sleep (`hurtServer`, here a tick late) too.
        if let Some(bed) = entity.sleeping {
            let bed_block = world.block(bed).filter(|b| is_bed(&b.id));
            if bed_block.is_none() || entity.wake_pending {
                wake(entity, world, game_time);
            }
        }
        entity.wake_pending = false;
        let dying = entity.villager.health <= 0.0;
        let mut door_updates = Vec::new();
        let mut trampled = None;
        let mut murder = None;
        let (mut traded, mut stop_trading) = (None, false);
        if dying {
            // `Villager.die` -> `releaseAllPois`: each remembered point of
            // interest still of its memory's kind gives its ticket back
            // (vanilla at the killing blow; here on its next tick).
            if let Some(ai) = entity.ai.as_deref_mut().filter(|ai| !ai.pois_released) {
                ai.pois_released = true;
                // `tellWitnessesThatIWasMurdered`: the villagers it saw hear
                // who killed it.
                if let Some(murderer) = entity.last_damage.and_then(|d| d.attacker) {
                    let observer = crate::villager_brain::Observer { id, position: entity.villager.body.position, eye_height: eye };
                    let witnesses = match ai.brain.memories.visible_mobs.get_mut() {
                        Some(visible) => visible.find_all(&observer, &seen, &*world, &mut ai.sight, |s| s.kind == "minecraft:villager"),
                        None => Vec::new(),
                    };
                    murder = Some((murderer, witnesses));
                }
                release_all_pois(&ai.brain.memories, entity.villager.profession, &mut self.pois);
            }
        }
        if dying && !entity.no_ai {
            let yaw = entity.ai.as_ref().map_or(0.0, |ai| ai.yaw);
            let tick_count = entity.tick_count;
            let landed = dying_travel(&mut entity.villager.body, &*world, yaw, tick_count, &mut entity.random, &mut entity.voices, VILLAGER_SOUNDS, None);
            // Its body landing on farmland still draws, and can trample it.
            if landed.is_some_and(|fallen| crate::world::living::fall_on_farmland(&mut entity.villager.body, world, fallen, &mut level_random, griefing)) {
                entity.previous_position = entity.villager.body.position;
            }
            // `Mob.tickHeadTurn`: the body keeps turning as it falls.
            if let Some(ai) = entity.ai.as_deref_mut() {
                ai.body_rotation.tick(ai.yaw, &mut ai.look_control, entity.previous_position, entity.villager.body.position);
            }
        } else if !entity.no_ai && entity.ai.is_some() {
            let trades = TradeArgs { book: self.trades.as_deref(), sequences: &mut self.trade_sequences };
            if let Some(damage) = villager_ai_step(entity, world, &seen, &nearby, &mut level_random, game_time, day_time, &mut remote, &mut self.pois, &mut door_updates, trades, griefing, &mut trampled) {
                entity.hurt_from(damage, "minecraft:fall", None, game_time);
            }
            // The items its memories now name, as seen this tick.
            entity.tracked_items = entity
                .ai
                .as_deref()
                .map(|ai| {
                    let m = &ai.brain.memories;
                    let entity_of = |t: Option<&crate::villager_brain::Tracker>| match t {
                        Some(&crate::villager_brain::Tracker::Entity { id, .. }) => Some(id),
                        _ => None,
                    };
                    let mut named: Vec<u64> = [m.nearest_visible_wanted_item.get().copied(), entity_of(m.look_target.get()), entity_of(m.walk_target.get().map(|w| &w.target))]
                        .into_iter()
                        .flatten()
                        .filter(|&id| (crate::villager_brain::ITEM_TARGET..PLAYER_TARGET).contains(&id))
                        .collect();
                    named.sort_unstable();
                    named.dedup();
                    named.into_iter().filter_map(|id| seen.iter().find(|s| s.id == id).cloned()).collect()
                })
                .unwrap_or_default();
            // `customServerAiStep` after the brain: a trade this tick earns
            // the trader good gossip (`TRADE`) and shows the villager's
            // happiness; with no offers it stops trading.
            traded = entity.last_traded_player.take();
            stop_trading = entity.trading_player.is_some() && entity.offers.as_ref().is_none_or(Vec::is_empty);
            // `LivingEntity.tick`: a sleeper looks level.
            if entity.sleeping.is_some() {
                if let Some(ai) = entity.ai.as_deref_mut() {
                    ai.look_control.pitch = 0.0;
                }
            }
        } else {
            entity.villager.body.trim_small_velocity();
        }
        self.level_random = level_random;
        if let Some(player) = traded {
            let uuid = self.uuid_of(PLAYER_TARGET + player);
            let entity = self.villager_mut(id).unwrap();
            Arc::make_mut(&mut entity.gossips).add(uuid, crate::gossip::GossipType::Trading, 2);
            if entity.trading_player == Some(player) {
                self.villager_update_special_prices(id, player);
            }
            self.entity_events.push((id, 14));
        }
        if stop_trading {
            self.villager_stop_trading(id);
        }
        if let Some((murderer, witnesses)) = murder {
            let uuid = self.uuid_of(murderer);
            for witness in witnesses {
                if let Some(w) = self.villager_mut(witness) {
                    Arc::make_mut(&mut w.gossips).add(uuid, crate::gossip::GossipType::MajorNegative, 25);
                }
            }
        }
        // The others' navigations hear of each door half's change, and
        // every navigation of farmland trampled to dirt
        // (`ServerLevel.sendBlockUpdated`: its collision grew).
        for update in door_updates {
            recompute_others(&mut self.villagers, id, &*world, update, game_time);
        }
        if let Some(pos) = trampled {
            recompute_for_block(&mut self.villagers, pos, &*world, game_time);
        }
        let entity = self.villager_mut(id).unwrap();
        let old = entity.previous_position;
        hazards::blocks_act(entity, &*world, old, game_time);
        self.push_entities(EntityKey::Villager(id), &*world, game_time, ticks);
        self.villager_pick_up(id, world);
        let entity = self.villager_mut(id).unwrap();
        let was_baby = entity.villager.age.baby();
        let _ = entity.villager.age.tick(entity.villager.health > 0.0);
        // `AgeableMob.ageBoundaryReached`: the grown size and a new brain.
        if was_baby != entity.villager.age.baby() && entity.villager.health > 0.0 {
            let ticks = entity.villager.age.ticks;
            entity.villager.set_age(ticks);
            if entity.ai.is_some() && !entity.no_ai {
                let seen = self.seen_all(id, players);
                let mut level_random = std::mem::take(&mut self.level_random);
                let mut remote_after = Vec::new();
                let entity = self.villagers.iter_mut().find(|e| e.id == id).unwrap();
                let trades = TradeArgs { book: self.trades.as_deref(), sequences: &mut self.trade_sequences };
                refresh_brain(entity, &*world, &seen, &mut level_random, game_time, day_time, &mut remote_after, &mut self.pois, trades, self.mob_griefing);
                self.level_random = level_random;
            }
        }
        // `Villager.tick`: the unhappy head-shake counts down, and gossip
        // fades once a day (`maybeDecayGossip`).
        let entity = self.villager_mut(id).unwrap();
        if entity.unhappy > 0 {
            entity.unhappy -= 1;
        }
        if entity.last_gossip_decay == 0 {
            entity.last_gossip_decay = game_time;
        } else if game_time >= entity.last_gossip_decay + 24_000 {
            if !entity.gossips.is_empty() {
                Arc::make_mut(&mut entity.gossips).decay();
            }
            entity.last_gossip_decay = game_time;
        }
        // What its brain did to others' brains.
        for write in remote {
            match write {
                Remote::Look { villager, target } => {
                    if let Some(ai) = self.villager_mut(villager).and_then(|e| e.ai.as_deref_mut()) {
                        ai.brain.memories.look_target.set(target);
                    }
                }
                Remote::Walk { villager, target } => {
                    if let Some(ai) = self.villager_mut(villager).and_then(|e| e.ai.as_deref_mut()) {
                        ai.brain.memories.walk_target.set(target);
                    }
                }
                Remote::Gossip { villager, time } => {
                    if let Some(ai) = self.villager_mut(villager).and_then(|e| e.ai.as_deref_mut()) {
                        ai.last_gossip_time = time;
                    }
                }
                Remote::Bed { pos, occupied } => set_bed_occupied(world, pos, occupied),
                Remote::PotentialJobSite { villager, pos } => {
                    if let Some(ai) = self.villager_mut(villager).and_then(|e| e.ai.as_deref_mut()) {
                        ai.brain.memories.potential_job_site.set(pos);
                    }
                }
                Remote::EraseJobSite { villager } => {
                    if let Some(ai) = self.villager_mut(villager).and_then(|e| e.ai.as_deref_mut()) {
                        ai.brain.memories.job_site.erase();
                    }
                }
                Remote::SpawnGolem(golem) => self.spawn_summoned_golem(golem),
                Remote::WorkedAtPoi { villager } => self.villager_work_restock(villager),
                Remote::GolemDetected { villager } => {
                    if let Some(ai) = self.villager_mut(villager).and_then(|e| e.ai.as_deref_mut()) {
                        ai.brain.memories.golem_detected_recently.set_for((), 599);
                    }
                }
                Remote::Eat { villager } => {
                    if let Some(e) = self.villager_mut(villager) {
                        crate::villager_inventory::eat_and_digest(&mut e.food_level, &mut e.inventory);
                    }
                }
                Remote::Birth(birth) => self.villager_birth(birth, &*world),
                Remote::EntityEvent { entity, event } => self.entity_events.push((entity, event)),
                Remote::ThrowItem { stack, position, velocity } => world.spawn_item(position, &stack, velocity, 10),
                Remote::PopItem { stack, position } => world.spawn_popped_item(position, &stack),
                Remote::ScheduleTick { pos, delay } => world.schedule_block_tick(pos, delay),
                Remote::Sound { villager, event, position } => {
                    if let Some(e) = self.villager_mut(villager) {
                        e.voices.push((Voice::Event(event, 1.0, 1.0), position));
                    }
                }
            }
        }
    }

    /// `VillagerMakeLove.breed` once the parent's brain is done: the partner
    /// rests from breeding (`setAge(6000)`, as the parent did), and the
    /// baby is made (`getBreedOffspring`: its constructor's random facing and
    /// brain from its own random, unseeded in vanilla and pinned by the
    /// harness; unemployed, of the type drawn), made a baby (`setAge(-24000)`,
    /// which makes its brain anew), put where the parent stood facing south
    /// (`snapTo`: its head keeps the constructor's turn), given the bed and
    /// shown hearts. It ticks from the next game tick.
    fn villager_birth(&mut self, birth: crate::villager_brain::Birth, world: &impl World) {
        let Some(partner) = self.villager_mut(birth.partner) else { return };
        partner.villager.set_age(6000);
        let kind = birth.kind.unwrap_or_else(|| partner.villager.kind.clone());
        let seed = self.born_seeds.pop_front().unwrap_or_else(|| {
            self.seed_uniquifier = self.seed_uniquifier.wrapping_mul(1_181_783_497_276_652_981);
            self.seed_uniquifier
        });
        let mut random = LegacyRandom::new(seed);
        let turn = random.next_float() * (std::f64::consts::PI * 2.0) as f32;
        let mut villager = Villager::new(birth.at);
        villager.kind = kind;
        let id = self.spawn_villager(villager, true);
        self.villager_mut(id).unwrap().random = random;
        self.activate_villager(id, turn);
        let (game_time, day_time) = (self.game_time, self.day_time);
        let mut level_random = std::mem::take(&mut self.level_random);
        let entity = self.villagers.iter_mut().find(|e| e.id == id).unwrap();
        entity.villager.set_age(crate::age::Age::BABY_START);
        let trades = TradeArgs { book: self.trades.as_deref(), sequences: &mut self.trade_sequences };
        refresh_brain(entity, world, &[], &mut level_random, game_time, day_time, &mut Vec::new(), &mut self.pois, trades, self.mob_griefing);
        self.level_random = level_random;
        let ai = entity.ai.as_deref_mut().unwrap();
        ai.yaw = 0.0;
        ai.body_rotation = crate::look::BodyRotation::new(0.0);
        ai.brain.memories.home.set(birth.bed);
        self.entity_events.push((id, 12));
        self.born_villagers.push(id);
    }
}

/// `LivingEntity.aiStep` up to its travel for a villager with a brain.
#[allow(clippy::too_many_arguments)]
fn villager_ai_step<W: World>(
    entity: &mut VillagerEntity,
    world: &mut W,
    seen: &[Seen],
    nearby: &[u64],
    level_random: &mut LegacyRandom,
    game_time: i64,
    day_time: i64,
    remote: &mut Vec<Remote>,
    pois: &mut crate::poi::PoiManager,
    door_updates: &mut Vec<DoorUpdate>,
    trades: TradeArgs,
    griefing: bool,
    trampled: &mut Option<(i32, i32, i32)>,
) -> Option<f32> {
    let baby = entity.villager.age.baby();
    let eye_height = entity.eye_height();
    let hurt = entity.recent_damage(game_time).map(|d| (d.kind.to_owned(), d.attacker));
    let ai = entity.ai.as_deref_mut().unwrap();
    let body = &mut entity.villager.body;
    if ai.no_jump_delay > 0 {
        ai.no_jump_delay -= 1;
    }
    body.trim_small_velocity();
    // `Mob.checkDespawn` holds a persistent mob's idle clock at zero;
    // `serverAiStep` counts it up.
    if entity.villager.persistence_required {
        entity.no_action_time = 0;
    }
    entity.no_action_time += 1;
    ai.sight.clear();
    let position = body.position;
    let fluid = FluidFrame::sample(world, position, body.width, body.height);
    let (can_update, surface) = crate::navigation::ground_view(&*world, body, fluid, true);
    // `PathNavigation.tick`: a waiting recomputation first.
    if ai.navigation.delayed_recompute(&*world, body, &ai.profile, fluid, game_time) {
        let id = ai.paths.fresh();
        ai.paths.current = Some(id);
        ai.paths.known.insert(id, (ai.navigation.target_pos.unwrap_or_default(), ai.navigation.nodes.len(), ai.navigation.next));
    }
    if let Some((wanted, speed)) = ai.navigation.tick_in(&*world, position, can_update, surface, body.width, ai.speed) {
        ai.move_control.set_wanted_position(wanted, speed);
    }
    if ai.navigation.nodes.is_empty() {
        ai.paths.current = None;
    }
    // `customServerAiStep`: the brain.
    let profession_before = entity.villager.profession;
    let mut look_at = None;
    let mut jump = false;
    let mut sounds = Vec::new();
    let mut doors = Vec::new();
    let mut blocks = Vec::new();
    let heard_at = body.position;
    let parts = CtxParts {
        world: &*world as &dyn World,
        me: Me {
            id: entity.id,
            body,
            fluid,
            eye_height,
            baby,
            navigation: &mut ai.navigation,
            paths: &mut ai.paths,
            profile: &ai.profile,
            look_at: &mut look_at,
            jump: &mut jump,
            random: &mut entity.random,
            sleeping: &mut entity.sleeping,
            yaw: &mut ai.yaw,
            profession: &mut entity.villager.profession,
            xp: entity.villager.xp,
            level: entity.villager.level,
            sounds: &mut sounds,
            doors: &mut doors,
            sight: &mut ai.sight,
            last_gossip_time: ai.last_gossip_time,
            no_ai: entity.no_ai,
            gossips: &mut entity.gossips,
            trading_player: entity.trading_player.map(|p| PLAYER_TARGET + p),
            hurt_time: entity.villager.damage.hurt_ticks,
            held_item: &mut entity.held_item,
            offers: OffersAccess { offers: &mut entity.offers, book: trades.book, sequences: trades.sequences, kind: &entity.villager.kind, level: entity.villager.level },
            inventory: &mut entity.inventory,
            food_level: &mut entity.food_level,
            age: &mut entity.villager.age,
            can_pick_up_loot: entity.can_pick_up_loot,
            mob_griefing: griefing,
            tick_count: entity.tick_count,
            blocks: &mut blocks,
        },
        others: seen,
        level_random,
        time: game_time,
        day_time,
        remote,
        pois,
    };
    ai.brain.tick(parts, nearby, hurt);
    // `setVillagerData` with a new profession forgets the offers.
    if entity.villager.profession != profession_before {
        entity.offers = None;
    }
    for (event, pitch) in sounds {
        entity.voices.push((Voice::Event(event, 1.0, pitch), heard_at));
    }
    // The blocks it harvested, planted and grew.
    for (pos, block) in blocks {
        world.set_block(pos, block);
    }
    for (pos, open, pitch) in doors {
        let halves = crate::villager_brain::door_halves(&*world, pos, open);
        let other = halves.get(1).map(|&(p, _)| (p, world.block(p)));
        if let Some(event) = set_door_open(world, pos, open) {
            door_updates.push(DoorUpdate { pos, other });
            let center = DVec3::new(f64::from(pos.0) + 0.5, f64::from(pos.1) + 0.5, f64::from(pos.2) + 0.5);
            entity.voices.push((Voice::Event(event, 1.0, pitch), center));
        }
    }
    if let Some(target) = look_at {
        ai.look_control.set_look_at(target);
    }
    // A raid's sound, one tick in a hundred (no raids here).
    let _ = entity.random.next_int(100);
    let eye_height = if entity.sleeping.is_some() { 0.2 } else { entity.villager.eye_height() };
    let body = &mut entity.villager.body;
    // Lying down or getting up moved it.
    let position = body.position;
    let obstacle_top = crate::control::obstacle_top(world, position);
    let control = ai.move_control.tick(position, body.on_ground, ai.yaw, ai.speed, ai.forward, ai.sideways, entity.effects.movement_speed(0.5), body.width, body.step_height, obstacle_top, |_, _| true);
    ai.yaw = control.yaw;
    ai.speed = control.speed;
    ai.forward = control.forward;
    ai.sideways = control.sideways;
    ai.look_control.tick(position, eye_height, ai.body_rotation.body_yaw, !ai.navigation.is_done());
    ai.jumping = jump || control.jump;
    body.living_jump(world, fluid, ai.jumping, &mut ai.no_jump_delay, 0.4);
    let input = DVec3::new(f64::from(ai.sideways), 0.0, f64::from(ai.forward));
    let mut damage = None;
    if fluid.in_water() {
        body.travel_water(world, input, ai.yaw);
    } else if fluid.in_lava() {
        body.travel_lava(world, input, ai.yaw, fluid.lava_height);
    } else {
        let landed = body.travel_air_jumping(world, input, ai.speed, ai.yaw, ai.jumping);
        // Trampling farmland lifts it onto the dirt (`snapTo`: its old
        // position too, so it has not moved this tick).
        if landed.is_some_and(|fallen| crate::world::living::fall_on_farmland(body, world, fallen, level_random, griefing)) {
            entity.previous_position = body.position;
            *trampled = Some(body.on_pos(world, 0.2));
        }
        damage = landed.and_then(|fallen| fall_damage(body, world, fallen, false, &mut entity.voices));
    }
    play_movement(body, entity.tick_count, &mut entity.random, &mut entity.voices, VILLAGER_SOUNDS);
    ai.body_rotation.tick(ai.yaw, &mut ai.look_control, entity.previous_position, body.position);
    damage
}

/// `Villager.releaseAllPois`: each remembered point of interest still of
/// its memory's kind gives its ticket back.
fn release_all_pois(m: &crate::villager_brain::Memories, profession: crate::villager::Profession, pois: &mut crate::poi::PoiManager) {
    let held: [(Option<(i32, i32, i32)>, &dyn Fn(crate::poi::PoiType) -> bool); 4] = [
        (m.home.get().copied(), &|t| t == crate::poi::PoiType::Home),
        // Its profession's `heldJobSite` (none for the unemployed).
        (m.job_site.get().copied(), &|t| profession.holds(t)),
        (m.potential_job_site.get().copied(), &crate::poi::PoiType::acquirable_job_site),
        (m.meeting_point.get().copied(), &|t| t == crate::poi::PoiType::Meeting),
    ];
    for (pos, kind) in held {
        if let Some(pos) = pos.filter(|&p| pois.kind(p).is_some_and(kind)) {
            pois.release(pos);
        }
    }
}

impl EntityWorld {
    /// Every villager killed at once (`/kill`, as a scenario begins): each
    /// gives back its points of interest, in the level's order.
    pub fn release_all_villager_pois(&mut self) {
        for entity in &mut self.villagers {
            if let Some(ai) = entity.ai.as_deref_mut().filter(|ai| !ai.pois_released) {
                ai.pois_released = true;
                release_all_pois(&ai.brain.memories, entity.villager.profession, &mut self.pois);
            }
        }
    }
}

/// `#minecraft:beds`.
fn is_bed(id: &str) -> bool {
    id.strip_prefix("minecraft:").and_then(|n| n.strip_suffix("_bed")).is_some_and(|color| {
        matches!(
            color,
            "white" | "orange" | "magenta" | "light_blue" | "yellow" | "lime" | "pink" | "gray" | "light_gray" | "cyan" | "purple" | "blue" | "brown" | "green" | "red" | "black"
        )
    })
}

/// A bed's `OCCUPIED` (`setBlockAndUpdate` on the half slept in), and
/// the other half's with it (`BedBlock.updateShape` copies it across).
fn set_bed_occupied(world: &mut impl World, pos: (i32, i32, i32), occupied: bool) {
    let Some(bed) = world.block(pos).filter(|b| is_bed(&b.id)) else { return };
    let value = if occupied { "true" } else { "false" };
    let (dx, dz) = match bed.property("facing") {
        Some("north") => (0, -1),
        Some("south") => (0, 1),
        Some("west") => (-1, 0),
        _ => (1, 0),
    };
    // The other half: the foot lies behind the head, the head ahead of the foot.
    let other = if bed.property("part") == Some("head") { (pos.0 - dx, pos.1, pos.2 - dz) } else { (pos.0 + dx, pos.1, pos.2 + dz) };
    world.set_block(pos, Some(bed.clone().with("occupied", value)));
    if let Some(half) = world.block(other).filter(|b| b.id == bed.id && b.property("part") != bed.property("part")) {
        world.set_block(other, Some(half.with("occupied", value)));
    }
}

/// `LivingEntity.stopSleeping` outside the brain (the bed gone, or hurt
/// awake): up from the bed when it stands (as the brain's own wake), the
/// standing size back, and `LAST_WOKEN`.
fn wake(entity: &mut VillagerEntity, world: &mut impl World, game_time: i64) {
    let Some(bed) = entity.sleeping.take() else { return };
    if let Some(block) = world.block(bed).filter(|b| is_bed(&b.id)) {
        let yaw = entity.ai.as_ref().map_or(0.0, |ai| ai.yaw);
        let facing = block.property("facing").unwrap_or("north").to_owned();
        set_bed_occupied(world, bed, false);
        let (position, turned) = crate::villager_brain::rise_from_bed(&*world, bed, &facing, yaw);
        entity.villager.body.position = position;
        if let Some(ai) = entity.ai.as_deref_mut() {
            ai.yaw = turned;
        }
    }
    let (width, height) = if entity.villager.age.baby() { (0.49, 0.98) } else { (0.6, 1.95) };
    entity.villager.body.width = width;
    entity.villager.body.height = height;
    if let Some(ai) = entity.ai.as_deref_mut() {
        ai.brain.memories.last_woken.set(game_time);
    }
}

impl VillagerEntity {
    /// Its eyes' height: lying down, 0.2.
    pub fn eye_height(&self) -> f32 {
        if self.sleeping.is_some() {
            0.2
        } else {
            self.villager.eye_height()
        }
    }
}

/// `Villager.refreshBrain` outside its brain's tick (grown up): the running
/// behaviors stop and a new brain takes over.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn refresh_brain<W: World>(entity: &mut VillagerEntity, world: &W, seen: &[Seen], level_random: &mut LegacyRandom, game_time: i64, day_time: i64, remote: &mut Vec<Remote>, pois: &mut crate::poi::PoiManager, trades: TradeArgs, griefing: bool) {
    let baby = entity.villager.age.baby();
    let eye_height = entity.eye_height();
    let Some(ai) = entity.ai.as_deref_mut() else { return };
    let fluid = FluidFrame::sample(world, entity.villager.body.position, entity.villager.body.width, entity.villager.body.height);
    let mut look_at = None;
    let mut jump = false;
    let mut sounds = Vec::new();
    let mut doors = Vec::new();
    let mut blocks = Vec::new();
    let parts = CtxParts {
        world: world as &dyn World,
        me: Me {
            id: entity.id,
            body: &mut entity.villager.body,
            fluid,
            eye_height,
            baby,
            navigation: &mut ai.navigation,
            paths: &mut ai.paths,
            profile: &ai.profile,
            look_at: &mut look_at,
            jump: &mut jump,
            random: &mut entity.random,
            sleeping: &mut entity.sleeping,
            yaw: &mut ai.yaw,
            profession: &mut entity.villager.profession,
            xp: entity.villager.xp,
            level: entity.villager.level,
            sounds: &mut sounds,
            doors: &mut doors,
            sight: &mut ai.sight,
            last_gossip_time: ai.last_gossip_time,
            no_ai: entity.no_ai,
            gossips: &mut entity.gossips,
            trading_player: entity.trading_player.map(|p| PLAYER_TARGET + p),
            hurt_time: entity.villager.damage.hurt_ticks,
            held_item: &mut entity.held_item,
            offers: OffersAccess { offers: &mut entity.offers, book: trades.book, sequences: trades.sequences, kind: &entity.villager.kind, level: entity.villager.level },
            inventory: &mut entity.inventory,
            food_level: &mut entity.food_level,
            age: &mut entity.villager.age,
            can_pick_up_loot: entity.can_pick_up_loot,
            mob_griefing: griefing,
            tick_count: entity.tick_count,
            blocks: &mut blocks,
        },
        others: seen,
        level_random,
        time: game_time,
        day_time,
        remote,
        pois,
    };
    ai.brain.refresh(parts, baby);
}

/// `DoorBlock.setOpen` on a door's lower half: both halves (the upper
/// copies the lower in `updateShape`), and the open or close sound of its
/// wood (`BlockSetType`). None when there is no such door or it already is.
fn set_door_open(world: &mut impl World, pos: (i32, i32, i32), open: bool) -> Option<&'static str> {
    let door = world.block(pos).filter(|b| b.id.ends_with("_door"))?;
    let value = if open { "true" } else { "false" };
    if door.property("open").unwrap_or("false") == value {
        return None;
    }
    let other = if door.property("half") == Some("upper") { (pos.0, pos.1 - 1, pos.2) } else { (pos.0, pos.1 + 1, pos.2) };
    world.set_block(pos, Some(door.clone().with("open", value)));
    if let Some(half) = world.block(other).filter(|b| b.id == door.id && b.property("half") != door.property("half")) {
        world.set_block(other, Some(half.with("open", value)));
    }
    let name = door.id.trim_start_matches("minecraft:");
    Some(match (name, open) {
        ("cherry_door", true) => "block.cherry_wood_door.open",
        ("cherry_door", false) => "block.cherry_wood_door.close",
        ("bamboo_door", true) => "block.bamboo_wood_door.open",
        ("bamboo_door", false) => "block.bamboo_wood_door.close",
        ("crimson_door" | "warped_door", true) => "block.nether_wood_door.open",
        ("crimson_door" | "warped_door", false) => "block.nether_wood_door.close",
        (n, true) if n.contains("copper") => "block.copper_door.open",
        (n, false) if n.contains("copper") => "block.copper_door.close",
        (_, true) => "block.wooden_door.open",
        (_, false) => "block.wooden_door.close",
    })
}

/// A door a villager opened or shut in its tick: the half it set, and the
/// other half with what it was before.
pub(crate) struct DoorUpdate {
    pos: (i32, i32, i32),
    other: Option<((i32, i32, i32), Option<minecraftoss_player::Block>)>,
}

/// `ServerLevel.sendBlockUpdated` for the other villagers: the half set
/// first (the other half not yet changed), then the other half; each
/// navigation whose path passes near recomputes (or waits).
/// `sendBlockUpdated`'s path recomputation for every villager whose path
/// runs near a block whose collision changed.
fn recompute_for_block<W: World>(villagers: &mut [VillagerEntity], pos: (i32, i32, i32), world: &W, time: i64) {
    for entity in villagers.iter_mut() {
        let Some(ai) = entity.ai.as_deref_mut() else { continue };
        let body = &entity.villager.body;
        if !ai.navigation.should_recompute(pos, body.position) {
            continue;
        }
        let fluid = FluidFrame::sample(world, body.position, body.width, body.height);
        if ai.navigation.recompute(world, body, &ai.profile, fluid, time) {
            let id = ai.paths.fresh();
            ai.paths.current = Some(id);
            ai.paths.known.insert(id, (ai.navigation.target_pos.unwrap_or_default(), ai.navigation.nodes.len(), ai.navigation.next));
        }
    }
}

fn recompute_others<W: World>(villagers: &mut [VillagerEntity], except: u64, world: &W, update: DoorUpdate, time: i64) {
    let mut rounds: Vec<((i32, i32, i32), crate::overlay::Overlay)> = Vec::new();
    let reverted: Vec<((i32, i32, i32), Option<minecraftoss_player::Block>)> = update.other.iter().map(|(p, b)| (*p, b.clone())).collect();
    rounds.push((update.pos, crate::overlay::Overlay { base: world, changed: reverted }));
    if let Some((other, _)) = &update.other {
        rounds.push((*other, crate::overlay::Overlay { base: world, changed: Vec::new() }));
    }
    for (pos, view) in rounds {
        for entity in villagers.iter_mut().filter(|e| e.id != except) {
            let Some(ai) = entity.ai.as_deref_mut() else { continue };
            let body = &entity.villager.body;
            if !ai.navigation.should_recompute(pos, body.position) {
                continue;
            }
            let fluid = FluidFrame::sample(&view, body.position, body.width, body.height);
            if ai.navigation.recompute(&view, body, &ai.profile, fluid, time) {
                let id = ai.paths.fresh();
                ai.paths.current = Some(id);
                ai.paths.known.insert(id, (ai.navigation.target_pos.unwrap_or_default(), ai.navigation.nodes.len(), ai.navigation.next));
            }
        }
    }
}
