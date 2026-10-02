//! Cow state and item interactions from pinned 26.3 Cow/AbstractCow/Animal.
//! The caller supplies data-pack tag membership and owns the shared tick.
use crate::age::{Age, AgeParticle};
use crate::animal::{self, AnimalEvent};
use crate::control::MoveControl;
use crate::health::{DamageResult, DamageState};
use crate::movement::Body;
use crate::navigation::{navigate_walk_to, GroundNavigation};
use glam::DVec3;
use minecraftoss_player::inventory::{Inventory, ItemStack};
use crate::fluid::FluidFrame;
use crate::walk_path::WalkProfile;
use minecraftoss_player::World;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CowVariant {
    Temperate,
    Warm,
    Cold,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CowSoundVariant {
    Classic,
    Moody,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionResult {
    Pass,
    SuccessServer,
    SuccessPredicted,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CowEvent {
    Hearts,
    Milk,
    AgeLock,
    AgeUnlock,
}

#[derive(Clone, Debug)]
pub struct Cow {
    pub body: Body,
    pub move_control: MoveControl,
    pub navigation: GroundNavigation,
    pub yaw: f32,
    pub speed: f32,
    pub forward: f32,
    pub sideways: f32,
    pub health: f32,
    pub damage: DamageState,
    pub age: Age,
    pub in_love: i32,
    pub persistence_required: bool,
    pub variant: CowVariant,
    pub sound_variant: CowSoundVariant,
    /// `MAX_HEALTH` (a horse's is its own).
    pub max_health: f32,
    /// Adult width and height, and a baby's scale.
    pub dimensions: (f32, f32, f32),
}
impl Cow {
    /// Request a ground path through the caller's gameplay terrain.
    /// None means the search cannot be evaluated for this state or terrain.
    /// The walk search settings for this cow's current size.
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
            body: Body::new(position, 0.9, 1.4),
            move_control: MoveControl::default(),
            navigation: GroundNavigation::default(),
            yaw: 0.0,
            speed: 0.0,
            forward: 0.0,
            sideways: 0.0,
            health: 10.0,
            damage: DamageState::default(),
            age: Age::default(),
            in_love: 0,
            persistence_required: false,
            variant: CowVariant::Temperate,
            sound_variant: CowSoundVariant::Classic,
            max_health: 10.0,
            dimensions: (0.9, 1.4, 0.5),
        }
    }
    pub fn sync_dimensions(&mut self) {
        let (width, height, baby_scale) = self.dimensions;
        if self.age.baby() {
            self.body.width = width * baby_scale;
            self.body.height = height * baby_scale;
        } else {
            self.body.width = width;
            self.body.height = height;
        }
    }
    pub fn hurt_generic(&mut self, amount: f32) -> DamageResult {
        self.damage.hurt_generic(&mut self.health, self.max_health, amount)
    }
    /// Runs in Animal.aiStep order after the other living/mob phases.
    /// Particle requests must be resolved against this cow's own RNG stream.
    pub fn tick_age_and_love(&mut self) -> (Vec<AgeParticle>, bool) {
        let before = self.age.baby();
        let age_events = self.age.tick(self.health > 0.0);
        if before != self.age.baby() {
            self.sync_dimensions();
        }
        if self.age.ticks != 0 {
            self.in_love = 0;
        }
        if self.in_love > 0 {
            self.in_love -= 1;
            return (age_events, self.in_love % 10 == 0);
        }
        (age_events, false)
    }
    /// Uses the same Inventory owned by player gameplay. The provided tag
    /// predicates come from authoritative item/entity tags, never a resource pack.
    pub fn interact(
        &mut self,
        inventory: &mut Inventory,
        hand: usize,
        infinite_materials: bool,
        in_cow_food: impl Fn(&str) -> bool,
        cannot_age_lock: bool,
    ) -> (InteractionResult, Vec<CowEvent>) {
        let Some(held) = inventory.slots.get(hand).and_then(Option::as_ref) else {
            return (InteractionResult::Pass, Vec::new());
        };
        let item = held.id.clone();
        if item == "minecraft:bucket" && !self.age.baby() {
            if infinite_materials {
                if !inventory
                    .slots
                    .iter()
                    .flatten()
                    .any(|s| s.id == "minecraft:milk_bucket")
                {
                    let _ = inventory.add_item(
                        ItemStack {
                            max: 1,
                            ..ItemStack::new("minecraft:milk_bucket", 1)
                        },
                        hand,
                    );
                }
            } else {
                animal::consume_one(inventory, hand);
                if inventory.slots[hand].is_none() {
                    inventory.slots[hand] = Some(ItemStack {
                        max: 1,
                        ..ItemStack::new("minecraft:milk_bucket", 1)
                    });
                } else {
                    let _ = inventory.add_item(
                        ItemStack {
                            max: 1,
                            ..ItemStack::new("minecraft:milk_bucket", 1)
                        },
                        hand,
                    );
                }
            }
            return (InteractionResult::SuccessPredicted, vec![CowEvent::Milk]);
        }
        let (result, events) = animal::interact(
            &mut self.age,
            &mut self.in_love,
            &mut self.persistence_required,
            inventory,
            hand,
            infinite_materials,
            in_cow_food,
            cannot_age_lock,
        );
        self.sync_dimensions();
        (
            result,
            events
                .into_iter()
                .map(|event| match event {
                    AnimalEvent::Hearts => CowEvent::Hearts,
                    AnimalEvent::AgeLock => CowEvent::AgeLock,
                    AnimalEvent::AgeUnlock => CowEvent::AgeUnlock,
                })
                .collect(),
        )
    }
}
