//! Copied from MinecraftOSS (`engine/viewer/src/block_particles.rs`); the camera is its forward vector.
use crate::{
    mesh::{redstone_wire_color_argb, Atlas, BiomeTint, ChunkMesh, Vertex},
    model,
    pack::{PackStack, ResourceId},
    scene::{Block, BlockPos, Scene},
};
use anyhow::Result;
use glam::{Vec2, Vec3};
use std::collections::HashSet;

struct Fragment {
    position: Vec3,
    previous: Vec3,
    velocity: Vec3,
    texture: ResourceId,
    uv_origin: Vec2,
    color: [f32; 3],
    size: f32,
    age: u8,
    lifetime: u8,
    stopped: bool,
}

pub struct BlockParticles {
    fragments: Vec<Fragment>,
    tint: BiomeTint,
    rng: u64,
}

impl BlockParticles {
    pub fn new(packs: &PackStack) -> Result<Self> {
        Ok(Self {
            fragments: Vec::new(),
            tint: BiomeTint::from_pack(packs)?,
            rng: 0x8a49_1a72_5731_6c49,
        })
    }

    pub fn reload(&mut self, packs: &PackStack) -> Result<()> {
        self.tint = BiomeTint::from_pack(packs)?;
        self.fragments.clear();
        Ok(())
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        ((self.rng >> 40) as u32 as f32) / ((1 << 24) as f32)
    }

    pub fn spawn<S: Scene>(
        &mut self,
        packs: &PackStack,
        scene: &S,
        pos: BlockPos,
        block: &Block,
        atlas: &Atlas,
    ) -> Result<()> {
        let Some((texture, tint)) = self.material(packs, scene, pos, block, atlas)? else {
            return Ok(());
        };
        let model = model::resolve_block(packs, block)?;
        let mut seen = HashSet::new();
        for element in model.elements {
            let from = element.from.map(|value| value.clamp(0.0, 1.0));
            let to = element.to.map(|value| value.clamp(0.0, 1.0));
            let key = [
                from[0].to_bits(),
                from[1].to_bits(),
                from[2].to_bits(),
                to[0].to_bits(),
                to[1].to_bits(),
                to[2].to_bits(),
            ];
            if seen.insert(key) {
                self.spawn_box(pos, from, to, texture.clone(), tint);
            }
        }
        // The particle engine caps active particles independently of chunks.
        if self.fragments.len() > 4096 {
            self.fragments.drain(..self.fragments.len() - 4096);
        }
        Ok(())
    }

    fn material<S: Scene>(
        &self,
        packs: &PackStack,
        scene: &S,
        pos: BlockPos,
        block: &Block,
        atlas: &Atlas,
    ) -> Result<Option<(ResourceId, [f32; 3])>> {
        if matches!(
            block.id.path.as_str(),
            "air" | "cave_air" | "void_air" | "water" | "lava" | "moving_piston"
        ) {
            return Ok(None);
        }
        let Some(texture) = model::block_particle_texture(packs, block)? else {
            return Ok(None);
        };
        if !atlas.contains(&texture) {
            return Ok(None);
        }
        let tint = self.particle_tint(scene, pos, block);
        Ok(Some((texture, tint)))
    }

    /// BlockColors.getTintSource(state, 0) followed by
    /// BlockTintSource.colorAsTerrainParticle. Grass blocks override this
    /// method to return white, since their particle texture is dirt.
    fn particle_tint<S: Scene>(&self, scene: &S, pos: BlockPos, block: &Block) -> [f32; 3] {
        match block.id.path.as_str() {
            "short_grass" | "fern" | "potted_fern" | "bush" => self.tint.grass(scene.biome_at(pos)),
            "tall_grass" | "large_fern" => {
                let sample = if block
                    .properties
                    .get("half")
                    .is_some_and(|half| half == "upper")
                {
                    (pos.0, pos.1 - 1, pos.2)
                } else {
                    pos
                };
                self.tint.grass(scene.biome_at(sample))
            }
            "spruce_leaves" => [0x61, 0x99, 0x61].map(|c| c as f32 / 255.0),
            "birch_leaves" => [0x80, 0xa7, 0x55].map(|c| c as f32 / 255.0),
            "oak_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves"
            | "mangrove_leaves" | "vine" => self.tint.foliage(scene.biome_at(pos)),
            "lily_pad" => [0x20, 0x80, 0x30].map(|c| c as f32 / 255.0),
            "redstone_wire" => {
                let power = block
                    .properties
                    .get("power")
                    .and_then(|value| value.parse::<u8>().ok())
                    .unwrap_or(0);
                let color = redstone_wire_color_argb(power);
                [
                    ((color >> 16) & 255) as f32 / 255.0,
                    ((color >> 8) & 255) as f32 / 255.0,
                    (color & 255) as f32 / 255.0,
                ]
            }
            _ => [1.0; 3],
        }
    }

    /// Entity.spawnSprintParticle: one BLOCK particle per sprint tick from
    /// the block 0.2 below the player's feet. The caller checks entity state.
    pub fn spawn_sprint<S: Scene>(
        &mut self,
        packs: &PackStack,
        scene: &S,
        feet: Vec3,
        movement: Vec3,
        atlas: &Atlas,
        particle_option: f32,
    ) -> Result<bool> {
        let pos = (
            feet.x.floor() as i32,
            (feet.y - 0.2).floor() as i32,
            feet.z.floor() as i32,
        );
        let Some(block) = scene.block(pos) else {
            return Ok(false);
        };
        let Some((texture, tint)) = self.material(packs, scene, pos, block, atlas)? else {
            return Ok(false);
        };
        // ClientLevel.addParticle applies the particle setting to this effect.
        if particle_option >= 0.75 || (particle_option >= 0.25 && self.random() < 1.0 / 3.0) {
            return Ok(false);
        }
        let x = feet.x + (self.random() - 0.5) * 0.6;
        let z = feet.z + (self.random() - 0.5) * 0.6;
        self.spawn_fragment(
            Vec3::new(x, feet.y + 0.1, z),
            Vec3::new(movement.x * -4.0, 1.5, movement.z * -4.0),
            texture,
            tint,
        );
        Ok(true)
    }

    fn spawn_box(
        &mut self,
        pos: BlockPos,
        from: [f32; 3],
        to: [f32; 3],
        texture: ResourceId,
        tint: [f32; 3],
    ) {
        let width = Vec3::from_array(to) - Vec3::from_array(from);
        if width.min_element() <= 0.0 {
            return;
        }
        // ClientLevel.addDestroyBlockEffect samples each shape box on a
        // quarter-block grid, with at least two samples per axis.
        let counts = width
            .to_array()
            .map(|size| ((size / 0.25).ceil() as usize).max(2));
        for x in 0..counts[0] {
            for y in 0..counts[1] {
                for z in 0..counts[2] {
                    let relative = Vec3::new(
                        (x as f32 + 0.5) / counts[0] as f32,
                        (y as f32 + 0.5) / counts[1] as f32,
                        (z as f32 + 0.5) / counts[2] as f32,
                    );
                    let position = Vec3::new(pos.0 as f32, pos.1 as f32, pos.2 as f32)
                        + Vec3::from_array(from)
                        + relative * width;
                    self.spawn_fragment(
                        position,
                        relative - Vec3::splat(0.5),
                        texture.clone(),
                        tint,
                    );
                }
            }
        }
    }

    fn spawn_fragment(
        &mut self,
        position: Vec3,
        impulse: Vec3,
        texture: ResourceId,
        tint: [f32; 3],
    ) {
        let lifetime = (4.0 / (self.random() * 0.9 + 0.1)) as u8;
        let direction = impulse
            + Vec3::new(
                (self.random() * 2.0 - 1.0) * 0.4,
                (self.random() * 2.0 - 1.0) * 0.4,
                (self.random() * 2.0 - 1.0) * 0.4,
            );
        let speed = (self.random() + self.random() + 1.0) * 0.15 * 0.4;
        let velocity = direction.normalize_or_zero() * speed + Vec3::Y * 0.1;
        // TerrainParticle halves SingleQuadParticle's 0.1..0.2 size.
        let size = 0.05 + self.random() * 0.05;
        let uv_origin = Vec2::new(self.random() * 0.75, self.random() * 0.75);
        self.fragments.push(Fragment {
            position,
            previous: position,
            velocity,
            texture,
            uv_origin,
            color: [0.6 * tint[0], 0.6 * tint[1], 0.6 * tint[2]],
            size,
            age: 0,
            lifetime,
            stopped: false,
        });
    }

    pub fn tick<S: Scene>(&mut self, scene: &S) {
        for fragment in &mut self.fragments {
            fragment.previous = fragment.position;
            fragment.age += 1;
            if fragment.stopped {
                continue;
            }
            fragment.velocity.y -= 0.04;
            let mut next = fragment.position + fragment.velocity;
            if fragment.velocity.y < 0.0 {
                let below = (
                    next.x.floor() as i32,
                    (next.y - 0.001).floor() as i32,
                    next.z.floor() as i32,
                );
                if let Some(block) = scene.block(below) {
                    if block.is_opaque() && fragment.position.y >= below.1 as f32 + 1.0 {
                        next.y = below.1 as f32 + 1.0;
                        fragment.stopped = true;
                    }
                }
            }
            fragment.position = next;
            fragment.velocity *= 0.98;
        }
        self.fragments
            .retain(|fragment| fragment.age <= fragment.lifetime);
    }

    pub fn append_mesh(
        &self,
        mesh: &mut ChunkMesh,
        atlas: &Atlas,
        forward: Vec3,
        partial: f32,
        sky_light: &crate::lighting::SkyLight,
    ) {
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward).normalize();
        for fragment in &self.fragments {
            let [u0, v0, u1, v1] = atlas.region(&fragment.texture);
            let uv_min = Vec2::new(
                u0 + (u1 - u0) * fragment.uv_origin.x,
                v0 + (v1 - v0) * fragment.uv_origin.y,
            );
            let uv_max = uv_min + Vec2::new((u1 - u0) * 0.25, (v1 - v0) * 0.25);
            let center = fragment
                .previous
                .lerp(fragment.position, partial.clamp(0.0, 1.0));
            let light_pos = (
                center.x.floor() as i32,
                center.y.floor() as i32,
                center.z.floor() as i32,
            );
            let light = sky_light.get(light_pos) as f32;
            let start = mesh.vertices.len() as u32;
            for (corner, uv) in [
                (Vec2::new(-1.0, -1.0), [uv_min.x, uv_max.y]),
                (Vec2::new(-1.0, 1.0), [uv_min.x, uv_min.y]),
                (Vec2::new(1.0, 1.0), [uv_max.x, uv_min.y]),
                (Vec2::new(1.0, -1.0), [uv_max.x, uv_max.y]),
            ] {
                mesh.vertices.push(Vertex {
                    position: (center + (right * corner.x + up * corner.y) * fragment.size)
                        .to_array(),
                    uv,
                    color: [fragment.color[0], fragment.color[1], fragment.color[2], 1.0],
                    sky_light: light,
                    block_light: sky_light.get_block(light_pos) as f32,
                });
            }
            mesh.indices.extend_from_slice(&[
                start,
                start + 2,
                start + 1,
                start,
                start + 3,
                start + 2,
            ]);
            mesh.faces += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.fragments.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mesh, scene::HandcraftedScene};
    use image::{Rgba, RgbaImage};
    use std::fs;

    #[test]
    fn grass_block_dirt_particles_are_not_biome_tinted() {
        let packs = PackStack::open(vec![]).unwrap();
        let particles = BlockParticles::new(&packs).unwrap();
        let scene = HandcraftedScene::new();
        let pos = (0, 0, 0);
        assert_eq!(
            particles.particle_tint(&scene, pos, &Block::new("minecraft:grass_block")),
            [1.0; 3]
        );
        assert_eq!(
            particles.particle_tint(&scene, pos, &Block::new("minecraft:dirt")),
            [1.0; 3]
        );
        assert_eq!(
            particles.particle_tint(&scene, pos, &Block::new("minecraft:stone")),
            [1.0; 3]
        );
        let short_grass =
            particles.particle_tint(&scene, pos, &Block::new("minecraft:short_grass"));
        assert!(short_grass[0] < 1.0 && short_grass[1] < 1.0);
    }

    #[test]
    fn leaf_particle_tints_follow_block_colors_registry() {
        let packs = PackStack::open(vec![]).unwrap();
        let particles = BlockParticles::new(&packs).unwrap();
        let scene = HandcraftedScene::new();
        let pos = (0, 0, 0);
        let tint = |id| particles.particle_tint(&scene, pos, &Block::new(id));
        assert_eq!(
            tint("minecraft:spruce_leaves"),
            [0x61, 0x99, 0x61].map(|c| c as f32 / 255.0)
        );
        assert_eq!(
            tint("minecraft:birch_leaves"),
            [0x80, 0xa7, 0x55].map(|c| c as f32 / 255.0)
        );
        assert_eq!(tint("minecraft:cherry_leaves"), [1.0; 3]);
        assert_eq!(
            tint("minecraft:oak_leaves"),
            particles.tint.foliage(scene.biome_at(pos))
        );
    }

    #[test]
    fn redstone_wire_particles_use_power_color() {
        let packs = PackStack::open(vec![]).unwrap();
        let particles = BlockParticles::new(&packs).unwrap();
        let scene = HandcraftedScene::new();
        let color = particles.particle_tint(
            &scene,
            (0, 0, 0),
            &Block::new("minecraft:redstone_wire").with("power", "15"),
        );
        assert_eq!(color, [1.0, 50.0 / 255.0, 0.0]);
    }

    #[test]
    fn full_cube_break_emits_the_vanilla_sixty_four_grid_positions() {
        let packs = PackStack::open(vec![]).unwrap();
        let mut particles = BlockParticles::new(&packs).unwrap();
        particles.spawn_box(
            (3, 4, 5),
            [0.0; 3],
            [1.0; 3],
            ResourceId::parse("minecraft:block/stone").unwrap(),
            [1.0; 3],
        );
        assert_eq!(particles.len(), 64);
        assert_eq!(
            particles.fragments[0].position,
            Vec3::new(3.125, 4.125, 5.125)
        );
        assert_eq!(
            particles.fragments[63].position,
            Vec3::new(3.875, 4.875, 5.875)
        );
    }

    #[test]
    fn sprint_fragment_uses_the_ground_block_material_and_particle_setting() {
        let temp = crate::test_dir::tempdir().unwrap();
        let root = temp.path();
        for (name, contents) in [
            (
                "pack.mcmeta",
                r#"{"pack":{"min_format":[97,1],"max_format":[97,1]}}"#,
            ),
            (
                "assets/test/blockstates/stone.json",
                r#"{"variants":{"":{"model":"test:block/stone"}}}"#,
            ),
            (
                "assets/test/models/block/stone.json",
                r##"{"textures":{"particle":"test:block/stone"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#particle"}}}]}"##,
            ),
        ] {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        let texture = root.join("assets/test/textures/block/stone.png");
        fs::create_dir_all(texture.parent().unwrap()).unwrap();
        RgbaImage::from_pixel(16, 16, Rgba([120, 120, 120, 255]))
            .save(texture)
            .unwrap();
        let packs = PackStack::open(vec![root.into()]).unwrap();
        let mut scene = HandcraftedScene::default();
        scene.set((0, 0, 0), Some(Block::new("test:stone")));
        let build = mesh::build_minimal(&scene, &packs).unwrap();
        let mut particles = BlockParticles::new(&packs).unwrap();
        assert!(particles
            .spawn_sprint(
                &packs,
                &scene,
                Vec3::new(0.5, 1.0, 0.5),
                Vec3::new(0.25, 0.0, 0.0),
                &build.atlas,
                0.0,
            )
            .unwrap());
        assert_eq!(particles.len(), 1);
        assert_eq!(particles.fragments[0].texture.key(), "test:block/stone");
        assert!(particles.fragments[0].velocity.x < 0.0);
        assert!(particles.fragments[0].velocity.y > 0.0);
        assert!(!particles
            .spawn_sprint(
                &packs,
                &scene,
                Vec3::new(0.5, 1.0, 0.5),
                Vec3::ZERO,
                &build.atlas,
                1.0,
            )
            .unwrap());
        assert_eq!(particles.len(), 1);
    }
}
