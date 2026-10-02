//! Pack-backed pinned 26.3 slime: `SlimeModel.createInnerBodyLayer` (a 6³
//! core with two eyes and a mouth, drawn cutout) under `SlimeOuterLayer`'s
//! 8³ shell (`createOuterBodyLayer`, `RenderTypes.entityTranslucent`, drawn
//! after the core), both from `entity/slime/slime` and sized by
//! `AbstractCubeMobRenderer.applySizeAndSquish` after `SlimeRenderer`'s
//! slight downscale: squashed wide on landing, stretched tall on take-off.
//! The death tip-over and hurt tint are not drawn.
use crate::{
    client_mobs::ClientMobs,
    cow_render::cube_scaled,
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
};
use glam::{Quat, Vec3};
use minecraftoss_entities::world::SlimeEntity;

type Part = ([f32; 3], [f32; 3], [f32; 2]);

/// The core, right eye, left eye and mouth.
const INNER: [Part; 4] = [
    ([-3., 17., -3.], [3., 23., 3.], [0., 16.]),
    ([-3.25, 18., -3.5], [-1.25, 20., -1.5], [32., 0.]),
    ([1.25, 18., -3.5], [3.25, 20., -1.5], [32., 4.]),
    ([0., 21., -3.5], [1., 22., -2.5], [32., 8.]),
];
const OUTER: Part = ([-4., 16., -4.], [4., 24., 4.], [0., 0.]);

/// `SlimeRenderer.scale`: `downscaleSlightly` (0.999), then
/// `applySizeAndSquish`, whose squash `w` shrinks with the size.
pub fn size_and_squish(size: i32, squish: f32) -> Vec3 {
    let size = size as f32;
    let ss = squish / (size * 0.5 + 1.0);
    let w = 1.0 / (ss + 1.0);
    Vec3::new(w * size, 1.0 / w * size, w * size) * 0.999
}

pub fn append_slimes<'a>(
    mesh: &mut ChunkMesh,
    translucent: &mut ChunkMesh,
    slimes: impl IntoIterator<Item = &'a SlimeEntity>,
    poses: &ClientMobs,
    atlas: &Atlas,
    light: &SkyLight,
    partial: f32,
) {
    let region = atlas.entity_region(&ResourceId::parse("minecraft:entity/slime/slime").unwrap());
    let partial = partial.clamp(0.0, 1.0);
    // Each mob's first vertex and overlay (`getOverlayCoords`).
    let mut marks = Vec::new();
    for entity in slimes {
        let slime = &entity.slime;
        let Some(mob) = poses.pose(entity.id, partial) else { continue };
        marks.push((mesh.vertices.len(), mob.overlay(0.0)));
        let feet = mob.feet;
        let sample = mob.light_block();
        let (sky, block) = (light.get(sample) as f32, light.get_block(sample) as f32);
        let rotation = mob.body_rotation(90.0);
        // `extractRenderState`: the squash between ticks.
        let squish = slime.previous_squish + (slime.squish - slime.previous_squish) * partial;
        let scale = size_and_squish(slime.size, squish);
        for (from, to, uv) in INNER {
            cube_scaled(mesh, feet, rotation, scale, region, sky, block, from, to, uv, [0.0; 3], Quat::IDENTITY, [1.0; 3], [64., 32.], None, false);
        }
        let (from, to, uv) = OUTER;
        cube_scaled(translucent, feet, rotation, scale, region, sky, block, from, to, uv, [0.0; 3], Quat::IDENTITY, [1.0; 3], [64., 32.], None, false);
    }
    crate::cow_render::apply_overlays(mesh, &marks);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landing_squashes_and_leaping_stretches() {
        let rest = size_and_squish(2, 0.0);
        assert!((rest - Vec3::splat(2.0 * 0.999)).length() < 1e-6);
        // `targetSquish` -0.5 on landing: wider and flatter.
        let landed = size_and_squish(2, -0.5);
        assert!(landed.x > rest.x && landed.y < rest.y, "{landed:?}");
        // 1 on take-off: narrower and taller.
        let leaping = size_and_squish(2, 1.0);
        assert!(leaping.x < rest.x && leaping.y > rest.y, "{leaping:?}");
        // Bigger slimes squash less.
        assert!(size_and_squish(4, 1.0).y / 4.0 < leaping.y / 2.0);
    }
}
