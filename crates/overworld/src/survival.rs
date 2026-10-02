//! Survival state and menu transactions. The CS simulation remains the movement/weapon authority.
use crate::*;
use mc_player::{
    chest::Chest,
    food::{FoodUse, FoodUseTick},
    furnace::Furnace,
    inventory::{Inventory, ItemStack},
    survival::{Difficulty, FoodData},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Survival {
    pub money: i32,
    pub loadout: Option<Loadout>,
    pub inventory: Inventory,
    pub furnaces: HashMap<String, Furnace>,
    pub chests: HashMap<String, Chest>,
    pub food: FoodData,
    pub health: f32,
    pub air: i32,
    pub spawn: Option<[f64; 3]>,
    pub last_y: f64,
    pub fall: f64,
    pub ticks: u64,
    pub respawn_pending: bool,
}
impl Default for Survival {
    fn default() -> Self {
        Self {
            money: 0,
            loadout: None,
            inventory: Inventory::default(),
            furnaces: HashMap::new(),
            chests: HashMap::new(),
            food: FoodData::default(),
            health: 20.,
            air: 300,
            spawn: None,
            last_y: 0.,
            fall: 0.,
            ticks: 0,
            respawn_pending: false,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Loadout {
    pub weapons: Vec<(String, i32, i32)>,
    pub active: usize,
    pub armor: i32,
    pub helmet: bool,
    pub defuse_kit: bool,
}
/// Payment is driven by consumed server death records, never entity disappearance.
pub fn mob_reward(kind: &str) -> i32 {
    match kind.trim_start_matches("minecraft:") {
        "zombie" | "zombie_villager" | "husk" | "drowned" | "skeleton" | "stray" |
        "bogged" | "wither_skeleton" | "creeper" | "spider" | "cave_spider" |
        "slime" | "magma_cube" | "enderman" | "endermite" | "silverfish" | "witch" |
        "blaze" | "ghast" | "piglin" | "piglin_brute" | "zombified_piglin" |
        "hoglin" | "zoglin" | "guardian" | "elder_guardian" | "shulker" | "phantom" |
        "pillager" | "vindicator" | "evoker" | "vex" | "ravager" | "warden" |
        "wither" | "ender_dragon" | "breeze" | "creaking" => 100,
        _ => 25,
    }
}
impl Runtime {
    pub fn reward_mob(&mut self, kind: &str) {
        self.mob_kills.push(kind.to_owned());
        let reward = mob_reward(kind);
        self.save.survival.money = (self.save.survival.money + reward).min(16000);
        self.notices.push(format!("+${reward} — {}", kind.trim_start_matches("minecraft:").replace('_', " ")));
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Closed,
    Inventory,
    Workbench(BlockPos),
    Furnace(BlockPos),
    Chest(BlockPos),
    Victory,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Inventory(usize),
    Craft(usize),
    Output,
    Container(usize),
}
impl Runtime {
    pub fn container_key(&self, p: BlockPos) -> String {
        format!("{}:{},{},{}", self.save.adventure.dimension, p.0, p.1, p.2)
    }
    pub fn drop_stack(&mut self, stack: ItemStack, p: [f64; 3]) {
        self.server.spawn_item(
            &stack.id,
            stack.count as i32,
            stack.components.as_ref().map(|c| c.to_string()).as_deref(),
            [p[0], p[1] + 0.4, p[2]],
            [0., 0.12, 0.],
            10,
            0,
        );
    }
    pub fn consume_selected(&mut self) {
        let i = self.save.selected.min(8);
        if let Some(s) = &mut self.inventory.slots[i] {
            s.count = s.count.saturating_sub(1);
            if s.count == 0 {
                self.inventory.slots[i] = None;
            }
        }
    }
    pub fn close_screen(&mut self) {
        let mut drops = self.inventory.settle_crafting();
        drops.extend(self.inventory.settle_workbench());
        drops.extend(self.inventory.settle_cursor());
        drops.extend(self.inventory.take_pending_drops());
        for s in drops {
            self.drop_stack(s, self.save.player);
        }
        self.screen = Screen::Closed;
    }
    pub fn click_slot(&mut self, slot: Slot, right: bool, shift: bool) {
        let screen = self.screen.clone();
        match (slot, screen) {
            (Slot::Inventory(i), Screen::Furnace(p)) if shift => {
                let k = self.container_key(p);
                self.save
                    .survival
                    .furnaces
                    .entry(k)
                    .or_default()
                    .quick_move_from_inventory(i, &mut self.inventory);
            }
            (Slot::Inventory(i), Screen::Chest(p)) if shift => {
                let k = self.container_key(p);
                self.save
                    .survival
                    .chests
                    .entry(k)
                    .or_default()
                    .quick_move_from_inventory(i, &mut self.inventory);
            }
            (Slot::Inventory(i), _) => {
                if let Some(s) = self.inventory.click(Some(i), right, shift) {
                    self.drop_stack(s, self.save.player);
                }
            }
            (Slot::Craft(i), Screen::Inventory) => {
                self.inventory.click_crafting_slot(i, right, shift)
            }
            (Slot::Craft(i), Screen::Workbench(_)) => {
                self.inventory.click_workbench_slot(i, right, shift)
            }
            (Slot::Output, Screen::Inventory) => {
                self.inventory.take_crafting_output(shift);
            }
            (Slot::Output, Screen::Workbench(_)) => {
                self.inventory.take_workbench_output(shift);
            }
            (Slot::Container(i), Screen::Furnace(p)) => {
                let k = self.container_key(p);
                self.save
                    .survival
                    .furnaces
                    .entry(k)
                    .or_default()
                    .click_slot(i, right, shift, &mut self.inventory);
            }
            (Slot::Container(i), Screen::Chest(p)) => {
                let k = self.container_key(p);
                self.save.survival.chests.entry(k).or_default().click_slot(
                    i,
                    right,
                    shift,
                    &mut self.inventory,
                );
            }
            _ => {}
        }
        for s in self.inventory.take_pending_drops() {
            self.drop_stack(s, self.save.player);
        }
    }
    pub fn open_block(&mut self, p: BlockPos) -> bool {
        let block = {
            let s = self.collision.scene.read().unwrap();
            Scene::block(&*s, p).cloned()
        };
        let Some(b) = block else { return false };
        self.screen = match b.id.path.as_str() {
            "crafting_table" => Screen::Workbench(p),
            "furnace" | "blast_furnace" | "smoker" => {
                let kind = match b.id.path.as_str() {
                    "blast_furnace" => mc_player::crafting::CookingKind::BlastFurnace,
                    "smoker" => mc_player::crafting::CookingKind::Smoker,
                    _ => mc_player::crafting::CookingKind::Furnace,
                };
                let k = self.container_key(p);
                self.save
                    .survival
                    .furnaces
                    .entry(k)
                    .or_insert_with(|| Furnace::new(kind));
                Screen::Furnace(p)
            }
            "chest" | "barrel" | "trapped_chest" => {
                let k = self.container_key(p);
                self.save.survival.chests.entry(k).or_default();
                Screen::Chest(p)
            }
            id if id.ends_with("_bed") && self.save.adventure.dimension == "overworld" => {
                self.save.survival.spawn = Some(self.save.player);
                if self.save.day_ticks % 24000. > 12500. {
                    self.save.day_ticks = (self.save.day_ticks / 24000.).floor() * 24000. + 24000.;
                }
                self.notices.push("Respawn point set".into());
                return true;
            }
            _ => return false,
        };
        true
    }
    pub fn tick_furnaces(&mut self) {
        let prefix = format!("{}:", self.save.adventure.dimension);
        let mut updates = vec![];
        for (key, furnace) in &mut self.save.survival.furnaces {
            if !key.starts_with(&prefix) {
                continue;
            }
            let coords: Vec<i32> = key[prefix.len()..]
                .split(',')
                .filter_map(|s| s.parse().ok())
                .collect();
            if coords.len() != 3 {
                continue;
            }
            let pos = (coords[0], coords[1], coords[2]);
            let scene = self.collision.scene.read().unwrap();
            if scene.generated_chunk((pos.0 >> 4, pos.2 >> 4)).is_none() {
                continue;
            }
            let before = furnace.is_lit();
            furnace.tick(&self.inventory.recipes);
            if before != furnace.is_lit() {
                if let Some(mut b) = Scene::block(&*scene, pos).cloned() {
                    b.properties
                        .insert("lit".into(), furnace.is_lit().to_string());
                    updates.push((pos, b));
                }
            }
        }
        for (p, b) in updates {
            self.set_block(p, Some(b));
        }
    }
    pub fn mine_survival(
        &mut self,
        dt: f32,
        feet: Vec3,
        yaw: f32,
        pitch: f32,
        ground: bool,
        attacking: bool,
    ) {
        if !attacking {
            self.mining.reset();
            self.mine_clock = 0.;
            self.mining_hits.tick(None);
            return;
        }
        self.mine_clock += dt;
        if self.mine_clock < 0.05 {
            return;
        }
        self.mine_clock -= 0.05;
        let mut player =
            mc_player::Player::new(glam_mc::DVec3::from_array(to_block(self.save.origin, feet)));
        player.yaw = (-yaw - 90.) as f64;
        player.pitch = pitch as f64;
        player.on_ground = ground;
        let held = self.inventory.slots[self.save.selected.min(8)].clone();
        let broken = {
            let mut s = self.collision.scene.write().unwrap();
            self.mining.tick(&mut *s, &player, held.as_ref(), true)
        };
        let target=self.mining.target();
        if broken.is_none() && self.mining_hits.tick(target) {
            if let Some(id)=target.and_then(|p|self.block_name(p)) {self.block_sound(&id,block_audio::Action::Hit);}
        }
        if let Some(b) = broken {
            self.mining_hits.tick(None);
            self.set_block(b.pos, None);
            self.inventory
                .wear_tool_after_mining(self.save.selected.min(8), b.hardness);
            self.save.survival.food.add_exhaustion(0.005);
            let pos = [
                b.pos.0 as f64 + 0.5,
                b.pos.1 as f64 + 0.5,
                b.pos.2 as f64 + 0.5,
            ];
            for stack in b.drops {
                self.drop_stack(stack, pos);
            }
            let key = self.container_key(b.pos);
            if let Some(mut f) = self.save.survival.furnaces.remove(&key) {
                for s in f.take_contents() {
                    self.drop_stack(s, pos);
                }
            }
            if let Some(mut c) = self.save.survival.chests.remove(&key) {
                for s in c.take_contents() {
                    self.drop_stack(s, pos);
                }
            }
            self.block_sound(&b.block.id,block_audio::Action::Break);
            if b.loot_unresolved {
                eprintln!("Unresolved loot table: {}", b.block.id);
            }
        }
    }
    /// Called at 20 Hz. Returns CS health delta; all damage enters the normal CS feedback path.
    pub fn survival_tick(
        &mut self,
        health: i32,
        alive: bool,
        ground: bool,
        using_food: bool,
        moved: f32,
    ) -> i32 {
        if !alive {
            if self.was_alive {
                self.close_screen();
                for stack in self.inventory.drain_on_death() {
                    self.drop_stack(stack, self.save.player);
                }
                self.save.survival.food = FoodData::default();
                self.save.survival.health = 20.;
                self.save.survival.respawn_pending = true;
                self.save.survival.air = 300;
                self.save.survival.fall = 0.;
                self.eating = None;
            }
            self.was_alive = false;
            return 0;
        }
        self.was_alive = true;
        let s = &mut self.save.survival;
        s.ticks += 1;
        s.food.add_exhaustion(moved.max(0.) * 0.01);
        let mut hp = health as f32 / 5.;
        if s.food.tick(&mut hp, 20., true, Difficulty::Normal) {
            hp -= 1.;
        }
        let p = self.save.player;
        let (body, eye) = {
            let scene = self.collision.scene.read().unwrap();
            (
                Scene::block(&*scene, block_pos(p)).map(|b| b.id.path.clone()),
                Scene::block(&*scene, block_pos([p[0], p[1] + 64.0 / BLOCK as f64, p[2]]))
                    .map(|b| b.id.path.clone()),
            )
        };
        if eye.as_deref() == Some("water") {
            s.air -= 1;
            if s.air <= -20 {
                hp -= 2.;
                s.air = 0;
            }
        } else {
            s.air = (s.air + 4).min(300);
        }
        if body.as_deref() == Some("lava") && s.ticks % 10 == 0 {
            hp -= 4.;
        }
        if p[1] < -64. && s.ticks % 10 == 0 {
            hp -= 4.;
        }
        if !ground && s.last_y > p[1] && s.last_y - p[1] < 10. {
            s.fall += s.last_y - p[1];
        }
        let climbing=collision::DynamicWorld::climbable(&*self.collision,to_source(self.save.origin,p),glam::Vec3::new(-16.,-16.,0.),glam::Vec3::new(16.,16.,72.))!=collision::Climbable::None;
        if body.as_deref() == Some("water") || climbing {
            s.fall = 0.;
        }
        if ground {
            if s.fall > 3. {
                hp -= (s.fall - 3.).ceil() as f32;
            }
            s.fall = 0.;
        }
        s.last_y = p[1];
        if !using_food {
            self.eating = None;
        }
        if using_food && self.eating.is_none() {
            if let Some(stack) = &self.inventory.slots[self.save.selected.min(8)] {
                self.eating = FoodUse::start(
                    self.save.selected.min(8),
                    stack,
                    &s.food,
                    mc_player::GameMode::Survival,
                );
            }
        }
        if let Some(eating) = &mut self.eating {
            match eating.tick(
                &mut self.inventory,
                &mut s.food,
                mc_player::GameMode::Survival,
            ) {
                FoodUseTick::Finished { overflow } => {
                    self.eating = None;
                    if let Some(stack) = overflow {
                        self.drop_stack(stack, p);
                    }
                    self.sounds.push(("entity.player.burp".into(), p));
                }
                FoodUseTick::Cancelled => self.eating = None,
                FoodUseTick::Continuing { emit_sound } => {
                    if emit_sound {
                        self.sounds.push(("entity.generic.eat".into(), p));
                    }
                }
            }
        }
        self.save.survival.health = hp.clamp(0., 20.);
        (hp * 5.).round() as i32 - health
    }
}
pub fn recipe_book(root: &std::path::Path) -> Result<mc_player::crafting::RecipeBook, String> {
    let book = mc_player::crafting::RecipeBook::from_jar(&root.join("client.jar"))
        .map_err(|e| e.to_string())?;
    let catalog = mc_player::item_catalog::ItemCatalog::from_path(
        &root.join("artifacts/item-catalog/26.3.json"),
    )
    .map_err(|e| e.to_string())?;
    Ok(book.with_item_catalog(Arc::new(catalog)))
}
impl Runtime {
    pub fn load_mining_catalog(&mut self) {
        let r = &self.registries;
        for (_, block) in r.blocks.blocks() {
            let state = block.default_state();
            let tagged = |name: &str| {
                r.block_tags
                    .id(&format!("minecraft:{name}"))
                    .is_some_and(|tag| r.block_in_tag(state, tag))
            };
            let requires = tagged("mineable/pickaxe");
            let mut tools = vec![];
            for (material, speed) in [
                ("wooden", 2.),
                ("stone", 4.),
                ("iron", 6.),
                ("golden", 12.),
                ("diamond", 8.),
                ("netherite", 9.),
            ] {
                let tag_material = match material {
                    "wooden" => "wood",
                    "golden" => "gold",
                    s => s,
                };
                for tool in ["pickaxe", "axe", "shovel", "hoe"] {
                    let matches = tagged(&format!("mineable/{tool}"));
                    let correct = matches && !tagged(&format!("incorrect_for_{tag_material}_tool"));
                    tools.push((
                        format!("minecraft:{material}_{tool}"),
                        if matches { speed } else { 1. },
                        correct,
                    ));
                }
            }
            self.mining.register_block(
                block.name.to_string(),
                r.blocks.state(state).destroy_speed,
                requires,
                tools,
            );
        }
    }
}

#[cfg(test)]
mod economy_tests {
    use super::*;
    #[test]
    fn survival_defaults_empty_and_wallet_loadout_roundtrip() {
        let mut s=Survival::default();
        assert!(s.inventory.slots.iter().all(Option::is_none));assert_eq!(s.money,0);assert!(s.loadout.is_none());
        s.money=125;s.loadout=Some(Loadout{weapons:vec![("weapon_usp_silencer".into(),7,18)],active:0,armor:47,helmet:true,defuse_kit:false});
        let restored:Survival=serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(restored.money,125);assert_eq!(restored.loadout.unwrap().weapons[0].1,7);
        let old:Survival=serde_json::from_str("{}").unwrap();assert_eq!(old.money,0);
        assert_eq!(mob_reward("minecraft:cow"),25);assert_eq!(mob_reward("minecraft:zombie"),100);assert_eq!(mob_reward("minecraft:blaze"),100);
    }
}
