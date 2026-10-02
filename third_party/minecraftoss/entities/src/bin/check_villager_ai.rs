//! Exact pinned 26.3 gate for villager brains (`scenarios/mobs/villager-idle.json`,
//! `villager-panic.json`): unemployed adults and babies in pens with no
//! points of interest, idling and playing, panicking near a still husk,
//! hurt (generically and by a still cow), through nightfall into rest, and
//! swimming in a pool; and (`villager-poi.json`) points of interest placed
//! and broken, beds and a bell claimed, babies jumping on beds, and strolls
//! towards a village; (`golem-village.json`) iron golems strolling
//! about villages, back to them from outside, and towards villagers who
//! want a golem; and (`golem-fights.json`) golems and zombies hunting each
//! other, a hurt zombie alerting another, and a golem leaving a creeper
//! be; and (`golem-summon.json`) villagers that slept lately summoning a
//! golem when frightened or as they gossip; and (`villager-gossip.json`)
//! gossip: summoned with it, about a far probe player's hurt and murder,
//! passed on as villagers meet, and fading daily; and (`villager-trading.json`,
//! with the data JAR) probes trading: villagers used, offers picked,
//! results taken, payments moved and screens closed, compared with the
//! villagers' experience, levels, offers, trading players and effects,
//! the probes' inventories and screens, and the experience dropped; and
//! (`villager-items.json`) villagers walking to the food and seeds they
//! want and picking them up, as the items' stacks and the villagers'
//! inventories show; and (`villager-breeding.json`) fed villagers breeding,
//! a baby born to the free bed (its random pinned by the harness as it is
//! made) or, with no bed, both angry; and (`villager-sharing.json`)
//! villagers throwing each other food, wheat and seeds as they meet, the
//! thrown items flying, landing and being picked up; and
//! (`villager-farming.json`, with the data JAR) farmers harvesting ripe
//! crops (their loot popped out), sowing bare farmland and growing crops
//! with bone meal, compared with the crops and farmland. The harness seeds each mob's
//! random (a summoned golem's by `summoned_mob_seed`), the brains'
//! shuffling lists and sensors' first scans, and records the level random
//! as the entities begin each tick and the overworld clock; this sets the
//! same, replays the commands and hurts, ticks, and compares every
//! villager's position, motion, rotations, random, activities, running
//! behaviors and memories (the other mobs' position and random), the
//! points of interest and village distances, the level random after the
//! entities, and every sound in the pen. Blocks take their shapes from the
//! block-state catalog.
#[path = "support/catalog.rs"]
mod catalog;
#[path = "support/sounds.rs"]
mod sounds;

use glam::DVec3;
use minecraftoss_entities::{
    cow::{Cow, CowSoundVariant},
    creeper::Creeper,
    gossip::{GossipType, Gossips},
    iron_golem::IronGolem,
    tempt::PlayerCandidate,
    trading::{MerchantOffer, TradeBook},
    villager::Profession,
    world::{PlayerAttack, VillagerUse},
    poi::PoiType,
    villager::Villager,
    villager_brain::{Tracker, WalkTarget},
    world::EntityWorld,
    zombie::{Zombie, ZombieKind},
};
use minecraftoss_player::{
    inventory::{Inventory, ItemStack},
    path_type::PathType,
    rng::LegacyRandom,
    Block, Pos, World, WorldItem,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, env, fs, sync::OnceLock};

static CATALOG: OnceLock<catalog::Catalog> = OnceLock::new();

fn catalog() -> &'static catalog::Catalog {
    CATALOG.get_or_init(|| catalog::Catalog::load("artifacts/block-state-catalog/26.3.json"))
}

/// An item entity in a pen, as `ItemEntity.tick` moves it where there is
/// no fluid.
#[derive(Clone, Debug)]
struct SceneItem {
    id: i32,
    position: DVec3,
    velocity: DVec3,
    item: String,
    count: i32,
    max: i32,
    components: Option<Value>,
    pickup_delay: i32,
    age: i32,
    tick_count: i32,
    on_ground: bool,
    no_gravity: bool,
    /// Made this tick: it ticks from the next (`EntityTickList`).
    fresh: bool,
    fall_distance: f64,
}

impl SceneItem {
    fn world_item(&self) -> WorldItem {
        WorldItem { id: self.id, position: self.position, item: self.item.clone(), count: self.count, components: self.components.clone(), pickup_delay: self.pickup_delay }
    }

    /// `isMergable`.
    fn mergable(&self) -> bool {
        self.count > 0 && self.pickup_delay != 32767 && self.age != -32768 && self.age < 6000 && self.count < self.max
    }

    fn meets(&self, min: DVec3, max: DVec3) -> bool {
        let p = self.position;
        p.x - 0.125 < max.x && p.x + 0.125 > min.x && p.y < max.y && p.y + 0.25 > min.y && p.z - 0.125 < max.z && p.z + 0.125 > min.z
    }
}

/// The blocks, the item entities in the order they were made, the next
/// entity ID, and the loot: block tables, the world's named random
/// sequences, and the harness's pinned seed for item entities' randoms.
#[derive(Default)]
struct Scene(BTreeMap<Pos, Block>, Vec<SceneItem>, i32, Loot);

#[derive(Default)]
struct Loot {
    book: Option<minecraftoss_player::loot::LootBook>,
    sequences: std::cell::RefCell<std::collections::HashMap<String, minecraftoss_player::rng::XoroshiroRandom>>,
    item_seed: Option<u64>,
    /// Scheduled block ticks (game time, position), in order, and the
    /// sounds the blocks made this tick.
    ticks: Vec<(i64, Pos)>,
    sounds: Vec<(String, DVec3)>,
    time: i64,
}

impl Scene {
    /// The block ticks due at `time`, before the entities (`blockTicks`):
    /// a composter at 7 becomes ready, with its sound.
    fn tick_blocks(&mut self, time: i64) {
        self.3.time = time;
        let due: Vec<Pos> = self.3.ticks.iter().filter(|(t, _)| *t <= time).map(|&(_, p)| p).collect();
        self.3.ticks.retain(|(t, _)| *t > time);
        for pos in due {
            if let Some(composter) = self.block(pos).filter(|b| b.id == "minecraft:composter" && b.property("level") == Some("7")) {
                self.set_block(pos, Some(composter.with("level", "8")));
                let center = DVec3::new(f64::from(pos.0) + 0.5, f64::from(pos.1) + 0.5, f64::from(pos.2) + 0.5);
                self.3.sounds.push(("minecraft:block.composter.ready".to_owned(), center));
            }
        }
    }

    /// Each item entity's tick (`ItemEntity.tick` without fluids): gravity,
    /// a resting item moving one tick in four, the move with its collisions
    /// and restitution, air drag and the ground's friction, the bounce, a
    /// merge with neighbours every 40 ticks (2 once it crossed a block),
    /// and its age.
    fn tick_items(&mut self, level_random: &mut LegacyRandom) {
        for index in 0..self.1.len() {
            let mut item = self.1[index].clone();
            if item.count <= 0 || item.fresh {
                continue;
            }
            item.tick_count += 1;
            if item.pickup_delay > 0 && item.pickup_delay != 32767 {
                item.pickup_delay -= 1;
            }
            let old = item.position;
            if !item.no_gravity {
                item.velocity = DVec3::new(item.velocity.x + 0.0, item.velocity.y - 0.04, item.velocity.z + 0.0);
            }
            let horizontal = item.velocity.x * item.velocity.x + item.velocity.z * item.velocity.z;
            if !(item.on_ground && !(horizontal > f64::from(1.0e-5_f32)) && (item.tick_count + item.id) % 4 != 0) {
                let requested = item.velocity;
                let movement = minecraftoss_player::collision::move_body(self, minecraftoss_player::collision::Bounds::standing(item.position, 0.25, 0.25), requested, item.on_ground, 0.0);
                let length = movement.length_squared();
                if length > 1.0e-7 || requested.length_squared() - length < 1.0e-7 {
                    item.position += movement;
                }
                let equal = |a: f64, b: f64| (b - a).abs() < f64::from(1.0e-5_f32);
                let (x_collision, z_collision) = (!equal(requested.x, movement.x), !equal(requested.z, movement.z));
                let moved_vertically = requested.y.abs() > 0.0;
                let mut vertical = false;
                if moved_vertically {
                    vertical = requested.y != movement.y;
                    item.on_ground = vertical && requested.y < 0.0;
                }
                // `checkFallDamage`: landing on farmland after any fall draws
                // from the level random (`FarmlandBlock.fallOn`), items too.
                if movement.y < 0.0 {
                    item.fall_distance -= movement.y;
                }
                if item.on_ground {
                    let on = (item.position.x.floor() as i32, (item.position.y - f64::from(0.2_f32)).floor() as i32, item.position.z.floor() as i32);
                    if item.fall_distance > 0.0 && self.0.get(&on).is_some_and(|b| b.id == "minecraft:farmland") {
                        let _ = level_random.next_float();
                    }
                    item.fall_distance = 0.0;
                }
                // `restituteMovementAfterCollisions`: items and stone do not
                // bounce, so collided motion stops (keeping zero's sign).
                if (moved_vertically && vertical) || x_collision || z_collision {
                    let current = item.velocity;
                    if x_collision {
                        item.velocity.x = -current.x * 0.0;
                    }
                    if z_collision {
                        item.velocity.z = -current.z * 0.0;
                    }
                    if vertical {
                        item.velocity.y = (0.0 - current.y) * 1.0 * 0.0;
                    }
                }
                let below = (item.position.x.floor() as i32, (item.position.y - f64::from(0.500001_f32)).floor() as i32, item.position.z.floor() as i32);
                let friction = if item.on_ground {
                    match self.0.get(&below).map(|b| b.id.as_str()) {
                        Some("minecraft:ice" | "minecraft:packed_ice" | "minecraft:frosted_ice") => 0.98_f32,
                        Some("minecraft:blue_ice") => 0.989_f32,
                        Some("minecraft:slime_block") => 0.8_f32,
                        _ => 0.6_f32,
                    }
                } else {
                    1.0
                };
                let (air, ground) = (f64::from(0.98_f32), f64::from(0.98_f32 * friction));
                item.velocity = DVec3::new(item.velocity.x * ground, item.velocity.y * air, item.velocity.z * ground);
                if item.on_ground && item.velocity.y < 0.0 {
                    item.velocity.y *= -0.5;
                }
            }
            let moved = old.x.floor() != item.position.x.floor() || old.y.floor() != item.position.y.floor() || old.z.floor() != item.position.z.floor();
            let rate = if moved { 2 } else { 40 };
            self.1[index] = item.clone();
            if item.tick_count % rate == 0 && item.mergable() {
                self.merge_with_neighbours(index);
            }
            let item = &mut self.1[index];
            if item.age != -32768 {
                item.age += 1;
            }
            if item.age >= 6000 {
                item.count = 0;
            }
        }
        self.1.retain(|i| i.count > 0);
        for item in &mut self.1 {
            item.fresh = false;
        }
    }

    /// `mergeWithNeighbours`: the mergable items in its box grown by half a
    /// block sideways, the bigger stack taking from the smaller.
    fn merge_with_neighbours(&mut self, index: usize) {
        let me = &self.1[index];
        let (min, max) = (me.position - DVec3::new(0.625, 0.0, 0.625), me.position + DVec3::new(0.625, 0.25, 0.625));
        let others: Vec<usize> = (0..self.1.len()).filter(|&i| i != index && self.1[i].mergable() && self.1[i].meets(min, max)).collect();
        for other in others {
            let (a, b) = (&self.1[index], &self.1[other]);
            if !b.mergable() || a.count <= 0 || a.item != b.item || a.components != b.components || a.count + b.count > b.max {
                continue;
            }
            let (to, from) = if b.count < a.count { (index, other) } else { (other, index) };
            let moved = (self.1[to].max.min(64) - self.1[to].count).min(self.1[from].count);
            self.1[to].count += moved;
            self.1[from].count -= moved;
            self.1[to].pickup_delay = self.1[to].pickup_delay.max(self.1[from].pickup_delay);
            self.1[to].age = self.1[to].age.min(self.1[from].age);
            if self.1[index].count <= 0 {
                break;
            }
        }
    }
}

impl World for Scene {
    fn items_in(&self, min: DVec3, max: DVec3) -> Vec<WorldItem> {
        // An item's box is 0.25 wide and high.
        self.1.iter().filter(|i| i.count > 0 && i.meets(min, max)).map(SceneItem::world_item).collect()
    }
    fn item(&self, id: i32) -> Option<WorldItem> {
        self.1.iter().find(|i| i.id == id && i.count > 0).map(SceneItem::world_item)
    }
    fn take_item(&mut self, id: i32, count: i32) {
        if let Some(item) = self.1.iter_mut().find(|i| i.id == id) {
            item.count -= count;
        }
        self.1.retain(|i| i.count > 0);
    }
    fn schedule_block_tick(&mut self, pos: Pos, delay: i32) {
        let due = self.3.time + i64::from(delay);
        self.3.ticks.push((due, pos));
    }
    fn trample_farmland(&mut self, pos: Pos, random: &mut LegacyRandom) {
        self.set_block(pos, Some(Block::new("minecraft:dirt")));
        // `updateShape` of a crop on it: on dirt it breaks (`destroyBlock`,
        // its loot popped out).
        let above = (pos.0, pos.1 + 1, pos.2);
        if let Some(crop) = self.block(above).filter(|b| minecraftoss_entities::crops::max_age(&b.id).is_some()) {
            for stack in self.block_drops(above, &crop) {
                let x = f64::from(above.0) + 0.5 + (random.next_double() * 0.5 - 0.25);
                let y = f64::from(above.1) + 0.5 + (random.next_double() * 0.5 - 0.25) - 0.125;
                let z = f64::from(above.2) + 0.5 + (random.next_double() * 0.5 - 0.25);
                if stack.count > 0 {
                    self.spawn_popped_item(DVec3::new(x, y, z), &stack);
                }
            }
            self.set_block(above, None);
        }
    }
    fn block_drops(&self, _pos: Pos, block: &Block) -> Vec<ItemStack> {
        let book = self.3.book.as_ref().expect("farming needs the data JAR's loot tables");
        let mut sequences = self.3.sequences.borrow_mut();
        book.roll_drops_named(block, None, 0, &mut sequences).unwrap_or_else(|| panic!("no loot for {block:?}"))
    }
    fn spawn_popped_item(&mut self, position: DVec3, stack: &ItemStack) {
        // `new ItemEntity(level, x, y, z, stack)`: its random (pinned to
        // `seed ^ id`) sets its bobbing (`bobOffs`) and turns it, then
        // throws it.
        let id = self.2;
        let mut random = LegacyRandom::new(self.3.item_seed.expect("popped items need a pinned item seed") ^ id as i64 as u64);
        let (_bob, _yaw) = (random.next_float(), random.next_float());
        let dx = random.next_double() * 0.2 - 0.1;
        let dz = random.next_double() * 0.2 - 0.1;
        self.spawn_item(position, stack, DVec3::new(dx, 0.2, dz), 10);
    }
    fn spawn_item(&mut self, position: DVec3, stack: &ItemStack, velocity: DVec3, pickup_delay: i32) {
        let id = self.2;
        self.2 += 1;
        self.1.push(SceneItem {
            id,
            position,
            velocity,
            item: stack.id.clone(),
            count: i32::from(stack.count),
            max: i32::from(stack.max),
            components: stack.components.clone(),
            pickup_delay,
            age: 0,
            tick_count: 0,
            on_ground: false,
            no_gravity: false,
            fresh: true,
            fall_distance: 0.0,
        });
    }
    fn block(&self, pos: Pos) -> Option<Block> {
        self.0.get(&pos).cloned()
    }
    fn collision_boxes(&self, pos: Pos) -> Vec<[f64; 6]> {
        self.0.get(&pos).map_or_else(Vec::new, |b| catalog().get(b).map_or_else(|| minecraftoss_player::authored_collision_boxes(b), |s| s.collision.clone()))
    }
    fn solid(&self, pos: Pos) -> bool {
        self.0.get(&pos).is_some_and(|b| catalog().get(b).is_some_and(|s| s.solid))
    }
    fn solid_render(&self, pos: Pos) -> bool {
        self.0.get(&pos).is_some_and(|b| catalog().get(b).is_some_and(|s| s.solid_render))
    }
    fn pathfindable(&self, pos: Pos) -> bool {
        self.0.get(&pos).is_none_or(|b| catalog().get(b).is_none_or(|s| s.pathfindable))
    }
    fn path_type_from_state(&self, pos: Pos) -> PathType {
        catalog().path_type(self.0.get(&pos))
    }
    fn set_block(&mut self, pos: Pos, block: Option<Block>) {
        match block {
            Some(block) if block.id != "minecraft:air" => {
                self.0.insert(pos, block);
            }
            _ => {
                self.0.remove(&pos);
            }
        }
    }
    fn step_sound(&self, pos: Pos) -> Option<(String, f32, f32)> {
        let block = self.0.get(&pos)?;
        catalog().get(block).and_then(|s| s.step.clone()).or_else(|| sounds::step_sound(&block.id))
    }
    fn suffocating(&self, pos: Pos) -> bool {
        self.0.get(&pos).is_some_and(|b| catalog().get(b).is_some_and(|s| s.suffocating))
    }
    fn can_see_sky(&self, _pos: Pos) -> bool {
        true
    }
    fn biome(&self, _pos: Pos) -> Option<String> {
        Some("minecraft:the_void".to_owned())
    }
}

/// A block from a command's state (`id[key=value,...]`).
fn parse_block(spec: &str) -> Block {
    let Some((id, rest)) = spec.split_once('[') else { return Block::new(spec) };
    let mut block = Block::new(id);
    for pair in rest.trim_end_matches(']').split(',').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap();
        block = block.with(key, value);
    }
    block
}

/// A point-of-interest change, with the tick of the command that made it.
type PoiChange = (i64, Pos, Option<PoiType>, Option<PoiType>);

/// A `fill` or `setblock` command on the scene, in the command's order
/// (`BlockPos.betweenClosed`: x fastest, then y, then z), with each point
/// of interest it makes or breaks.
fn place(scene: &mut Scene, command: &str, tick: i64, poi_changes: &mut Vec<PoiChange>) {
    let parts: Vec<&str> = command.split_whitespace().collect();
    let n = |i: usize| parts[i].parse::<i32>().unwrap();
    let (min, max, spec) = match parts.first() {
        Some(&"fill") => ((n(1), n(2), n(3)), (n(4), n(5), n(6)), parts[7]),
        Some(&"setblock") => ((n(1), n(2), n(3)), (n(1), n(2), n(3)), parts[4]),
        _ => return,
    };
    let block = parse_block(spec);
    for z in min.2..=max.2 {
        for y in min.1..=max.1 {
            for x in min.0..=max.0 {
                let pos = (x, y, z);
                let old = scene.block(pos).and_then(|b| PoiType::of_block(&b));
                let new = PoiType::of_block(&block);
                scene.set_block(pos, Some(block.clone()));
                if old != new {
                    poi_changes.push((tick, pos, old, new));
                }
            }
        }
    }
}

/// Saved gossip (`GossipContainer.CODEC`: target UUID ints, type, value),
/// in its order.
fn gossip_entries(value: &Value) -> Vec<(u128, GossipType, i32)> {
    value
        .as_array()
        .map(|list| {
            list.iter()
                .map(|g| {
                    let ints: Vec<i32> = g["Target"].as_array().unwrap().iter().map(|i| i.as_i64().unwrap() as i32).collect();
                    let target = minecraftoss_entities::gossip::uuid_from_ints([ints[0], ints[1], ints[2], ints[3]]);
                    (target, GossipType::from_id(g["Type"].as_str().unwrap()).unwrap(), g["Value"].as_i64().unwrap() as i32)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Gossip with each target's types in type order (vanilla's order within
/// a target follows identity hash codes).
fn by_target_then_type(mut entries: Vec<(u128, GossipType, i32)>) -> Vec<(u128, GossipType, i32)> {
    let mut start = 0;
    while start < entries.len() {
        let target = entries[start].0;
        let end = start + entries[start..].iter().take_while(|e| e.0 == target).count();
        entries[start..end].sort_by_key(|e| e.1);
        start = end;
    }
    entries
}

/// A probe player: as brains and hits know it, facing, and its inventory.
struct Probe {
    candidate: PlayerCandidate,
    yaw: f32,
    inventory: Inventory,
    uuid: String,
}

/// A stack as `ItemStack.CODEC` writes it.
fn stack_json(stack: &Option<ItemStack>) -> Value {
    match stack {
        Some(s) if s.count > 0 => {
            let mut v = json!({ "id": s.id, "count": s.count });
            if let Some(c) = &s.components {
                v["components"] = c.clone();
            }
            v
        }
        _ => Value::Null,
    }
}

/// Offers as JSON with floats compared by their `float` value.
fn normalize_offers(value: &Value) -> Value {
    let mut value = value.clone();
    if let Value::Array(list) = &mut value {
        for offer in list {
            if let Some(m) = offer.get("priceMultiplier").and_then(Value::as_f64) {
                offer["priceMultiplier"] = Value::from(format!("{:08x}", (m as f32).to_bits()));
            }
        }
    }
    value
}

/// A probe player as the brains and hits know it.
fn probe_candidate(id: u64, position: DVec3) -> PlayerCandidate {
    PlayerCandidate {
        id,
        position,
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
    }
}

fn number(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value["bits"].as_str().unwrap(), 16).unwrap())
}

fn exact(observed: &Value, actual: f64, label: &str) {
    assert_eq!(actual.to_bits(), number(observed).to_bits(), "{label}: {actual} vs {}", number(observed));
}

/// An entity as memories name it: a player or item by its entity number
/// (brains know players past `u64::MAX / 2`, items past `u64::MAX / 4`).
fn entity_number(id: u64) -> u64 {
    if id >= u64::MAX / 2 {
        id - u64::MAX / 2
    } else if id >= minecraftoss_entities::villager_brain::ITEM_TARGET {
        id - minecraftoss_entities::villager_brain::ITEM_TARGET
    } else {
        id
    }
}

fn tracker(t: Tracker) -> Value {
    match t {
        Tracker::Entity { id, .. } => json!({ "entity": entity_number(id) }),
        Tracker::Block((x, y, z)) => json!({ "block": [x, y, z] }),
    }
}

fn walk(w: WalkTarget) -> Value {
    let speed = f64::from(w.speed);
    json!({ "target": tracker(w.target), "speed": { "decimal": format!("{speed:?}"), "bits": format!("{:016x}", speed.to_bits()) }, "close_enough": w.close_enough })
}

/// The observed box of points of interest and the village samples.
struct PoiWatch {
    min: Pos,
    max: Pos,
    samples: Vec<Pos>,
}

fn pos_of(v: &Value) -> Pos {
    (v[0].as_i64().unwrap() as i32, v[1].as_i64().unwrap() as i32, v[2].as_i64().unwrap() as i32)
}

/// The snapshot's points of interest in the box (chunk by chunk, x
/// fastest; section by section upwards; types by ID; each type's records
/// in its set's order) and the village distances at the samples, against
/// the world's.
fn compare_pois(world: &mut EntityWorld, data: &Value, watch: Option<&PoiWatch>, at: &str, frames: &mut usize) {
    let Some(watch) = watch else { return };
    let theirs: Vec<(Pos, String, i64)> = data["poi_records"].as_array().unwrap().iter().map(|r| (pos_of(&r["pos"]), r["type"].as_str().unwrap().to_owned(), r["free_tickets"].as_i64().unwrap())).collect();
    let mut ours: Vec<(Pos, String, i64)> = Vec::new();
    for cz in watch.min.2 >> 4..=watch.max.2 >> 4 {
        for cx in watch.min.0 >> 4..=watch.max.0 >> 4 {
            let mut chunk: Vec<(i32, String, Pos, i64)> = world.pois.records_in_chunk(cx, cz).into_iter().map(|r| (r.pos.1 >> 4, r.kind.id().to_owned(), r.pos, i64::from(r.free_tickets))).collect();
            // A stable sort keeps each type's set order.
            chunk.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
            let inside = |p: Pos| (watch.min.0..=watch.max.0).contains(&p.0) && (watch.min.1..=watch.max.1).contains(&p.1) && (watch.min.2..=watch.max.2).contains(&p.2);
            ours.extend(chunk.into_iter().filter(|c| inside(c.2)).map(|(_, kind, pos, free)| (pos, kind, free)));
        }
    }
    assert_eq!(ours, theirs, "{at} points of interest");
    let villages: Vec<i64> = watch.samples.iter().map(|&(x, y, z)| i64::from(world.pois.sections_to_village((x >> 4, y >> 4, z >> 4)))).collect();
    assert_eq!(json!(villages), data["sections_to_village"], "{at} village distances");
    *frames += 1;
}

fn main() {
    let path = env::args().nth(1).expect("usage: check_villager_ai TRACE.jsonl");
    let mut suite = Value::Null;
    let mut scene = Scene::default();
    let mut world = EntityWorld::default();
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    let mut scenario = String::new();
    let (mut frames, mut heard, mut moved, mut hurts) = (0, 0, 0, 0);
    let mut golem_frames = 0;
    let mut golem_goals: BTreeMap<String, usize> = BTreeMap::new();
    let mut monster_frames = 0;
    let mut deaths = 0;
    // The tag a summoned golem is known by, and how many appeared.
    let mut summoned_tag: Option<String> = None;
    let mut summoned = 0;
    let mut behaviors: BTreeMap<String, usize> = BTreeMap::new();
    let mut complete = false;
    let mut poi_changes: Vec<PoiChange> = Vec::new();
    let mut poi_frames = 0;
    let mut watch: Option<PoiWatch> = None;
    // Each villager's seed and brain generation: a rebuilt brain's lists
    // are reseeded from `seed ^ gameTime` (`BrainProbe.afterRefresh`).
    let mut brain_seeds: BTreeMap<u64, u64> = BTreeMap::new();
    let mut generations: BTreeMap<u64, u32> = BTreeMap::new();
    // Probe players by tag, and the trade data trading needs.
    let mut probes: BTreeMap<String, Probe> = BTreeMap::new();
    let (mut gossip_frames, mut attacks) = (0, 0);
    let book: Option<std::sync::Arc<TradeBook>> = env::args().nth(2).map(|jar| std::sync::Arc::new(TradeBook::from_jar(std::path::Path::new(&jar), std::path::Path::new("artifacts/item-catalog/26.3.json")).expect("trade data")));
    let jar = env::args().nth(2);
    let max_stack = |item: &str| book.as_ref().map_or(64, |b| b.max_stack(item)) as u8;
    let (mut trade_frames, mut trade_actions, mut experience) = (0, 0, 0i64);
    let (mut item_frames, mut inventory_frames, mut crop_frames) = (0, 0, 0);
    // Babies' pinned tags and seeds, in turn, and how many were born.
    let mut born_pins: std::collections::VecDeque<(String, u64)> = Default::default();
    let mut trades_started = false;
    let mut births = 0;
    // The UUIDs targets are recorded by: the mobs' observed ones.
    let mut mob_uuids: BTreeMap<u64, String> = BTreeMap::new();
    for line in fs::read_to_string(path).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_ne!(row["type"], "error");
        match row["type"].as_str().unwrap() {
            "manifest" => suite = row["data"]["suite"].clone(),
            "complete" => complete = true,
            "scenario_start" => {
                scenario = row["scenario"].as_str().unwrap().to_owned();
                // The level's points of interest stay from one scenario to
                // the next, as its blocks do; the kill that begins each gives
                // the villagers' back.
                world.release_all_villager_pois();
                let pois = std::mem::take(&mut world.pois);
                let sequences = std::mem::take(&mut world.trade_sequences);
                world = EntityWorld::default();
                world.pois = pois;
                world.set_day_time(4000);
                scene.1.clear();
                // The world's loot sequences go on from scenario to scenario.
                if scene.3.book.is_none() {
                    scene.3.book = jar.as_ref().map(|j| minecraftoss_player::loot::LootBook::from_jar(std::path::Path::new(j)).expect("loot tables"));
                }
                scene.3.item_seed = None;
                scene.3.ticks.clear();
                // So do the trade sets' random sequences.
                if let Some(book) = &book {
                    world.set_trades(book.clone(), suite["seed"].as_str().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0) as u64);
                    if trades_started {
                        world.trade_sequences = sequences;
                    }
                    trades_started = true;
                }
                experience = 0;
                ids.clear();
                poi_changes.clear();
                brain_seeds.clear();
                generations.clear();
                probes.clear();
                born_pins.clear();
                let definition = suite["scenarios"].as_array().unwrap().iter().find(|c| c["id"] == scenario.as_str()).unwrap();
                let spec = &definition["observe"]["poi_records"];
                watch = spec.is_object().then(|| PoiWatch {
                    min: pos_of(&spec["min"]),
                    max: pos_of(&spec["max"]),
                    samples: spec["village_samples"].as_array().map_or_else(Vec::new, |s| s.iter().map(pos_of).collect()),
                });
            }
            // Commands of every phase, in order.
            "command" => {
                // On the server thread `ServerLevel.updatePOIOnBlockStateChange`'s
                // task runs at once (`BlockableEventLoop.execute`).
                place(&mut scene, row["data"]["command"].as_str().unwrap(), row["tick"].as_i64().unwrap(), &mut poi_changes);
                for (_, pos, old, new) in poi_changes.drain(..) {
                    world.pois.block_changed(pos, old, new);
                }
            }
            "item_entity_seed" => {
                scene.3.item_seed = Some(row["data"]["seed"].as_str().unwrap().parse::<i64>().unwrap() as u64);
            }
            "born_mob_seed" => {
                let seed: u64 = row["data"]["seed"].as_str().unwrap().parse::<i64>().unwrap() as u64;
                world.push_born_seed(seed);
                born_pins.push_back((row["data"]["tag"].as_str().unwrap().to_owned(), seed));
            }
            "summoned_mob_seed" => {
                let seed: u64 = row["data"]["seed"].as_str().unwrap().parse().unwrap();
                world.set_summoned_golem_seed(Some(seed));
                summoned_tag = Some(row["data"]["tag"].as_str().unwrap().to_owned());
            }
            "entity_set_random_seed" => {
                let id = ids[row["data"]["tag"].as_str().unwrap()];
                let seed: u64 = row["data"]["seed"].as_str().unwrap().parse().unwrap();
                brain_seeds.insert(id, seed);
                if let Some(entity) = world.villager_mut(id) {
                    entity.random = LegacyRandom::new(seed);
                    if let Some(ai) = entity.ai.as_deref_mut() {
                        ai.brain.reseed(seed);
                    }
                } else if let Some(entity) = world.zombie_mut(id) {
                    entity.random = LegacyRandom::new(seed);
                } else if let Some(entity) = world.iron_golem_mut(id) {
                    entity.random = LegacyRandom::new(seed);
                } else if let Some(entity) = world.creeper_mut(id) {
                    entity.random = LegacyRandom::new(seed);
                } else {
                    world.cow_mut(id).unwrap().random = LegacyRandom::new(seed);
                }
            }
            "player_probe" => {
                // A probe player, known by its UUID to the gossip about it.
                let data = &row["data"];
                let tag = data["tag"].as_str().unwrap().to_owned();
                let pos = &data["pos"];
                let position = DVec3::new(pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap(), pos[2].as_f64().unwrap());
                // Its entity number: recorded, or the next after the mobs
                // placed at setup, in the probes' order.
                let id = match (probes.get(&tag), data["entity_id"].as_u64()) {
                    (Some(p), _) => p.candidate.id,
                    (None, Some(id)) => id,
                    (None, None) => ids.values().copied().max().unwrap_or(0) + 1 + probes.len() as u64,
                };
                let definition = suite["scenarios"].as_array().unwrap().iter().find(|c| c["id"] == scenario.as_str()).unwrap();
                let action = definition["actions"].as_array().unwrap().iter().find(|a| a["type"] == "player_probe" && a["tag"] == tag.as_str()).unwrap();
                world.set_player_uuid(id, minecraftoss_entities::gossip::parse_uuid(data["uuid"].as_str().unwrap()).unwrap());
                // Its main hand (slot 0) and other slots.
                let mut inventory = Inventory::default();
                let stack = |item: &str, count: u8| ItemStack { max: max_stack(item), ..ItemStack::new(item, count) };
                let item = data["item"].as_str().unwrap();
                if item != "minecraft:air" {
                    inventory.slots[0] = Some(stack(item, 1));
                }
                for s in action["inventory"].as_array().into_iter().flatten() {
                    inventory.slots[s["slot"].as_u64().unwrap() as usize] = Some(stack(s["item"].as_str().unwrap(), s["count"].as_u64().unwrap() as u8));
                }
                probes.insert(tag, Probe { candidate: probe_candidate(id, position), yaw: action["yaw"].as_f64().unwrap_or(0.0) as f32, inventory, uuid: data["uuid"].as_str().unwrap().to_owned() });
            }
            "player_attack" => {
                // `Player.attack` with a bare hand at full strength.
                let data = &row["data"];
                let tick = row["tick"].as_i64().unwrap();
                let at = format!("{scenario} tick {tick} attack");
                exact(&data["attack_damage"], 1.0, &format!("{at} attack damage"));
                exact(&data["strength"], 1.0, &format!("{at} strength"));
                let probe = &probes[data["probe"].as_str().unwrap()];
                let (candidate, yaw) = (probe.candidate, probe.yaw);
                let attack = PlayerAttack { player_id: candidate.id, position: candidate.position, yaw, attack_damage: 1.0, strength: 1.0, sprinting: false, can_critical: false, can_sweep: false };
                let target = ids[data["target"].as_str().unwrap()];
                let result = world.player_attack(&attack, target);
                assert!(result.hurt, "{at} hurts");
                exact(&data["health_after"], f64::from(world.villagers().iter().find(|v| v.id == target).map_or(0.0, |v| v.villager.health)), &format!("{at} health after"));
                attacks += 1;
            }
            "villager_interact" => {
                // `Player.interactOn` with the main hand.
                let data = &row["data"];
                let at = format!("{scenario} tick {} interact", row["tick"]);
                let probe = probes.get_mut(data["probe"].as_str().unwrap()).unwrap();
                let held = probe.inventory.slots[0].as_ref().map(|s| s.id.clone());
                let used = world.villager_interact(ids[data["tag"].as_str().unwrap()], probe.candidate.id, true, held.as_deref());
                assert_eq!(data["consumes_action"].as_bool().unwrap(), used != VillagerUse::Pass, "{at} consumed");
                assert_eq!(data["merchant_open"].as_bool().unwrap(), world.merchant_menu(probe.candidate.id).is_some(), "{at} screen open");
                trade_actions += 1;
            }
            "merchant_select" | "merchant_click" | "merchant_close" => {
                let data = &row["data"];
                let kind = row["type"].as_str().unwrap();
                let at = format!("{scenario} tick {} {kind}", row["tick"]);
                let probe = probes.get_mut(data["probe"].as_str().unwrap()).unwrap();
                let player = probe.candidate.id;
                let eyes = probe.candidate.position + DVec3::Y * 1.62;
                let valid = world.merchant_still_valid(player, eyes);
                if kind == "merchant_close" {
                    assert_eq!(data["was_merchant"].as_bool().unwrap(), world.merchant_menu(player).is_some(), "{at} screen");
                    let drops = world.merchant_close(player, &mut probe.inventory, 0);
                    assert!(drops.is_empty(), "{at}: nothing dropped");
                } else {
                    assert_eq!(data["valid"].as_bool().unwrap(), valid, "{at} still valid");
                    if valid && kind == "merchant_select" {
                        world.merchant_select(player, data["index"].as_i64().unwrap() as i32, &mut probe.inventory);
                    } else if valid {
                        let (slot, right, shift) = (data["slot"].as_u64().unwrap() as usize, data["button"] == 1, data["mode"] == "quick_move");
                        match slot {
                            0 | 1 => world.merchant_click_payment(player, slot, right, shift, &mut probe.inventory),
                            2 => world.merchant_click_result(player, shift, &mut probe.inventory),
                            _ if shift => world.merchant_quick_move_inventory(player, if slot < 30 { slot - 3 + 9 } else { slot - 30 }, &mut probe.inventory),
                            // An inventory slot: the standard pickup.
                            _ => {
                                probe.inventory.click(Some(if slot < 30 { slot - 3 + 9 } else { slot - 30 }), right, false);
                            }
                        }
                    }
                }
                trade_actions += 1;
            }
            "entity_hurt" => {
                // `hurtServer` before the tick: generic, or `mob_attack`
                // from a mob (knocking it back from the attacker on a full
                // hit); the last damage for the brain's hurt-by sensor.
                let data = &row["data"];
                let tick = row["tick"].as_i64().unwrap();
                let attacker = data["attacker"].as_str().map(|tag| ids[tag]);
                let from = attacker.map(|a| world.body_mut(a).unwrap().position);
                let time = world.game_time();
                let kind = match data["source"].as_str().unwrap() {
                    "minecraft:generic" => "minecraft:generic",
                    "minecraft:magic" => "minecraft:magic",
                    "minecraft:mob_attack" => "minecraft:mob_attack",
                    other => panic!("unexpected damage source {other}"),
                };
                let entity = world.villager_mut(ids[data["tag"].as_str().unwrap()]).unwrap();
                exact(&data["health_before"], f64::from(entity.villager.health), &format!("{scenario} tick {tick} health before hurt"));
                let result = entity.hurt_from(number(&data["amount"]) as f32, kind, attacker, time);
                assert_eq!(result.applied, data["applied"].as_bool().unwrap(), "{scenario} tick {tick} hurt applied");
                if let Some(from) = from.filter(|_| result.applied && result.full) {
                    entity.knockback_from(from);
                }
                exact(&data["health_after"], f64::from(entity.villager.health), &format!("{scenario} tick {tick} health after hurt"));
                hurts += 1;
            }
            "snapshot" if scenario == "villager_warmup" => poi_changes.clear(),
            "snapshot" => {
                let tick = row["tick"].as_i64().unwrap();
                let data = &row["data"];
                let observes_offers = suite["scenarios"].as_array().unwrap().iter().find(|c| c["id"] == scenario.as_str()).is_some_and(|c| c["observe"]["villager_offers"] == true);
                let no_entities = serde_json::Map::new();
                let observed = data["entities"].as_object().unwrap_or(&no_entities);
                if tick == 0 {
                    world.set_game_time(data["game_time"].as_i64().unwrap());
                    // The item entities as summoned (lying still, without
                    // gravity), and the next entity's ID.
                    for e in data["entity_type_states"]["minecraft:item"].as_array().into_iter().flatten() {
                        let stack = &e["item"];
                        let item = stack["id"].as_str().unwrap().to_owned();
                        scene.1.push(SceneItem {
                            id: e["entity_numeric_id"].as_i64().unwrap() as i32,
                            position: DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"])),
                            velocity: DVec3::new(number(&e["vx"]), number(&e["vy"]), number(&e["vz"])),
                            max: i32::from(max_stack(&item)),
                            item,
                            count: stack["count"].as_i64().unwrap() as i32,
                            components: stack.get("components").cloned(),
                            pickup_delay: e["pickup_delay"].as_i64().unwrap() as i32,
                            age: e["item_age"].as_i64().unwrap() as i32,
                            tick_count: e["entity_tick_count"].as_i64().unwrap_or(0) as i32,
                            on_ground: e["on_ground"].as_bool().unwrap(),
                            no_gravity: true,
                            fresh: false,
                            fall_distance: 0.0,
                        });
                    }
                    let newest = observed.values().flat_map(|s| s.as_array().into_iter().flatten()).chain(data["entity_type_states"]["minecraft:item"].as_array().into_iter().flatten()).filter_map(|e| e["entity_numeric_id"].as_i64()).max().unwrap_or(0);
                    scene.2 = newest as i32 + 1;
                    // New brains read the schedule at the clock's time.
                    if let Some(clock) = data["overworld_clock"].as_i64() {
                        world.set_day_time(clock);
                    }
                    // Tags for mobs yet to appear (a summoned golem) have no state.
                    let mut order: Vec<(&String, &Value)> = observed.iter().filter_map(|(tag, states)| Some((tag, states.as_array()?.first()?))).collect();
                    order.sort_by_key(|(_, e)| e["entity_numeric_id"].as_u64().unwrap());
                    for (tag, e) in order {
                        let position = DVec3::new(number(&e["x"]), number(&e["y"]), number(&e["z"]));
                        let on_ground = e["on_ground"].as_bool().unwrap();
                        world.set_next_entity_id(e["entity_numeric_id"].as_u64().unwrap());
                        let id = match e["type"].as_str().unwrap() {
                            "minecraft:villager" => {
                                let mut villager = Villager::new(position);
                                villager.persistence_required = true;
                                villager.set_age(e["age"].as_i64().unwrap() as i32);
                                villager.body.on_ground = on_ground;
                                if e["health"].is_object() {
                                    villager.health = number(&e["health"]) as f32;
                                }
                                // Its data, experience and offers as summoned.
                                if let Some(kind) = e["villager_type"].as_str() {
                                    villager.kind = kind.to_owned();
                                }
                                if let Some(profession) = e["villager_profession"].as_str().and_then(Profession::from_id) {
                                    villager.profession = profession;
                                }
                                if let Some(level) = e["villager_level"].as_i64() {
                                    villager.level = level as i32;
                                }
                                if let Some(xp) = e["villager_xp"].as_i64() {
                                    villager.xp = xp as i32;
                                }
                                // Without AI it has no brain to tick.
                                let id = if e["no_ai"].as_bool() == Some(true) {
                                    world.spawn_villager(villager, true)
                                } else {
                                    world.spawn_villager_active(villager, number(&e["yaw"]) as f32)
                                };
                                // A memory it was summoned with.
                                if let Some(slept) = e["brain_memories"]["last_slept"]["value"].as_i64() {
                                    world.villager_mut(id).unwrap().ai.as_deref_mut().unwrap().brain.memories.last_slept.set(slept);
                                }
                                if let Some(offers) = e["offers"].as_array() {
                                    world.villager_mut(id).unwrap().offers = Some(offers.iter().map(|o| MerchantOffer::from_json(o).unwrap()).collect());
                                }
                                // Its inventory, food and looting as summoned.
                                if let Some(slots) = e["inventory"].as_array() {
                                    let entity = world.villager_mut(id).unwrap();
                                    for (slot, stack) in slots.iter().enumerate().filter(|(_, s)| !s.is_null()) {
                                        let item = stack["id"].as_str().unwrap();
                                        entity.inventory.slots[slot] = Some(ItemStack { max: max_stack(item), components: stack.get("components").cloned(), ..ItemStack::new(item, stack["count"].as_u64().unwrap() as u8) });
                                    }
                                    entity.food_level = e["food_level"].as_i64().unwrap() as i32;
                                    entity.can_pick_up_loot = e["can_pick_up_loot"].as_bool().unwrap();
                                }
                                // Gossip it was summoned with (`readAdditionalSaveData`:
                                // the codec's container, then `clear` and `putAll`).
                                if e.get("gossips").is_some() {
                                    let entity = world.villager_mut(id).unwrap();
                                    let saved = Gossips::from_entries(&gossip_entries(&e["gossips"]));
                                    std::sync::Arc::make_mut(&mut entity.gossips).replace_with(&saved);
                                    entity.last_gossip_decay = e["last_gossip_decay"].as_i64().unwrap();
                                }
                                id
                            }
                            // Monsters with their goals.
                            "minecraft:zombie" => {
                                let mut zombie = Zombie::new(position);
                                zombie.persistence_required = true;
                                zombie.body.on_ground = on_ground;
                                world.spawn_zombie_active(zombie, number(&e["yaw"]) as f32)
                            }
                            "minecraft:creeper" => {
                                let mut creeper = Creeper::new(position);
                                creeper.yaw = number(&e["yaw"]) as f32;
                                creeper.persistence_required = true;
                                creeper.body.on_ground = on_ground;
                                world.spawn_creeper(creeper, false)
                            }
                            // Still bystanders.
                            "minecraft:husk" => {
                                let mut husk = Zombie::new(position);
                                husk.kind = ZombieKind::Husk;
                                husk.persistence_required = true;
                                world.spawn_zombie(husk, true)
                            }
                            "minecraft:iron_golem" => {
                                let mut golem = IronGolem::new(position);
                                golem.yaw = number(&e["yaw"]) as f32;
                                golem.persistence_required = true;
                                golem.body.on_ground = on_ground;
                                golem.player_created = e["player_created"].as_bool().unwrap_or(false);
                                world.spawn_iron_golem(golem, false)
                            }
                            "minecraft:cow" => {
                                let mut cow = Cow::new(position);
                                cow.yaw = number(&e["yaw"]) as f32;
                                cow.persistence_required = true;
                                cow.sound_variant = CowSoundVariant::Classic;
                                world.spawn_cow(cow, true)
                            }
                            other => panic!("unsupported mob {other}"),
                        };
                        world.body_mut(id).unwrap().on_ground = on_ground;
                        ids.insert(tag.clone(), id);
                    }
                    let _ = world.take_sounds();
                    compare_pois(&mut world, data, watch.as_ref(), &format!("{scenario} tick 0"), &mut poi_frames);
                    continue;
                }
                // The level random as this tick's entities began, and the
                // clock the schedules read.
                if let Some(state) = data["level_random_before_entities"].as_u64() {
                    *world.level_random_mut() = LegacyRandom::from_raw_state(state);
                }
                if let Some(clock) = data["overworld_clock"].as_i64() {
                    world.set_day_time(clock);
                }
                // `isBrightOutside` as this tick's entities saw it.
                if let Some(bright) = observed.values().find_map(|states| states.get(0).and_then(|s| s["bright_outside"].as_bool())) {
                    world.set_bright_outside(bright);
                }
                // `Mob.checkDespawn` before each entity ticks (persistent
                // mobs' idle clocks restart), then the tick.
                // `level.players()`: the probes in the order they joined.
                let mut players: Vec<PlayerCandidate> = probes.values().map(|p| p.candidate).collect();
                players.sort_by_key(|p| p.id);
                // What each probe holds, as villagers see it.
                for p in probes.values() {
                    world.set_player_main_hand(p.candidate.id, p.inventory.slots[0].as_ref().map(|s| s.id.as_str()));
                }
                let feet: Vec<DVec3> = players.iter().map(|p| p.position).collect();
                scene.tick_blocks(world.game_time() + 1);
                world.check_despawn(&feet, false, &|_| true);
                world.tick_with_players(&mut scene, &players);
                // The item entities tick after the villagers, which are older.
                scene.tick_items(world.level_random_mut());
                // A baby takes the next pinned tag, and its brain the seed.
                for baby in world.take_born_villagers() {
                    let (tag, seed) = born_pins.pop_front().expect("each baby has a pinned seed and tag");
                    assert!(!ids.contains_key(&tag), "{scenario} tick {tick}: one baby per tag");
                    ids.insert(tag, baby);
                    brain_seeds.insert(baby, seed);
                    births += 1;
                }
                // A golem the villagers summoned takes the pinned tag.
                for golem in world.take_summoned_golems() {
                    let tag = summoned_tag.clone().expect("a summoned golem has a pinned seed and tag");
                    assert!(!ids.contains_key(&tag), "{scenario} tick {tick}: one golem summoned per tag");
                    ids.insert(tag, golem);
                    summoned += 1;
                }
                assert_eq!(world.game_time(), data["game_time"].as_i64().unwrap(), "{scenario} tick {tick} game time");
                for (&id, &seed) in &brain_seeds {
                    let Some(ai) = world.villager_mut(id).and_then(|e| e.ai.as_deref_mut()) else { continue };
                    let seen = generations.entry(id).or_insert(0);
                    if ai.brain.generation != *seen {
                        *seen = ai.brain.generation;
                        let time = ai.brain.rebuilt_at.unwrap();
                        ai.brain.reseed_lists(seed ^ time as u64);
                    }
                }
                let blocks_heard = std::mem::take(&mut scene.3.sounds);
                let (compared, _) = sounds::check_heard_with(&mut world, &blocks_heard, data, &format!("{scenario} tick {tick}"), &sounds::pens(&suite, &scenario));
                heard += compared;
                // Every mob's UUID first, so a target seen before its own frame
                // is known.
                for (tag, states) in observed {
                    if let (Some(&id), Some(uuid)) = (ids.get(tag), states.as_array().and_then(|s| s.first()).and_then(|e| e["uuid"].as_str())) {
                        mob_uuids.insert(id, uuid.to_owned());
                    }
                }
                for (tag, states) in observed {
                    let at = format!("{scenario} tick {tick} {tag}");
                    // A mob that died is gone twenty ticks later; a summoned
                    // golem not yet made has no id.
                    let Some(e) = states.as_array().and_then(|s| s.first()) else {
                        if let Some(&id) = ids.get(tag) {
                            assert!(world.body_mut(id).is_none(), "{at} removed");
                            deaths += 1;
                        }
                        continue;
                    };
                    assert!(ids.contains_key(tag), "{at}: vanilla made it, we did not");
                    if let Some(uuid) = e["uuid"].as_str() {
                        mob_uuids.insert(ids[tag], uuid.to_owned());
                    }
                    let target_uuid = |target: Option<minecraftoss_entities::monster_ai::TargetInfo>| {
                        target.map(|t| match t.target {
                            minecraftoss_entities::monster_ai::Target::Player(id) => probes.values().find(|p| p.candidate.id == id).map(|p| p.uuid.clone()).unwrap_or_default(),
                            minecraftoss_entities::monster_ai::Target::Villager(id) | minecraftoss_entities::monster_ai::Target::Mob(id) => mob_uuids.get(&id).cloned().unwrap_or_default(),
                        })
                    };
                    if e["type"] == "minecraft:zombie" || e["type"] == "minecraft:creeper" {
                        let id = ids[tag];
                        let (ai, body, random, health, tick_count, no_action) = if let Some(z) = world.zombies().iter().find(|z| z.id == id) {
                            (z.ai.as_deref().unwrap(), &z.zombie.body, z.random.raw_state(), z.zombie.health, z.tick_count, z.no_action_time)
                        } else {
                            let c = world.creepers().iter().find(|c| c.id == id).unwrap();
                            (&c.ai, &c.creeper.body, c.random.raw_state(), c.creeper.health, c.tick_count, c.no_action_time)
                        };
                        let state = &ai.state;
                        for (field, actual) in [
                            ("x", body.position.x),
                            ("y", body.position.y),
                            ("z", body.position.z),
                            ("vx", body.velocity.x),
                            ("vy", body.velocity.y),
                            ("vz", body.velocity.z),
                            ("health", f64::from(health)),
                            ("yaw", f64::from(ai.yaw)),
                            ("head_yaw", f64::from(state.look_control.head_yaw)),
                            ("body_yaw", f64::from(ai.body_rotation.body_yaw)),
                            ("pitch", f64::from(state.look_control.pitch)),
                        ] {
                            exact(&e[field], actual, &format!("{at} {field}"));
                        }
                        assert_eq!(e["on_ground"], body.on_ground, "{at} on ground");
                        assert_eq!(e["entity_tick_count"], tick_count, "{at} tick count");
                        assert_eq!(e["no_action_time"], no_action, "{at} idle time");
                        assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), random, "{at} random");
                        let goals: Vec<&str> = e["running_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                        assert_eq!(goals, ai.running_goals(), "{at} goals");
                        assert_eq!(e["target_uuid"].as_str().map(str::to_owned), target_uuid(state.target()), "{at} target");
                        let nodes: Vec<(i32, i32, i32)> = e["path_nodes"].as_array().unwrap().iter().map(|n| (n[0].as_i64().unwrap() as i32, n[1].as_i64().unwrap() as i32, n[2].as_i64().unwrap() as i32)).collect();
                        assert_eq!(nodes, state.navigation.nodes, "{at} path");
                        monster_frames += 1;
                        continue;
                    }
                    if e["type"] == "minecraft:iron_golem" {
                        let entity = world.iron_golems().iter().find(|g| g.id == ids[tag]).unwrap();
                        let ai = &entity.ai;
                        let (body, state) = (&entity.golem.body, &ai.state);
                        for (field, actual) in [
                            ("x", body.position.x),
                            ("y", body.position.y),
                            ("z", body.position.z),
                            ("vx", body.velocity.x),
                            ("vy", body.velocity.y),
                            ("vz", body.velocity.z),
                            ("yaw", f64::from(ai.yaw)),
                            ("speed", f64::from(ai.speed)),
                            ("head_yaw", f64::from(state.look_control.head_yaw)),
                            ("body_yaw", f64::from(ai.body_rotation.body_yaw)),
                            ("pitch", f64::from(state.look_control.pitch)),
                            ("move_control_x", ai.move_control.wanted.x),
                            ("move_control_y", ai.move_control.wanted.y),
                            ("move_control_z", ai.move_control.wanted.z),
                            ("look_control_x", state.look_control.wanted.x),
                            ("look_control_y", state.look_control.wanted.y),
                            ("look_control_z", state.look_control.wanted.z),
                        ] {
                            exact(&e[field], actual, &format!("{at} {field}"));
                        }
                        assert_eq!(e["on_ground"], body.on_ground, "{at} on ground");
                        assert_eq!(e["entity_tick_count"], entity.tick_count, "{at} tick count");
                        assert_eq!(e["ambient_sound_time"], entity.ambient_sound_time, "{at} ambient time");
                        // The harness records a mob's random once it has ticked.
                        if let Some(random) = e["random_state"].as_str() {
                            assert_eq!(random.parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random");
                        } else {
                            assert_eq!(entity.tick_count, 0, "{at} random before its first tick");
                        }
                        let goals: Vec<&str> = e["running_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                        assert_eq!(goals, ai.running_goals(), "{at} goals");
                        let targets: Vec<&str> = e["running_target_goals"].as_array().unwrap().iter().map(|g| g.as_str().unwrap()).collect();
                        assert_eq!(targets, ai.running_targets(), "{at} target goals");
                        for goal in ai.running_goals() {
                            *golem_goals.entry(goal.to_owned()).or_default() += 1;
                        }
                        assert_eq!(e["move_control_wanted"], ai.move_control.has_wanted(), "{at} move wanted");
                        assert_eq!(e["look_control_wanted"], state.look_control.cooldown > 0, "{at} look wanted");
                        assert_eq!(e["navigation_done"], state.navigation.is_done(), "{at} navigation done");
                        let nodes: Vec<(i32, i32, i32)> = e["path_nodes"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|n| (n[0].as_i64().unwrap() as i32, n[1].as_i64().unwrap() as i32, n[2].as_i64().unwrap() as i32))
                            .collect();
                        assert_eq!(nodes, state.navigation.nodes, "{at} path");
                        assert_eq!(e["offer_flower_tick"].as_i64().unwrap(), i64::from(entity.offer_flower_tick()), "{at} offer flower");
                        assert_eq!(e["attack_animation_tick"].as_i64().unwrap(), i64::from(entity.golem.attack_animation_tick), "{at} attack animation");
                        assert_eq!(e["anger_end_time"].as_i64().unwrap(), state.enderman.anger_end_time, "{at} anger end");
                        exact(&e["health"], f64::from(entity.golem.health), &format!("{at} health"));
                        assert_eq!(e["target_uuid"].as_str().map(str::to_owned), target_uuid(state.target()), "{at} target");
                        moved += usize::from(body.position.distance_squared(entity.previous_position) > 0.0);
                        golem_frames += 1;
                        continue;
                    }
                    if e["type"] != "minecraft:villager" {
                        // A bystander: where it is and its random.
                        let id = ids[tag];
                        let random = world.zombie_mut(id).map(|z| z.random.raw_state()).or_else(|| world.cow_mut(id).map(|c| c.random.raw_state())).unwrap();
                        assert_eq!(e["random_state"].as_str().unwrap().parse::<u64>().unwrap(), random, "{at} random");
                        let body = world.body_mut(id).unwrap();
                        for (field, actual) in [("x", body.position.x), ("y", body.position.y), ("z", body.position.z)] {
                            exact(&e[field], actual, &format!("{at} {field}"));
                        }
                        continue;
                    }
                    let entity = world.villagers().iter().find(|v| v.id == ids[tag]).unwrap();
                    let Some(ai) = entity.ai.as_deref() else {
                        // A villager without AI: where it is, its random and gossip.
                        let body = &entity.villager.body;
                        for (field, actual) in [("x", body.position.x), ("y", body.position.y), ("z", body.position.z), ("vx", body.velocity.x), ("vy", body.velocity.y), ("vz", body.velocity.z)] {
                            exact(&e[field], actual, &format!("{at} {field}"));
                        }
                        if let Some(random) = e["random_state"].as_str() {
                            assert_eq!(random.parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random");
                        }
                        if e.get("gossips").is_some() {
                            assert_eq!(entity.gossips.unpack(), by_target_then_type(gossip_entries(&e["gossips"])), "{at} gossip");
                            gossip_frames += 1;
                        }
                        continue;
                    };
                    let body = &entity.villager.body;
                    for (field, actual) in [
                        ("x", body.position.x),
                        ("y", body.position.y),
                        ("z", body.position.z),
                        ("vx", body.velocity.x),
                        ("vy", body.velocity.y),
                        ("vz", body.velocity.z),
                        ("yaw", f64::from(ai.yaw)),
                        ("head_yaw", f64::from(ai.look_control.head_yaw)),
                        ("body_yaw", f64::from(ai.body_rotation.body_yaw)),
                    ] {
                        exact(&e[field], actual, &format!("{at} {field}"));
                    }
                    assert_eq!(e["on_ground"], body.on_ground, "{at} on ground");
                    if let Some(age) = e["age"].as_i64() {
                        assert_eq!(age, i64::from(entity.villager.age.ticks), "{at} age");
                    }
                    if let Some(profession) = e["villager_profession"].as_str() {
                        assert_eq!(entity.villager.profession.id(), profession, "{at} profession");
                    }
                    // The harness records a mob's random once it has ticked (a
                    // baby's not in the tick it was born).
                    match e["random_state"].as_str() {
                        Some(random) => assert_eq!(random.parse::<u64>().unwrap(), entity.random.raw_state(), "{at} random"),
                        None => assert_eq!(entity.tick_count, 0, "{at} random before its first tick"),
                    }
                    let activities: Vec<&str> = {
                        let mut names: Vec<&str> = ai.brain.activities.active.iter().map(|a| a.name()).collect();
                        names.sort();
                        names
                    };
                    assert_eq!(e["brain_activities"], json!(activities), "{at} activities");
                    let running = ai.brain.running();
                    assert_eq!(e["brain_running"], json!(running), "{at} running behaviors");
                    for name in &running {
                        *behaviors.entry(name.clone()).or_default() += 1;
                    }
                    // The memories this brain keeps.
                    let m = &ai.brain.memories;
                    let mut ours = serde_json::Map::new();
                    let mut put = |key: &str, value: Option<Value>| {
                        if let Some(value) = value {
                            ours.insert(key.to_owned(), json!({ "value": value }));
                        }
                    };
                    put("mobs", m.mobs.get().map(|v| json!(v)));
                    put("visible_mobs", m.visible_mobs.get().map(|v| json!(v.ids)));
                    put("visible_villager_babies", m.visible_villager_babies.get().map(|v| json!(v)));
                    put("walk_target", m.walk_target.get().map(|&w| walk(w)));
                    put("look_target", m.look_target.get().map(|&t| tracker(t)));
                    put("interaction_target", m.interaction_target.get().map(|&id| json!(entity_number(id))));
                    put("breed_target", m.breed_target.get().map(|&id| json!(entity_number(id))));
                    put("nearest_hostile", m.nearest_hostile.get().map(|&id| json!(id)));
                    let players = |ids: &Vec<u64>| json!(ids.iter().map(|&id| entity_number(id)).collect::<Vec<_>>());
                    put("nearest_players", m.nearest_players.get().map(players));
                    put("nearest_visible_player", m.nearest_visible_player.get().map(|&id| json!(entity_number(id))));
                    put("nearest_visible_wanted_item", m.nearest_visible_wanted_item.get().map(|&id| json!(entity_number(id))));
                    put("nearest_visible_targetable_player", m.nearest_visible_attackable_player.get().map(|&id| json!(entity_number(id))));
                    put("nearest_visible_targetable_players", m.nearest_visible_attackable_players.get().map(players));
                    let global = |p: Pos| json!({ "dimension": "minecraft:overworld", "pos": [p.0, p.1, p.2] });
                    put("home", m.home.get().map(|&p| global(p)));
                    put("meeting_point", m.meeting_point.get().map(|&p| global(p)));
                    put("job_site", m.job_site.get().map(|&p| global(p)));
                    put("potential_job_site", m.potential_job_site.get().map(|&p| global(p)));
                    put("nearest_bed", m.nearest_bed.get().map(|&p| json!([p.0, p.1, p.2])));
                    put("last_slept", m.last_slept.get().map(|&t| json!(t)));
                    put("doors_to_close", m.doors_to_close.get().map(|doors| {
                        let mut doors = doors.clone();
                        doors.sort();
                        json!(doors.iter().map(|p| [p.0, p.1, p.2]).collect::<Vec<_>>())
                    }));
                    put("secondary_job_site", m.secondary_job_site.get().map(|sites| json!(sites.iter().map(|p| [p.0, p.1, p.2]).collect::<Vec<_>>())));
                    put("last_worked_at_poi", m.last_worked_at_poi.get().map(|&t| json!(t)));
                    put("last_woken", m.last_woken.get().map(|&t| json!(t)));
                    put("hurt_by", m.hurt_by.get().map(|kind| json!(kind)));
                    put("hurt_by_entity", m.hurt_by_entity.get().map(|&id| json!(entity_number(id))));
                    put("cant_reach_walk_target_since", m.cant_reach_walk_target_since.get().map(|&t| json!(t)));
                    put("path", m.path.get().and_then(|id| ai.paths.known.get(id)).map(|&(target, nodes, _)| json!({ "target": [target.0, target.1, target.2], "nodes": nodes })));
                    if m.golem_detected_recently.present() {
                        ours.insert("golem_detected_recently".to_owned(), json!({ "value": true, "ttl": m.golem_detected_recently.ttl().unwrap() }));
                    }
                    let mut theirs = e["brain_memories"].as_object().cloned().unwrap_or_default();
                    // The path's progress is compared through the navigation.
                    if let Some(path) = theirs.get_mut("path") {
                        path["value"].as_object_mut().unwrap().remove("next");
                    }
                    assert_eq!(Value::Object(theirs), Value::Object(ours), "{at} memories");
                    // Trading: experience, level, trader, what it holds up, the
                    // head-shake, its offers and effects.
                    if let Some(xp) = e["villager_xp"].as_i64() {
                        assert_eq!(xp, i64::from(entity.villager.xp), "{at} experience");
                        assert_eq!(e["villager_level"].as_i64().unwrap(), i64::from(entity.villager.level), "{at} level");
                        let trader = entity.trading_player.and_then(|id| probes.iter().find(|(_, p)| p.candidate.id == id)).map(|(tag, _)| format!("Probe_{tag}"));
                        assert_eq!(e["trading_player"].as_str().map(str::to_owned), trader, "{at} trading player");
                        assert!(e["last_traded_player"].is_null() || entity.last_traded_player.is_some(), "{at} last traded player");
                        let held = entity.held_item.as_ref().map(|h| ItemStack { max: 64, components: (!h.components.is_empty()).then(|| Value::Object(h.components.clone())), ..ItemStack::new(h.id.clone(), h.count as u8) });
                        assert_eq!(stack_json(&held), e["main_hand"], "{at} held item");
                        assert_eq!(e["villager_unhappy"].as_i64().unwrap(), i64::from(entity.unhappy), "{at} unhappy");
                        let ours = entity.offers.as_ref().map(|o| Value::Array(o.iter().map(MerchantOffer::to_json).collect()));
                        // Offers as made so far, when the scenario records them.
                        if observes_offers {
                            assert_eq!(ours.as_ref().map(normalize_offers), e.get("offers").map(normalize_offers), "{at} offers");
                        }
                        if let Some(effects) = e["effects"].as_array() {
                            let theirs: Vec<(String, i64, i64)> = effects.iter().map(|x| (x["id"].as_str().unwrap().to_owned(), x["duration"].as_i64().unwrap(), x["amplifier"].as_i64().unwrap_or(0))).collect();
                            let mut ours: Vec<(String, i64, i64)> = entity.effects.iter().map(|x| (x.effect.id().to_owned(), i64::from(x.duration), i64::from(x.amplifier))).collect();
                            ours.sort();
                            assert_eq!(ours, theirs, "{at} effects");
                        }
                        trade_frames += 1;
                    }
                    // Its eight slots and food level.
                    if let Some(slots) = e["inventory"].as_array() {
                        let ours: Vec<Value> = entity.inventory.slots.iter().map(stack_json).collect();
                        assert_eq!(&ours, slots, "{at} inventory");
                        assert_eq!(e["food_level"].as_i64().unwrap(), i64::from(entity.food_level), "{at} food level");
                        assert_eq!(e["can_pick_up_loot"].as_bool().unwrap(), entity.can_pick_up_loot, "{at} can pick up loot");
                        inventory_frames += 1;
                    }
                    // Gossip in the map's order, and its times.
                    if e.get("gossips").is_some() {
                        let theirs = by_target_then_type(gossip_entries(&e["gossips"]));
                        assert_eq!(entity.gossips.unpack(), theirs, "{at} gossip");
                        assert_eq!(e["last_gossip_time"].as_i64().unwrap(), ai.last_gossip_time, "{at} gossip time");
                        assert_eq!(e["last_gossip_decay"].as_i64().unwrap(), entity.last_gossip_decay, "{at} gossip decay time");
                        gossip_frames += 1;
                    }
                    moved += usize::from(body.position.distance_squared(entity.previous_position) > 0.0);
                }
                if let Some(seed) = data["random_state"]["seed"].as_u64() {
                    assert_eq!(world.level_random_mut().raw_state(), seed, "{scenario} tick {tick} level random");
                }
                // The item entities left: their stacks, pickup delays, ages
                // and motion.
                if let Some(items) = data["entity_type_states"]["minecraft:item"].as_array() {
                    let theirs: Vec<(i64, Value, i64, i64)> = items.iter().map(|e| (e["entity_numeric_id"].as_i64().unwrap(), e["item"].clone(), e["pickup_delay"].as_i64().unwrap(), e["item_age"].as_i64().unwrap())).collect();
                    let ours: Vec<(i64, Value, i64, i64)> = scene
                        .1
                        .iter()
                        .map(|i| {
                            let mut stack = json!({ "id": i.item, "count": i.count });
                            if let Some(c) = &i.components {
                                stack["components"] = c.clone();
                            }
                            (i64::from(i.id), stack, i64::from(i.pickup_delay), i64::from(i.age))
                        })
                        .collect();
                    assert_eq!(ours, theirs, "{scenario} tick {tick} items");
                    for (e, i) in items.iter().zip(&scene.1) {
                        let at = format!("{scenario} tick {tick} item {}", i.id);
                        for (field, actual) in [("x", i.position.x), ("y", i.position.y), ("z", i.position.z), ("vx", i.velocity.x), ("vy", i.velocity.y), ("vz", i.velocity.z)] {
                            exact(&e[field], actual, &format!("{at} {field}"));
                        }
                        assert_eq!(e["on_ground"].as_bool().unwrap(), i.on_ground, "{at} on ground");
                    }
                    item_frames += 1;
                }
                // The probes' inventories and screens, and the experience
                // trades dropped.
                for (tag, p) in &probes {
                    let Some(theirs) = data["player_probes"].get(tag.as_str()).filter(|t| t.get("inventory").is_some()) else { continue };
                    let at = format!("{scenario} tick {tick} probe {tag}");
                    let ours: serde_json::Map<String, Value> = p.inventory.slots.iter().enumerate().take(41).filter(|(_, s)| s.as_ref().is_some_and(|s| s.count > 0)).map(|(i, s)| (i.to_string(), stack_json(s))).collect();
                    assert_eq!(Value::Object(ours), theirs["inventory"], "{at} inventory");
                    assert_eq!(stack_json(&p.inventory.cursor), theirs["cursor"], "{at} cursor");
                    // Gson leaves out empty slots.
                    let menu = world.merchant_menu(p.candidate.id).map(|m| {
                        let mut v = json!({ "payment_a": stack_json(&m.payment[0]), "payment_b": stack_json(&m.payment[1]), "result": stack_json(&m.result), "future_xp": m.future_xp });
                        v.as_object_mut().unwrap().retain(|_, x| !x.is_null());
                        v
                    });
                    assert_eq!(menu.unwrap_or(Value::Null), theirs.get("merchant").cloned().unwrap_or(Value::Null), "{at} screen");
                }
                experience += world.take_trade_experience().iter().map(|&(_, xp)| i64::from(xp)).sum::<i64>();
                if let Some(total) = data["experience_total"].as_i64() {
                    assert_eq!(total, experience, "{scenario} tick {tick} experience dropped");
                }
                compare_pois(&mut world, data, watch.as_ref(), &format!("{scenario} tick {tick}"), &mut poi_frames);
                // Beds in the observed region: occupied as vanilla has them.
                if let Some(blocks) = data["blocks"].as_object() {
                    for (key, state) in blocks {
                        let p: Vec<i32> = key.split(',').map(|v| v.parse().unwrap()).collect();
                        let ours = scene.block((p[0], p[1], p[2]));
                        if state["id"].as_str().unwrap().ends_with("_bed") {
                            let ours = ours.unwrap_or_else(|| panic!("{scenario} tick {tick} bed at {key} missing"));
                            assert_eq!(ours.property("occupied").unwrap_or("false"), state["properties"]["occupied"].as_str().unwrap(), "{scenario} tick {tick} bed {key} occupied");
                        } else if state["id"].as_str().unwrap().ends_with("_door") {
                            let ours = ours.unwrap_or_else(|| panic!("{scenario} tick {tick} door at {key} missing"));
                            assert_eq!(ours.property("open").unwrap_or("false"), state["properties"]["open"].as_str().unwrap(), "{scenario} tick {tick} door {key} open");
                        } else {
                            // Crops, farmland and what they became: the block and its age.
                            let id = ours.as_ref().map_or("minecraft:air", |b| b.id.as_str());
                            assert_eq!(id, state["id"].as_str().unwrap(), "{scenario} tick {tick} block {key}");
                            if let Some(age) = state["properties"]["age"].as_str() {
                                assert_eq!(ours.as_ref().and_then(|b| b.property("age")), Some(age), "{scenario} tick {tick} block {key} age");
                            }
                            crop_frames += 1;
                        }
                    }
                }
                frames += 1;
            }
            _ => {}
        }
    }
    assert!(complete, "trace is complete");
    if poi_frames > 0 {
        println!("{poi_frames} exact point-of-interest frames matched");
    }
    if golem_frames > 0 {
        println!("{golem_frames} exact iron golem frames matched (running goals {golem_goals:?})");
    }
    if monster_frames > 0 {
        println!("{monster_frames} exact monster frames matched ({deaths} removed-mob frames)");
    }
    if summoned > 0 {
        println!("{summoned} golems summoned by villagers, as in vanilla");
    }
    if trade_frames > 0 {
        println!("{trade_frames} exact villager trading frames matched ({trade_actions} trading actions, {experience} experience dropped)");
    }
    if gossip_frames > 0 {
        println!("{gossip_frames} exact villager gossip frames matched ({attacks} probe attacks)");
    }
    if crop_frames > 0 {
        println!("{crop_frames} exact crop and farmland block states matched");
    }
    if births > 0 {
        println!("{births} babies born, as in vanilla");
    }
    if item_frames > 0 {
        println!("{item_frames} exact item entity frames matched");
    }
    if inventory_frames > 0 {
        println!("{inventory_frames} exact villager inventory frames matched");
    }
    println!("{frames} exact villager frames matched ({heard} sounds, {hurts} hurts, {moved} moving villager-ticks; running behaviors {behaviors:?})");
}
