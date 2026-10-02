//! `HumanoidArmorLayer` for humanoid mobs: each worn armour piece drawn on
//! the humanoid armour meshes (`HumanoidModel.createArmorMeshSet`: the
//! outer set inflated by 1.0 for the head, chest and feet, the inner by
//! 0.5 for the legs; armour legs a tenth thinner), posed as the wearer's
//! own parts, with its equipment asset's `humanoid` or `humanoid_leggings`
//! layers (`EquipmentLayerRenderer`; leather dyed its undyed colour, then
//! its overlay). Trims and enchantment glint are not drawn yet.
use crate::{
    cow_render::cube_scaled,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
};
use glam::{DVec3, Quat, Vec3};

/// The armour materials' equipment assets with humanoid textures.
pub const MATERIALS: [&str; 8] = ["leather", "chainmail", "iron", "gold", "diamond", "netherite", "copper", "turtle_scute"];

/// An armour item's equipment asset and the slot it is worn in (0 head,
/// 1 chest, 2 legs, 3 feet), as its `equippable` component names them.
pub fn armor_asset(item: &str) -> Option<(&'static str, usize)> {
    let name = item.strip_prefix("minecraft:")?;
    if name == "turtle_helmet" {
        return Some(("turtle_scute", 0));
    }
    let (material, piece) = name.rsplit_once('_')?;
    let slot = match piece {
        "helmet" => 0,
        "chestplate" => 1,
        "leggings" => 2,
        "boots" => 3,
        _ => return None,
    };
    let asset = match material {
        "leather" => "leather",
        "chainmail" => "chainmail",
        "iron" => "iron",
        "golden" => "gold",
        "diamond" => "diamond",
        "netherite" => "netherite",
        "copper" => "copper",
        _ => return None,
    };
    Some((asset, slot))
}

/// The wearer's humanoid parts: each part's pivot (model pixels) and
/// rotation — head, body, right arm, left arm, right leg, left leg.
#[derive(Clone, Copy, Debug)]
pub struct HumanoidParts {
    pub head: (Vec3, Quat),
    pub body: (Vec3, Quat),
    pub arms: [(Vec3, Quat); 2],
    pub legs: [(Vec3, Quat); 2],
}

/// `color_when_undyed` of leather armour (`-6265536`).
const UNDYED_LEATHER: [f32; 3] = [160.0 / 255.0, 101.0 / 255.0, 64.0 / 255.0];

/// Draws the armour in `armor` (head, chest, legs, feet item IDs) on a
/// humanoid posed by `parts`, in the wearer's frame (`feet`, `rotation`,
/// its model `scale`), lit like it.
#[allow(clippy::too_many_arguments)]
pub fn append_humanoid_armor(mesh: &mut ChunkMesh, atlas: &Atlas, feet: DVec3, rotation: Quat, scale: f32, parts: &HumanoidParts, armor: &[Option<String>; 4], sky: f32, block: f32) {
    for (slot, item) in armor.iter().enumerate() {
        let Some((asset, worn)) = item.as_deref().and_then(armor_asset) else { continue };
        // `shouldRender`: only in the slot it is made for.
        if worn != slot {
            continue;
        }
        let layer = if slot == 2 { "humanoid_leggings" } else { "humanoid" };
        let g = if slot == 2 { 0.5 } else { 1.0 };
        let textures: &[(&str, Option<[f32; 3]>)] = if asset == "leather" { &[("leather", Some(UNDYED_LEATHER)), ("leather_overlay", None)] } else { &[(asset, None)] };
        for &(texture, tint) in textures {
            let Ok(id) = ResourceId::parse(&format!("minecraft:entity/equipment/{layer}/{texture}")) else { continue };
            if !atlas.contains(&id) {
                continue;
            }
            let region = atlas.entity_region(&id);
            let mut cube = |from: [f32; 3], size: [f32; 3], grow: f32, uv: [f32; 2], (pivot, pose): (Vec3, Quat), mirror: bool| {
                let lo = [from[0] - grow, from[1] - grow, from[2] - grow];
                let hi = [from[0] + size[0] + grow, from[1] + size[1] + grow, from[2] + size[2] + grow];
                cube_scaled(mesh, feet, rotation, Vec3::splat(scale), region, sky, block, lo, hi, uv, pivot.to_array(), pose, tint.unwrap_or([1.0; 3]), [64.0, 32.0], Some(size), mirror);
            };
            match slot {
                0 => {
                    cube([-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], g, [0.0, 0.0], parts.head, false);
                    cube([-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], g + 0.5, [32.0, 0.0], parts.head, false);
                }
                1 => {
                    cube([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], g, [16.0, 16.0], parts.body, false);
                    cube([-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], g, [40.0, 16.0], parts.arms[0], false);
                    cube([-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], g, [40.0, 16.0], parts.arms[1], true);
                }
                _ => {
                    if slot == 2 {
                        cube([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], g, [16.0, 16.0], parts.body, false);
                    }
                    cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], g - 0.1, [0.0, 16.0], parts.legs[0], false);
                    cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], g - 0.1, [0.0, 16.0], parts.legs[1], true);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn armour_items_name_their_asset_and_slot() {
        assert_eq!(armor_asset("minecraft:golden_chestplate"), Some(("gold", 1)));
        assert_eq!(armor_asset("minecraft:turtle_helmet"), Some(("turtle_scute", 0)));
        assert_eq!(armor_asset("minecraft:iron_boots"), Some(("iron", 3)));
        assert_eq!(armor_asset("minecraft:iron_sword"), None);
    }
}
