//! Structure pieces placed from one template (vanilla
//! `TemplateStructurePiece`): igloos, shipwrecks, ocean ruins, End cities,
//! mansions and Nether fossils.

use crate::feature::template::{BoundingBox, PlaceSettings, Template};
use crate::feature::{Ctx, Library};
use crate::structure::piece::PieceBase;
use minecraftoss_core::nbt::Tag;
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::WorldgenRandom;
use minecraftoss_core::BlockPos;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct TemplatePiece {
    pub base: PieceBase,
    pub template_name: String,
    pub template: Arc<Template>,
    pub settings: PlaceSettings,
    pub position: BlockPos,
}

impl TemplatePiece {
    pub fn new(lib: &Library, gen_depth: i32, template_name: &str, settings: PlaceSettings, position: BlockPos) -> Self {
        let template = lib.templates.get(&lib.registries, template_name);
        let mut base = PieceBase::new(gen_depth, template.bounding_box(&settings, position));
        base.set_orientation(Some(Direction::North));
        Self { base, template_name: template_name.to_owned(), template, settings, position }
    }

    /// `TemplateStructurePiece.move`.
    pub fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base.bbox = self.base.bbox.moved(dx, dy, dz);
        self.position = self.position.offset(dx, dy, dz);
    }

    /// `TemplateStructurePiece.postProcess`: the template clipped to the
    /// chunk (its bounds recomputed from the current position), then its
    /// data markers through `marker`, then jigsaw blocks replaced by their
    /// final states.
    pub fn place(
        &mut self,
        ctx: &mut Ctx,
        random: &mut WorldgenRandom,
        chunk_bb: &BoundingBox,
        reference: BlockPos,
        marker: &mut dyn FnMut(&mut Ctx, &str, BlockPos, &mut WorldgenRandom, &BoundingBox),
    ) {
        self.settings.bounding_box = Some(*chunk_bb);
        self.base.bbox = self.template.bounding_box(&self.settings, self.position);
        if !self.template.place_in_world(ctx, self.position, reference, &self.settings, random, 2) {
            return;
        }
        let lib = ctx.lib;
        for info in self.template.filter_blocks(lib, self.position, &self.settings, "minecraft:structure_block", random) {
            let Some(nbt) = &info.nbt else { continue };
            if nbt.get("mode").and_then(Tag::as_str).is_some_and(|m| m.eq_ignore_ascii_case("data")) {
                let metadata = nbt.get("metadata").and_then(Tag::as_str).unwrap_or("").to_owned();
                marker(ctx, &metadata, info.pos, random, chunk_bb);
            }
        }
        for info in self.template.filter_blocks(lib, self.position, &self.settings, "minecraft:jigsaw", random) {
            let Some(nbt) = &info.nbt else { continue };
            let text = nbt.get("final_state").and_then(Tag::as_str).unwrap_or("minecraft:air");
            let text = text.split('{').next().unwrap_or(text);
            let state = lib.registries.blocks.parse_state(text).unwrap_or(lib.blocks.air);
            ctx.set_block_update(info.pos, state);
        }
    }
}
