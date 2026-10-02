//! Pack-backed Minecraft inventory, workbench and furnace menus.
use crate::{
    LoadedLevel,
    overworld_client::{Building, Slot, image},
};
use bevy::prelude::*;
use overworld::{
    mc_player::inventory::ItemStack,
    survival::{Screen, Slot as Action},
    terrain::{item_icons, pack::ResourceId},
};
use std::collections::HashMap;
#[derive(Resource, Default)]
pub struct MinecraftUi {
    root: Option<Entity>,
    key: String,
    icons: HashMap<String, Handle<Image>>,
    textures: HashMap<String, Handle<Image>>,
}
fn text(
    commands: &mut Commands,
    parent: Entity,
    label: impl Into<String>,
    x: f32,
    y: f32,
    size: f32,
    color: Color,
    font: &Handle<Font>,
) {
    let e = commands
        .spawn((
            Text::new(label),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(color),
            Node {
                position_type: PositionType::Absolute,
                left: px(x),
                top: px(y),
                ..default()
            },
            bevy::ui::FocusPolicy::Pass,
        ))
        .id();
    commands.entity(parent).add_child(e);
}
fn texture(
    ui: &mut MinecraftUi,
    images: &mut Assets<Image>,
    runtime: &overworld::Runtime,
    key: &str,
) -> Option<Handle<Image>> {
    if let Some(h) = ui.textures.get(key) {
        return Some(h.clone());
    }
    let bytes = runtime
        .packs
        .texture(&ResourceId::parse(&format!("minecraft:{key}")).ok()?)
        .ok()??;
    let mut pixels = ::image::load_from_memory(&bytes).ok()?.to_rgba8();
    if key == "gui/container/generic_54" {
        let source = pixels.clone();
        let mut single = ::image::RgbaImage::new(176, 166);
        ::image::imageops::replace(
            &mut single,
            &::image::imageops::crop_imm(&source, 0, 0, 176, 71).to_image(),
            0,
            0,
        );
        ::image::imageops::replace(
            &mut single,
            &::image::imageops::crop_imm(&source, 0, 126, 176, 95).to_image(),
            0,
            71,
        );
        pixels = single;
    }
    let h = images.add(image(pixels.as_raw(), pixels.width(), pixels.height()));
    ui.textures.insert(key.into(), h.clone());
    Some(h)
}
#[allow(clippy::too_many_arguments)]
fn slot(
    commands: &mut Commands,
    parent: Entity,
    ui: &mut MinecraftUi,
    images: &mut Assets<Image>,
    runtime: &overworld::Runtime,
    font: &Handle<Font>,
    action: Action,
    stack: Option<&ItemStack>,
    x: f32,
    y: f32,
    scale: f32,
    selected: bool,
) {
    let e = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(x),
                top: px(y),
                width: px(16. * scale),
                height: px(16. * scale),
                border: UiRect::all(px(if selected { 2. } else { 0. })),
                ..default()
            },
            BorderColor::all(Color::WHITE),
            BackgroundColor(Color::NONE),
        ))
        .id();
    commands.entity(parent).add_child(e);
    if action != Action::Inventory(999) {
        commands.entity(e).insert((Button, Slot(action)));
    } else {
        commands.entity(e).insert(bevy::ui::FocusPolicy::Pass);
    }
    if let Some(stack) = stack {
        let icon = if let Some(h) = ui.icons.get(&stack.id) {
            Some(h.clone())
        } else {
            let h = item_icons::item_icon(&runtime.packs, &stack.id, 32)
                .ok()
                .flatten()
                .map(|p| images.add(image(p.as_raw(), p.width(), p.height())));
            if let Some(h) = &h {
                ui.icons.insert(stack.id.clone(), h.clone());
            }
            h
        };
        if let Some(h) = icon {
            let icon = commands
                .spawn((
                    ImageNode::new(h),
                    Node {
                        width: px(16. * scale),
                        height: px(16. * scale),
                        ..default()
                    },
                    bevy::ui::FocusPolicy::Pass,
                ))
                .id();
            commands.entity(e).add_child(icon);
        } else {
            text(
                commands,
                e,
                stack.id.trim_start_matches("minecraft:"),
                0.,
                0.,
                10.,
                Color::WHITE,
                font,
            );
        }
        if stack.count > 1 {
            text(
                commands,
                e,
                stack.count.to_string(),
                9. * scale,
                9. * scale,
                7. * scale,
                Color::WHITE,
                font,
            );
        }
        if let Some((damage, max)) = runtime.inventory.recipes.durability(stack) {
            if damage > 0 && max > 0 {
                let bar = commands
                    .spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(scale),
                            bottom: px(scale),
                            width: px(14. * scale * (1. - damage as f32 / max as f32)),
                            height: px(scale),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.3, 0.9, 0.15)),
                        bevy::ui::FocusPolicy::Pass,
                    ))
                    .id();
                commands.entity(e).add_child(bar);
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub fn draw(
    mut commands: Commands,
    level: Res<LoadedLevel>,
    building: Res<Building>,
    front: Res<crate::menu::Menu>,
    art: Res<crate::ui_assets::UiAssets>,
    mut ui: ResMut<MinecraftUi>,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window>,
    slots: Query<(&Interaction, &Slot)>,
    preview: Res<crate::menu_scene::Preview>,
) {
    let Some(runtime) = &level.0.overworld else {
        return;
    };
    let runtime = runtime.lock().unwrap();
    let Ok(window) = windows.single() else { return };
    let pointer = window.cursor_position().unwrap_or(Vec2::ZERO);
    let hover = slots
        .iter()
        .find(|(i, _)| **i == Interaction::Hovered)
        .map(|(_, s)| s.0);
    let f = match runtime.screen {
        Screen::Furnace(p) => runtime
            .save
            .survival
            .furnaces
            .get(&runtime.container_key(p)),
        _ => None,
    };
    let chest = match runtime.screen {
        Screen::Chest(p) => runtime.save.survival.chests.get(&runtime.container_key(p)),
        _ => None,
    };
    let key = format!(
        "{}:{}:{}:{:?}:{:?}:{:?}:{:?}:{:?}:{:?}:{}:{:?}:{}:{}:{:?}:{}:{}",
        front.open(),
        window.width(),
        window.height(),
        runtime.screen,
        runtime.inventory.slots,
        runtime.inventory.crafting,
        runtime.inventory.workbench,
        runtime.inventory.cursor,
        f,
        building.selected,
        chest,
        building.active,
        building.status,
        hover,
        runtime.save.survival.food.level,
        (runtime.save.adventure.dragon.health as i32)
    );
    let key = format!(
        "{key}:{}:{}",
        runtime.save.adventure.dimension,
        runtime.save.survival.air.max(0) / 20
    );
    // Rebuild a carried icon at the cursor; static screens remain cached.
    let key = if runtime.inventory.cursor.is_some() {
        format!("{key}:{pointer:?}")
    } else {
        key
    };
    if ui.key == key {
        return;
    }
    ui.key = key;
    if let Some(e) = ui.root.take() {
        commands.entity(e).despawn();
    }
    if front.open() {
        return;
    }
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            GlobalZIndex(100),
            bevy::ui::FocusPolicy::Pass,
        ))
        .id();
    ui.root = Some(root);
    let font = &art.regular;
    let open = runtime.screen != Screen::Closed;
    let scale = (window.height() / 240.).floor().clamp(2., 4.);
    if runtime.screen == Screen::Victory {
        commands
            .entity(root)
            .insert(BackgroundColor(Color::srgba(0.02, 0.015, 0.025, 0.96)));
        text(
            &mut commands,
            root,
            "FREE THE END",
            window.width() * 0.3,
            window.height() * 0.3,
            48.,
            Color::srgb(0.7, 0.35, 0.9),
            font,
        );
        text(
            &mut commands,
            root,
            "You defeated the Ender Dragon.\nMinecraft Survival × Counter-Strike\n\nPress Enter to return to the Overworld.",
            window.width() * 0.3,
            window.height() * 0.44,
            24.,
            Color::WHITE,
            font,
        );
        return;
    }
    if open {
        commands
            .entity(root)
            .insert(BackgroundColor(Color::srgba(0., 0., 0., 0.65)));
        let x = (window.width() - 176. * scale) / 2.;
        let y = (window.height() - 166. * scale) / 2.;
        let panel = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(x),
                    top: px(y),
                    width: px(176. * scale),
                    height: px(166. * scale),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.78, 0.78, 0.78)),
            ))
            .id();
        commands.entity(root).add_child(panel);
        let (tex, title) = match runtime.screen {
            Screen::Workbench(_) => ("crafting_table", "Crafting"),
            Screen::Furnace(_) => ("furnace", "Furnace"),
            Screen::Chest(_) => ("generic_54", "Chest"),
            _ => ("inventory", "Crafting"),
        };
        if let Some(h) = texture(
            &mut ui,
            &mut images,
            &runtime,
            &format!("gui/container/{tex}"),
        ) {
            let bg = commands
                .spawn((
                    ImageNode {
                        image: h,
                        rect: Some(Rect::new(0., 0., 176., 166.)),
                        ..default()
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(176. * scale),
                        height: px(166. * scale),
                        ..default()
                    },
                    bevy::ui::FocusPolicy::Pass,
                ))
                .id();
            commands.entity(panel).add_child(bg);
        }
        let dark = Color::srgb(0.25, 0.25, 0.25);
        text(
            &mut commands,
            panel,
            title,
            if runtime.screen == Screen::Inventory {
                97. * scale
            } else {
                28. * scale
            },
            6. * scale,
            8. * scale,
            dark,
            font,
        );
        if runtime.screen != Screen::Inventory {
            text(
                &mut commands,
                panel,
                "Inventory",
                8. * scale,
                72. * scale,
                8. * scale,
                dark,
                font,
            );
        }
        for i in 0..36 {
            let (x, y) = if i < 9 {
                (8. + i as f32 * 18., 142.)
            } else {
                (
                    8. + ((i - 9) % 9) as f32 * 18.,
                    84. + ((i - 9) / 9) as f32 * 18.,
                )
            };
            slot(
                &mut commands,
                panel,
                &mut ui,
                &mut images,
                &runtime,
                font,
                Action::Inventory(i),
                runtime.inventory.slots[i].as_ref(),
                x * scale,
                y * scale,
                scale,
                false,
            );
        }
        match runtime.screen {
            Screen::Inventory => {
                let character = commands
                    .spawn((
                        ImageNode::new(preview.image.clone()),
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(26. * scale),
                            top: px(9. * scale),
                            width: px(50. * scale),
                            height: px(70. * scale),
                            ..default()
                        },
                        bevy::ui::FocusPolicy::Pass,
                    ))
                    .id();
                commands.entity(panel).add_child(character);
                for i in 0..4 {
                    slot(
                        &mut commands,
                        panel,
                        &mut ui,
                        &mut images,
                        &runtime,
                        font,
                        Action::Craft(i),
                        runtime.inventory.crafting[i].as_ref(),
                        (98. + (i % 2) as f32 * 18.) * scale,
                        (18. + (i / 2) as f32 * 18.) * scale,
                        scale,
                        false,
                    );
                }
                let output = runtime.inventory.crafting_output();
                slot(
                    &mut commands,
                    panel,
                    &mut ui,
                    &mut images,
                    &runtime,
                    font,
                    Action::Output,
                    output.as_ref(),
                    154. * scale,
                    28. * scale,
                    scale,
                    false,
                );
                for (i, y) in [(39, 8.), (38, 26.), (37, 44.), (36, 62.)] {
                    slot(
                        &mut commands,
                        panel,
                        &mut ui,
                        &mut images,
                        &runtime,
                        font,
                        Action::Inventory(i),
                        runtime.inventory.slots[i].as_ref(),
                        8. * scale,
                        y * scale,
                        scale,
                        false,
                    );
                }
                slot(
                    &mut commands,
                    panel,
                    &mut ui,
                    &mut images,
                    &runtime,
                    font,
                    Action::Inventory(40),
                    runtime.inventory.slots[40].as_ref(),
                    77. * scale,
                    62. * scale,
                    scale,
                    false,
                );
            }
            Screen::Workbench(_) => {
                for i in 0..9 {
                    slot(
                        &mut commands,
                        panel,
                        &mut ui,
                        &mut images,
                        &runtime,
                        font,
                        Action::Craft(i),
                        runtime.inventory.workbench[i].as_ref(),
                        (30. + (i % 3) as f32 * 18.) * scale,
                        (17. + (i / 3) as f32 * 18.) * scale,
                        scale,
                        false,
                    );
                }
                let output = runtime.inventory.workbench_output();
                slot(
                    &mut commands,
                    panel,
                    &mut ui,
                    &mut images,
                    &runtime,
                    font,
                    Action::Output,
                    output.as_ref(),
                    124. * scale,
                    35. * scale,
                    scale,
                    false,
                );
            }
            Screen::Furnace(_) => {
                if let Some(f) = f {
                    for (i, x, y) in [(0, 56., 17.), (1, 56., 53.), (2, 116., 35.)] {
                        slot(
                            &mut commands,
                            panel,
                            &mut ui,
                            &mut images,
                            &runtime,
                            font,
                            Action::Container(i),
                            f.slots[i].as_ref(),
                            x * scale,
                            y * scale,
                            scale,
                            false,
                        );
                    }
                    let progress = f.cook_progress as f32 / f.cook_total.max(1) as f32;
                    let e = commands
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(80. * scale),
                                top: px(39. * scale),
                                width: px(22. * scale * progress),
                                height: px(8. * scale),
                                ..default()
                            },
                            BackgroundColor(Color::WHITE),
                        ))
                        .id();
                    commands.entity(panel).add_child(e);
                    let heat = f.lit_remaining as f32 / f.lit_total.max(1) as f32;
                    let e = commands
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(57. * scale),
                                top: px((50. - heat * 12.) * scale),
                                width: px(12. * scale),
                                height: px(12. * scale * heat),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(1., 0.55, 0.05)),
                        ))
                        .id();
                    commands.entity(panel).add_child(e);
                }
            }
            Screen::Chest(_) => {
                if let Some(c) = chest {
                    for i in 0..27 {
                        slot(
                            &mut commands,
                            panel,
                            &mut ui,
                            &mut images,
                            &runtime,
                            font,
                            Action::Container(i),
                            c.slots[i].as_ref(),
                            (8. + (i % 9) as f32 * 18.) * scale,
                            (18. + (i / 9) as f32 * 18.) * scale,
                            scale,
                            false,
                        );
                    }
                }
            }
            _ => {}
        }
        text(
            &mut commands,
            root,
            "Left click: pick up / place   •   Right click: split / place one   •   Shift-click: transfer   •   I / Esc: close",
            x,
            y + 174. * scale,
            15.,
            Color::WHITE,
            font,
        );
        if let Some(stack) = runtime.inventory.cursor.as_ref() {
            slot(
                &mut commands,
                root,
                &mut ui,
                &mut images,
                &runtime,
                font,
                Action::Inventory(999),
                Some(stack),
                pointer.x - 8. * scale,
                pointer.y - 8. * scale,
                scale,
                false,
            );
        }
        if runtime.inventory.cursor.is_none() {
            if let Some(action) = hover {
                let stack = match action {
                    Action::Inventory(i) => runtime.inventory.slots.get(i).and_then(Option::as_ref),
                    Action::Craft(i) => {
                        if matches!(runtime.screen, Screen::Workbench(_)) {
                            runtime.inventory.workbench.get(i).and_then(Option::as_ref)
                        } else {
                            runtime.inventory.crafting.get(i).and_then(Option::as_ref)
                        }
                    }
                    Action::Container(i) => f
                        .and_then(|f| f.slots.get(i).and_then(Option::as_ref))
                        .or_else(|| chest.and_then(|c| c.slots.get(i).and_then(Option::as_ref))),
                    _ => None,
                };
                if let Some(stack) = stack {
                    text(
                        &mut commands,
                        root,
                        stack.id.trim_start_matches("minecraft:").replace('_', " "),
                        pointer.x + 16.,
                        pointer.y - 22.,
                        20.,
                        Color::WHITE,
                        font,
                    );
                }
            }
        }
    } else {
        let scale = 2.5;
        let x = (window.width() - 182. * scale) / 2.;
        let y = window.height() - 25. * scale;
        let bar = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(x),
                    top: px(y),
                    width: px(182. * scale),
                    height: px(22. * scale),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.15, 0.15, 0.15, 0.75)),
            ))
            .id();
        commands.entity(root).add_child(bar);
        if let Some(h) = texture(&mut ui, &mut images, &runtime, "gui/sprites/hud/hotbar") {
            let e = commands
                .spawn((
                    ImageNode::new(h),
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(182. * scale),
                        height: px(22. * scale),
                        ..default()
                    },
                    bevy::ui::FocusPolicy::Pass,
                ))
                .id();
            commands.entity(bar).add_child(e);
        }
        for i in 0..9 {
            slot(
                &mut commands,
                bar,
                &mut ui,
                &mut images,
                &runtime,
                font,
                Action::Inventory(i),
                runtime.inventory.slots[i].as_ref(),
                (3. + i as f32 * 20.) * scale,
                3. * scale,
                scale,
                building.active && i == building.selected,
            );
        }
        text(
            &mut commands,
            root,
            format!(
                "SURVIVAL  ·  {}   |   Hunger {}/20   |   Air {}",
                runtime.save.adventure.dimension,
                runtime.save.survival.food.level,
                runtime.save.survival.air.max(0) / 20
            ),
            x,
            y - 47.,
            16.,
            Color::WHITE,
            font,
        );
        text(
            &mut commands,
            root,
            if building.active {
                "LMB Mine · RMB Use/Place · I Inventory · N Weapons · O Options · Q Drop"
            } else {
                "CS Weapons · RMB / E Use block · N Tools · I Inventory · O Options"
            },
            x,
            y - 26.,
            14.,
            Color::WHITE,
            font,
        );
        if !building.status.is_empty() {
            text(
                &mut commands,
                root,
                &building.status,
                window.width() * 0.15,
                window.height() * 0.75,
                18.,
                Color::WHITE,
                font,
            );
        }
    }
    if runtime.save.adventure.dimension == "end" && !runtime.save.adventure.defeated {
        let width = window.width() * 0.4;
        let x = window.width() * 0.3;
        text(
            &mut commands,
            root,
            "Ender Dragon",
            x + width * 0.3,
            22.,
            25.,
            Color::WHITE,
            font,
        );
        let bar = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(x),
                    top: px(54.),
                    width: px(width * (runtime.save.adventure.dragon.health / 200.).clamp(0., 1.)),
                    height: px(12.),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.75, 0.1, 0.65)),
            ))
            .id();
        commands.entity(root).add_child(bar);
    }
}
