//! Explosions and primed TNT on the server level (26.3 `ServerExplosion`,
//! `ExplosionDamageCalculator`, `ServerLevel.explode`, `PrimedTnt`,
//! `TntBlock`, `BlockGetter.clip` with `ClipContext.Block.COLLIDER`).
//!
//! Rays from the centre pick the exploded blocks with the level random;
//! vanilla collects them in a `HashSet`, whose iteration order is emulated
//! before `Util.shuffle` reorders them with the level random again.
//! Entities are pushed by the fraction of sample points that see the
//! centre; item entities lose health. Blocks drop their loot through the
//! 16-item stack collectors, containers spill their contents and TNT hit by
//! the blast is primed with a short fuse.

use super::container::Stack;
use super::entity::{Entity, EntityKind, TntData};
use super::physics::{Aabb, ClipBlocks};
use super::{update, Level};
use minecraftoss_core::loot::LootParams;
use minecraftoss_core::random::RandomSource;
use minecraftoss_core::{BlockPos, BlockStateId};
use std::collections::HashSet;

/// `Explosion.BlockInteraction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockInteraction {
    Keep,
    Destroy,
    DestroyWithDecay,
    TriggerBlock,
}

/// One `ServerExplosion`.
#[derive(Clone, Copy, Debug)]
pub struct Explosion {
    pub center: [f64; 3],
    pub radius: f32,
    pub fire: bool,
    pub interaction: BlockInteraction,
    /// The direct source entity's ID (`source`), excluded from the blast.
    pub source: Option<i32>,
}

/// `Mth.lerp`.
fn lerp(delta: f64, a: f64, b: f64) -> f64 {
    a + delta * (b - a)
}

impl Level<'_> {
    /// `TntBlock.prime` without a source: a primed TNT entity at the block.
    pub(super) fn prime_tnt(&mut self, pos: BlockPos) -> bool {
        if !self.tnt_explodes {
            return false;
        }
        let tnt = self.new_primed_tnt([f64::from(pos.x) + 0.5, f64::from(pos.y), f64::from(pos.z) + 0.5]);
        self.add_entity(tnt);
        // The priming sound's seed comes from the sound seed generator.
        true
    }

    /// `PrimedTnt(level, x, y, z, owner)`: a random horizontal nudge.
    pub(super) fn new_primed_tnt(&mut self, pos: [f64; 3]) -> Entity {
        let rot = self.random.next_f64() * f64::from(std::f32::consts::TAU);
        let delta = [-rot.sin() * 0.02, f64::from(0.2f32), -rot.cos() * 0.02];
        Entity::primed_tnt(pos, delta, TntData { fuse: 80, power: 4.0 })
    }

    /// `TntBlock.onPlace` / `neighborChanged`.
    pub(super) fn tnt_block_changed(&mut self, pos: BlockPos) {
        if self.has_neighbor_signal(pos) && self.prime_tnt(pos) {
            self.remove_block(pos, false);
        }
    }

    /// `PrimedTnt.tick` (it skips `baseTick`).
    pub(super) fn tick_tnt(&mut self, e: &mut Entity) {
        let gravity = e.gravity();
        if gravity != 0.0 {
            e.delta = [e.delta[0] + 0.0, e.delta[1] - gravity, e.delta[2] + 0.0];
        }
        let delta = e.delta;
        self.entity_move_self(e, delta);
        self.entity_apply_effects_from_blocks(e);
        let drag = f64::from(0.98f32);
        e.delta = [e.delta[0] * drag, e.delta[1] * drag, e.delta[2] * drag];
        if e.on_ground {
            e.delta = [e.delta[0] * 0.7, e.delta[1] * -0.5, e.delta[2] * 0.7];
        }
        let EntityKind::PrimedTnt(data) = &mut e.kind else { return };
        data.fuse -= 1;
        if data.fuse <= 0 {
            let power = data.power;
            e.removed = true;
            if self.tnt_explodes {
                // `getY(0.0625)`: a sixteenth of the way up the box.
                let y = e.pos[1] + f64::from(e.height) * 0.0625;
                let interaction = if self.tnt_explosion_drop_decay { BlockInteraction::DestroyWithDecay } else { BlockInteraction::Destroy };
                self.explode(Explosion { center: [e.pos[0], y, e.pos[2]], radius: power, fire: false, interaction, source: Some(e.id) });
            }
        } else {
            self.entity_update_fluid_interaction(e);
        }
    }

    /// `ServerExplosion.explode`; returns the number of blocks it picked.
    pub fn explode(&mut self, ex: Explosion) -> usize {
        let to_blow = self.exploded_positions(&ex);
        self.hurt_entities(&ex);
        if ex.interaction != BlockInteraction::Keep {
            self.interact_with_blocks(&ex, to_blow.clone());
        }
        if ex.fire {
            self.unsupported.push("fire from explosions".to_owned());
        }
        to_blow.len()
    }

    /// `ExplosionDamageCalculator.getBlockExplosionResistance`.
    fn explosion_resistance(&self, state: BlockStateId) -> Option<f32> {
        let blocks = &self.registries().blocks;
        let fluid = self.fluid_state(state);
        if blocks.is_air(state) && fluid.is_none() {
            return None;
        }
        // Water and lava both resist with 100.
        let fluid_resistance = if fluid.is_some() { 100.0f32 } else { 0.0 };
        Some(blocks.state(state).explosion_resistance.max(fluid_resistance))
    }

    /// `ServerExplosion.calculateExplodedPositions`, in the iteration order
    /// of vanilla's `HashSet`.
    fn exploded_positions(&mut self, ex: &Explosion) -> Vec<BlockPos> {
        let mut seen = HashSet::new();
        let mut inserted = Vec::new();
        for xx in 0..16 {
            for yy in 0..16 {
                for zz in 0..16 {
                    if !(xx == 0 || xx == 15 || yy == 0 || yy == 15 || zz == 0 || zz == 15) {
                        continue;
                    }
                    let unit = |i: i32| f64::from(i as f32 / 15.0f32 * 2.0f32 - 1.0f32);
                    let (mut xd, mut yd, mut zd) = (unit(xx), unit(yy), unit(zz));
                    let d = (xd * xd + yd * yd + zd * zd).sqrt();
                    xd /= d;
                    yd /= d;
                    zd /= d;
                    let mut remaining = ex.radius * (0.7f32 + self.random.next_f32() * 0.6f32);
                    let [mut xp, mut yp, mut zp] = ex.center;
                    let step = f64::from(0.3f32);
                    while remaining > 0.0 {
                        let pos = BlockPos::new(xp.floor() as i32, yp.floor() as i32, zp.floor() as i32);
                        let state = self.block(pos);
                        if self.outside(pos.y) || pos.x.abs() >= 30_000_000 || pos.z.abs() >= 30_000_000 {
                            break;
                        }
                        if let Some(resistance) = self.explosion_resistance(state) {
                            remaining -= (resistance + 0.3f32) * 0.3f32;
                        }
                        if remaining > 0.0 && seen.insert(pos) {
                            inserted.push(pos);
                        }
                        xp += xd * step;
                        yp += yd * step;
                        zp += zd * step;
                        remaining -= 0.225_000_01_f32;
                    }
                }
            }
        }
        super::piston::java_hash_order(&inserted, inserted.len())
    }

    /// `ServerExplosion.getSeenPercent`.
    fn seen_percent(&self, center: [f64; 3], bb: &Aabb) -> f32 {
        let xs = 1.0 / ((bb.max[0] - bb.min[0]) * 2.0 + 1.0);
        let ys = 1.0 / ((bb.max[1] - bb.min[1]) * 2.0 + 1.0);
        let zs = 1.0 / ((bb.max[2] - bb.min[2]) * 2.0 + 1.0);
        let x_offset = (1.0 - (1.0 / xs).floor() * xs) / 2.0;
        let z_offset = (1.0 - (1.0 / zs).floor() * zs) / 2.0;
        if xs < 0.0 || ys < 0.0 || zs < 0.0 {
            return 0.0;
        }
        let (mut hits, mut count) = (0i32, 0i32);
        let mut xx = 0.0;
        while xx <= 1.0 {
            let mut yy = 0.0;
            while yy <= 1.0 {
                let mut zz = 0.0;
                while zz <= 1.0 {
                    let x = lerp(xx, bb.min[0], bb.max[0]);
                    let y = lerp(yy, bb.min[1], bb.max[1]);
                    let z = lerp(zz, bb.min[2], bb.max[2]);
                    if !self.clip_hits([x + x_offset, y, z + z_offset], center, ClipBlocks::Collider, false) {
                        hits += 1;
                    }
                    count += 1;
                    zz += zs;
                }
                yy += ys;
            }
            xx += xs;
        }
        hits as f32 / count as f32
    }

    /// `ServerExplosion.hurtEntities`.
    fn hurt_entities(&mut self, ex: &Explosion) {
        if ex.radius < 1.0e-5 {
            return;
        }
        let double_radius = ex.radius * 2.0;
        let r = f64::from(double_radius);
        let lo = |c: f64| f64::from((c - r - 1.0).floor() as i32);
        let hi = |c: f64| f64::from((c + r + 1.0).floor() as i32);
        let area = Aabb::new(lo(ex.center[0]), lo(ex.center[1]), lo(ex.center[2]), hi(ex.center[0]), hi(ex.center[1]), hi(ex.center[2]));
        let targets: Vec<usize> =
            (0..self.entities.len()).filter(|&i| !self.entities[i].removed && Some(self.entities[i].id) != ex.source && self.entities[i].bb.intersects(&area)).collect();
        for i in targets {
            let e = &self.entities[i];
            if matches!(e.kind, EntityKind::LightningBolt(_)) {
                continue;
            }
            let dx = e.pos[0] - ex.center[0];
            let dy = e.pos[1] - ex.center[1];
            let dz = e.pos[2] - ex.center[2];
            let dist = (dx * dx + dy * dy + dz * dz).sqrt() / r;
            if dist > 1.0 {
                continue;
            }
            let origin = [e.pos[0], e.pos[1] + f64::from(e.eye_height()), e.pos[2]];
            let direction = super::entity::normalize([origin[0] - ex.center[0], origin[1] - ex.center[1], origin[2] - ex.center[2]]);
            let exposure = self.seen_percent(ex.center, &self.entities[i].bb);
            // `getEntityDamageAmount`, then `hurtServer`.
            let pow = (1.0 - dist) * f64::from(exposure);
            let damage = ((pow * pow + pow) / 2.0 * 7.0 * r + 1.0) as f32;
            let knockback = (1.0 - dist) * f64::from(exposure) * f64::from(1.0f32) * (1.0 - 0.0);
            let explosion_proof = self.entities[i].item_data().is_some_and(|d| self.explosion_proof_item(&d.stack));
            let e = &mut self.entities[i];
            if let EntityKind::Item(data) = &mut e.kind {
                if !explosion_proof {
                    data.health = (data.health as f32 - damage) as i32;
                    if data.health <= 0 {
                        e.removed = true;
                    }
                }
            }
            let push = [direction[0] * knockback, direction[1] * knockback, direction[2] * knockback];
            if push.iter().all(|v| v.is_finite()) {
                e.delta = [e.delta[0] + push[0], e.delta[1] + push[1], e.delta[2] + push[2]];
            }
        }
    }

    /// Items with `damage_resistant` against explosions (`canBeHurtBy`).
    fn explosion_proof_item(&self, stack: &Stack) -> bool {
        stack.id == "minecraft:nether_star"
    }

    /// `ServerExplosion.interactWithBlocks`.
    fn interact_with_blocks(&mut self, ex: &Explosion, mut targets: Vec<BlockPos>) {
        // `Util.shuffle`.
        let mut i = targets.len();
        while i > 1 {
            let j = self.random.next_i32_bound(i as i32) as usize;
            targets.swap(i - 1, j);
            i -= 1;
        }
        let mut stacks: Vec<(BlockPos, Stack)> = Vec::new();
        for pos in targets {
            let state = self.block(pos);
            self.on_explosion_hit(state, pos, ex, &mut stacks);
        }
        for (pos, stack) in stacks {
            self.pop_resource(pos, stack);
        }
    }

    /// `BlockBehaviour.onExplosionHit` (and `TntBlock`'s `wasExploded`).
    fn on_explosion_hit(&mut self, state: BlockStateId, pos: BlockPos, ex: &Explosion, stacks: &mut Vec<(BlockPos, Stack)>) {
        if self.registries().blocks.is_air(state) || ex.interaction == BlockInteraction::TriggerBlock {
            return;
        }
        for class in ["BeehiveBlock", "CreakingHeartBlock", "BellBlock", "AbstractCandleBlock"] {
            if self.is_a(state, class) {
                self.unsupported.push(format!("explosion hitting {class}"));
            }
        }
        let tnt = self.is_a(state, "TntBlock");
        if !tnt {
            let params = LootParams {
                origin: Some([f64::from(pos.x) + 0.5, f64::from(pos.y) + 0.5, f64::from(pos.z) + 0.5]),
                tool: Some(Stack::empty()),
                this_entity: ex.source.is_some(),
                explosion_radius: (ex.interaction == BlockInteraction::DestroyWithDecay).then_some(ex.radius),
                ..LootParams::default()
            };
            // `spawnAfterBreak` without the player experience hack: no draws.
            let registries = self.lib.registries.clone();
            match registries.loot.block_drops(&registries, state, &params, &mut self.random_sequences, &mut self.random) {
                Ok(drops) => {
                    for stack in drops {
                        add_or_append_stack(self, stacks, stack, pos);
                    }
                }
                Err(e) => self.unsupported.push(format!("drops of {}: {e}", self.name(state))),
            }
        }
        self.set_block(pos, BlockStateId::AIR, update::ALL, update::LIMIT);
        if tnt && self.tnt_explodes {
            // `wasExploded`: a primed TNT with a short random fuse.
            let mut primed = self.new_primed_tnt([f64::from(pos.x) + 0.5, f64::from(pos.y), f64::from(pos.z) + 0.5]);
            let fuse = self.random.next_i32_bound((80 / 4).max(1)) + 80 / 8;
            if let EntityKind::PrimedTnt(data) = &mut primed.kind {
                data.fuse = fuse;
            }
            self.add_entity(primed);
        }
    }
}

/// `ServerExplosion.addOrAppendStack`: merge into earlier collectors up to
/// 16 items each (`ItemEntity.areMergable`/`merge`).
fn add_or_append_stack(level: &Level, stacks: &mut Vec<(BlockPos, Stack)>, mut stack: Stack, pos: BlockPos) {
    for (_, collected) in stacks.iter_mut() {
        let max = level.lib.registries.items.max_stack(&stack.id);
        if collected.count + stack.count <= max && collected.same_item_same_components(&stack) {
            let delta = (max.min(16) - collected.count).min(stack.count);
            collected.count += delta;
            stack.count -= delta;
        }
        if stack.is_empty() {
            return;
        }
    }
    stacks.push((pos, stack));
}
