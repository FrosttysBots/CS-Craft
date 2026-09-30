//! VTF → GPU textures, cached by path.
//!
//! DXT textures with block-aligned sizes are uploaded compressed with their
//! whole mip chain; everything else is decoded to RGBA8 on the CPU.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct TextureInfo {
    pub handle: Handle<Image>,
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
}

#[derive(Default, Resource)]
pub struct TextureCache {
    loaded: HashMap<String, Option<TextureInfo>>,
    pub white: Handle<Image>,
    pub black: Handle<Image>,
    pub missing: Handle<Image>,
    pub stats_compressed: usize,
    pub stats_decoded: usize,
    pub stats_missing: usize,
}

fn sampler_for(flags: u32) -> ImageSampler {
    let clamp_s = flags & vtf::flags::CLAMPS != 0;
    let clamp_t = flags & vtf::flags::CLAMPT != 0;
    let point = flags & vtf::flags::POINTSAMPLE != 0;
    let filter = if point { ImageFilterMode::Nearest } else { ImageFilterMode::Linear };
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: if clamp_s { ImageAddressMode::ClampToEdge } else { ImageAddressMode::Repeat },
        address_mode_v: if clamp_t { ImageAddressMode::ClampToEdge } else { ImageAddressMode::Repeat },
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: filter,
        min_filter: filter,
        mipmap_filter: filter,
        anisotropy_clamp: if point { 1 } else { 16 },
        ..Default::default()
    })
}

/// A 1x1 texture of one colour.
pub fn solid(color: [u8; 4]) -> Image {
    let mut img = Image::new(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        color.to_vec(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = sampler_for(0);
    img
}

fn checkerboard() -> Image {
    let n = 16u32;
    let mut data = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let on = ((x / 4) + (y / 4)) % 2 == 0;
            data.extend_from_slice(if on { &[255, 0, 255, 255] } else { &[20, 20, 20, 255] });
        }
    }
    let mut img = Image::new(
        Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = sampler_for(vtf::flags::POINTSAMPLE);
    img
}

impl TextureCache {
    pub fn new(images: &mut Assets<Image>) -> Self {
        Self {
            white: images.add(solid([255, 255, 255, 255])),
            black: images.add(solid([0, 0, 0, 255])),
            missing: images.add(checkerboard()),
            ..Default::default()
        }
    }

    /// Loads a texture by `$basetexture`-style name.
    pub fn load(&mut self, fs: &vfs::Vfs, images: &mut Assets<Image>, name: &str, srgb: bool) -> Option<TextureInfo> {
        let path = vmt::vtf_path(name);
        let key = format!("{path}|{srgb}");
        if let Some(hit) = self.loaded.get(&key) {
            return hit.clone();
        }
        let result = fs
            .read(&path)
            .and_then(|bytes| vtf::Vtf::parse(&bytes).ok())
            .and_then(|v| self.upload(&v, images, srgb));
        if result.is_none() {
            self.stats_missing += 1;
        }
        self.loaded.insert(key, result.clone());
        result
    }

    fn upload(&mut self, v: &vtf::Vtf, images: &mut Assets<Image>, srgb: bool) -> Option<TextureInfo> {
        let (w, h) = (v.width as u32, v.height as u32);
        if w == 0 || h == 0 {
            return None;
        }
        let bc = match v.format {
            vtf::Format::Dxt1 | vtf::Format::Dxt1OneBitAlpha => {
                Some(if srgb { TextureFormat::Bc1RgbaUnormSrgb } else { TextureFormat::Bc1RgbaUnorm })
            }
            vtf::Format::Dxt3 => Some(if srgb { TextureFormat::Bc2RgbaUnormSrgb } else { TextureFormat::Bc2RgbaUnorm }),
            vtf::Format::Dxt5 => Some(if srgb { TextureFormat::Bc3RgbaUnormSrgb } else { TextureFormat::Bc3RgbaUnorm }),
            _ => None,
        };
        let mips = v.mip_count.max(1);
        let (data, format) = match bc {
            Some(fmt) if w % 4 == 0 && h % 4 == 0 => {
                let mut data = Vec::new();
                for level in 0..mips {
                    data.extend_from_slice(v.level(level, 0, 0)?);
                }
                self.stats_compressed += 1;
                (data, fmt)
            }
            _ => {
                let mut data = Vec::new();
                for level in 0..mips {
                    data.extend_from_slice(&v.rgba8(level, 0, 0)?);
                }
                self.stats_decoded += 1;
                (data, if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm })
            }
        };
        let mut img = Image::new(
            Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            TextureDimension::D2,
            data,
            format,
            RenderAssetUsages::RENDER_WORLD,
        );
        img.texture_descriptor.mip_level_count = mips as u32;
        img.sampler = sampler_for(v.flags);
        let has_alpha = v.format.has_alpha()
            || v.flags & (vtf::flags::ONEBITALPHA | vtf::flags::EIGHTBITALPHA) != 0;
        Some(TextureInfo {
            handle: images.add(img),
            width: w,
            height: h,
            has_alpha,
        })
    }
}

/// f32 → IEEE half (round to nearest even), for HDR lightmaps.
pub fn f32_to_f16(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let mant = bits & 0x7F_FFFF;
    if exp == 255 {
        return sign | 0x7C00 | if mant != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7BFF; // clamp to max finite
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = mant | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = (m >> shift) as u16;
        let round = (m >> (shift - 1)) & 1;
        return sign | (half + round as u16);
    }
    let half = sign | ((e as u16) << 10) | (mant >> 13) as u16;
    let round = (mant >> 12) & 1 != 0 && ((mant & 0xFFF) != 0 || (mant >> 13) & 1 != 0);
    half + round as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_round_trip() {
        for v in [0.0f32, 1.0, 0.5, 2.25, 100.0, 0.001, 65504.0, 1e-5] {
            let h = f32_to_f16(v);
            let back = vtf::f16_to_f32(h);
            assert!((back - v).abs() <= v.abs() * 0.001 + 1e-6, "{v} -> {h:#x} -> {back}");
        }
    }
}
