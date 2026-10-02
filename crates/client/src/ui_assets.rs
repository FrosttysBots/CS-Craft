//! Original interface art and fonts, read from the user's installation at runtime.
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use std::collections::HashMap;

#[derive(Resource)]
pub struct UiAssets {
    pub regular: Handle<Font>,
    pub bold: Handle<Font>,
    pub images: HashMap<String, Handle<Image>>,
}
impl UiAssets {
    pub fn image(&self, key: &str) -> Option<Handle<Image>> { self.images.get(key).cloned() }
}

/// VFONT1 wraps an sfnt font with a salted byte stream. Decode in memory only.
fn decode_font(mut bytes: Vec<u8>) -> Option<Vec<u8>> {
    let trailer = bytes.len().checked_sub(7)?;
    if &bytes[trailer + 1..] != b"VFONT1" { return None; }
    let salt_len = bytes[trailer] as usize;
    if salt_len == 0 { return None; }
    let end = (trailer + 1).checked_sub(salt_len)?;
    let mut key = 0xa7u8;
    for &salt in &bytes[end..trailer] { key ^= salt.wrapping_add(0xa7); }
    for b in &mut bytes[..end] {
        let encoded = *b;
        *b ^= key;
        key = encoded.wrapping_add(0xa7);
    }
    bytes.truncate(end);
    if !bytes.starts_with(&[0, 1, 0, 0]) && !bytes.starts_with(b"OTTO") { return None; }
    Some(bytes)
}

pub fn load(mut commands: Commands, level: Res<crate::LoadedLevel>, mut fonts: ResMut<Assets<Font>>, mut images: ResMut<Assets<Image>>) {
    let fs = &level.0.vfs;
    let mut font = |name: &str| {
        fs.read(&format!("panorama/fonts/{name}.vfont")).and_then(decode_font).map(|bytes| fonts.add(Font::from_bytes(bytes))).unwrap_or_else(|| {
            warn!("Missing or invalid original font: {name}"); Handle::default()
        })
    };
    let regular = font("stratum2regular");
    let bold = font("stratum2bold");
    let mut art = HashMap::new();
    if let Ok(root)=overworld::asset_root(){
        if let Ok(bytes)=std::fs::read(root.join("resourcepacks/local/minecraft-26.3/assets/minecraft/textures/block/grass_block_side.png")){
            if let Ok(img)=Image::from_buffer(&bytes,ImageType::Extension("png"),CompressedImageFormats::NONE,true,ImageSampler::nearest(),RenderAssetUsages::RENDER_WORLD){art.insert("map-overworld".into(),images.add(img));}
        }
    }
    let mut png = |key: &str, path: &str| {
        if let Some(bytes) = fs.read(path) {
            if let Ok(img) = Image::from_buffer(&bytes, ImageType::Extension("png"), CompressedImageFormats::NONE, true, ImageSampler::linear(), RenderAssetUsages::RENDER_WORLD) {
                art.insert(key.to_string(), images.add(img));
            }
        }
    };
    for (key, path) in [
        ("health", "healtharmor/icon-cross1"), ("armor", "healtharmor/icon-shield"),
        ("helmet", "healtharmor/icon-armor-helmet"), ("cart", "money/icon-cart"),
        ("c4", "weaponpanel/icon-c4"), ("defuser", "weaponpanel/icon-defuser"),
        ("avatar", "teamcounter/teamcounter_botavatar"),
        ("ct", "winpanel/ct_patch"), ("t", "winpanel/t_patch"),
        ("logo", "freezepanel/logo"), ("radar-player", "radar/icon-player-indicator"),
        ("radar-ct", "radar/icon-ct-on-map"), ("radar-t", "radar/icon-t-on-map"),
    ] { png(key, &format!("materials/panorama/images/hud/{path}.png")); }
    for (side, model) in [("ct", "ctm_sas"), ("t", "tm_phoenix")] {
        png(&format!("character-{side}"), &format!("resource/flash/econ/characters/customplayer_{model}.png"));
    }
    png("knife_default_t", "materials/panorama/images/hud/deathnotice/icon-knife_default_t.png");
    for weapon in &level.0.catalog.weapons {
        let name = weapon.class_name.trim_start_matches("weapon_");
        png(name, &format!("materials/panorama/images/hud/deathnotice/icon-{name}.png"));
        png(&format!("item-{name}"), &format!("resource/flash/econ/weapons/base_weapons/weapon_{name}.png"));
    }
    for path in fs.archive_paths_with_prefix("materials/panorama/images/map_icons/screenshots/360p/") {
        if let Some(map)=path.rsplit('/').next().and_then(|p|p.strip_suffix(".png")) { png(&format!("map-{map}"), &path); }
    }
    png("menu-background", "materials/panorama/images/map_icons/screenshots/1080p/de_dust2.png");
    info!("Original UI: Stratum regular/bold, {} images", art.len());
    commands.insert_resource(UiAssets { regular, bold, images: art });
}

/// Covers text spawned later by menus, too. Runs before Bevy's text layout.
pub fn fonts(assets: Res<UiAssets>, mut text: Query<&mut TextFont, Added<TextFont>>) {
    for mut font in &mut text {
        let bold = matches!(font.font_size, FontSize::Px(size) if size >= 24.0);
        font.font = bevy::text::FontSource::Handle(if bold { assets.bold.clone() } else { assets.regular.clone() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_salted_font_and_rejects_bad_trailers() {
        let data = b"\0\x01\0\0testfont";
        let salts = [5u8, 27];
        let mut key = salts.iter().fold(0xa7u8, |key, s| key ^ s.wrapping_add(0xa7));
        let mut encoded = Vec::new();
        for &b in data { let value = b ^ key; encoded.push(value); key = value.wrapping_add(0xa7); }
        encoded.extend(salts); encoded.push(3); encoded.extend(b"VFONT1");
        assert_eq!(decode_font(encoded.clone()).as_deref(), Some(data.as_slice()));
        for n in 0..encoded.len() { assert!(decode_font(encoded[..n].to_vec()).is_none()); }
        let at = encoded.len() - 7; encoded[at] = 255;
        assert!(decode_font(encoded).is_none());
    }
}
