//! Overworld loading screen and shader warm-up.
//!
//! The world itself is built before the window opens
//! ([`overworld::Runtime::pregenerate`]). What is left is GPU work: the
//! first frames upload every section mesh and the block atlas, and Bevy
//! compiles a render pipeline the first time each material/mesh layout is
//! drawn. This screen covers those frames. Behind it, one small mesh per
//! material is drawn so pipelines that normally appear only later (the
//! block-breaking cracks, translucent water, particle and cloud materials)
//! are compiled now instead of hitching mid-game. The screen lifts once the
//! render world reports no pipelines waiting.

use crate::overworld_client::VoxelMaterial;
use crate::player::PlayerCamera;
use crate::LoadedLevel;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy::render::render_resource::PipelineCache;
use bevy::render::{Render, RenderApp, RenderSystems};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Frames the screen is shown before heavy uploads start, so it is on
/// screen while they run.
const SHOW_FRAMES: u32 = 3;
/// Consecutive frames with no pipeline waiting before play starts.
const QUIET_FRAMES: u32 = 10;
/// Never hold the player longer than this.
const TIMEOUT_SECS: f32 = 60.0;

/// Pipelines the render world is still compiling (written there, read here).
#[derive(Resource, Clone, Default)]
pub struct PendingPipelines(Arc<AtomicUsize>);

#[derive(Resource)]
pub struct Loading {
    pub active: bool,
    frames: u32,
    quiet: u32,
    started: Option<Instant>,
    root: Option<Entity>,
    text: Option<Entity>,
    warmups: Vec<Entity>,
}

impl Default for Loading {
    fn default() -> Self {
        Self { active: true, frames: 0, quiet: 0, started: None, root: None, text: None, warmups: Vec::new() }
    }
}

impl Loading {
    /// True while the renderer should hold back heavy uploads so the
    /// loading screen gets presented first.
    pub fn holding(&self) -> bool {
        self.active && self.frames < SHOW_FRAMES
    }
}

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        let pending = PendingPipelines::default();
        app.insert_resource(pending.clone()).init_resource::<Loading>().add_systems(Update, update);
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.insert_resource(pending).add_systems(Render, count_pending.in_set(RenderSystems::Cleanup));
        }
    }
}

fn count_pending(cache: Res<PipelineCache>, pending: Res<PendingPipelines>) {
    pending.0.store(cache.waiting_pipelines().count(), Ordering::Relaxed);
}

#[allow(clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    mut loading: ResMut<Loading>,
    level: Option<Res<LoadedLevel>>,
    pending: Res<PendingPipelines>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut voxel: ResMut<Assets<VoxelMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    cameras: Query<(&GlobalTransform, &Camera), With<PlayerCamera>>,
    mut texts: Query<&mut Text>,
) {
    if !loading.active {
        return;
    }
    let Some(level) = level else { return };
    let Some(runtime) = &level.0.overworld else {
        // Only the Minecraft world streams; CS:GO maps load up front.
        loading.active = false;
        return;
    };
    let started = *loading.started.get_or_insert_with(Instant::now);
    loading.frames += 1;
    if loading.root.is_none() {
        let mut text = None;
        let root = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    row_gap: px(12.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.13, 0.10, 0.08)),
                GlobalZIndex(i32::MAX - 1),
            ))
            .with_children(|c| {
                c.spawn((Text::new("Loading terrain"), TextFont { font_size: FontSize::Px(30.0), ..default() }, TextColor(Color::WHITE)));
                text = Some(
                    c.spawn((Text::new(""), TextFont { font_size: FontSize::Px(18.0), ..default() }, TextColor(Color::srgb(0.75, 0.75, 0.75))))
                        .id(),
                );
            })
            .id();
        loading.root = Some(root);
        loading.text = text;
    }

    // Once the screen is up and the world's materials exist, draw one small
    // mesh per material in front of the camera (hidden by the screen).
    if loading.frames == SHOW_FRAMES + 2 && loading.warmups.is_empty() {
        if let Some((cam, _)) = cameras.iter().find(|(_, c)| c.order == 1).or_else(|| cameras.iter().next()) {
            let ahead = cam.translation() + cam.forward().as_vec3() * 60.0;
            let voxel_mesh = meshes.add(crate::overworld_client::warmup_mesh());
            let plain_mesh = meshes.add(Rectangle::new(4.0, 4.0));
            let voxel_ids: Vec<_> = voxel.ids().collect();
            let voxel_handles: Vec<_> = voxel_ids.into_iter().filter_map(|id| voxel.get_strong_handle(id)).collect();
            debug!("warming {} voxel and {} standard materials", voxel_handles.len(), standard.len());
            for (n, handle) in voxel_handles.into_iter().enumerate() {
                let e = commands
                    .spawn((
                        Mesh3d(voxel_mesh.clone()),
                        MeshMaterial3d(handle),
                        Transform::from_translation(ahead + Vec3::X * n as f32),
                        NoFrustumCulling,
                    ))
                    .id();
                loading.warmups.push(e);
            }
            let standard_ids: Vec<_> = standard.ids().collect();
            let standard_handles: Vec<_> = standard_ids.into_iter().filter_map(|id| standard.get_strong_handle(id)).collect();
            for (n, handle) in standard_handles.into_iter().enumerate() {
                let e = commands
                    .spawn((
                        Mesh3d(plain_mesh.clone()),
                        MeshMaterial3d(handle),
                        Transform::from_translation(ahead - Vec3::X * n as f32),
                        NoFrustumCulling,
                    ))
                    .id();
                loading.warmups.push(e);
            }
        }
    }

    let waiting = pending.0.load(Ordering::Relaxed);
    let uploads_left = runtime.lock().map(|r| r.preloaded_sections()).unwrap_or(0);
    let ready = loading.frames > SHOW_FRAMES + 4 && waiting == 0 && uploads_left == 0;
    loading.quiet = if ready { loading.quiet + 1 } else { 0 };
    if let Some(mut t) = loading.text.and_then(|e| texts.get_mut(e).ok()) {
        t.0 = if uploads_left > 0 || loading.frames <= SHOW_FRAMES {
            "Building terrain".into()
        } else if waiting > 0 {
            format!("Preparing shaders ({waiting} left)")
        } else {
            "Almost there".into()
        };
    }
    let timed_out = started.elapsed().as_secs_f32() > TIMEOUT_SECS;
    if loading.quiet >= QUIET_FRAMES || timed_out {
        loading.active = false;
        let warmed = loading.warmups.len();
        let root = loading.root.take();
        for e in std::mem::take(&mut loading.warmups).into_iter().chain(root) {
            commands.entity(e).despawn();
        }
        info!("loading screen done after {} frames, {:.2}s; {warmed} materials warmed{}", loading.frames, started.elapsed().as_secs_f32(), if timed_out { " (timed out)" } else { "" });
    }
}
