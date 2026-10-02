//! Pinned 26.3 chicken lifecycle state; active steering is added separately.
use crate::{
    age::Age,
    animal::{self, AnimalEvent},
    control::MoveControl,
    cow::InteractionResult,
    health::{DamageResult, DamageState},
    movement::Body,
    navigation::{navigate_walk_to, GroundNavigation},
    
};
use glam::DVec3;
use minecraftoss_player::{
    inventory::Inventory,
    rng::{LegacyRandom, LootRandom},
};
use crate::fluid::FluidFrame;
use crate::walk_path::WalkProfile;
use minecraftoss_player::path_type::PathType;
use minecraftoss_player::World;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChickenVariant {
    Temperate,
    Warm,
    Cold,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChickenSoundVariant {
    Classic,
    Picky,
}

#[derive(Clone)]
pub struct Chicken {
    pub body: Body,
    pub move_control: MoveControl,
    pub navigation: GroundNavigation,
    pub yaw: f32,
    pub speed: f32,
    pub forward: f32,
    pub sideways: f32,
    pub age: Age,
    pub in_love: i32,
    pub persistence_required: bool,
    pub health: f32,
    pub damage: DamageState,
    pub variant: ChickenVariant,
    pub sound_variant: ChickenSoundVariant,
    pub egg_time: i32,
    pub is_chicken_jockey: bool,
    pub flap: f32,
    pub flap_speed: f32,
    pub old_flap: f32,
    pub old_flap_speed: f32,
    pub flapping: f32,
}

impl Chicken {
    /// Apply the pinned biome-priority appearance selector, then the sound
    /// registry draw, preserving both calls on the supplied level RNG stream.
    pub fn select_spawn_variants(
        &mut self,
        warm_biome: bool,
        cold_biome: bool,
        random: &mut impl LootRandom,
    ) {
        self.variant = if warm_biome {
            ChickenVariant::Warm
        } else if cold_biome {
            ChickenVariant::Cold
        } else {
            ChickenVariant::Temperate
        };
        let _ = random.next_int(1);
        self.sound_variant = if random.next_int(2) == 0 {
            ChickenSoundVariant::Classic
        } else {
            ChickenSoundVariant::Picky
        };
    }

    /// The walk search settings for this chicken's current size.
    pub fn walk_profile(&self) -> WalkProfile {
        #[allow(unused_mut)]
        let mut profile = WalkProfile::animal(self.body.width, self.body.height);
        profile.set_malus(PathType::Water, 0.0);
        profile
    }

    /// `PathNavigation.moveTo` over the world.
    pub fn navigate_to<W: World + ?Sized>(&mut self, world: &W, fluid: FluidFrame, target: DVec3, speed: f64) -> Option<bool> {
        let profile = self.walk_profile();
        navigate_walk_to(&self.body, &mut self.navigation, world, &profile, fluid, target, speed, 1)
    }

    /// `moveTo(entity, speed)`: as [`Self::navigate_to`], but the current
    /// path stays when no new one can be made.
    pub fn navigate_to_entity<W: World + ?Sized>(&mut self, world: &W, fluid: FluidFrame, target: DVec3, speed: f64) -> bool {
        let profile = self.walk_profile();
        crate::navigation::navigate_walk_to_entity(&self.body, &mut self.navigation, world, &profile, fluid, target, speed, 1)
    }

    pub fn hurt_generic(&mut self, amount: f32) -> DamageResult {
        self.damage.hurt_generic(&mut self.health, 4.0, amount)
    }

    pub fn interact_food(
        &mut self,
        inventory: &mut Inventory,
        hand: usize,
        infinite_materials: bool,
        is_chicken_food: impl Fn(&str) -> bool,
        cannot_age_lock: bool,
    ) -> (InteractionResult, Vec<AnimalEvent>) {
        animal::interact(
            &mut self.age,
            &mut self.in_love,
            &mut self.persistence_required,
            inventory,
            hand,
            infinite_materials,
            is_chicken_food,
            cannot_age_lock,
        )
    }

    pub fn new(position: DVec3) -> Self {
        Self {
            body: Body::new(position, 0.4, 0.7),
            move_control: MoveControl::default(),
            navigation: GroundNavigation::default(),
            yaw: 0.0,
            speed: 0.0,
            forward: 0.0,
            sideways: 0.0,
            age: Age::default(),
            in_love: 0,
            persistence_required: false,
            health: 4.0,
            damage: DamageState::default(),
            variant: ChickenVariant::Temperate,
            sound_variant: ChickenSoundVariant::Classic,
            // Vanilla draws 6000..11999 in the constructor; an entity-spawn
            // integration must supply that RNG draw or loaded EggLayTime.
            egg_time: 6000,
            is_chicken_jockey: false,
            flap: 0.0,
            flap_speed: 0.0,
            old_flap: 0.0,
            old_flap_speed: 0.0,
            flapping: 1.0,
        }
    }

    pub fn sync_dimensions(&mut self) {
        self.body.width = if self.age.baby() { 0.3 } else { 0.4 };
        self.body.height = if self.age.baby() { 0.4 } else { 0.7 };
    }

    /// Runs after Animal.aiStep, including on a NoAI chicken.
    /// Returns whether the pinned chicken-lay gift table produced an egg.
    pub fn ai_step(&mut self, random: &mut LegacyRandom) -> bool {
        self.old_flap = self.flap;
        self.old_flap_speed = self.flap_speed;
        self.flap_speed = (self.flap_speed
            + (if self.body.on_ground {
                -1.0_f32
            } else {
                4.0_f32
            }) * 0.3_f32)
            .clamp(0.0, 1.0);
        if !self.body.on_ground && self.flapping < 1.0 {
            self.flapping = 1.0;
        }
        self.flapping *= 0.9_f32;
        if !self.body.on_ground && self.body.velocity.y < 0.0 {
            self.body.velocity.y *= 0.6;
        }
        self.flap += self.flapping * 2.0_f32;
        if self.health > 0.0 && !self.age.baby() && !self.is_chicken_jockey {
            self.egg_time -= 1;
            if self.egg_time <= 0 {
                // Chicken-lay table yields one egg in the measured default
                // loot context, then pitch uses two floats before the reset.
                let _ = random.next_float();
                let _ = random.next_float();
                self.egg_time = random.next_int(6000) as i32 + 6000;
                return true;
            }
        }
        false
    }
}
