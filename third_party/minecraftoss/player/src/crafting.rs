//! Shaped and shapeless crafting against recipes and item tags loaded from the
//! player's external, pinned Minecraft data JAR. No vanilla data is bundled.
use crate::{inventory::ItemStack, item_catalog::ItemCatalog};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
    sync::Arc,
};
use zip::ZipArchive;

#[derive(Clone, Debug)]
struct Ingredient(Vec<String>);
impl Ingredient {
    fn parse(value: &Value) -> Option<Self> {
        match value {
            Value::String(id) => Some(Self(vec![id.clone()])),
            Value::Array(options) => {
                let ids = options
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                (!ids.is_empty()).then_some(Self(ids))
            }
            _ => None,
        }
    }
}

#[derive(Debug)]
enum Kind {
    Shaped(Vec<Vec<Option<Ingredient>>>),
    Shapeless(Vec<Ingredient>),
}
#[derive(Debug)]
struct Recipe {
    id: String,
    category: String,
    group: Option<String>,
    kind: Kind,
    result: ItemStack,
}

#[derive(Clone, Copy, Debug)]
pub struct CraftingRecipeRef<'a> {
    pub id: &'a str,
    pub category: &'a str,
    pub group: Option<&'a str>,
    pub result: &'a ItemStack,
    pub width: usize,
    pub height: usize,
    pub ingredients: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SmeltingRecipe {
    pub result: ItemStack,
    pub cooking_ticks: u32,
    pub experience: f32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CookingKind {
    #[default]
    Furnace,
    BlastFurnace,
    Smoker,
}
impl CookingKind {
    pub fn block_id(self) -> &'static str {
        match self {
            Self::Furnace => "furnace",
            Self::BlastFurnace => "blast_furnace",
            Self::Smoker => "smoker",
        }
    }
    pub fn from_block_id(id: &str) -> Option<Self> {
        match id {
            "furnace" => Some(Self::Furnace),
            "blast_furnace" => Some(Self::BlastFurnace),
            "smoker" => Some(Self::Smoker),
            _ => None,
        }
    }
    /// The default CookingFuel speed provider is conditional on the furnace
    /// block: the smoker and blast furnace match the FAST_FURNACE predicate.
    pub fn default_speed_multiplier(self) -> f32 {
        match self {
            Self::Furnace => 1.0,
            Self::BlastFurnace | Self::Smoker => 2.0,
        }
    }
    pub fn default_burn_divisor(self) -> u32 {
        match self {
            Self::Furnace => 1,
            Self::BlastFurnace | Self::Smoker => 2,
        }
    }
}

#[derive(Debug)]
struct SmeltingEntry {
    id: String,
    kind: CookingKind,
    category: String,
    group: Option<String>,
    ingredient: Ingredient,
    recipe: SmeltingRecipe,
}

#[derive(Clone, Debug)]
enum AutoUnlockCriterion {
    ChangedItem(Ingredient),
    OccupiedSlots(usize),
}

#[derive(Debug)]
struct AutoUnlockRule {
    recipe_id: String,
    criterion: AutoUnlockCriterion,
}

#[derive(Clone, Copy, Debug)]
pub struct CookingRecipeRef<'a> {
    pub id: &'a str,
    pub category: &'a str,
    pub group: Option<&'a str>,
    pub result: &'a ItemStack,
}

#[derive(Default, Debug)]
pub struct RecipeBook {
    recipes: Vec<Recipe>,
    smelting: Vec<SmeltingEntry>,
    tags: HashMap<String, Vec<String>>,
    auto_unlocks: Vec<AutoUnlockRule>,
    display_indices: HashMap<String, Vec<u32>>,
    item_catalog: Option<Arc<ItemCatalog>>,
}
impl RecipeBook {
    pub fn from_jar(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("open data JAR {}", path.display()))?;
        let mut archive = ZipArchive::new(file)?;
        let mut tags = HashMap::new();
        let mut raw_recipes = Vec::new();
        let mut raw_advancements = Vec::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let name = entry.name().to_owned();
            if !name.ends_with(".json") {
                continue;
            }
            let is_recipe = name.starts_with("data/minecraft/recipe/");
            let is_tag = name.starts_with("data/minecraft/tags/item/");
            let is_recipe_advancement = name.starts_with("data/minecraft/advancement/recipes/");
            if !is_recipe && !is_tag && !is_recipe_advancement {
                continue;
            }
            let mut content = String::new();
            entry.read_to_string(&mut content)?;
            let value: Value =
                serde_json::from_str(&content).with_context(|| format!("parse {name}"))?;
            if is_recipe {
                raw_recipes.push((name, value));
            } else if is_recipe_advancement {
                raw_advancements.push(value);
            } else if let Some(values) = value.get("values").and_then(Value::as_array) {
                let id = name
                    .trim_start_matches("data/minecraft/tags/item/")
                    .trim_end_matches(".json");
                let entries = values
                    .iter()
                    .filter_map(|v| v.as_str().or_else(|| v.get("id").and_then(Value::as_str)))
                    .map(str::to_owned)
                    .collect();
                tags.insert(format!("minecraft:{id}"), entries);
            }
        }
        raw_recipes.sort_by(|a, b| a.0.cmp(&b.0));
        let mut recipes = Vec::new();
        let mut smelting = Vec::new();
        for (name, value) in raw_recipes {
            let cooking_kind = match value.get("type").and_then(Value::as_str) {
                Some("minecraft:smelting") => Some(CookingKind::Furnace),
                Some("minecraft:blasting") => Some(CookingKind::BlastFurnace),
                Some("minecraft:smoking") => Some(CookingKind::Smoker),
                _ => None,
            };
            if let Some(kind) = cooking_kind {
                if let Some(ingredient) = value.get("ingredient").and_then(Ingredient::parse) {
                    if let Some(result) = parse_result(&value) {
                        smelting.push(SmeltingEntry {
                            id: format!(
                                "minecraft:{}",
                                name.trim_start_matches("data/minecraft/recipe/")
                                    .trim_end_matches(".json")
                            ),
                            kind,
                            category: value
                                .get("category")
                                .and_then(Value::as_str)
                                .unwrap_or("misc")
                                .to_owned(),
                            group: value
                                .get("group")
                                .and_then(Value::as_str)
                                .filter(|group| !group.is_empty())
                                .map(str::to_owned),
                            ingredient,
                            recipe: SmeltingRecipe {
                                result,
                                cooking_ticks: value
                                    .get("cookingtime")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(200)
                                    as u32,
                                experience: value
                                    .get("experience")
                                    .and_then(Value::as_f64)
                                    .unwrap_or(0.0)
                                    as f32,
                            },
                        });
                    }
                }
            } else if let Some(mut recipe) = parse_recipe(&value) {
                recipe.id = format!(
                    "minecraft:{}",
                    name.trim_start_matches("data/minecraft/recipe/")
                        .trim_end_matches(".json")
                );
                recipe.category = value
                    .get("category")
                    .and_then(Value::as_str)
                    .unwrap_or("misc")
                    .to_owned();
                recipe.group = value
                    .get("group")
                    .and_then(Value::as_str)
                    .filter(|group| !group.is_empty())
                    .map(str::to_owned);
                recipes.push(recipe);
            }
        }
        let known_ids = recipes
            .iter()
            .map(|recipe| recipe.id.as_str())
            .chain(smelting.iter().map(|entry| entry.id.as_str()))
            .collect::<HashSet<_>>();
        let auto_unlocks = raw_advancements
            .iter()
            .flat_map(|advancement| auto_unlock_rules(advancement, &known_ids))
            .collect();
        Ok(Self {
            recipes,
            smelting,
            tags,
            auto_unlocks,
            display_indices: HashMap::new(),
            item_catalog: None,
        })
    }

    /// Load display IDs observed from the pinned server recipe manager. The
    /// client HashMap iterates known displays by these integer-key buckets.
    pub fn with_display_catalog(mut self, path: &Path) -> Result<Self> {
        let value: Value = serde_json::from_reader(
            File::open(path).with_context(|| format!("open display catalog {}", path.display()))?,
        )?;
        anyhow::ensure!(
            value.get("schema_version").and_then(Value::as_u64) == Some(1)
                && value.get("minecraft_version").and_then(Value::as_str) == Some("26.3"),
            "unsupported recipe display catalog {}",
            path.display()
        );
        let recipes = value
            .get("recipes")
            .and_then(Value::as_object)
            .context("recipe display catalog missing recipes")?;
        let mut seen = HashSet::new();
        for (id, displays) in recipes {
            let indices = displays
                .as_array()
                .context("recipe display list must be an array")?
                .iter()
                .map(|display| {
                    display
                        .get("index")
                        .and_then(Value::as_u64)
                        .and_then(|index| u32::try_from(index).ok())
                        .context("invalid recipe display index")
                })
                .collect::<Result<Vec<_>>>()?;
            for index in &indices {
                anyhow::ensure!(
                    seen.insert(*index),
                    "duplicate recipe display index {index}"
                );
            }
            self.display_indices.insert(id.clone(), indices);
        }
        Ok(self)
    }

    pub fn display_indices(&self, id: &str) -> Option<&[u32]> {
        self.display_indices.get(id).map(Vec::as_slice)
    }

    /// Pinned 26.3 recipe advancements mostly use one inventory_changed
    /// predicate. InventoryChangeTrigger tests a single predicate against the
    /// changed stack, and a slots-only predicate against current occupancy.
    pub fn auto_unlocks_for_change<'a>(
        &'a self,
        changed_item: &'a ItemStack,
        occupied_slots: usize,
    ) -> impl Iterator<Item = &'a str> + 'a {
        self.auto_unlocks.iter().filter_map(move |rule| {
            let matches = match &rule.criterion {
                AutoUnlockCriterion::ChangedItem(ingredient) => {
                    self.matches_ingredient(ingredient, &changed_item.id)
                }
                AutoUnlockCriterion::OccupiedSlots(minimum) => occupied_slots >= *minimum,
            };
            matches.then_some(rule.recipe_id.as_str())
        })
    }

    pub fn with_item_catalog(mut self, catalog: Arc<ItemCatalog>) -> Self {
        for recipe in &mut self.recipes {
            recipe.result.max = catalog.max_stack(&recipe.result.id);
        }
        for entry in &mut self.smelting {
            entry.recipe.result.max = catalog.max_stack(&entry.recipe.result.id);
        }
        self.item_catalog = Some(catalog);
        self
    }

    pub fn item_catalog(&self) -> Option<&ItemCatalog> {
        self.item_catalog.as_deref()
    }

    pub fn max_stack(&self, id: &str) -> u8 {
        self.item_catalog()
            .map_or(64, |catalog| catalog.max_stack(id))
    }

    /// A stack's explicit component patch takes precedence over its default
    /// item component. A null patch removes the default component.
    pub fn equipment_slot<'a>(&'a self, stack: &'a ItemStack) -> Option<&'a str> {
        if let Some(value) = stack
            .components
            .as_ref()
            .and_then(|components| components.get("minecraft:equippable"))
        {
            return value.get("slot").and_then(Value::as_str);
        }
        self.item_catalog()
            .and_then(|catalog| catalog.get(&stack.id))
            .and_then(|item| item.equippable_slot.as_deref())
    }

    pub fn equipment_asset<'a>(&'a self, stack: &'a ItemStack) -> Option<&'a str> {
        if let Some(value) = stack
            .components
            .as_ref()
            .and_then(|components| components.get("minecraft:equippable"))
        {
            return value.get("asset_id").and_then(Value::as_str);
        }
        self.item_catalog()
            .and_then(|catalog| catalog.get(&stack.id))
            .and_then(|item| item.equipment_asset.as_deref())
    }

    pub fn armor_points(&self, stack: &ItemStack, slot: &str) -> f64 {
        if stack
            .components
            .as_ref()
            .is_some_and(|components| components.get("minecraft:attribute_modifiers").is_some())
        {
            return 0.0;
        }
        self.item_catalog()
            .and_then(|catalog| catalog.get(&stack.id))
            .filter(|item| item.equippable_slot.as_deref() == Some(slot))
            .map_or(0.0, |item| item.armor_points)
    }

    /// A stack's attribute modifiers: its component patch's when it has
    /// one, else the item's defaults.
    pub fn item_modifiers(&self, stack: &ItemStack) -> Vec<crate::item_catalog::AttributeModifier> {
        use crate::item_catalog::{AttributeModifier, ModifierOperation};
        if let Some(patch) = stack.components.as_ref().and_then(|components| components.get("minecraft:attribute_modifiers")) {
            // The list form, or an object holding `modifiers`.
            let list = patch.as_array().or_else(|| patch.get("modifiers").and_then(Value::as_array));
            return list
                .into_iter()
                .flatten()
                .filter_map(|m| {
                    Some(AttributeModifier {
                        attribute: m.get("type").or_else(|| m.get("attribute")).and_then(Value::as_str)?.to_owned(),
                        id: m.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(),
                        amount: m.get("amount").and_then(Value::as_f64)?,
                        operation: ModifierOperation::parse(m.get("operation").and_then(Value::as_str)?)?,
                        slot: m.get("slot").and_then(Value::as_str).unwrap_or("any").to_owned(),
                    })
                })
                .collect();
        }
        self.item_catalog().and_then(|catalog| catalog.get(&stack.id)).map(|item| item.attribute_modifiers.clone()).unwrap_or_default()
    }

    /// The player's `ATTACK_DAMAGE` and `ATTACK_SPEED` (1 and 4 in
    /// `Player.createAttributes`) with the main hand's modifiers.
    pub fn attack_attributes(&self, held: Option<&ItemStack>) -> (f64, f64) {
        use crate::item_catalog::attribute_value;
        let modifiers = held.map(|stack| self.item_modifiers(stack)).unwrap_or_default();
        let of = |attribute: &'static str| modifiers.iter().filter(move |m| m.in_main_hand() && m.attribute == attribute);
        (attribute_value(1.0, of("minecraft:attack_damage"), (0.0, 2048.0)), attribute_value(4.0, of("minecraft:attack_speed"), (0.0, 1024.0)))
    }

    /// The `weapon` component: durability lost per attack and how long a
    /// hit disables a shield.
    pub fn weapon(&self, stack: &ItemStack) -> Option<(u32, f32)> {
        if let Some(patch) = stack.components.as_ref().and_then(|components| components.get("minecraft:weapon")) {
            let per_attack = patch.get("item_damage_per_attack").and_then(Value::as_u64).unwrap_or(1) as u32;
            let disable = patch.get("disable_blocking_for_seconds").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            return Some((per_attack, disable));
        }
        self.item_catalog().and_then(|catalog| catalog.get(&stack.id)).and_then(|item| item.weapon)
    }

    /// The armor toughness and knockback resistance a worn piece adds, by
    /// its material (`ArmorMaterials`: diamond 2 toughness; netherite 3 and
    /// 0.1 knockback resistance).
    pub fn armor_toughness(&self, stack: &ItemStack, slot: &str) -> (f64, f64) {
        if stack
            .components
            .as_ref()
            .is_some_and(|components| components.get("minecraft:attribute_modifiers").is_some())
        {
            return (0.0, 0.0);
        }
        let asset = self
            .item_catalog()
            .and_then(|catalog| catalog.get(&stack.id))
            .filter(|item| item.equippable_slot.as_deref() == Some(slot))
            .and_then(|item| item.equipment_asset.as_deref());
        match asset {
            Some("minecraft:diamond") => (2.0, 0.0),
            Some("minecraft:netherite") => (3.0, 0.1),
            _ => (0.0, 0.0),
        }
    }

    /// Returns the damage and maximum damage for a damageable stack, honoring
    /// explicit component patches and the UNBREAKABLE component.
    pub fn durability(&self, stack: &ItemStack) -> Option<(u32, u32)> {
        let defaults = self
            .item_catalog()
            .and_then(|catalog| catalog.get(&stack.id));
        let patch = stack.components.as_ref();
        if patch
            .and_then(|components| components.get("minecraft:unbreakable"))
            .is_some_and(|value| !value.is_null())
        {
            return None;
        }
        let max_damage = match patch.and_then(|components| components.get("minecraft:max_damage")) {
            Some(value) => u32::try_from(value.as_u64()?).ok()?,
            None => defaults.map_or(0, |item| item.max_damage),
        };
        if max_damage == 0 {
            return None;
        }
        let damage = match patch.and_then(|components| components.get("minecraft:damage")) {
            Some(value) => u32::try_from(value.as_u64()?).ok()?.min(max_damage),
            None => 0,
        };
        Some((damage, max_damage))
    }

    /// Default or stack-patched TOOL damage for one mined block.
    pub fn mining_tool_wear(&self, stack: &ItemStack) -> Option<(u32, u32)> {
        let (_, max_damage) = self.durability(stack)?;
        let defaults = self
            .item_catalog()
            .and_then(|catalog| catalog.get(&stack.id));
        let damage_per_block = match stack
            .components
            .as_ref()
            .and_then(|components| components.get("minecraft:tool"))
        {
            Some(value) => u32::try_from(value.get("damage_per_block")?.as_u64()?).ok()?,
            None => defaults.map_or(0, |item| item.damage_per_block),
        };
        if damage_per_block == 0 {
            return None;
        }
        Some((max_damage, damage_per_block))
    }

    pub fn stack(&self, id: &str, count: u8) -> ItemStack {
        let mut stack = ItemStack::new(id, count);
        stack.max = self.max_stack(id);
        stack
    }

    pub fn fuel_ticks(&self, id: &str) -> u32 {
        if let Some(catalog) = self.item_catalog() {
            return catalog.get(id).map_or(0, |item| item.burn_ticks);
        }
        crate::furnace::fuel_ticks(id)
    }

    pub fn fuel_speed(&self, id: &str) -> f32 {
        self.item_catalog()
            .and_then(|catalog| catalog.get(id))
            .map_or(1.0, |item| item.speed_multiplier)
    }

    pub fn is_fuel(&self, id: &str) -> bool {
        if let Some(catalog) = self.item_catalog() {
            return catalog.get(id).is_some_and(|item| item.fuel_component);
        }
        crate::furnace::fuel_ticks(id) > 0
    }

    pub fn crafting_remainder(&self, id: &str) -> Option<ItemStack> {
        if let Some(catalog) = self.item_catalog() {
            let (id, count) = catalog.get(id)?.remainder.as_ref()?;
            return Some(self.stack(id, *count));
        }
        (id == "minecraft:lava_bucket").then(|| ItemStack::new("minecraft:bucket", 1))
    }

    pub fn count(&self) -> usize {
        self.recipes.len()
    }

    pub fn crafting_recipes(&self) -> impl Iterator<Item = CraftingRecipeRef<'_>> {
        self.recipes.iter().map(|recipe| {
            let (width, height, ingredients) = match &recipe.kind {
                Kind::Shaped(rows) => (
                    rows[0].len(),
                    rows.len(),
                    rows.iter().flatten().filter(|cell| cell.is_some()).count(),
                ),
                Kind::Shapeless(items) => (0, 0, items.len()),
            };
            CraftingRecipeRef {
                id: &recipe.id,
                category: &recipe.category,
                group: recipe.group.as_deref(),
                result: &recipe.result,
                width,
                height,
                ingredients,
            }
        })
    }

    pub fn crafting_grid_candidates(
        &self,
        recipe_id: &str,
        grid_width: usize,
        grid_height: usize,
    ) -> Option<Vec<Option<Vec<String>>>> {
        let recipe = self.recipes.iter().find(|recipe| recipe.id == recipe_id)?;
        let mut grid = vec![None; grid_width * grid_height];
        match &recipe.kind {
            Kind::Shaped(rows) => {
                let (width, height) = (rows[0].len(), rows.len());
                if width > grid_width || height > grid_height {
                    return None;
                }
                // 26.3 PlaceRecipeHelper centers a dimension only when the
                // recipe is smaller than half the grid dimension.
                let offset_x = if (width as f32) < grid_width as f32 / 2.0 {
                    (grid_width - width) / 2
                } else {
                    0
                };
                let offset_y = if (height as f32) < grid_height as f32 / 2.0 {
                    (grid_height - height) / 2
                } else {
                    0
                };
                for (y, row) in rows.iter().enumerate() {
                    for (x, ingredient) in row.iter().enumerate() {
                        if let Some(ingredient) = ingredient {
                            grid[(y + offset_y) * grid_width + x + offset_x] =
                                Some(self.ingredient_items(ingredient));
                        }
                    }
                }
            }
            Kind::Shapeless(ingredients) => {
                if ingredients.len() > grid.len() {
                    return None;
                }
                for (slot, ingredient) in ingredients.iter().enumerate() {
                    grid[slot] = Some(self.ingredient_items(ingredient));
                }
            }
        }
        Some(grid)
    }

    fn ingredient_items(&self, ingredient: &Ingredient) -> Vec<String> {
        fn expand(
            book: &RecipeBook,
            candidate: &str,
            visited: &mut HashSet<String>,
            result: &mut Vec<String>,
        ) {
            if let Some(tag) = candidate.strip_prefix('#') {
                if visited.insert(tag.to_owned()) {
                    if let Some(entries) = book.tags.get(tag) {
                        for entry in entries {
                            expand(book, entry, visited, result);
                        }
                    }
                    visited.remove(tag);
                }
            } else if !result.iter().any(|id| id == candidate) {
                result.push(candidate.to_owned());
            }
        }
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        for candidate in &ingredient.0 {
            expand(self, candidate, &mut visited, &mut result);
        }
        result
    }

    pub fn smelting_count(&self) -> usize {
        self.cooking_count(CookingKind::Furnace)
    }

    pub fn cooking_count(&self, kind: CookingKind) -> usize {
        self.smelting
            .iter()
            .filter(|entry| entry.kind == kind)
            .count()
    }

    pub fn cooking_recipes(&self, kind: CookingKind) -> impl Iterator<Item = CookingRecipeRef<'_>> {
        self.smelting
            .iter()
            .filter(move |entry| entry.kind == kind)
            .map(|entry| CookingRecipeRef {
                id: &entry.id,
                category: &entry.category,
                group: entry.group.as_deref(),
                result: &entry.recipe.result,
            })
    }

    pub fn cooking_recipe_accepts(
        &self,
        kind: CookingKind,
        recipe_id: &str,
        item_id: &str,
    ) -> bool {
        self.smelting
            .iter()
            .find(|entry| entry.kind == kind && entry.id == recipe_id)
            .is_some_and(|entry| self.matches_ingredient(&entry.ingredient, item_id))
    }

    /// First display item for the cooking input. The ingredient choices in
    /// the external recipe and item tags retain their pack order.
    pub fn cooking_display_ingredient(
        &self,
        kind: CookingKind,
        recipe_id: &str,
    ) -> Option<ItemStack> {
        let ingredient = &self
            .smelting
            .iter()
            .find(|entry| entry.kind == kind && entry.id == recipe_id)?
            .ingredient;
        ingredient.0.iter().find_map(|candidate| {
            self.first_ingredient_item(candidate, &mut HashSet::new())
                .map(|id| self.stack(id, 1))
        })
    }

    fn first_ingredient_item<'a>(
        &'a self,
        candidate: &'a str,
        visited: &mut HashSet<String>,
    ) -> Option<&'a str> {
        let Some(tag) = candidate.strip_prefix('#') else {
            return Some(candidate);
        };
        if !visited.insert(tag.to_owned()) {
            return None;
        }
        let item = self
            .tags
            .get(tag)?
            .iter()
            .find_map(|entry| self.first_ingredient_item(entry, visited));
        visited.remove(tag);
        item
    }

    pub fn smelting_for(&self, input: &ItemStack) -> Option<&SmeltingRecipe> {
        self.cooking_for(CookingKind::Furnace, input)
    }

    pub fn cooking_for(&self, kind: CookingKind, input: &ItemStack) -> Option<&SmeltingRecipe> {
        self.smelting
            .iter()
            .filter(|entry| entry.kind == kind)
            .find(|entry| self.matches_ingredient(&entry.ingredient, &input.id))
            .map(|entry| &entry.recipe)
    }

    pub fn matching(
        &self,
        grid: &[Option<ItemStack>],
        width: usize,
        height: usize,
    ) -> Option<ItemStack> {
        if grid.len() != width * height {
            return None;
        }
        self.recipes.iter().find_map(|recipe| {
            let matched = self.recipe_matches(recipe, grid, width, height);
            matched.then(|| recipe.result.clone())
        })
    }

    pub fn matches_crafting_id(
        &self,
        recipe_id: &str,
        grid: &[Option<ItemStack>],
        width: usize,
        height: usize,
    ) -> bool {
        self.recipes
            .iter()
            .find(|recipe| recipe.id == recipe_id)
            .is_some_and(|recipe| self.recipe_matches(recipe, grid, width, height))
    }

    fn recipe_matches(
        &self,
        recipe: &Recipe,
        grid: &[Option<ItemStack>],
        width: usize,
        height: usize,
    ) -> bool {
        match &recipe.kind {
            Kind::Shaped(pattern) => self.matches_shaped(pattern, grid, width, height),
            Kind::Shapeless(ingredients) => self.matches_shapeless(ingredients, grid),
        }
    }

    fn matches_ingredient(&self, ingredient: &Ingredient, id: &str) -> bool {
        ingredient
            .0
            .iter()
            .any(|candidate| self.matches_id(candidate, id, &mut HashSet::new()))
    }

    fn matches_id(&self, candidate: &str, id: &str, visited: &mut HashSet<String>) -> bool {
        if let Some(tag) = candidate.strip_prefix('#') {
            if !visited.insert(tag.to_owned()) {
                return false;
            }
            let matched = self.tags.get(tag).is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| self.matches_id(entry, id, visited))
            });
            visited.remove(tag);
            matched
        } else {
            candidate == id
        }
    }

    /// Resolve an item tag from the pinned gameplay data JAR, including nested tags.
    pub fn item_in_tag(&self, tag: &str, id: &str) -> bool {
        self.matches_id(&format!("#{tag}"), id, &mut HashSet::new())
    }

    fn matches_shaped(
        &self,
        pattern: &[Vec<Option<Ingredient>>],
        grid: &[Option<ItemStack>],
        width: usize,
        height: usize,
    ) -> bool {
        let ph = pattern.len();
        let pw = pattern.first().map_or(0, Vec::len);
        if pw == 0 || ph == 0 || pw > width || ph > height {
            return false;
        }
        for oy in 0..=height - ph {
            for ox in 0..=width - pw {
                for mirror in [false, true] {
                    let mut matched = true;
                    for y in 0..height {
                        for x in 0..width {
                            let expected = if x >= ox && x < ox + pw && y >= oy && y < oy + ph {
                                let px = if mirror { pw - 1 - (x - ox) } else { x - ox };
                                pattern[y - oy][px].as_ref()
                            } else {
                                None
                            };
                            let actual = grid[y * width + x].as_ref();
                            if !match (expected, actual) {
                                (None, None) => true,
                                (Some(ingredient), Some(stack)) => {
                                    self.matches_ingredient(ingredient, &stack.id)
                                }
                                _ => false,
                            } {
                                matched = false;
                                break;
                            }
                        }
                    }
                    if matched {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn matches_shapeless(&self, ingredients: &[Ingredient], grid: &[Option<ItemStack>]) -> bool {
        let items = grid.iter().filter_map(Option::as_ref).collect::<Vec<_>>();
        if items.len() != ingredients.len() {
            return false;
        }
        fn assign(
            book: &RecipeBook,
            ingredients: &[Ingredient],
            items: &[&ItemStack],
            index: usize,
            used: u16,
        ) -> bool {
            if index == ingredients.len() {
                return true;
            }
            items.iter().enumerate().any(|(slot, stack)| {
                used & (1 << slot) == 0
                    && book.matches_ingredient(&ingredients[index], &stack.id)
                    && assign(book, ingredients, items, index + 1, used | (1 << slot))
            })
        }
        assign(self, ingredients, &items, 0, 0)
    }
}

fn auto_unlock_rules(advancement: &Value, known_ids: &HashSet<&str>) -> Vec<AutoUnlockRule> {
    let Some(requirements) = advancement
        .get("requirements")
        .and_then(Value::as_array)
        .filter(|groups| groups.len() == 1)
        .and_then(|groups| groups[0].as_array())
    else {
        return Vec::new();
    };
    let Some(criteria) = advancement.get("criteria").and_then(Value::as_object) else {
        return Vec::new();
    };
    let rewards = advancement
        .get("rewards")
        .and_then(|rewards| rewards.get("recipes"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|id| known_ids.contains(id))
        .collect::<Vec<_>>();
    let mut rules = Vec::new();
    for requirement in requirements.iter().filter_map(Value::as_str) {
        let Some(condition) = criteria
            .get(requirement)
            .filter(|criterion| {
                criterion.get("trigger").and_then(Value::as_str)
                    == Some("minecraft:inventory_changed")
            })
            .and_then(|criterion| criterion.get("conditions"))
        else {
            continue;
        };
        let criterion = if let Some(item) = condition
            .get("items")
            .and_then(Value::as_array)
            .filter(|items| items.len() == 1)
            .and_then(|items| items[0].get("items"))
            .and_then(Ingredient::parse)
        {
            AutoUnlockCriterion::ChangedItem(item)
        } else if let Some(minimum) = condition
            .get("slots")
            .and_then(|slots| slots.get("occupied"))
            .and_then(|occupied| occupied.get("min"))
            .and_then(Value::as_u64)
            .and_then(|minimum| usize::try_from(minimum).ok())
        {
            AutoUnlockCriterion::OccupiedSlots(minimum)
        } else {
            continue;
        };
        for id in &rewards {
            rules.push(AutoUnlockRule {
                recipe_id: (*id).to_owned(),
                criterion: criterion.clone(),
            });
        }
    }
    rules
}

fn parse_recipe(value: &Value) -> Option<Recipe> {
    let result = parse_result(value)?;
    let kind = match value.get("type")?.as_str()? {
        "minecraft:crafting_shaped" => {
            let key = value.get("key")?.as_object()?;
            let pattern = value.get("pattern")?.as_array()?;
            let rows = pattern
                .iter()
                .map(|row| {
                    row.as_str()?
                        .chars()
                        .map(|symbol| {
                            if symbol == ' ' {
                                Some(None)
                            } else {
                                Some(Some(Ingredient::parse(key.get(&symbol.to_string())?)?))
                            }
                        })
                        .collect::<Option<Vec<_>>>()
                })
                .collect::<Option<Vec<_>>>()?;
            if rows.is_empty() || rows.iter().any(|row| row.len() != rows[0].len()) {
                return None;
            }
            Kind::Shaped(trim_pattern(rows)?)
        }
        "minecraft:crafting_shapeless" => Kind::Shapeless(
            value
                .get("ingredients")?
                .as_array()?
                .iter()
                .map(Ingredient::parse)
                .collect::<Option<Vec<_>>>()?,
        ),
        _ => return None,
    };
    Some(Recipe {
        id: String::new(),
        category: "misc".to_owned(),
        group: None,
        kind,
        result,
    })
}

fn parse_result(value: &Value) -> Option<ItemStack> {
    let result = value.get("result")?;
    let id = result.get("id")?.as_str()?;
    let count = u8::try_from(result.get("count").and_then(Value::as_u64).unwrap_or(1)).ok()?;
    Some(ItemStack::new(id, count))
}

fn trim_pattern(rows: Vec<Vec<Option<Ingredient>>>) -> Option<Vec<Vec<Option<Ingredient>>>> {
    let mut top = rows.len();
    let mut bottom = 0;
    let mut left = rows[0].len();
    let mut right = 0;
    for (y, row) in rows.iter().enumerate() {
        for (x, cell) in row.iter().enumerate() {
            if cell.is_some() {
                top = top.min(y);
                bottom = bottom.max(y + 1);
                left = left.min(x);
                right = right.max(x + 1);
            }
        }
    }
    (top < bottom && left < right).then(|| {
        rows[top..bottom]
            .iter()
            .map(|row| row[left..right].to_vec())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::Inventory;
    #[test]
    fn shaped_recipe_matches_offset_and_mirror() {
        let mut book = RecipeBook::default();
        book.recipes.push(
            parse_recipe(&serde_json::json!({
                "type":"minecraft:crafting_shaped",
                "key":{"A":"minecraft:stone","B":"minecraft:dirt"},
                "pattern":[" AB "],"result":{"id":"minecraft:bricks"}
            }))
            .unwrap(),
        );
        let grid = [
            None,
            None,
            Some(ItemStack::new("minecraft:dirt", 1)),
            Some(ItemStack::new("minecraft:stone", 1)),
        ];
        assert_eq!(book.matching(&grid, 2, 2).unwrap().id, "minecraft:bricks");
    }
    #[test]
    fn shapeless_recipe_expands_item_tag() {
        let mut book = RecipeBook::default();
        book.tags
            .insert("minecraft:logs".into(), vec!["minecraft:oak_log".into()]);
        book.recipes.push(
            parse_recipe(&serde_json::json!({
                "type":"minecraft:crafting_shapeless", "ingredients":["#minecraft:logs"],
                "result":{"id":"minecraft:oak_planks","count":4}
            }))
            .unwrap(),
        );
        let grid = [
            None,
            Some(ItemStack::new("minecraft:oak_log", 1)),
            None,
            None,
        ];
        assert_eq!(book.matching(&grid, 2, 2).unwrap().count, 4);
    }
    #[test]
    fn taking_result_consumes_inputs_and_preserves_item_counts() {
        let mut book = RecipeBook::default();
        book.tags.insert(
            "minecraft:oak_logs".into(),
            vec!["minecraft:oak_log".into()],
        );
        book.recipes.push(
            parse_recipe(&serde_json::json!({
                "type":"minecraft:crafting_shapeless", "ingredients":["#minecraft:oak_logs"],
                "result":{"id":"minecraft:oak_planks","count":4}
            }))
            .unwrap(),
        );
        let mut inv = Inventory::default().with_recipes(book);
        inv.crafting[0] = Some(ItemStack::new("minecraft:oak_log", 2));
        assert_eq!(inv.crafting_output().unwrap().count, 4);
        assert!(inv.take_crafting_output(false));
        assert_eq!(inv.count("minecraft:oak_log"), 1);
        assert_eq!(inv.count("minecraft:oak_planks"), 4);
        assert!(inv.take_crafting_output(false));
        assert_eq!(inv.count("minecraft:oak_log"), 0);
        assert_eq!(inv.count("minecraft:oak_planks"), 8);
        assert!(!inv.take_crafting_output(false));
    }
    #[test]
    fn three_by_three_furnace_consumes_eight_cobblestone() {
        let mut book = RecipeBook::default();
        book.recipes.push(
            parse_recipe(&serde_json::json!({
                "type":"minecraft:crafting_shaped",
                "key":{"#":"minecraft:cobblestone"},
                "pattern":["###","# #","###"],
                "result":{"id":"minecraft:furnace"}
            }))
            .unwrap(),
        );
        let mut inv = Inventory::default().with_recipes(book);
        for index in [0, 1, 2, 3, 5, 6, 7, 8] {
            inv.workbench[index] = Some(ItemStack::new("minecraft:cobblestone", 1));
        }
        assert_eq!(inv.workbench_output().unwrap().id, "minecraft:furnace");
        assert!(inv.take_workbench_output(false));
        assert_eq!(inv.count("minecraft:cobblestone"), 0);
        assert_eq!(inv.count("minecraft:furnace"), 1);
        assert!(inv.workbench_output().is_none());
    }
    #[test]
    fn closing_workbench_returns_unused_inputs() {
        let mut inv = Inventory::default();
        inv.workbench[0] = Some(ItemStack::new("minecraft:cobblestone", 12));
        inv.workbench[8] = Some(ItemStack::new("minecraft:oak_log", 3));
        assert!(inv.settle_workbench().is_empty());
        assert_eq!(inv.count("minecraft:cobblestone"), 12);
        assert_eq!(inv.count("minecraft:oak_log"), 3);
        assert!(inv.workbench.iter().all(Option::is_none));
    }
    #[test]
    fn pinned_jar_furnace_recipe_matches_when_available() {
        let jar = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../harness/.gradle/loom-cache/minecraftMaven/net/minecraft/minecraft-common-1fad6b3808/26.3/minecraft-common-1fad6b3808-26.3.jar",
        );
        if !jar.exists() {
            return;
        }
        let book = RecipeBook::from_jar(&jar).unwrap();
        let mut grid: [Option<ItemStack>; 9] = std::array::from_fn(|_| None);
        for index in [0, 1, 2, 3, 5, 6, 7, 8] {
            grid[index] = Some(ItemStack::new("minecraft:cobblestone", 1));
        }
        assert_eq!(
            book.matching(&grid, 3, 3).map(|item| item.id),
            Some("minecraft:furnace".into())
        );
    }
}
