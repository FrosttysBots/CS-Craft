//! Bevy client for CSGO Rewrite: renders a map from the player's CS:GO
//! install and plays it with CS:GO movement.
//!
//! [`CsgoPlugin`] is self-contained so the whole game can be added to
//! another Bevy app (the planned multi-game "stitch").

pub mod audio;
pub mod characters;
pub mod player_anim;
pub mod skinning;
pub mod viewmodel;
pub mod coords;
pub mod effects;
pub mod hud;
pub mod material;
pub mod player;
pub mod props;
pub mod sky;
pub mod textures;
pub mod world_mesh;

use bevy::asset::embedded_asset;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::window::{PresentMode, WindowResolution};
use material::WorldMaterial;
use player::{LocalPlayer, PlayerCamera, SimWorld};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

const BOT_NAMES_T: [&str; 8] = ["Albert", "Crusher", "Dragomir", "Gunner", "Kasper", "Minh", "Rock", "Vitaliy"];
const BOT_NAMES_CT: [&str; 8] = ["Brian", "Darryl", "Elliot", "Frank", "Harold", "Jon", "Pete", "Walt"];

/// CS:GO's default 90° horizontal FOV at 4:3, as a vertical angle.
pub const VERTICAL_FOV_DEG: f32 = 73.74;

#[derive(Clone, Debug)]
pub struct ClientOptions {
    pub map: String,
    /// Take a screenshot after `frames` frames, then exit.
    pub screenshot: Option<PathBuf>,
    pub frames: u32,
    /// Start in noclip at this Source position with (pitch, yaw).
    pub camera: Option<(glam::Vec3, f32, f32)>,
    pub exposure: f32,
    pub width: u32,
    pub height: u32,
    /// Enemy bots; the local team gets one fewer so teams are even.
    pub bots: u32,
    /// Play as a Terrorist instead of a Counter-Terrorist.
    pub terrorist: bool,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            map: "de_dust2".into(),
            screenshot: None,
            frames: 120,
            camera: None,
            exposure: 2.0,
            width: 1600,
            height: 900,
            bots: 5,
            terrorist: false,
        }
    }
}

#[derive(Resource)]
pub struct LevelInfo {
    pub name: String,
}

/// The loaded level, handed to the startup system once.
#[derive(Resource)]
struct PendingLevel(Mutex<Option<level::Level>>);

#[derive(Resource)]
pub struct LoadedLevel(pub level::Level);

#[derive(Resource, Clone)]
struct Options(ClientOptions);

pub struct CsgoPlugin;

impl Plugin for CsgoPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/world.wgsl");
        app.add_plugins(MaterialPlugin::<WorldMaterial>::default())
            .insert_resource(Time::<Fixed>::from_hz(sim::TICK_RATE as f64))
            .add_systems(Startup, (spawn_level, hud::setup))
            .add_systems(
                Update,
                (player::look, player::grab_cursor, player::update_camera).chain(),
            )
            .init_resource::<effects::PendingEvents>()
            .init_resource::<effects::Effects>()
            .init_resource::<player::BotBrains>()
            .init_resource::<characters::Characters>()
            .init_resource::<viewmodel::ViewModelState>()
            .add_systems(
                Update,
                (
                    effects::process,
                    effects::draw,
                    characters::sync,
                    viewmodel::update,
                    hud::update.run_if(|| std::env::var_os("CSR_NO_HUD").is_none()),
                )
                    .chain()
                    .after(player::update_camera),
            )
            .add_systems(FixedUpdate, player::tick);
    }
}

/// Loads the map and runs the game window. Returns a process exit code.
pub fn run(opts: ClientOptions) -> i32 {
    let t = Instant::now();
    let level = match level::Level::load(&opts.map, None) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    info!("loaded {} in {:.2}s {:?}", opts.map, t.elapsed().as_secs_f32(), level.timings);

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: format!("CSGO Rewrite — {}", opts.map),
                    resolution: WindowResolution::new(opts.width, opts.height),
                    present_mode: match std::env::var("CSR_PRESENT").as_deref() {
                        Ok("immediate") => PresentMode::Immediate,
                        Ok("mailbox") => PresentMode::Mailbox,
                        Ok("fifo") => PresentMode::Fifo,
                        _ => PresentMode::AutoNoVsync,
                    },
                    ..default()
                }),
                ..default()
            }),
    )
    .add_plugins(FrameTimeDiagnosticsPlugin::default())
    .insert_resource(ClearColor(Color::srgb(0.55, 0.68, 0.85)))
    .insert_resource(LevelInfo { name: opts.map.clone() })
    .insert_resource(PendingLevel(Mutex::new(Some(level))))
    .insert_resource(Options(opts.clone()))
    .insert_resource(player::InputEnabled(opts.screenshot.is_none()))
    .add_plugins(CsgoPlugin);
    if opts.screenshot.is_some() {
        // Automated runs never have focus; don't let the unfocused 60 Hz
        // throttle skew frame-rate readings.
        app.insert_resource(bevy::winit::WinitSettings {
            focused_mode: bevy::winit::UpdateMode::Continuous,
            unfocused_mode: bevy::winit::UpdateMode::Continuous,
        })
        .add_systems(Update, smoke_test);
    }
    match app.run() {
        AppExit::Success => 0,
        AppExit::Error(code) => code.get() as i32,
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_level(
    mut commands: Commands,
    pending: Res<PendingLevel>,
    options: Res<Options>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<WorldMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut bindposes: ResMut<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
) {
    let Some(level) = pending.0.lock().unwrap().take() else { return };
    let opts = &options.0;
    let t = Instant::now();
    let mut textures = textures::TextureCache::new(&mut images);

    // World geometry, split into the playable world and the 3D skybox.
    let sky = level.bsp.sky_camera();
    let sky_faces = sky
        .map(|s| world_mesh::sky_faces(&level.bsp, &s))
        .unwrap_or_default();
    let atlas = world_mesh::LightmapAtlas::build(&level.bsp);
    let main = world_mesh::build(&level.bsp, &atlas, &|f| !sky_faces.contains(&f));
    let skybox = world_mesh::build(&level.bsp, &atlas, &|f| sky_faces.contains(&f));
    let lightmap = images.add(atlas.to_image());
    drop(atlas);
    let world_shared = world_mesh::SharedHandles {
        lightmap: lightmap.clone(),
    };
    let mut world_draws = 0;
    let mut world_materials: std::collections::HashMap<String, Handle<WorldMaterial>> = Default::default();
    for (in_sky, build) in [(false, &main), (true, &skybox)] {
        for (name, mesh) in &build.meshes {
            let mat = world_materials
                .entry(name.clone())
                .or_insert_with(|| {
                    materials.add(world_mesh::world_material(name, &level.vfs, &mut textures, &mut images, &world_shared, opts.exposure))
                })
                .clone();
            let mut e = commands.spawn((Mesh3d(meshes.add(mesh.clone())), MeshMaterial3d(mat)));
            match (in_sky, sky) {
                (true, Some(s)) => {
                    e.insert((sky::sky_transform(&s), sky::sky_layer()));
                }
                _ => {
                    e.insert(Transform::IDENTITY);
                }
            }
            world_draws += 1;
        }
    }
    let t_world = t.elapsed().as_secs_f32();

    let sun = props::Sun::from_bsp(&level.bsp);
    let skip_props = std::env::var_os("CSR_NO_PROPS").is_some();
    let prop_stats = if skip_props {
        props::PropStats { instances: 0, draws: 0, materials: 0, failed_models: 0, sky_draws: 0, triangles: 0 }
    } else { props::spawn(
        &mut commands,
        &level,
        &level.collision,
        &mut meshes,
        &mut materials,
        &mut images,
        &mut textures,
        &world_shared,
        opts.exposure,
        sun,
        sky,
    ) };
    info!(
        "world: {} tris ({} in 3D skybox) in {world_draws} batches ({t_world:.2}s); props: {} instances, {} tris, {} draws ({} in 3D skybox), {} materials, {} failed models; textures: {} compressed, {} decoded, {} missing ({:.2}s total); sun {:?}; sky {:?}",
        main.triangles + skybox.triangles,
        skybox.triangles,
        prop_stats.instances,
        prop_stats.triangles,
        prop_stats.draws,
        prop_stats.sky_draws,
        prop_stats.materials,
        prop_stats.failed_models,
        textures.stats_compressed,
        textures.stats_decoded,
        textures.stats_missing,
        t.elapsed().as_secs_f32(),
        sun,
        sky
    );

    // Characters and sounds.
    let t_chars = Instant::now();
    let character_models = characters::load(
        &level,
        &mut meshes,
        &mut materials,
        &mut images,
        &mut textures,
        &world_shared,
        &mut bindposes,
        opts.exposure,
    );
    let view_models = viewmodel::load(
        &level,
        &mut meshes,
        &mut materials,
        &mut images,
        &mut textures,
        &world_shared,
        &mut bindposes,
        opts.exposure,
    );
    info!("characters and viewmodels loaded in {:.2}s", t_chars.elapsed().as_secs_f32());
    let sounds = audio::SoundLibrary::load(&level.vfs);
    info!(
        "characters: T {}, CT {}, {} weapon world models; {} sound entries; {} weapons",
        character_models.terrorist.is_some(),
        character_models.counter_terrorist.is_some(),
        character_models.weapons.len(),
        sounds.len(),
        level.catalog.weapons.len()
    );
    commands.insert_resource(character_models);
    commands.insert_resource(view_models);
    commands.insert_resource(sounds);

    // Simulation: the local player on CT, bots on both teams.
    let mut world = sim::World::with_catalog(level.collision.clone(), level.catalog.clone());
    world.load_spawns(&level.bsp);
    let (mine, theirs) = if opts.terrorist {
        (sim::Team::Terrorist, sim::Team::CounterTerrorist)
    } else {
        (sim::Team::CounterTerrorist, sim::Team::Terrorist)
    };
    let id = world.add_player("You", mine);
    let names = |team: sim::Team| if team == sim::Team::Terrorist { &BOT_NAMES_T } else { &BOT_NAMES_CT };
    for i in 0..opts.bots {
        world.add_bot(names(theirs)[i as usize % 8], theirs);
    }
    for i in 0..opts.bots.saturating_sub(1) {
        world.add_bot(names(mine)[i as usize % 8], mine);
    }
    let p = world.player(id).expect("player just added");
    let eye = p.movement.eye_position();
    let mut local = LocalPlayer {
        id,
        pitch: 0.0,
        yaw: p.view_angles.y,
        sensitivity: 2.0,
        noclip: false,
        noclip_pos: eye,
        prev_eye: eye,
        curr_eye: eye,
        latched: 0,
        pending_slot: 0,
    };
    if let Some((pos, pitch, yaw)) = opts.camera {
        local.noclip = true;
        local.noclip_pos = pos;
        local.prev_eye = pos;
        local.curr_eye = pos;
        local.pitch = pitch;
        local.yaw = yaw;
    }

    // Sky camera first (3D skybox on its own layer, cleared to the 2D sky
    // colour), then the main camera on top with a fresh depth buffer.
    let sky_clear = sky::sky_color(&level.vfs, &level.bsp).unwrap_or(Color::srgb(0.55, 0.68, 0.85));
    let start = Transform::from_translation(coords::to_bevy(local.curr_eye));
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 0,
            clear_color: ClearColorConfig::Custom(sky_clear),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: VERTICAL_FOV_DEG.to_radians(),
            near: 16.0,
            far: 400_000.0,
            ..default()
        }),
        Tonemapping::None,
        start,
        sky::sky_layer(),
        PlayerCamera,
    ));
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: VERTICAL_FOV_DEG.to_radians(),
            near: 4.0,
            far: 40_000.0,
            ..default()
        }),
        Tonemapping::None,
        start,
        PlayerCamera,
        bevy::audio::SpatialListener::new(16.0 / 39.37),
    ));
    // Viewmodel camera: drawn last with its own depth buffer and FOV.
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 2,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: viewmodel::vertical_fov(viewmodel::VIEWMODEL_FOV),
            near: 1.0,
            far: 512.0,
            ..default()
        }),
        Tonemapping::None,
        start,
        viewmodel::layer(),
        PlayerCamera,
    ));
    commands.insert_resource(local);
    commands.insert_resource(player::BotBrains(sim::bots::Bots::with_skill(0.6).with_nav(level.nav.clone())));
    commands.insert_resource(SimWorld(world));
    commands.insert_resource(LoadedLevel(level));
}

fn smoke_test(
    mut frame: Local<u32>,
    options: Res<Options>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    world: Option<Res<SimWorld>>,
    local: Option<ResMut<LocalPlayer>>,
) {
    *frame += 1;
    let opts = &options.0;
    let Some(path) = &opts.screenshot else { return };
    let mut local = local;
    // CSR_SLOT=n: switch to weapon slot n early in the run.
    if *frame == 10 {
        if let (Some(l), Ok(n)) = (local.as_mut(), std::env::var("CSR_SLOT").map(|v| v.parse::<u8>().unwrap_or(1))) {
            l.pending_slot = n;
        }
    }
    // CSR_WATCH_BOT=n: park the camera 120 units in front of bot n, facing it.
    if let (Some(world), Some(mut local), Ok(n)) = (world, local, std::env::var("CSR_WATCH_BOT").map(|v| v.parse::<usize>().unwrap_or(0))) {
        if let Some(bot) = world.0.players.iter().filter(|p| p.is_bot && p.alive).nth(n) {
            let dist = std::env::var("CSR_WATCH_DIST").ok().and_then(|v| v.parse().ok()).unwrap_or(120.0);
            let (fwd, _, _) = movement::angle_vectors(glam::Vec3::new(0.0, bot.view_angles.y, 0.0));
            let eye = bot.movement.origin + fwd * dist + glam::Vec3::new(0.0, 0.0, 60.0);
            let (pitch, yaw) = sim::bots::look_angles(eye, bot.movement.origin + glam::Vec3::new(0.0, 0.0, 40.0));
            local.noclip = true;
            local.noclip_pos = eye;
            local.prev_eye = eye;
            local.curr_eye = eye;
            local.pitch = pitch;
            local.yaw = yaw;
        }
    }
    if *frame == opts.frames {
        let fps = diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FPS)
            .and_then(|d| d.average())
            .unwrap_or(0.0);
        let ms = diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
            .and_then(|d| d.average())
            .unwrap_or(0.0);
        info!("smoke test: {fps:.1} fps average ({ms:.2} ms/frame) at frame {}", *frame);
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
    }
    if *frame == opts.frames + 30 {
        exit.write(AppExit::Success);
    }
}
