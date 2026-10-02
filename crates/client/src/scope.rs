//! Scope presentation. The sniper mask and optic dot are original VTF assets,
//! loaded from the user's installation; no game art is bundled.
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use crate::player::{LocalPlayer, SimWorld};

#[derive(Component)]
pub struct ScopePart(u8);

pub fn setup(mut commands: Commands, level: Res<crate::LoadedLevel>, mut images: ResMut<Assets<Image>>) {
    let mut cache = crate::textures::TextureCache::default();
    let arc = cache.load(&level.0.vfs, &mut images, "sprites/scope_arc", true).map(|t| t.handle);
    // The original optic dot uses its brightness as a coverage mask; its grey
    // background is not an opaque rectangle. Decode only the base mip for UI.
    let dot = level.0.vfs.read("materials/models/weapons/shared/scope/scope_dot_white.vtf")
        .and_then(|bytes| vtf::Vtf::parse(&bytes).ok()).and_then(|vtf| {
            let mut rgba = vtf.rgba8(0, 0, 0)?;
            for pixel in rgba.chunks_exact_mut(4) {
                let coverage = ((pixel[0] as f32 / 255.0 - 0.5) * 2.0).clamp(0.0, 1.0);
                pixel.copy_from_slice(&[255, 255, 255, (coverage * 255.0) as u8]);
            }
            let mut image = Image::new(
                bevy::render::render_resource::Extent3d { width: vtf.width as u32, height: vtf.height as u32, depth_or_array_layers: 1 },
                bevy::render::render_resource::TextureDimension::D2, rgba,
                bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::RENDER_WORLD,
            );
            image.sampler = bevy::image::ImageSampler::linear();
            Some(images.add(image))
        });
    for part in 0..11 {
        let mut entity = commands.spawn((
            Node { position_type: PositionType::Absolute, ..default() },
            GlobalZIndex(-1), Visibility::Hidden, ScopePart(part),
        ));
        if part < 4 {
            if let Some(image) = &arc {
                // In the original quarter arc (0, 0) is the clear centre.
                entity.insert(ImageNode { image: image.clone(), flip_x: part % 2 == 0, flip_y: part < 2, ..default() });
            }
        } else if part == 10 {
            if let Some(image) = &dot { entity.insert(ImageNode::new(image.clone())); }
        } else { entity.insert(BackgroundColor(Color::BLACK)); }
    }
}

pub fn update(
    settings: Res<crate::settings::Settings>,
    local: Res<LocalPlayer>, world: Res<SimWorld>, window: Single<&Window, With<PrimaryWindow>>,
    mut parts: Query<(&ScopePart, &mut Node, &mut Visibility, Option<&mut ImageNode>)>,
    mut crosshair: Query<(&crate::hud::Crosshair, &mut Visibility, &mut Node, &mut BackgroundColor), (With<crate::hud::Crosshair>, Without<ScopePart>)>,
) {
    let p = world.0.player(local.view_target()).filter(|p| p.alive && !local.noclip);
    let weapon = p.and_then(|p| p.active_weapon());
    let zoomed = weapon.is_some_and(|w| w.zoom_level > 0);
    let sniper = weapon.is_some_and(|w| w.hides_viewmodel());
    let regular = p.is_some() && !zoomed && !weapon.is_some_and(|w| w.def.class == weapons::WeaponClass::Sniper);
    for (part,mut v,mut node,mut color) in &mut crosshair { *v = if regular && (part.0 != 4 || settings.cross_dot) { Visibility::Inherited } else { Visibility::Hidden }; let (x,y,w,h)=crate::menu::cross_bar(&settings,part.0);node.left=px(x);node.top=px(y);node.width=px(w);node.height=px(h);color.0=settings.crosshair_color(); }
    let (width, height) = (window.width(), window.height());
    let size = width.min(height);
    let radius = size * 0.5;
    let (left, top) = ((width - size) * 0.5, (height - size) * 0.5);
    for (part, mut node, mut visibility, image) in &mut parts {
        let shown = if part.0 == 10 { zoomed && !sniper } else { sniper };
        *visibility = if shown { Visibility::Inherited } else { Visibility::Hidden };
        let (x, y, w, h) = match part.0 {
            n @ 0..=3 => (left + (n % 2) as f32 * radius, top + (n / 2) as f32 * radius, radius, radius),
            4 => (0.0, 0.0, left + 1.0, height),
            5 => (left + size - 1.0, 0.0, left + 1.0, height),
            6 => (left, 0.0, size, top),
            7 => (left, top + size, size, top),
            8 => (left, (height * 0.5).floor(), size, 1.0),
            9 => ((width * 0.5).floor(), top, 1.0, size),
            _ => (width * 0.5 - 16.0, height * 0.5 - 16.0, 32.0, 32.0),
        };
        node.left = px(x); node.top = px(y); node.width = px(w); node.height = px(h);
        if part.0 == 10 {
            if let Some(mut image) = image {
                image.color = if weapon.is_some_and(|w| w.def.class_name == "weapon_aug") { Color::srgb(0.1, 1.0, 0.1) } else { Color::srgb(1.0, 0.1, 0.1) };
            }
        }
    }
}
