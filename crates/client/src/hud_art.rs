//! HUD images and the installed map overview. Never exposes unspotted enemies.
use bevy::prelude::*;
use crate::{ui_assets::UiAssets, player::{LocalPlayer, SimWorld}};

#[derive(Component)]
pub struct ArmorIcon;
#[derive(Component)]
pub struct BuyIcon;
#[derive(Component)]
pub struct RadarDot(sim::PlayerId);
#[derive(Resource, Default)]
pub struct Radar { pub(crate) origin: Vec2, pub(crate) scale: f32 }
#[derive(Resource, Default)]
pub struct Inventory { signature: String, root: Option<Entity> }

fn icon(commands: &mut Commands, handle: Handle<Image>, node: Node) -> Entity {
    commands.spawn((ImageNode::new(handle), node)).id()
}

pub fn setup(mut commands: Commands, art: Res<UiAssets>, level: Res<crate::LoadedLevel>, world: Res<SimWorld>, mut images: ResMut<Assets<Image>>) {
    if std::env::var_os("CSR_NO_HUD").is_some() {
        commands.init_resource::<Radar>();
        commands.init_resource::<Inventory>();
        return;
    }
    for (key, x) in [("health", 24.0), ("armor", 178.0)] {
        if let Some(image) = art.image(key) {
            let entity = icon(&mut commands, image, Node { position_type: PositionType::Absolute, left: px(x), bottom: px(24.0), width: px(27.0), height: px(27.0), ..default() });
            if key == "armor" { commands.entity(entity).insert(ArmorIcon); }
        }
        commands.spawn((Node { position_type: PositionType::Absolute, left: px(x), bottom: px(12.0), width: px(118.0), height: px(2.0), ..default() }, BackgroundColor(Color::srgba(0.9, 0.9, 0.88, 0.7))));
    }
    if let Some(image) = art.image("cart") {
        let entity = icon(&mut commands, image, Node { position_type: PositionType::Absolute, left: px(160.0), top: px(291.0), width: px(28.0), height: px(28.0), ..default() });
        commands.entity(entity).insert(BuyIcon);
    }
    // Full-map, north-up radar. Its scale and translation come from the map,
    // including workshop maps; positions use the same interpolated render time.
    let mut radar = Radar::default();
    if let Some(doc) = level.0.overview().and_then(|text| kv::parse(&text).ok()) {
        if let Some(root) = doc.iter().find_map(|(_, v)| v.as_block()) {
            let number = |k| root.str(k).and_then(kv::parse_f32).unwrap_or(0.0);
            radar.origin = Vec2::new(number("pos_x"), number("pos_y"));
            radar.scale = number("scale");
            let material = root.str("material").unwrap_or("");
            if let Some(img) = level.0.vfs.read(&format!("resource/overviews/{}_radar.dds", level.0.name)).or_else(|| level.0.vfs.read(&format!("resource/{material}_radar.dds"))).and_then(|bytes| radar_image(&bytes)) {
                icon(&mut commands, images.add(img), Node { position_type: PositionType::Absolute, left: px(22.0), top: px(24.0), width: px(250.0), height: px(250.0), ..default() });
            } else { warn!("Radar image not available: {material}"); }
        }
    }
    for p in &world.0.players {
        if let Some(image) = art.image(if p.team == sim::Team::CounterTerrorist { "radar-ct" } else { "radar-t" }) {
            let entity = icon(&mut commands, image, Node { position_type: PositionType::Absolute, width: px(10.0), height: px(10.0), ..default() });
            commands.entity(entity).insert((RadarDot(p.id), Visibility::Hidden, GlobalZIndex(1)));
        }
    }
    commands.insert_resource(radar);
    commands.init_resource::<Inventory>();
}

fn radar_image(bytes: &[u8]) -> Option<Image> {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    if bytes.get(..4)? != b"DDS " { return None; }
    let u32_at = |at| Some(u32::from_le_bytes(bytes.get(at..at+4)?.try_into().ok()?));
    let (w, h) = (u32_at(16)? as usize, u32_at(12)? as usize);
    if w == 0 || h == 0 || w > 4096 || h > 4096 { return None; }
    let format = match bytes.get(84..88)? { b"DXT1" => vtf::Format::Dxt1, b"DXT3" => vtf::Format::Dxt3, b"DXT5" => vtf::Format::Dxt5, _ => return None };
    let mut rgba = vtf::decode_rgba8(format, bytes.get(128..)?, w, h)?;
    for y in 0..h { for x in 0..w {
        let radius = ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0).hypot((y as f32 + 0.5) / h as f32 * 2.0 - 1.0);
        if radius > 1.0 { rgba[(y*w+x)*4+3] = 0; }
    } }
    Some(Image::new(Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 }, TextureDimension::D2, rgba, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD))
}

#[allow(clippy::too_many_arguments)]
pub fn update(mut commands: Commands, art: Res<UiAssets>, world: Res<SimWorld>, local: Res<LocalPlayer>, radar: Res<Radar>, poses: Res<crate::interpolation::RenderPoses>, time: Res<Time<Fixed>>, mut state: ResMut<Inventory>, mut armor: Query<&mut ImageNode, With<ArmorIcon>>, mut carts: Query<&mut Visibility, (With<BuyIcon>, Without<RadarDot>)>, mut dots: Query<(&RadarDot, &mut Node, &mut Visibility), Without<BuyIcon>>) {
    let w = &world.0;
    let Some(p) = w.player(local.view_target()) else { return };
    for mut image in &mut armor { if let Some(handle) = art.image(if p.helmet { "helmet" } else { "armor" }) { image.image = handle; } }
    for mut visibility in &mut carts { *visibility = if p.alive && p.in_buy_zone && w.competitive() && w.round.can_buy(&w.rules, w.time()) { Visibility::Inherited } else { Visibility::Hidden }; }
    for (dot, mut node, mut visibility) in &mut dots {
        *visibility = Visibility::Hidden;
        let Some(teammate) = w.player(dot.0).filter(|t| t.team == p.team && t.alive) else { continue };
        if radar.scale <= 0.0 { continue; }
        let pos = poses.sample(teammate, time.overstep_fraction()).origin;
        let uv = Vec2::new(pos.x-radar.origin.x, radar.origin.y-pos.y) / (1024.0 * radar.scale);
        if uv.distance(Vec2::splat(0.5)) > 0.5 { continue; }
        node.left = px(22.0 + uv.x * 250.0 - 5.0); node.top = px(24.0 + uv.y * 250.0 - 5.0);
        *visibility = Visibility::Inherited;
    }
    let signature = format!("{:?}|{}|{}|{:?}|{:?}", p.id, p.alive, p.active, p.weapons.iter().map(|wp| &wp.def.class_name).collect::<Vec<_>>(), w.players.iter().map(|p| (p.id, p.alive)).collect::<Vec<_>>());
    if signature == state.signature { return; }
    state.signature = signature;
    if let Some(root) = state.root.take() { commands.entity(root).despawn(); }
    state.root = Some(commands.spawn((crate::hud::GameHud,Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() })).with_children(|root| {
        if p.alive {
            root.spawn(Node { position_type: PositionType::Absolute, bottom: px(78.0), right: px(24.0), flex_direction: FlexDirection::Column, align_items: AlignItems::End, row_gap: px(8.0), ..default() }).with_children(|inventory| {
                for (index, weapon) in p.weapons.iter().enumerate() {
                    let key = weapon.def.class_name.trim_start_matches("weapon_");
                    let color = if index == p.active { Color::WHITE } else { Color::srgba(0.8, 0.82, 0.82, 0.5) };
                    inventory.spawn(Node { align_items: AlignItems::Center, column_gap: px(12.0), ..default() }).with_children(|row| {
                        if let Some(image) = art.image(key).or_else(|| art.image(if key.contains("knife") { "knife_default_ct" } else { "c4" }).filter(|_| key.contains("knife") || key == "c4")) {
                            row.spawn((ImageNode { color, ..ImageNode::new(image) }, Node { width: px(95.0), height: px(30.0), ..default() }));
                        } else { row.spawn((Text::new(key.to_uppercase()), TextColor(color), TextFont { font_size: FontSize::Px(18.0), ..default() })); }
                        row.spawn((Text::new(sim::weapon::slot(&weapon.def).to_string()), TextColor(color), TextFont { font_size: FontSize::Px(15.0), ..default() }));
                    });
                }
            });
        }
        for (side, left) in [(sim::Team::CounterTerrorist, true), (sim::Team::Terrorist, false)] {
            root.spawn(Node { position_type: PositionType::Absolute, top: px(6.0), left: if left { Val::Auto } else { percent(50) }, right: if left { percent(50) } else { Val::Auto }, margin: if left { UiRect::right(px(106.0)) } else { UiRect::left(px(106.0)) }, column_gap: px(3.0), ..default() }).with_children(|team| {
                for player in w.players.iter().filter(|p| p.team == side) {
                    if let Some(image) = art.image("avatar") {
                        team.spawn((ImageNode { color: if player.alive { Color::WHITE } else { Color::srgba(0.3,0.3,0.3,0.5) }, ..ImageNode::new(image) }, Node { width: px(36.0), height: px(42.0), border: UiRect::bottom(px(3.0)), ..default() }, BackgroundColor(if left { Color::srgb(0.23,0.36,0.48) } else { Color::srgb(0.46,0.39,0.24) })));
                    }
                }
            });
        }
    }).id());
}
