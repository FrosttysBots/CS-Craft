//! Minecraft options use the installed pack's widget sprites and ASCII bitmap font.
use crate::{
    LoadedLevel,
    menu::{Action, Menu, Page},
    settings::Settings,
};
use bevy::prelude::*;
use std::collections::HashMap;

pub struct MinecraftSettingsPlugin;
impl Plugin for MinecraftSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::minecraft_graphics::MinecraftGraphicsPlugin).init_resource::<Ui>()
            .add_systems(Update, (draw, hover).chain().after(crate::menu::draw));
    }
}
#[derive(Resource, Default)]
struct Ui {
    root: Option<Entity>,
    key: String,
    normal: Handle<Image>,
    highlighted: Handle<Image>,
    ascii: Option<::image::RgbaImage>,
    labels: HashMap<String, (Handle<Image>, u32)>,
}
#[derive(Component)]
struct MinecraftButton;
fn load(runtime: &overworld::Runtime, key: &str) -> Option<::image::RgbaImage> {
    let id = overworld::terrain::pack::ResourceId::parse(&format!("minecraft:{key}")).ok()?;
    ::image::load_from_memory(&runtime.packs.texture(&id).ok()??)
        .ok()
        .map(|i| i.to_rgba8())
}
fn label(
    c: &mut Commands,
    parent: Entity,
    ui: &mut Ui,
    images: &mut Assets<Image>,
    text: &str,
    scale: f32,
) {
    let Some(ascii) = &ui.ascii else {
        crate::menu::label(c, parent, text, 18., Color::WHITE);
        return;
    };
    let (image, width) = ui
        .labels
        .entry(text.to_owned())
        .or_insert_with(|| {
            // ASCII provider in the original pack uses a 16-column, 8-pixel cell grid.
            let cell = ascii.width() / 16;
            let glyphs: Vec<_> = text
                .bytes()
                .map(|b| {
                    let b = if (32..127).contains(&b) { b } else { b'?' };
                    let (x, y) = (u32::from(b % 16) * cell, u32::from(b / 16) * cell);
                    let width = if b == b' ' {
                        cell / 2
                    } else {
                        (0..cell)
                            .rev()
                            .find(|dx| (0..cell).any(|dy| ascii.get_pixel(x + dx, y + dy)[3] > 0))
                            .map_or(0, |w| w + 1)
                    };
                    (x, y, width)
                })
                .collect();
            let width = glyphs.iter().map(|g| g.2 + 1).sum::<u32>().max(1);
            let mut pixels = ::image::RgbaImage::new(width + 1, cell + 1);
            let mut cursor = 0;
            for (x, y, w) in glyphs {
                for shadow in [true, false] {
                    for dy in 0..cell {
                        for dx in 0..w {
                            let alpha = ascii.get_pixel(x + dx, y + dy)[3];
                            if alpha > 0 {
                                pixels.put_pixel(
                                    cursor + dx + u32::from(shadow),
                                    dy + u32::from(shadow),
                                    ::image::Rgba(if shadow {
                                        [50, 50, 50, alpha]
                                    } else {
                                        [255, 255, 255, alpha]
                                    }),
                                );
                            }
                        }
                    }
                }
                cursor += w + 1;
            }
            (
                images.add(crate::overworld_client::image(
                    pixels.as_raw(),
                    pixels.width(),
                    pixels.height(),
                )),
                width + 1,
            )
        })
        .clone();
    let e = c
        .spawn((
            ImageNode::new(image),
            Node {
                width: px(width as f32 * scale),
                height: px((ascii.height() / 16 + 1) as f32 * scale),
                ..default()
            },
            bevy::ui::FocusPolicy::Pass,
        ))
        .id();
    c.entity(parent).add_child(e);
}
fn button(
    c: &mut Commands,
    parent: Entity,
    ui: &mut Ui,
    images: &mut Assets<Image>,
    text: &str,
    action: Action,
    width: f32,
    scale: f32,
) {
    let e = c
        .spawn((
            Button,
            MinecraftButton,
            action,
            ImageNode {
                image: ui.normal.clone(),
                image_mode: bevy::ui::widget::NodeImageMode::Sliced(TextureSlicer {
                    border: BorderRect::all(2.),
                    center_scale_mode: SliceScaleMode::Stretch,
                    sides_scale_mode: SliceScaleMode::Stretch,
                    max_corner_scale: scale,
                }),
                ..default()
            },
            Node {
                width: px(width * scale),
                height: px(20. * scale),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .id();
    c.entity(parent).add_child(e);
    label(c, e, ui, images, text, scale);
}
fn draw(
    mut c: Commands,
    menu: Res<Menu>,
    settings: Res<Settings>,
    level: Res<LoadedLevel>,
    mut ui: ResMut<Ui>,
    mut images: ResMut<Assets<Image>>,
    window: Single<&Window>,
) {
    let key = format!(
        "{:?}:{}:{}:{settings:?}:{}",
        menu.page,
        window.width(),
        window.height(),
        menu.status
    );
    if ui.key == key {
        return;
    }
    ui.key = key;
    if let Some(e) = ui.root.take() {
        c.entity(e).despawn();
    }
    if !matches!(menu.page,Some(Page::Minecraft | Page::MinecraftGraphics)) {
        return;
    }
    if ui.ascii.is_none() {
        if let Some(runtime) = &level.0.overworld {
            let runtime = runtime.lock().unwrap();
            ui.ascii = load(&runtime, "font/ascii");
            for (key, hover) in [
                ("gui/sprites/widget/button", false),
                ("gui/sprites/widget/button_highlighted", true),
            ] {
                if let Some(p) = load(&runtime, key) {
                    let h = images.add(crate::overworld_client::image(
                        p.as_raw(),
                        p.width(),
                        p.height(),
                    ));
                    if hover {
                        ui.highlighted = h;
                    } else {
                        ui.normal = h;
                    }
                }
            }
        }
    }
    let scale = (window.height() / 300.).floor().clamp(1., 3.);
    let root = c
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(9. * scale),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.02, 0.42)),
            GlobalZIndex(105),
        ))
        .id();
    ui.root = Some(root);
    if menu.page==Some(Page::MinecraftGraphics) {
        graphics(&mut c,root,&mut ui,&mut images,&settings,scale);
        return;
    }
    label(
        &mut c,
        root,
        &mut ui,
        &mut images,
        "Minecraft Options",
        scale * 1.5,
    );
    label(
        &mut c,
        root,
        &mut ui,
        &mut images,
        "Game Mode: Survival",
        scale,
    );
    for (name, value, id) in [
        ("Mob Sounds", settings.mc_mobs, 20),
        ("Block Sounds", settings.mc_blocks, 21),
    ] {
        let row = c
            .spawn(Node {
                column_gap: px(4. * scale),
                align_items: AlignItems::Center,
                ..default()
            })
            .id();
        c.entity(root).add_child(row);
        button(
            &mut c,
            row,
            &mut ui,
            &mut images,
            "-",
            Action::Adjust(id, -0.1),
            24.,
            scale,
        );
        let mid = c
            .spawn(Node {
                width: px(230. * scale),
                justify_content: JustifyContent::Center,
                ..default()
            })
            .id();
        c.entity(row).add_child(mid);
        label(
            &mut c,
            mid,
            &mut ui,
            &mut images,
            &format!("{name}: {:.0}%", value * 100.),
            scale,
        );
        button(
            &mut c,
            row,
            &mut ui,
            &mut images,
            "+",
            Action::Adjust(id, 0.1),
            24.,
            scale,
        );
    }
    button(&mut c,root,&mut ui,&mut images,"Video Settings...",Action::Page(Page::MinecraftGraphics),286.,scale);
    for (name, value, id) in [
        ("Minimap", settings.mc_minimap, 22),
        ("View Bobbing", settings.mc_hand_motion, 23),
        ("Menu Background Blur", settings.menu_blur, 24),
    ] {
        button(
            &mut c,
            root,
            &mut ui,
            &mut images,
            &format!("{name}: {}", if value { "ON" } else { "OFF" }),
            Action::Adjust(id, 1.),
            286.,
            scale,
        );
    }
    label(
        &mut c,
        root,
        &mut ui,
        &mut images,
        "F3: Debug / Coordinates  |  RMB: Use",
        scale,
    );
    label(
        &mut c,
        root,
        &mut ui,
        &mut images,
        "I: Inventory  N: Weapons  B: Buy",
        scale,
    );
    button(
        &mut c,
        root,
        &mut ui,
        &mut images,
        "Done",
        Action::Resume,
        200.,
        scale,
    );
    if !menu.status.is_empty() {
        label(&mut c, root, &mut ui, &mut images, &menu.status, scale);
    }
}
fn hover(
    ui: Res<Ui>,
    mut buttons: Query<
        (&Interaction, &mut ImageNode),
        (With<MinecraftButton>, Changed<Interaction>),
    >,
) {
    for (i, mut image) in &mut buttons {
        image.image = if *i == Interaction::None {
            ui.normal.clone()
        } else {
            ui.highlighted.clone()
        };
    }
}

fn graphics(c:&mut Commands,root:Entity,ui:&mut Ui,images:&mut Assets<Image>,s:&Settings,scale:f32) {
    label(c,root,ui,images,"Minecraft Video Settings",scale*1.5);
    for (name,value,id,step) in [("Render Distance (chunks)",s.mc_render_distance,25,1.),("Entity Distance (blocks)",s.mc_entity_distance,26,16.)] {
        let row=c.spawn(Node {column_gap:px(4.*scale),align_items:AlignItems::Center,..default()}).id();
        c.entity(root).add_child(row);
        button(c,row,ui,images,"-",Action::Adjust(id,-step),24.,scale);
        let mid=c.spawn(Node{width:px(266.*scale),justify_content:JustifyContent::Center,..default()}).id();
        c.entity(row).add_child(mid);
        label(c,mid,ui,images,&format!("{name}: {value}"),scale);
        button(c,row,ui,images,"+",Action::Adjust(id,step),24.,scale);
    }
    button(c,root,ui,images,&format!("Clouds: {}",["OFF","FAST","FANCY"][s.mc_clouds.min(2)]),Action::Adjust(28,1.),322.,scale);
    for (name,value,id) in [("Entity Frustum Culling",s.mc_entity_culling,27),("Show FPS",s.mc_show_fps,29),("Minimap",s.mc_minimap,22),("Menu Background Blur",s.menu_blur,24)] {
        button(c,root,ui,images,&format!("{name}: {}",if value {"ON"}else{"OFF"}),Action::Adjust(id,1.),322.,scale);
    }
    label(c,root,ui,images,"Lower distances reduce rendering work.",scale);
    label(c,root,ui,images,"F3: FPS, frame times, XYZ, chunks, entities",scale);
    button(c,root,ui,images,"Done",Action::Page(Page::Minecraft),200.,scale);
}
