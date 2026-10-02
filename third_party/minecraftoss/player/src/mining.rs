//! Fixed-tick survival block breaking. A repeatable 26.3 catalog supplies
//! default-state hardness and item tool properties when available; loot is
//! still a separate, limited implementation.
use crate::{
    inventory::ItemStack, loot::LootBook, rng::XoroshiroRandom, Block, Player, Pos, World,
};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::{collections::HashMap, path::Path};

#[derive(Clone, Copy, Debug)]
struct ToolInfo {
    speed: f32,
    correct: bool,
}

#[derive(Clone, Debug)]
struct CatalogBlock {
    hardness: f32,
    requires_tool: bool,
    tools: HashMap<String, ToolInfo>,
}

/// Exported by the pinned Fabric harness after two exact vanilla runs.
#[derive(Clone, Debug)]
pub struct MiningCatalog {
    blocks: HashMap<String, CatalogBlock>,
}
impl MiningCatalog {
    pub fn from_path(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_slice(&bytes)
    }

    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let root: Value = serde_json::from_slice(bytes)?;
        if root["schema_version"] != 1 || root["minecraft_version"] != "26.3" {
            bail!("expected pinned 26.3 mining catalog schema 1");
        }
        let raw = root["blocks"]
            .as_object()
            .ok_or_else(|| anyhow!("missing block catalog"))?;
        let mut blocks = HashMap::with_capacity(raw.len());
        for (id, value) in raw {
            let hardness = exact_f32(&value["hardness"])?;
            let requires_tool = value["requires_tool"]
                .as_bool()
                .ok_or_else(|| anyhow!("missing requires_tool for {id}"))?;
            let raw_tools = value["tools"]
                .as_object()
                .ok_or_else(|| anyhow!("missing tools for {id}"))?;
            let mut tools = HashMap::with_capacity(raw_tools.len());
            for (item, tool) in raw_tools {
                tools.insert(
                    item.clone(),
                    ToolInfo {
                        speed: exact_f32(&tool["speed"])?,
                        correct: tool["correct"]
                            .as_bool()
                            .ok_or_else(|| anyhow!("missing correct for {id}/{item}"))?,
                    },
                );
            }
            blocks.insert(
                id.clone(),
                CatalogBlock {
                    hardness,
                    requires_tool,
                    tools,
                },
            );
        }
        Ok(Self { blocks })
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }
}

fn exact_f32(value: &Value) -> Result<f32> {
    let decimal = value["decimal"]
        .as_str()
        .ok_or_else(|| anyhow!("missing float decimal"))?;
    let bits = value["bits"]
        .as_str()
        .ok_or_else(|| anyhow!("missing float bits"))?;
    let float = decimal.parse::<f32>()?;
    if float.to_bits() != u32::from_str_radix(bits, 16)? {
        bail!("float decimal and bits disagree");
    }
    Ok(float)
}

#[derive(Default)]
pub struct Mining {
    catalog: Option<MiningCatalog>,
    loot: Option<LootBook>,
    loot_seed: u64,
    loot_sequences: HashMap<String, XoroshiroRandom>,
    target: Option<Pos>,
    target_state: Option<Block>,
    destroying_item: Option<ItemStack>,
    ticks: u32,
    progress: f32,
}

pub struct BrokenBlock {
    pub pos: Pos,
    pub block: Block,
    pub hardness: f32,
    pub drops: Vec<ItemStack>,
    /// The supplied vanilla table needs context or random rules this engine
    /// cannot evaluate yet. No fallback drop is invented in that case.
    pub loot_unresolved: bool,
}

impl Mining {
    pub fn with_catalog(catalog: MiningCatalog) -> Self {
        Self {
            catalog: Some(catalog),
            ..Self::default()
        }
    }
    /// Host-supplied registry hardness and tag-derived tool properties.
    pub fn register_block(&mut self,id:String,hardness:f32,requires_tool:bool,tools:Vec<(String,f32,bool)>) {
        let catalog=self.catalog.get_or_insert_with(||MiningCatalog{blocks:HashMap::new()});
        catalog.blocks.insert(id,CatalogBlock{hardness,requires_tool,tools:tools.into_iter().map(|(id,speed,correct)|(id,ToolInfo{speed,correct})).collect()});
    }
    pub fn target(&self) -> Option<Pos> { self.target }
    pub fn progress(&self) -> f32 {
        self.progress
    }
    pub fn set_loot(&mut self, loot: LootBook) {
        self.loot = Some(loot);
    }
    pub fn set_loot_seed(&mut self, seed: u64) {
        self.loot_seed = seed;
        self.loot_sequences.clear();
    }

    pub fn reset(&mut self) {
        self.target = None;
        self.target_state = None;
        self.destroying_item = None;
        self.ticks = 0;
        self.progress = 0.0;
    }

    /// MultiPlayerGameMode.sameDestroyTarget compares the held item's type
    /// and components, independent of its count or hotbar slot.
    pub fn reset_if_item_changed(&mut self, held_item: Option<&ItemStack>) -> bool {
        if self.target.is_none() {
            return false;
        }
        let same = match (self.destroying_item.as_ref(), held_item) {
            (Some(previous), Some(current)) => previous.same_item(current),
            (None, None) => true,
            _ => false,
        };
        if same {
            return false;
        }
        self.reset();
        true
    }

    pub fn tick(
        &mut self,
        world: &mut impl World,
        player: &Player,
        held_item: Option<&ItemStack>,
        attacking: bool,
    ) -> Option<BrokenBlock> {
        if !attacking {
            self.reset();
            return None;
        }
        self.reset_if_item_changed(held_item);
        let Some(hit) = player.target(world, 5.0) else {
            self.reset();
            return None;
        };
        let Some(block) = world.block(hit.pos) else {
            self.reset();
            return None;
        };
        if self.target != Some(hit.pos) || self.target_state.as_ref() != Some(&block) {
            self.target = Some(hit.pos);
            self.target_state = Some(block.clone());
            self.destroying_item = held_item.cloned();
            self.ticks = 0;
            self.progress = 0.0;
        }
        let held_id = held_item.map(|item| item.id.as_str());
        let fallback = BlockInfo::for_id(&block.id);
        let (hardness, correct_tool, mut speed) = if let Some(info) = self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.blocks.get(&block.id))
        {
            let tool = held_id.and_then(|item| info.tools.get(item));
            (
                info.hardness,
                !info.requires_tool || tool.is_some_and(|tool| tool.correct),
                tool.map_or(1.0, |tool| tool.speed),
            )
        } else if let Some(info) = fallback {
            let correct = !info.requires_tool
                || (tool_matches(held_id, info.tool) && tool_level(held_id) >= info.min_tier);
            let speed = if tool_matches(held_id, info.tool) {
                tool_speed(held_id)
            } else {
                1.0
            };
            (info.hardness, correct, speed)
        } else {
            return None;
        };
        if hardness < 0.0 {
            self.progress = 0.0;
            return None;
        }
        let eye = player.eye().floor();
        if world
            .block((eye.x as i32, eye.y as i32, eye.z as i32))
            .is_some_and(|block| block.id == "minecraft:water")
        {
            speed *= 0.2;
        }
        if !player.on_ground {
            speed /= 5.0;
        }
        self.ticks += 1;
        self.progress =
            speed / hardness / if correct_tool { 30.0 } else { 100.0 } * self.ticks as f32;
        if self.progress < 1.0 {
            return None;
        }
        world.set_block(hit.pos, None);
        self.reset();
        let (drops, loot_unresolved) = if correct_tool {
            if let Some(loot) = &self.loot {
                match loot.roll_drops_named(
                    &block,
                    held_item,
                    self.loot_seed,
                    &mut self.loot_sequences,
                ) {
                    Some(drops) => (drops, false),
                    None => (Vec::new(), true),
                }
            } else {
                (
                    fallback
                        .and_then(|info| info.drop)
                        .map(|id| vec![ItemStack::new(id, 1)])
                        .unwrap_or_default(),
                    false,
                )
            }
        } else {
            (vec![], false)
        };
        Some(BrokenBlock {
            pos: hit.pos,
            block,
            hardness,
            drops,
            loot_unresolved,
        })
    }
}

#[derive(Clone, Copy)]
struct BlockInfo {
    hardness: f32,
    requires_tool: bool,
    min_tier: u8,
    tool: &'static str,
    drop: Option<&'static str>,
}
impl BlockInfo {
    fn for_id(id: &str) -> Option<Self> {
        let id = id.rsplit(':').next().unwrap_or(id);
        let (hardness, requires_tool, min_tier, tool, drop) = match id {
            "stone" => (1.5, true, 1, "pickaxe", Some("minecraft:cobblestone")),
            "cobblestone" => (2.0, true, 1, "pickaxe", Some("minecraft:cobblestone")),
            "grass_block" => (0.6, false, 0, "shovel", Some("minecraft:dirt")),
            "dirt" => (0.5, false, 0, "shovel", Some("minecraft:dirt")),
            "sand" => (0.5, false, 0, "shovel", Some("minecraft:sand")),
            "oak_planks" => (2.0, false, 0, "axe", Some("minecraft:oak_planks")),
            "oak_log" => (2.0, false, 0, "axe", Some("minecraft:oak_log")),
            "crafting_table" => (2.5, false, 0, "axe", Some("minecraft:crafting_table")),
            "chest" => (2.5, false, 0, "axe", Some("minecraft:chest")),
            "furnace" => (3.5, true, 1, "pickaxe", Some("minecraft:furnace")),
            "oak_leaves" | "birch_leaves" => (0.2, false, 0, "hoe", None),
            "glass" | "blue_stained_glass" => (0.3, false, 0, "none", None),
            "tinted_glass" => (0.3, false, 0, "none", Some("minecraft:tinted_glass")),
            "bricks" => (2.0, true, 1, "pickaxe", Some("minecraft:bricks")),
            "gold_block" => (3.0, true, 3, "pickaxe", Some("minecraft:gold_block")),
            "bedrock" | "end_portal_frame" => (-1.0, false, 0, "none", None),
            "obsidian" | "crying_obsidian" => (50.0, true, 4, "pickaxe", Some("minecraft:obsidian")),
            "iron_ore" | "deepslate_iron_ore" => (3.0, true, 2, "pickaxe", Some("minecraft:raw_iron")),
            "gold_ore" | "deepslate_gold_ore" => (3.0, true, 3, "pickaxe", Some("minecraft:raw_gold")),
            "diamond_ore" | "deepslate_diamond_ore" => (3.0, true, 3, "pickaxe", Some("minecraft:diamond")),
            "coal_ore" | "deepslate_coal_ore" => (3.0, true, 1, "pickaxe", Some("minecraft:coal")),
            "gravel" => (0.6, false, 0, "shovel", Some("minecraft:gravel")),
            "netherrack" => (0.4, true, 1, "pickaxe", Some("minecraft:netherrack")),
            "end_stone" => (3.0, true, 1, "pickaxe", Some("minecraft:end_stone")),
            "deepslate" => (3.0, true, 1, "pickaxe", Some("minecraft:cobbled_deepslate")),
            "cobbled_deepslate" => (3.5, true, 1, "pickaxe", Some("minecraft:cobbled_deepslate")),
            "water" | "lava" | "nether_portal" | "end_portal" => return None,
            s if s.ends_with("_log") || s.ends_with("_wood") || s.ends_with("_planks") => (2.0, false, 0, "axe", None),
            s if s.ends_with("_leaves") => (0.2, false, 0, "hoe", None),
            s if s.ends_with("_terracotta") || s.ends_with("_bricks") || s.ends_with("_stone") => (2.0, true, 1, "pickaxe", None),
            "diorite" | "granite" | "andesite" | "tuff" => (1.5, true, 1, "pickaxe", None),
            "short_grass" | "fern" | "dead_bush" | "torch" | "fire" => (0.0, false, 0, "none", None),
            _ => return None,
        };
        Some(Self {
            hardness,
            requires_tool,
            min_tier,
            tool,
            drop,
        })
    }
}
fn tool_matches(held: Option<&str>, kind: &str) -> bool {
    kind != "none" && held.is_some_and(|id| id.rsplit(':').next().unwrap_or(id).ends_with(kind))
}
fn tool_speed(held: Option<&str>) -> f32 {
    match held
        .unwrap_or("")
        .rsplit(':')
        .next()
        .unwrap_or("")
        .split('_')
        .next()
    {
        Some("wooden") => 2.0,
        Some("golden") => 12.0,
        Some("stone") => 4.0,
        Some("iron") => 6.0,
        Some("diamond") => 8.0,
        Some("netherite") => 9.0,
        _ => 1.0,
    }
}
fn tool_level(held: Option<&str>) -> u8 {
    match held
        .unwrap_or("")
        .rsplit(':')
        .next()
        .unwrap_or("")
        .split('_')
        .next()
    {
        Some("wooden" | "golden") => 1,
        Some("stone") => 2,
        Some("iron") => 3,
        Some("diamond") => 4,
        Some("netherite") => 5,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct TestWorld(BTreeMap<Pos, Block>);
    impl World for TestWorld {
        fn block(&self, pos: Pos) -> Option<Block> {
            self.0.get(&pos).cloned()
        }
        fn set_block(&mut self, pos: Pos, block: Option<Block>) {
            if let Some(block) = block {
                self.0.insert(pos, block);
            } else {
                self.0.remove(&pos);
            }
        }
    }

    #[test]
    fn bare_hand_dirt_takes_fifteen_ticks_and_drops_dirt() {
        let mut world = TestWorld::default();
        world.set_block((0, 2, -2), Some(Block::new("minecraft:dirt")));
        let mut player = Player::new(DVec3::new(0.5, 1.0, 0.5));
        player.on_ground = true;
        let mut mining = Mining::default();
        for _ in 0..14 {
            assert!(mining.tick(&mut world, &player, None, true).is_none());
        }
        let result = mining.tick(&mut world, &player, None, true).unwrap();
        assert_eq!(result.drops, vec![ItemStack::new("minecraft:dirt", 1)]);
        assert_eq!(result.pos, (0, 2, -2));
        assert!(world.block(result.pos).is_none());
    }

    #[test]
    fn switching_items_restarts_held_mining_from_zero() {
        let mut world = TestWorld::default();
        world.set_block((0, 2, -2), Some(Block::new("minecraft:dirt")));
        let mut player = Player::new(DVec3::new(0.5, 1.0, 0.5));
        player.on_ground = true;
        let mut mining = Mining::default();
        let stone = ItemStack::new("minecraft:stone", 1);
        let planks = ItemStack::new("minecraft:oak_planks", 1);
        for _ in 0..7 {
            assert!(mining
                .tick(&mut world, &player, Some(&stone), true)
                .is_none());
        }
        assert!(mining.progress() > 0.0);
        assert!(mining.reset_if_item_changed(Some(&planks)));
        assert_eq!(mining.progress(), 0.0);
        for _ in 0..14 {
            assert!(mining
                .tick(&mut world, &player, Some(&planks), true)
                .is_none());
        }
        assert!(mining
            .tick(&mut world, &player, Some(&planks), true)
            .is_some());
    }

    #[test]
    fn stack_count_does_not_reset_mining_but_components_do() {
        let mut world = TestWorld::default();
        world.set_block((0, 2, -2), Some(Block::new("minecraft:dirt")));
        let mut player = Player::new(DVec3::new(0.5, 1.0, 0.5));
        player.on_ground = true;
        let mut mining = Mining::default();
        let original = ItemStack::new("minecraft:stone", 1);
        let more = ItemStack::new("minecraft:stone", 64);
        mining.tick(&mut world, &player, Some(&original), true);
        let progress = mining.progress();
        assert!(!mining.reset_if_item_changed(Some(&more)));
        assert_eq!(mining.progress(), progress);
        let mut altered = more;
        altered.components = Some(json!({"minecraft:custom_name":"Other"}));
        assert!(mining.reset_if_item_changed(Some(&altered)));
        assert_eq!(mining.progress(), 0.0);
    }

    #[test]
    fn bare_hand_stone_breaks_without_drop() {
        let mut world = TestWorld::default();
        world.set_block((0, 2, -2), Some(Block::new("minecraft:stone")));
        let mut player = Player::new(DVec3::new(0.5, 1.0, 0.5));
        player.on_ground = true;
        let mut mining = Mining::default();
        let mut result = None;
        for _ in 0..151 {
            result = mining.tick(&mut world, &player, None, true);
            if result.is_some() {
                break;
            }
        }
        assert!(result.is_some());
        assert!(result.unwrap().drops.is_empty());
    }

    fn sample_catalog() -> MiningCatalog {
        let value = json!({
            "schema_version": 1,
            "minecraft_version": "26.3",
            "blocks": {
                "minecraft:deepslate": {
                    "hardness": {"decimal": "3.0", "bits": "40400000"},
                    "requires_tool": true,
                    "tools": {
                        "minecraft:wooden_pickaxe": {
                            "speed": {"decimal": "2.0", "bits": "40000000"},
                            "correct": true
                        }
                    }
                },
                "minecraft:dirt": {
                    "hardness": {"decimal": "0.5", "bits": "3f000000"},
                    "requires_tool": false,
                    "tools": {}
                }
            }
        });
        MiningCatalog::from_slice(value.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn catalog_breaks_previously_unknown_block_with_measured_tool_speed() {
        let mut world = TestWorld::default();
        world.set_block((0, 2, -2), Some(Block::new("minecraft:deepslate")));
        let mut player = Player::new(DVec3::new(0.5, 1.0, 0.5));
        player.on_ground = true;
        let mut mining = Mining::with_catalog(sample_catalog());
        for _ in 0..44 {
            assert!(mining
                .tick(
                    &mut world,
                    &player,
                    Some(&ItemStack::new("minecraft:wooden_pickaxe", 1)),
                    true
                )
                .is_none());
        }
        let result = mining
            .tick(
                &mut world,
                &player,
                Some(&ItemStack::new("minecraft:wooden_pickaxe", 1)),
                true,
            )
            .unwrap();
        assert_eq!(result.pos, (0, 2, -2));
        assert!(result.drops.is_empty()); // This fixture has no loot book.
    }

    #[test]
    fn mining_water_penalty_uses_eye_submersion() {
        let mut player = Player::new(DVec3::new(0.5, 1.0, 0.5));
        player.on_ground = true;
        player.in_water = true;
        let mut waist_world = TestWorld::default();
        waist_world.set_block((0, 1, 0), Some(Block::new("minecraft:water")));
        waist_world.set_block((0, 2, -2), Some(Block::new("minecraft:dirt")));
        let mut mining = Mining::with_catalog(sample_catalog());
        for _ in 0..14 {
            assert!(mining.tick(&mut waist_world, &player, None, true).is_none());
        }
        assert!(mining.tick(&mut waist_world, &player, None, true).is_some());

        let mut eye_world = TestWorld::default();
        eye_world.set_block((0, 2, 0), Some(Block::new("minecraft:water")));
        eye_world.set_block((0, 2, -2), Some(Block::new("minecraft:dirt")));
        let mut mining = Mining::with_catalog(sample_catalog());
        for _ in 0..74 {
            assert!(mining.tick(&mut eye_world, &player, None, true).is_none());
        }
        assert!(mining.tick(&mut eye_world, &player, None, true).is_some());
    }

    #[test]
    fn catalog_rejects_hardness_decimal_bit_mismatch() {
        let bytes = json!({
            "schema_version": 1,
            "minecraft_version": "26.3",
            "blocks": {"minecraft:dirt": {
                "hardness": {"decimal": "0.5", "bits": "3f800000"},
                "requires_tool": false,
                "tools": {}
            }}
        })
        .to_string();
        assert!(MiningCatalog::from_slice(bytes.as_bytes()).is_err());
    }

    #[test]
    fn pinned_catalog_deepslate_and_shears_when_available() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/mining-catalog/26.3.json");
        if !path.is_file() {
            return;
        }
        let catalog = MiningCatalog::from_path(&path).unwrap();
        assert_eq!(catalog.block_count(), 1286);
        let leaves = &catalog.blocks["minecraft:oak_leaves"];
        assert_eq!(
            leaves.tools["minecraft:shears"].speed.to_bits(),
            15.0_f32.to_bits()
        );

        let mut world = TestWorld::default();
        world.set_block((0, 2, -2), Some(Block::new("minecraft:deepslate")));
        let mut player = Player::new(DVec3::new(0.5, 1.0, 0.5));
        player.on_ground = true;
        let mut mining = Mining::with_catalog(catalog);
        let jar = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../harness/.gradle/loom-cache/minecraftMaven/net/minecraft/minecraft-common-1fad6b3808/26.3/minecraft-common-1fad6b3808-26.3.jar",
        );
        if jar.is_file() {
            mining.set_loot(LootBook::from_jar(&jar).unwrap());
        }
        for _ in 0..44 {
            assert!(mining
                .tick(
                    &mut world,
                    &player,
                    Some(&ItemStack::new("minecraft:wooden_pickaxe", 1)),
                    true
                )
                .is_none());
        }
        let broken = mining
            .tick(
                &mut world,
                &player,
                Some(&ItemStack::new("minecraft:wooden_pickaxe", 1)),
                true,
            )
            .unwrap();
        if jar.is_file() {
            assert_eq!(
                broken.drops,
                vec![ItemStack::new("minecraft:cobbled_deepslate", 1)]
            );
            world.set_block((0, 2, -2), Some(Block::new("minecraft:oak_leaves")));
            let sheared = mining
                .tick(
                    &mut world,
                    &player,
                    Some(&ItemStack::new("minecraft:shears", 1)),
                    true,
                )
                .unwrap();
            assert_eq!(
                sheared.drops,
                vec![ItemStack::new("minecraft:oak_leaves", 1)]
            );
        }
    }

    #[test]
    fn dirt_attack_fixture_breaks_on_fifteenth_local_tick() {
        let mut world = TestWorld::default();
        world.set_block((0, 2, 2), Some(Block::new("minecraft:dirt")));
        let mut player = Player::new(DVec3::new(0.5, 1.0, 4.5));
        player.on_ground = true;
        let mut mining = Mining::with_catalog(sample_catalog());
        for client_tick in 10..=24 {
            let result = mining.tick(&mut world, &player, None, true);
            assert_eq!(result.is_some(), client_tick == 24);
        }
        assert!(world.block((0, 2, 2)).is_none());
    }
}
