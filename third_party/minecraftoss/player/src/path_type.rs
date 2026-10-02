//! `PathType`: how pathfinding sees a block, source-informed by pinned 26.3
//! `PathType` and `WalkNodeEvaluator.getPathTypeFromState`. Worlds backed
//! by the block catalog classify every state exactly; the name-based
//! classification here serves authored scenes and fixtures.

use crate::{authored_collision_boxes, Block};

/// `PathType`, in the pinned declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PathType {
    Blocked,
    Open,
    Walkable,
    WalkableDoor,
    Trapdoor,
    PowderSnow,
    OnTopOfPowderSnow,
    Fence,
    Lava,
    Water,
    WaterBorder,
    Rail,
    UnpassableRail,
    FireInNeighbor,
    Fire,
    DamagingInNeighbor,
    Damaging,
    DoorOpen,
    DoorWoodClosed,
    DoorIronClosed,
    Breach,
    Leaves,
    StickyHoney,
    Cocoa,
    DamageCautious,
    OnTopOfTrapdoor,
    BigMobsCloseToDanger,
}

impl PathType {
    pub const COUNT: usize = 27;

    /// `PathType.getMalus`: the cost a mob pays unless it sets its own.
    pub fn default_malus(self) -> f32 {
        match self {
            Self::Blocked | Self::PowderSnow | Self::Fence | Self::Lava | Self::UnpassableRail | Self::Damaging => -1.0,
            Self::DoorWoodClosed | Self::DoorIronClosed | Self::Leaves => -1.0,
            Self::Water | Self::WaterBorder | Self::FireInNeighbor | Self::DamagingInNeighbor | Self::StickyHoney => 8.0,
            Self::Fire => 16.0,
            Self::Breach | Self::BigMobsCloseToDanger => 4.0,
            _ => 0.0,
        }
    }
}

/// `WalkNodeEvaluator.getPathTypeFromState` by block name: blocks the
/// authored shape list gives a full cube are not pathfindable, water is
/// water, and the named families follow vanilla's classes and tags.
pub fn path_type_of_block(block: Option<&Block>) -> PathType {
    let Some(block) = block else { return PathType::Open };
    let id = block.id.rsplit(':').next().unwrap_or(&block.id);
    if matches!(id, "air" | "cave_air" | "void_air") {
        return PathType::Open;
    }
    if id.ends_with("_trapdoor") || id == "lily_pad" || id == "big_dripleaf" {
        return PathType::Trapdoor;
    }
    match id {
        "powder_snow" => return PathType::PowderSnow,
        "cactus" | "sweet_berry_bush" => return PathType::Damaging,
        "honey_block" => return PathType::StickyHoney,
        "cocoa" => return PathType::Cocoa,
        "wither_rose" | "pointed_dripstone" | "sulfur_spike" => return PathType::DamageCautious,
        "lava" => return PathType::Lava,
        _ => {}
    }
    let lit = block.property("lit") == Some("true");
    if matches!(id, "fire" | "soul_fire" | "magma_block" | "lava_cauldron") || (matches!(id, "campfire" | "soul_campfire") && lit) {
        return PathType::Fire;
    }
    if id.ends_with("_door") {
        return if block.property("open") == Some("true") {
            PathType::DoorOpen
        } else if id == "iron_door" {
            PathType::DoorIronClosed
        } else {
            PathType::DoorWoodClosed
        };
    }
    if matches!(id, "rail" | "powered_rail" | "detector_rail" | "activator_rail") {
        return PathType::Rail;
    }
    if id.ends_with("_leaves") {
        return PathType::Leaves;
    }
    if id.ends_with("_fence") || id.ends_with("_wall") || (id.ends_with("_fence_gate") && block.property("open") != Some("true")) {
        return PathType::Fence;
    }
    let full = authored_collision_boxes(block) == [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]];
    if full {
        PathType::Blocked
    } else if id == "water" || block.property("waterlogged") == Some("true") {
        PathType::Water
    } else {
        PathType::Open
    }
}
