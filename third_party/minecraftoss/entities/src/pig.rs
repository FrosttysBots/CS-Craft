//! Pinned 26.3 Pig/Animal state. Active steering and goal behavior are separate.
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
use minecraftoss_player::inventory::Inventory;
use crate::fluid::FluidFrame;
use crate::walk_path::WalkProfile;
use minecraftoss_player::World;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PigVariant {
    Temperate,
    Warm,
    Cold,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PigSoundVariant {
    Classic,
    Mini,
    Big,
}

#[derive(Clone, Debug)]
pub struct Pig {
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
    pub variant: PigVariant,
    pub sound_variant: PigSoundVariant,
    pub saddled: bool,
}

impl Pig {
    /// The walk search settings for this pig's current size.
    pub fn walk_profile(&self) -> WalkProfile {
        #[allow(unused_mut)]
        let mut profile = WalkProfile::animal(self.body.width, self.body.height);
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

    pub fn new(position: DVec3) -> Self {
        Self {
            body: Body::new(position, 0.9, 0.9),
            move_control: MoveControl::default(),
            navigation: GroundNavigation::default(),
            yaw: 0.0,
            speed: 0.0,
            forward: 0.0,
            sideways: 0.0,
            age: Age::default(),
            in_love: 0,
            persistence_required: false,
            health: 10.0,
            damage: DamageState::default(),
            variant: PigVariant::Temperate,
            sound_variant: PigSoundVariant::Classic,
            saddled: false,
        }
    }

    pub fn sync_dimensions(&mut self) {
        let size = if self.age.baby() { 0.45 } else { 0.9 };
        self.body.width = size;
        self.body.height = size;
    }

    pub fn hurt_generic(&mut self, amount: f32) -> DamageResult {
        self.damage.hurt_generic(&mut self.health, 10.0, amount)
    }

    /// Pig.mobInteract delegates an eligible saddle stack to Equippable's
    /// living-entity interaction. Mounting an already-saddled pig needs an
    /// actual player and remains a separate world interaction.
    pub fn interact_saddle(
        &mut self,
        inventory: &mut Inventory,
        hand: usize,
        infinite_materials: bool,
    ) -> InteractionResult {
        if self.health <= 0.0 || self.age.baby() || self.saddled {
            return InteractionResult::Pass;
        }
        if inventory
            .slots
            .get(hand)
            .and_then(Option::as_ref)
            .map(|stack| stack.id.as_str())
            != Some("minecraft:saddle")
        {
            return InteractionResult::Pass;
        }
        if !infinite_materials {
            animal::consume_one(inventory, hand);
        }
        self.saddled = true;
        InteractionResult::SuccessPredicted
    }

    pub fn interact_food(
        &mut self,
        inventory: &mut Inventory,
        hand: usize,
        infinite_materials: bool,
        is_pig_food: impl Fn(&str) -> bool,
        cannot_age_lock: bool,
    ) -> (InteractionResult, Vec<AnimalEvent>) {
        animal::interact(
            &mut self.age,
            &mut self.in_love,
            &mut self.persistence_required,
            inventory,
            hand,
            infinite_materials,
            is_pig_food,
            cannot_age_lock,
        )
    }
}
