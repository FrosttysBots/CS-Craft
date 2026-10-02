//! Sheep wool and shearing state from pinned 26.3 Sheep, Animal and AgeableMob.
//! Presentation, grazing AI and world item entities are separate consumers.
use crate::age::Age;
use crate::animal::{self, AnimalEvent};
use crate::cow::InteractionResult;
use crate::loot::ShearingLootBook;
use minecraftoss_player::inventory::{Inventory, ItemStack};
use minecraftoss_player::rng::LootRandom;

pub const DYE_NAMES: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SheepSpawnClimate {
    Temperate,
    Warm,
    Cold,
}

/// Pinned 26.3 `SheepColorSpawnRules`: weighted outer color and, for the
/// common branch, a second weighted roll for its rare pink variant.
pub fn spawn_color(climate: SheepSpawnClimate, random: &mut impl LootRandom) -> u8 {
    let choices: [(u8, u32); 5] = match climate {
        SheepSpawnClimate::Temperate => [(15, 5), (7, 5), (8, 5), (12, 3), (0, 82)],
        SheepSpawnClimate::Warm => [(7, 5), (8, 5), (0, 5), (15, 3), (12, 82)],
        SheepSpawnClimate::Cold => [(8, 5), (7, 5), (0, 5), (12, 3), (15, 82)],
    };
    let mut roll = random.next_int(100);
    for (index, (color, weight)) in choices.into_iter().enumerate() {
        if roll < weight {
            return if index == 4 && random.next_int(500) == 499 {
                6 // pink
            } else {
                color
            };
        }
        roll -= weight;
    }
    unreachable!("weights total 100")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Wool {
    data: u8,
}

impl Default for Wool {
    fn default() -> Self {
        Self { data: 0 }
    }
}

impl Wool {
    pub fn from_data(data: u8) -> Self {
        Self { data }
    }

    pub fn data(self) -> u8 {
        self.data
    }

    pub fn color(self) -> &'static str {
        DYE_NAMES[(self.data & 15) as usize]
    }

    pub fn set_color(&mut self, color: u8) {
        assert!(color < 16);
        self.data = (self.data & 0xf0) | color;
    }

    pub fn sheared(self) -> bool {
        self.data & 16 != 0
    }

    pub fn set_sheared(&mut self, sheared: bool) {
        if sheared {
            self.data |= 16;
        } else {
            self.data &= !16;
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Sheep {
    pub age: Age,
    pub in_love: i32,
    pub wool: Wool,
    pub persistence_required: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ShearsResult {
    Sheared {
        drops: Vec<ItemStack>,
        tool_damage: u32,
    },
    Consumed,
}

impl Sheep {
    pub fn for_natural_spawn(
        climate: SheepSpawnClimate,
        world_random: &mut impl LootRandom,
    ) -> Self {
        let mut sheep = Self::default();
        sheep.wool.set_color(spawn_color(climate, world_random));
        sheep
    }

    pub fn interact_food(
        &mut self,
        inventory: &mut Inventory,
        hand: usize,
        infinite_materials: bool,
        is_food: impl Fn(&str) -> bool,
        cannot_age_lock: bool,
    ) -> (InteractionResult, Vec<AnimalEvent>) {
        animal::interact(
            &mut self.age,
            &mut self.in_love,
            &mut self.persistence_required,
            inventory,
            hand,
            infinite_materials,
            is_food,
            cannot_age_lock,
        )
    }

    pub fn ready_for_shearing(&self) -> bool {
        !self.wool.sheared() && !self.age.baby()
    }

    pub fn use_shears(&mut self, loot: &mut ShearingLootBook) -> Option<ShearsResult> {
        if !self.ready_for_shearing() {
            return Some(ShearsResult::Consumed);
        }
        let drops = loot.roll_sheep(self.wool.color())?;
        self.wool.set_sheared(true);
        Some(ShearsResult::Sheared {
            drops,
            tool_damage: 1,
        })
    }

    /// Sheep.ate calls Animal.ate before clearing sheared state.
    pub fn ate(&mut self) {
        self.wool.set_sheared(false);
        if self.age.can_grow() {
            self.age.grow(60, false);
        }
    }
}
