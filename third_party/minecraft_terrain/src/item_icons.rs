//! Inventory icons and names for items, as MinecraftOSS's viewer makes them
//! (`engine/viewer/src/interface.rs`: `load_catalog_icons`, `item_name`).
use crate::interface::{item_model_reference, item_model_tints};
use crate::pack::{PackStack, ResourceId};
use anyhow::Result;
use image::{imageops::FilterType, RgbaImage};
use std::collections::HashMap;

/// An item's GUI icon: its block model rastered, its generated layers, or
/// its flat texture.
pub fn item_icon(packs: &PackStack, key: &str, icon_size: usize) -> Result<Option<RgbaImage>> {
    let id = ResourceId::parse(key)?;
    let definition = packs.item_definition(&id)?;
    let model = definition.as_ref().and_then(|value| item_model_reference(&value["model"]));
    let tints = definition
        .as_ref()
        .map(|value| item_model_tints(packs, &value["model"]))
        .transpose()?
        .unwrap_or_default();
    if let Some(icon) = model
        .map(|model| crate::item_icon::block_icon(packs, model, &tints, icon_size))
        .transpose()?
        .flatten()
    {
        return Ok(Some(icon));
    }
    if let Some(icon) = model
        .map(|model| generated_item_icon(packs, model, &tints, icon_size))
        .transpose()?
        .flatten()
    {
        return Ok(Some(icon));
    }
    let texture = model.map(|model| model_texture(packs, model)).transpose()?.flatten();
    let texture = texture.or_else(|| ResourceId::parse(&format!("{}:item/{}", id.namespace, id.path)).ok());
    let Some(texture) = texture else { return Ok(None) };
    let Some(bytes) = packs.texture(&texture)? else { return Ok(None) };
    let decoded = image::load_from_memory(&bytes)?.to_rgba8();
    let side = decoded.width().min(decoded.height());
    let first = image::imageops::crop_imm(&decoded, 0, 0, side, side).to_image();
    Ok(Some(image::imageops::resize(&first, icon_size as u32, icon_size as u32, FilterType::Nearest)))
}

/// The pack's English names (`lang/en_us.json`).
pub fn language(packs: &PackStack) -> Result<HashMap<String, String>> {
    let id = ResourceId::parse("minecraft:lang/en_us")?;
    let Some(value) = packs.json(&id, "lang/en_us.json")? else {
        return Ok(HashMap::new());
    };
    Ok(value
        .as_object()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(key, text)| Some((key.clone(), text.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default())
}

/// `item_name`: the item's or its block's name, else its id's path.
pub fn item_name(language: &HashMap<String, String>, id: &str) -> String {
    let path = id.split(':').nth(1).unwrap_or(id);
    language
        .get(&format!("item.minecraft.{path}"))
        .or_else(|| language.get(&format!("block.minecraft.{path}")))
        .cloned()
        .unwrap_or_else(|| path.replace('_', " "))
}


/// Item/generated renders each layer in order; layer 0's tint is the liquid,
/// and the untinted bottle on layer 1 is drawn over it.
fn generated_item_icon(
    packs: &PackStack,
    model: &str,
    tints: &[[u8; 3]],
    icon_size: usize,
) -> Result<Option<RgbaImage>> {
    let mut textures = HashMap::<String, String>::new();
    let mut current = Some(ResourceId::parse(model)?);
    for _ in 0..12 {
        let Some(id) = current.take() else { break };
        let Some(value) = packs.model(&id)? else {
            break;
        };
        if let Some(entries) = value.get("textures").and_then(serde_json::Value::as_object) {
            for (key, texture) in entries {
                if let Some(texture) = texture
                    .as_str()
                    .or_else(|| texture.get("sprite").and_then(serde_json::Value::as_str))
                {
                    textures
                        .entry(key.clone())
                        .or_insert_with(|| texture.into());
                }
            }
        }
        current = value
            .get("parent")
            .and_then(serde_json::Value::as_str)
            .map(ResourceId::parse)
            .transpose()?;
    }
    if !textures.contains_key("layer1") {
        return Ok(None);
    }
    let mut icon = RgbaImage::new(icon_size as u32, icon_size as u32);
    for layer in 0..16 {
        let key = format!("layer{layer}");
        let Some(mut texture) = textures.get(&key).map(String::as_str) else {
            break;
        };
        for _ in 0..8 {
            if let Some(reference) = texture.strip_prefix('#') {
                let Some(next) = textures.get(reference) else {
                    break;
                };
                texture = next;
            } else {
                break;
            }
        }
        let Some(bytes) = packs.texture(&ResourceId::parse(texture)?)? else {
            continue;
        };
        let source = image::load_from_memory(&bytes)?.to_rgba8();
        let side = source.width().min(source.height());
        if side == 0 {
            continue;
        }
        let tint = tints.get(layer).copied().unwrap_or([255; 3]);
        let mut frame = image::imageops::crop_imm(&source, 0, 0, side, side).to_image();
        for pixel in frame.pixels_mut() {
            for channel in 0..3 {
                pixel[channel] = (pixel[channel] as u16 * tint[channel] as u16 / 255) as u8;
            }
        }
        let scaled = image::imageops::resize(
            &frame,
            icon_size as u32,
            icon_size as u32,
            FilterType::Nearest,
        );
        image::imageops::overlay(&mut icon, &scaled, 0, 0);
    }
    Ok((icon.pixels().any(|pixel| pixel[3] > 0)).then_some(icon))
}

fn model_texture(packs: &PackStack, model: &str) -> Result<Option<ResourceId>> {
    let mut textures = HashMap::<String, String>::new();
    let mut current = Some(ResourceId::parse(model)?);
    for _ in 0..12 {
        let Some(id) = current.take() else { break };
        let Some(value) = packs.model(&id)? else {
            break;
        };
        if let Some(entries) = value.get("textures").and_then(serde_json::Value::as_object) {
            for (key, texture) in entries {
                if let Some(texture) = texture
                    .as_str()
                    .or_else(|| texture.get("sprite").and_then(serde_json::Value::as_str))
                {
                    textures
                        .entry(key.clone())
                        .or_insert_with(|| texture.into());
                }
            }
        }
        current = value
            .get("parent")
            .and_then(serde_json::Value::as_str)
            .map(ResourceId::parse)
            .transpose()?;
    }
    for key in [
        "layer0", "all", "top", "up", "side", "front", "end", "particle",
    ] {
        let Some(mut texture) = textures.get(key).map(String::as_str) else {
            continue;
        };
        for _ in 0..8 {
            if let Some(reference) = texture.strip_prefix('#') {
                let Some(next) = textures.get(reference) else {
                    break;
                };
                texture = next;
            } else {
                return ResourceId::parse(texture).map(Some);
            }
        }
    }
    Ok(None)
}
