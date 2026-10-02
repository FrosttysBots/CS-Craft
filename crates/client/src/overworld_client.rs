//! Presentation and input bridge: one CS player/camera, one Minecraft world.
#[path = "minecraft_feedback.rs"]
mod feedback;
#[path = "mob_feedback_preview.rs"]
mod mob_preview;
use crate::{
    LoadedLevel,
    player::{LocalPlayer, SimWorld},
};
use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology},
    pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin},
    prelude::*,
    render::render_resource::{
        AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
        TextureDimension, TextureFormat,
    },
    shader::ShaderRef,
};
use collision::DynamicWorld;
use overworld::{
    glam_mc,
    terrain::{
        self,
        scene::Block,
        sections::SectionPos,
    },
};
use std::collections::HashMap;

#[derive(ShaderType, Clone, Debug)]
pub struct Parameters {
    pub daylight: Vec4,
    pub fog: Vec4,
}
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct VoxelMaterial {
    #[uniform(0)]
    pub params: Parameters,
    #[texture(1)]
    #[sampler(2)]
    pub atlas: Handle<Image>,
    pub transparent: bool,
    pub multiply: bool,
}
impl Material for VoxelMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://client/shaders/overworld.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://client/shaders/overworld.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        if self.multiply {
            AlphaMode::Multiply
        } else if self.transparent {
            AlphaMode::Blend
        } else {
            AlphaMode::Mask(0.5)
        }
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _: &MaterialPipeline,
        d: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        d.primitive.cull_mode = Some(bevy::render::render_resource::Face::Back);
        Ok(())
    }
}
// Keep the large atlas resident; animation uploads touch only changed sprite rectangles.
#[derive(Clone)]
struct AtlasPatch {
    level: u32,
    origin: bevy::render::render_resource::Origin3d,
    size: Extent3d,
    pixels: std::sync::Arc<[u8]>,
}
#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
struct AtlasUpdates {
    atlas: Handle<Image>,
    serial: u64,
    patches: Vec<AtlasPatch>,
}
fn upload_atlas(
    updates: Res<AtlasUpdates>,
    images: Res<bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>>,
    queue: Res<bevy::render::renderer::RenderQueue>,
    mut uploaded: Local<u64>,
) {
    if updates.serial == *uploaded {
        return;
    }
    let Some(image) = images.get(&updates.atlas) else {
        return;
    };
    for patch in &updates.patches {
        queue.write_texture(
            bevy::render::render_resource::TexelCopyTextureInfo {
                texture: &image.texture,
                mip_level: patch.level,
                origin: patch.origin,
                aspect: bevy::render::render_resource::TextureAspect::All,
            },
            &patch.pixels,
            bevy::render::render_resource::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(patch.size.width * 4),
                rows_per_image: Some(patch.size.height),
            },
            patch.size,
        );
    }
    *uploaded = updates.serial;
}
#[derive(Resource, Default)]
pub struct Building {
    pub active: bool,
    pub inventory_open: bool,
    pub using_block: bool,
    pub selected: usize,
    pub status: String,
    cooldown: f32,
    pub initialized: bool,
    survival_clock: f32,
    last_feet: Vec3,
    health_loaded: bool,
}
#[derive(Resource, Default)]
struct RenderWorld {
    preview_frame: u32,
    sections: HashMap<SectionPos, [Option<Entity>; 2]>,
    solid: Handle<VoxelMaterial>,
    water: Handle<VoxelMaterial>,
    cracks: Handle<VoxelMaterial>,
    atlas: Handle<Image>,
    mob: Option<Entity>,
    initialized: bool,
    mesh_handles: HashMap<Entity, Handle<Mesh>>,
    animation_tick: u64,
    epoch: u64,
    render_distance: Option<i32>,
}
#[derive(Component)]
struct VoxelRadar;
#[derive(Resource, Default)]
struct MapView {
    map: crate::overworld_minimap::Minimap,
    entity: Option<Entity>,
}
#[derive(Component)]
pub(super) struct Slot(pub overworld::survival::Slot);
#[derive(Resource, Default)]
struct Sounds {
    loaded: HashMap<String, Handle<AudioSource>>,
    counter: usize,
    footsteps: overworld::block_audio::Footsteps,
}
pub struct OverworldPlugin;
impl Plugin for OverworldPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/overworld.wgsl");
        app.add_plugins(MaterialPlugin::<VoxelMaterial>::default())
            .init_resource::<Building>()
            .init_resource::<feedback::Feedback>()
            .init_resource::<RenderWorld>()
            .init_resource::<AtlasUpdates>()
            .add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
                AtlasUpdates,
            >::default())
            .init_resource::<Sounds>()
            .init_resource::<MapView>()
            .init_resource::<crate::minecraft_ui::MinecraftUi>()
            .add_systems(
                RunFixedMainLoop,
                input
                    .before(crate::menu::input)
                    .in_set(bevy::app::RunFixedMainLoopSystems::BeforeFixedMainLoop),
            )
            .add_systems(FixedUpdate, tick.after(crate::player::tick))
            .add_systems(
                Update,
                (render, feedback::update, crate::minecraft_ui::draw, sounds, minimap)
                    .chain()
                    .after(crate::player::update_camera),
            );
        if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render_app.add_systems(
                bevy::render::Render,
                upload_atlas.in_set(bevy::render::RenderSystems::PrepareResources),
            );
        }
    }
}

pub(crate) fn input(
    level: Option<Res<LoadedLevel>>,
    mut building: ResMut<Building>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    front: Res<crate::menu::Menu>,
    mut cursor: Single<&mut bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>,
    slots: Query<(&Interaction, &Slot)>,
    buy: Res<crate::buymenu::BuyMenu>,
    local: Res<LocalPlayer>,
    settings: Res<crate::settings::Settings>,
    mut commands: Commands,
    block: Res<crate::player::InputBlockFrames>,
    world: Res<SimWorld>,
) {
    use overworld::survival::Screen;
    let Some(level) = level else { return };
    let Some(runtime) = &level.0.overworld else {
        return;
    };
    if front.open() || buy.open {
        return;
    }
    let mut runtime = runtime.lock().unwrap();
    let was_open = building.inventory_open;
    if !mouse.pressed(MouseButton::Right) { building.using_block=false; }
    if !was_open && block.0==0 && world.0.player(local.id).is_some_and(|p|p.alive) && cursor.grab_mode != bevy::window::CursorGrabMode::None &&
        (keys.just_pressed(settings.binds[7]) || (mouse.just_pressed(MouseButton::Right) && !keys.pressed(KeyCode::ShiftLeft) && !keys.pressed(KeyCode::ShiftRight))) {
        let mut ray=overworld::mc_player::Player::new(glam_mc::DVec3::from_array(overworld::to_block(runtime.save.origin,local.curr_eye)));
        // Player::target adds its eye height, so use the render eye directly via the ray helper below.
        ray.pos.y -= ray.eye_height();
        ray.yaw=(-local.yaw-90.) as f64; ray.pitch=local.pitch as f64;
        let target={let scene=runtime.collision.scene.read().unwrap();ray.target(&*scene,5.).map(|h|h.pos)};
        if let Some(p)=target {
            let opened=runtime.open_block(p);
            let handled=if opened {true} else {
                let scene=runtime.collision.scene.clone();let scene=scene.read().unwrap();
                runtime.server.use_block(&scene,p,"north")
            };
            if handled { building.using_block=true;building.cooldown=0.25; }
        }
    }
    if keys.just_pressed(KeyCode::KeyN) && !building.inventory_open {
        building.active = !building.active;
        building.status.clear();
    }
    if keys.just_pressed(KeyCode::KeyI) {
        if runtime.screen != Screen::Closed {
            runtime.close_screen();
        } else {
            runtime.screen = Screen::Inventory;
        }
    }
    if runtime.screen != Screen::Closed && keys.just_pressed(KeyCode::Escape) {
        runtime.close_screen();
        keys.clear_just_pressed(KeyCode::Escape);
    }
    building.inventory_open = runtime.screen != Screen::Closed;
    if building.inventory_open {
        cursor.grab_mode = bevy::window::CursorGrabMode::None;
        cursor.visible = true;
        if mouse.just_pressed(MouseButton::Left) || mouse.just_pressed(MouseButton::Right) {
            if let Some((_, slot)) = slots.iter().find(|(i, _)| **i != Interaction::None) {
                runtime.click_slot(
                    slot.0,
                    mouse.just_pressed(MouseButton::Right),
                    keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
                );
            }
        }
        return;
    }
    if was_open || keys.just_pressed(KeyCode::KeyI) || keys.just_released(KeyCode::Escape) {
        cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
        cursor.visible = false;
        commands.insert_resource(crate::player::InputBlockFrames(2));
    }
    if building.active {
        for (i, k) in [
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
            KeyCode::Digit5,
            KeyCode::Digit6,
            KeyCode::Digit7,
            KeyCode::Digit8,
            KeyCode::Digit9,
        ]
        .iter()
        .enumerate()
        {
            if keys.just_pressed(*k) {
                building.selected = i;
            }
        }
        if scroll.delta.y != 0. {
            building.selected = (building.selected as i32
                - if scroll.delta.y > 0. { 1 } else { -1 })
            .rem_euclid(9) as usize;
        }
        if keys.just_pressed(KeyCode::KeyQ) {
            let selected = building.selected;
            if let Some(stack) = runtime
                .inventory
                .drop_selected(selected, keys.pressed(KeyCode::ControlLeft))
            {
                let p = runtime.save.player;
                runtime.drop_stack(stack, p);
            }
        }
    }
    if building.initialized {
        runtime.save.selected = building.selected;
    }
}

#[allow(clippy::too_many_arguments)]
fn tick(
    level: Res<LoadedLevel>,
    mut world: ResMut<SimWorld>,
    mut local: ResMut<LocalPlayer>,
    mut building: ResMut<Building>,
    mouse: Res<ButtonInput<MouseButton>>,
    front: Res<crate::menu::Menu>,
    enabled: Res<crate::player::InputEnabled>,
    cursor: Single<&bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>,
    mut pending: ResMut<crate::effects::PendingEvents>,
    mut poses: ResMut<crate::interpolation::RenderPoses>,
    buy: Res<crate::buymenu::BuyMenu>,
    block: Res<crate::player::InputBlockFrames>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    use overworld::survival::Screen;
    let Some(runtime) = &level.0.overworld else {
        return;
    };
    if front.open() {
        return;
    }
    let mut runtime = runtime.lock().unwrap();
    if !building.health_loaded {
        let catalog = world.0.catalog.clone();
        if let Some(p) = world.0.player_mut(local.id) {
            p.money = runtime.save.survival.money;
            if let Some(saved) = runtime.save.survival.loadout.as_ref().filter(|s| !s.weapons.is_empty()) {
                p.weapons = saved.weapons.iter().filter_map(|(id, clip, reserve)| {
                    catalog.get(id).map(|d| {
                        let mut w = sim::weapon::WeaponState::new(std::sync::Arc::new(d.clone()));
                        w.clip = *clip; w.reserve = *reserve; w
                    })
                }).collect();
                p.active = saved.active.min(p.weapons.len().saturating_sub(1));
                p.last_active = p.active;
                p.armor = saved.armor; p.helmet = saved.helmet; p.defuse_kit = saved.defuse_kit;
            }
            if !p.weapons.iter().any(|w| w.def.class==weapons::WeaponClass::Knife) {
                let id=if p.team==sim::Team::Terrorist {"weapon_knife_t"} else {"weapon_knife"};
                if let Some(d)=catalog.get(id) { p.weapons.push(sim::weapon::WeaponState::new(std::sync::Arc::new(d.clone()))); }
            }
            p.health = (runtime.save.survival.health * 5.).round().clamp(1., 100.) as i32;
        }
        building.health_loaded = true;
    }
    if let Some(p) = world.0.player(local.id) {
        runtime.save.survival.money = p.money;
        runtime.save.survival.loadout = Some(overworld::survival::Loadout {
            weapons: if p.alive { p.weapons.iter().map(|w| (w.def.class_name.clone(), w.clip, w.reserve)).collect() } else { vec![] },
            active: p.active, armor: p.armor, helmet: p.helmet, defuse_kit: p.defuse_kit,
        });
    }
    let Some(player) = world.0.player(local.id) else {
        return;
    };
    let feet = player.movement.origin;
    let alive = player.alive;
    let eye = player.eye();
    let bounds = player.bounds();
    let ground = player.movement.on_ground;
    let health = player.health;
    let controls = enabled.0
        && block.0==0
        && alive
        && !building.inventory_open
        && !buy.open
        && cursor.grab_mode != bevy::window::CursorGrabMode::None;
    building.cooldown = (building.cooldown - sim::TICK_DT).max(0.);
    let (forward, _, _) = movement::angle_vectors(Vec3::new(local.pitch, local.yaw, 0.));
    let screenshot_mining = !enabled.0 && std::env::var_os("CSR_TEST_MINING").is_some();
    if !screenshot_mining || runtime.mining.progress() < 0.65 {
    runtime.mine_survival(
        sim::TICK_DT,
        feet,
        local.yaw,
        local.pitch,
        ground,
        (controls && building.active && mouse.pressed(MouseButton::Left)) || screenshot_mining,
    );
    }
    if controls && building.cooldown <= 0. {
        let hit = runtime.collision.trace_box(
            eye,
            eye + forward * overworld::BLOCK * 5.,
            Vec3::ZERO,
            Vec3::ZERO,
            collision::contents::MASK_SHOT,
        );
        let mut target = hit.hit().then(|| {
            overworld::block_pos(overworld::to_block(
                runtime.save.origin,
                hit.end - hit.normal,
            ))
        });
        if runtime.inventory.slots[building.selected]
            .as_ref()
            .is_some_and(|s| s.id == "minecraft:bucket")
        {
            let mut p = overworld::mc_player::Player::new(glam_mc::DVec3::from_array(
                overworld::to_block(runtime.save.origin, feet),
            ));
            p.yaw = (-local.yaw - 90.) as f64;
            p.pitch = local.pitch as f64;
            let scene = runtime.collision.scene.read().unwrap();
            if let Some(h) = p.target_source_fluid(&*scene, 5.) {
                target = Some(h.pos);
            }
        }
        let place = hit.hit().then(|| {
            overworld::block_pos(overworld::to_block(
                runtime.save.origin,
                hit.end + hit.normal,
            ))
        });
        if building.active && !building.using_block && mouse.pressed(MouseButton::Right) {
            let used = runtime.use_item(target, place);
            if !used {
                if let (Some(pos), Some(stack)) =
                    (place, runtime.inventory.slots[building.selected].clone())
                {
                    let a = overworld::to_source(
                        runtime.save.origin,
                        [pos.0 as f64, pos.1 as f64, pos.2 as f64],
                    );
                    let b = overworld::to_source(
                        runtime.save.origin,
                        [(pos.0 + 1) as f64, (pos.1 + 1) as f64, (pos.2 + 1) as f64],
                    );
                    if !(a.min(b).cmplt(bounds.1).all() && a.max(b).cmpgt(bounds.0).all())
                        && runtime.registries.blocks.parse_state(&stack.id).is_ok()
                        && runtime.block_name(pos).is_none()
                    {
                        let mut block = Block::new(&stack.id);
                        if stack.id.ends_with("furnace")
                            || stack.id.ends_with("chest")
                            || stack.id.ends_with("smoker")
                        {
                            block.properties.insert("facing".into(), "north".into());
                        }
                        if orient_climbable(&mut block,hit.normal) && runtime.place_block(pos, block) {
                            runtime.consume_selected();
                            building.status.clear();
                        }
                    }
                }
            }
            building.cooldown = 0.25;
        }
    }

    if alive && !runtime.was_alive {
        runtime.save.survival.respawn_pending = false;
        let spawn = runtime.save.survival.spawn.unwrap_or(runtime.save.origin);
        if runtime.save.adventure.dimension != "overworld" {
            if let Err(e) = runtime.change_dimension(
                overworld::terrain::terrain::Dimension::Overworld,
                Some(spawn),
            ) {
                building.status = e;
            }
        } else {
            runtime.teleport =
                Some(overworld::to_source(runtime.save.origin, spawn) + Vec3::Z * 0.5);
        }
        runtime.was_alive = true;
    }
    let tick_feet = runtime.teleport.unwrap_or(feet);
    runtime.tick(sim::TICK_DT, tick_feet, alive);
    if let Some(p) = world.0.player_mut(local.id) { p.money = runtime.save.survival.money; }
    building.survival_clock += sim::TICK_DT;

    let event_start = world.0.events.len();
    if building.survival_clock >= 0.05 {
        building.survival_clock -= 0.05;
        let moved = feet.distance(building.last_feet) / overworld::BLOCK;
        building.last_feet = feet;
        let delta = runtime.survival_tick(
            health,
            alive,
            ground,
            controls && building.active && mouse.pressed(MouseButton::Right),
            moved.min(2.),
        );
        if delta < 0 {
            world.0.damage_environment(local.id, -delta, "survival");
        } else if delta > 0 {
            if let Some(p) = world.0.player_mut(local.id) {
                p.health = (p.health + delta).min(100);
            }
        }
    }
    for source in std::mem::take(&mut runtime.player_hits) {
        let hit=source.hit;
        if hit.player_id == 1 && alive {
            let raw = hit.damage * 5.;
            let mc_damage = raw * (1. - (runtime.inventory.armor_value() as f32 * 0.04).min(0.8));
            let mut protected = mc_damage;
            if let Some(p) = world.0.player_mut(local.id) {
                // Use the stronger armor rather than multiplying both armor systems.
                let absorbed = (raw * 0.5).min(p.armor.max(0) as f32 * 2.);
                if raw - absorbed < protected {
                    protected = raw - absorbed;
                    p.armor = (p.armor - (absorbed * 0.5).ceil() as i32).max(0);
                }
            }
            let damage = protected.round().max(1.) as i32;
            world.0.damage_environment(local.id, damage, &source.cause());
        }
    }
    if let Some(pos) = runtime.teleport.take() {
        if let Some(p) = world.0.player_mut(local.id) {
            p.movement.origin = pos;
            p.movement.velocity = Vec3::ZERO;
            local.prev_eye = p.eye();
            local.curr_eye = p.eye();
            local.render_eye = p.eye();
        }
        poses.capture(&world.0);
        building.last_feet = pos;
    }
    if runtime.screen == Screen::Victory && keys.just_pressed(KeyCode::Enter) {
        let target = runtime.save.survival.spawn.or(Some(runtime.save.origin));
        if let Err(e) =
            runtime.change_dimension(overworld::terrain::terrain::Dimension::Overworld, target)
        {
            building.status = e;
        }
    }
    if let Some(msg) = runtime.notices.last() {
        building.status = msg.clone();
    }
    building.inventory_open = runtime.screen != Screen::Closed;
    pending
        .0
        .extend(world.0.events[event_start..].iter().cloned());
}

pub(super) fn image(pixels: &[u8], width: u32, height: u32) -> Image {
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels.to_vec(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Nearest,
        min_filter: ImageFilterMode::Nearest,
        ..default()
    });
    image
}

fn minimap(
    mut commands: Commands,
    level: Res<LoadedLevel>,
    local: Res<LocalPlayer>,
    mut map: ResMut<MapView>,
    time: Res<Time>,
    mut radar: ResMut<crate::hud_art::Radar>,
    mut images: ResMut<Assets<Image>>,
    mut nodes: Query<(&mut ImageNode, &mut Visibility), With<VoxelRadar>>,
    stats: Res<crate::minecraft_graphics::RenderStats>,
    front: Res<crate::menu::Menu>,
    settings: Res<crate::settings::Settings>,
) {
    for (_,mut visible) in &mut nodes { *visible=if front.open() || !settings.mc_minimap || stats.debug {Visibility::Hidden} else {Visibility::Inherited}; }
    if front.open() || !settings.mc_minimap || stats.debug {return;}
    let Some(runtime) = &level.0.overworld else {
        return;
    };
    let runtime = runtime.lock().unwrap();
    let scene = runtime.collision.scene.read().unwrap();
    let feet = overworld::to_block(runtime.save.origin, local.render_eye);
    if let Some((image, corner)) = map.map.update(
        time.delta_secs_f64(),
        feet,
        &scene,
        &runtime.packs,
        &runtime.atlas,
        &mut images,
    ) {
        let origin = overworld::to_source(
            runtime.save.origin,
            [corner[0] as f64, 0., corner[1] as f64],
        );
        radar.origin = Vec2::new(origin.x, origin.y);
        radar.scale = overworld::BLOCK * 128. / 1024.;
        if let Some(e) = map.entity {
            if let Ok((mut node, mut visible)) = nodes.get_mut(e) {
                node.image = image;
                *visible = if front.open() {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                };
            }
        } else {
            map.entity = Some(
                commands
                    .spawn((
                        ImageNode::new(image),
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(22.),
                            top: px(24.),
                            width: px(250.),
                            height: px(250.),
                            ..default()
                        },
                        VoxelRadar,
                    ))
                    .id(),
            );
        }
    }
}

// Face normals are in Source coordinates; vine properties name the supporting face.
fn orient_climbable(block:&mut Block,normal:Vec3)->bool {
    let (outward,support)=if normal.x>0.5 {("east","west")} else if normal.x< -0.5 {("west","east")} else if normal.y>0.5 {("north","south")} else {("south","north")};
    match block.id.path.as_str() {
        "ladder"=>{
            if normal.z.abs()>0.5 {return false;}
            block.properties.insert("facing".into(),outward.into());
        }
        "vine"=>{
            if normal.z>0.5 {return false;}
            for side in ["north","south","east","west","up"] {block.properties.insert(side.into(),"false".into());}
            block.properties.insert(if normal.z< -0.5 {"up"} else {support}.into(),"true".into());
        }
        _=>{}
    }
    true
}

#[derive(Component)]
struct MinecraftSound { mob: bool, range: f32 }
#[derive(Component)]
struct MinecraftLocalSound(f32);
fn audible_gain(distance: f32, range: f32) -> f32 {
    (1. - distance / range).clamp(0.,1.)
}
fn sounds(
    mut commands: Commands,
    level: Res<LoadedLevel>,
    local: Res<LocalPlayer>,
    world: Res<SimWorld>,
    settings: Res<crate::settings::Settings>,
    mut sounds: ResMut<Sounds>,
    mut assets: ResMut<Assets<AudioSource>>,
    mut playing: Query<(&MinecraftSound, &Transform, &mut bevy::audio::SpatialAudioSink)>,
    mut local_playing: Query<(&MinecraftLocalSound,&mut bevy::audio::AudioSink)>,
) {
    use bevy::audio::{AudioSinkPlayback, Decodable, PlaybackSettings, SpatialScale, Volume};
    for (sound,mut sink) in &mut local_playing {sink.set_volume(Volume::Linear(settings.volume*settings.mc_blocks*sound.0));}
    let listener=crate::coords::to_bevy(local.render_eye);
    for (sound,pos,mut sink) in &mut playing {
        let category=if sound.mob {settings.mc_mobs} else {settings.mc_blocks};
        sink.set_volume(Volume::Linear(settings.volume * category * 0.6 * audible_gain(pos.translation.distance(listener)/overworld::BLOCK,sound.range)));
    }
    let Some(runtime) = &level.0.overworld else {
        return;
    };
    let mut runtime = runtime.lock().unwrap();
    if let Some(p) = world.0.player(local.id) {
        let feet=overworld::to_block(runtime.save.origin,p.movement.origin);
        if sounds.footsteps.update(feet,p.alive&&p.movement.on_ground&&!local.noclip) {
            let point=overworld::to_block(runtime.save.origin,p.movement.origin-Vec3::Z*2.);
            if let Some(id)=runtime.block_name(overworld::block_pos(point)) {
                runtime.block_sound(&id,overworld::block_audio::Action::Step);
            }
        }
    }
    // A player's own action feedback should not be attenuated as a distant emitter.
    // It also has priority over busy ambient/mob queues.
    for sound in std::mem::take(&mut runtime.local_block_sounds) {
        if settings.volume<=0. || settings.mc_blocks<=0. {continue;}
        sounds.counter=sounds.counter.wrapping_add(1);
        let variant=sounds.counter%8;
        let key=format!("{}:{variant}",sound.event);
        let handle=if let Some(h)=sounds.loaded.get(&key){h.clone()}else{
            let Some((_,bytes))=runtime.sound_bytes(&sound.event,variant) else {continue;};
            let source=AudioSource{bytes:bytes.into()};
            if std::panic::catch_unwind(||source.decoder()).is_err(){continue;}
            let h=assets.add(source);sounds.loaded.insert(key,h.clone());h
        };
        commands.spawn((MinecraftLocalSound(sound.volume),AudioPlayer(handle),
            PlaybackSettings::DESPAWN.with_spatial(false)
                .with_speed(sound.pitch)
                .with_volume(Volume::Linear(settings.volume*settings.mc_blocks*sound.volume))));
    }
    let origin=runtime.save.origin;
    let mut events=std::mem::take(&mut runtime.sounds);
    events.sort_by(|a,b| {
        let distance=|p|crate::coords::to_bevy(overworld::to_source(origin,p)).distance_squared(listener);
        distance(a.1).total_cmp(&distance(b.1))
    });
    let mut emitted=0;
    for (event, point) in events {
        let pos=crate::coords::to_bevy(overworld::to_source(origin,point));
        let mob=event.contains(":entity.") && !event.contains(":entity.player.");
        let range=if event.contains("ender_dragon") || event.contains("explode") {64.} else {16.};
        let category=if mob {settings.mc_mobs} else {settings.mc_blocks};
        let gain=audible_gain(pos.distance(listener)/overworld::BLOCK,range);
        if gain<=0. || category<=0. || settings.volume<=0. {continue;}
        if emitted>=24 {break;} emitted+=1;
        sounds.counter = sounds.counter.wrapping_add(1);
        let key = format!("{event}:{}", sounds.counter % 4);
        let handle = if let Some(h) = sounds.loaded.get(&key) {
            h.clone()
        } else {
            let Some((_, bytes)) = runtime.sound_bytes(&event, sounds.counter) else {
                continue;
            };
            let source = AudioSource {
                bytes: bytes.into(),
            };
            if std::panic::catch_unwind(|| source.decoder()).is_err() {
                continue;
            }
            let h = assets.add(source);
            sounds.loaded.insert(key, h.clone());
            h
        };
        commands.spawn((
            MinecraftSound {mob,range},
            AudioPlayer(handle),
            PlaybackSettings::DESPAWN
                .with_volume(Volume::Linear(settings.volume * category * 0.6 * gain))
                .with_spatial(true)
                .with_spatial_scale(SpatialScale::new(1. / overworld::BLOCK)),
            Transform::from_translation(pos),
        ));
    }
}
fn mesh_part(vertices:&[terrain::mesh::SectionVertex],mut indices:Vec<u32>,origin:[f64;3])->Mesh {
    let mut remap=vec![u32::MAX;vertices.len()];
    let mut used=Vec::with_capacity(vertices.len().min(indices.len()));
    for i in &mut indices {
        let mapped=&mut remap[*i as usize];
        if *mapped==u32::MAX {*mapped=used.len() as u32;used.push(vertices[*i as usize]);}
        *i=*mapped;
    }
    mesh(&used,indices,origin)
}
/// One triangle with the terrain's exact vertex layout, so drawing it with
/// a voxel material compiles the same pipeline the world will use.
pub(crate) fn warmup_mesh() -> Mesh {
    let v = |x: f32, y: f32| terrain::mesh::SectionVertex { position: [x, y, 0.0], uv: [0.0, 0.0], color: [255; 4], light: [240, 0], pad: [0; 2] };
    mesh(&[v(0.0, 0.0), v(0.1, 0.0), v(0.0, 0.1)], vec![0, 1, 2], [0.0; 3])
}
pub(super) fn mesh(vertices: &[terrain::mesh::SectionVertex], indices: Vec<u32>, origin: [f64; 3]) -> Mesh {
    mesh_data(vertices, indices, origin, None)
}
fn mesh_data(vertices: &[terrain::mesh::SectionVertex], indices: Vec<u32>, origin: [f64;3], native:Option<&[terrain::mesh::Vertex]>) -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    let positions: Vec<[f32; 3]> = vertices
        .iter()
        .map(|v| {
            [
                (v.position[0] as f64 - origin[0]) as f32 * overworld::BLOCK,
                (v.position[1] as f64 - origin[1]) as f32 * overworld::BLOCK,
                (v.position[2] as f64 - origin[2]) as f32 * overworld::BLOCK,
            ]
        })
        .collect();
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 1., 0.]; vertices.len()]);
    m.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vertices.iter().map(|v| v.uv).collect::<Vec<_>>(),
    );
    m.insert_attribute(
        Mesh::ATTRIBUTE_UV_1,
        vertices
            .iter()
            .map(|v| [v.light[0] as f32 / 240., v.light[1] as f32 / 240.])
            .collect::<Vec<_>>(),
    );
    m.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        native.map_or_else(
            || vertices.iter().map(|v| v.color.map(|c| c as f32 / 255.)).collect::<Vec<_>>(),
            |vertices| vertices.iter().map(|v| v.color).collect::<Vec<_>>(),
        ),
    );
    m.insert_indices(Indices::U32(indices));
    m
}
/// Native mob alpha carries signed hurt overlays, not opacity; do not quantize it.
fn entity_mesh(data: terrain::mesh::ChunkMesh, origin:[f64;3]) -> Mesh {
    let vertices:Vec<_> = data.vertices.iter().map(terrain::mesh::SectionVertex::from_vertex).collect();
    mesh_data(&vertices, data.indices, origin, Some(&data.vertices))
}
fn remove(
    entity: Entity,
    state: &mut RenderWorld,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
) {
    commands.entity(entity).despawn();
    if let Some(h) = state.mesh_handles.remove(&entity) {
        meshes.remove(h.id());
    }
}

#[allow(clippy::too_many_arguments)]
fn render(
    mut commands: Commands,
    level: Res<LoadedLevel>,
    local: Res<LocalPlayer>,
    mut state: ResMut<RenderWorld>,
    mut atlas_updates: ResMut<AtlasUpdates>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<VoxelMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<&mut Camera, With<crate::player::PlayerCamera>>,
    mut visibility: Query<&mut Visibility>,
    windows: Query<&Window>,
    mut art: ResMut<crate::ui_assets::UiAssets>,
    mut building: ResMut<Building>,
    settings: Res<crate::settings::Settings>,
    mut stats: ResMut<crate::minecraft_graphics::RenderStats>,
    (enabled, loading): (Res<crate::player::InputEnabled>, Res<crate::loading::Loading>),
) {
    let Some(runtime) = &level.0.overworld else {
        return;
    };
    // Let the loading screen reach the display before the atlas and the
    // pre-built world are uploaded.
    if loading.holding() {
        return;
    }
    let mut runtime = runtime.lock().unwrap();
    let distance = std::env::var("CSR_RENDER_DISTANCE").ok().and_then(|s|s.parse().ok()).unwrap_or(settings.mc_render_distance);
    if state.render_distance != Some(distance) {
        runtime.set_render_distance(distance);
        state.render_distance=Some(distance);
    }
    if state.epoch != runtime.epoch {
        let entities: Vec<_> = state.sections.drain().flat_map(|(_, v)| v.into_iter().flatten()).collect();
        for e in entities {
            remove(e, &mut state, &mut commands, &mut meshes);
        }
        state.epoch = runtime.epoch;
    }
    if !state.initialized {
        building.selected = runtime.save.selected.min(8);
        building.active = true;
        building.initialized = true;
        if std::env::var_os("CSR_TEST_BUILD").is_some() {
            building.active = true;
            building.inventory_open = true;
            runtime.screen = overworld::survival::Screen::Inventory;
            if let Ok(screen) = std::env::var("CSR_TEST_SCREEN") {
                let (x, y, z) = overworld::block_pos(runtime.save.origin);
                if screen == "crafting" {
                    runtime.open_block((x + 1, y, z + 2));
                }
                if screen == "furnace" {
                    runtime.open_block((x + 2, y, z));
                }
            }
        }
        let a = &runtime.atlas.pixels;
        let mut atlas_image = image(a.as_raw(), a.width(), a.height());
        for mip in &runtime.atlas.mipmaps {
            atlas_image
                .data
                .as_mut()
                .unwrap()
                .extend_from_slice(mip.as_raw());
        }
        atlas_image.texture_descriptor.mip_level_count = 1 + runtime.atlas.mipmaps.len() as u32;
        atlas_image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            mag_filter: ImageFilterMode::Nearest,
            min_filter: ImageFilterMode::Nearest,
            mipmap_filter: ImageFilterMode::Linear,
            ..default()
        });
        state.atlas = images.add(atlas_image);
        let params = Parameters {
            daylight: Vec4::new(1., 3500., 0., 0.),
            fog: Vec4::new(0.35, 0.55, 0.8, 1.),
        };
        state.solid = materials.add(VoxelMaterial {
            params: params.clone(),
            atlas: state.atlas.clone(),
            transparent: false,
            multiply: false,
        });
        state.water = materials.add(VoxelMaterial {
            params,
            atlas: state.atlas.clone(),
            transparent: true,
            multiply: false,
        });
        state.cracks = materials.add(VoxelMaterial {
            params: Parameters {daylight: Vec4::new(1.,3500.,0.,2.),fog:Vec4::ZERO},
            atlas: state.atlas.clone(),transparent:true,multiply:true,
        });
        state.initialized = true;
        let pos = overworld::terrain::pack::ResourceId::parse("minecraft:block/grass_block_side")
            .unwrap();
        if let Ok(Some(bytes)) = runtime.packs.texture(&pos) {
            if let Ok(img) = Image::from_buffer(
                &bytes,
                bevy::image::ImageType::Extension("png"),
                bevy::image::CompressedImageFormats::NONE,
                true,
                ImageSampler::nearest(),
                RenderAssetUsages::default(),
            ) {
                art.images.insert("map-overworld".into(), images.add(img));
            }
        }
    }
    let p = overworld::to_block(runtime.save.origin, local.render_eye);
    let f = crate::coords::view_forward(local.render_angles.x, local.render_angles.y);
    let aspect = windows
        .iter()
        .next()
        .map_or(16. / 9., |w| w.width() / w.height());
    let camera = terrain::sections::CullCamera {
        position: glam_mc::DVec3::from_array(p),
        forward: glam_mc::Vec3::from_array(f.to_array()),
        fov_degrees: crate::VERTICAL_FOV_DEG,
        aspect,
        yaw_degrees: local.render_angles.y,
        pitch_degrees: local.render_angles.x,
    };
    let started=std::time::Instant::now();
    let update = runtime.frame(&camera);
    stats.terrain_ms=started.elapsed().as_secs_f32()*1000.;
    stats.uploads=update.uploads.len();
    let (pending,mean)=runtime.stream.lighting_stats();
    stats.pending_lights=pending;stats.worker_light_ms=mean as f32/1000.;
    stats.light_updates=update.lights.len();
    stats.visible_sections=update.visible.len();
    let started=std::time::Instant::now();
    let origin = runtime.save.origin;
    for pos in update.removed {
        if let Some(entities) = state.sections.remove(&pos) {
            for e in entities.into_iter().flatten() {
                remove(e, &mut state, &mut commands, &mut meshes);
            }
        }
    }
    for (pos, m) in update.uploads {
        use bevy::camera::primitives::MeshAabb;
        let split=m.transparent_start.unwrap_or(m.indices.len() as u32) as usize;
        let mut entities=state.sections.remove(&pos).unwrap_or([None;2]);
        for (pass,(ix,mat)) in [(m.indices[..split].to_vec(),state.solid.clone()),(m.indices[split..].to_vec(),state.water.clone())].into_iter().enumerate() {
            if ix.is_empty() {
                if let Some(e)=entities[pass].take(){remove(e,&mut state,&mut commands,&mut meshes);}
                continue;
            }
            let updated=mesh_part(&m.vertices,ix,origin);
            let bounds=updated.compute_aabb();
            let e=if let Some((e,h))=entities[pass].and_then(|e|state.mesh_handles.get(&e).cloned().map(|h|(e,h))) {
                let _=meshes.insert(h.id(),updated);
                e
            }else{
                let h=meshes.add(updated);
                let e=commands.spawn((Mesh3d(h.clone()),MeshMaterial3d(mat),Transform::IDENTITY)).id();
                state.mesh_handles.insert(e,h);entities[pass]=Some(e);e
            };
            // Bevy does not automatically refresh AABBs when mesh vertices change.
            if let Some(bounds)=bounds {commands.entity(e).insert(bounds);}
        }
        state.sections.insert(pos,entities);
    }
    let visible: std::collections::HashSet<_> =
        update.visible.into_iter().map(|(p, _)| p).collect();
    for (pos, entities) in &state.sections {
        for e in entities.iter().flatten() {
            if let Ok(mut v) = visibility.get_mut(*e) {
                let target = if visible.contains(pos) { Visibility::Inherited } else { Visibility::Hidden };
                if *v != target { *v=target; }
            }
        }
    }
    stats.sections=state.sections.len();
    stats.upload_ms=started.elapsed().as_secs_f32()*1000.;
    let started=std::time::Instant::now();
    let view=overworld::rendering::EntityView::new(&camera,settings.mc_entity_distance as f32,settings.mc_entity_culling);
    let mut mob = runtime.mob_mesh_for(Some(&view));
    if !enabled.0 && std::env::var_os("CSR_TEST_MOB_HURT").is_some() {
        state.preview_frame += 1;
        mob_preview::append(&mut runtime,&mut mob,&camera,state.preview_frame);
    }
    stats.mob_vertices=mob.vertices.len();
    if !mob.indices.is_empty() {
        let updated=entity_mesh(mob,origin);
        if let Some(h)=state.mob.and_then(|e|state.mesh_handles.get(&e)) {
            let _=meshes.insert(h.id(),updated);
        } else {
        let h = meshes.add(updated);
        let e = commands
            .spawn((
                Mesh3d(h.clone()),
                MeshMaterial3d(state.solid.clone()),
                // The aggregate is rebuilt with moving entities; its initial AABB becomes stale.
                // Individual mobs are conservatively culled before their geometry is generated.
                bevy::camera::visibility::NoFrustumCulling,
                Transform::IDENTITY,
            ))
            .id();
        state.mesh_handles.insert(e, h);
        state.mob = Some(e);
        }
    } else if let Some(e)=state.mob.take() {
        remove(e,&mut state,&mut commands,&mut meshes);
    }
    stats.entities_ms=started.elapsed().as_secs_f32()*1000.;
    let day = runtime.daylight();
    let fog = match runtime.save.adventure.dimension.as_str() {
        "nether" => Vec4::new(0.16, 0.025, 0.015, 1.),
        "end" => Vec4::new(0.035, 0.025, 0.05, 1.),
        _ => Vec4::new(0.30, 0.49, 0.78, 1.) * (0.15 + day * 0.85),
    };
    let distance = (runtime.stream.view_distance() as f32 * 16.).max(
        if runtime.save.adventure.dimension == "end" {
            144.
        } else {
            0.
        },
    ) * overworld::BLOCK;
    for h in [&state.solid, &state.water, &state.cracks] {
        if let Some(mut m) = materials.get_mut(h) {
            let daylight = Vec4::new(
                day,
                distance,
                if runtime.save.adventure.dimension == "overworld" {
                    0.025
                } else {
                    0.15
                },
                if m.multiply {2.} else if m.transparent { 1. } else { 0. },
            );
            if m.params.daylight!=daylight || m.params.fog!=fog {
                m.params.daylight=daylight;m.params.fog=fog;
            }
        }
    }
    for mut cam in &mut cameras {
        if cam.order == 0 {
            let color=Color::linear_rgb(fog.x, fog.y, fog.z);
            if !matches!(cam.clear_color,ClearColorConfig::Custom(current) if current==color) {cam.clear_color=ClearColorConfig::Custom(color);}
        }
    }
    let tick = runtime.save.day_ticks as u64;
    if tick != state.animation_tick {
        let mut patches = Vec::new();
        for tile in &runtime.atlas.animated_tiles {
            let current = tile.state_at(tick);
            let previous = tile.state_at(state.animation_tick);
            if current.current == previous.current && (!tile.interpolate || current == previous) {
                continue;
            }
            for level in 0..=runtime.atlas.mipmaps.len() {
                let frame = tile.image_at(current, level);
                patches.push(AtlasPatch {
                    level: level as u32,
                    origin: bevy::render::render_resource::Origin3d {
                        x: tile.origin.0 >> level,
                        y: tile.origin.1 >> level,
                        z: 0,
                    },
                    size: Extent3d {
                        width: frame.width(),
                        height: frame.height(),
                        depth_or_array_layers: 1,
                    },
                    pixels: std::sync::Arc::from(frame.as_raw().as_slice()),
                });
            }
        }
        if !patches.is_empty() {
            atlas_updates.atlas = state.atlas.clone();
            atlas_updates.serial += 1;
            atlas_updates.patches = patches;
        }
        state.animation_tick = tick;
    }
}

#[cfg(test)]
mod audio_distance_tests {
    use super::*;
    #[test]
    fn ladder_and_vine_placement_faces_the_supporting_wall() {
        for (normal,facing,support) in [(Vec3::X,"east","west"),(-Vec3::X,"west","east"),(Vec3::Y,"north","south"),(-Vec3::Y,"south","north")] {
            let mut b=Block::new("minecraft:ladder");assert!(orient_climbable(&mut b,normal));assert_eq!(b.properties["facing"],facing);
            let mut b=Block::new("minecraft:vine");assert!(orient_climbable(&mut b,normal));assert_eq!(b.properties[support],"true");
        }
        assert!(!orient_climbable(&mut Block::new("minecraft:ladder"),Vec3::Z));
        assert!(!orient_climbable(&mut Block::new("minecraft:vine"),Vec3::Z));
        let mut b=Block::new("minecraft:vine");assert!(orient_climbable(&mut b,-Vec3::Z));assert_eq!(b.properties["up"],"true");
    }
    #[test]
    fn distant_events_fade_and_do_not_play_beyond_their_radius() {
        let gain=|blocks:f32|audible_gain(blocks,16.)/(blocks*blocks).max(1.);
        assert_eq!(audible_gain(0.,16.),1.);
        assert!(gain(8.) < gain(2.)/10.);
        assert_eq!(gain(16.),0.);assert_eq!(gain(80.),0.);
        assert!(audible_gain(32.,64.)>0.);
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    #[test]
    #[ignore = "requires installed CS/MC assets and an isolated CSR_WORLD_DIR"]
    fn rmb_opens_blocks_in_weapon_mode_and_leaves_attack2_for_empty_space() {
        assert!(std::env::var_os("CSR_WORLD_DIR").is_some());
        let level=level::Level::load_seed("overworld",None,Some(853104)).unwrap();
        let shared=level.overworld.as_ref().unwrap();
        let mut world=sim::World::with_catalog(level.collision.clone(),level.catalog.clone());
        world.set_mode(sim::GameMode::Special,Default::default());
        let id=world.add_player("interaction test",sim::Team::CounterTerrorist);
        let (eye,target)={
            let mut w=shared.lock().unwrap();
            let (x,y,z)=overworld::block_pos(w.save.origin);
            for dx in -2..=4 {for dy in 0..=3 {for dz in -2..=2 {w.set_block((x+dx,y+dy,z+dz),None);}}}
            (overworld::to_source(w.save.origin,[x as f64+0.5,y as f64+1.62,z as f64+0.5]),(x+2,y+1,z))
        };
        let mut a=App::new();
        a.insert_resource(LoadedLevel(level)).insert_resource(SimWorld(world)).insert_resource(LocalPlayer::new(id,eye,0.))
            .init_resource::<Building>().init_resource::<crate::menu::Menu>().init_resource::<crate::buymenu::BuyMenu>()
            .init_resource::<crate::settings::Settings>().init_resource::<crate::player::InputBlockFrames>()
            .init_resource::<crate::tracers::InspectRequest>().insert_resource(crate::player::InputEnabled(true))
            .init_resource::<ButtonInput<KeyCode>>().init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<bevy::input::mouse::AccumulatedMouseMotion>().init_resource::<bevy::input::mouse::AccumulatedMouseScroll>()
            .add_systems(Update,(input,crate::player::look).chain());
        let cursor=a.world_mut().spawn((bevy::window::PrimaryWindow,bevy::window::CursorOptions{grab_mode:bevy::window::CursorGrabMode::Locked,visible:false,..default()})).id();
        for name in ["crafting_table","furnace","chest","oak_door","stone"] {
            {
                let loaded=a.world().resource::<LoadedLevel>();
                let mut w=loaded.0.overworld.as_ref().unwrap().lock().unwrap();w.close_screen();
                let block=if name=="oak_door" {Block::new("minecraft:oak_door").with("facing","west").with("half","lower").with("hinge","left").with("open","false").with("powered","false")} else {Block::new(&format!("minecraft:{name}"))};
                w.set_block(target,Some(block));
            }
            a.world_mut().entity_mut(cursor).get_mut::<bevy::window::CursorOptions>().unwrap().grab_mode=bevy::window::CursorGrabMode::Locked;
            a.world_mut().resource_mut::<Building>().inventory_open=false;
            a.world_mut().resource_mut::<crate::player::InputBlockFrames>().0=0;
            a.world_mut().resource_mut::<LocalPlayer>().latched=0;
            a.world_mut().resource_mut::<ButtonInput<MouseButton>>().reset_all();
            a.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Right);
            a.update();
            let expected=name!="stone";
            assert_eq!(a.world().resource::<Building>().using_block,expected,"{name}");
            assert_eq!(a.world().resource::<LocalPlayer>().latched&movement::buttons::ATTACK2!=0,!expected,"secondary action for {name}");
            a.world_mut().resource_mut::<ButtonInput<MouseButton>>().reset_all();a.update();
        }
    }
}

#[cfg(test)]
mod mesh_upload_tests {
    use super::*;
    #[test]
    fn material_pass_uploads_only_referenced_vertices() {
        let v=terrain::mesh::SectionVertex::from_vertex(&terrain::mesh::Vertex {position:[1.,2.,3.],uv:[0.,0.],color:[1.;4],sky_light:15.,block_light:0.});
        let mesh=mesh_part(&[v;100],vec![70,71,72,70,72,73],[0.;3]);
        assert_eq!(mesh.count_vertices(),4);
        assert_eq!(mesh.indices().unwrap().iter().collect::<Vec<_>>(),vec![0,1,2,0,2,3]);
    }
}

#[cfg(test)]
mod block_audio_integration {
    use super::*;
    use bevy::audio::{Decodable,PlaybackSettings};
    use overworld::block_audio::{Action,Sound};
    #[test]
    #[ignore="requires installed CS/MC assets and an isolated CSR_WORLD_DIR"]
    fn minecraft_block_audio_decodes_and_reaches_local_playback() {
        assert!(std::env::var_os("CSR_WORLD_DIR").is_some());
        let level=level::Level::load_seed("overworld",None,Some(853108)).unwrap();
        let mut world=sim::World::with_catalog(level.collision.clone(),level.catalog.clone());
        world.set_mode(sim::GameMode::Special,Default::default());
        let id=world.add_player("block sound test",sim::Team::CounterTerrorist);
        let feet={
            let mut r=level.overworld.as_ref().unwrap().lock().unwrap();
            let (x,y,z)=overworld::block_pos(r.save.origin);
            for dx in -1..=4 {for dy in 0..=3 {for dz in -1..=1 {assert!(r.set_block((x+dx,y+dy,z+dz),None));}}}
            let target=(x+2,y+1,z);
            assert!(r.place_block(target,Block::new("minecraft:stone")));
            assert_eq!(r.local_block_sounds.len(),1);
            assert!(r.local_block_sounds[0].event.ends_with("stone.place"));
            assert!(!r.place_block(target,Block::new("minecraft:stone")));
            assert_eq!(r.local_block_sounds.len(),1,"failed placement must be silent");
            r.local_block_sounds.clear();
            let feet=overworld::to_source(r.save.origin,[x as f64+0.5,y as f64,z as f64+0.5]);
            for _ in 0..200 {
                r.mine_survival(0.05,feet,0.,0.,true,true);
                if r.block_name(target).is_none(){break;}
            }
            assert!(r.block_name(target).is_none(),"survival mining must complete");
            assert!(r.local_block_sounds.iter().any(|s|s.event.ends_with("stone.hit")));
            assert_eq!(r.local_block_sounds.iter().filter(|s|s.event.ends_with("stone.break")).count(),1);
            r.mine_survival(0.05,feet,0.,0.,true,false);
            r.local_block_sounds.clear();
            let mut decoded=0;
            for block in ["stone","oak_planks","grass_block","dirt","glass","sand","white_wool","snow_block","deepslate"] {
                for action in [Action::Step,Action::Hit,Action::Break,Action::Place] {
                    let sound=Sound::for_block(&format!("minecraft:{block}"),action);
                    for variant in 0..8 {
                        let (_,bytes)=r.sound_bytes(&sound.event,variant).unwrap_or_else(||panic!("missing {}",sound.event));
                        let source=AudioSource{bytes:bytes.into()};
                        let samples:Vec<_>=source.decoder().collect();
                        assert!(!samples.is_empty());assert!(samples.iter().any(|s|s.abs()>0.0001));decoded+=1;
                    }
                    r.local_block_sounds.push(sound);
                }
            }
            println!("PASS: {decoded} original block sound variants decoded; real placement, mining hits and break completion routed.");
            for dx in 0..=4 {r.set_block((x+dx,y-1,z),Some(Block::new("minecraft:stone")));}
            feet
        };
        {let p=world.player_mut(id).unwrap();p.alive=true;p.movement.origin=feet;p.movement.on_ground=true;}
        let mut a=App::new();
        a.insert_resource(LoadedLevel(level)).insert_resource(SimWorld(world))
            .insert_resource(LocalPlayer::new(id,feet+Vec3::Z*64.,0.))
            .init_resource::<crate::settings::Settings>().init_resource::<Sounds>()
            .init_resource::<Assets<AudioSource>>().add_systems(Update,sounds);
        a.update();
        let count=|a:&mut App|a.world_mut().query_filtered::<&PlaybackSettings,With<MinecraftLocalSound>>().iter(a.world()).count();
        assert_eq!(count(&mut a),36);
        for p in a.world_mut().query_filtered::<&PlaybackSettings,With<MinecraftLocalSound>>().iter(a.world()) {
            assert!(!p.spatial,"own action audio must not fade with reach distance");assert!(p.volume.to_linear()>0.);assert!(p.speed>0.);
        }
        a.update();assert_eq!(count(&mut a),36,"idle must be silent");
        a.world_mut().resource_mut::<SimWorld>().0.player_mut(id).unwrap().movement.origin+=Vec3::X*overworld::BLOCK*2.;
        a.update();assert_eq!(count(&mut a),37,"walking must queue a native step");
        {let mut w=a.world_mut().resource_mut::<SimWorld>();let p=w.0.player_mut(id).unwrap();p.movement.on_ground=false;p.movement.origin+=Vec3::X*overworld::BLOCK*2.;}
        a.update();assert_eq!(count(&mut a),37,"air movement must be silent");
        a.world_mut().resource_mut::<crate::settings::Settings>().mc_blocks=0.;
        a.world().resource::<LoadedLevel>().0.overworld.as_ref().unwrap().lock().unwrap().block_sound("minecraft:stone",Action::Place);
        a.update();assert_eq!(count(&mut a),37,"block volume must mute new sounds");
        println!("PASS: local playback settings, walking/idle/air behavior and block-volume muting.");
    }
}

#[cfg(test)]
mod mob_overlay_tests {
    use super::*;
    #[test]
    fn native_hurt_overlay_survives_entity_mesh_upload() {
        let colors=[[0.7,0.7,0.7,-terrain::client_mobs::RED_OVERLAY_ALPHA],[1.;4]];
        let data=terrain::mesh::ChunkMesh{vertices:colors.iter().map(|&color| terrain::mesh::Vertex{position:[0.;3],uv:[0.;2],color,sky_light:15.,block_light:0.}).collect(),..default()};
        let mesh=entity_mesh(data,[0.;3]);
        let Some(bevy::mesh::VertexAttributeValues::Float32x4(actual))=mesh.attribute(Mesh::ATTRIBUTE_COLOR) else {panic!("missing native entity colors")};
        assert_eq!(actual.as_slice(),&colors);
    }
}
