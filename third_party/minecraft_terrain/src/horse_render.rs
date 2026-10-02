//! Pack-backed horse and donkey models. Geometry, UVs and poses are from
//! pinned 26.3 `AbstractEquineModel.createBodyMesh`/`setupAnim`,
//! `BabyHorseModel`, `DonkeyModel` and `BabyDonkeyModel`, as
//! `LayerDefinitions` bakes them (a horse's adult layer scaled by 1.1, a
//! donkey's by 0.87); `HorseRenderer` picks the coat and `HorseMarkingLayer`
//! draws the markings again over it, translucent.
use crate::{
    client_mobs::ClientMobs,
    cow_render::cube_scaled,
    lighting::SkyLight,
    mesh::{Atlas, ChunkMesh},
    pack::ResourceId,
};
use glam::{EulerRot, Quat, Vec3};
use minecraftoss_entities::{
    horse::{HorseKind, HorseState},
    world::CowEntity,
};
use std::f32::consts::PI;

/// `HorseRenderer.LOCATION_BY_VARIANT`'s coat (the variant's low byte,
/// wrapping) and `HorseMarkingLayer`'s markings (the next byte; none has
/// no texture), or the donkey's texture.
pub fn texture_ids(horse: &HorseState, baby: bool) -> (ResourceId, Option<ResourceId>) {
    let suffix = if baby { "_baby" } else { "" };
    match horse.kind {
        HorseKind::Donkey => (ResourceId::parse(&format!("minecraft:entity/horse/donkey{suffix}")).unwrap(), None),
        HorseKind::Horse => {
            const COATS: [&str; 7] = ["white", "creamy", "chestnut", "brown", "black", "gray", "darkbrown"];
            const MARKINGS: [Option<&str>; 5] = [None, Some("white"), Some("whitefield"), Some("whitedots"), Some("blackdots")];
            let coat = COATS[(horse.variant & 0xFF).rem_euclid(7) as usize];
            let markings = MARKINGS[((horse.variant & 0xFF00) >> 8).rem_euclid(5) as usize];
            (
                ResourceId::parse(&format!("minecraft:entity/horse/horse_{coat}{suffix}")).unwrap(),
                markings.map(|m| ResourceId::parse(&format!("minecraft:entity/horse/horse_markings_{m}{suffix}")).unwrap()),
            )
        }
    }
}

/// Every equine texture the renderers can ask for.
pub fn all_textures() -> Vec<ResourceId> {
    let mut out = Vec::new();
    for suffix in ["", "_baby"] {
        for name in ["horse_white", "horse_creamy", "horse_chestnut", "horse_brown", "horse_black", "horse_gray", "horse_darkbrown", "horse_markings_white", "horse_markings_whitefield", "horse_markings_whitedots", "horse_markings_blackdots", "donkey"] {
            out.push(ResourceId::parse(&format!("minecraft:entity/horse/{name}{suffix}")).unwrap());
        }
    }
    out
}

/// A model part's placement in model pixels: its pivot and rotation, its
/// parents' composed in (`ModelPart.translateAndRotate`).
#[derive(Clone, Copy)]
struct Part {
    pivot: Vec3,
    rotation: Quat,
}

impl Part {
    const ROOT: Part = Part { pivot: Vec3::ZERO, rotation: Quat::IDENTITY };

    /// A child at `offset` turned by `rotation` (x, y, z radians, applied
    /// as `rotationZYX`).
    fn child(self, offset: [f32; 3], [x, y, z]: [f32; 3]) -> Part {
        Part {
            pivot: self.pivot + self.rotation * Vec3::from_array(offset),
            rotation: self.rotation * Quat::from_euler(EulerRot::ZYX, z, y, x),
        }
    }
}

/// One cuboid: its corner, size, texture offset, `CubeDeformation` and
/// whether it is mirrored.
struct Cube {
    from: [f32; 3],
    size: [f32; 3],
    uv: [f32; 2],
    grow: f32,
    mirror: bool,
}

const fn cube(from: [f32; 3], size: [f32; 3], uv: [f32; 2]) -> Cube {
    Cube { from, size, uv, grow: 0.0, mirror: false }
}

/// What `setupAnim` reads from `EquineRenderState`.
struct Anim {
    /// The head's yaw from the body and its pitch, degrees.
    y_rot: f32,
    x_rot: f32,
    walk_speed: f32,
    walk_position: f32,
    eat: f32,
    stand: f32,
    feeding: f32,
    animate_tail: bool,
    age: f32,
    in_water: bool,
    age_scale: f32,
}

/// `AbstractEquineModel` and its baby overrides.
#[derive(Clone, Copy)]
struct Tuning {
    leg_stand_angle: f32,
    leg_standing_y: f32,
    leg_standing_z: f32,
    leg_standing_x_rot: f32,
    tail_x_rot: f32,
}

const ADULT: Tuning = Tuning { leg_stand_angle: PI / 12.0, leg_standing_y: 12.0, leg_standing_z: 4.0, leg_standing_x_rot: -PI / 3.0, tail_x_rot: 0.0 };
const BABY_HORSE: Tuning = Tuning { leg_standing_y: 4.0, leg_standing_z: 0.0, tail_x_rot: -PI / 2.0, ..ADULT };
const BABY_DONKEY: Tuning = Tuning { leg_stand_angle: PI / 3.0, leg_standing_y: 1.0, leg_standing_z: 0.5, leg_standing_x_rot: 0.0, tail_x_rot: -PI / 4.0 };

fn mth_sin(x: f32) -> f32 {
    minecraftoss_player::mth::sin(f64::from(x))
}

fn mth_cos(x: f32) -> f32 {
    crate::client_mobs::mth_cos(x)
}

fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

/// `AbstractEquineModel.setupAnim`'s shared rotations and offsets.
struct Posed {
    head_x: f32,
    head_y: f32,
    body_x: f32,
    /// Front legs' Y and Z (both sides), and each leg's X rotation: left
    /// hind, right hind, left front, right front.
    front_y: f32,
    front_z: f32,
    legs: [f32; 4],
    tail: [f32; 2],
    tail_offset: [f32; 2],
}

fn setup(anim: &Anim, tuning: Tuning, front: [f32; 2]) -> Posed {
    let clamped = anim.y_rot.clamp(-20.0, 20.0);
    let mut head_rot_x = anim.x_rot * (PI / 180.0);
    if anim.walk_speed > 0.2 {
        head_rot_x += mth_cos(anim.walk_position * 0.8) * 0.15 * anim.walk_speed;
    }
    let (eat, stand) = (anim.eat, anim.stand);
    let i_standing = 1.0 - stand;
    let head_y = clamped * (PI / 180.0);
    let water = if anim.in_water { 0.2 } else { 1.0 };
    let leg_anim = mth_cos(water * anim.walk_position * 0.6662 + PI);
    let leg_x = leg_anim * 0.8 * anim.walk_speed;
    let base_head = (1.0 - stand.max(eat)) * (PI / 6.0 + head_rot_x + anim.feeding * mth_sin(anim.age) * 0.05);
    let head_x = stand * (PI / 12.0 + head_rot_x) + eat * (2.181_661_6 + mth_sin(anim.age) * 0.05) + base_head;
    let head_y = stand * clamped * (PI / 180.0) + (1.0 - stand.max(eat)) * head_y;
    let front_y = front[0] - tuning.leg_standing_y * stand;
    let front_z = front[1] + tuning.leg_standing_z * stand;
    let stand_angle = tuning.leg_stand_angle * stand;
    let bob = mth_cos(anim.age * 0.6 + PI);
    let right = (tuning.leg_standing_x_rot + bob) * stand + leg_x * i_standing;
    let left = (tuning.leg_standing_x_rot - bob) * stand - leg_x * i_standing;
    Posed {
        head_x,
        head_y,
        body_x: stand * (-PI / 4.0),
        front_y,
        front_z,
        legs: [
            stand_angle - leg_anim * 0.5 * anim.walk_speed * i_standing,
            stand_angle + leg_anim * 0.5 * anim.walk_speed * i_standing,
            right,
            left,
        ],
        tail: [tuning.tail_x_rot + PI / 6.0 + anim.walk_speed * 0.75, if anim.animate_tail { mth_cos(anim.age * 0.7) } else { 0.0 }],
        tail_offset: [anim.walk_speed * anim.age_scale, anim.walk_speed * 2.0 * anim.age_scale],
    }
}

/// The posed model's cuboids, each with its part.
fn model(kind: HorseKind, baby: bool, chest: bool, anim: &Anim) -> Vec<(Part, Cube)> {
    let mut out = Vec::new();
    let root = Part::ROOT;
    match (kind, baby) {
        (_, false) => {
            let p = setup(anim, ADULT, [14.0, -10.0]);
            let g = 0.0;
            let body = root.child([0.0, 11.0, 5.0], [p.body_x, 0.0, 0.0]);
            out.push((body, Cube { grow: 0.05, ..cube([-5.0, -8.0, -17.0], [10.0, 10.0, 22.0], [0.0, 32.0]) }));
            let tail = body.child([0.0, -5.0 + p.tail_offset[0], 2.0 + p.tail_offset[1]], [p.tail[0], p.tail[1], 0.0]);
            out.push((tail, Cube { grow: g, ..cube([-1.5, 0.0, 0.0], [3.0, 14.0, 4.0], [42.0, 36.0]) }));
            if kind == HorseKind::Donkey && chest {
                let chest = cube([-4.0, 0.0, -2.0], [8.0, 8.0, 3.0], [26.0, 21.0]);
                out.push((body.child([6.0, -8.0, 0.0], [0.0, -PI / 2.0, 0.0]), cube(chest.from, chest.size, chest.uv)));
                out.push((body.child([-6.0, -8.0, 0.0], [0.0, PI / 2.0, 0.0]), chest));
            }
            // `animateHeadPartsPlacement`.
            let head_parts_y = 4.0 + lerp(anim.eat, lerp(anim.stand, 0.0, -8.0), 7.0);
            let head_parts_z = lerp(anim.stand, -12.0, -4.0);
            let head_parts = root.child([0.0, head_parts_y, head_parts_z], [p.head_x, p.head_y, 0.0]);
            out.push((head_parts, cube([-2.05, -6.0, -2.0], [4.0, 12.0, 7.0], [0.0, 35.0])));
            let head = head_parts.child([0.0; 3], [0.0; 3]);
            out.push((head, cube([-3.0, -11.0, -2.0], [6.0, 5.0, 7.0], [0.0, 13.0])));
            match kind {
                HorseKind::Horse => {
                    out.push((head, Cube { grow: -0.001, ..cube([0.55, -13.0, 4.0], [2.0, 3.0, 1.0], [19.0, 16.0]) }));
                    out.push((head, Cube { grow: -0.001, ..cube([-2.55, -13.0, 4.0], [2.0, 3.0, 1.0], [19.0, 16.0]) }));
                }
                HorseKind::Donkey => {
                    let ear = || cube([-1.0, -7.0, 0.0], [2.0, 7.0, 1.0], [0.0, 12.0]);
                    out.push((head.child([1.25, -10.0, 4.0], [PI / 12.0, 0.0, PI / 12.0]), ear()));
                    out.push((head.child([-1.25, -10.0, 4.0], [PI / 12.0, 0.0, -PI / 12.0]), ear()));
                }
            }
            out.push((head_parts, cube([-1.0, -11.0, 5.01], [2.0, 16.0, 2.0], [56.0, 36.0])));
            out.push((head_parts, cube([-2.0, -11.0, -7.0], [4.0, 5.0, 5.0], [0.0, 25.0])));
            let leg = |uv_mirror: bool, from: [f32; 3]| Cube { mirror: uv_mirror, ..cube(from, [4.0, 11.0, 4.0], [48.0, 21.0]) };
            out.push((root.child([4.0, 14.0, 7.0], [p.legs[0], 0.0, 0.0]), leg(true, [-3.0, -1.01, -1.0])));
            out.push((root.child([-4.0, 14.0, 7.0], [p.legs[1], 0.0, 0.0]), leg(false, [-1.0, -1.01, -1.0])));
            out.push((root.child([4.0, p.front_y, p.front_z], [p.legs[2], 0.0, 0.0]), leg(true, [-3.0, -1.01, -1.9])));
            out.push((root.child([-4.0, p.front_y, p.front_z], [p.legs[3], 0.0, 0.0]), leg(false, [-1.0, -1.01, -1.9])));
        }
        (HorseKind::Horse, true) => {
            let p = setup(anim, BABY_HORSE, [16.0, -5.4]);
            let body = root.child([0.0, 12.5, 0.0], [p.body_x, 0.0, 0.0]);
            out.push((body, cube([-4.0, -3.5, -7.0], [8.0, 7.0, 14.0], [0.0, 13.0])));
            let tail = body.child([0.0, -1.0 + p.tail_offset[0], 7.0 + p.tail_offset[1]], [p.tail[0], p.tail[1], 0.0]);
            out.push((tail, cube([-1.5, -1.5, -1.0], [3.0, 3.0, 8.0], [24.0, 34.0])));
            let leg = |uv: [f32; 2]| cube([-1.5, -1.0, -1.5], [3.0, 9.0, 3.0], uv);
            out.push((root.child([2.4, 16.0, 5.4], [p.legs[0], 0.0, 0.0]), leg([12.0, 46.0])));
            out.push((root.child([-2.4, 16.0, 5.4], [p.legs[1], 0.0, 0.0]), leg([0.0, 46.0])));
            out.push((root.child([2.4, p.front_y, p.front_z], [p.legs[2], 0.0, 0.0]), leg([12.0, 34.0])));
            out.push((root.child([-2.4, p.front_y, p.front_z], [p.legs[3], 0.0, 0.0]), leg([0.0, 34.0])));
            let neck_y = 10.0 + lerp(anim.eat, lerp(anim.stand, 0.0, -2.0), 2.0);
            let neck = root.child([0.0, neck_y, lerp(anim.stand, -6.0, -4.0)], [p.head_x, p.head_y, 0.0]);
            out.push((neck, cube([-2.0, -6.0, -2.0], [4.0, 8.0, 4.0], [30.0, 0.0])));
            let head = neck.child([0.0, -6.0516, -0.2951], [0.0; 3]);
            out.push((head, cube([-3.0, -3.9484, -6.705], [6.0, 4.0, 9.0], [0.0, 0.0])));
            out.push((head.child([2.0, -4.2484, 1.9451], [0.0, 0.0, 0.2618]), cube([-1.0, -2.5, -0.8], [2.0, 3.0, 1.0], [0.0, 4.0])));
            out.push((head.child([-2.0, -4.2484, 1.645], [0.0, 0.0, -0.2618]), cube([-1.0, -2.5, -0.5], [2.0, 3.0, 1.0], [0.0, 0.0])));
        }
        (HorseKind::Donkey, true) => {
            let mut p = setup(anim, BABY_DONKEY, [3.5, -5.3]);
            // `BabyDonkeyModel.setupAnim` pitches the head again from a
            // fixed -30 degrees, without the walk's nod.
            let head_rot_x = -30.0 * (PI / 180.0);
            let base_head = (1.0 - anim.stand.max(anim.eat)) * (PI / 6.0 + head_rot_x + anim.feeding * mth_sin(anim.age) * 0.05);
            p.head_x = anim.stand * (PI / 12.0 + head_rot_x) + anim.eat * (PI / 2.0 + mth_sin(anim.age) * 0.05) + base_head;
            let body = root.child([1.0, 14.0, 0.0], [p.body_x, 0.0, 0.0]);
            out.push((body, cube([-5.0, -3.0, -7.0], [8.0, 6.0, 14.0], [0.0, 13.0])));
            let tail = body.child([0.0, -1.5 + p.tail_offset[0], 6.5 + p.tail_offset[1]], [p.tail[0], p.tail[1], 0.0]);
            out.push((tail.child([0.0; 3], [-0.7418, 0.0, 0.0]), cube([-2.5, -1.0, -0.5], [3.0, 3.0, 8.0], [24.0, 33.0])));
            // `offsetLegPositionWhenStanding`: both hind legs from the left's.
            let hind_y = lerp(anim.stand, 3.5, -0.3);
            let hind_y = lerp(anim.stand, hind_y, -0.3);
            let leg = |uv: [f32; 2]| cube([-2.5, -1.5, -1.5], [3.0, 8.0, 3.0], uv);
            out.push((body.child([2.25, lerp(anim.stand, 3.5, -0.3), 5.25], [p.legs[0], 0.0, 0.0]), leg([12.0, 44.0])));
            out.push((body.child([-2.4, hind_y, 5.4], [p.legs[1], 0.0, 0.0]), leg([0.0, 44.0])));
            out.push((body.child([2.4, p.front_y, p.front_z], [p.legs[2], 0.0, 0.0]), leg([12.0, 33.0])));
            out.push((body.child([-2.4, p.front_y, p.front_z], [p.legs[3], 0.0, 0.0]), leg([0.0, 33.0])));
            let head_parts = body.child([0.0, lerp(anim.eat, -3.0, -1.2), lerp(anim.stand, -5.0, -3.6)], [p.head_x, p.head_y, 0.0]);
            out.push((head_parts.child([0.0; 3], [0.3927, 0.0, 0.0]), cube([-3.0, -6.0, -3.0], [4.0, 8.0, 4.0], [30.0, 9.0])));
            let head = head_parts.child([0.0, -5.0, -3.0], [0.0; 3]);
            out.push((head.child([0.0, -1.0, 1.0], [0.3927, 0.0, 0.0]), cube([-4.0, -3.6, -8.4], [6.0, 4.0, 9.0], [0.0, 0.0])));
            out.push((head.child([2.0, -3.5, -1.0], [0.48, 0.0, 0.48]), cube([-2.0, -6.5, -0.3], [2.0, 7.0, 1.0], [0.0, 0.0])));
            out.push((head.child([-2.0, -3.5, -1.0], [0.48, 0.0, -0.48]), Cube { mirror: true, ..cube([-2.0, -6.5, -0.3], [2.0, 7.0, 1.0], [22.0, 0.0]) }));
        }
    }
    out
}

/// `LayerDefinitions`' mesh scale for the adult layers.
fn layer_scale(kind: HorseKind, baby: bool) -> f32 {
    match (kind, baby) {
        (_, true) => 1.0,
        (HorseKind::Horse, false) => 1.1,
        (HorseKind::Donkey, false) => 0.87,
    }
}

pub fn append_horses<'a>(
    mesh: &mut ChunkMesh,
    translucent: &mut ChunkMesh,
    horses: impl IntoIterator<Item = &'a CowEntity>,
    poses: &ClientMobs,
    atlas: &Atlas,
    light: &SkyLight,
    partial: f32,
) {
    let mut marks = Vec::new();
    for entity in horses {
        let Some(horse) = &entity.horse else { continue };
        let Some(mob) = poses.pose(entity.id, partial) else { continue };
        marks.push((mesh.vertices.len(), mob.overlay(0.0)));
        let baby = entity.cow.age.baby();
        let t = partial.clamp(0.0, 1.0);
        let anim = Anim {
            y_rot: mob.head_yaw,
            x_rot: mob.head_pitch,
            walk_speed: mob.walk_speed,
            walk_position: mob.walk_position,
            eat: lerp(t, horse.eat_anim_o, horse.eat_anim),
            stand: lerp(t, horse.stand_anim_o, horse.stand_anim),
            feeding: lerp(t, horse.mouth_anim_o, horse.mouth_anim),
            animate_tail: horse.tail_counter > 0,
            age: mob.age_in_ticks,
            in_water: entity.cow.body.touching_water,
            age_scale: if baby { 0.5 } else { 1.0 },
        };
        let rotation = mob.body_rotation(90.0);
        let scale = Vec3::splat(layer_scale(horse.kind, baby));
        let pos = mob.light_block();
        let sky = light.get(pos) as f32;
        let block = light.get_block(pos) as f32;
        let (texture, markings) = texture_ids(horse, baby);
        let cubes = model(horse.kind, baby, horse.chest, &anim);
        let emit = |target: &mut ChunkMesh, texture: &ResourceId| {
            let region = atlas.entity_region(texture);
            for (part, c) in &cubes {
                let from = c.from.map(|v| v - c.grow);
                let to = [c.from[0] + c.size[0] + c.grow, c.from[1] + c.size[1] + c.grow, c.from[2] + c.size[2] + c.grow];
                cube_scaled(target, mob.feet, rotation, scale, region, sky, block, from, to, c.uv, part.pivot.to_array(), part.rotation, [1.0; 3], [64.0; 2], Some(c.size), c.mirror);
            }
        };
        emit(mesh, &texture);
        if let Some(markings) = markings {
            emit(translucent, &markings);
        }
    }
    crate::cow_render::apply_overlays(mesh, &marks);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn still() -> Anim {
        Anim { y_rot: 0.0, x_rot: 0.0, walk_speed: 0.0, walk_position: 0.0, eat: 0.0, stand: 0.0, feeding: 0.0, animate_tail: false, age: 0.0, in_water: false, age_scale: 1.0 }
    }

    #[test]
    fn a_horse_stands_on_its_hooves() {
        // A leg's foot at its pivot plus the cube's bottom (Y 14 - 1.01 +
        // 11), scaled about the model's feet (24.016), meets the ground.
        let cubes = model(HorseKind::Horse, false, false, &still());
        let (leg, cube) = cubes.last().unwrap();
        let foot = leg.pivot.y + cube.from[1] + cube.size[1];
        assert!((foot - 23.99).abs() < 1e-4);
        assert_eq!(layer_scale(HorseKind::Horse, false), 1.1);
    }

    #[test]
    fn grazing_lowers_the_head_and_rearing_lifts_the_body() {
        let mut anim = still();
        anim.eat = 1.0;
        let grazing = model(HorseKind::Horse, false, false, &anim);
        let (head_parts, _) = &grazing[2];
        assert_eq!(head_parts.pivot.y, 11.0, "the neck drops seven pixels");
        anim.eat = 0.0;
        anim.stand = 1.0;
        let rearing = model(HorseKind::Horse, false, false, &anim);
        let (body, _) = &rearing[0];
        assert!((body.rotation.to_euler(EulerRot::XYZ).0 + PI / 4.0).abs() < 1e-6);
    }

    #[test]
    fn coats_and_markings_come_from_the_variant() {
        let mut horse = HorseState::new(HorseKind::Horse);
        horse.variant = 257;
        let (coat, markings) = texture_ids(&horse, false);
        assert_eq!(coat, ResourceId::parse("minecraft:entity/horse/horse_creamy").unwrap());
        assert_eq!(markings, Some(ResourceId::parse("minecraft:entity/horse/horse_markings_white").unwrap()));
        let (coat, markings) = texture_ids(&HorseState::new(HorseKind::Donkey), true);
        assert_eq!(coat, ResourceId::parse("minecraft:entity/horse/donkey_baby").unwrap());
        assert!(markings.is_none());
    }
}
