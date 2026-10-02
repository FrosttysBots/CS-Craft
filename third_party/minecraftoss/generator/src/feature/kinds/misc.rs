//! Template-based features: `TemplateFeature` (desert wells, sulfur
//! springs) and `FossilFeature`.
//!
//! Source-informed from the pinned 26.3 JAR (`feature.TemplateFeature`,
//! `feature.FossilFeature`).

use super::Placeable;
use crate::feature::template::{BoundingBox, Mirror, PlaceSettings, Processor, RandomMode, Rotation};
use crate::feature::{Ctx, Library};
use crate::providers::Weighted;
use minecraftoss_core::chunk::HeightmapKind;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::BlockPos;
use serde_json::Value;

/// Parses a feature of a type this module implements, or `None`.
pub fn parse(lib: &mut Library, kind: &str, json: &Value) -> Option<Result<Box<dyn Placeable>, String>> {
    Some(match kind {
        "template" => TemplateFeature::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        "fossil" => FossilFeature::parse(lib, json).map(|f| Box::new(f) as Box<dyn Placeable>),
        _ => return None,
    })
}

#[derive(Debug)]
struct TemplateEntry {
    id: String,
    rotations: Vec<Rotation>,
}

/// `TemplateFeature`.
#[derive(Debug)]
pub struct TemplateFeature {
    templates: Weighted<TemplateEntry>,
    processors: Vec<Processor>,
}

impl TemplateFeature {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let templates = Weighted::parse(&json["templates"], |entry| {
            let id = entry["id"].as_str().ok_or("template entry lacks id")?.to_owned();
            let rotations = match entry.get("rotations") {
                Some(list) => list
                    .as_array()
                    .ok_or("rotations is not a list")?
                    .iter()
                    .map(|r| r.as_str().and_then(Rotation::parse).ok_or_else(|| format!("bad rotation {r}")))
                    .collect::<Result<_, _>>()?,
                None => Rotation::ALL.to_vec(),
            };
            Ok(TemplateEntry { id, rotations })
        })?;
        let processors = match json.get("processors") {
            Some(list) => lib.processor_list(list)?.as_ref().clone(),
            None => Vec::new(),
        };
        Ok(Self { templates, processors })
    }
}

/// `TemplateFeature.getRotatedOffset`.
fn rotated_offset(rotation: Rotation, negative: Direction, size: i32) -> (i32, i32, i32) {
    let (x, y, z) = rotation.rotate(negative).offset();
    let half = size / 2;
    (x * half, y * half, z * half)
}

impl Placeable for TemplateFeature {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let Some(entry) = self.templates.pick(random) else { return false };
        let rotation = entry.rotations[random.next_i32_bound(entry.rotations.len() as i32) as usize];
        let template = ctx.lib.templates.get(&ctx.lib.registries, &entry.id);
        let (ax, ay, az) = rotated_offset(rotation, Direction::West, template.size[0]);
        let (bx, by, bz) = rotated_offset(rotation, Direction::North, template.size[2]);
        let pos = origin.offset(ax + bx, ay + by, az + bz);
        let settings = PlaceSettings { rotation, random: RandomMode::Shared, processors: self.processors.clone(), ..PlaceSettings::default() };
        template.place_in_world(ctx, pos, pos, &settings, random, 3)
    }
}

/// `FossilFeature`.
#[derive(Debug)]
pub struct FossilFeature {
    fossils: Vec<String>,
    overlays: Vec<String>,
    fossil_processors: Vec<Processor>,
    overlay_processors: Vec<Processor>,
    max_empty_corners: i32,
}

impl FossilFeature {
    fn parse(lib: &mut Library, json: &Value) -> Result<Self, String> {
        let names = |key: &str| -> Result<Vec<String>, String> {
            json[key].as_array().ok_or_else(|| format!("fossil lacks {key}"))?.iter().map(|v| v.as_str().map(str::to_owned).ok_or_else(|| format!("bad {key} entry"))).collect()
        };
        let fossils = names("fossil_structures")?;
        let overlays = names("overlay_structures")?;
        if fossils.len() != overlays.len() || fossils.is_empty() {
            return Err("fossil structure lists must be equal, non-empty lengths".into());
        }
        Ok(Self {
            fossils,
            overlays,
            fossil_processors: lib.processor_list(&json["fossil_processors"])?.as_ref().clone(),
            overlay_processors: lib.processor_list(&json["overlay_processors"])?.as_ref().clone(),
            max_empty_corners: super::int(json, "max_empty_corners_allowed")?,
        })
    }
}

impl Placeable for FossilFeature {
    fn place(&self, ctx: &mut Ctx, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let rotation = Rotation::random(random);
        let index = random.next_i32_bound(self.fossils.len() as i32) as usize;
        let base = ctx.lib.templates.get(&ctx.lib.registries, &self.fossils[index]);
        let overlay = ctx.lib.templates.get(&ctx.lib.registries, &self.overlays[index]);
        let chunk = origin.chunk();
        let (min_x, min_z) = (chunk.min_block_x(), chunk.min_block_z());
        let bounds = BoundingBox::new(min_x - 16, ctx.min_y(), min_z - 16, min_x + 15 + 16, ctx.max_y(), min_z + 15 + 16);
        let mut settings = PlaceSettings { rotation, bounding_box: Some(bounds), random: RandomMode::Shared, ..PlaceSettings::default() };
        let size = base.size(rotation);
        let low = origin.offset(-size[0] / 2, 0, -size[2] / 2);
        let mut lowest = origin.y;
        for x in 0..size[0] {
            for z in 0..size[2] {
                lowest = lowest.min(ctx.height(HeightmapKind::OceanFloorWg, low.x + x, low.z + z));
            }
        }
        let target_y = (lowest - 15 - random.next_i32_bound(10)).max(ctx.min_y() + 10);
        let target = base.zero_position_with_transform(low.at_y(target_y), Mirror::None, rotation);
        let empty = base
            .bounding_box(&settings, target)
            .corners()
            .iter()
            .filter(|&&p| {
                let state = ctx.block(p);
                ctx.is_air(state) || ctx.is(state, "minecraft:lava") || ctx.is(state, "minecraft:water")
            })
            .count() as i32;
        if empty > self.max_empty_corners {
            return false;
        }
        settings.processors = self.fossil_processors.clone();
        base.place_in_world(ctx, target, target, &settings, random, 260);
        settings.processors = self.overlay_processors.clone();
        overlay.place_in_world(ctx, target, target, &settings, random, 260);
        true
    }
}
