//! Buy menu (B): CS:GO's categories with prices from the item schema,
//! filtered by team. Number keys pick a category then an item (0 goes
//! back), or click with the mouse. Items the rules would refuse are greyed
//! out using the simulation's own check ([`sim::World::buy_cost`]), and
//! purchases are sent through the normal tick input.

use crate::audio::SoundLibrary;
use crate::localize::Localization;
use crate::player::{InputEnabled, LocalPlayer, SimWorld};
use crate::LoadedLevel;
use bevy::audio::AudioSource;
use bevy::prelude::*;
use sim::{BuyError, Purchase, Team};
use weapons::WeaponClass;

pub const CATEGORIES: [&str; 6] = ["PISTOLS", "HEAVY", "SMGs", "RIFLES", "EQUIPMENT", "GRENADES"];

#[derive(Resource, Default)]
pub struct BuyMenu {
    pub open: bool,
    /// True for the frame the menu closed (to recapture the mouse).
    pub just_closed: bool,
    pub category: Option<usize>,
    signature: String,
    root: Option<Entity>,
    hovered: Option<usize>,
    wheels: std::collections::HashMap<(usize, Option<usize>), Handle<Image>>,
}

#[derive(Component, Clone, Copy)]
pub enum BuyAction {
    Category(usize),
    Item(Purchase),
    Back,
}

/// One entry of a category.
pub struct Entry {
    pub item: Purchase,
    pub name: String,
}

fn in_category(class: WeaponClass, cat: usize) -> bool {
    match cat {
        0 => class == WeaponClass::Pistol,
        1 => matches!(class, WeaponClass::Shotgun | WeaponClass::MachineGun),
        2 => class == WeaponClass::Smg,
        3 => matches!(class, WeaponClass::Rifle | WeaponClass::Sniper),
        5 => class == WeaponClass::Grenade,
        _ => false,
    }
}

/// The items a team sees in a category, cheapest first.
pub fn entries(world: &sim::World, loc: &Localization, team: Team, cat: usize) -> Vec<Entry> {
    let side_ok = |side: weapons::Side| match side {
        weapons::Side::Both => true,
        weapons::Side::Terrorist => team == Team::Terrorist,
        weapons::Side::CounterTerrorist => team == Team::CounterTerrorist,
    };
    if cat == 4 {
        let mut out = vec![
            Entry { item: Purchase::Kevlar, name: "Kevlar Vest".into() },
            Entry { item: Purchase::KevlarHelmet, name: "Kevlar + Helmet".into() },
        ];
        if let Some(zeus) = world.catalog.get("weapon_taser") {
            out.push(Entry { item: Purchase::Weapon(zeus.def_index), name: loc.weapon_name(zeus) });
        }
        if team == Team::CounterTerrorist {
            out.push(Entry { item: Purchase::DefuseKit, name: "Defuse Kit".into() });
        }
        return out;
    }
    let mut defs: Vec<&weapons::WeaponDefinition> = world
        .catalog
        .weapons
        .iter()
        .filter(|w| w.price > 0 && in_category(w.class, cat) && side_ok(w.side))
        .collect();
    defs.sort_by_key(|w| (w.price, w.def_index));
    defs.into_iter()
        .map(|w| Entry { item: Purchase::Weapon(w.def_index), name: loc.weapon_name(w) })
        .collect()
}

/// Display price of an entry (what it costs at full price).
pub fn list_price(world: &sim::World, item: Purchase) -> i32 {
    match item {
        Purchase::Weapon(i) => world.catalog.weapons.iter().find(|w| w.def_index == i).map_or(0, |w| w.price),
        Purchase::Kevlar => sim::rules::KEVLAR_PRICE,
        Purchase::KevlarHelmet => sim::rules::KEVLAR_PRICE + sim::rules::HELMET_PRICE,
        Purchase::DefuseKit => sim::rules::DEFUSE_KIT_PRICE,
        Purchase::None => 0,
    }
}

fn can_shop(world: &sim::World, id: sim::PlayerId) -> bool {
    world.player(id).is_some_and(|p| p.alive && (world.mode == sim::GameMode::Special || (world.competitive() && p.in_buy_zone && world.round.can_buy(&world.rules, world.time()))))
}

#[allow(clippy::too_many_arguments)]
fn request(
    item: Purchase,
    world: &sim::World,
    local: &mut LocalPlayer,
    commands: &mut Commands,
    sounds: &mut SoundLibrary,
    level: &LoadedLevel,
    sources: &mut Assets<AudioSource>,
) {
    match world.buy_cost(local.id, item) {
        Ok(_) => local.pending_buys.push(item),
        Err(_) => sounds.play(commands, &level.0.vfs, sources, "uipanorama.buymenu_failure", None),
    }
}

/// Keyboard and mouse handling; runs before mouse look.
#[allow(clippy::too_many_arguments)]
pub fn input(
    mut commands: Commands,
    mut menu: ResMut<BuyMenu>,
    mut local: ResMut<LocalPlayer>,
    world: Res<SimWorld>,
    keys: Res<ButtonInput<KeyCode>>,
    enabled: Res<InputEnabled>,
    loc: Res<Localization>,
    level: Res<LoadedLevel>,
    mut sounds: ResMut<SoundLibrary>,
    mut sources: ResMut<Assets<AudioSource>>,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    front: Res<crate::menu::Menu>,
    settings: Res<crate::settings::Settings>,
    building: Res<crate::overworld_client::Building>,
) {
    menu.just_closed = false;
    if front.open() || building.inventory_open {menu.open=false;return;}
    if !enabled.0 {
        return;
    }
    let shop = can_shop(&world.0, local.id);
    if keys.just_pressed(settings.binds[9]) {
        if menu.open {
            menu.open = false;
            menu.just_closed = true;
        } else if shop {
            menu.open = true;
            menu.category = None;
            local.latched = 0;
            local.pending_slot = 0;
            local.pending_drop = false;
        } else {
            sounds.play(&mut commands, &level.0.vfs, &mut sources, "buymenu_cant_buy.click", None);
        }
    }
    if !menu.open {
        return;
    }
    if !shop || keys.just_pressed(KeyCode::Escape) {
        menu.open = false;
        menu.just_closed = true;
        return;
    }
    let team = world.0.player(local.id).map_or(Team::Spectator, |p| p.team);
    let digits = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    if keys.just_pressed(KeyCode::Digit0) || keys.just_pressed(KeyCode::Backspace) {
        menu.category = None;
    }
    for (n, key) in digits.iter().enumerate() {
        if !keys.just_pressed(*key) {
            continue;
        }
        match menu.category {
            None if n < CATEGORIES.len() => menu.category = Some(n),
            Some(cat) => {
                if let Some(e) = entries(&world.0, &loc, team, cat).get(n) {
                    request(e.item, &world.0, &mut local, &mut commands, &mut sounds, &level, &mut sources);
                }
            }
            None => {}
        }
    }
    let entries = menu.category.map(|cat| entries(&world.0, &loc, team, cat));
    let count = entries.as_ref().map_or(6, |e| e.len().max(6));
    if let Ok(window) = windows.single() {
        let (center, radius) = wheel_geometry(window);
        menu.hovered = window.cursor_position().and_then(|pos| sector(pos-center, radius, count));
        if mouse.just_pressed(MouseButton::Right) { menu.category = None; }
        if mouse.just_pressed(MouseButton::Left) {
            if let Some(n) = menu.hovered {
                match &entries {
                    None => menu.category = Some(n),
                    Some(entries) => if let Some(e) = entries.get(n) {
                        request(e.item, &world.0, &mut local, &mut commands, &mut sounds, &level, &mut sources);
                    }
                }
            }
        }
    }
}

fn wheel_geometry(window: &Window) -> (Vec2, f32) {
    (Vec2::new(window.width() * 0.31, window.height() * 0.49), (window.height() * 0.34).min(280.0))
}
fn sector(delta: Vec2, radius: f32, count: usize) -> Option<usize> {
    if delta.length() < radius * 0.25 || delta.length() > radius { return None; }
    let angle = (delta.y.atan2(delta.x) + std::f32::consts::FRAC_PI_2 + std::f32::consts::PI / count as f32).rem_euclid(std::f32::consts::TAU);
    Some((angle / std::f32::consts::TAU * count as f32).floor() as usize % count)
}
fn wheel_image(count: usize, hovered: Option<usize>) -> Image {
    use bevy::{asset::RenderAssetUsages, render::render_resource::{Extent3d, TextureDimension, TextureFormat}};
    let size = 640usize;
    let mut rgba = vec![0; size * size * 4];
    for y in 0..size { for x in 0..size {
        let d = Vec2::new(x as f32 + 0.5 - size as f32/2.0, y as f32 + 0.5 - size as f32/2.0);
        let r = d.length();
        if r >= 319.0 { continue; }
        let at = (y*size+x)*4;
        let selected = sector(d, 320.0, count).is_some_and(|s| Some(s) == hovered);
        let angle = (d.y.atan2(d.x) + std::f32::consts::FRAC_PI_2 + std::f32::consts::PI/count as f32).rem_euclid(std::f32::consts::TAU);
        let phase = angle / std::f32::consts::TAU * count as f32;
        let edge = r > 80.0 && ((phase.fract().min(1.0-phase.fract()) * std::f32::consts::TAU/count as f32 * r) < 1.0);
        let color = if edge || (r-80.0).abs()<1.0 || r>317.0 { [150,150,145,180] } else if selected { [112,113,106,235] } else { [24,27,29,220] };
        rgba[at..at+4].copy_from_slice(&color);
    } }
    Image::new(Extent3d { width: size as u32, height: size as u32, depth_or_array_layers: 1 }, TextureDimension::D2, rgba, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD)
}

fn label(parent: &mut ChildSpawnerCommands, text: impl Into<String>, size: f32, color: Color) {
    parent.spawn((Text::new(text), TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(color)));
}

#[allow(clippy::too_many_arguments)]
pub fn draw(mut commands: Commands, mut menu: ResMut<BuyMenu>, local: Res<LocalPlayer>, world: Res<SimWorld>, loc: Res<Localization>, art: Res<crate::ui_assets::UiAssets>, windows: Query<&Window>, mut images: ResMut<Assets<Image>>) {
    let world = &world.0;
    let Some(p) = world.player(local.id) else { return };
    let Ok(window) = windows.single() else { return };
    let left = world.round.live_start + world.rules.buy_time - world.time();
    let signature = if menu.open { format!("{:?}|{:?}|{}|{}|{}|{}|{:?}|{}|{}x{}", menu.category, menu.hovered, p.money, p.armor, p.helmet, p.defuse_kit, p.weapons.iter().map(|w| &w.def.class_name).collect::<Vec<_>>(), left.ceil() as i32, window.width(), window.height()) } else { String::new() };
    if signature == menu.signature { return; }
    menu.signature = signature;
    if let Some(root) = menu.root.take() { commands.entity(root).despawn(); }
    if !menu.open { return; }
    let category = menu.category;
    let entries = category.map(|cat| entries(world, &loc, p.team, cat));
    let count = entries.as_ref().map_or(6, |e| e.len().max(6));
    let hovered = menu.hovered;
    let wheel = menu.wheels.entry((count, hovered)).or_insert_with(|| images.add(wheel_image(count, hovered))).clone();
    let (center, radius) = wheel_geometry(window);
    let side = if p.team == Team::CounterTerrorist { "ct" } else { "t" };
    let root = commands.spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() }, BackgroundColor(Color::srgba(0.04,0.05,0.06,0.97)), GlobalZIndex(30))).with_children(|screen| {
        screen.spawn(Node { position_type: PositionType::Absolute, top: px(44.0), left: px(50.0), flex_direction: FlexDirection::Column, ..default() }).with_children(|title| {
            label(title, "BUY MENU", 30.0, Color::WHITE);
            label(title, format!("Buy Time Remaining: 00:{:02}", left.max(0.0).ceil() as i32), 19.0, Color::srgb(0.7,0.72,0.72));
        });
        screen.spawn((ImageNode::new(wheel), Node { position_type: PositionType::Absolute, left: px(center.x-radius), top: px(center.y-radius), width: px(radius*2.0), height: px(radius*2.0), ..default() }));
        if let Some(image) = art.image(side) { screen.spawn((ImageNode::new(image), Node { position_type: PositionType::Absolute, left: px(center.x-radius*0.17), top: px(center.y-radius*0.17), width: px(radius*0.34), height: px(radius*0.34), ..default() })); }
        for n in 0..count {
            let angle = -std::f32::consts::FRAC_PI_2 + n as f32 * std::f32::consts::TAU/count as f32;
            let pos = center + Vec2::new(angle.cos(), angle.sin()) * radius * 0.65;
            screen.spawn(Node { position_type: PositionType::Absolute, left: px(pos.x-88.0), top: px(pos.y-40.0), width: px(176.0), height: px(80.0), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() }).with_children(|cell| {
                match &entries {
                    None => { label(cell, CATEGORIES[n], 21.0, Color::WHITE); label(cell, (n+1).to_string(), 14.0, Color::srgb(0.6,0.62,0.63)); },
                    Some(entries) => if let Some(e) = entries.get(n) {
                        let check = world.buy_cost(local.id, e.item);
                        let color = if check.is_ok() { Color::WHITE } else { Color::srgb(0.55,0.55,0.55) };
                        let key = match e.item { Purchase::Weapon(i) => world.catalog.weapons.iter().find(|w| w.def_index == i).map(|w| w.class_name.trim_start_matches("weapon_").to_string()), Purchase::Kevlar => Some("armor".into()), Purchase::KevlarHelmet => Some("helmet".into()), Purchase::DefuseKit => Some("defuser".into()), _ => None };
                        if let Some(image) = key.as_deref().and_then(|key| art.image(key)) { cell.spawn((ImageNode { color, ..ImageNode::new(image) }, Node { max_width: px(82.0), height: px(25.0), ..default() })); }
                        label(cell, format!("{}  {}", n+1, e.name), 16.0, color);
                        label(cell, if matches!(check, Err(BuyError::AlreadyOwned)) { "OWNED".into() } else { format!("${}", check.unwrap_or_else(|_| list_price(world,e.item))) }, 17.0, color);
                    }
                }
            });
        }
        screen.spawn(Node { position_type: PositionType::Absolute, left: px(center.x-radius), top: px(center.y+radius+18.0), ..default() }).with_children(|money| { label(money, format!("${}",p.money), 32.0, Color::srgb(0.65,0.85,0.48)); });
        if let Some(image) = art.image(&format!("character-{side}")) { screen.spawn((ImageNode::new(image), Node { position_type: PositionType::Absolute, right: percent(10), top: percent(12), height: percent(67), ..default() })); }
        screen.spawn(Node { position_type: PositionType::Absolute, left: percent(61), bottom: percent(15), width: percent(32), flex_direction: FlexDirection::Column, ..default() }).with_children(|detail| {
            label(detail, category.map_or("SELECT A CATEGORY", |c| CATEGORIES[c]), 28.0, Color::WHITE);
            if category == Some(5) { label(detail, "Carry up to four grenades. Press 4 to equip.", 18.0, Color::srgb(0.7,0.7,0.7)); }
            if let Some(e) = entries.as_ref().and_then(|e| hovered.and_then(|i| e.get(i))) { label(detail, &e.name, 24.0, Color::WHITE); }
        });
        screen.spawn(Node { position_type: PositionType::Absolute, bottom: px(25.0), left: px(50.0), ..default() }).with_children(|footer| label(footer, "[ESC] CLOSE      [B] CLOSE      [RIGHT CLICK / 0] BACK", 18.0, Color::srgb(0.7,0.72,0.72)));
    }).id();
    menu.root = Some(root);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wheel_mouse_and_number_sectors_agree() {
        for count in [6,7,8,9] { for n in 0..count {
            let angle = -std::f32::consts::FRAC_PI_2 + n as f32 * std::f32::consts::TAU/count as f32;
            assert_eq!(sector(Vec2::new(angle.cos(),angle.sin())*180.0,280.0,count),Some(n));
        } }
        assert_eq!(sector(Vec2::ZERO,280.0,6),None);
        assert_eq!(sector(Vec2::X*281.0,280.0,6),None);
    }
}
