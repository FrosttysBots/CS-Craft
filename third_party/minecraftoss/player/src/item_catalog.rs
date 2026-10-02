//! Default item components measured in the pinned 26.3 furnace context.
//! This sparse catalog is external data; absent items use vanilla's 64-stack,
//! non-fuel, no-remainder, non-equippable and zero-armor defaults.
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

#[derive(Clone, Debug)]
pub struct ItemProperties {
    pub fuel_component: bool,
    pub burn_ticks: u32,
    pub speed_multiplier: f32,
    pub max_stack: u8,
    pub remainder: Option<(String, u8)>,
    pub equippable_slot: Option<String>,
    pub equipment_asset: Option<String>,
    pub armor_points: f64,
    pub max_damage: u32,
    pub damage_per_block: u32,
    /// The default `attribute_modifiers` component, in order.
    pub attribute_modifiers: Vec<AttributeModifier>,
    /// The `weapon` component: durability lost per attack, and how long a
    /// hit disables a shield.
    pub weapon: Option<(u32, f32)>,
}

/// `AttributeModifier.Operation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModifierOperation {
    AddValue,
    AddMultipliedBase,
    AddMultipliedTotal,
}

impl ModifierOperation {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "add_value" => Self::AddValue,
            "add_multiplied_base" => Self::AddMultipliedBase,
            "add_multiplied_total" => Self::AddMultipliedTotal,
            _ => return None,
        })
    }
}

/// One entry of an item's `attribute_modifiers`.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeModifier {
    pub attribute: String,
    pub id: String,
    pub amount: f64,
    pub operation: ModifierOperation,
    /// `EquipmentSlotGroup`: any, mainhand, offhand, hand, feet, legs,
    /// chest, head, armor, body or saddle.
    pub slot: String,
}

impl AttributeModifier {
    /// `EquipmentSlotGroup.test` for the main hand.
    pub fn in_main_hand(&self) -> bool {
        matches!(self.slot.as_str(), "mainhand" | "hand" | "any")
    }
}

/// `AttributeInstance.calculateValue`: the base with added values, then
/// the multipliers of that base, then of the total, clamped to the
/// attribute's range.
pub fn attribute_value<'a>(base: f64, modifiers: impl IntoIterator<Item = &'a AttributeModifier> + Clone, range: (f64, f64)) -> f64 {
    let mut base = base;
    for m in modifiers.clone() {
        if m.operation == ModifierOperation::AddValue {
            base += m.amount;
        }
    }
    let mut value = base;
    for m in modifiers.clone() {
        if m.operation == ModifierOperation::AddMultipliedBase {
            value += base * m.amount;
        }
    }
    for m in modifiers {
        if m.operation == ModifierOperation::AddMultipliedTotal {
            value *= 1.0 + m.amount;
        }
    }
    value.clamp(range.0, range.1)
}

#[derive(Clone, Debug)]
pub struct ItemCatalog {
    items: HashMap<String, ItemProperties>,
}
impl ItemCatalog {
    pub fn from_path(path: &Path) -> Result<Self> {
        let bytes =
            fs::read(path).with_context(|| format!("read item catalog {}", path.display()))?;
        let root: Value = serde_json::from_slice(&bytes)?;
        if root.get("schema_version").and_then(Value::as_u64) != Some(1)
            || root.get("minecraft_version").and_then(Value::as_str) != Some("26.3")
        {
            bail!("unsupported item catalog version at {}", path.display());
        }
        let raw = root
            .get("items")
            .and_then(Value::as_object)
            .context("missing items")?;
        let mut items = HashMap::with_capacity(raw.len());
        for (id, value) in raw {
            let fuel_component = value
                .get("fuel_component")
                .and_then(Value::as_bool)
                .context("fuel_component")?;
            let burn_ticks = value
                .get("burn_ticks")
                .and_then(Value::as_u64)
                .context("burn_ticks")?;
            let burn_ticks = u32::try_from(burn_ticks)?;
            let speed_bits = value
                .get("speed_bits")
                .and_then(Value::as_str)
                .context("speed_bits")?;
            let speed_multiplier = f32::from_bits(u32::from_str_radix(speed_bits, 16)?);
            let speed_decimal = value
                .get("speed_decimal")
                .and_then(Value::as_str)
                .context("speed_decimal")?
                .parse::<f32>()?;
            if speed_decimal.to_bits() != speed_multiplier.to_bits() {
                bail!("speed decimal/bit mismatch for {id}");
            }
            if !speed_multiplier.is_finite() {
                bail!("nonfinite speed multiplier for {id}");
            }
            let max_stack = value
                .get("max_stack")
                .and_then(Value::as_u64)
                .context("max_stack")?;
            let max_stack = u8::try_from(max_stack)?;
            if max_stack == 0 || max_stack > 64 {
                bail!("invalid max stack for {id}");
            }
            let remainder = value
                .get("remainder")
                .map(|rem| -> Result<_> {
                    let rem_id = rem
                        .get("id")
                        .and_then(Value::as_str)
                        .context("remainder id")?;
                    let count = rem
                        .get("count")
                        .and_then(Value::as_u64)
                        .context("remainder count")?;
                    Ok((rem_id.to_owned(), u8::try_from(count)?))
                })
                .transpose()?;
            let equippable_slot = value
                .get("equippable_slot")
                .map(|slot| {
                    let name = slot.as_str().context("equippable_slot")?;
                    if !matches!(
                        name,
                        "mainhand"
                            | "offhand"
                            | "feet"
                            | "legs"
                            | "chest"
                            | "head"
                            | "body"
                            | "saddle"
                    ) {
                        bail!("unknown equipment slot {name} for {id}");
                    }
                    Ok(name.to_owned())
                })
                .transpose()?;
            let equipment_asset = value
                .get("equipment_asset")
                .map(|asset| asset.as_str().context("equipment_asset").map(str::to_owned))
                .transpose()?;
            if equipment_asset.is_some() && equippable_slot.is_none() {
                bail!("equipment asset without slot for {id}");
            }
            let armor_points = match value.get("armor_bits") {
                Some(bits) => {
                    let bits = bits.as_str().context("armor_bits")?;
                    let points = f64::from_bits(u64::from_str_radix(bits, 16)?);
                    let decimal = value
                        .get("armor_decimal")
                        .and_then(Value::as_str)
                        .context("armor_decimal")?
                        .parse::<f64>()?;
                    if !points.is_finite() || points.to_bits() != decimal.to_bits() {
                        bail!("armor decimal/bit mismatch for {id}");
                    }
                    points
                }
                None => 0.0,
            };
            let max_damage = value
                .get("max_damage")
                .map(|value| {
                    value
                        .as_u64()
                        .context("max_damage")
                        .and_then(|n| Ok(u32::try_from(n)?))
                })
                .transpose()?
                .unwrap_or(0);
            let damage_per_block = value
                .get("damage_per_block")
                .map(|value| {
                    value
                        .as_u64()
                        .context("damage_per_block")
                        .and_then(|n| Ok(u32::try_from(n)?))
                })
                .transpose()?
                .unwrap_or(0);
            let attribute_modifiers = value
                .get("attribute_modifiers")
                .map(|list| -> Result<Vec<AttributeModifier>> {
                    list.as_array()
                        .context("attribute_modifiers")?
                        .iter()
                        .map(|m| {
                            let text = |key: &str| m.get(key).and_then(Value::as_str).with_context(|| format!("modifier {key} for {id}"));
                            let amount = f64::from_bits(u64::from_str_radix(text("amount_bits")?, 16)?);
                            if amount.to_bits() != text("amount_decimal")?.parse::<f64>()?.to_bits() {
                                bail!("modifier amount decimal/bit mismatch for {id}");
                            }
                            Ok(AttributeModifier {
                                attribute: text("attribute")?.to_owned(),
                                id: text("id")?.to_owned(),
                                amount,
                                operation: ModifierOperation::parse(text("operation")?).context("modifier operation")?,
                                slot: text("slot")?.to_owned(),
                            })
                        })
                        .collect()
                })
                .transpose()?
                .unwrap_or_default();
            let weapon = value
                .get("weapon")
                .map(|w| -> Result<(u32, f32)> {
                    let per_attack = u32::try_from(w.get("item_damage_per_attack").and_then(Value::as_u64).context("item_damage_per_attack")?)?;
                    let bits = w.get("disable_blocking_for_seconds_bits").and_then(Value::as_str).context("disable_blocking")?;
                    Ok((per_attack, f32::from_bits(u32::from_str_radix(bits, 16)?)))
                })
                .transpose()?;
            items.insert(
                id.clone(),
                ItemProperties {
                    fuel_component,
                    burn_ticks,
                    speed_multiplier,
                    max_stack,
                    remainder,
                    equippable_slot,
                    equipment_asset,
                    armor_points,
                    max_damage,
                    damage_per_block,
                    attribute_modifiers,
                    weapon,
                },
            );
        }
        Ok(Self { items })
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn get(&self, id: &str) -> Option<&ItemProperties> {
        self.items.get(id)
    }
    pub fn equipment_assets(&self) -> impl Iterator<Item = &str> {
        self.items
            .values()
            .filter_map(|item| item.equipment_asset.as_deref())
    }
    pub fn max_stack(&self, id: &str) -> u8 {
        self.get(id).map_or(64, |item| item.max_stack)
    }
}
