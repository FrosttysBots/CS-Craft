//! Keyless local case opening, driven by real time while the match is paused.
use crate::{
    cosmetics::{self, Catalog, Inventory, OwnedSkin, Random},
    menu::{Menu, Page},
};
use bevy::{
    asset::RenderAssetUsages,
    audio::{AudioPlayer, AudioSource, PlaybackSettings, Volume},
    image::{CompressedImageFormats, ImageSampler, ImageType},
    input::{ButtonState, keyboard::KeyboardInput},
    prelude::*,
};
use std::collections::HashMap;
const CARD: f32 = 180.;
const WINNER: usize = 38;
const DURATION: f32 = 6.4;
#[derive(Clone)]
enum Screen {
    Cases,
    Souvenirs,
    Skins,
    Contents(usize),
    Opening {
        case: usize,
        item: OwnedSkin,
        reel: Vec<usize>,
        start: f32,
        jitter: f32,
    },
    Reveal(OwnedSkin),
    Stickers {
        serial: u64,
        slot: usize,
    },
}
#[derive(Resource)]
pub struct CaseUi {
    screen: Screen,
    page: usize,
    root: Option<Entity>,
    dirty: bool,
    size: Vec2,
    command: String,
    console: bool,
    message: String,
    icons: HashMap<String, Handle<Image>>,
    sounds: HashMap<String, Handle<AudioSource>>,
    pub fixture: bool,
    tick: i32,
    refresh: f32,
}
impl Default for CaseUi {
    fn default() -> Self {
        Self {
            screen: Screen::Cases,
            page: 0,
            root: None,
            dirty: true,
            size: Vec2::ZERO,
            command: String::new(),
            console: false,
            message: String::new(),
            icons: default(),
            sounds: default(),
            fixture: false,
            tick: -1,
            refresh: 0.,
        }
    }
}
impl CaseUi {
    pub fn preview_item(&self) -> Option<&OwnedSkin> {
        if let Screen::Reveal(i) = &self.screen {
            Some(i)
        } else {
            None
        }
    }
}
#[derive(Component, Clone, Copy)]
pub enum Action {
    Cases,
    Souvenirs,
    Skins,
    Case(usize),
    Open(usize),
    Equip(u64),
    Inspect(u64),
    Reset(u64),
    Next(i32),
    Console,
    Close,
    Stickers(u64, usize),
    Apply(u64, usize, u32),
    Scrape(u64, usize),
}
#[derive(Component)]
pub struct Reel;
pub fn setup(
    mut commands: Commands,
    level: Res<crate::LoadedLevel>,
    mut sources: ResMut<Assets<AudioSource>>,
    catalog: Res<Catalog>,
) {
    let mut ui = CaseUi::default();
    let mut inventory = Inventory::load(&cosmetics::profile_path()).unwrap_or_else(|e| {
        ui.message = e;
        Inventory::default()
    });
    for (key, path) in [
        ("open", "sound/ui/panorama/case_unlock_01.wav"),
        ("tick", "sound/ui/csgo_ui_crate_item_scroll.wav"),
        ("3", "sound/ui/item_reveal3_rare.wav"),
        ("4", "sound/ui/item_reveal4_mythical.wav"),
        ("5", "sound/ui/item_reveal5_legendary.wav"),
        ("6", "sound/ui/item_reveal6_ancient.wav"),
        ("7", "sound/ui/item_reveal6_ancient.wav"),
        ("1", "sound/ui/item_reveal3_rare.wav"),
        ("2", "sound/ui/item_reveal3_rare.wav"),
    ] {
        if let Some(bytes) = level.0.vfs.read(path).and_then(crate::audio::playable) {
            ui.sounds.insert(
                key.into(),
                sources.add(AudioSource {
                    bytes: bytes.into(),
                }),
            );
        }
    }
    // Visual fixtures never change the user's collection.
    if let Ok(fixture) = std::env::var("CSR_TEST_CASE") {
        ui.fixture = true;
        inventory = Inventory::default();
        let selected = std::env::var("CSR_TEST_CASE_ID")
            .ok()
            .and_then(|q| catalog.cases.iter().position(|c| c.id.to_string() == q))
            .unwrap_or(0);
        if let Some(case) = catalog.cases.get(selected) {
            let _ = inventory.grant(case.id, 5);
            if fixture == "opening" || fixture == "reveal" {
                let mut rng = Random(42);
                if let Ok(item) = inventory.open(case, &catalog, &mut rng) {
                    ui.screen = if fixture == "reveal" {
                        Screen::Reveal(item)
                    } else {
                        opening(selected, item, &catalog, 0., &mut rng)
                    };
                }
            } else if fixture == "contents" {
                ui.screen = Screen::Contents(selected);
            }
        }
    }
    if let Ok(paint) = std::env::var("CSR_TEST_SKIN")
        .ok()
        .as_deref()
        .unwrap_or("")
        .parse::<u32>()
    {
        if let Some(f) = catalog.finishes.iter().find(|f| {
            f.id == paint && std::env::var("CSR_TEST_SKIN_WEAPON").map_or(true, |w| w == f.weapon)
        }) {
            ui.fixture = true;
            inventory = Inventory::default();
            let mut item = OwnedSkin {
                serial: 1,
                weapon: f.weapon.clone(),
                paint: f.id,
                seed: 278,
                wear: f.wear[0],
                stattrak: (std::env::var("CSR_TEST_QUALITY").as_deref() == Ok("stattrak"))
                    .then_some(123),
                ..default()
            };
            if let Ok(kit) = std::env::var("CSR_TEST_STICKER")
                .unwrap_or_default()
                .parse::<u32>()
            {
                for s in &mut item.stickers {
                    *s = Some(cosmetics::StickerSlot { kit, wear: 0. });
                }
                inventory.sticker_stock.insert(kit, 4);
            }
            inventory.skins.push(item.clone());
            inventory.equip(1).unwrap();
            if std::env::var("CSR_TEST_MENU").as_deref() == Ok("cases") {
                ui.screen = Screen::Reveal(item);
            }
        }
    }
    info!(
        "Cosmetics: {} cases, {} weapon finishes; {} owned skins",
        catalog.cases.len(),
        catalog.finishes.len(),
        inventory.skins.len()
    );
    commands.insert_resource(inventory);
    commands.insert_resource(ui);
}
fn opening(
    case: usize,
    item: OwnedSkin,
    catalog: &Catalog,
    start: f32,
    rng: &mut Random,
) -> Screen {
    let mut reel: Vec<_> = (0..48)
        .filter_map(|_| cosmetics::roll(&catalog.cases[case], catalog, rng))
        .collect();
    if let Some(i) = catalog
        .finishes
        .iter()
        .position(|f| f.id == item.paint && f.weapon == item.weapon)
    {
        reel[WINNER] = i;
    }
    Screen::Opening {
        case,
        item,
        reel,
        start,
        jitter: (rng.unit() - 0.5) * CARD * 0.6,
    }
}
pub fn reel_offset(elapsed: f32, jitter: f32) -> f32 {
    let t = (elapsed / DURATION).clamp(0., 1.);
    let end = WINNER as f32 * CARD + CARD * 0.5 + jitter;
    let start = 3. * CARD;
    start + (end - start) * (1. - (1. - t).powi(5))
}
fn sound(commands: &mut Commands, ui: &CaseUi, key: &str, volume: f32) {
    if let Some(h) = ui.sounds.get(key) {
        commands.spawn((
            AudioPlayer(h.clone()),
            crate::audio::SoundGain(0.65),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume * 0.65)),
        ));
    }
}
fn mutate<T>(
    ui: &CaseUi,
    inventory: &mut Inventory,
    f: impl FnOnce(&mut Inventory) -> Result<T, String>,
) -> Result<T, String> {
    if ui.fixture {
        f(inventory)
    } else {
        inventory.transaction(&cosmetics::profile_path(), f)
    }
}
#[allow(clippy::too_many_arguments)]
pub fn input(
    mut commands: Commands,
    menu: Res<Menu>,
    catalog: Res<Catalog>,
    mut inventory: ResMut<Inventory>,
    mut ui: ResMut<CaseUi>,
    actions: Query<(&Interaction, &Action), Changed<Interaction>>,
    mut keys: MessageReader<KeyboardInput>,
    time: Res<Time<Real>>,
    settings: Res<crate::settings::Settings>,
) {
    if menu.page != Some(Page::Cases) {
        keys.clear();
        return;
    }
    if !ui.fixture && time.elapsed_secs() >= ui.refresh {
        ui.refresh = time.elapsed_secs() + 2.;
        if let Ok(fresh) = Inventory::load(&cosmetics::profile_path()) {
            if fresh != *inventory {
                *inventory = fresh;
                ui.dirty = true;
            }
        }
    }
    for event in keys.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }
        if event.key_code == KeyCode::Backquote {
            ui.console = !ui.console;
            ui.dirty = true;
            continue;
        }
        if !ui.console {
            continue;
        }
        match event.key_code {
            KeyCode::Enter => {
                let result = if ui.fixture {
                    Err("Console changes are disabled in screenshot fixtures".into())
                } else {
                    cosmetics::command(
                        &ui.command,
                        &catalog,
                        &mut inventory,
                        &cosmetics::profile_path(),
                    )
                };
                ui.message = result.unwrap_or_else(|e| e);
                ui.command.clear();
            }
            KeyCode::Backspace => {
                ui.command.pop();
            }
            _ => {
                if let Some(text) = &event.text {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        if ui.command.len() < 200 {
                            ui.command.push(c);
                        }
                    }
                }
            }
        }
        ui.dirty = true;
    }
    for (interaction, action) in &actions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if matches!(ui.screen, Screen::Opening { .. }) {
            continue;
        }
        ui.dirty = true;
        ui.message.clear();
        match *action {
            Action::Stickers(serial, slot) => {
                ui.screen = Screen::Stickers { serial, slot };
                ui.page = 0;
            }
            Action::Apply(serial, slot, kit) => {
                let result = mutate(&ui, &mut inventory, |i| {
                    i.apply_sticker(serial, slot, kit, &catalog)
                });
                ui.message = result
                    .map(|_| "Sticker applied.".into())
                    .unwrap_or_else(|e| e);
                if let Some(item) = inventory.skins.iter().find(|s| s.serial == serial) {
                    ui.screen = Screen::Reveal(item.clone());
                }
            }
            Action::Scrape(serial, slot) => {
                ui.message = mutate(&ui, &mut inventory, |i| i.scrape(serial, slot))
                    .map(|_| "Sticker scraped. Five scrapes remove it permanently.".into())
                    .unwrap_or_else(|e| e);
                if let Some(item) = inventory.skins.iter().find(|s| s.serial == serial) {
                    ui.screen = Screen::Reveal(item.clone());
                }
            }
            Action::Souvenirs => {
                ui.screen = Screen::Souvenirs;
                ui.page = 0;
            }
            Action::Cases => {
                ui.screen = Screen::Cases;
                ui.page = 0;
            }
            Action::Skins => {
                ui.screen = Screen::Skins;
                ui.page = 0;
            }
            Action::Case(i) => {
                ui.screen = Screen::Contents(i);
                ui.page = 0;
            }
            Action::Open(i) => {
                let mut rng = Random::fresh();
                match mutate(&ui, &mut inventory, |inventory| {
                    inventory.open(&catalog.cases[i], &catalog, &mut rng)
                }) {
                    Ok(item) => {
                        ui.screen = opening(i, item, &catalog, time.elapsed_secs(), &mut rng);
                        ui.tick = -1;
                        sound(&mut commands, &ui, "open", settings.volume);
                    }
                    Err(e) => ui.message = e,
                }
            }
            Action::Equip(id) => {
                ui.message = mutate(&ui, &mut inventory, |i| i.equip(id))
                    .map(|_| "Finish equipped. Applies to this weapon on either team.".into())
                    .unwrap_or_else(|e| e);
            }
            Action::Reset(id) => {
                ui.message = mutate(&ui, &mut inventory, |i| {
                    let weapon = i
                        .skins
                        .iter()
                        .find(|s| s.serial == id)
                        .ok_or("Skin no longer owned")?
                        .weapon
                        .clone();
                    i.equipped.remove(cosmetics::equip_slot(&weapon));
                    Ok(())
                })
                .map(|_| "Default finish restored.".into())
                .unwrap_or_else(|e| e);
            }
            Action::Inspect(id) => {
                if let Some(item) = inventory.skins.iter().find(|s| s.serial == id) {
                    ui.screen = Screen::Reveal(item.clone());
                }
            }
            Action::Next(delta) => {
                ui.page = ui.page.saturating_add_signed(delta as isize);
            }
            Action::Console => {
                ui.console = !ui.console;
            }
            Action::Close => {
                ui.screen = Screen::Skins;
                ui.page = 0;
            }
        }
    }
    if let Screen::Opening {
        item,
        start,
        jitter,
        ..
    } = &ui.screen
    {
        let elapsed = time.elapsed_secs() - start;
        let tick = (reel_offset(elapsed, *jitter) / CARD) as i32;
        if elapsed >= DURATION + 0.45 {
            let item = item.clone();
            if let Some(f) = catalog.finish(&item) {
                sound(&mut commands, &ui, &f.rarity.to_string(), settings.volume);
            }
            ui.screen = Screen::Reveal(item);
            ui.dirty = true;
        } else if tick != ui.tick {
            ui.tick = tick;
            sound(&mut commands, &ui, "tick", settings.volume);
        }
    }
}
fn panel(c: &mut Commands, parent: Entity, node: Node) -> Entity {
    let e = c.spawn(node).id();
    c.entity(parent).add_children(&[e]);
    e
}
fn text(c: &mut Commands, p: Entity, value: impl Into<String>, size: f32, color: Color) -> Entity {
    crate::menu::label(c, p, value.into().replace("★ ", ""), size, color)
}
fn button(c: &mut Commands, p: Entity, value: impl Into<String>, action: Action) -> Entity {
    let e = panel(
        c,
        p,
        Node {
            min_height: px(35.),
            padding: UiRect::axes(px(16.), px(8.)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
    );
    c.entity(e).insert((
        Button,
        action,
        BackgroundColor(Color::srgba(0.23, 0.30, 0.33, 0.9)),
    ));
    text(c, e, value, 16., Color::WHITE);
    e
}
fn rarity(r: u8) -> Color {
    match r {
        1 => Color::srgb(0.70, 0.73, 0.76),
        2 => Color::srgb(0.43, 0.65, 0.83),
        7 => Color::srgb(0.90, 0.68, 0.22),
        4 => Color::srgb(0.53, 0.28, 0.88),
        5 => Color::srgb(0.83, 0.17, 0.90),
        6 => Color::srgb(0.92, 0.27, 0.27),
        _ => Color::srgb(0.28, 0.43, 0.94),
    }
}
fn image(
    c: &mut Commands,
    p: Entity,
    key: &str,
    ui: &mut CaseUi,
    fs: &vfs::Vfs,
    images: &mut Assets<Image>,
    width: Val,
    height: Val,
) {
    if !ui.icons.contains_key(key) {
        if let Some(bytes) = fs.read(&format!("resource/flash/{key}.png")) {
            if let Ok(img) = Image::from_buffer(
                &bytes,
                ImageType::Extension("png"),
                CompressedImageFormats::NONE,
                true,
                ImageSampler::linear(),
                RenderAssetUsages::RENDER_WORLD,
            ) {
                ui.icons.insert(key.into(), images.add(img));
            }
        }
    }
    if let Some(h) = ui.icons.get(key) {
        let e = c
            .spawn((
                ImageNode::new(h.clone()),
                Node {
                    width,
                    height,
                    flex_shrink: 0.,
                    ..default()
                },
            ))
            .id();
        c.entity(p).add_children(&[e]);
    }
}
fn sticker_image(
    c: &mut Commands,
    p: Entity,
    s: &cosmetics::StickerDefinition,
    ui: &mut CaseUi,
    fs: &vfs::Vfs,
    images: &mut Assets<Image>,
) {
    if !ui.icons.contains_key(&s.material) {
        if let Some(mat) = vmt::Material::load(&s.material, &|p| fs.read_string(p)) {
            let mut cache = crate::textures::TextureCache::default();
            if let Some(tex) = mat
                .base_texture()
                .and_then(|n| cache.load(fs, images, n, true))
            {
                ui.icons.insert(s.material.clone(), tex.handle);
            }
        }
    }
    if let Some(icon) = ui.icons.get(&s.material) {
        let e = c
            .spawn((
                ImageNode::new(icon.clone()),
                Node {
                    width: px(48.),
                    height: px(48.),
                    ..default()
                },
            ))
            .id();
        c.entity(p).add_children(&[e]);
    }
}
#[allow(clippy::too_many_arguments)]
pub fn draw(
    mut commands: Commands,
    menu: Res<Menu>,
    catalog: Res<Catalog>,
    inventory: Res<Inventory>,
    mut ui: ResMut<CaseUi>,
    level: Res<crate::LoadedLevel>,
    mut images: ResMut<Assets<Image>>,
    preview: Res<crate::menu_scene::Preview>,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
) {
    if menu.page != Some(Page::Cases) {
        if let Some(root) = ui.root.take() {
            commands.entity(root).despawn();
        }
        ui.dirty = true;
        return;
    }
    let size = Vec2::new(window.width(), window.height());
    if !ui.dirty && ui.root.is_some() && ui.size == size {
        return;
    }
    ui.dirty = false;
    ui.size = size;
    if let Some(root) = ui.root.take() {
        commands.entity(root).despawn();
    }
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(142.),
                right: px(26.),
                top: px(78.),
                bottom: px(22.),
                padding: UiRect::all(px(18.)),
                flex_direction: FlexDirection::Column,
                row_gap: px(12.),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.035, 0.052, 0.068, if menu.in_match {0.58} else {0.96})),
            GlobalZIndex(110),
        ))
        .id();
    ui.root = Some(root);
    let nav = panel(
        &mut commands,
        root,
        Node {
            column_gap: px(10.),
            ..default()
        },
    );
    if !matches!(ui.screen, Screen::Opening { .. }) {
        button(&mut commands, nav, "CASES", Action::Cases);
        button(&mut commands, nav, "SOUVENIRS", Action::Souvenirs);
        button(
            &mut commands,
            nav,
            format!("MY SKINS ({})", inventory.skins.len()),
            Action::Skins,
        );
        button(&mut commands, nav, "CONSOLE", Action::Console);
    }
    text(
        &mut commands,
        root,
        "Local collection • Keyless cases and souvenir packages",
        15.,
        Color::srgb(0.65, 0.72, 0.76),
    );
    if ui.console {
        text(
            &mut commands,
            root,
            "give_case <name or ID> [count]  |  give_skin <weapon> <paint> [stattrak]  |  stickers <search>  |  give_sticker <ID> [count]",
            15.,
            Color::srgb(0.74, 0.82, 0.70),
        );
        text(
            &mut commands,
            root,
            format!("> {}_", ui.command),
            19.,
            Color::WHITE,
        );
    }
    if !ui.message.is_empty() {
        let message = ui.message.lines().take(5).collect::<Vec<_>>().join("\n");
        text(
            &mut commands,
            root,
            message,
            16.,
            Color::srgb(0.84, 0.85, 0.68),
        );
    }
    let fs = &level.0.vfs;
    match ui.screen.clone() {
        Screen::Cases | Screen::Souvenirs | Screen::Skins | Screen::Contents(_) => {
            let screen = ui.screen.clone();
            let mut finish_indices = vec![];
            let case_indices: Vec<_> = catalog
                .cases
                .iter()
                .enumerate()
                .filter(|(_, c)| c.souvenir == matches!(screen, Screen::Souvenirs))
                .map(|(i, _)| i)
                .collect();
            let count = match screen {
                Screen::Cases | Screen::Souvenirs => case_indices.len(),
                Screen::Skins => inventory.skins.len(),
                Screen::Contents(i) => {
                    let case = &catalog.cases[i];
                    let header = panel(
                        &mut commands,
                        root,
                        Node {
                            align_items: AlignItems::Center,
                            column_gap: px(20.),
                            ..default()
                        },
                    );
                    image(
                        &mut commands,
                        header,
                        &case.icon,
                        &mut ui,
                        fs,
                        &mut images,
                        px(112.),
                        px(84.),
                    );
                    text(
                        &mut commands,
                        header,
                        format!(
                            "{}  •  {} owned",
                            case.name,
                            inventory.cases.get(&case.id).copied().unwrap_or(0)
                        ),
                        25.,
                        Color::WHITE,
                    );
                    button(&mut commands, header, "OPEN CASE", Action::Open(i));
                    text(
                        &mut commands,
                        root,
                        "Contains one item from this collection. Gold items are rare-special drops; souvenir packages award souvenir quality.",
                        14.,
                        Color::srgb(0.65, 0.70, 0.74),
                    );
                    finish_indices = case.contents.clone();
                    finish_indices.len()
                }
                _ => 0,
            };
            let rows = (((size.y - if ui.console { 410. } else { 330. }) / 143.).floor() as usize)
                .clamp(1, 4);
            let cols = (((size.x - 230.) / 205.).floor() as usize).clamp(2, 7);
            let per = rows * cols;
            let pages = count.div_ceil(per).max(1);
            ui.page = ui.page.min(pages - 1);
            let grid = panel(
                &mut commands,
                root,
                Node {
                    flex_grow: 1.,
                    align_content: AlignContent::Start,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: px(10.),
                    row_gap: px(10.),
                    ..default()
                },
            );
            if count == 0 {
                text(
                    &mut commands,
                    grid,
                    "Your collection is empty. Open the console and type: give_case Revolution 5",
                    21.,
                    Color::WHITE,
                );
            }
            for i in ui.page * per..((ui.page + 1) * per).min(count) {
                let (title, icon, color, action, detail) = match screen {
                    Screen::Cases | Screen::Souvenirs => {
                        let index = case_indices[i];
                        let c = &catalog.cases[index];
                        (
                            c.name.clone(),
                            c.icon.clone(),
                            Color::srgb(0.67, 0.58, 0.30),
                            Action::Case(index),
                            format!(
                                "{} owned  •  ID {}",
                                inventory.cases.get(&c.id).copied().unwrap_or(0),
                                c.id
                            ),
                        )
                    }
                    Screen::Skins => {
                        let item = &inventory.skins[inventory.skins.len() - 1 - i];
                        let Some(f) = catalog.finish(item) else {
                            continue;
                        };
                        (
                            item.display_name(f),
                            f.icon.clone(),
                            rarity(f.rarity),
                            Action::Inspect(item.serial),
                            format!(
                                "{}{}",
                                item.condition(),
                                if inventory
                                    .active(&item.weapon)
                                    .is_some_and(|s| s.serial == item.serial)
                                {
                                    "  • Equipped"
                                } else {
                                    ""
                                }
                            ),
                        )
                    }
                    _ => {
                        let f = &catalog.finishes[finish_indices[i]];
                        (
                            f.name.clone(),
                            f.icon.clone(),
                            rarity(f.rarity),
                            Action::Case(match screen {
                                Screen::Contents(i) => i,
                                _ => 0,
                            }),
                            String::new(),
                        )
                    }
                };
                let card = panel(
                    &mut commands,
                    grid,
                    Node {
                        width: px((size.x - 230. - (cols - 1) as f32 * 10.) / cols as f32),
                        height: px(133.),
                        padding: UiRect::all(px(8.)),
                        flex_direction: FlexDirection::Column,
                        border: UiRect::bottom(px(3.)),
                        ..default()
                    },
                );
                commands.entity(card).insert((
                    Button,
                    action,
                    BackgroundColor(Color::srgba(0.14, 0.18, 0.21, 0.94)),
                    BorderColor::all(color),
                ));
                image(
                    &mut commands,
                    card,
                    &icon,
                    &mut ui,
                    fs,
                    &mut images,
                    px(114.),
                    px(70.),
                );
                text(&mut commands, card, title, 15., Color::WHITE);
                if !detail.is_empty() {
                    text(
                        &mut commands,
                        card,
                        detail,
                        13.,
                        Color::srgb(0.69, 0.75, 0.78),
                    );
                }
            }
            let pager = panel(
                &mut commands,
                root,
                Node {
                    column_gap: px(12.),
                    align_items: AlignItems::Center,
                    ..default()
                },
            );
            if ui.page > 0 {
                button(&mut commands, pager, "PREVIOUS", Action::Next(-1));
            }
            text(
                &mut commands,
                pager,
                format!("{} / {}", ui.page + 1, pages),
                16.,
                Color::WHITE,
            );
            if ui.page + 1 < pages {
                button(&mut commands, pager, "NEXT", Action::Next(1));
            }
        }
        Screen::Opening { case, reel, .. } => {
            text(
                &mut commands,
                root,
                format!("Unlocking {}", catalog.cases[case].name),
                30.,
                Color::WHITE,
            );
            let wrap = panel(
                &mut commands,
                root,
                Node {
                    width: percent(100),
                    height: px(194.),
                    margin: UiRect::top(px(65.)),
                    position_type: PositionType::Relative,
                    overflow: Overflow::clip(),
                    ..default()
                },
            );
            commands
                .entity(wrap)
                .insert(BackgroundColor(Color::srgb(0.018, 0.024, 0.03)));
            let strip = panel(
                &mut commands,
                wrap,
                Node {
                    position_type: PositionType::Absolute,
                    top: px(8.),
                    height: px(178.),
                    width: px(CARD * reel.len() as f32),
                    ..default()
                },
            );
            commands.entity(strip).insert(Reel);
            for i in reel {
                let f = &catalog.finishes[i];
                let card = panel(
                    &mut commands,
                    strip,
                    Node {
                        width: px(CARD),
                        height: percent(100),
                        padding: UiRect::all(px(10.)),
                        flex_shrink: 0.,
                        flex_direction: FlexDirection::Column,
                        border: UiRect::bottom(px(5.)),
                        ..default()
                    },
                );
                commands.entity(card).insert((
                    BackgroundColor(Color::srgb(0.12, 0.15, 0.19)),
                    BorderColor::all(rarity(f.rarity)),
                ));
                image(
                    &mut commands,
                    card,
                    if f.rarity == 7 {
                        "econ/weapon_cases/default_rare_item"
                    } else {
                        &f.icon
                    },
                    &mut ui,
                    fs,
                    &mut images,
                    px(160.),
                    px(110.),
                );
                text(
                    &mut commands,
                    card,
                    if f.rarity == 7 {
                        "Rare Special Item"
                    } else {
                        &f.name
                    },
                    16.,
                    Color::WHITE,
                );
            }
            let marker = panel(
                &mut commands,
                wrap,
                Node {
                    position_type: PositionType::Absolute,
                    left: percent(50),
                    width: px(2.),
                    height: percent(100),
                    ..default()
                },
            );
            commands
                .entity(marker)
                .insert((BackgroundColor(Color::srgb(0.98, 0.77, 0.32)), ZIndex(3)));
        }
        Screen::Stickers { serial, slot } => {
            text(
                &mut commands,
                root,
                format!("APPLY STICKER • SLOT {}", slot + 1),
                26.,
                Color::WHITE,
            );
            text(
                &mut commands,
                root,
                "Applying consumes one owned sticker. Scraping is permanent; five scrapes remove it.",
                15.,
                Color::srgb(0.7, 0.75, 0.8),
            );
            let stock: Vec<_> = inventory
                .sticker_stock
                .iter()
                .filter(|(_, n)| **n > 0)
                .collect();
            let per = 6;
            let pages = stock.len().div_ceil(per).max(1);
            ui.page = ui.page.min(pages - 1);
            for (&kit, &count) in stock.into_iter().skip(ui.page * per).take(per) {
                if let Some(s) = catalog.stickers.get(&kit) {
                    let row = panel(
                        &mut commands,
                        root,
                        Node {
                            column_gap: px(10.),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    );
                    sticker_image(&mut commands, row, s, &mut ui, fs, &mut images);
                    button(
                        &mut commands,
                        row,
                        format!("{}  ×{count}", s.name),
                        Action::Apply(serial, slot, kit),
                    );
                }
            }
            if inventory.sticker_stock.values().all(|n| *n == 0) {
                text(
                    &mut commands,
                    root,
                    "No stickers owned. Use stickers <search>, then give_sticker <ID>.",
                    19.,
                    Color::WHITE,
                );
            }
            let row = panel(
                &mut commands,
                root,
                Node {
                    column_gap: px(10.),
                    ..default()
                },
            );
            if ui.page > 0 {
                button(&mut commands, row, "PREVIOUS", Action::Next(-1));
            }
            if ui.page + 1 < pages {
                button(&mut commands, row, "NEXT", Action::Next(1));
            }
            button(&mut commands, row, "BACK TO ITEM", Action::Inspect(serial));
        }
        Screen::Reveal(item) => {
            if let Some(f) = catalog.finish(&item) {
                text(
                    &mut commands,
                    root,
                    item.display_name(f),
                    32.,
                    rarity(f.rarity),
                );
                let center = panel(
                    &mut commands,
                    root,
                    Node {
                        flex_grow: 1.,
                        width: percent(100),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                );
                let model = commands
                    .spawn((
                        ImageNode::new(preview.image.clone()),
                        Node {
                            width: px((size.x * 0.60).min(720.)),
                            height: px((size.y * 0.30).min(300.)),
                            ..default()
                        },
                    ))
                    .id();
                commands.entity(center).add_children(&[model]);
                text(
                    &mut commands,
                    root,
                    format!(
                        "{}   •   Wear {:.6}   •   Pattern {}",
                        item.condition(),
                        item.wear,
                        item.seed
                    ),
                    19.,
                    Color::srgb(0.79, 0.82, 0.84),
                );
                let row = panel(
                    &mut commands,
                    root,
                    Node {
                        column_gap: px(14.),
                        ..default()
                    },
                );
                button(
                    &mut commands,
                    row,
                    "EQUIP FINISH",
                    Action::Equip(item.serial),
                );
                button(
                    &mut commands,
                    row,
                    "USE DEFAULT",
                    Action::Reset(item.serial),
                );
                button(&mut commands, row, "CONTINUE", Action::Close);
                if let Some(count) = item.stattrak {
                    text(
                        &mut commands,
                        root,
                        format!("StatTrak™ Confirmed kills: {count:06}"),
                        18.,
                        Color::srgb(1., 0.56, 0.18),
                    );
                }
                if let Some(event) = item.souvenir {
                    text(
                        &mut commands,
                        root,
                        format!("Souvenir • Event {event} • Local package award"),
                        16.,
                        Color::srgb(0.86, 0.70, 0.25),
                    );
                }
                let slots = catalog
                    .models
                    .get(&item.weapon)
                    .map_or(0, |m| m.stickers.len().min(4));
                let row = panel(
                    &mut commands,
                    root,
                    Node {
                        column_gap: px(8.),
                        ..default()
                    },
                );
                for slot in 0..slots {
                    let card = panel(
                        &mut commands,
                        row,
                        Node {
                            flex_direction: FlexDirection::Column,
                            width: percent(25),
                            ..default()
                        },
                    );
                    if let Some(sticker) = item.stickers[slot] {
                        let name = catalog
                            .stickers
                            .get(&sticker.kit)
                            .map_or("Unknown sticker", |s| s.name.as_str());
                        text(
                            &mut commands,
                            card,
                            format!("{}: {name} ({:.0}%)", slot + 1, 100. * (1. - sticker.wear)),
                            12.,
                            Color::WHITE,
                        );
                        button(
                            &mut commands,
                            card,
                            "SCRAPE",
                            Action::Scrape(item.serial, slot),
                        );
                    } else {
                        button(
                            &mut commands,
                            card,
                            format!("APPLY • SLOT {}", slot + 1),
                            Action::Stickers(item.serial, slot),
                        );
                    }
                }
            }
        }
    }
}
pub fn animate(
    ui: Res<CaseUi>,
    time: Res<Time<Real>>,
    mut nodes: Query<&mut Node, With<Reel>>,
    mut hover: Query<(&Interaction, &mut BackgroundColor), With<Action>>,
) {
    if let Screen::Opening { start, jitter, .. } = ui.screen {
        for mut node in &mut nodes {
            node.left =
                px((ui.size.x - 204.) * 0.5 - reel_offset(time.elapsed_secs() - start, jitter));
        }
    }
    for (interaction, mut color) in &mut hover {
        *color = BackgroundColor(if *interaction == Interaction::Hovered {
            Color::srgb(0.25, 0.31, 0.36)
        } else {
            Color::srgb(0.14, 0.18, 0.21)
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reel_is_monotonic_and_winner_stays_under_marker() {
        for jitter in [-54., 0., 54.] {
            let mut previous = 0.;
            for i in 0..1000 {
                let x = reel_offset(i as f32 * 0.01, jitter);
                assert!(x >= previous);
                previous = x;
            }
            assert_eq!((reel_offset(7., jitter) / CARD) as usize, WINNER);
        }
    }
    #[test]
    fn opening_button_consumes_once_and_awards_the_reel_winner() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let mut menu = Menu::default();
        menu.page = Some(Page::Cases);
        let catalog = Catalog {
            cases: vec![cosmetics::Case {
                id: 4880,
                contents: vec![0],
                ..default()
            }],
            finishes: vec![cosmetics::Finish {
                id: 801,
                weapon: "weapon_ak47".into(),
                rarity: 6,
                wear: [0.05, 0.7],
                ..default()
            }],
            ..default()
        };
        let mut inventory = Inventory::default();
        inventory.grant(4880, 1).unwrap();
        app.insert_resource(menu)
            .insert_resource(catalog)
            .insert_resource(inventory)
            .insert_resource(CaseUi {
                fixture: true,
                ..default()
            })
            .insert_resource(crate::settings::Settings::default())
            .add_message::<KeyboardInput>()
            .add_systems(Update, input);
        app.world_mut()
            .spawn((Interaction::Pressed, Action::Open(0)));
        app.update();
        app.update();
        let inventory = app.world().resource::<Inventory>();
        assert_eq!(inventory.cases[&4880], 0);
        assert_eq!(inventory.skins.len(), 1);
        let ui = app.world().resource::<CaseUi>();
        if let Screen::Opening { item, reel, .. } = &ui.screen {
            assert_eq!(item, &inventory.skins[0]);
            assert_eq!(reel[WINNER], 0);
        } else {
            panic!("opening did not start");
        }
    }
}
