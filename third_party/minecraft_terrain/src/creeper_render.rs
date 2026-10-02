//! Pack-backed pinned 26.3 creeper: `CreeperModel.createBodyLayer` (a head
//! and body on four legs), its `setupAnim` (head turn and pitch, the legs
//! swinging in diagonal pairs with the walk), and `CreeperRenderer`'s
//! swell (`scale`: wider and slightly taller, with a wobble) and white fuse
//! flash (`getWhiteOverlayProgress`). Vanilla mixes the white overlay into
//! the texture; here the vertex colour brightens it, as for primed TNT.
//! The charged creeper's energy layer is not drawn yet.
use crate::{
    cow_render::cube_scaled,
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
    client_mobs::ClientMobs,
};
use glam::{Quat, Vec3};
use minecraftoss_entities::world::CreeperEntity;

type Part = ([f32; 3], [f32; 3], [f32; 2], [f32; 3]);

const HEAD: Part = ([-4., -8., -4.], [4., 0., 4.], [0., 0.], [0., 6., 0.]);
const BODY: Part = ([-4., 0., -2.], [4., 12., 2.], [16., 16.], [0., 6., 0.]);
/// The legs' pivots, each with the phase of its swing: the model's
/// "right_hind_leg" and "left_front_leg" pair up, as do the other two.
const LEGS: [([f32; 3], f32); 4] = [
    ([-2., 18., 4.], std::f32::consts::PI),
    ([2., 18., 4.], 0.0),
    ([-2., 18., -4.], 0.0),
    ([2., 18., -4.], std::f32::consts::PI),
];

/// `CreeperRenderer.scale`: the swell widens the creeper by up to 40%
/// and heightens it by up to 10%, wobbling as it goes.
pub fn swell_scale(swelling: f32) -> Vec3 {
    let wobble = 1.0 + (swelling * 100.0).sin() * swelling * 0.01;
    let g = swelling.clamp(0.0, 1.0);
    let g = g * g;
    let g = g * g;
    let s = (1.0 + g * 0.4) * wobble;
    let hs = (1.0 + g * 0.1) / wobble;
    Vec3::new(s, hs, s)
}

/// `CreeperRenderer.getWhiteOverlayProgress`: white on alternate tenths of
/// the swell, from half strength.
pub fn white_overlay(swelling: f32) -> f32 {
    if (swelling * 10.0) as i32 % 2 == 0 {
        0.0
    } else {
        swelling.clamp(0.5, 1.0)
    }
}

pub fn append_creepers<'a>(
    mesh: &mut ChunkMesh,
    creepers: impl IntoIterator<Item = &'a CreeperEntity>,
    poses: &ClientMobs,
    atlas: &Atlas,
    light: &SkyLight,
    partial: f32,
) {
    let texture = ResourceId::parse("minecraft:entity/creeper/creeper").unwrap();
    let region = atlas.entity_region(&texture);
    let partial = partial.clamp(0.0, 1.0);
    // Each mob's first vertex and overlay (`getOverlayCoords`).
    let mut marks = Vec::new();
    for entity in creepers {
        let creeper = &entity.creeper;
        if creeper.exploded {
            continue;
        }
        let Some(mob) = poses.pose(entity.id, partial) else { continue };
        let feet = mob.feet;
        let sample = mob.light_block();
        let (sky, block) = (light.get(sample) as f32, light.get_block(sample) as f32);
        let rotation = mob.body_rotation(90.0);
        let head = Quat::from_euler(glam::EulerRot::ZYX, 0.0, mob.head_yaw.to_radians(), mob.head_pitch.to_radians());
        // `Creeper.getSwelling`: the fuse between ticks over two short of
        // its length.
        let swell = creeper.old_swell as f32 + partial * (creeper.swell - creeper.old_swell) as f32;
        let swelling = swell / (creeper.max_swell - 2) as f32;
        let scale = swell_scale(swelling);
        // The fuse's white flash is the overlay's white rows.
        marks.push((mesh.vertices.len(), mob.overlay(white_overlay(swelling))));
        let tint = [1.0; 3];
        let (swing, speed) = (mob.walk_position, mob.walk_speed);
        let mut parts: Vec<(Part, Quat)> = vec![(HEAD, head), (BODY, Quat::IDENTITY)];
        for (pivot, phase) in LEGS {
            let angle = crate::client_mobs::mth_cos(swing * 0.6662 + phase) * 1.4 * speed;
            parts.push((([-2., 0., -2.], [2., 6., 2.], [0., 16.], pivot), Quat::from_rotation_x(angle)));
        }
        for ((from, to, uv, pivot), pose) in parts {
            cube_scaled(mesh, feet, rotation, scale, region, sky, block, from, to, uv, pivot, pose, tint, [64., 32.], None, false);
        }
    }
    crate::cow_render::apply_overlays(mesh, &marks);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fuse_swells_and_flashes() {
        assert_eq!(swell_scale(0.0), Vec3::ONE);
        let full = swell_scale(1.0);
        assert!(full.x > 1.35 && full.y > 1.05, "{full:?}");
        assert_eq!(white_overlay(0.05), 0.0);
        assert_eq!(white_overlay(0.15), 0.5);
        assert_eq!(white_overlay(0.95), 0.95);
    }
}
