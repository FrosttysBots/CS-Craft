//! Local CS:GO-style front end. All presented game modes are playable offline.
#[path = "world_seed.rs"]
mod world_seed;
pub(crate) use world_seed::input as seed_input;
use crate::{
    player::{InputEnabled, LocalPlayer},
    settings::{ACTIONS, RESOLUTIONS, Settings},
    ui_assets::UiAssets,
};
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Main,
    Inventory,
    Cases,
    Play,
    Pause,
    Settings,
    Graphics,
    Audio,
    Controls,
    Crosshair,
    Minecraft,
    MinecraftGraphics,
}
#[derive(Resource)]
pub struct Menu {
    pub page: Option<Page>,
    pub in_match: bool,
    pub maps: Vec<String>,
    pub map: usize,
    pub world_seed:i64,
    pub seed_edit:Option<world_seed::Editor>,
    pub deathmatch: bool,
    pub terrorist: bool,
    pub bots: u32,
    pub capture: Option<usize>,
    pub status: String,
    dirty: bool,
    scroll: f32,
    size: Vec2,
    root: Option<Entity>,
}
impl Default for Menu {
    fn default() -> Self {
        Self {
            page: None,
            in_match: true,
            maps: Vec::new(),
            map: 0,
            world_seed:42,seed_edit:None,
            deathmatch: false,
            terrorist: false,
            bots: 5,
            capture: None,
            status: String::new(),
            dirty: true,
            scroll: 0.0,
            size: Vec2::ZERO,
            root: None,
        }
    }
}
impl Menu {
    pub fn open(&self) -> bool {
        self.page.is_some()
    }
}
#[derive(Resource, Clone, Default)]
pub struct Relaunch(pub Arc<Mutex<Option<crate::ClientOptions>>>);
#[derive(Component, Clone, Copy)]
pub enum Action {
    Page(Page),
    Resume,
    Disconnect,
    Quit,
    Start,
    SelectMap(usize),
    EditSeed,
    NewSeed,
    ApplySeed,
    SelectWeapon(usize),
    Filter(usize),
    Equip(bool),
    Rotate(f32),
    Mode,
    Team,
    Bots(i32),
    Adjust(u8, f32),
    Bind(usize),
    Save,
    Defaults,
}

pub(crate) fn setup(
    mut menu: ResMut<Menu>,
    opts: Res<crate::Options>,
    level: Res<crate::LoadedLevel>,
    mut clock: ResMut<Time<Virtual>>,
) {
    let mut maps: Vec<String> = std::fs::read_dir(level.0.game_dir.join("maps"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| e.path().file_name()?.to_str().map(str::to_string))
        .filter_map(|n| n.strip_suffix(".bsp").map(str::to_string))
        .collect();
    maps.extend(
        level
            .0
            .vfs
            .archive_paths_with_prefix("maps/")
            .iter()
            .filter_map(|p| {
                p.strip_prefix("maps/")?
                    .strip_suffix(".bsp")
                    .map(str::to_string)
            }),
    );
    maps.retain(|p| !p.contains('/') && (p.starts_with("de_") || p.starts_with("cs_")));
    maps.push("overworld".into());
    maps.sort();
    maps.dedup();
    if maps.is_empty() {
        maps.push(opts.0.map.clone());
    }
    menu.world_seed=opts.0.world_seed;
    menu.map = maps.iter().position(|p| p == &opts.0.map).unwrap_or(0);
    menu.maps = maps;
    menu.deathmatch = opts.0.deathmatch || menu.maps[menu.map].starts_with("cs_");
    menu.terrorist = opts.0.terrorist;
    menu.bots = if overworld::is_map(&menu.maps[menu.map]){0}else{opts.0.bots};
    let test = std::env::var("CSR_TEST_MENU").ok();
    if opts.0.screenshot.is_some() && std::env::var_os("CSR_TEST_SEED").is_some() {menu.seed_edit=Some(world_seed::Editor::new("-8675309123456789".into()));}
    if (!opts.0.direct && opts.0.screenshot.is_none()) || test.is_some() {
        menu.page = Some(match test.as_deref() {
            Some("minecraft") => Page::Minecraft,
            Some("mcgraphics") => Page::MinecraftGraphics,
            Some("pause") => Page::Pause,
            Some("graphics") => Page::Graphics,
            Some("audio") => Page::Audio,
            Some("controls") => Page::Controls,
            Some("crosshair") => Page::Crosshair,
            Some("play") => Page::Play,
            Some("inventory") => Page::Inventory,
            Some("cases") => Page::Cases,
            _ => Page::Main,
        });
        menu.in_match = test.as_deref().is_some_and(|v| matches!(v,"pause"|"minecraft"|"mcgraphics")) || (test.is_some() && std::env::var_os("CSR_TEST_PAUSED").is_some());
        clock.pause();
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn input(
    mut commands: Commands,
    mut menu: ResMut<Menu>,
    mut settings: ResMut<Settings>,
    keys: Res<ButtonInput<KeyCode>>,
    actions: Query<(&Interaction, &Action), Changed<Interaction>>,
    mut exit: MessageWriter<AppExit>,
    relaunch: Res<Relaunch>,
    opts: Res<crate::Options>,
    mut local: ResMut<LocalPlayer>,
    mut buy: ResMut<crate::buymenu::BuyMenu>,
    mut clock: ResMut<Time<Virtual>>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    enabled: Res<InputEnabled>,
    mut inventory: ResMut<crate::inventory::Inventory>,
    collection: Res<crate::inventory::Collection>,
    building: Res<crate::overworld_client::Building>,
) {
    if !enabled.0 {
        return;
    }
    let was_mc = matches!(menu.page,Some(Page::Minecraft | Page::MinecraftGraphics));
    let was_open = menu.open();
    if keys.just_pressed(KeyCode::KeyO) && building.initialized && menu.in_match && !building.inventory_open && menu.capture.is_none() {
        menu.page = if matches!(menu.page,Some(Page::Minecraft | Page::MinecraftGraphics)) { None } else {Some(Page::Minecraft)};
        menu.dirty=true;
    }
    if let Some(index) = menu.capture {
        if keys.just_pressed(KeyCode::Escape) {
            menu.capture = None;
            menu.dirty = true;
        } else if let Some(key) = keys
            .get_just_pressed()
            .find(|k| crate::settings::KEYS.contains(k))
        {
            settings.rebind(index, *key);
            menu.capture = None;
            menu.dirty = true;
        }
    } else if keys.just_pressed(KeyCode::Escape) && !buy.open {
        menu.page = match menu.page {
            None => Some(Page::Pause),
            Some(Page::Pause | Page::Minecraft) => None,
            Some(Page::MinecraftGraphics) => Some(Page::Minecraft),
            Some(Page::Main) => Some(Page::Main),
            _ => Some(if menu.in_match {
                Page::Pause
            } else {
                Page::Main
            }),
        };
        menu.dirty = true;
    }
    if menu.open() && menu.capture.is_none() {
        for (interaction, action) in &actions {
            if *interaction != Interaction::Pressed {
                continue;
            }
            menu.dirty = true;
            menu.status.clear();
            match *action {
                Action::Page(p) => {
                    menu.page = Some(p);
                    menu.scroll = 0.0;
                }
                Action::Resume => {
                    if menu.in_match {
                        menu.page = None;
                    }
                }
                Action::Disconnect => {
                    menu.in_match = false;
                    menu.page = Some(Page::Main);
                }
                Action::Quit => {
                    exit.write(AppExit::Success);
                }
                Action::EditSeed=>{if menu.seed_edit.is_none() {menu.seed_edit=Some(world_seed::Editor::new(menu.world_seed.to_string()));}}
                Action::ApplySeed=>{world_seed::commit(&mut menu);}
                Action::NewSeed=>{menu.seed_edit=None;menu.world_seed=world_seed::random_seed();}
                Action::SelectMap(index) => {
                    if index < menu.maps.len() {
                        menu.map = index;
                        if overworld::is_map(&menu.maps[index]){menu.bots=0;}
                        if menu.maps[index].starts_with("cs_") || overworld::is_map(&menu.maps[index]) {
                            menu.deathmatch = true;
                        }
                    }
                }
                Action::SelectWeapon(index) => {
                    if let Some(def) = collection.0.weapons.get(index) {
                        inventory.selected = def.class_name.clone();
                        inventory.yaw = 0.0;
                    }
                }
                Action::Filter(filter) => {
                    inventory.filter = filter;
                    menu.scroll = 0.0;
                }
                Action::Rotate(delta) => {
                    inventory.yaw += delta;
                    menu.dirty = false;
                }
                Action::Equip(ct) => {
                    if inventory.equip(&collection.0, ct) {
                        menu.status = match inventory.save() {
                            Ok(()) => {
                                "Loadout saved. Applied on your next Deathmatch spawn.".into()
                            }
                            Err(e) => format!("Could not save loadout: {e}"),
                        };
                    } else {
                        menu.status =
                            "This weapon cannot fill a primary or pistol slot for that team."
                                .into();
                    }
                }
                Action::Mode => {
                    if menu.maps[menu.map].starts_with("de_") {
                        menu.deathmatch = !menu.deathmatch;
                    } else {
                        menu.status = if overworld::is_map(&menu.maps[menu.map]){"Special mode: solo, no bots, unlimited time."}else{"Hostage maps currently support Deathmatch."}.into();
                    }
                }
                Action::Team => menu.terrorist = !menu.terrorist,
                Action::Bots(d) => menu.bots = if overworld::is_map(&menu.maps[menu.map]){0}else{(menu.bots as i32 + d).clamp(0, 10) as u32},
                Action::Bind(i) => {
                    menu.capture = Some(i);
                    menu.status =
                        "Press a keyboard key. Esc cancels. Existing bindings swap.".into();
                }
                Action::Defaults => {
                    *settings = Settings::default();
                    menu.status = "Defaults restored. Apply to save.".into();
                }
                Action::Save => {
                    menu.status = match settings.save() {
                        Ok(()) => "Settings saved.".into(),
                        Err(e) => format!("Could not save settings: {e}"),
                    };
                }
                Action::Start => match settings.save() {
                    Err(e) => menu.status = format!("Could not save settings: {e}"),
                    Ok(()) => {
                        let mut next = opts.0.clone();
                        next.map = menu.maps[menu.map].clone();
                        next.world_seed=menu.world_seed;
                        next.deathmatch = menu.deathmatch;
                        next.terrorist = menu.terrorist;
                        next.bots = if overworld::is_map(&next.map){0}else{menu.bots};
                        next.direct = true;
                        next.screenshot = None;
                        next.camera = None;
                        *relaunch.0.lock().unwrap() = Some(next);
                        exit.write(AppExit::Success);
                    }
                },
                Action::Adjust(id, delta) => match id {
                    0 => settings.volume = (settings.volume + delta).clamp(0.0, 1.0),
                    1 => {
                        settings.sensitivity = ((settings.sensitivity + delta) * 10.0)
                            .round()
                            .clamp(1.0, 100.0)
                            / 10.0
                    }
                    2 => settings.exposure = (settings.exposure + delta).clamp(0.5, 4.0),
                    3 => settings.vsync = !settings.vsync,
                    4 => settings.fullscreen = !settings.fullscreen,
                    5 => {
                        settings.resolution =
                            (settings.resolution as i32 + delta as i32).rem_euclid(4) as usize
                    }
                    6 => settings.msaa = !settings.msaa,
                    7 => settings.blood = !settings.blood,
                    8 => settings.hitmarker = !settings.hitmarker,
                    9 => settings.invert_y = !settings.invert_y,
                    10 => {
                        settings.cross_color =
                            (settings.cross_color as i32 + delta as i32).rem_euclid(5) as usize
                    }
                    11 => settings.cross_size = (settings.cross_size + delta).clamp(2.0, 20.0),
                    12 => settings.cross_gap = (settings.cross_gap + delta).clamp(0.0, 16.0),
                    13 => {
                        settings.cross_thickness =
                            (settings.cross_thickness + delta).clamp(1.0, 6.0)
                    }
                    14 => settings.cross_dot = !settings.cross_dot,
                    20 => settings.mc_mobs=(settings.mc_mobs+delta).clamp(0.,1.),
                    21 => settings.mc_blocks=(settings.mc_blocks+delta).clamp(0.,1.),
                    22 => settings.mc_minimap=!settings.mc_minimap,
                    23 => settings.mc_hand_motion=!settings.mc_hand_motion,
                    24 => settings.menu_blur=!settings.menu_blur,
                    25 => settings.mc_render_distance=(settings.mc_render_distance+delta as i32).clamp(3,16),
                    26 => settings.mc_entity_distance=(settings.mc_entity_distance+delta as i32).clamp(32,192),
                    27 => settings.mc_entity_culling=!settings.mc_entity_culling,
                    28 => settings.mc_clouds=(settings.mc_clouds+1)%3,
                    29 => settings.mc_show_fps=!settings.mc_show_fps,
                    _ => {}
                },
            }
        }
    }
    if was_mc && !matches!(menu.page,Some(Page::Minecraft | Page::MinecraftGraphics)) {
        if let Err(e)=settings.save() { menu.status=format!("Could not save settings: {e}"); menu.page=Some(Page::Minecraft); menu.dirty=true; }
    }
    if menu.open() {
        buy.open = false;
        buy.just_closed = false;
        local.latched = 0;
        local.pending_drop = false;
        local.pending_slot = 0;
        local.pending_buys.clear();
        clock.pause();
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    } else if was_open {
        clock.unpause();
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
        // A UI click cannot also fire in the resumed match.
        commands.insert_resource(crate::player::InputBlockFrames(2));
    }
}

pub(crate) fn label(
    commands: &mut Commands,
    parent: Entity,
    text: impl Into<String>,
    size: f32,
    color: Color,
) -> Entity {
    let e = commands
        .spawn((
            Text::new(text),
            TextFont {
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(color),
        ))
        .id();
    commands.entity(parent).add_children(&[e]);
    e
}
fn button(
    commands: &mut Commands,
    parent: Entity,
    text: impl Into<String>,
    action: Action,
    width: Val,
) -> Entity {
    let e = commands
        .spawn((
            Button,
            action,
            Node {
                width,
                min_height: px(34.0),
                padding: UiRect::axes(px(10.0), px(7.0)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.30, 0.36, 0.40, 0.68)),
        ))
        .id();
    commands.entity(parent).add_children(&[e]);
    label(commands, e, text, 16.0, Color::srgb(0.93, 0.95, 0.96));
    e
}
fn column(commands: &mut Commands, parent: Entity, node: Node) -> Entity {
    let e = commands.spawn(node).id();
    commands.entity(parent).add_children(&[e]);
    e
}
fn row(commands: &mut Commands, parent: Entity, name: &str, value: String, id: u8, step: f32) {
    let r = column(
        commands,
        parent,
        Node {
            width: percent(100),
            height: px(38.0),
            align_items: AlignItems::Center,
            column_gap: px(10.0),
            ..default()
        },
    );
    let left = column(
        commands,
        r,
        Node {
            width: percent(48),
            ..default()
        },
    );
    label(commands, left, name, 18.0, Color::srgb(0.80, 0.85, 0.89));
    button(commands, r, "−", Action::Adjust(id, -step), px(38.0));
    let middle = column(
        commands,
        r,
        Node {
            width: px(160.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
    );
    label(commands, middle, value, 18.0, Color::WHITE);
    button(commands, r, "+", Action::Adjust(id, step), px(38.0));
}

fn building_available(menu:&Menu)->bool {menu.maps.get(menu.map).is_some_and(|m|overworld::is_map(m))}

pub fn draw(
    mut commands: Commands,
    mut menu: ResMut<Menu>,
    settings: Res<Settings>,
    art: Res<UiAssets>,
    inventory: Res<crate::inventory::Inventory>,
    collection: Res<crate::inventory::Collection>,
    preview: Res<crate::menu_scene::Preview>,
    loc: Res<crate::localize::Localization>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    let size = Vec2::new(window.width(), window.height());
    if menu.size != size {
        menu.size = size;
        menu.dirty = true;
    }
    if !menu.dirty && !settings.is_changed() {
        return;
    }
    menu.dirty = false;
    if let Some(root) = menu.root.take() {
        commands.entity(root).despawn();
    }
    let Some(page) = menu.page else {
        return;
    };
    if matches!(page,Page::Minecraft | Page::MinecraftGraphics) { return; }
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            BackgroundColor(if menu.in_match {Color::srgba(0.02,0.03,0.04,0.36)} else {Color::srgb(0.035, 0.055, 0.072)}),
            GlobalZIndex(100),
        ))
        .id();
    menu.root = Some(root);
    if !menu.in_match && matches!(page, Page::Main | Page::Play | Page::Inventory | Page::Cases) {
        if let Some(img) = art.image("menu-background") {
            let backdrop = commands
                .spawn((
                    ImageNode {
                        color: Color::srgb(0.60, 0.65, 0.69),
                        ..ImageNode::new(img)
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        width: percent(100),
                        height: percent(100),
                        ..default()
                    },
                ))
                .id();
            commands.entity(root).add_children(&[backdrop]);
        }
    }
    let rail = column(
        &mut commands,
        root,
        Node {
            width: px(116.0),
            flex_shrink: 0.0,
            height: percent(100),
            padding: UiRect::axes(px(8.0), px(26.0)),
            flex_direction: FlexDirection::Column,
            row_gap: px(12.0),
            ..default()
        },
    );
    commands
        .entity(rail)
        .insert(BackgroundColor(Color::srgba(0.025, 0.04, 0.052, 0.90)));
    if let Some(logo) = art.image("logo") {
        let e = commands
            .spawn((
                ImageNode::new(logo),
                Node {
                    width: px(100.0),
                    height: px(45.0),
                    margin: UiRect::bottom(px(26.0)),
                    ..default()
                },
            ))
            .id();
        commands.entity(rail).add_children(&[e]);
    }

    for (name, p) in [
        ("HOME", Page::Main),
        ("PLAY", Page::Play),
        ("INVENTORY", Page::Inventory),
        ("CASES", Page::Cases),
        ("SETTINGS", Page::Settings),
    ] {
        button(&mut commands, rail, name, Action::Page(p), percent(100));
    }
    if menu.in_match {
        button(&mut commands, rail, "RESUME", Action::Resume, percent(100));
        button(
            &mut commands,
            rail,
            "LEAVE",
            Action::Disconnect,
            percent(100),
        );
    }
    column(
        &mut commands,
        rail,
        Node {
            flex_grow: 1.0,
            ..default()
        },
    );
    button(&mut commands, rail, "QUIT", Action::Quit, percent(100));
    let content = column(
        &mut commands,
        root,
        Node {
            flex_grow: 1.0,
            height: percent(100),
            padding: UiRect::axes(px(26.0), px(22.0)),
            min_width: px(0.0),
            flex_direction: FlexDirection::Column,
            row_gap: px(12.0),
            ..default()
        },
    );
    label(
        &mut commands,
        content,
        match page {
            Page::Main => "COUNTER-STRIKE: GLOBAL OFFENSIVE",
            Page::Inventory => "INVENTORY",
            Page::Cases => "INVENTORY / CASES & SKINS",
            Page::Play if overworld::is_map(&menu.maps[menu.map]) => "PLAY  /  SPECIAL  /  OVERWORLD",
            Page::Play => "PLAY  /  PRACTICE WITH BOTS",
            Page::Pause => "MATCH PAUSED",
            Page::Settings => "GAME SETTINGS",
            Page::Graphics => "VIDEO SETTINGS",
            Page::Audio => "AUDIO SETTINGS",
            Page::Controls => "KEYBOARD / MOUSE",
            Page::Crosshair => "CROSSHAIR",
            Page::Minecraft => "Minecraft Options",
            Page::MinecraftGraphics => "Minecraft Video Settings",
        },
        30.0,
        Color::WHITE,
    );
    let settings_page = matches!(
        page,
        Page::Settings | Page::Graphics | Page::Audio | Page::Controls | Page::Crosshair
    );
    if settings_page {
        let tabs = column(
            &mut commands,
            content,
            Node {
                column_gap: px(8.0),
                flex_wrap: FlexWrap::Wrap,
                ..default()
            },
        );
        for (name, p) in [
            ("GAME", Page::Settings),
            ("VIDEO", Page::Graphics),
            ("AUDIO", Page::Audio),
            ("CONTROLS", Page::Controls),
            ("CROSSHAIR", Page::Crosshair),
        ] {
            button(&mut commands, tabs, name, Action::Page(p), Val::Auto);
        }
    }
    let body = column(
        &mut commands,
        content,
        Node {
            flex_grow: 1.0,
            flex_basis: px(0.0),
            min_height: px(0.0),
            width: percent(100),
            flex_direction: FlexDirection::Column,
            row_gap: px(if page == Page::Controls { 2.0 } else { 5.0 }),
            ..default()
        },
    );
    let on = |v: bool| if v { "Enabled" } else { "Disabled" }.to_string();
    match page {
        Page::Cases | Page::Minecraft | Page::MinecraftGraphics => {}
        Page::Main => {
            let scene = column(
                &mut commands,
                body,
                Node {
                    flex_grow: 1.0,
                    width: percent(100),
                    ..default()
                },
            );
            let model = commands
                .spawn((
                    ImageNode::new(preview.image.clone()),
                    Node {
                        position_type: PositionType::Absolute,
                        right: px(100.0),
                        bottom: px(0.0),
                        height: percent(100),
                        aspect_ratio: Some(0.9),
                        ..default()
                    },
                ))
                .id();
            commands.entity(scene).add_children(&[model]);
            let news = column(
                &mut commands,
                scene,
                Node {
                    width: px(350.0),
                    padding: UiRect::all(px(20.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(18.0),
                    align_self: AlignSelf::FlexStart,
                    margin: UiRect::top(px(26.0)),
                    ..default()
                },
            );
            commands
                .entity(news)
                .insert(BackgroundColor(Color::srgba(0.04, 0.065, 0.085, 0.84)));
            label(
                &mut commands,
                news,
                "PLAY COUNTER-STRIKE",
                28.0,
                Color::WHITE,
            );
            if let Some(img) = art.image("map-de_dust2") {
                picture(&mut commands, news, img, percent(100), px(150.0));
            }
            label(
                &mut commands,
                news,
                "LOCAL PLAY",
                21.0,
                Color::WHITE,
            );
            label(
                &mut commands,
                news,
                "Competitive, Deathmatch or solo Overworld Special mode. Choose a map to begin.",
                19.0,
                Color::srgb(0.72, 0.78, 0.82),
            );
            button(
                &mut commands,
                news,
                "PLAY",
                Action::Page(Page::Play),
                percent(100),
            );
            button(
                &mut commands,
                news,
                "OPEN INVENTORY",
                Action::Page(Page::Inventory),
                percent(100),
            );
            let profile = column(
                &mut commands,
                scene,
                Node {
                    position_type: PositionType::Absolute,
                    right: px(0.0),
                    top: px(0.0),
                    width: px(150.0),
                    height: percent(100),
                    padding: UiRect::all(px(16.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(15.0),
                    ..default()
                },
            );
            commands
                .entity(profile)
                .insert(BackgroundColor(Color::srgba(0.025, 0.04, 0.055, 0.78)));
            if let Some(img) = art.image("ct") {
                picture(&mut commands, profile, img, px(64.0), px(64.0));
            }
            label(&mut commands, profile, "YOU", 24.0, Color::WHITE);
            label(
                &mut commands,
                profile,
                "LOCAL PROFILE",
                14.0,
                Color::srgb(0.70, 0.78, 0.84),
            );
            label(
                &mut commands,
                profile,
                "Offline practice",
                18.0,
                Color::srgb(0.72, 0.82, 0.62),
            );
            column(
                &mut commands,
                profile,
                Node {
                    flex_grow: 1.0,
                    ..default()
                },
            );
            label(
                &mut commands,
                profile,
                "CS:GO LEGACY",
                15.0,
                Color::srgb(0.7, 0.76, 0.8),
            );
            label(
                &mut commands,
                body,
                "LOCAL PLAY  /  BASE WEAPON COLLECTION  /  PRACTICE LOADOUTS",
                16.0,
                Color::srgb(0.83, 0.86, 0.88),
            );
        }
        Page::Inventory => {
            button(&mut commands,body,"CASES & OWNED SKINS",Action::Page(Page::Cases),px(240.));
            let tabs = column(
                &mut commands,
                body,
                Node {
                    column_gap: px(5.0),
                    ..default()
                },
            );
            for (i, name) in crate::inventory::FILTERS.iter().enumerate() {
                button(
                    &mut commands,
                    tabs,
                    if inventory.filter == i {
                        format!("[{name}]")
                    } else {
                        name.to_string()
                    },
                    Action::Filter(i),
                    Val::Auto,
                );
            }
            let area = column(
                &mut commands,
                body,
                Node {
                    flex_grow: 1.0,
                    flex_basis: px(0.0),
                    min_height: px(0.0),
                    column_gap: px(18.0),
                    ..default()
                },
            );
            let grid = scroll_grid(&mut commands, area, menu.scroll);
            let width = ((size.x - 600.0) / 4.0 - 12.0).max(112.0);
            for (i, def) in collection
                .0
                .weapons
                .iter()
                .enumerate()
                .filter(|(_, d)| crate::inventory::visible(d, inventory.filter))
            {
                let selected = def.class_name == inventory.selected;
                let title = loc.weapon_name(def);
                let card = card(
                    &mut commands,
                    grid,
                    &title,
                    Action::SelectWeapon(i),
                    selected,
                    width,
                    110.0,
                );
                if let Some(img) = art.image(&format!(
                    "item-{}",
                    def.class_name.trim_start_matches("weapon_")
                )) {
                    card_image(&mut commands, card, img, 68.0);
                }
            }
            let detail = column(
                &mut commands,
                area,
                Node {
                    width: px(360.0),
                    flex_shrink: 0.0,
                    padding: UiRect::all(px(18.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(6.0),
                    ..default()
                },
            );
            commands
                .entity(detail)
                .insert(BackgroundColor(Color::srgba(0.045, 0.065, 0.082, 0.92)));
            if let Some(def) = collection.0.get(&inventory.selected) {
                label(
                    &mut commands,
                    detail,
                    loc.weapon_name(def),
                    30.0,
                    Color::WHITE,
                );
                label(
                    &mut commands,
                    detail,
                    "BASE ITEM  •  LOCAL COLLECTION",
                    14.0,
                    Color::srgb(0.68, 0.74, 0.79),
                );
                picture(
                    &mut commands,
                    detail,
                    preview.image.clone(),
                    percent(100),
                    px((size.y - 580.0).clamp(130.0, 310.0)),
                );
                let turn = column(
                    &mut commands,
                    detail,
                    Node {
                        column_gap: px(8.0),
                        ..default()
                    },
                );
                button(
                    &mut commands,
                    turn,
                    "ROTATE LEFT",
                    Action::Rotate(-0.3),
                    percent(50),
                );
                button(
                    &mut commands,
                    turn,
                    "ROTATE RIGHT",
                    Action::Rotate(0.3),
                    percent(50),
                );
                label(
                    &mut commands,
                    detail,
                    format!(
                        "Damage {}   •   {} RPM   •   {} rounds",
                        def.damage as i32,
                        def.rpm() as i32,
                        def.clip_size.max(0)
                    ),
                    16.0,
                    Color::srgb(0.77, 0.82, 0.86),
                );
                let equip = column(
                    &mut commands,
                    detail,
                    Node {
                        column_gap: px(6.0),
                        ..default()
                    },
                );
                for (ct, name) in [(false, "EQUIP FOR T"), (true, "EQUIP FOR CT")] {
                    if crate::inventory::allowed(def, ct) {
                        button(&mut commands, equip, name, Action::Equip(ct), Val::Auto);
                    }
                }
            }
            label(
                &mut commands,
                detail,
                "DEATHMATCH LOADOUTS",
                17.0,
                Color::WHITE,
            );
            for (i, pair) in inventory.loadouts.chunks(2).enumerate() {
                let display = pair
                    .iter()
                    .map(|name| {
                        collection
                            .0
                            .get(name)
                            .map(|d| loc.weapon_name(d))
                            .unwrap_or_else(|| name.clone())
                    })
                    .collect::<Vec<_>>()
                    .join(" / ");
                label(
                    &mut commands,
                    detail,
                    format!("{}  {}", ["T", "CT"][i], display),
                    15.0,
                    Color::srgb(0.7, 0.76, 0.8),
                );
            }
            label(
                &mut commands,
                body,
                "Equip applies on your next Deathmatch spawn. Competitive retains its buy economy.",
                16.0,
                Color::srgb(0.83, 0.86, 0.88),
            );
        }
        Page::Pause => {
            if building_available(&menu) { button(&mut commands,body,"MINECRAFT OPTIONS  [O]",Action::Page(Page::Minecraft),px(320.)); }
            label(
                &mut commands,
                body,
                "Local match paused",
                30.0,
                Color::WHITE,
            );
            label(
                &mut commands,
                body,
                "Resume your match or adjust your settings.",
                21.0,
                Color::srgb(0.7, 0.76, 0.8),
            );
            button(
                &mut commands,
                body,
                "RESUME GAME",
                Action::Resume,
                px(280.0),
            );
        }
        Page::Play => {
            let options = column(
                &mut commands,
                body,
                Node {
                    column_gap: px(10.0),
                    align_items: AlignItems::Center,
                    ..default()
                },
            );
            button(
                &mut commands,
                options,
                format!(
                    "MODE  •  {}",
                    if overworld::is_map(&menu.maps[menu.map]) {"Special"} else if menu.deathmatch {
                        "Deathmatch"
                    } else {
                        "Competitive"
                    }
                ),
                Action::Mode,
                px(260.0),
            );
            if !overworld::is_map(&menu.maps[menu.map]) {
            button(
                &mut commands,
                options,
                format!(
                    "TEAM  •  {}",
                    if menu.terrorist {
                        "Terrorists"
                    } else {
                        "Counter-Terrorists"
                    }
                ),
                Action::Team,
                px(260.0),
            );
            button(&mut commands, options, "−", Action::Bots(-1), px(34.0));
            label(
                &mut commands,
                options,
                format!(
                    "{} enemies / {} allies",
                    menu.bots,
                    menu.bots.saturating_sub(1)
                ),
                17.0,
                Color::WHITE,
            );
            button(&mut commands, options, "+", Action::Bots(1), px(34.0));
            } else {
                label(&mut commands,options,"SOLO  •  NO BOTS  •  UNLIMITED TIME",17.,Color::WHITE);
            }
            if overworld::is_map(&menu.maps[menu.map]) {
                let row=column(&mut commands,body,Node{column_gap:px(10.),align_items:AlignItems::Center,..default()});
                label(&mut commands,row,"WORLD SEED",15.,Color::WHITE);
                let value=menu.seed_edit.as_ref().map_or_else(||menu.world_seed.to_string(),world_seed::Editor::display);
                let field=button(&mut commands,row,value,Action::EditSeed,px(370.));
                if menu.seed_edit.is_some() { commands.entity(field).insert(BackgroundColor(Color::srgba(0.16,0.40,0.57,0.95))); }
                button(&mut commands,row,"APPLY",Action::ApplySeed,px(80.));
                button(&mut commands,row,"RANDOM SEED",Action::NewSeed,px(145.));
                label(&mut commands,body,"Click to edit • Numbers or words • Ctrl+A replace • Ctrl+V paste • Enter / Apply to save • Esc cancel",14.,Color::srgb(0.80,0.86,0.90));
                label(&mut commands,body,"The same seed resumes its saved world. A blank field chooses a random seed.",14.,Color::srgb(0.80,0.86,0.90));
            }
            let grid = scroll_grid(&mut commands, body, menu.scroll);
            let cols = if size.x >= 1200.0 { 6.0 } else { 5.0 };
            let width = ((size.x - 168.0 - (cols - 1.0) * 12.0 - 20.0) / cols).floor();
            let card_height = if size.y < 800.0 { 88.0 } else { 118.0 };
            for (i, name) in menu.maps.iter().enumerate() {
                let title = name
                    .trim_start_matches("de_")
                    .trim_start_matches("cs_")
                    .replace('_', " ")
                    .to_uppercase();
                let c = card(
                    &mut commands,
                    grid,
                    &title,
                    Action::SelectMap(i),
                    menu.map == i,
                    width,
                    card_height,
                );
                if let Some(img) = art.image(&format!("map-{name}")) {
                    // Crop the installed 640×360 image to fill the short map card.
                    let crop = 640.0 * (card_height - 32.0) / (width - 4.0);
                    let banner = commands
                        .spawn((
                            ImageNode {
                                image: img,
                                rect: if overworld::is_map(name){None}else{Some(Rect::new(
                                    0.0,
                                    (360.0 - crop) * 0.5,
                                    640.0,
                                    (360.0 + crop) * 0.5,
                                ))},
                                image_mode: bevy::ui::widget::NodeImageMode::Stretch,
                                ..default()
                            },
                            Node {
                                position_type: PositionType::Absolute,
                                top: px(0.0),
                                left: px(0.0),
                                width: percent(100),
                                height: px(card_height - 32.0),
                                ..default()
                            },
                        ))
                        .id();
                    commands.entity(c).add_children(&[banner]);
                }
            }
            let footer = column(
                &mut commands,
                body,
                Node {
                    column_gap: px(20.0),
                    align_items: AlignItems::Center,
                    margin: UiRect::top(px(8.0)),
                    ..default()
                },
            );
            let selected = menu.maps.get(menu.map).map(String::as_str).unwrap_or("");
            label(
                &mut commands,
                footer,
                format!("SELECTED  •  {selected}"),
                21.0,
                Color::WHITE,
            );
            column(
                &mut commands,
                footer,
                Node {
                    flex_grow: 1.0,
                    ..default()
                },
            );
            let go = button(&mut commands, footer, "GO", Action::Start, px(180.0));
            commands
                .entity(go)
                .insert(BackgroundColor(Color::srgb(0.28, 0.43, 0.22)));
        }
        Page::Settings => {
            row(
                &mut commands,
                body,
                "Blood effects",
                on(settings.blood),
                7,
                1.0,
            );
            row(
                &mut commands,
                body,
                "Optional hit confirmation",
                on(settings.hitmarker),
                8,
                1.0,
            );
            label(
                &mut commands,
                body,
                "Hurt sounds and directional damage feedback are always active.",
                18.0,
                Color::srgb(0.65, 0.73, 0.8),
            );
        }
        Page::Graphics => {
            row(
                &mut commands,
                body,
                "Display mode",
                if settings.fullscreen {
                    "Borderless fullscreen"
                } else {
                    "Windowed"
                }
                .into(),
                4,
                1.0,
            );
            let (w, h) = RESOLUTIONS[settings.resolution];
            row(
                &mut commands,
                body,
                "Resolution",
                format!("{w} × {h}"),
                5,
                1.0,
            );
            row(
                &mut commands,
                body,
                "Wait for vertical sync",
                on(settings.vsync),
                3,
                1.0,
            );
            row(
                &mut commands,
                body,
                "Multisampling anti-aliasing",
                if settings.msaa { "4× MSAA" } else { "None" }.into(),
                6,
                1.0,
            );
            row(
                &mut commands,
                body,
                "Brightness",
                format!("{:.1}", settings.exposure),
                2,
                0.1,
            );
        }
        Page::Audio => {
            row(
                &mut commands,
                body,
                "Master volume",
                format!("{}%", (settings.volume * 100.0).round()),
                0,
                0.05,
            );
            label(
                &mut commands,
                body,
                "Stereo positional audio • weapon, impact and player sounds",
                19.0,
                Color::srgb(0.7, 0.76, 0.8),
            );
        }
        Page::Controls => {
            row(
                &mut commands,
                body,
                "Mouse sensitivity",
                format!("{:.1}", settings.sensitivity),
                1,
                0.1,
            );
            row(
                &mut commands,
                body,
                "Invert mouse",
                on(settings.invert_y),
                9,
                1.0,
            );
            for (i, name) in ACTIONS.iter().enumerate() {
                let r = column(
                    &mut commands,
                    body,
                    Node {
                        height: px(28.0),
                        column_gap: px(20.0),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                );
                let l = column(
                    &mut commands,
                    r,
                    Node {
                        width: px(260.0),
                        ..default()
                    },
                );
                label(&mut commands, l, *name, 17.0, Color::srgb(0.8, 0.85, 0.89));
                let binding = button(
                    &mut commands,
                    r,
                    if menu.capture == Some(i) {
                        "Press a key…".into()
                    } else {
                        crate::settings::key_name(settings.binds[i])
                    },
                    Action::Bind(i),
                    px(180.0),
                );
                commands.entity(binding).insert(Node {
                    width: px(180.0),
                    min_height: px(26.0),
                    padding: UiRect::axes(px(12.0), px(2.0)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                });
            }
        }
        Page::Crosshair => {
            row(
                &mut commands,
                body,
                "Colour",
                ["Green", "Cyan", "Yellow", "White", "Pink"][settings.cross_color].into(),
                10,
                1.0,
            );
            row(
                &mut commands,
                body,
                "Length",
                format!("{:.0}", settings.cross_size),
                11,
                1.0,
            );
            row(
                &mut commands,
                body,
                "Gap",
                format!("{:.0}", settings.cross_gap),
                12,
                1.0,
            );
            row(
                &mut commands,
                body,
                "Thickness",
                format!("{:.0}", settings.cross_thickness),
                13,
                1.0,
            );
            row(
                &mut commands,
                body,
                "Centre dot",
                on(settings.cross_dot),
                14,
                1.0,
            );
            let preview = column(
                &mut commands,
                body,
                Node {
                    height: px(160.0),
                    width: px(460.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
            );
            commands
                .entity(preview)
                .insert(BackgroundColor(Color::srgb(0.12, 0.16, 0.19)));
            let pivot = column(
                &mut commands,
                preview,
                Node {
                    width: px(0.0),
                    height: px(0.0),
                    ..default()
                },
            );
            for i in 0..5 {
                if i == 4 && !settings.cross_dot {
                    continue;
                }
                let (x, y, w, h) = cross_bar(&settings, i);
                let e = commands
                    .spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(x),
                            top: px(y),
                            width: px(w),
                            height: px(h),
                            ..default()
                        },
                        BackgroundColor(settings.crosshair_color()),
                    ))
                    .id();
                commands.entity(pivot).add_children(&[e]);
            }
        }
    }
    label(
        &mut commands,
        content,
        if menu.status.is_empty() {
            ""
        } else {
            &menu.status
        },
        17.0,
        Color::srgb(0.83, 0.83, 0.63),
    );
    if settings_page {
        let footer = column(
            &mut commands,
            content,
            Node {
                column_gap: px(12.0),
                ..default()
            },
        );
        button(
            &mut commands,
            footer,
            "APPLY / SAVE",
            Action::Save,
            px(200.0),
        );
        button(
            &mut commands,
            footer,
            "RESTORE DEFAULTS",
            Action::Defaults,
            px(220.0),
        );
    }
}
pub fn cross_bar(s: &Settings, i: u8) -> (f32, f32, f32, f32) {
    let (l, t, g) = (s.cross_size, s.cross_thickness, s.cross_gap);
    match i {
        0 => (-g - l, -t / 2.0, l, t),
        1 => (g, -t / 2.0, l, t),
        2 => (-t / 2.0, -g - l, t, l),
        3 => (-t / 2.0, g, t, l),
        _ => (-t / 2.0, -t / 2.0, t, t),
    }
}

#[derive(Component)]
pub struct Card {
    selected: bool,
}
#[derive(Component)]
pub(crate) struct CardGrid;
fn picture(commands: &mut Commands, parent: Entity, image: Handle<Image>, width: Val, height: Val) {
    let e = commands
        .spawn((
            ImageNode::new(image),
            Node {
                width,
                height,
                flex_shrink: 0.0,
                ..default()
            },
        ))
        .id();
    commands.entity(parent).add_children(&[e]);
}
fn scroll_grid(commands: &mut Commands, parent: Entity, scroll: f32) -> Entity {
    let grid = column(
        commands,
        parent,
        Node {
            flex_grow: 1.0,
            flex_basis: px(0.0),
            min_height: px(0.0),
            min_width: px(0.0),
            width: percent(100),
            flex_wrap: FlexWrap::Wrap,
            align_content: AlignContent::FlexStart,
            column_gap: px(12.0),
            row_gap: px(12.0),
            padding: UiRect::all(px(10.0)),
            overflow: Overflow::scroll_y(),
            ..default()
        },
    );
    commands.entity(grid).insert((
        CardGrid,
        RelativeCursorPosition::default(),
        ScrollPosition(Vec2::new(0.0, scroll)),
        BackgroundColor(Color::srgba(0.035, 0.055, 0.073, 0.73)),
    ));
    grid
}
fn card(
    commands: &mut Commands,
    parent: Entity,
    title: &str,
    action: Action,
    selected: bool,
    width: f32,
    height: f32,
) -> Entity {
    let e = commands
        .spawn((
            Button,
            action,
            Card { selected },
            RelativeCursorPosition::default(),
            UiTransform::IDENTITY,
            ZIndex(0),
            Node {
                width: px(width),
                height: px(height),
                flex_shrink: 0.0,
                border: UiRect::all(px(2.0)),
                ..default()
            },
            BorderColor::all(if selected {
                Color::srgb(0.78, 0.84, 0.54)
            } else {
                Color::srgba(0.5, 0.6, 0.65, 0.18)
            }),
            BoxShadow::new(
                Color::srgba(0.0, 0.0, 0.0, 0.35),
                px(0.0),
                px(2.0),
                px(0.0),
                px(3.0),
            ),
            BackgroundColor(Color::srgb(0.11, 0.15, 0.18)),
        ))
        .id();
    commands.entity(parent).add_children(&[e]);
    let text = label(commands, e, title, 16.0, Color::WHITE);
    commands.entity(text).insert(Node {
        position_type: PositionType::Absolute,
        bottom: px(4.0),
        left: px(8.0),
        ..default()
    });
    e
}
fn card_image(commands: &mut Commands, parent: Entity, image: Handle<Image>, height: f32) {
    let e = commands
        .spawn((
            ImageNode::new(image),
            Node {
                position_type: PositionType::Absolute,
                left: px(0.0),
                top: px(0.0),
                width: percent(100),
                height: px(height - 4.0),
                ..default()
            },
        ))
        .id();
    commands.entity(parent).add_children(&[e]);
}
pub(crate) fn cards(
    mut cards: Query<(
        &Card,
        &Action,
        &Interaction,
        &RelativeCursorPosition,
        &mut UiTransform,
        &mut BorderColor,
        &mut BoxShadow,
        &mut ZIndex,
    )>,
    mut grids: Query<(&RelativeCursorPosition, &ComputedNode, &mut ScrollPosition), With<CardGrid>>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    time: Res<Time<Real>>,
    mut menu: ResMut<Menu>,
    opts: Res<crate::Options>,
) {
    let dt = 1.0 - (-18.0 * time.delta_secs()).exp();
    for (card, action, interaction, cursor, mut transform, mut border, mut shadow, mut z) in
        &mut cards
    {
        let fixture = opts.0.screenshot.is_some()
            && std::env::var_os("CSR_TEST_HOVER").is_some()
            && matches!(action,Action::SelectMap(i) if *i==(menu.map+1)%menu.maps.len().max(1));
        let hover = *interaction != Interaction::None || fixture;
        let target = if hover { 1.035 } else { 1.0 };
        transform.scale = transform.scale.lerp(Vec2::splat(target), dt);
        let tilt = if hover {
            cursor.normalized.map_or(0.4, |p| p.x - 0.5) * 0.035
        } else {
            0.0
        };
        transform.rotation = Rot2::radians(tilt);
        transform.translation = Val2 {
            x: px(0.0),
            y: px(if hover { -4.0 } else { 0.0 }),
        };
        *border = BorderColor::all(if card.selected {
            Color::srgb(0.78, 0.84, 0.54)
        } else if hover {
            Color::srgb(0.9, 0.94, 0.96)
        } else {
            Color::srgba(0.5, 0.6, 0.65, 0.18)
        });
        *shadow = BoxShadow::new(
            Color::srgba(0.0, 0.0, 0.0, if hover { 0.72 } else { 0.35 }),
            px(0.0),
            px(if hover { 9.0 } else { 2.0 }),
            px(0.0),
            px(if hover { 12.0 } else { 3.0 }),
        );
        z.0 = if hover { 2 } else { 0 };
    }
    let dy = wheel
        .read()
        .map(|e| {
            e.y * if e.unit == bevy::input::mouse::MouseScrollUnit::Line {
                42.0
            } else {
                1.0
            }
        })
        .sum::<f32>();
    if dy != 0.0 {
        for (cursor, node, mut scroll) in &mut grids {
            if cursor.cursor_over {
                let max = ((node.content_size().y - node.size().y) * node.inverse_scale_factor())
                    .max(0.0);
                scroll.y = (scroll.y - dy).clamp(0.0, max);
                menu.scroll = scroll.y;
            }
        }
    }
}

pub fn hover(
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (With<Action>, Without<Card>, Changed<Interaction>),
    >,
) {
    for (i, mut c) in &mut buttons {
        c.0 = match i {
            Interaction::Hovered => Color::srgba(0.42, 0.50, 0.55, 0.9),
            Interaction::Pressed => Color::srgb(0.55, 0.62, 0.45),
            _ => Color::srgba(0.30, 0.36, 0.40, 0.68),
        };
    }
}

pub(crate) fn apply(
    settings: Res<Settings>,
    opts: Res<crate::Options>,
    mut local: ResMut<LocalPlayer>,
    mut sounds: ResMut<crate::audio::SoundLibrary>,
    mut materials: ResMut<Assets<crate::material::WorldMaterial>>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    mut cameras: Query<&mut Msaa, With<crate::player::PlayerCamera>>,
    mut sinks: Query<(&crate::audio::SoundGain, &mut bevy::audio::AudioSink)>,
    mut spatial: Query<(&crate::audio::SoundGain, &mut bevy::audio::SpatialAudioSink)>,
) {
    if !settings.is_changed() {
        return;
    }
    local.sensitivity = settings.sensitivity;
    sounds.master_volume = settings.volume;
    use bevy::audio::AudioSinkPlayback;
    for (gain, mut sink) in &mut sinks {
        sink.set_volume(bevy::audio::Volume::Linear(settings.volume * gain.0));
    }
    for (gain, mut sink) in &mut spatial {
        sink.set_volume(bevy::audio::Volume::Linear(settings.volume * gain.0));
    }
    if opts.0.screenshot.is_some() {
        return;
    }
    window.present_mode = if settings.vsync {
        bevy::window::PresentMode::AutoVsync
    } else {
        bevy::window::PresentMode::AutoNoVsync
    };
    window.mode = if settings.fullscreen {
        bevy::window::WindowMode::BorderlessFullscreen(bevy::window::MonitorSelection::Current)
    } else {
        bevy::window::WindowMode::Windowed
    };
    let (w, h) = RESOLUTIONS[settings.resolution];
    window.resolution.set(w as f32, h as f32);
    for mut aa in &mut cameras {
        *aa = if settings.msaa {
            Msaa::Sample4
        } else {
            Msaa::Off
        };
    }
    for (_, material) in materials.iter_mut() {
        material.params.exposure = settings.exposure;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn harness() -> App {
        let mut a = App::new();
        a.init_resource::<Menu>()
            .init_resource::<Settings>()
            .init_resource::<crate::overworld_client::Building>()
            .init_resource::<Relaunch>()
            .init_resource::<crate::inventory::Inventory>()
            .init_resource::<crate::inventory::Collection>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<Time<Virtual>>()
            .init_resource::<crate::buymenu::BuyMenu>()
            .insert_resource(crate::Options(crate::ClientOptions::default()))
            .insert_resource(LocalPlayer::new(1, glam::Vec3::ZERO, 0.0))
            .insert_resource(InputEnabled(true))
            .add_message::<bevy::input::keyboard::KeyboardInput>()
            .add_systems(Update, (seed_input, input, crate::player::grab_cursor).chain());
        a.world_mut().spawn((
            PrimaryWindow,
            CursorOptions {
                grab_mode: CursorGrabMode::Locked,
                visible: false,
                ..default()
            },
        ));
        a
    }
    #[test]
    fn world_seed_editor_uses_text_events_keeps_order_and_consumes_menu_shortcuts() {
        use bevy::input::{keyboard::{KeyboardInput,Key},ButtonState};
        let mut a=harness();
        {let mut menu=a.world_mut().resource_mut::<Menu>();menu.page=Some(Page::Play);menu.maps=vec!["overworld".into()];}
        a.world_mut().resource_mut::<crate::overworld_client::Building>().initialized=true;
        let click=a.world_mut().spawn((Interaction::Pressed,Action::EditSeed)).id();a.update();
        assert!(a.world().resource::<Menu>().seed_edit.is_some());a.world_mut().despawn(click);
        let send=|a:&mut App,key_code,text:&str,repeat| {
            a.world_mut().resource_mut::<Messages<KeyboardInput>>().write(KeyboardInput{key_code,logical_key:Key::Character(text.into()),state:ButtonState::Pressed,text:Some(text.into()),repeat,window:Entity::PLACEHOLDER});
        };
        send(&mut a,KeyCode::Numpad7,"7",false);send(&mut a,KeyCode::Numpad7,"7",true);a.update();
        send(&mut a,KeyCode::Enter,"",false);a.update();assert_eq!(a.world().resource::<Menu>().world_seed,77);
        {let mut m=a.world_mut().resource_mut::<Menu>();m.seed_edit=Some(world_seed::Editor::new("77".into()));}
        a.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyO);
        send(&mut a,KeyCode::KeyO,"o",false);a.update();assert_eq!(a.world().resource::<Menu>().page,Some(Page::Play));
        send(&mut a,KeyCode::Escape,"",false);a.update();assert_eq!(a.world().resource::<Menu>().world_seed,77);assert_eq!(a.world().resource::<Menu>().page,Some(Page::Play));
        {let mut m=a.world_mut().resource_mut::<Menu>();m.seed_edit=Some(world_seed::Editor::new("-9223372036854775808".into()));}
        a.world_mut().spawn((Interaction::Pressed,Action::ApplySeed));a.update();
        assert_eq!(a.world().resource::<Menu>().world_seed,i64::MIN);assert!(a.world().resource::<Menu>().seed_edit.is_none());
    }
    #[test]
    fn map_cards_select_directly_and_ignore_invalid_indices() {
        let mut a = harness();
        {
            let mut m = a.world_mut().resource_mut::<Menu>();
            m.page = Some(Page::Play);
            m.maps = (0..27).map(|i| format!("de_test{i}")).collect();
        }
        let e = a
            .world_mut()
            .spawn((Interaction::Pressed, Action::SelectMap(26)))
            .id();
        a.update();
        assert_eq!(a.world().resource::<Menu>().map, 26);
        a.world_mut()
            .entity_mut(e)
            .insert((Interaction::Pressed, Action::SelectMap(99)));
        a.update();
        assert_eq!(a.world().resource::<Menu>().map, 26);
    }
    #[test]
    fn escape_pause_resume_clears_actions_and_recaptures_cursor() {
        let mut a = harness();
        {
            let mut l = a.world_mut().resource_mut::<LocalPlayer>();
            l.latched = 1;
            l.pending_drop = true;
            l.pending_slot = 2;
        }
        a.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        a.update();
        assert_eq!(a.world().resource::<Menu>().page, Some(Page::Pause));
        assert!(a.world().resource::<Time<Virtual>>().is_paused());
        assert_eq!(a.world().resource::<LocalPlayer>().latched, 0);
        assert!(!a.world().resource::<LocalPlayer>().pending_drop);
        a.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        a.update();
        a.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        a.update();
        assert!(!a.world().resource::<Menu>().open());
        assert!(!a.world().resource::<Time<Virtual>>().is_paused());
        assert!(a.world().resource::<crate::player::InputBlockFrames>().0 > 0);
        let mut q = a.world_mut().query::<&CursorOptions>();
        let cursor = q.single(a.world()).unwrap();
        assert_eq!(cursor.grab_mode, CursorGrabMode::Locked);
        assert!(!cursor.visible);
    }
    #[test]
    fn hostage_map_cannot_select_unimplemented_bomb_rules() {
        let mut a = harness();
        {
            let mut m = a.world_mut().resource_mut::<Menu>();
            m.page = Some(Page::Play);
            m.maps = vec!["de_dust2".into(), "cs_office".into()];
        }
        let e = a
            .world_mut()
            .spawn((Interaction::Pressed, Action::SelectMap(1)))
            .id();
        a.update();
        assert_eq!(a.world().resource::<Menu>().map, 1);
        assert!(a.world().resource::<Menu>().deathmatch);
        a.world_mut().entity_mut(e).insert(Action::Mode);
        a.world_mut().entity_mut(e).insert(Interaction::Pressed);
        a.update();
        assert!(a.world().resource::<Menu>().deathmatch);
    }
    #[test]
    fn overworld_selection_and_stale_bot_buttons_cannot_enable_bots() {
        let mut a=harness();
        {let mut m=a.world_mut().resource_mut::<Menu>();m.page=Some(Page::Play);m.maps=vec!["de_dust2".into(),"overworld".into()];m.bots=10;}
        let e=a.world_mut().spawn((Interaction::Pressed,Action::SelectMap(1))).id();a.update();
        assert_eq!(a.world().resource::<Menu>().bots,0);
        a.world_mut().entity_mut(e).insert((Interaction::Pressed,Action::Bots(1)));a.update();
        assert_eq!(a.world().resource::<Menu>().bots,0);
    }
    #[test]
    fn clicking_resume_leaves_mouse_gated_until_release() {
        let mut a = harness();
        a.world_mut().resource_mut::<Menu>().page = Some(Page::Pause);
        a.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        a.world_mut().spawn((Interaction::Pressed, Action::Resume));
        a.update();
        assert!(!a.world().resource::<Menu>().open());
        assert!(a.world().resource::<crate::player::InputBlockFrames>().0 > 0);
    }
    #[test]
    fn minecraft_options_key_pauses_and_closes_buy_without_leaking_input() {
        let mut a=harness();
        a.world_mut().resource_mut::<crate::overworld_client::Building>().initialized=true;
        a.world_mut().resource_mut::<crate::buymenu::BuyMenu>().open=true;
        a.world_mut().resource_mut::<LocalPlayer>().latched=3;
        a.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyO);
        a.update();
        assert_eq!(a.world().resource::<Menu>().page,Some(Page::Minecraft));
        assert!(a.world().resource::<Time<Virtual>>().is_paused());
        assert!(!a.world().resource::<crate::buymenu::BuyMenu>().open);
        assert_eq!(a.world().resource::<LocalPlayer>().latched,0);
        a.world_mut().resource_mut::<ButtonInput<KeyCode>>().reset_all();
        a.world_mut().spawn((Interaction::Pressed,Action::Adjust(20,-0.1)));
        a.update();
        assert!((a.world().resource::<Settings>().mc_mobs-0.9).abs()<0.001);
    }
    #[test]
    fn minecraft_video_controls_apply_and_escape_returns_to_options() {
        let mut a=harness();
        a.world_mut().resource_mut::<Menu>().page=Some(Page::MinecraftGraphics);
        let e=a.world_mut().spawn((Interaction::Pressed,Action::Adjust(25,2.))).id();a.update();
        assert_eq!(a.world().resource::<Settings>().mc_render_distance,8);
        a.world_mut().entity_mut(e).insert((Interaction::Pressed,Action::Adjust(28,1.)));a.update();
        assert_eq!(a.world().resource::<Settings>().mc_clouds,1);
        a.world_mut().entity_mut(e).insert((Interaction::None,Action::Adjust(29,1.)));
        a.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Escape);a.update();
        assert_eq!(a.world().resource::<Menu>().page,Some(Page::Minecraft));
        assert!(a.world().resource::<Time<Virtual>>().is_paused());
    }
    #[test]
    fn minecraft_options_do_not_open_in_cs_maps_or_over_inventory() {
        for inventory_open in [false,true] {
            let mut a=harness();
            {let mut b=a.world_mut().resource_mut::<crate::overworld_client::Building>();b.initialized=inventory_open;b.inventory_open=inventory_open;}
            a.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyO);
            a.update();assert!(!a.world().resource::<Menu>().open());
        }
    }

}
