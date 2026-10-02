//! Bounded impact/blood effects and damage feedback from simulation events.
use crate::{
    coords::to_bevy,
    effects::Effects,
    player::{LocalPlayer, SimWorld},
};
use bevy::prelude::*;

#[derive(Resource)]
pub struct FeedbackAssets {
    disc: Handle<Mesh>,
    quad: Handle<Mesh>,
    blood: Handle<StandardMaterial>,
    hole: Handle<StandardMaterial>,
    dust: Handle<StandardMaterial>,
}
#[derive(Component)]
pub struct ImpactFx {
    born: f32,
    life: f32,
    velocity: Vec3,
    billboard: bool,
}
#[derive(Component)]
pub struct HurtEdge(pub u8);
#[derive(Resource, Default)]
pub struct Hurt {
    time: Option<f32>,
    direction: glam::Vec3,
    amount: f32,
}

pub fn setup(
    mut commands: Commands,
    local: Res<LocalPlayer>,
    level: Res<crate::LoadedLevel>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let blood = level
        .0
        .vfs
        .read("materials/decals/blood1.vtf")
        .and_then(|bytes| vtf::Vtf::parse(&bytes).ok())
        .and_then(|v| {
            let mut rgba = v.rgba8(0, 0, 0)?;
            for pixel in rgba.chunks_exact_mut(4) {
                multiply_decal_to_alpha(pixel);
            }
            use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
            let mut image = Image::new(
                Extent3d {
                    width: v.width as u32,
                    height: v.height as u32,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                rgba,
                TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::RENDER_WORLD,
            );
            image.sampler = bevy::image::ImageSampler::linear();
            Some(images.add(image))
        });
    let assets = FeedbackAssets {
        disc: meshes.add(Circle::new(1.0)),
        quad: meshes.add(Rectangle::new(1.0, 1.0)),
        blood: materials.add(StandardMaterial {
            base_color: Color::srgba(0.28, 0.008, 0.005, 0.88),
            base_color_texture: blood,
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        }),
        hole: materials.add(StandardMaterial {
            base_color: Color::srgba(0.035, 0.026, 0.018, 0.92),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        }),
        dust: materials.add(StandardMaterial {
            base_color: Color::srgba(0.66, 0.59, 0.44, 0.65),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        }),
    };
    // Queue these pipeline/material variants during startup so the first
    // short-lived hit puff is not lost to asynchronous shader compilation.
    let (forward, _, _) = movement::angle_vectors(glam::Vec3::new(local.pitch, local.yaw, 0.0));
    let pos = to_bevy(local.curr_eye + forward * 16.0);
    for (blood, moving) in [(true, true), (false, true), (false, false)] {
        spawn(
            &mut commands,
            &assets,
            pos,
            -to_bevy(forward),
            0.001,
            blood,
            moving,
            Vec3::ZERO,
            0.0,
            2.0,
        );
    }
    commands.insert_resource(assets);
    commands.init_resource::<Hurt>();
    for side in 0..4 {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            GlobalZIndex(8),
            BackgroundColor(Color::NONE),
            HurtEdge(side),
        ));
    }
}

fn spawn(
    commands: &mut Commands,
    assets: &FeedbackAssets,
    pos: Vec3,
    normal: Vec3,
    size: f32,
    blood: bool,
    moving: bool,
    velocity: Vec3,
    now: f32,
    life: f32,
) {
    let normal = normal.normalize_or_zero();
    if normal.length_squared() < 0.5 || !pos.is_finite() {
        return;
    }
    commands.spawn((
        Mesh3d(if blood {
            assets.quad.clone()
        } else {
            assets.disc.clone()
        }),
        MeshMaterial3d(if blood {
            assets.blood.clone()
        } else if moving {
            assets.dust.clone()
        } else {
            assets.hole.clone()
        }),
        Transform::from_translation(pos + normal * 0.12)
            .with_rotation(Quat::from_rotation_arc(Vec3::Z, normal))
            .with_scale(Vec3::splat(size)),
        ImpactFx {
            born: now,
            life,
            velocity,
            billboard: moving,
        },
    ));
}

// DecalModulate uses neutral 128 grey; DXT compression shifts it by a few
// levels. Discard that neutral fringe so overlapping puffs have no square.
// Convert the coverage in memory to an alpha mask for the particle material.
fn multiply_decal_to_alpha(pixel: &mut [u8]) {
    let darkest = pixel[0].min(pixel[1]).min(pixel[2]);
    let coverage = (120u16.saturating_sub(darkest as u16) * 255 / 120) as u8;
    let alpha = (coverage as u16 * pixel[3] as u16 / 255) as u8;
    pixel.copy_from_slice(&[255, 255, 255, alpha]);
}

pub fn impact_sound(surface: &str) -> &'static str {
    let name = surface.to_ascii_lowercase();
    if name.contains("metal") {
        "SolidMetal.BulletImpact"
    } else if name.contains("wood") || name.contains("crate") {
        "Wood.BulletImpact"
    } else if name.contains("glass") {
        "Glass.BulletImpact"
    } else {
        "Concrete.BulletImpact"
    }
}

/// Shared by CS player damage and accepted Minecraft mob hits.
#[allow(clippy::too_many_arguments)]
fn blood_hit(
    commands: &mut Commands,
    assets: &FeedbackAssets,
    collision: &collision::CollisionWorld,
    pos: glam::Vec3,
    direction: glam::Vec3,
    eye: Vec3,
    now: f32,
    enabled: bool,
    budget: &mut usize,
) {
    if !enabled || *budget < 8 {
        return;
    }
    *budget -= 8;
    for i in 0..6 {
        let phase = i as f32 * 2.4;
        let drift = to_bevy(direction) * 40.0
            + Vec3::new(
                phase.cos() * 24.0,
                10.0 + phase.sin() * 18.0,
                phase.sin() * 24.0,
            );
        spawn(
            commands,
            assets,
            to_bevy(pos) + drift * 0.03,
            (eye - to_bevy(pos)).normalize_or_zero(),
            3.0 + i as f32,
            true,
            true,
            drift,
            now,
            0.22 + i as f32 * 0.025,
        );
    }
    let tr = collision.trace_ray(
        pos + direction * 3.0,
        pos + direction * 180.0,
        collision::contents::MASK_SOLID,
    );
    if tr.hit() && !tr.start_solid && !collision.is_sky(tr.surface) {
        spawn(
            commands,
            assets,
            to_bevy(tr.end),
            to_bevy(tr.normal),
            16.0,
            true,
            false,
            Vec3::ZERO,
            now,
            25.0,
        );
    }
    let floor = collision.trace_ray(
        pos,
        pos - glam::Vec3::Z * 100.0,
        collision::contents::MASK_SOLID,
    );
    if floor.hit() && !floor.start_solid {
        spawn(
            commands,
            assets,
            to_bevy(floor.end),
            to_bevy(floor.normal),
            12.0,
            true,
            false,
            Vec3::ZERO,
            now,
            25.0,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update(
    mut commands: Commands,
    mut effects: ResMut<Effects>,
    assets: Res<FeedbackAssets>,
    mut hurt: ResMut<Hurt>,
    settings: Res<crate::settings::Settings>,
    world: Res<SimWorld>,
    local: Res<LocalPlayer>,
    level: Res<crate::LoadedLevel>,
    time: Res<Time>,
    mut particles: Query<(Entity, &mut Transform, &mut ImpactFx)>,
    mut edges: Query<(&HurtEdge, &mut Node, &mut BackgroundColor)>,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let eye = to_bevy(local.render_eye);
    let mut budget = 512usize.saturating_sub(particles.iter().count());
    let reset = effects
        .feedback
        .iter()
        .any(|e| matches!(e, sim::GameEvent::RoundStart { .. }));
    if reset {
        for (e, _, _) in &particles {
            commands.entity(e).despawn();
        }
        hurt.time = None;
        budget = 512;
    }
    for event in effects.feedback.drain(..) {
        match event {
            sim::GameEvent::RoundStart { .. } => {}
            sim::GameEvent::Impact {
                pos,
                normal,
                surface,
            } => {
                if budget < 4 || level.0.collision.is_sky(surface) {
                    continue;
                }
                budget -= 4;
                spawn(
                    &mut commands,
                    &assets,
                    to_bevy(pos),
                    to_bevy(normal),
                    1.7,
                    false,
                    false,
                    Vec3::ZERO,
                    now,
                    30.0,
                );
                for i in 0..3 {
                    let drift = to_bevy(normal) * 25.0
                        + Vec3::new(i as f32 * 9.0 - 9.0, 8.0, 5.0 - i as f32 * 5.0);
                    spawn(
                        &mut commands,
                        &assets,
                        to_bevy(pos),
                        to_bevy(normal),
                        1.0 + i as f32 * 0.4,
                        false,
                        true,
                        drift,
                        now,
                        0.2,
                    );
                }
            }
            sim::GameEvent::Damage {
                attacker,
                victim,
                health,
                pos,
                ..
            } => {
                let origin = world
                    .0
                    .player(attacker)
                    .map_or(pos - glam::Vec3::X, |p| p.eye());
                let direction = (pos - origin).normalize_or_zero();
                if victim == local.view_target() {
                    hurt.time = Some(now);
                    hurt.direction = -direction;
                    hurt.amount = (health as f32 / 45.0).clamp(0.35, 1.0);
                }
                blood_hit(
                    &mut commands,
                    &assets,
                    &level.0.collision,
                    pos,
                    direction,
                    eye,
                    now,
                    settings.blood,
                    &mut budget,
                );
            }
            _ => {}
        }
    }
    if let Some(runtime) = &level.0.overworld {
        let hits = std::mem::take(&mut runtime.lock().unwrap().mob_hits);
        for hit in hits {
            effects.last_hit = now;
            effects.last_hit_headshot = false;
            blood_hit(
                &mut commands,
                &assets,
                &level.0.collision,
                hit.point,
                (hit.point - hit.from).normalize_or_zero(),
                eye,
                now,
                settings.blood,
                &mut budget,
            );
        }
    }
    if !reset {
        for (e, mut transform, mut fx) in &mut particles {
            if now - fx.born > fx.life {
                commands.entity(e).despawn();
                continue;
            }
            if fx.billboard {
                transform.translation += fx.velocity * dt;
                fx.velocity.y -= 90.0 * dt;
                transform.look_at(eye, Vec3::Y);
            }
        }
    }
    let strength = hurt
        .time
        .map_or(0.0, |t| (1.0 - (now - t) / 0.65).clamp(0.0, 1.0))
        * hurt.amount;
    let (f, r, _) = movement::angle_vectors(glam::Vec3::new(0.0, local.render_angles.y, 0.0));
    let d = hurt.direction;
    let weights = [
        d.dot(f).max(0.0),
        (-d.dot(f)).max(0.0),
        (-d.dot(r)).max(0.0),
        d.dot(r).max(0.0),
    ];
    for (edge, mut node, mut color) in &mut edges {
        *node = Node {
            position_type: PositionType::Absolute,
            ..default()
        };
        match edge.0 {
            0 => {
                node.top = px(0.0);
                node.width = percent(100);
                node.height = px(18.0);
            }
            1 => {
                node.bottom = px(0.0);
                node.width = percent(100);
                node.height = px(18.0);
            }
            2 => {
                node.left = px(0.0);
                node.width = px(18.0);
                node.height = percent(100);
            }
            _ => {
                node.right = px(0.0);
                node.width = px(18.0);
                node.height = percent(100);
            }
        }
        color.0 = Color::srgba(
            0.65,
            0.025,
            0.015,
            strength * (0.08 + weights[edge.0 as usize] * 0.65),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surface_sounds_match_material_family() {
        assert_eq!(
            impact_sound("de_dust/metal_door"),
            "SolidMetal.BulletImpact"
        );
        assert_eq!(impact_sound("props/wood_crate"), "Wood.BulletImpact");
        assert_eq!(impact_sound("unknown"), "Concrete.BulletImpact");
    }
}

#[cfg(test)]
mod decal_tests {
    use super::*;
    #[test]
    fn multiply_white_is_transparent_and_dark_is_covered() {
        let mut white = [255, 255, 255, 255];
        multiply_decal_to_alpha(&mut white);
        assert_eq!(white[3], 0);
        let mut dark = [10, 0, 0, 255];
        multiply_decal_to_alpha(&mut dark);
        assert_eq!(dark, [255, 255, 255, 255]);
        let mut neutral = [128, 128, 128, 255];
        multiply_decal_to_alpha(&mut neutral);
        assert_eq!(neutral[3], 0);
        let mut grey = [60, 60, 60, 128];
        multiply_decal_to_alpha(&mut grey);
        assert_eq!(grey[3], 63);
    }
}

#[cfg(test)]
mod mob_blood_tests {
    use super::*;
    #[test]
    fn shared_blood_respects_setting_budget_and_short_lifetime() {
        let mut world = World::new();
        let assets = FeedbackAssets {
            disc: default(),
            quad: default(),
            blood: default(),
            hole: default(),
            dust: default(),
        };
        let collision = collision::CollisionWorld::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut budget = 512;
        {
            let mut commands = Commands::new(&mut queue, &world);
            blood_hit(
                &mut commands,
                &assets,
                &collision,
                glam::Vec3::ZERO,
                glam::Vec3::X,
                Vec3::Z * 10.,
                1.,
                false,
                &mut budget,
            );
        }
        queue.apply(&mut world);
        assert_eq!(world.query::<&ImpactFx>().iter(&world).count(), 0);
        assert_eq!(budget, 512);
        {
            let mut commands = Commands::new(&mut queue, &world);
            for _ in 0..100 {
                blood_hit(
                    &mut commands,
                    &assets,
                    &collision,
                    glam::Vec3::ZERO,
                    glam::Vec3::X,
                    Vec3::Z * 10.,
                    1.,
                    true,
                    &mut budget,
                );
            }
        }
        queue.apply(&mut world);
        assert_eq!(budget, 0);
        assert_eq!(world.query::<&ImpactFx>().iter(&world).count(), 384);
        assert!(world
            .query::<&ImpactFx>()
            .iter(&world)
            .all(|fx| fx.billboard && fx.life < 0.5));
    }
}
