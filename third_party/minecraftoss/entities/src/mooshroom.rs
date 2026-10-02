//! Source-informed MushroomCow bowl and brown-flower transaction layer.
use crate::cow::{Cow, CowEvent, InteractionResult};
use glam::DVec3;
use minecraftoss_player::inventory::{Inventory, ItemStack};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MushroomVariant {
    Red,
    Brown,
}

/// Built-in SuspiciousEffectHolder flower entries from pinned Blocks bootstrap.
/// Data-pack or modded item extensions can supply their own payload to interact.
pub fn flower_effects(item: &str) -> Option<Value> {
    let (effect, duration) = match item {
        "minecraft:dandelion" | "minecraft:golden_dandelion" | "minecraft:blue_orchid" => {
            ("minecraft:saturation", 7)
        }
        "minecraft:torchflower" | "minecraft:poppy" => ("minecraft:night_vision", 100),
        "minecraft:allium" => ("minecraft:fire_resistance", 60),
        "minecraft:azure_bluet" => ("minecraft:blindness", 220),
        "minecraft:red_tulip"
        | "minecraft:orange_tulip"
        | "minecraft:white_tulip"
        | "minecraft:pink_tulip" => ("minecraft:weakness", 140),
        "minecraft:oxeye_daisy" => ("minecraft:regeneration", 140),
        "minecraft:cornflower" => ("minecraft:jump_boost", 100),
        "minecraft:wither_rose" => ("minecraft:wither", 140),
        "minecraft:lily_of_the_valley" => ("minecraft:poison", 220),
        _ => return None,
    };
    Some(json!([{"id": effect, "duration": duration}]))
}

#[derive(Clone, Debug)]
pub struct MushroomCow {
    pub cow: Cow,
    pub state: MushroomCowState,
}

#[derive(Clone, Debug)]
pub struct MushroomCowState {
    pub variant: MushroomVariant,
    /// The data component payload returned by the flower's
    /// SuspiciousEffectHolder. Its lookup belongs to the gameplay registry.
    pub stew_effects: Option<Value>,
    /// UUID of the most recent bolt that changed this mooshroom.
    pub last_lightning_bolt_uuid: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MushroomShearing {
    pub drop_item: &'static str,
    pub drop_count: u8,
    pub tool_damage: u32,
}

impl MushroomCow {
    pub fn new(position: DVec3, variant: MushroomVariant) -> Self {
        Self {
            cow: Cow::new(position),
            state: MushroomCowState {
                variant,
                stew_effects: None,
                last_lightning_bolt_uuid: None,
            },
        }
    }

    pub fn interact(
        &mut self,
        inventory: &mut Inventory,
        hand: usize,
        infinite_materials: bool,
        in_cow_food: impl Fn(&str) -> bool,
        flower_effects: impl Fn(&str) -> Option<Value>,
    ) -> (InteractionResult, Vec<CowEvent>) {
        self.state.interact(
            &mut self.cow,
            inventory,
            hand,
            infinite_materials,
            in_cow_food,
            flower_effects,
        )
    }
}

impl MushroomCowState {
    /// MushroomCow.thunderHit toggles for a new bolt UUID, including babies.
    /// Only the most recent UUID is retained, so a past bolt can toggle again.
    pub fn lightning_hit(&mut self, bolt_uuid: &str) -> bool {
        if self.last_lightning_bolt_uuid.as_deref() == Some(bolt_uuid) {
            return false;
        }
        self.variant = match self.variant {
            MushroomVariant::Red => MushroomVariant::Brown,
            MushroomVariant::Brown => MushroomVariant::Red,
        };
        self.last_lightning_bolt_uuid = Some(bolt_uuid.to_owned());
        true
    }

    /// MushroomCow.readyForShearing and the pinned shear loot table's
    /// red/brown mushroom entry. Conversion belongs to EntityWorld.
    pub fn shear(&self, cow: &Cow) -> Option<MushroomShearing> {
        if cow.age.baby() || cow.health <= 0.0 {
            return None;
        }
        Some(MushroomShearing {
            drop_item: match self.variant {
                MushroomVariant::Red => "minecraft:red_mushroom",
                MushroomVariant::Brown => "minecraft:brown_mushroom",
            },
            drop_count: 5,
            tool_damage: 1,
        })
    }

    pub fn interact(
        &mut self,
        cow: &mut Cow,
        inventory: &mut Inventory,
        hand: usize,
        infinite_materials: bool,
        in_cow_food: impl Fn(&str) -> bool,
        flower_effects: impl Fn(&str) -> Option<Value>,
    ) -> (InteractionResult, Vec<CowEvent>) {
        let Some(held) = inventory.slots.get(hand).and_then(Option::as_ref) else {
            return (InteractionResult::Pass, Vec::new());
        };
        let item = held.id.clone();
        if item == "minecraft:bowl" && !cow.age.baby() {
            let effects = self.stew_effects.take();
            let mut stew = ItemStack {
                max: 1,
                ..ItemStack::new(
                    if effects.is_some() {
                        "minecraft:suspicious_stew"
                    } else {
                        "minecraft:mushroom_stew"
                    },
                    1,
                )
            };
            if let Some(effects) = effects {
                stew.components = Some(json!({"minecraft:suspicious_stew_effects": effects}));
            }
            if infinite_materials {
                if !inventory
                    .slots
                    .iter()
                    .flatten()
                    .any(|stack| stack.same_item(&stew))
                {
                    let _ = inventory.add_item(stew, hand);
                }
            } else {
                let count = inventory.slots[hand].as_ref().unwrap().count;
                if count == 1 {
                    inventory.slots[hand] = Some(stew);
                } else {
                    inventory.slots[hand].as_mut().unwrap().count -= 1;
                    let _ = inventory.add_item(stew, hand);
                }
            }
            return (InteractionResult::SuccessPredicted, Vec::new());
        }
        if self.variant == MushroomVariant::Brown && !cow.age.baby() {
            if let Some(effects) = flower_effects(&item) {
                if self.stew_effects.is_none() {
                    if !infinite_materials {
                        let count = inventory.slots[hand].as_ref().unwrap().count;
                        if count == 1 {
                            inventory.slots[hand] = None;
                        } else {
                            inventory.slots[hand].as_mut().unwrap().count -= 1;
                        }
                    }
                    self.stew_effects = Some(effects);
                }
                return (InteractionResult::SuccessPredicted, Vec::new());
            }
        }
        cow.interact(inventory, hand, infinite_materials, in_cow_food, false)
    }
}
