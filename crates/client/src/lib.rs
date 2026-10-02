//! Bevy client for CSGO Rewrite: renders a map from the player's CS:GO
//! install and plays it with CS:GO movement.
//!
//! [`CsgoPlugin`] is self-contained so the whole game can be added to
//! another Bevy app (the planned multi-game "stitch").

pub mod loading;
pub mod overworld_client;
mod overworld_minimap;
mod minecraft_ui;
mod chat;
pub mod audio;
pub mod tracers;
pub mod inventory;
pub mod cosmetics;
pub mod cosmetic_catalog;
pub mod skin_finish;
pub mod decorations;
pub mod cosmetic_geometry;
pub mod case_ui;
pub mod menu_scene;
mod menu_blur;
mod minecraft_settings;
mod minecraft_graphics;
pub mod settings;
pub mod menu;
pub mod feedback;
pub mod utility_fx;
pub mod buymenu;
pub mod items;
pub mod interpolation;
pub mod characters;
pub mod player_anim;
pub mod skinning;
pub mod viewmodel;
pub mod coords;
pub mod effects;
pub mod hud;
pub mod ui_assets;
pub mod hud_art;
pub mod localize;
pub mod material;
pub mod player;
pub mod props;
pub mod scoreboard;
pub mod scope;
pub mod sky;
pub mod textures;
pub mod world_mesh;
pub mod visibility;

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
    pub world_seed:i64,
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
    /// Deathmatch (respawns, fixed loadout) instead of competitive rounds.
    pub deathmatch: bool,
    pub direct: bool,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            map: "de_dust2".into(),
            world_seed:42,
            screenshot: None,
            frames: 120,
            camera: None,
            exposure: 2.0,
            width: 1600,
            height: 900,
            bots: 5,
            terrorist: false,
            deathmatch: false,
            direct: false,
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
        app.add_plugins((overworld_client::OverworldPlugin, loading::LoadingPlugin, menu_blur::PostProcessPlugin, minecraft_settings::MinecraftSettingsPlugin))
            .add_plugins(MaterialPlugin::<WorldMaterial>::default())
            .insert_resource(Time::<Fixed>::from_hz(sim::TICK_RATE as f64))
            .add_systems(Startup, (spawn_level, ui_assets::load, case_ui::setup, hud::setup, hud_art::setup, scope::setup, feedback::setup, utility_fx::setup, menu_scene::setup, menu::setup, hud::mark_roots).chain())
            .add_systems(
                RunFixedMainLoop,
                (menu::seed_input, menu::input, buymenu::input, player::look, player::grab_cursor).chain()
                    .in_set(bevy::app::RunFixedMainLoopSystems::BeforeFixedMainLoop),
            )
            .add_systems(Update, player::update_camera)
            .init_resource::<interpolation::RenderPoses>()
            .init_resource::<tracers::Muzzles>()
            .init_resource::<skin_finish::SkinTextures>()
            .init_resource::<decorations::Decorations>()
            .init_resource::<tracers::Tracers>()
            .init_resource::<tracers::InspectRequest>()
            .init_resource::<menu::Menu>()
            .insert_resource(inventory::Inventory::load())
            .init_resource::<player::InputBlockFrames>()
            .init_resource::<buymenu::BuyMenu>()
            .init_resource::<scoreboard::Scoreboard>()
            .init_resource::<items::GroundInstances>()
            .init_resource::<effects::PendingEvents>()
            .init_resource::<effects::Effects>()
            .init_resource::<chat::Chat>()
            .add_systems(RunFixedMainLoop, chat::input.before(overworld_client::input).in_set(bevy::app::RunFixedMainLoopSystems::BeforeFixedMainLoop))
            .add_systems(Update,chat::draw.after(effects::process).before(hud::update))
            .init_resource::<player::BotBrains>()
            .init_resource::<characters::Characters>()
            .init_resource::<viewmodel::ViewModelState>()
            .add_systems(
                Update,
                (
                    menu::apply,
                    (skin_finish::record, skin_finish::assign, effects::process).chain(),
                    (feedback::update, utility_fx::update).chain(),
                    sky::follow_camera,
                    visibility::update,
                    effects::draw,
                    characters::sync,
                    items::sync,
                    viewmodel::update,
                    tracers::draw,
                    scope::update,
                    buymenu::draw,
                    scoreboard::draw,
                    hud_art::update.run_if(|| std::env::var_os("CSR_NO_HUD").is_none()),
                    (menu_scene::update,skin_finish::update,decorations::update).chain(),
                    (menu::draw, case_ui::input, case_ui::draw, case_ui::animate).chain(),
                    menu::cards,
                    menu::hover,
                    (ui_assets::fonts, hud::menu_visibility).chain(),
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
    let level = match level::Level::load_seed(&opts.map, None,Some(opts.world_seed)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    info!("loaded {} in {:.2}s {:?}", opts.map, t.elapsed().as_secs_f32(), level.timings);

    let mut video_settings=settings::Settings::load();
    if opts.screenshot.is_some() {
        if let Ok(overrides)=std::env::var("CSR_TEST_MC_SETTINGS") {video_settings=settings::Settings::parse(&format!("{}\n{overrides}",video_settings.encode()));}
    }
    // Minecraft: build the whole view before the window opens (at the
    // render distance the game will use, or the stream would start over).
    if let Some(runtime) = &level.overworld {
        let mut runtime = runtime.lock().unwrap();
        let distance = std::env::var("CSR_RENDER_DISTANCE").ok().and_then(|s| s.parse().ok()).unwrap_or(video_settings.mc_render_distance);
        runtime.set_render_distance(distance);
        if std::env::var_os("CSR_NO_PREGEN").is_none() {
            let t = Instant::now();
            let aspect = opts.width as f32 / opts.height.max(1) as f32;
            let done = runtime.pregenerate(VERTICAL_FOV_DEG, aspect, std::time::Duration::from_secs(90), &mut |p| {
                eprintln!("Preparing world: {}/{} chunks, {} sections ({:.1}s)", p.chunks, p.chunks_total, p.sections, p.seconds);
            });
            eprintln!("World prepared in {:.2}s: {} sections at render distance {distance}{}", t.elapsed().as_secs_f32(), done.sections, if done.done { "" } else { " (timed out; the rest streams in)" });
        }
        runtime.stream.set_generation_threads(overworld::play_generation_threads());
    }
    let relaunch = menu::Relaunch::default();
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
    .insert_resource(video_settings)
    .insert_resource(relaunch.clone())
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
    let result = app.run();
    let next = relaunch.0.lock().unwrap().take();
    if let Some(next) = next {
        // A new world also gets fresh GPU assets, simulation and event state.
        // Restart after the old window/event loop is closed (winit cannot
        // create a second event loop in this process on every platform).
        let exe = match std::env::current_exe() { Ok(p)=>p, Err(e)=>{eprintln!("Cannot locate launcher: {e}");return 1;} };
        let mut command=std::process::Command::new(exe);
        command.args(["play", &next.map, "--direct", "--bots", &next.bots.to_string(), "--team", if next.terrorist{"t"}else{"ct"}]);
        command.args(["--seed", &next.world_seed.to_string()]);
        if next.deathmatch { command.arg("--deathmatch"); }
        return match command.spawn() {Ok(_)=>0,Err(e)=>{eprintln!("Could not launch map: {e}");1}};
    }
    match result { AppExit::Success=>0,AppExit::Error(code)=>code.get() as i32 }

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
    if level.overworld.is_none() { sky::spawn_2d(&mut commands, &level, &mut meshes, &mut materials, &mut images, &mut textures, &world_shared); }
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
                    e.insert((Transform::IDENTITY, visibility::MapClusters::mesh(&level.bsp, mesh)));
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
    let cosmetics=cosmetics::Catalog::load(&level.vfs).unwrap_or_else(|e|{error!("Cosmetics: {e}");default()});
    let view_models = viewmodel::load(
        &level,
        &cosmetics,
        &mut meshes,
        &mut materials,
        &mut images,
        &mut textures,
        &world_shared,
        &mut bindposes,
        opts.exposure,
    );
    commands.insert_resource(cosmetics);
    let ground_models = items::load(
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
    let localization = localize::Localization::load(&level.vfs);
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
    commands.insert_resource(ground_models);
    commands.insert_resource(localization);

    // Simulation: the local player on CT, bots on both teams.
    let mut world = sim::World::with_catalog(level.collision.clone(), level.catalog.clone());
    world.load_map(&level.bsp, level.overview().as_deref());
    if let Some(runtime)=&level.overworld {
        let runtime=runtime.lock().unwrap(); let pos=runtime.spawn_source();
        world.spawns.clear();
        for team in [sim::Team::CounterTerrorist,sim::Team::Terrorist] {
            world.spawns.push(sim::SpawnPoint{origin:pos,yaw:0.,team});
            if let Some(nav)=&level.nav { for a in nav.areas.iter().filter(|a|{let d=a.center().distance(pos);d>160.&&d<600.}).step_by(15).take(12) {
                world.spawns.push(sim::SpawnPoint{origin:a.center()+glam::Vec3::Z*0.1,yaw:0.,team});
            }}
        }
    }
    let (mine, theirs) = if opts.terrorist {
        (sim::Team::Terrorist, sim::Team::CounterTerrorist)
    } else {
        (sim::Team::CounterTerrorist, sim::Team::Terrorist)
    };
    let id = world.add_player("You", mine);
    let names = |team: sim::Team| if team == sim::Team::Terrorist { &BOT_NAMES_T } else { &BOT_NAMES_CT };
    let bot_count=if level.overworld.is_some(){0}else{opts.bots};
    for i in 0..bot_count {
        world.add_bot(names(theirs)[i as usize % 8], theirs);
    }
    for i in 0..bot_count.saturating_sub(1) {
        world.add_bot(names(mine)[i as usize % 8], mine);
    }
    if level.overworld.is_some() {
        world.set_mode(sim::GameMode::Special, sim::RulesConfig::default());
        info!("Overworld Special mode: {} player, no CS bots, unlimited time",world.players.len());
    } else if !opts.deathmatch {
        world.set_mode(sim::GameMode::Competitive, competitive_rules(&level));
    }
    // Deterministic visual checks only; interactive sessions ignore these controls.
    if opts.screenshot.is_some() {
        if let Ok(name) = std::env::var("CSR_TEST_WEAPON") {
            if let Some(def) = world.catalog.get(&name).cloned() {
                let p = world.player_mut(id).unwrap();
                let mut weapon = sim::weapon::WeaponState::new(std::sync::Arc::new(def));
                if let Ok(level) = std::env::var("CSR_TEST_ZOOM") { weapon.set_zoom(level.parse().unwrap_or(0), 0.0); }
                p.weapons.retain(|w| sim::weapon::slot(&w.def) != sim::weapon::slot(&weapon.def));
                p.weapons.insert(0, weapon);
                p.active = 0;
            }
        }
    }
    let inventory = inventory::Inventory::load();
    if opts.screenshot.is_none() { inventory::apply(&mut world, id, &inventory); }
    commands.insert_resource(inventory::Collection(level.catalog.clone()));
    if let Some(runtime)=&level.overworld {let runtime=runtime.lock().unwrap();if let Some(p)=world.player_mut(id){p.movement.origin=runtime.spawn_source();
        p.view_angles.y=(0..24).map(|i|i as f32*15.).max_by(|a,b|{
            let distance=|yaw:f32|{let (f,_,_)=movement::angle_vectors(glam::Vec3::new(0.,yaw,0.));level.collision.trace_ray(p.eye(),p.eye()+f*1500.,collision::contents::MASK_SHOT).fraction};distance(*a).total_cmp(&distance(*b))
        }).unwrap_or(0.);
    }}
    let p = world.player(id).expect("player just added");
    let mut local = LocalPlayer::new(id, p.movement.eye_position(), p.view_angles.y);
    if let Some((pos, pitch, yaw)) = opts.camera {
        local.noclip = true;
        local.noclip_pos = pos;
        local.prev_eye = pos;
        local.curr_eye = pos;
        local.pitch = pitch;
        local.yaw = yaw;
    }

    if opts.screenshot.is_some() {
        if let Ok(look) = std::env::var("CSR_TEST_LOOK") {
            let values: Vec<f32> = look.split(',').filter_map(|v|v.parse().ok()).collect();
            if values.len()==2 {local.pitch=values[0];local.yaw=values[1];}
        }
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
        bevy::audio::SpatialListener::new(if level.overworld.is_some() {16.0} else {16.0 / 39.37}),
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
        player::ViewModelCamera,
        menu_blur::PostProcessSettings::default(),
    ));
    commands.insert_resource(local);
    commands.insert_resource(player::BotBrains(sim::bots::Bots::with_skill(0.6).with_nav(level.nav.clone())));
    commands.insert_resource(SimWorld(world));
    commands.insert_resource(LoadedLevel(level));
}

/// Competitive rules from the install's own `gamemode_competitive.cfg`.
pub fn competitive_rules(level: &level::Level) -> sim::RulesConfig {
    let cvars = level.cfg("gamemode_competitive.cfg").map(|t| sim::rules::parse_cfg(&t)).unwrap_or_default();
    sim::RulesConfig::from_cvars(&cvars)
}

fn smoke_test(
    mut frame: Local<u32>,
    mut frame_times: Local<Vec<f32>>,
    render_stats: Res<minecraft_graphics::RenderStats>,
    real: Res<Time<Real>>,
    options: Res<Options>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    mut world: Option<ResMut<SimWorld>>,
    local: Option<ResMut<LocalPlayer>>,
    mut menu: ResMut<buymenu::BuyMenu>,
    mut inspect: ResMut<tracers::InspectRequest>,
    mut events: ResMut<effects::PendingEvents>,
    level: Res<LoadedLevel>,
) {
    *frame += 1;

    let opts = &options.0;
    let Some(path) = &opts.screenshot else { return };
    if *frame > 120 && *frame <= opts.frames { frame_times.push(real.delta_secs() * 1000.0); }
    // CSR_FRAME_LOG=ms: log every frame slower than that, with renderer stats.
    if let Some(limit) = std::env::var("CSR_FRAME_LOG").ok().and_then(|v| v.parse::<f32>().ok()) {
        let ms = real.delta_secs() * 1000.0;
        if ms > limit {
            let stream = level.0.overworld.as_ref().and_then(|r| r.lock().ok()).map(|r| (r.stream_tick_ms, r.stream_tick_loaded)).unwrap_or_default();
            info!("slow frame {}: {ms:.1} ms (terrain {:.2} ms, {} uploads in {:.2} ms, entities {:.2} ms, {} sections, last stream tick {:.2} ms / {} chunks)", *frame, render_stats.terrain_ms, render_stats.uploads, render_stats.upload_ms, render_stats.entities_ms, render_stats.sections, stream.0, stream.1);
        }
    }
    if *frame == opts.frames.saturating_sub(90) && std::env::var_os("CSR_INSPECT").is_some(){inspect.0+=1;}
    // CSR_BUY_MENU=n: show the buy menu on category n (0 = none picked).
    if let Ok(cat) = std::env::var("CSR_BUY_MENU").map(|v| v.parse::<usize>().unwrap_or(0)) {
        menu.open = true;
        menu.category = cat.checked_sub(1);
    }
    let mut local = local;
    if std::env::var_os("CSR_TEST_TRAVERSE").is_some() {
        if let Some(l)=local.as_mut() {
            // Reproducible screenshot-only streaming route: 0.2 blocks/frame, rotating camera.
            l.noclip=true;
            let f=frame.saturating_sub(120) as f32;
            l.noclip_pos=glam::Vec3::new(f*overworld::BLOCK*0.2,0.,overworld::BLOCK*35.);
            l.prev_eye=l.noclip_pos; l.curr_eye=l.noclip_pos;
            l.yaw=f*0.3; l.pitch=15.;
            if let Some(w)=world.as_mut() {
                if let Some(p)=w.0.player_mut(l.id) {
                    p.movement.origin=l.noclip_pos-glam::Vec3::Z*64.;
                    p.movement.velocity=glam::Vec3::ZERO;
                }
            }
        }
    }
    // Explicit screenshot-only fixture, never enabled in interactive play.
    // This checks rendering/event plumbing, not combat accuracy.
    if *frame == opts.frames.saturating_sub(8) && std::env::var_os("CSR_TEST_FEEDBACK").is_some() {
        if let Some(l)=local.as_ref() {
            let (forward,_,_)=movement::angle_vectors(l.render_angles);
            let start=l.render_eye;
            let tr=level.0.collision.trace_ray(start,start+forward*500.0,collision::contents::MASK_SOLID);
            if tr.hit() {events.0.push(sim::GameEvent::Impact{pos:tr.end,normal:tr.normal,surface:tr.surface});}
            let pos=start+forward*70.0;
            events.0.push(sim::GameEvent::Damage{attacker:l.id,victim:l.id,health:25,group:sim::weapon::HitGroup::Chest,pos});
        }
    }
    if *frame == opts.frames.saturating_sub(12) && std::env::var_os("CSR_TEST_DROP").is_some() {
        if let Some(l)=local.as_mut(){l.pending_drop=true;}
    }
    if *frame == opts.frames.saturating_sub(10) && std::env::var_os("CSR_TEST_DROP").is_some() {
        if let Some(l)=local.as_mut(){l.pitch=45.0;}
    }
    // CSR_SLOT=n: switch to weapon slot n early in the run.
    if *frame == 10 {
        if let (Some(l), Ok(n)) = (local.as_mut(), std::env::var("CSR_SLOT").map(|v| v.parse::<u8>().unwrap_or(1))) {
            l.pending_slot = n;
        }
    }
    // CSR_TEST_RELOAD=n: empty the held gun and press reload at frame n.
    if let (Some(w), Some(l), Ok(at)) = (world.as_mut(), local.as_mut(), std::env::var("CSR_TEST_RELOAD").map(|v| v.parse::<u32>().unwrap_or(20))) {
        if *frame == at {
            if let Some(gun) = w.0.player_mut(l.id).and_then(|p| { let a = p.active; p.weapons.get_mut(a) }) {
                gun.clip = 0;
            }
            l.latched |= movement::buttons::RELOAD;
        }
    }
    if *frame==opts.frames.saturating_sub(90) {
        if let (Some(w),Some(l),Ok(kind))=(world.as_mut(),local.as_ref(),std::env::var("CSR_TEST_UTILITY")) {
            let (forward,_,_)=movement::angle_vectors(glam::Vec3::new(0.0,l.yaw,0.0));
            let now=w.0.time();
            let pos=w.0.collision.trace_ray(l.curr_eye+forward*330.0,l.curr_eye+forward*330.0-glam::Vec3::Z*200.0,collision::contents::MASK_SOLID).end;
            if kind=="flash" {if let Some(p)=w.0.player_mut(l.view_target()){p.flash_until=now+5.0;}}
            else {w.0.utility_fields.push(sim::utility::Field{id:999999,owner:l.id,kind:if kind=="fire"{sim::utility::Kind::Molotov}else{sim::utility::Kind::Smoke},pos,born:now-1.0,until:now+20.0});}
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
        info!("overworld renderer: sections {}/{}, uploads {}, entity vertices {}, terrain {:.2} ms, upload {:.2} ms, entities {:.2} ms",render_stats.visible_sections,render_stats.sections,render_stats.uploads,render_stats.mob_vertices,render_stats.terrain_ms,render_stats.upload_ms,render_stats.entities_ms);
        info!("lighting: {} pending, {} accepted, worker mean {:.2} ms",render_stats.pending_lights,render_stats.light_updates,render_stats.worker_light_ms);
        info!("smoke test: {fps:.1} fps average ({ms:.2} ms/frame) at frame {}", *frame);
        if !frame_times.is_empty() {
            frame_times.sort_by(f32::total_cmp);
            let mean = frame_times.iter().sum::<f32>() / frame_times.len() as f32;
            let percentile = |p: f32| frame_times[((frame_times.len() - 1) as f32 * p) as usize];
            info!("frame pacing after warmup: {} frames, {:.1} fps, mean {:.2} ms, p95 {:.2} ms, p99 {:.2} ms, max {:.2} ms", frame_times.len(), 1000.0 / mean, mean, percentile(0.95), percentile(0.99), percentile(1.0));
        }
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
    }
    if *frame == opts.frames + 30 {
        exit.write(AppExit::Success);
    }
}
