//! Source-informed pinned 26.3 villager body and basic living state.
//! Sources: EntityTypes.VILLAGER, Villager.BABY_DIMENSIONS,
//! Villager.createAttributes, AgeableMob, LivingEntity.
use crate::{age::Age, health::DamageState, movement::Body, poi::PoiType};
use glam::DVec3;

#[derive(Clone, Debug)]
pub struct Villager {
    pub body: Body,
    pub health: f32,
    pub damage: DamageState,
    pub age: Age,
    pub persistence_required: bool,
    /// `VillagerData`: its biome type, profession and level, and its
    /// trading experience.
    pub kind: String,
    pub profession: Profession,
    pub level: i32,
    pub xp: i32,
}

/// `VillagerProfession`, in registry order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profession {
    #[default]
    None,
    Armorer,
    Butcher,
    Cartographer,
    Cleric,
    Farmer,
    Fisherman,
    Fletcher,
    Leatherworker,
    Librarian,
    Mason,
    Nitwit,
    Shepherd,
    Toolsmith,
    Weaponsmith,
}

impl Profession {
    pub const ALL: [Profession; 15] = [
        Self::None,
        Self::Armorer,
        Self::Butcher,
        Self::Cartographer,
        Self::Cleric,
        Self::Farmer,
        Self::Fisherman,
        Self::Fletcher,
        Self::Leatherworker,
        Self::Librarian,
        Self::Mason,
        Self::Nitwit,
        Self::Shepherd,
        Self::Toolsmith,
        Self::Weaponsmith,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::None => "minecraft:none",
            Self::Armorer => "minecraft:armorer",
            Self::Butcher => "minecraft:butcher",
            Self::Cartographer => "minecraft:cartographer",
            Self::Cleric => "minecraft:cleric",
            Self::Farmer => "minecraft:farmer",
            Self::Fisherman => "minecraft:fisherman",
            Self::Fletcher => "minecraft:fletcher",
            Self::Leatherworker => "minecraft:leatherworker",
            Self::Librarian => "minecraft:librarian",
            Self::Mason => "minecraft:mason",
            Self::Nitwit => "minecraft:nitwit",
            Self::Shepherd => "minecraft:shepherd",
            Self::Toolsmith => "minecraft:toolsmith",
            Self::Weaponsmith => "minecraft:weaponsmith",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    /// The job site it works at.
    pub fn job_site(self) -> Option<PoiType> {
        Some(match self {
            Self::Armorer => PoiType::Armorer,
            Self::Butcher => PoiType::Butcher,
            Self::Cartographer => PoiType::Cartographer,
            Self::Cleric => PoiType::Cleric,
            Self::Farmer => PoiType::Farmer,
            Self::Fisherman => PoiType::Fisherman,
            Self::Fletcher => PoiType::Fletcher,
            Self::Leatherworker => PoiType::Leatherworker,
            Self::Librarian => PoiType::Librarian,
            Self::Mason => PoiType::Mason,
            Self::Shepherd => PoiType::Shepherd,
            Self::Toolsmith => PoiType::Toolsmith,
            Self::Weaponsmith => PoiType::Weaponsmith,
            Self::None | Self::Nitwit => return None,
        })
    }

    /// `heldJobSite`.
    pub fn holds(self, kind: PoiType) -> bool {
        self.job_site() == Some(kind)
    }

    /// `acquirableJobSite`: any job site for the unemployed, none for a
    /// nitwit, its own otherwise.
    pub fn can_acquire(self, kind: PoiType) -> bool {
        match self {
            Self::None => kind.acquirable_job_site(),
            _ => self.holds(kind),
        }
    }

    /// The profession whose job site this is (`AssignProfessionFromJobSite`:
    /// the first in the registry).
    pub fn of_job_site(kind: PoiType) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.holds(kind))
    }

    /// `workSound`.
    pub fn work_sound(self) -> Option<&'static str> {
        Some(match self {
            Self::Armorer => "entity.villager.work_armorer",
            Self::Butcher => "entity.villager.work_butcher",
            Self::Cartographer => "entity.villager.work_cartographer",
            Self::Cleric => "entity.villager.work_cleric",
            Self::Farmer => "entity.villager.work_farmer",
            Self::Fisherman => "entity.villager.work_fisherman",
            Self::Fletcher => "entity.villager.work_fletcher",
            Self::Leatherworker => "entity.villager.work_leatherworker",
            Self::Librarian => "entity.villager.work_librarian",
            Self::Mason => "entity.villager.work_mason",
            Self::Shepherd => "entity.villager.work_shepherd",
            Self::Toolsmith => "entity.villager.work_toolsmith",
            Self::Weaponsmith => "entity.villager.work_weaponsmith",
            Self::None | Self::Nitwit => return None,
        })
    }

    /// `secondaryPoi`: the blocks it tends beside its job site.
    pub fn secondary_poi(self, block: &str) -> bool {
        self == Self::Farmer && block == "minecraft:farmland"
    }
}

/// `VillagerType.byBiome` (`BY_BIOME`, plains otherwise).
pub fn type_for_biome(biome: &str) -> &'static str {
    match biome.trim_start_matches("minecraft:") {
        "badlands" | "desert" | "eroded_badlands" | "wooded_badlands" => "minecraft:desert",
        "bamboo_jungle" | "jungle" | "sparse_jungle" => "minecraft:jungle",
        "savanna_plateau" | "savanna" | "windswept_savanna" => "minecraft:savanna",
        "deep_frozen_ocean" | "frozen_ocean" | "frozen_river" | "ice_spikes" | "snowy_beach" | "snowy_taiga" | "snowy_plains" | "grove" | "snowy_slopes"
        | "frozen_peaks" | "jagged_peaks" => "minecraft:snow",
        "swamp" | "mangrove_swamp" => "minecraft:swamp",
        "old_growth_spruce_taiga" | "old_growth_pine_taiga" | "windswept_gravelly_hills" | "windswept_hills" | "taiga" | "windswept_forest" => "minecraft:taiga",
        _ => "minecraft:plains",
    }
}

impl Villager {
    pub fn new(position: DVec3) -> Self {
        Self {
            body: Body::new(position, 0.6, 1.95),
            health: 20.0,
            damage: DamageState::default(),
            age: Age::default(),
            persistence_required: false,
            kind: "minecraft:plains".to_owned(),
            profession: Profession::None,
            level: 1,
            xp: 0,
        }
    }

    pub fn set_age(&mut self, ticks: i32) {
        self.age.set(ticks);
        self.body.width = if self.age.baby() { 0.49 } else { 0.6 };
        self.body.height = if self.age.baby() { 0.98 } else { 1.95 };
    }

    pub fn eye_height(&self) -> f32 {
        if self.age.baby() {
            0.63
        } else {
            1.62
        }
    }
}
