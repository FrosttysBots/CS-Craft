// Adapted from the pinned mashup; see third_party/README.md.
//! The MW2 minimap over the Minecraft world: a top-down picture of the
//! blocks around the player, one pixel a block, coloured by each surface
//! block's texture and shaded by height against its northern neighbour as
//! vanilla's maps shade them.
use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use overworld::core::chunk::HeightmapKind;
use overworld::terrain::mesh::Atlas;
use overworld::terrain::pack::PackStack;
use overworld::terrain::scene::{HandcraftedScene, Scene};

/// Blocks across the picture.
const SIZE: i32 = 128;
/// Seconds between redraws, and the move that forces one.
const REDRAW_SECONDS: f64 = 2.0;
const RECENTRE: i32 = 16;

/// Frames a new picture waits before it shows, so its texture has reached
/// the GPU by the time its corners move with it.
const SWAP_FRAMES: u8 = 3;

/// Two pictures: the one shown and the next one being drawn, which takes
/// over together with its own corners so the map never jumps.
#[derive(Default)]
pub(crate) struct Minimap {
    drawing: Option<((i32,i32), i32, Vec<u8>)>,
    handles: [Option<Handle<Image>>; 2],
    /// The picture shown and its north-west corner block.
    shown: Option<(usize, [i32; 2])>,
    /// The next picture, its corner, and frames until it shows.
    pending: Option<(usize, [i32; 2], u8)>,
    centre: Option<(i32, i32)>,
    since: f64,
    colours: HashMap<String, Option<[f32; 3]>>,
}

impl Minimap {
    /// Redraws the picture around the player when due; the corner blocks it
    /// spans (north-west, south-east).
    pub(crate) fn update(
        &mut self,
        dt: f64,
        feet: [f64; 3],
        scene: &HandcraftedScene,
        packs: &PackStack,
        atlas: &Atlas,
        images: &mut Assets<Image>,
    ) -> Option<(Handle<Image>, [i32; 2])> {
        self.since += dt;
        if let Some((index, corner, frames)) = self.pending.as_mut() {
            if *frames == 0 {
                self.shown = Some((*index, *corner));
                self.pending = None;
            } else {
                *frames -= 1;
            }
        }
        let here = (feet[0].floor() as i32, feet[2].floor() as i32);
        let moved = self
            .centre
            .is_none_or(|(x, z)| (x - here.0).abs() >= RECENTRE || (z - here.1).abs() >= RECENTRE);
        if self.pending.is_none() && self.drawing.is_none() && (moved || self.since >= REDRAW_SECONDS) {
            self.since = 0.0;
            let centre = (
                here.0.div_euclid(RECENTRE) * RECENTRE,
                here.1.div_euclid(RECENTRE) * RECENTRE,
            );
            self.centre = Some(centre);
            self.drawing=Some((centre,0,vec![0u8;(SIZE*SIZE*4) as usize]));
        }
        if let Some((centre,row,mut pixels))=self.drawing.take() {
            let end=(row+8).min(SIZE);
            self.draw_rows(centre,row,end,&mut pixels,scene,packs,atlas);
            if end<SIZE {self.drawing=Some((centre,end,pixels));}
            else {
            let image = Image::new(
                Extent3d {
                    width: SIZE as u32,
                    height: SIZE as u32,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                pixels,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::default(),
            );
            let index = self.shown.map_or(0, |(shown, _)| 1 - shown);
            match self.handles[index].as_ref() {
                Some(handle) => {
                    let _ = images.insert(handle.id(), image);
                }
                None => self.handles[index] = Some(images.add(image)),
            }
            self.pending = Some((
                index,
                [centre.0 - SIZE / 2, centre.1 - SIZE / 2],
                SWAP_FRAMES,
            ));
        }
        }
        let (index, corner) = self.shown?;
        Some((self.handles[index].clone()?, corner))
    }

    fn draw_rows(
        &mut self,
        (cx, cz): (i32, i32),
        start: i32,
        end: i32,
        out: &mut [u8],
        scene: &HandcraftedScene,
        packs: &PackStack,
        atlas: &Atlas,
    ) {
        let (x0, z0) = (cx - SIZE / 2, cz - SIZE / 2);
        let mut heights = vec![i32::MIN; (SIZE * (end - start + 1)) as usize];
        let height_at = |x: i32, z: i32| -> Option<i32> {
            let chunk = scene.generated_chunk((x >> 4, z >> 4))?;
            Some(
                chunk.heightmaps.get(
                    HeightmapKind::WorldSurface,
                    (x & 15) as usize,
                    (z & 15) as usize,
                ) - 1,
            )
        };
        // One row north of the picture too, for the first row's shading.
        for row in start..=end {
            for col in 0..SIZE {
                if let Some(h) = height_at(x0 + col, z0 + row - 1) {
                    heights[((row-start) * SIZE + col) as usize] = h;
                }
            }
        }
        for row in start..end {
            for col in 0..SIZE {
                let h = heights[((row-start+1) * SIZE + col) as usize];
                if h == i32::MIN {
                    continue;
                }
                let (x, z) = (x0 + col, z0 + row);
                // The surface block, stepping down past air left by edits.
                let mut y = h;
                let block = loop {
                    match Scene::block(scene, (x, y, z)) {
                        Some(block) => break Some(block),
                        None if y > h - 8 => y -= 1,
                        None => break None,
                    }
                };
                let Some(block) = block else { continue };
                let Some(colour) = self.colour(block, packs, atlas) else {
                    continue;
                };
                let north = heights[((row-start) * SIZE + col) as usize];
                let shade = if north == i32::MIN || y == north {
                    220.0 / 255.0
                } else if y > north {
                    1.0
                } else {
                    180.0 / 255.0
                };
                let at = ((row * SIZE + col) * 4) as usize;
                for k in 0..3 {
                    out[at + k] = (colour[k] * shade * 255.0).clamp(0.0, 255.0) as u8;
                }
                out[at + 3] = 255;
            }
        }
    }

    /// A block's colour on the map: its tint for grass, foliage and water,
    /// else the average of its particle texture.
    fn colour(
        &mut self,
        block: &overworld::terrain::scene::Block,
        packs: &PackStack,
        atlas: &Atlas,
    ) -> Option<[f32; 3]> {
        let path = block.id.path.as_str();
        let tinted = match path {
            "grass_block" | "short_grass" | "tall_grass" | "fern" | "large_fern" => {
                Some([0.49, 0.72, 0.33])
            }
            p if p.ends_with("_leaves") => Some([0.30, 0.55, 0.20]),
            "water" | "bubble_column" | "kelp" | "kelp_plant" | "seagrass" | "tall_seagrass" => {
                Some([0.25, 0.42, 0.85])
            }
            "lava" => Some([0.85, 0.35, 0.05]),
            _ => None,
        };
        if tinted.is_some() {
            return tinted;
        }
        let key = block.id.key();
        if let Some(cached) = self.colours.get(&key) {
            return *cached;
        }
        let colour = overworld::terrain::model::block_particle_texture(packs, block)
            .ok()
            .flatten()
            .filter(|texture| atlas.contains(texture))
            .and_then(|texture| {
                let [u0, v0, u1, v1] = atlas.region(&texture);
                let (w, h) = (atlas.pixels.width() as f32, atlas.pixels.height() as f32);
                let (px0, py0, px1, py1) = (
                    (u0 * w) as u32,
                    (v0 * h) as u32,
                    (u1 * w) as u32,
                    (v1 * h) as u32,
                );
                let (mut sum, mut n) = ([0.0f32; 3], 0.0f32);
                for y in py0..py1.min(atlas.pixels.height()) {
                    for x in px0..px1.min(atlas.pixels.width()) {
                        let p = atlas.pixels.get_pixel(x, y);
                        if p[3] > 128 {
                            for k in 0..3 {
                                sum[k] += f32::from(p[k]) / 255.0;
                            }
                            n += 1.0;
                        }
                    }
                }
                (n > 0.0).then(|| sum.map(|s| s / n))
            });
        self.colours.insert(key, colour);
        colour
    }
}
