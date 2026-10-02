//! Default-item food components measured from the pinned 26.3 vanilla registry.
//! `FoodUse` follows Consumable.startConsuming/shouldEmitParticlesAndSounds/onConsume.
use crate::{
    inventory::{Inventory, ItemStack},
    survival::{EffectKind, FoodData, SurvivalStatus},
    GameMode,
};
use std::{collections::HashMap, sync::OnceLock};

#[derive(Clone, Debug)]
pub struct FoodInfo {
    pub nutrition: u8,
    pub saturation: f32,
    pub can_always_eat: bool,
    pub consume_ticks: u32,
    pub animation: String,
    pub sound: String,
    pub particles: bool,
    pub remainder: Option<ItemStack>,
}

pub fn catalog() -> &'static HashMap<String, FoodInfo> {
    static CATALOG: OnceLock<HashMap<String, FoodInfo>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let document: serde_json::Value =
            serde_json::from_str(include_str!("../data/food-26.3.json"))
                .expect("valid measured food catalog");
        assert_eq!(document["minecraft_version"], "26.3");
        document["items"]
            .as_object()
            .expect("food entries")
            .iter()
            .map(|(id, value)| {
                let saturation_bits = u32::from_str_radix(
                    value["saturation_bits"].as_str().expect("saturation bits"),
                    16,
                )
                .expect("hex saturation bits");
                let remainder = value.get("remainder").map(|rem| {
                    ItemStack::new(
                        rem["id"].as_str().expect("remainder id"),
                        rem["count"].as_u64().expect("remainder count") as u8,
                    )
                });
                (
                    id.clone(),
                    FoodInfo {
                        nutrition: value["nutrition"].as_u64().expect("nutrition") as u8,
                        saturation: f32::from_bits(saturation_bits),
                        can_always_eat: value["can_always_eat"].as_bool().expect("can_always_eat"),
                        consume_ticks: value["consume_ticks"].as_u64().expect("consume_ticks")
                            as u32,
                        animation: value["animation"].as_str().expect("animation").to_owned(),
                        sound: value["sound"]
                            .as_str()
                            .expect("sound")
                            .trim_start_matches("minecraft:")
                            .to_owned(),
                        particles: value["particles"].as_bool().expect("particles"),
                        remainder,
                    },
                )
            })
            .collect()
    })
}

/// Default 26.3 Consumables.onConsume effects. The boolean requests the
/// separate collision-checked random teleport used by chorus fruit.
pub fn apply_consumed_food_effects(
    id: &str,
    status: &mut SurvivalStatus,
    mut next_random: impl FnMut() -> f32,
) -> bool {
    use EffectKind::*;
    match id {
        "minecraft:chicken" if next_random() < 0.3 => status.add_effect(Hunger, 600, 0),
        "minecraft:poisonous_potato" if next_random() < 0.6 => status.add_effect(Poison, 100, 0),
        "minecraft:rotten_flesh" if next_random() < 0.8 => status.add_effect(Hunger, 600, 0),
        "minecraft:spider_eye" => status.add_effect(Poison, 100, 0),
        "minecraft:pufferfish" => {
            status.add_effect(Poison, 1200, 1);
            status.add_effect(Hunger, 300, 2);
            status.add_effect(Nausea, 300, 0);
        }
        "minecraft:golden_apple" => {
            status.add_effect(Regeneration, 100, 1);
            status.add_effect(Absorption, 2400, 0);
        }
        "minecraft:enchanted_golden_apple" => {
            status.add_effect(Regeneration, 400, 1);
            status.add_effect(Resistance, 6000, 0);
            status.add_effect(FireResistance, 6000, 0);
            status.add_effect(Absorption, 2400, 3);
        }
        "minecraft:honey_bottle" => status.remove_effect(Poison),
        "minecraft:chorus_fruit" => return true,
        _ => {}
    }
    false
}

#[derive(Clone, Debug)]
pub struct FoodUse {
    pub slot: usize,
    pub stack: ItemStack,
    pub info: FoodInfo,
    pub elapsed_ticks: u32,
}

#[derive(Debug, PartialEq)]
pub enum FoodUseTick {
    Continuing { emit_sound: bool },
    Finished { overflow: Option<ItemStack> },
    Cancelled,
}

impl FoodUse {
    pub fn start(slot: usize, stack: &ItemStack, food: &FoodData, mode: GameMode) -> Option<Self> {
        let info = catalog().get(&stack.id)?.clone();
        if stack.count == 0
            || (mode != GameMode::Creative && food.level >= 20 && !info.can_always_eat)
        {
            return None;
        }
        Some(Self {
            slot,
            stack: stack.clone(),
            info,
            elapsed_ticks: 0,
        })
    }

    pub fn remaining_ticks(&self) -> u32 {
        self.info.consume_ticks.saturating_sub(self.elapsed_ticks)
    }

    pub fn tick(
        &mut self,
        inventory: &mut Inventory,
        food: &mut FoodData,
        mode: GameMode,
    ) -> FoodUseTick {
        let Some(current) = inventory.slots.get(self.slot).and_then(Option::as_ref) else {
            return FoodUseTick::Cancelled;
        };
        if !current.same_item(&self.stack) || current.count == 0 {
            return FoodUseTick::Cancelled;
        }
        self.elapsed_ticks += 1;
        let remaining = self.remaining_ticks();
        if remaining > 0 {
            // Consumable uses elapsed > floor(duration * .21875), then every fourth remaining tick.
            let lead_in = (self.info.consume_ticks as f32 * 0.21875) as u32;
            return FoodUseTick::Continuing {
                emit_sound: self.elapsed_ticks > lead_in && remaining % 4 == 0,
            };
        }
        food.eat(self.info.nutrition, self.info.saturation);
        let mut overflow = None;
        if mode != GameMode::Creative {
            let current = inventory.slots[self.slot].as_mut().expect("validated slot");
            current.count -= 1;
            if current.count == 0 {
                inventory.slots[self.slot] = self.info.remainder.clone();
            } else if let Some(remainder) = &self.info.remainder {
                overflow = inventory.add_item(remainder.clone(), self.slot);
            }
        }
        FoodUseTick::Finished { overflow }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_food_values_and_duration() {
        let data = catalog();
        assert_eq!(data.len(), 40);
        assert_eq!(data["minecraft:apple"].nutrition, 4);
        assert_eq!(data["minecraft:cooked_beef"].nutrition, 8);
        assert_eq!(
            data["minecraft:golden_carrot"].saturation.to_bits(),
            14.400001f32.to_bits()
        );
        assert_eq!(data["minecraft:dried_kelp"].consume_ticks, 16);
        assert_eq!(data["minecraft:honey_bottle"].consume_ticks, 40);
    }

    #[test]
    fn special_food_effects_use_pinned_probabilities_and_durations() {
        let mut status = SurvivalStatus::default();
        apply_consumed_food_effects("minecraft:chicken", &mut status, || 0.3);
        assert_eq!(status.effect(EffectKind::Hunger), None);
        apply_consumed_food_effects("minecraft:chicken", &mut status, || 0.299);
        assert_eq!(status.effect(EffectKind::Hunger).unwrap().duration, 600);
        apply_consumed_food_effects("minecraft:golden_apple", &mut status, || panic!("no roll"));
        assert_eq!(
            status.effect(EffectKind::Regeneration).unwrap().amplifier,
            1
        );
        assert_eq!(
            status.effect(EffectKind::Absorption).unwrap().duration,
            2400
        );
        assert_eq!(status.absorption, 4.0);
        apply_consumed_food_effects("minecraft:pufferfish", &mut status, || panic!("no roll"));
        assert_eq!(status.effect(EffectKind::Poison).unwrap().duration, 1200);
        apply_consumed_food_effects("minecraft:honey_bottle", &mut status, || panic!("no roll"));
        assert_eq!(status.effect(EffectKind::Poison), None);
        assert!(apply_consumed_food_effects(
            "minecraft:chorus_fruit",
            &mut status,
            || panic!("no roll")
        ));
    }

    #[test]
    fn consume_only_after_full_use_and_leave_remainder() {
        let mut inventory = Inventory::default();
        inventory.slots[0] = Some(ItemStack::new("minecraft:mushroom_stew", 1));
        let mut food = FoodData::default();
        food.level = 10;
        let mut use_item = FoodUse::start(
            0,
            inventory.slots[0].as_ref().unwrap(),
            &food,
            GameMode::Survival,
        )
        .unwrap();
        for _ in 0..31 {
            assert!(matches!(
                use_item.tick(&mut inventory, &mut food, GameMode::Survival),
                FoodUseTick::Continuing { .. }
            ));
            assert_eq!(food.level, 10);
        }
        assert_eq!(
            use_item.tick(&mut inventory, &mut food, GameMode::Survival),
            FoodUseTick::Finished { overflow: None }
        );
        assert_eq!(food.level, 16);
        assert_eq!(inventory.slots[0].as_ref().unwrap().id, "minecraft:bowl");
    }

    #[test]
    fn full_hunger_blocks_normal_food_but_not_always_edible() {
        let food = FoodData::default();
        assert!(FoodUse::start(
            0,
            &ItemStack::new("minecraft:apple", 1),
            &food,
            GameMode::Survival
        )
        .is_none());
        assert!(FoodUse::start(
            0,
            &ItemStack::new("minecraft:golden_apple", 1),
            &food,
            GameMode::Survival
        )
        .is_some());
        assert!(FoodUse::start(
            0,
            &ItemStack::new("minecraft:apple", 1),
            &food,
            GameMode::Creative
        )
        .is_some());
    }

    #[test]
    fn replaced_stack_cancels_without_restoring_hunger() {
        let mut inventory = Inventory::default();
        inventory.slots[0] = Some(ItemStack::new("minecraft:apple", 2));
        let mut food = FoodData::default();
        food.level = 14;
        let mut using = FoodUse::start(
            0,
            inventory.slots[0].as_ref().unwrap(),
            &food,
            GameMode::Survival,
        )
        .unwrap();
        inventory.slots[0] = Some(ItemStack::new("minecraft:bread", 2));
        assert_eq!(
            using.tick(&mut inventory, &mut food, GameMode::Survival),
            FoodUseTick::Cancelled
        );
        assert_eq!(food.level, 14);
        assert_eq!(inventory.slots[0].as_ref().unwrap().count, 2);
    }

    #[test]
    fn stacked_food_consumes_one_and_moves_remainder_to_inventory() {
        let mut inventory = Inventory::default();
        inventory.slots[0] = Some(ItemStack::new("minecraft:honey_bottle", 2));
        let mut food = FoodData::default();
        food.level = 8;
        let mut using = FoodUse::start(
            0,
            inventory.slots[0].as_ref().unwrap(),
            &food,
            GameMode::Survival,
        )
        .unwrap();
        for _ in 0..39 {
            assert!(matches!(
                using.tick(&mut inventory, &mut food, GameMode::Survival),
                FoodUseTick::Continuing { .. }
            ));
        }
        assert_eq!(
            using.tick(&mut inventory, &mut food, GameMode::Survival),
            FoodUseTick::Finished { overflow: None }
        );
        assert_eq!(inventory.slots[0].as_ref().unwrap().count, 1);
        assert_eq!(inventory.count("minecraft:glass_bottle"), 1);
        assert_eq!(food.level, 14);
    }
}
