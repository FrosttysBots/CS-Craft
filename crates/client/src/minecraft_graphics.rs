//! Live Minecraft video controls, conservative cloud geometry, and F3 diagnostics.
use crate::{LoadedLevel, Options, menu::Menu, player::LocalPlayer, settings::Settings};
use bevy::prelude::*;
use overworld::glam_mc;
use std::collections::VecDeque;

pub struct MinecraftGraphicsPlugin;
impl Plugin for MinecraftGraphicsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderStats>()
            .init_resource::<Clouds>()
            .add_systems(
                Update,
                (diagnostics, clouds).after(crate::player::update_camera),
            );
    }
}
#[derive(Resource, Default)]
pub(crate) struct RenderStats {
    pub sections: usize,
    pub visible_sections: usize,
    pub uploads: usize,
    pub mob_vertices: usize,
    pub terrain_ms: f32,
    pub pending_lights: usize,
    pub light_updates: usize,
    pub worker_light_ms: f32,
    pub upload_ms: f32,
    pub entities_ms: f32,
    pub debug: bool,
    samples: VecDeque<f32>,
    elapsed: f32,
    root: Option<Entity>,
    initialized: bool,
}
#[derive(Component)]
struct DebugText;
fn diagnostics(
    mut c: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    real: Res<Time<Real>>,
    settings: Res<Settings>,
    menu: Res<Menu>,
    level: Res<LoadedLevel>,
    local: Res<LocalPlayer>,
    windows: Query<&Window>,
    options: Res<Options>,
    mut stats: ResMut<RenderStats>,
    mut texts: Query<(&mut Text, &mut Node), With<DebugText>>,
) {
    let Some(runtime) = &level.0.overworld else {
        return;
    };
    if !stats.initialized {
        stats.initialized = true;
        stats.debug = options.0.screenshot.is_some() && std::env::var_os("CSR_TEST_F3").is_some();
    }
    if keys.just_pressed(KeyCode::F3) && !menu.open() {
        stats.debug = !stats.debug;
        stats.elapsed = 1.;
    }
    let dt = real.delta_secs() * 1000.;
    if dt > 0. {
        stats.samples.push_back(dt);
    }
    while stats.samples.len() > 120 {
        stats.samples.pop_front();
    }
    let shown = (stats.debug || settings.mc_show_fps) && !menu.open();
    if stats.root.is_none() {
        stats.root = Some(
            c.spawn((
                DebugText,
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(17.),
                    ..default()
                },
                TextColor(Color::WHITE),
                BackgroundColor(Color::srgba(0., 0., 0., 0.65)),
                GlobalZIndex(60),
                Node {
                    position_type: PositionType::Absolute,
                    top: px(8.),
                    left: px(8.),
                    padding: UiRect::all(px(7.)),
                    display: Display::None,
                    ..default()
                },
            ))
            .id(),
        );
    }
    stats.elapsed += real.delta_secs();
    for (mut text, mut node) in &mut texts {
        let target = if shown { Display::Flex } else { Display::None };
        if node.display != target {
            node.display = target;
        }
        if !shown || stats.elapsed < 0.25 {
            continue;
        }
        stats.elapsed = 0.;
        let mut samples: Vec<_> = stats.samples.iter().copied().collect();
        samples.sort_by(f32::total_cmp);
        let mean = samples.iter().sum::<f32>() / samples.len().max(1) as f32;
        let fps = 1000. / mean.max(0.01);
        if !stats.debug {
            text.0 = format!("{fps:.0} FPS  |  {mean:.1} ms");
            continue;
        }
        let r = runtime.lock().unwrap();
        let p = overworld::to_block(r.save.origin, local.render_eye);
        let total = r.poses.iter().count();
        let camera = overworld::terrain::sections::CullCamera {
            position: glam_mc::DVec3::from_array(p),
            forward: glam_mc::Vec3::from_array(
                crate::coords::view_forward(local.render_angles.x, local.render_angles.y)
                    .to_array(),
            ),
            fov_degrees: crate::VERTICAL_FOV_DEG,
            aspect: windows
                .iter()
                .next()
                .map_or(16. / 9., |w| w.width() / w.height()),
            yaw_degrees: local.render_angles.y,
            pitch_degrees: local.render_angles.x,
        };
        let view = overworld::rendering::EntityView::new(
            &camera,
            settings.mc_entity_distance as f32,
            settings.mc_entity_culling,
        );
        let shown = r
            .poses
            .iter()
            .filter(|(_, m)| {
                view.visible(m.pose((r.clock / 0.05).clamp(0., 1.)).feet.to_array(), 4.)
            })
            .count();
        let max = samples.last().copied().unwrap_or(0.);
        let p95 = samples
            .get(samples.len().saturating_sub(1) * 95 / 100)
            .copied()
            .unwrap_or(0.);
        text.0 = format!(
            "CSGO + Minecraft / Rust\n{fps:.0} FPS | {mean:.1} ms | p95 {p95:.1} | max {max:.1}\n\nXYZ (eye): {:.2} / {:.2} / {:.2}\nChunk: {} / {}   Dimension: {}\nRender: {} chunks | Entities: {} blocks\nSections: {} visible / {} resident | uploads: {}\nTracked mobs: {shown} visible / {total} | vertices: {}\nCPU terrain: {:.2} ms | uploads: {:.2} ms | entities: {:.2} ms\nLighting: {} pending | {} accepted | worker mean {:.2} ms\nChunk integration: {:.2} ms | {} loaded\n\nF3: close debug   O: Minecraft options",
            p[0],
            p[1],
            p[2],
            (p[0].floor() as i32).div_euclid(16),
            (p[2].floor() as i32).div_euclid(16),
            r.save.adventure.dimension,
            r.stream.view_distance(),
            settings.mc_entity_distance,
            stats.visible_sections,
            stats.sections,
            stats.uploads,
            stats.mob_vertices,
            stats.terrain_ms,
            stats.upload_ms,
            stats.entities_ms,stats.pending_lights,stats.light_updates,stats.worker_light_ms,r.stream_tick_ms,r.stream_tick_loaded
        );
    }
}

#[derive(Resource, Default)]
struct Clouds {
    mask: Option<overworld::terrain::clouds::CloudMask>,
    tried: bool,
    entity: Option<Entity>,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    key: Option<((i32, i32), i32, usize, i32, u64)>,
}
fn clouds(
    mut c: Commands,
    level: Res<LoadedLevel>,
    local: Res<LocalPlayer>,
    settings: Res<Settings>,
    mut state: ResMut<Clouds>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut transforms: Query<(&mut Transform, &mut Visibility)>,
) {
    let Some(runtime) = &level.0.overworld else {
        return;
    };
    let runtime = runtime.lock().unwrap();
    let on = settings.mc_clouds > 0 && runtime.save.adventure.dimension == "overworld";
    if !on {
        if let Some(e) = state.entity {
            if let Ok((_, mut v)) = transforms.get_mut(e) {
                *v = Visibility::Hidden;
            }
        }
        return;
    }
    if !state.tried {
        state.tried = true;
        state.mask = overworld::terrain::clouds::CloudMask::from_pack(&runtime.packs).ok();
    }
    let Some(mask) = &state.mask else {
        return;
    };
    let p = overworld::to_block(runtime.save.origin, local.render_eye);
    let ticks = runtime.save.day_ticks;
    let center = mask.center(p[0] as f32, p[2] as f32, ticks);
    // Rebuild only once per 96-block tile. The margin covers the entire tile.
    let center = (center.0.div_euclid(8) * 8, center.1.div_euclid(8) * 8);
    let band = if p[1] < 192.33 {
        0
    } else if p[1] > 196.33 {
        2
    } else {
        1
    };
    let radius = (runtime.stream.view_distance().max(8) * 16 + 11) / 12 + 8;
    let key = (center, band, settings.mc_clouds, radius, runtime.epoch);
    if state.key != Some(key) {
        let cloud = mask.build_options(center, p[1] as f32, radius, settings.mc_clouds == 1);
        let vertices: Vec<_> = cloud
            .vertices
            .iter()
            .map(overworld::terrain::mesh::SectionVertex::from_vertex)
            .collect();
        let mesh = crate::overworld_client::mesh(&vertices, cloud.indices, runtime.save.origin);
        if state.entity.is_some() {
            let _ = meshes.insert(state.mesh.id(), mesh);
        } else {
            state.mesh = meshes.add(mesh);
            state.material = materials.add(StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,
                ..default()
            });
            state.entity = Some(
                c.spawn((
                    Mesh3d(state.mesh.clone()),
                    MeshMaterial3d(state.material.clone()),
                    Transform::IDENTITY,
                    bevy::camera::visibility::NoFrustumCulling,
                ))
                .id(),
            );
        }
        state.key = Some(key);
    }
    let brightness = 0.15 + runtime.daylight() * 0.85;
    if let Some(mut m) = materials.get_mut(&state.material) {
        m.base_color = Color::linear_rgb(brightness, brightness, brightness);
    }
    if let Some(e) = state.entity {
        if let Ok((mut transform, mut v)) = transforms.get_mut(e) {
            transform.translation = Vec3::new(
                -(ticks as f32) * 0.03 * overworld::BLOCK,
                0.,
                -3.96 * overworld::BLOCK,
            );
            if *v != Visibility::Inherited {
                *v = Visibility::Inherited;
            }
        }
    }
}
