//! A world seen with some blocks changed on top of it: what a mob's
//! pathfinding reads between a block change and its landing in the level
//! (a door a villager opens during its brain's tick).
use minecraftoss_player::{authored_collision_boxes, path_type, Block, Pos, World};

/// `base` with `changed` blocks (`None` for air) laid over it; changed
/// positions answer from the authored shapes and path types.
pub struct Overlay<'a> {
    pub base: &'a dyn World,
    pub changed: Vec<(Pos, Option<Block>)>,
}

impl Overlay<'_> {
    fn changed(&self, pos: Pos) -> Option<&Option<Block>> {
        self.changed.iter().rev().find(|(p, _)| *p == pos).map(|(_, b)| b)
    }
}

impl World for Overlay<'_> {
    fn block(&self, pos: Pos) -> Option<Block> {
        match self.changed(pos) {
            Some(block) => block.clone(),
            None => self.base.block(pos),
        }
    }
    fn set_block(&mut self, pos: Pos, block: Option<Block>) {
        self.changed.push((pos, block));
    }
    fn collision_boxes(&self, pos: Pos) -> Vec<[f64; 6]> {
        match self.changed(pos) {
            Some(block) => block.as_ref().map_or_else(Vec::new, authored_collision_boxes),
            None => self.base.collision_boxes(pos),
        }
    }
    fn path_type_from_state(&self, pos: Pos) -> path_type::PathType {
        match self.changed(pos) {
            Some(block) => path_type::path_type_of_block(block.as_ref()),
            None => self.base.path_type_from_state(pos),
        }
    }
    fn pathfindable(&self, pos: Pos) -> bool {
        self.base.pathfindable(pos)
    }
    fn floatable_fluid(&self, pos: Pos) -> bool {
        self.base.floatable_fluid(pos)
    }
    fn solid(&self, pos: Pos) -> bool {
        self.base.solid(pos)
    }
    fn solid_render(&self, pos: Pos) -> bool {
        self.base.solid_render(pos)
    }
    fn light_path_cost(&self, pos: Pos) -> f32 {
        self.base.light_path_cost(pos)
    }
    fn can_see_sky(&self, pos: Pos) -> bool {
        self.base.can_see_sky(pos)
    }
    fn rain_at(&self, pos: Pos) -> bool {
        self.base.rain_at(pos)
    }
    fn min_y(&self) -> i32 {
        self.base.min_y()
    }
    fn max_y(&self) -> i32 {
        self.base.max_y()
    }
    fn block_in_tag(&self, pos: Pos, tag: &str) -> bool {
        self.base.block_in_tag(pos, tag)
    }
    fn step_sound(&self, pos: Pos) -> Option<(String, f32, f32)> {
        self.base.step_sound(pos)
    }
    fn suffocating(&self, pos: Pos) -> bool {
        self.base.suffocating(pos)
    }
    fn sturdy_underside(&self, pos: Pos) -> bool {
        self.base.sturdy_underside(pos)
    }
    fn fall_sound(&self, pos: Pos) -> Option<(String, f32, f32)> {
        self.base.fall_sound(pos)
    }
    fn difficulty_inputs(&self, pos: Pos) -> (i64, i64, f32) {
        self.base.difficulty_inputs(pos)
    }
}
